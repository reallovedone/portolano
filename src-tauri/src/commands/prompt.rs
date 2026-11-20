//! Credential prompts on the backend side (WP11): when a session has to ask the user for a
//! credential, and what happens when the user asks to keep what was typed.
//!
//! The vault is optional. A connection whose effective credential is absent (none, inherited
//! from nowhere, or a dangling reference) is asked for at connect time; the typed secrets live
//! in the session for as long as it runs and are never written to the document or to disk.
//! Only when the user ticks "save to vault", a vault exists and is unlocked, and the
//! authentication succeeded, a vault credential is created and the connection points to it.

use std::sync::{Arc, Mutex};

use portolano_core_model::prompt::{PromptHooks, SavedCredential, SavedSecret};
use portolano_core_model::{CredentialRef, DocumentStore, ErrorCode, Node};
use portolano_vault::{CredentialInput, SecretInput, Vault, VaultStatus};
use uuid::Uuid;

use crate::error::{poisoned, AppError};
use crate::state::AppState;

/// Decides where the credential of a connection comes from.
///
/// - `Ok(Some(id))`: read it from the (unlocked) vault.
/// - `Ok(None)`: ask the user — the connection has no credential, the reference is dangling
///   (there is no vault, or the vault does not hold it), or the user chose to type it
///   (`ask_requested`).
/// - `Err(vault_locked)`: the credential is in a vault that exists but is locked; the UI offers
///   the unlock, or "enter manually for this session" (`ask_requested`).
pub fn vault_credential(
    state: &AppState,
    credential: Option<Uuid>,
    ask_requested: bool,
) -> Result<Option<Uuid>, AppError> {
    let Some(id) = credential else {
        return Ok(None);
    };
    if ask_requested {
        return Ok(None);
    }
    let vault = state.vault.lock().map_err(poisoned)?;
    match vault.status()? {
        VaultStatus::NotCreated => Ok(None),
        VaultStatus::Locked => Err(AppError::new(
            ErrorCode::VaultLocked,
            "The vault is locked: unlock it, or enter the credential by hand.",
        )),
        VaultStatus::Unlocked => Ok(vault.list()?.iter().any(|c| c.id == id).then_some(id)),
    }
}

/// Event (to the main webview) telling the UI that "save to the vault" failed after a
/// successful connection. Payload: `{ connectionId, connectionName, code, message }`; the
/// message is the `AppError` message, which never contains the secret.
pub const CREDENTIAL_SAVE_FAILED_EVENT: &str = "credential-save-failed";

fn save_failed_payload(
    connection_id: Uuid,
    connection_name: &str,
    error: &AppError,
) -> serde_json::Value {
    serde_json::json!({
        "connectionId": connection_id,
        "connectionName": connection_name,
        "code": error.code,
        "message": error.message,
    })
}

/// Hooks of a session that may prompt for `connection_id`'s credential.
pub fn hooks(state: &AppState, connection_id: Uuid, connection_name: String) -> PromptHooks {
    let probe_vault = state.vault.clone();
    let vault = state.vault.clone();
    let store = state.store.clone();
    let notifier = state.notifier.clone();
    PromptHooks {
        can_save: Arc::new(move || {
            // Never wait for a busy vault (an unlock may be running): just do not offer.
            probe_vault
                .try_lock()
                .map(|v| v.is_unlocked())
                .unwrap_or(false)
        }),
        saved: Arc::new(move |credential| {
            if let Err(e) = save(&vault, &store, connection_id, &connection_name, credential) {
                // The message never contains the secret.
                tracing::warn!(
                    code = e.code.as_str(),
                    "could not save the typed credential to the vault: {}",
                    e.message
                );
                // The session stays connected: the UI shows a non-blocking warning.
                notifier.emit(
                    CREDENTIAL_SAVE_FAILED_EVENT,
                    save_failed_payload(connection_id, &connection_name, &e),
                );
            }
        }),
    }
}

