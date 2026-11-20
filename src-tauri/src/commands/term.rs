//! Terminal family commands (local shell + SSH) and the known_hosts commands (WP5).
//!
//! `on_event` carries raw binary frames (`TermEvent::encode`), not JSON. `term_write` takes the
//! typed bytes as the raw request body and the session id in the `x-session-id` header.
//!
//! `term_open` resolves everything that can be known up front (connection, credential, jump
//! chain, vault) and fails synchronously with a stable code (`not_found`, `invalid_input`,
//! `vault_locked`, `unsupported`). The network part runs in the background and reports through
//! the channel: `HostKeyPrompt`, `CredentialPrompt`, data, or `Error`. Secrets are read from
//! the unlocked vault here, in the backend, and never leave it. The vault is optional: a
//! connection without a usable credential makes the session ask the user (`prompt.rs`).

use std::collections::HashSet;
use std::sync::Arc;

use portolano_core_model::prompt::CredentialAnswer;
use portolano_core_model::{known_hosts_path, resolve_credential, ConnectionOptions, Node};
use portolano_session_term::{
    FrameSink, KnownHostEntry, LocalSpec, Session, SessionSpec, SshAuth, SshHop, SshSpec,
};
use portolano_vault::RevealedSecret;
use secrecy::SecretString;
use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, Request};
use tauri::State;
use uuid::Uuid;

use crate::commands::model::with_document;
use crate::commands::prompt;
use crate::error::{poisoned, AppError};
use crate::state::{AppState, BoxFut, SessionHandle, SessionId, TermSession};

/// How many jump hosts a chain may have (also stops accidental cycles).
const MAX_JUMPS: usize = 8;
const SSH_DEFAULT_PORT: u16 = 22;

/// The part of a connection needed to open a terminal, read from the document.
enum Plan {
    Local(LocalSpec),
    Ssh {
        /// Hops from the first jump host to the target (the target is last).
        hops: Vec<HopPlan>,
        keepalive_secs: Option<u32>,
    },
}

struct HopPlan {
    connection_id: Uuid,
    name: String,
    /// The connection's optional user name: pre-fills the credential prompt.
    username: Option<String>,
    host: String,
    port: u16,
    credential: Option<Uuid>,
}

fn connection_not_found(id: Uuid) -> AppError {
    AppError::not_found(format!("connection {id} not found"))
}

fn plan(state: &AppState, connection_id: Uuid) -> Result<Plan, AppError> {
    with_document(state, |doc| {
        let connection = |id: Uuid| match doc.node(id) {
            Some(Node::Connection(c)) => Ok(c),
            Some(Node::Folder(_)) => Err(AppError::invalid_input(format!(
                "{id} is a folder, not a connection"
            ))),
            None => Err(connection_not_found(id)),
        };
        let target = connection(connection_id)?;
        match &target.options {
            ConnectionOptions::LocalShell { program, args, cwd } => Ok(Plan::Local(LocalSpec {
                program: program.clone(),
                args: args.clone(),
                cwd: cwd.clone(),
            })),
            ConnectionOptions::Ssh {
                jump_host,
                keepalive_secs,
            } => {
                let hop =
                    |id: Uuid, c: &portolano_core_model::Connection| -> Result<HopPlan, AppError> {
                        Ok(HopPlan {
                            connection_id: id,
                            name: c.name.clone(),
                            username: c.username.clone(),
                            host: c.host.trim().to_string(),
                            port: c.port.unwrap_or(SSH_DEFAULT_PORT),
                            credential: resolve_credential(doc, id)?,
                        })
                    };
                // Walk the jump chain backwards from the target, then reverse it.
                let mut hops = vec![hop(connection_id, target)?];
                let mut seen: HashSet<Uuid> = HashSet::from([connection_id]);
                let mut next = *jump_host;
                while let Some(id) = next {
                    if !seen.insert(id) || hops.len() > MAX_JUMPS {
                        return Err(AppError::invalid_input(
                            "the jump host chain is too long or contains a cycle",
                        ));
                    }
                    let jump = connection(id)?;
                    match &jump.options {
                        ConnectionOptions::Ssh { jump_host, .. } => next = *jump_host,
                        _ => {
                            return Err(AppError::invalid_input(format!(
                                "the jump host \"{}\" is not an SSH connection",
                                jump.name
                            )))
                        }
                    }
                    hops.push(hop(id, jump)?);
                }
                hops.reverse();
                Ok(Plan::Ssh {
                    hops,
                    keepalive_secs: *keepalive_secs,
                })
            }
            _ => Err(AppError::invalid_input(format!(
                "connection \"{}\" is not a terminal connection",
                target.name
            ))),
        }
    })
}

