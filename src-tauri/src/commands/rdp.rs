//! RDP session entry point, called by `fb_open` (see `fb.rs`), and the `FbSession` handle
//! behind the shared `fb_*` commands.
//!
//! `fb_open` returns as soon as the session is registered: the connection (TCP, TLS,
//! certificate prompt, login) runs in the session task and reports progress and failures
//! as `FbEvent`s on the channel (`CertPrompt`, `Resize`, `Error`, `Closed`). What can be
//! decided without the network is done here and returned as the command's error:
//! `not_found`, `invalid_input`, `vault_locked`.
//!
//! Credentials are resolved here, in the backend: `resolve_credential` picks the vault entry
//! and the secret is read from the (unlocked) vault. It goes straight into the session
//! configuration and never reaches the UI, the log or an error message. The vault is optional:
//! without a usable credential the session asks the user (`CredentialPrompt`, see `prompt.rs`),
//! pre-filling the connection's `username` and domain.

use std::sync::Arc;

use portolano_core_model::prompt::CredentialAnswer;
use portolano_core_model::{resolve_credential, ConnectionOptions, ErrorCode, Node};
use portolano_session_rdp::cert::CertStore;
use portolano_session_rdp::{EventSink, FbEvent, RdpAsk, RdpConfig, RdpCredentials, RdpSession};
use portolano_vault::RevealedSecret;
use secrecy::SecretString;
use tauri::ipc::{Channel, InvokeResponseBody};
use uuid::Uuid;

use crate::commands::model::with_document;
use crate::commands::prompt;
use crate::error::{poisoned, AppError};
use crate::state::{AppState, FbSession, SessionHandle, SessionId};

const DEFAULT_PORT: u16 = 3389;

/// What the document says about a connection, before the vault is consulted.
struct Target {
    host: String,
    port: u16,
    options_size: Option<(u16, u16)>,
    options_domain: Option<String>,
    ignore_cert: bool,
    allow_without_nla: bool,
    credential: Option<Uuid>,
    connection_name: String,
    /// The connection's optional user name: pre-fills the credential prompt.
    username: Option<String>,
}

fn read_target(state: &AppState, connection_id: Uuid) -> Result<Target, AppError> {
    with_document(state, |doc| {
        let node = doc
            .node(connection_id)
            .ok_or_else(|| AppError::not_found(format!("connection {connection_id} not found")))?;
        let Node::Connection(connection) = node else {
            return Err(AppError::invalid_input(format!(
                "{connection_id} is a folder, not a connection"
            )));
        };
        let ConnectionOptions::Rdp {
            width,
            height,
            domain,
            ignore_cert,
            allow_without_nla,
        } = &connection.options
        else {
            return Err(AppError::invalid_input(format!(
                "connection {connection_id} is not an RDP connection"
            )));
        };
        let host = connection.host.trim().to_owned();
        if host.is_empty() {
            return Err(AppError::invalid_input("the connection has no host"));
        }
        Ok(Target {
            host,
            port: connection.port.unwrap_or(DEFAULT_PORT),
            options_size: width.zip(*height),
            options_domain: domain.clone().filter(|d| !d.trim().is_empty()),
            ignore_cert: *ignore_cert,
            allow_without_nla: *allow_without_nla,
            credential: resolve_credential(doc, connection_id)?,
            connection_name: connection.name.clone(),
            username: connection.username.clone().filter(|u| !u.trim().is_empty()),
        })
    })
}

/// Reads user name, password and domain of a vault credential.
fn read_credential(
    state: &AppState,
    id: Uuid,
) -> Result<(RdpCredentials, Option<String>), AppError> {
    let vault = state.vault.lock().map_err(poisoned)?;
    let summary = vault
        .list()?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| AppError::not_found(format!("credential {id} not found")))?;
    let password = match vault.reveal_secret(id)? {
        RevealedSecret::Password { password } => SecretString::from(password),
        RevealedSecret::SshKey { .. } => {
            return Err(AppError::invalid_input(
                "the credential of this connection is an SSH key, not a password",
            ))
        }
    };
    Ok((
        RdpCredentials {
            username: summary.username,
            password,
        },
        summary.domain.filter(|d| !d.trim().is_empty()),
    ))
}