/// Creates a vault credential from what the user typed and points the connection to it.
pub fn save(
    vault: &Mutex<Vault>,
    store: &Mutex<DocumentStore>,
    connection_id: Uuid,
    connection_name: &str,
    credential: SavedCredential,
) -> Result<Uuid, AppError> {
    let secret = match credential.secret {
        SavedSecret::Password(password) => SecretInput::Password { password },
        SavedSecret::SshKey {
            private_key_pem,
            passphrase,
        } => SecretInput::SshKey {
            private_key_pem,
            passphrase,
        },
    };
    let id = vault
        .lock()
        .map_err(poisoned)?
        .upsert(CredentialInput {
            id: None,
            name: format!("{connection_name} ({})", credential.username),
            username: credential.username,
            domain: credential.domain,
            secret,
        })?
        .id;

    let linked = (|| {
        let mut store = store.lock().map_err(poisoned)?;
        store.ensure_loaded()?;
        let Some(Node::Connection(connection)) = store.document().node(connection_id).cloned()
        else {
            return Err(AppError::not_found(format!(
                "connection {connection_id} not found"
            )));
        };
        let mut connection = connection;
        connection.credential = CredentialRef::Id { id };
        store.upsert_node(Node::Connection(connection))?;
        Ok(())
    })();
    if let Err(e) = linked {
        // Do not leave a credential nobody points to.
        match vault.lock() {
            Ok(mut v) => {
                if let Err(undo) = v.delete(id) {
                    tracing::warn!("could not undo the saved credential: {undo}");
                }
            }
            Err(_) => tracing::warn!("could not undo the saved credential: vault state poisoned"),
        }
        return Err(e);
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use portolano_core_model::{Connection, ConnectionOptions};
    use portolano_vault::{KdfParams, RevealedSecret};
    use secrecy::SecretString;

    use super::*;

    fn scratch() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp11");
        std::fs::create_dir_all(&base).expect("create base");
        tempfile::tempdir_in(base).expect("scratch")
    }

    fn state(dir: &Path) -> AppState {
        let mut state = AppState::new(dir.to_path_buf());
        let kdf = KdfParams {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        };
        let path: PathBuf = dir.join("vault.bin");
        state.vault = Arc::new(Mutex::new(
            Vault::with_kdf_params(path, kdf).expect("vault params"),
        ));
        state
    }

    fn put_connection(state: &AppState, credential: CredentialRef) -> Uuid {
        let id = Uuid::new_v4();
        let mut store = state.store.lock().expect("store");
        store.ensure_loaded().expect("load");
        store
            .upsert_node(Node::Connection(Connection {
                sort_index: None,
                icon: None,
                color: None,
                id,
                parent_id: None,
                name: "srv".into(),
                host: "h".into(),
                port: None,
                credential,
                username: None,
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::Ssh {
                    jump_host: None,
                    keepalive_secs: None,
                },
            }))
            .expect("upsert");
        id
    }

    fn typed_password(user: &str, password: &str) -> SavedCredential {
        SavedCredential {
            username: user.into(),
            domain: Some("CORP".into()),
            secret: SavedSecret::Password(SecretString::from(password.to_owned())),
        }
    }

    fn create_vault(state: &AppState) {
        state
            .vault
            .lock()
            .expect("vault")
            .create(SecretString::from("master-for-tests".to_owned()))
            .expect("create vault");
    }

    #[test]
    fn a_connection_without_credential_asks_and_never_needs_a_vault() {
        let dir = scratch();
        let state = state(dir.path());
        // No vault file exists at all.
        assert_eq!(
            state.vault.lock().expect("vault").status().expect("status"),
            VaultStatus::NotCreated
        );
        assert_eq!(vault_credential(&state, None, false).expect("none"), None);
        // A dangling reference with a vault that was never created is "no credential".
        assert_eq!(
            vault_credential(&state, Some(Uuid::new_v4()), false).expect("dangling"),
            None
        );
        assert!(!(hooks(&state, Uuid::new_v4(), "x".into()).can_save)());
    }

    #[test]
    fn a_locked_vault_keeps_the_unlock_flow_unless_the_user_types_by_hand() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = Uuid::new_v4();
        state.vault.lock().expect("vault").lock().expect("lock");
        let err = vault_credential(&state, Some(id), false).expect_err("locked");
        assert_eq!(err.code, ErrorCode::VaultLocked);
        // "Enter manually for this session" leads to the same prompt.
        assert_eq!(
            vault_credential(&state, Some(id), true).expect("manual"),
            None
        );
        // Saving is not offered while the vault is locked.
        assert!(!(hooks(&state, Uuid::new_v4(), "x".into()).can_save)());
    }

    #[test]
    fn an_unlocked_vault_returns_known_credentials_and_treats_unknown_ones_as_dangling() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let known = state
            .vault
            .lock()
            .expect("vault")
            .upsert(CredentialInput {
                id: None,
                name: "k".into(),
                username: "u".into(),
                domain: None,
                secret: SecretInput::Password {
                    password: SecretString::from("p".to_owned()),
                },
            })
            .expect("upsert")
            .id;
        assert_eq!(
            vault_credential(&state, Some(known), false).expect("known"),
            Some(known)
        );
        assert_eq!(
            vault_credential(&state, Some(Uuid::new_v4()), false).expect("dangling"),
            None
        );
        // Typing by hand wins even when the vault could answer.
        assert_eq!(
            vault_credential(&state, Some(known), true).expect("manual"),
            None
        );
        assert!((hooks(&state, Uuid::new_v4(), "x".into()).can_save)());
    }

    #[test]
    fn saving_creates_the_vault_credential_and_updates_the_document() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = put_connection(&state, CredentialRef::None);

        let credential_id = save(
            &state.vault,
            &state.store,
            id,
            "srv",
            typed_password("alice", "pw-typed"),
        )
        .expect("save");

        let vault = state.vault.lock().expect("vault");
        let summary = vault
            .list()
            .expect("list")
            .into_iter()
            .find(|c| c.id == credential_id)
            .expect("created");
        assert_eq!(summary.username, "alice");
        assert_eq!(summary.domain.as_deref(), Some("CORP"));
        assert_eq!(summary.name, "srv (alice)");
        match vault.reveal_secret(credential_id).expect("reveal") {
            RevealedSecret::Password { password } => assert_eq!(password, "pw-typed"),
            RevealedSecret::SshKey { .. } => panic!("expected a password"),
        }
        drop(vault);

        // The connection now points at it, in memory and on disk.
        let reloaded = DocumentStore::load(portolano_core_model::connections_path(dir.path()))
            .expect("reload");
        let Some(Node::Connection(c)) = reloaded.document().node(id) else {
            panic!("connection missing");
        };
        assert_eq!(c.credential, CredentialRef::Id { id: credential_id });
        // The document holds no secret.
        let file = std::fs::read_to_string(portolano_core_model::connections_path(dir.path()))
            .expect("read");
        assert!(!file.contains("pw-typed"));
    }

    #[test]
    fn a_hook_call_saves_and_a_failed_link_leaves_no_orphan_credential() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = put_connection(&state, CredentialRef::Inherit);
        (hooks(&state, id, "srv".into()).saved)(typed_password("bob", "pw-2"));
        assert_eq!(
            state
                .vault
                .lock()
                .expect("vault")
                .list()
                .expect("list")
                .len(),
            1
        );

        // A connection that no longer exists: the credential is removed again.
        let gone = Uuid::new_v4();
        let err = save(
            &state.vault,
            &state.store,
            gone,
            "gone",
            typed_password("carol", "pw-3"),
        )
        .expect_err("no such connection");
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(
            state
                .vault
                .lock()
                .expect("vault")
                .list()
                .expect("list")
                .len(),
            1
        );
    }

    #[test]
    fn saving_with_a_locked_vault_fails_without_touching_the_document() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = put_connection(&state, CredentialRef::None);
        state.vault.lock().expect("vault").lock().expect("lock");
        let err = save(
            &state.vault,
            &state.store,
            id,
            "srv",
            typed_password("a", "b"),
        )
        .expect_err("locked");
        assert_eq!(err.code, ErrorCode::VaultLocked);
        let store = state.store.lock().expect("store");
        let Some(Node::Connection(c)) = store.document().node(id) else {
            panic!("connection missing");
        };
        assert_eq!(c.credential, CredentialRef::None);
    }

    #[test]
    fn a_failed_save_is_sent_to_the_ui_without_the_secret() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = put_connection(&state, CredentialRef::None);
        let notices: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
        let sink = notices.clone();
        state.notifier.connect(move |event, payload| {
            sink.lock().expect("sink").push((event.to_owned(), payload));
        });

        // The vault gets locked between the prompt and the save: the save fails.
        state.vault.lock().expect("vault").lock().expect("lock");
        (hooks(&state, id, "srv".into()).saved)(typed_password("dave", "pw-secret-4"));

        let notices = notices.lock().expect("notices");
        assert_eq!(notices.len(), 1);
        let (event, payload) = &notices[0];
        assert_eq!(event, CREDENTIAL_SAVE_FAILED_EVENT);
        assert_eq!(payload["connectionId"], id.to_string());
        assert_eq!(payload["connectionName"], "srv");
        assert_eq!(payload["code"], "vault_locked");
        assert!(payload["message"].is_string());
        assert!(!payload.to_string().contains("pw-secret-4"));
    }

    #[test]
    fn a_successful_save_sends_no_notice() {
        let dir = scratch();
        let state = state(dir.path());
        create_vault(&state);
        let id = put_connection(&state, CredentialRef::None);
        let count = Arc::new(Mutex::new(0usize));
        let sink = count.clone();
        state.notifier.connect(move |_, _| {
            *sink.lock().expect("sink") += 1;
        });
        (hooks(&state, id, "srv".into()).saved)(typed_password("erin", "pw-5"));
        assert_eq!(*count.lock().expect("count"), 0);
    }
}