/// Resolves the credential of a hop. A connection without a usable credential is asked for at
/// connect time (`SshAuth::Ask`); a credential in a locked vault is `vault_locked` (the UI
/// offers the unlock, or typing it by hand, which is `ask_requested`).
fn resolve_hop(state: &AppState, plan: &HopPlan, ask_requested: bool) -> Result<SshHop, AppError> {
    if plan.host.is_empty() {
        return Err(AppError::invalid_input(format!(
            "connection \"{}\" has no host",
            plan.name
        )));
    }
    let Some(credential_id) = prompt::vault_credential(state, plan.credential, ask_requested)?
    else {
        return Ok(SshHop {
            host: plan.host.clone(),
            port: plan.port,
            username: plan.username.clone().unwrap_or_default(),
            auth: SshAuth::Ask(prompt::hooks(state, plan.connection_id, plan.name.clone())),
        });
    };
    let vault = state.vault.lock().map_err(poisoned)?;
    let summary = vault
        .list()?
        .into_iter()
        .find(|c| c.id == credential_id)
        .ok_or_else(|| AppError::not_found(format!("credential {credential_id} not found")))?;
    let auth = match vault.reveal_secret(credential_id)? {
        RevealedSecret::Password { password } => SshAuth::Password(SecretString::from(password)),
        RevealedSecret::SshKey {
            private_key_pem,
            passphrase,
        } => SshAuth::Key {
            private_key: SecretString::from(private_key_pem),
            passphrase: passphrase.filter(|p| !p.is_empty()).map(SecretString::from),
        },
    };
    Ok(SshHop {
        host: plan.host.clone(),
        port: plan.port,
        username: summary.username,
        auth,
    })
}

fn spec_for(state: &AppState, plan: Plan, ask_requested: bool) -> Result<SessionSpec, AppError> {
    match plan {
        Plan::Local(local) => Ok(SessionSpec::Local(local)),
        Plan::Ssh {
            hops,
            keepalive_secs,
        } => {
            let mut resolved = hops
                .iter()
                .map(|h| resolve_hop(state, h, ask_requested))
                .collect::<Result<Vec<_>, _>>()?;
            let target = resolved
                .pop()
                .ok_or_else(|| AppError::internal("empty SSH hop list"))?;
            let mut spec = SshSpec::new(target, known_hosts_path(&state.data_dir));
            spec.jumps = resolved;
            spec.keepalive_secs = keepalive_secs;
            Ok(SessionSpec::Ssh(Box::new(spec)))
        }
    }
}

/// Adapter from the crate's `Session` to the app's `TermSession` handle trait.
struct TermHandle(Session);

impl TermSession for TermHandle {
    fn write(&self, data: Vec<u8>) -> BoxFut<'_, Result<(), AppError>> {
        Box::pin(self.0.write(data))
    }

    fn ack(&self, bytes: u64) -> Result<(), AppError> {
        self.0.ack(bytes);
        Ok(())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        self.0.resize(cols, rows)
    }

    fn host_key_decision(&self, accept: bool) -> Result<(), AppError> {
        self.0.host_key_decision(accept)
    }

    fn credential(&self, answer: CredentialAnswer, save_to_vault: bool) -> Result<(), AppError> {
        self.0.credential(answer, save_to_vault)
    }

    fn credential_cancel(&self) -> Result<(), AppError> {
        self.0.credential_cancel()
    }

    fn close(&self) {
        self.0.close();
    }
}

fn channel_sink(channel: Channel<InvokeResponseBody>) -> FrameSink {
    Arc::new(
        move |frame| match channel.send(InvokeResponseBody::Raw(frame)) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!("terminal channel closed: {e}");
                false
            }
        },
    )
}

#[tauri::command]
pub async fn term_open(
    state: State<'_, AppState>,
    connection_id: Uuid,
    cols: u16,
    rows: u16,
    on_event: Channel<InvokeResponseBody>,
    ask_credentials: Option<bool>,
) -> Result<SessionId, AppError> {
    let spec = spec_for(
        &state,
        plan(&state, connection_id)?,
        ask_credentials.unwrap_or(false),
    )?;
    let session = Session::open(spec, cols.max(1), rows.max(1), channel_sink(on_event))?;
    state
        .sessions
        .insert(SessionHandle::Term(Arc::new(TermHandle(session))))
}

#[tauri::command]
pub async fn term_write(request: Request<'_>, state: State<'_, AppState>) -> Result<(), AppError> {
    let id: SessionId = request
        .headers()
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::invalid_input("missing or malformed x-session-id header"))?;
    let data = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(_) => {
            return Err(AppError::invalid_input(
                "term_write expects the raw bytes as the request body",
            ))
        }
    };
    let session = state.sessions.term(id)?;
    session.write(data).await
}

#[tauri::command]
pub async fn term_ack(
    state: State<'_, AppState>,
    session_id: SessionId,
    bytes: u64,
) -> Result<(), AppError> {
    state.sessions.term(session_id)?.ack(bytes)
}