/// Session configuration of a connection. `width`/`height` are the size of the tab (used
/// unless the connection fixes a size).
fn build_config(
    state: &AppState,
    connection_id: Uuid,
    width: u16,
    height: u16,
    ask_requested: bool,
) -> Result<RdpConfig, AppError> {
    let target = read_target(state, connection_id)?;
    let (width, height, fixed_size) = match target.options_size {
        Some((w, h)) => (w, h, true),
        None => (width, height, false),
    };
    let mut config = RdpConfig::new(
        target.host,
        target.port,
        width,
        height,
        CertStore::in_data_dir(&state.data_dir),
    );
    config.fixed_size = fixed_size;
    config.ignore_cert = target.ignore_cert;
    config.allow_without_nla = target.allow_without_nla;
    config.domain = target.options_domain;
    match prompt::vault_credential(state, target.credential, ask_requested)? {
        Some(id) => {
            let (credentials, credential_domain) = read_credential(state, id)?;
            // The domain of the credential wins over the connection's option.
            if credential_domain.is_some() {
                config.domain = credential_domain;
            }
            config.credentials = Some(credentials);
        }
        // No usable credential: the session asks before it connects.
        None => {
            config.ask = Some(RdpAsk {
                username_hint: target.username,
                hooks: prompt::hooks(state, connection_id, target.connection_name),
            });
        }
    }
    Ok(config)
}

/// Registers a session for `config` that reports to `sink`.
fn start_session(
    state: &AppState,
    config: RdpConfig,
    sink: EventSink,
) -> Result<SessionId, AppError> {
    tracing::info!(
        host = %config.host,
        port = config.port,
        credentials = config.credentials.is_some(),
        asks = config.ask.is_some(),
        "opening an RDP session"
    );
    let store = config.cert_store.clone();
    let (host, port) = (config.host.clone(), config.port);
    let session = RdpSession::start(config, sink);
    state
        .sessions
        .insert(SessionHandle::Fb(Arc::new(RdpFbSession {
            session,
            store,
            host,
            port,
        })))
}

pub async fn open(
    state: &AppState,
    connection_id: Uuid,
    width: u16,
    height: u16,
    on_event: Channel<InvokeResponseBody>,
    ask_requested: bool,
) -> Result<SessionId, AppError> {
    let config = build_config(state, connection_id, width, height, ask_requested)?;
    let sink: EventSink = Arc::new(move |event: FbEvent| {
        match on_event.send(InvokeResponseBody::Raw(event.encode())) {
            Ok(()) => true,
            Err(e) => {
                // The webview is gone (reload or window closed): nobody can see the session.
                tracing::debug!("RDP event channel closed: {e}");
                false
            }
        }
    });
    start_session(state, config, sink)
}

/// The handle registered in the `SessionManager`.
struct RdpFbSession {
    session: RdpSession,
    store: CertStore,
    host: String,
    port: u16,
}

impl FbSession for RdpFbSession {
    fn pointer(&self, x: u16, y: u16, buttons: u8, wheel: i16) -> Result<(), AppError> {
        Ok(self.session.pointer(x, y, buttons, wheel)?)
    }

    fn key(&self, code: &str, down: bool) -> Result<(), AppError> {
        Ok(self.session.key(code, down)?)
    }

    fn resize(&self, width: u16, height: u16) -> Result<(), AppError> {
        Ok(self.session.resize(width, height)?)
    }

    fn set_visible(&self, visible: bool) -> Result<(), AppError> {
        Ok(self.session.set_visible(visible)?)
    }

    fn cert_decision(&self, accept: bool) -> Result<(), AppError> {
        Ok(self.session.cert_decision(accept)?)
    }

    fn forget_certificate(&self) -> Result<(), AppError> {
        let removed = self.store.forget(&self.host, self.port).map_err(|e| {
            AppError::new(
                ErrorCode::Io,
                format!("cannot remove the stored certificate: {e}"),
            )
        })?;
        tracing::info!(host = %self.host, port = self.port, removed, "stored RDP certificate removed");
        Ok(())
    }

    fn credential(&self, answer: CredentialAnswer, save_to_vault: bool) -> Result<(), AppError> {
        Ok(self.session.credential(answer, save_to_vault)?)
    }

    fn credential_cancel(&self) -> Result<(), AppError> {
        Ok(self.session.credential_cancel()?)
    }

    fn close(&self) {
        self.session.close();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use portolano_core_model::{Connection, CredentialRef, ErrorCode, Folder};
    use portolano_vault::{CredentialInput, KdfParams, SecretInput, Vault};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp6/app");
            let dir = root.join(name);
            // Start clean; a missing directory is fine.
            if let Err(e) = fs::remove_dir_all(&dir) {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            fs::create_dir_all(&dir).expect("create dir");
            Self(dir)
        }
    }