#[tauri::command]
pub async fn term_resize(
    state: State<'_, AppState>,
    session_id: SessionId,
    cols: u16,
    rows: u16,
) -> Result<(), AppError> {
    state.sessions.term(session_id)?.resize(cols, rows)
}

#[tauri::command]
pub async fn ssh_host_key_decision(
    state: State<'_, AppState>,
    session_id: SessionId,
    accept: bool,
) -> Result<(), AppError> {
    state.sessions.term(session_id)?.host_key_decision(accept)
}

#[tauri::command]
pub async fn known_hosts_list(state: State<'_, AppState>) -> Result<Vec<KnownHostEntry>, AppError> {
    Ok(portolano_session_term::known_hosts_list(
        &known_hosts_path(&state.data_dir),
    )?)
}

#[tauri::command]
pub async fn known_hosts_remove(
    state: State<'_, AppState>,
    host: String,
    port: u16,
) -> Result<(), AppError> {
    Ok(portolano_session_term::known_hosts_remove(
        &known_hosts_path(&state.data_dir),
        &host,
        port,
    )?)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use portolano_core_model::prompt::{CredentialPrompt, PromptKind};
    use portolano_core_model::{Connection, CredentialRef, ErrorCode, Folder};
    use portolano_session_term::{TAG_CREDENTIAL_PROMPT, TAG_DATA, TAG_ERROR, TAG_HOST_KEY_PROMPT};
    use portolano_vault::{CredentialInput, KdfParams, SecretInput, Vault};

    use super::*;

    /// The error of a result whose success type need not be `Debug`.
    trait ErrorOf {
        fn error(self) -> AppError;
    }

    impl<T> ErrorOf for Result<T, AppError> {
        fn error(self) -> AppError {
            match self {
                Ok(_) => panic!("expected an error"),
                Err(e) => e,
            }
        }
    }

    fn scratch() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp5");
        std::fs::create_dir_all(&base).expect("create base");
        tempfile::tempdir_in(base).expect("scratch")
    }

    fn state(dir: &Path) -> AppState {
        let state = AppState::new(dir.to_path_buf());
        // Cheap key derivation: the default (64 MiB) is slow in debug builds.
        let kdf = KdfParams {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        };
        *state.vault.lock().expect("vault") =
            Vault::with_kdf_params(dir.join("vault.bin"), kdf).expect("vault params");
        state
    }

    fn upsert(state: &AppState, node: Node) {
        let mut store = state.store.lock().expect("store");
        store.ensure_loaded().expect("load");
        store.upsert_node(node).expect("upsert");
    }

    fn ssh(
        name: &str,
        host: &str,
        port: Option<u16>,
        cred: CredentialRef,
        jump: Option<Uuid>,
    ) -> Connection {
        Connection {
            sort_index: None,
            icon: None,
            color: None,
            id: Uuid::new_v4(),
            parent_id: None,
            name: name.into(),
            host: host.into(),
            port,
            credential: cred,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Ssh {
                jump_host: jump,
                keepalive_secs: Some(15),
            },
        }
    }

    fn add_credential(state: &AppState, username: &str, password: &str) -> Uuid {
        let mut vault = state.vault.lock().expect("vault");
        vault
            .create(SecretString::from("master-for-tests"))
            .expect("create vault");
        vault
            .upsert(CredentialInput {
                id: None,
                name: "test".into(),
                username: username.into(),
                domain: None,
                secret: SecretInput::Password {
                    password: SecretString::from(password.to_string()),
                },
            })
            .expect("upsert credential")
            .id
    }

    #[test]
    fn local_shell_plan_needs_no_credential_or_vault() {
        let dir = scratch();
        let state = state(dir.path());
        let id = Uuid::new_v4();
        upsert(
            &state,
            Node::Connection(Connection {
                sort_index: None,
                icon: None,
                color: None,
                id,
                parent_id: None,
                name: "shell".into(),
                host: String::new(),
                port: None,
                credential: CredentialRef::None,
                username: None,
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::LocalShell {
                    program: Some("/bin/sh".into()),
                    args: vec!["-l".into()],
                    cwd: None,
                },
            }),
        );
        match spec_for(&state, plan(&state, id).expect("plan"), false).expect("spec") {
            SessionSpec::Local(l) => {
                assert_eq!(l.program.as_deref(), Some("/bin/sh"));
                assert_eq!(l.args, vec!["-l"]);
            }
            SessionSpec::Ssh(_) => panic!("expected a local spec"),
        }
    }

    #[test]
    fn unknown_folder_and_wrong_kind_are_reported_with_stable_codes() {
        let dir = scratch();
        let state = state(dir.path());
        assert_eq!(
            plan(&state, Uuid::new_v4()).error().code,
            ErrorCode::NotFound
        );

        let folder = Uuid::new_v4();
        upsert(
            &state,
            Node::Folder(Folder {
                sort_index: None,
                icon: None,
                color: None,
                id: folder,
                parent_id: None,
                name: "f".into(),
                credential: CredentialRef::None,
            }),
        );
        assert_eq!(plan(&state, folder).error().code, ErrorCode::InvalidInput);

        let web = Uuid::new_v4();
        upsert(
            &state,
            Node::Connection(Connection {
                sort_index: None,
                icon: None,
                color: None,
                id: web,
                parent_id: None,
                name: "w".into(),
                host: "h".into(),
                port: None,
                credential: CredentialRef::None,
                username: None,
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::Web {
                    url: "https://example.com".into(),
                    isolated_profile: false,
                },
            }),
        );
        assert_eq!(plan(&state, web).error().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn ssh_with_a_locked_vault_is_vault_locked() {
        let dir = scratch();
        let state = state(dir.path());
        let cred = add_credential(&state, "u", "p");
        state.vault.lock().expect("vault").lock().expect("lock");
        let c = ssh("srv", "h", None, CredentialRef::Id { id: cred }, None);
        let id = c.id;
        upsert(&state, Node::Connection(c));
        let err = spec_for(&state, plan(&state, id).expect("plan"), false).error();
        assert_eq!(err.code, ErrorCode::VaultLocked);
    }

    /// Deliberately changed by WP11: a connection without a credential used to fail with
    /// `invalid_input` (and a dangling reference with `not_found`); now the session asks.
    #[test]
    fn ssh_without_a_credential_or_with_a_dangling_one_asks_at_connect_time() {
        let dir = scratch();
        let state = state(dir.path());
        add_credential(&state, "u", "p");
        let mut none = ssh("a", "h", None, CredentialRef::None, None);
        none.username = Some("alice".into());
        let dangling = ssh(
            "b",
            "h",
            None,
            CredentialRef::Id { id: Uuid::new_v4() },
            None,
        );
        let (none_id, dangling_id) = (none.id, dangling.id);
        upsert(&state, Node::Connection(none));
        upsert(&state, Node::Connection(dangling));
        for (id, hint) in [(none_id, "alice"), (dangling_id, "")] {
            match spec_for(&state, plan(&state, id).expect("plan"), false).expect("spec") {
                SessionSpec::Ssh(s) => {
                    assert!(matches!(s.target.auth, SshAuth::Ask(_)));
                    // The connection's own user name pre-fills the prompt.
                    assert_eq!(s.target.username, hint);
                }
                SessionSpec::Local(_) => panic!("expected an SSH spec"),
            }
        }
    }

    #[test]
    fn no_vault_at_all_still_gives_a_spec_that_asks() {
        let dir = scratch();
        let state = state(dir.path());
        // `state()` creates no vault file: the app was never set up for credentials.
        let c = ssh(
            "srv",
            "h",
            None,
            CredentialRef::Id { id: Uuid::new_v4() },
            None,
        );
        let id = c.id;
        upsert(&state, Node::Connection(c));
        match spec_for(&state, plan(&state, id).expect("plan"), false).expect("spec") {
            SessionSpec::Ssh(s) => assert!(matches!(s.target.auth, SshAuth::Ask(_))),
            SessionSpec::Local(_) => panic!("expected an SSH spec"),
        }
    }

    #[test]
    fn a_locked_vault_can_be_bypassed_by_typing_the_credential() {
        let dir = scratch();
        let state = state(dir.path());
        let cred = add_credential(&state, "u", "p");
        state.vault.lock().expect("vault").lock().expect("lock");
        let c = ssh("srv", "h", None, CredentialRef::Id { id: cred }, None);
        let id = c.id;
        upsert(&state, Node::Connection(c));
        // The unlock flow stays ...
        let err = spec_for(&state, plan(&state, id).expect("plan"), false).error();
        assert_eq!(err.code, ErrorCode::VaultLocked);
        // ... and "enter manually for this session" leads to the prompt.
        match spec_for(&state, plan(&state, id).expect("plan"), true).expect("spec") {
            SessionSpec::Ssh(s) => assert!(matches!(s.target.auth, SshAuth::Ask(_))),
            SessionSpec::Local(_) => panic!("expected an SSH spec"),
        }
    }

    #[test]
    fn credential_is_inherited_and_the_jump_chain_is_ordered_first_to_target() {
        let dir = scratch();
        let state = state(dir.path());
        let cred = add_credential(&state, "alice", "pw");
        let folder = Uuid::new_v4();
        upsert(
            &state,
            Node::Folder(Folder {
                sort_index: None,
                icon: None,
                color: None,
                id: folder,
                parent_id: None,
                name: "lab".into(),
                credential: CredentialRef::Id { id: cred },
            }),
        );
        let mut outer = ssh(
            "outer",
            "outer.example",
            Some(2200),
            CredentialRef::Inherit,
            None,
        );
        outer.parent_id = Some(folder);
        let mut inner = ssh(
            "inner",
            "inner.example",
            None,
            CredentialRef::Inherit,
            Some(outer.id),
        );
        inner.parent_id = Some(folder);
        let mut target = ssh(
            "target",
            "target.example",
            Some(2222),
            CredentialRef::Inherit,
            Some(inner.id),
        );
        target.parent_id = Some(folder);
        let target_id = target.id;
        for c in [outer, inner, target] {
            upsert(&state, Node::Connection(c));
        }
        match spec_for(&state, plan(&state, target_id).expect("plan"), false).expect("spec") {
            SessionSpec::Ssh(s) => {
                let hosts: Vec<(&str, u16)> =
                    s.jumps.iter().map(|h| (h.host.as_str(), h.port)).collect();
                assert_eq!(hosts, vec![("outer.example", 2200), ("inner.example", 22)]);
                assert_eq!(
                    (s.target.host.as_str(), s.target.port),
                    ("target.example", 2222)
                );
                assert_eq!(s.target.username, "alice");
                assert_eq!(s.keepalive_secs, Some(15));
                assert_eq!(
                    s.known_hosts_path,
                    portolano_core_model::known_hosts_path(dir.path())
                );
                // Debug output never shows the secret.
                assert!(!format!("{s:?}").contains("pw\""));
            }
            SessionSpec::Local(_) => panic!("expected an SSH spec"),
        }
    }

    #[test]
    fn jump_cycles_and_non_ssh_jump_hosts_are_rejected() {
        let dir = scratch();
        let state = state(dir.path());
        let mut a = ssh("a", "a", None, CredentialRef::None, None);
        let b = ssh("b", "b", None, CredentialRef::None, Some(a.id));
        a.options = ConnectionOptions::Ssh {
            jump_host: Some(b.id),
            keepalive_secs: None,
        };
        let a_id = a.id;
        upsert(&state, Node::Connection(a));
        upsert(&state, Node::Connection(b));
        assert_eq!(plan(&state, a_id).error().code, ErrorCode::InvalidInput);

        let shell = Uuid::new_v4();
        upsert(
            &state,
            Node::Connection(Connection {
                sort_index: None,
                icon: None,
                color: None,
                id: shell,
                parent_id: None,
                name: "sh".into(),
                host: String::new(),
                port: None,
                credential: CredentialRef::None,
                username: None,
                tags: vec![],
                notes: String::new(),
                options: ConnectionOptions::LocalShell {
                    program: None,
                    args: vec![],
                    cwd: None,
                },
            }),
        );
        let c = ssh("c", "c", None, CredentialRef::None, Some(shell));
        let c_id = c.id;
        upsert(&state, Node::Connection(c));
        assert_eq!(plan(&state, c_id).error().code, ErrorCode::InvalidInput);
    }

    struct Collected(Mutex<Vec<Vec<u8>>>);

    impl Collected {
        fn sink(self: &Arc<Self>) -> FrameSink {
            let me = self.clone();
            Arc::new(move |f| {
                me.0.lock().expect("frames").push(f);
                true
            })
        }
        fn text(&self) -> String {
            let frames = self.0.lock().expect("frames");
            let bytes: Vec<u8> = frames
                .iter()
                .filter(|f| f[0] == TAG_DATA)
                .flat_map(|f| f[1..].to_vec())
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        fn has_tag(&self, tag: u8) -> bool {
            self.0.lock().expect("frames").iter().any(|f| f[0] == tag)
        }
        fn prompts(&self) -> Vec<CredentialPrompt> {
            self.0
                .lock()
                .expect("frames")
                .iter()
                .filter(|f| f[0] == TAG_CREDENTIAL_PROMPT)
                .map(|f| serde_json::from_slice(&f[1..]).expect("prompt json"))
                .collect()
        }
        fn error_code(&self) -> Option<String> {
            self.0
                .lock()
                .expect("frames")
                .iter()
                .find(|f| f[0] == TAG_ERROR)
                .and_then(|f| serde_json::from_slice::<serde_json::Value>(&f[1..]).ok())
                .and_then(|j| j["code"].as_str().map(str::to_owned))
        }
        /// Whether `needle` is anywhere in the bytes sent to the UI channel.
        fn contains(&self, needle: &str) -> bool {
            let needle = needle.as_bytes();
            self.0
                .lock()
                .expect("frames")
                .iter()
                .any(|f| f.windows(needle.len()).any(|w| w == needle))
        }
    }

    async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn local_shell_through_the_session_manager_and_term_handle() {
        let dir = scratch();
        let state = state(dir.path());
        // HISTFILE=/dev/null: an interactive `sh` (bash on macOS) would otherwise write
        // `~/.bash_history` in the real home folder.
        let spec = SessionSpec::Local(LocalSpec {
            program: Some("/usr/bin/env".into()),
            args: vec!["HISTFILE=/dev/null".into(), "/bin/sh".into()],
            cwd: Some(dir.path().to_string_lossy().into_owned()),
        });
        let out = Arc::new(Collected(Mutex::default()));
        let session = Session::open(spec, 80, 24, out.sink()).expect("open");
        let id = state
            .sessions
            .insert(SessionHandle::Term(Arc::new(TermHandle(session))))
            .expect("insert");

        let handle = state.sessions.term(id).expect("lookup");
        handle
            .write(b"echo via-handle-$((20+22))\n".to_vec())
            .await
            .expect("write");
        wait_for("the echo", || out.text().contains("via-handle-42")).await;
        handle.ack(4096).expect("ack");
        handle.resize(100, 30).expect("resize");
        assert_eq!(
            handle.host_key_decision(true).error().code,
            ErrorCode::InvalidInput
        );
        // A framebuffer lookup of a terminal session is a wrong-family error.
        assert!(state.sessions.fb(id).is_err());

        state.sessions.close(id).expect("close");
        assert_eq!(state.sessions.close(id).error().code, ErrorCode::NotFound);
        assert_eq!(
            handle.write(b"x".to_vec()).await.error().code,
            ErrorCode::Io
        );
    }

    fn server_password() -> Option<String> {
        if std::env::var_os("PORTOLANO_TEST_SERVERS_DOWN").is_some() {
            eprintln!("SKIPPED: PORTOLANO_TEST_SERVERS_DOWN is set");
            return None;
        }
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 12222));
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err() {
            eprintln!("SKIPPED: the test sshd on 127.0.0.1:12222 is not reachable");
            return None;
        }
        let Ok(text) = std::fs::read_to_string(crate::commands::test_env_path()) else {
            eprintln!("SKIPPED: the generated test credentials (test.env) are missing");
            return None;
        };
        text.lines()
            .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
            .map(|v| v.trim().trim_matches('"').to_string())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ssh_end_to_end_with_the_credential_resolved_from_the_vault() {
        let Some(password) = server_password() else {
            return;
        };
        let dir = scratch();
        let state = state(dir.path());
        let cred = add_credential(&state, "tester", &password);
        let folder = Uuid::new_v4();
        upsert(
            &state,
            Node::Folder(Folder {
                sort_index: None,
                icon: None,
                color: None,
                id: folder,
                parent_id: None,
                name: "lab".into(),
                credential: CredentialRef::Id { id: cred },
            }),
        );
        let mut c = ssh(
            "test sshd",
            "127.0.0.1",
            Some(12222),
            CredentialRef::Inherit,
            None,
        );
        c.parent_id = Some(folder);
        let id = c.id;
        upsert(&state, Node::Connection(c));

        let spec = spec_for(&state, plan(&state, id).expect("plan"), false).expect("spec");
        let out = Arc::new(Collected(Mutex::default()));
        let session = Session::open(spec, 80, 24, out.sink()).expect("open");
        let sid = state
            .sessions
            .insert(SessionHandle::Term(Arc::new(TermHandle(session))))
            .expect("insert");
        let handle = state.sessions.term(sid).expect("lookup");

        wait_for("the host key prompt", || out.has_tag(TAG_HOST_KEY_PROMPT)).await;
        handle.host_key_decision(true).expect("accept");
        wait_for("the shell", || !out.text().is_empty()).await;
        handle
            .write(b"echo whoami-$(id -un)\n".to_vec())
            .await
            .expect("write");
        wait_for("the command output", || {
            out.text().contains("whoami-tester")
        })
        .await;
        assert!(!out.text().contains(&password));
        let known = portolano_session_term::known_hosts_list(
            &portolano_core_model::known_hosts_path(dir.path()),
        )
        .expect("known hosts");
        assert_eq!(known.len(), 1);
        state.sessions.close_all();
    }

    // ---- credentials on demand (WP11): the whole path, with no vault ----

    static SSH_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A connection to the test sshd that carries no credential.
    fn asking_connection(state: &AppState, username: Option<&str>) -> Uuid {
        let mut c = ssh("asks", "127.0.0.1", Some(12222), CredentialRef::None, None);
        c.username = username.map(str::to_owned);
        let id = c.id;
        upsert(state, Node::Connection(c));
        id
    }

    fn open_session(
        state: &AppState,
        id: Uuid,
        ask: bool,
    ) -> (Arc<Collected>, Arc<dyn TermSession>, SessionId) {
        let spec = spec_for(state, plan(state, id).expect("plan"), ask).expect("spec");
        let out = Arc::new(Collected(Mutex::default()));
        let session = Session::open(spec, 80, 24, out.sink()).expect("open");
        let sid = state
            .sessions
            .insert(SessionHandle::Term(Arc::new(TermHandle(session))))
            .expect("insert");
        (out, state.sessions.term(sid).expect("lookup"), sid)
    }

    fn typed(user: &str, password: &str) -> CredentialAnswer {
        CredentialAnswer {
            username: user.into(),
            password: Some(SecretString::from(password.to_owned())),
            key_path: None,
            passphrase: None,
            domain: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ssh_with_no_vault_prompts_rejects_a_wrong_password_then_connects() {
        let Some(password) = server_password() else {
            return;
        };
        let _serial = SSH_SERIAL.lock().await;
        let dir = scratch();
        let state = state(dir.path());
        let id = asking_connection(&state, Some("tester"));
        let (out, _handle, sid) = open_session(&state, id, false);

        wait_for("the host key prompt", || out.has_tag(TAG_HOST_KEY_PROMPT)).await;
        state
            .sessions
            .term(sid)
            .expect("t")
            .host_key_decision(true)
            .expect("accept");
        wait_for("the credential prompt", || !out.prompts().is_empty()).await;
        let first = out.prompts().remove(0);
        assert_eq!(first.kind, PromptKind::SshKey);
        // `Connection.username` pre-fills the prompt: only the password is missing.
        assert_eq!(first.username_hint.as_deref(), Some("tester"));
        assert_eq!(first.target, "127.0.0.1:12222");
        assert!(!first.can_save_to_vault, "there is no vault to save into");

        // Wrong password typed by hand: the prompt comes back with the reason, same session.
        let wrong = "definitely-wrong-pw-77";
        state
            .sessions
            .credential(sid, typed("tester", wrong), false)
            .expect("wrong answer");
        wait_for("the second prompt", || out.prompts().len() >= 2).await;
        let second = out.prompts().remove(1);
        assert!(second.error.is_some_and(|e| !e.contains(wrong)));
        assert!(out.error_code().is_none());

        state
            .sessions
            .credential(sid, typed("tester", &password), false)
            .expect("right answer");
        wait_for("the shell", || !out.text().is_empty()).await;
        state
            .sessions
            .term(sid)
            .expect("t")
            .write(b"echo whoami-$(id -un)\n".to_vec())
            .await
            .expect("write");
        wait_for("the command output", || {
            out.text().contains("whoami-tester")
        })
        .await;

        // Nothing secret reached the UI, and nothing at all was written about a vault.
        assert!(!out.contains(&password) && !out.contains(wrong));
        assert!(!dir.path().join("vault.bin").exists());
        let doc = std::fs::read_to_string(portolano_core_model::connections_path(dir.path()))
            .unwrap_or_default();
        assert!(!doc.contains(&password) && !doc.contains(wrong));
        state.sessions.close_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_the_prompt_through_the_manager_ends_as_cancelled() {
        let Some(_) = server_password() else {
            return;
        };
        let _serial = SSH_SERIAL.lock().await;
        let dir = scratch();
        let state = state(dir.path());
        let id = asking_connection(&state, None);
        let (out, _handle, sid) = open_session(&state, id, false);
        wait_for("the host key prompt", || out.has_tag(TAG_HOST_KEY_PROMPT)).await;
        state
            .sessions
            .term(sid)
            .expect("t")
            .host_key_decision(true)
            .expect("accept");
        wait_for("the credential prompt", || !out.prompts().is_empty()).await;
        assert_eq!(out.prompts()[0].username_hint, None);
        state.sessions.credential_cancel(sid).expect("cancel");
        wait_for("the error", || out.error_code().is_some()).await;
        assert_eq!(out.error_code().as_deref(), Some("cancelled"));
        state.sessions.close_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn save_to_vault_creates_the_credential_and_points_the_connection_to_it() {
        let Some(password) = server_password() else {
            return;
        };
        let _serial = SSH_SERIAL.lock().await;
        let dir = scratch();
        let state = state(dir.path());
        // A vault that exists and is unlocked, with nothing assigned to the connection.
        state
            .vault
            .lock()
            .expect("vault")
            .create(SecretString::from("master-for-tests"))
            .expect("create vault");
        let id = asking_connection(&state, Some("tester"));
        let (out, _handle, sid) = open_session(&state, id, false);
        wait_for("the host key prompt", || out.has_tag(TAG_HOST_KEY_PROMPT)).await;
        state
            .sessions
            .term(sid)
            .expect("t")
            .host_key_decision(true)
            .expect("accept");
        wait_for("the credential prompt", || !out.prompts().is_empty()).await;
        assert!(out.prompts()[0].can_save_to_vault);

        state
            .sessions
            .credential(sid, typed("tester", &password), true)
            .expect("answer");
        wait_for("the shell", || !out.text().is_empty()).await;
        // The save happens after the authentication succeeded.
        wait_for("the connection to point at a credential", || {
            let store = state.store.lock().expect("store");
            matches!(
                store.document().node(id),
                Some(Node::Connection(c)) if matches!(c.credential, CredentialRef::Id { .. })
            )
        })
        .await;
        let credential_id = {
            let store = state.store.lock().expect("store");
            match store.document().node(id) {
                Some(Node::Connection(c)) => match c.credential {
                    CredentialRef::Id { id } => id,
                    _ => panic!("expected an id"),
                },
                _ => panic!("connection missing"),
            }
        };
        {
            let vault = state.vault.lock().expect("vault");
            let summary = vault
                .list()
                .expect("list")
                .into_iter()
                .find(|c| c.id == credential_id)
                .expect("credential saved");
            assert_eq!(summary.username, "tester");
            match vault.reveal_secret(credential_id).expect("reveal") {
                RevealedSecret::Password { password: saved } => assert_eq!(saved, password),
                RevealedSecret::SshKey { .. } => panic!("expected a password"),
            }
        }
        // Next time the vault answers: no prompt.
        match spec_for(&state, plan(&state, id).expect("plan"), false).expect("spec") {
            SessionSpec::Ssh(s) => assert!(matches!(s.target.auth, SshAuth::Password(_))),
            SessionSpec::Local(_) => panic!("expected an SSH spec"),
        }
        // The document on disk holds the reference, never the secret.
        let doc = std::fs::read_to_string(portolano_core_model::connections_path(dir.path()))
            .expect("document");
        assert!(doc.contains(&credential_id.to_string()) && !doc.contains(&password));
        assert!(!out.contains(&password));
        state.sessions.close_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_jump_chain_asks_for_each_hop_by_name_with_its_own_user_hint() {
        let Some(password) = server_password() else {
            return;
        };
        let _serial = SSH_SERIAL.lock().await;
        let dir = scratch();
        let state = state(dir.path());
        let mut jump = ssh(
            "bastion",
            "127.0.0.1",
            Some(12222),
            CredentialRef::None,
            None,
        );
        jump.username = Some("tester".into());
        // The "internal" target is the same sshd seen from its container: 127.0.0.1:22.
        let mut target = ssh(
            "inner",
            "127.0.0.1",
            Some(22),
            CredentialRef::None,
            Some(jump.id),
        );
        target.username = Some("tester".into());
        let target_id = target.id;
        upsert(&state, Node::Connection(jump));
        upsert(&state, Node::Connection(target));
        let (out, handle, sid) = open_session(&state, target_id, false);

        wait_for("the first host key prompt", || {
            out.has_tag(TAG_HOST_KEY_PROMPT)
        })
        .await;
        handle.host_key_decision(true).expect("accept");
        wait_for("the first credential prompt", || !out.prompts().is_empty()).await;
        assert_eq!(out.prompts()[0].target, "127.0.0.1:12222");
        state
            .sessions
            .credential(sid, typed("tester", &password), false)
            .expect("first hop");
        wait_for("the second host key prompt", || {
            out.0
                .lock()
                .expect("frames")
                .iter()
                .filter(|f| f[0] == TAG_HOST_KEY_PROMPT)
                .count()
                >= 2
        })
        .await;
        handle.host_key_decision(true).expect("accept");
        wait_for("the second credential prompt", || out.prompts().len() >= 2).await;
        assert_eq!(out.prompts()[1].target, "127.0.0.1:22");
        state
            .sessions
            .credential(sid, typed("tester", &password), false)
            .expect("second hop");
        wait_for("the shell", || !out.text().is_empty()).await;
        assert!(!out.contains(&password));
        state.sessions.close_all();
    }

    // Needs a unix shell (`/usr/bin/env`, `/bin/sh`): there is none on the Windows runner.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn credential_commands_reject_a_session_that_is_not_waiting_or_not_a_session() {
        let dir = scratch();
        let state = state(dir.path());
        let spec = SessionSpec::Local(LocalSpec {
            program: Some("/usr/bin/env".into()),
            args: vec!["HISTFILE=/dev/null".into(), "/bin/sh".into()],
            cwd: Some(dir.path().to_string_lossy().into_owned()),
        });
        let out = Arc::new(Collected(Mutex::default()));
        let session = Session::open(spec, 80, 24, out.sink()).expect("open");
        let sid = state
            .sessions
            .insert(SessionHandle::Term(Arc::new(TermHandle(session))))
            .expect("insert");
        // A local shell never prompts: nothing is pending.
        assert_eq!(
            state.sessions.credential_cancel(sid).error().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            state
                .sessions
                .credential(sid, typed("u", "p"), false)
                .error()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            state
                .sessions
                .credential_cancel(Uuid::new_v4())
                .error()
                .code,
            ErrorCode::NotFound
        );
        state.sessions.close_all();
    }
}