    /// The xrdp keeps one desktop per user and a new login takes it over: tests that log in
    /// run one at a time.
    static RDP_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn rdp_connection(id: u128, parent: Option<u128>, credential: CredentialRef) -> Node {
        Node::Connection(Connection {
            id: u(id),
            parent_id: parent.map(u),
            name: format!("rdp{id}"),
            host: "127.0.0.1".into(),
            port: Some(13389),
            credential,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: Some("CONNDOM".into()),
                ignore_cert: false,
                allow_without_nla: false,
            },
        })
    }

    fn put(state: &AppState, node: Node) {
        let mut store = state.store.lock().expect("store");
        store.ensure_loaded().expect("load");
        store.upsert_node(node).expect("upsert");
    }

    fn unlocked_vault(
        state: &AppState,
        dir: &TempDir,
        credentials: &[(u128, &str, &str, Option<&str>)],
    ) {
        let mut vault = Vault::with_kdf_params(
            dir.0.join("vault.bin"),
            KdfParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
        )
        .expect("vault");
        vault
            .create(SecretString::from("master-for-tests".to_owned()))
            .expect("create");
        for (n, user, password, domain) in credentials {
            vault
                .upsert(CredentialInput {
                    id: Some(u(*n)),
                    name: format!("cred{n}"),
                    username: (*user).into(),
                    domain: domain.map(str::to_owned),
                    secret: SecretInput::Password {
                        password: SecretString::from((*password).to_owned()),
                    },
                })
                .expect("upsert credential");
        }
        *state.vault.lock().expect("vault") = vault;
    }

    #[test]
    fn config_resolves_the_inherited_credential_in_the_backend() {
        use secrecy::ExposeSecret;
        let dir = TempDir::new("config");
        let state = AppState::new(dir.0.clone());
        put(
            &state,
            Node::Folder(Folder {
                id: u(1),
                parent_id: None,
                name: "f".into(),
                credential: CredentialRef::Id { id: u(100) },
            }),
        );
        put(&state, rdp_connection(2, Some(1), CredentialRef::Inherit));
        unlocked_vault(&state, &dir, &[(100, "alice", "pw-123", None)]);

        let config = build_config(&state, u(2), 1400, 900, false).expect("config");
        let credentials = config.credentials.as_ref().expect("credentials");
        assert_eq!(credentials.username, "alice");
        assert_eq!(credentials.password.expose_secret(), "pw-123");
        assert_eq!(config.domain.as_deref(), Some("CONNDOM"));
        assert_eq!(
            (config.width, config.height, config.fixed_size),
            (1400, 900, false)
        );
        assert_eq!((config.host.as_str(), config.port), ("127.0.0.1", 13389));
        // The debug output of the configuration is safe to log.
        assert!(!format!("{config:?}").contains("pw-123"));
    }

    #[test]
    fn credential_domain_wins_and_fixed_size_comes_from_the_connection() {
        let dir = TempDir::new("domain");
        let state = AppState::new(dir.0.clone());
        let mut node = rdp_connection(2, None, CredentialRef::Id { id: u(100) });
        if let Node::Connection(c) = &mut node {
            c.options = ConnectionOptions::Rdp {
                width: Some(1024),
                height: Some(768),
                domain: None,
                ignore_cert: true,
                allow_without_nla: true,
            };
            c.port = None;
        }
        put(&state, node);
        unlocked_vault(&state, &dir, &[(100, "bob", "x", Some("CORP"))]);
        let config = build_config(&state, u(2), 1400, 900, false).expect("config");
        assert_eq!(config.domain.as_deref(), Some("CORP"));
        assert_eq!(
            (config.width, config.height, config.fixed_size),
            (1024, 768, true)
        );
        assert_eq!(config.port, 3389);
        assert!(config.ignore_cert);
        assert!(
            config.allow_without_nla,
            "the connection's option reaches the session"
        );
    }

    #[test]
    fn nla_is_required_unless_the_connection_says_otherwise() {
        let dir = TempDir::new("nla-default");
        let state = AppState::new(dir.0.clone());
        put(
            &state,
            rdp_connection(2, None, CredentialRef::Id { id: u(100) }),
        );
        unlocked_vault(&state, &dir, &[(100, "bob", "x", None)]);
        let config = build_config(&state, u(2), 1400, 900, false).expect("config");
        assert!(!config.allow_without_nla);
        assert!(!config.ignore_cert);
    }

    #[test]
    fn open_errors_are_stable_codes() {
        let dir = TempDir::new("errors");
        let state = AppState::new(dir.0.clone());
        let code = |r: Result<RdpConfig, AppError>| r.expect_err("error").code;

        assert_eq!(
            code(build_config(&state, u(99), 800, 600, false)),
            ErrorCode::NotFound
        );

        put(
            &state,
            Node::Folder(Folder {
                id: u(1),
                parent_id: None,
                name: "f".into(),
                credential: CredentialRef::None,
            }),
        );
        assert_eq!(
            code(build_config(&state, u(1), 800, 600, false)),
            ErrorCode::InvalidInput
        );

        // A credential is referenced but no vault was ever created: dangling, so the
        // session asks (WP11; this used to be `vault_locked`).
        put(
            &state,
            rdp_connection(2, None, CredentialRef::Id { id: u(100) }),
        );
        let config = build_config(&state, u(2), 800, 600, false).expect("config");
        assert!(config.credentials.is_none() && config.ask.is_some());

        // A vault that exists but is locked keeps the unlock flow ...
        unlocked_vault(&state, &dir, &[(100, "u", "p", None)]);
        state.vault.lock().expect("vault").lock().expect("lock");
        assert_eq!(
            code(build_config(&state, u(2), 800, 600, false)),
            ErrorCode::VaultLocked
        );
        // ... with "enter manually for this session" as the alternative.
        let config = build_config(&state, u(2), 800, 600, true).expect("manual");
        assert!(config.credentials.is_none() && config.ask.is_some());

        // Unlocked but the credential does not exist: dangling, so the session asks
        // (this used to be `not_found`).
        let second = TempDir::new("errors-second-vault");
        unlocked_vault(&state, &second, &[]);
        let config = build_config(&state, u(2), 800, 600, false).expect("config");
        assert!(config.ask.is_some());

        // No credential at all: the session asks; it no longer falls back to the server's
        // own login screen and never needs a vault.
        put(&state, rdp_connection(3, None, CredentialRef::None));
        let config = build_config(&state, u(3), 800, 600, false).expect("config");
        assert!(config.credentials.is_none() && config.ask.is_some());
    }

    #[test]
    fn the_connections_username_prefills_the_prompt() {
        let dir = TempDir::new("username");
        let state = AppState::new(dir.0.clone());
        let mut node = rdp_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            c.username = Some("  alice ".into());
        }
        put(&state, node);
        let config = build_config(&state, u(2), 800, 600, false).expect("config");
        let ask = config.ask.expect("asks");
        // Whitespace around it is dropped by the prompt, the hint is the stored value.
        assert_eq!(ask.username_hint.as_deref().map(str::trim), Some("alice"));
        // Without a stored user name there is nothing to pre-fill.
        put(&state, rdp_connection(3, None, CredentialRef::None));
        let config = build_config(&state, u(3), 800, 600, false).expect("config");
        assert_eq!(config.ask.expect("asks").username_hint, None);
    }

    #[test]
    fn a_non_rdp_connection_is_rejected() {
        let dir = TempDir::new("kind");
        let state = AppState::new(dir.0.clone());
        let mut node = rdp_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            c.options = ConnectionOptions::Vnc { view_only: false };
        }
        put(&state, node);
        let err = build_config(&state, u(2), 800, 600, false).expect_err("error");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    /// The whole command path against the test xrdp: document -> vault -> session manager ->
    /// `FbSession` -> channel bytes -> `session_close`. Skips without the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_and_close_through_the_session_manager() {
        let password = fs::read_to_string(crate::commands::test_env_path())
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
                    .map(|v| v.trim().trim_matches('"').to_owned())
            });
        let Some(password) = password else {
            eprintln!("SKIP: dev/keys/test.env has no TEST_PASSWORD");
            return;
        };
        let reachable = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 13389)),
        )
        .await
        .is_ok_and(|r| r.is_ok());
        if !reachable {
            eprintln!("SKIP: no RDP server on 127.0.0.1:13389");
            return;
        }

        let _serial = RDP_SERIAL.lock().await;
        let dir = TempDir::new("session");
        let state = AppState::new(dir.0.clone());
        let mut node = rdp_connection(2, None, CredentialRef::Id { id: u(100) });
        if let Node::Connection(c) = &mut node {
            c.options = ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: false,
                allow_without_nla: true,
            };
        }
        put(&state, node);
        unlocked_vault(&state, &dir, &[(100, "tester", &password, None)]);

        let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let sink_bodies = bodies.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink_bodies.lock().expect("bodies").push(bytes);
            }
            Ok(())
        });
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        assert_eq!(state.sessions.len(), 1);
        let fb = state.sessions.fb(id).expect("fb session");

        // First contact: certificate prompt (tag 3), then accept it.
        let wait_for = |tag: u8, limit: Duration| {
            let bodies = bodies.clone();
            async move {
                let deadline = Instant::now() + limit;
                while Instant::now() < deadline {
                    if bodies
                        .lock()
                        .expect("bodies")
                        .iter()
                        .any(|b| b.first() == Some(&tag))
                    {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                false
            }
        };
        assert!(wait_for(3, Duration::from_secs(20)).await, "no CertPrompt");
        fb.cert_decision(true).expect("decision");
        assert!(wait_for(0, Duration::from_secs(60)).await, "no Resize");
        assert!(wait_for(1, Duration::from_secs(60)).await, "no Rect");
        fb.pointer(10, 10, 0, 0).expect("pointer");
        fb.key("ShiftLeft", true).expect("key");
        fb.key("ShiftLeft", false).expect("key");
        fb.set_visible(false).expect("hide");
        fb.set_visible(true).expect("show");
        fb.resize(1000, 700).expect("resize");

        // No credential or secret ever appears in what the UI receives.
        assert!(bodies
            .lock()
            .expect("bodies")
            .iter()
            .filter(|b| b.first().is_some_and(|t| *t >= 3))
            .all(|b| !String::from_utf8_lossy(b).contains(&password)));

        state.sessions.close(id).expect("close");
        assert!(state.sessions.is_empty());
        // The handle is gone: later calls report a missing session.
        assert_eq!(
            state.sessions.fb(id).err().expect("gone").code,
            ErrorCode::NotFound
        );
    }

    /// The whole command path with no vault and no credential anywhere: `fb_open` ->
    /// credential prompt -> `session_credential` -> certificate prompt -> frames.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_without_a_vault_prompts_then_connects_and_sends_no_secret() {
        let password = fs::read_to_string(crate::commands::test_env_path())
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("TEST_PASSWORD="))
                    .map(|v| v.trim().trim_matches('"').to_owned())
            });
        let reachable = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 13389)),
        )
        .await
        .is_ok_and(|r| r.is_ok());
        let (Some(password), true) = (password, reachable) else {
            eprintln!("SKIP: no RDP server on 127.0.0.1:13389 or no dev/keys/test.env");
            return;
        };

        let _serial = RDP_SERIAL.lock().await;
        let dir = TempDir::new("no-vault");
        let state = AppState::new(dir.0.clone());
        let mut node = rdp_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            c.username = Some("tester".into());
            c.options = ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: false,
                allow_without_nla: true,
            };
        }
        put(&state, node);

        let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let sink_bodies = bodies.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink_bodies.lock().expect("bodies").push(bytes);
            }
            Ok(())
        });
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        let fb = state.sessions.fb(id).expect("fb session");

        let wait_for = |tag: u8, limit: Duration| {
            let bodies = bodies.clone();
            async move {
                let deadline = Instant::now() + limit;
                while Instant::now() < deadline {
                    if bodies
                        .lock()
                        .expect("bodies")
                        .iter()
                        .any(|b| b.first() == Some(&tag))
                    {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                false
            }
        };
        assert!(
            wait_for(6, Duration::from_secs(20)).await,
            "no CredentialPrompt"
        );
        let prompt: portolano_core_model::prompt::CredentialPrompt = {
            let bodies = bodies.lock().expect("bodies");
            let frame = bodies.iter().find(|b| b[0] == 6).expect("frame");
            serde_json::from_slice(&frame[1..]).expect("prompt json")
        };
        assert_eq!(prompt.username_hint.as_deref(), Some("tester"));
        assert_eq!(prompt.target, "127.0.0.1:13389");
        assert!(!prompt.can_save_to_vault);
        // Nothing but the prompt happened before the answer: no certificate, no pixels.
        assert!(bodies.lock().expect("bodies").iter().all(|b| b[0] == 6));

        state
            .sessions
            .credential(
                id,
                CredentialAnswer {
                    username: "tester".into(),
                    password: Some(SecretString::from(password.clone())),
                    key_path: None,
                    passphrase: None,
                    domain: None,
                },
                false,
            )
            .expect("answer");
        assert!(wait_for(3, Duration::from_secs(20)).await, "no CertPrompt");
        fb.cert_decision(true).expect("decision");
        assert!(wait_for(0, Duration::from_secs(60)).await, "no Resize");
        assert!(wait_for(1, Duration::from_secs(60)).await, "no Rect");

        assert!(bodies
            .lock()
            .expect("bodies")
            .iter()
            .filter(|b| b[0] != 1 && b[0] != 2)
            .all(|b| !String::from_utf8_lossy(b).contains(&password)));
        assert!(!dir.0.join("vault.bin").exists());
        state.sessions.close(id).expect("close");
    }
}
