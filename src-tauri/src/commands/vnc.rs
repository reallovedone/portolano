//! VNC session entry point, called by `fb_open` (see `fb.rs`), and the `FbSession` handle
//! behind the shared `fb_*` commands.
//!
//! `fb_open` returns as soon as the session is registered: the connection (TCP, handshake,
//! password prompt) runs in the session task and reports progress and failures as `FbEvent`s
//! on the channel (`CredentialPrompt`, `Resize`, `Error`, `Closed`). What can be decided
//! without the network is done here and returned as the command's error: `not_found`,
//! `invalid_input`, `vault_locked`.
//!
//! Credentials are resolved here, in the backend, exactly like RDP: `resolve_credential` picks
//! the vault entry (an SSH-key credential is `invalid_input`). Its password serves VNC
//! Authentication and VeNCrypt Plain; its user name serves VeNCrypt Plain only (the
//! placeholder `SAVED_USERNAME` that a password-only prompt saves does not count, then
//! `Connection.username` is used). Secrets go straight into the session configuration and
//! never reach the UI, the log or an error message. The vault is optional: without a usable
//! credential the session asks, and only when the server actually wants one (security type
//! None needs no prompt at all). The desktop size is dictated by the server: the tab size
//! passed to `fb_open` is ignored.
//!
//! Encrypted sessions (VeNCrypt over TLS) pin the server certificate in `vnc_known_certs` in
//! the data directory; `fb_cert_decision` answers the `CertPrompt` of the session. Plain VNC
//! traffic is not encrypted, and the session says so with `FbEvent::Security`.

use std::sync::Arc;

use portolano_core_model::prompt::CredentialAnswer;
use portolano_core_model::{resolve_credential, ConnectionOptions, ErrorCode, Node};
use portolano_session_vnc::cert::{CertStore, PlainStore};
use portolano_session_vnc::{EventSink, FbEvent, VncAsk, VncConfig, VncSession, SAVED_USERNAME};
use portolano_vault::RevealedSecret;
use secrecy::SecretString;
use tauri::ipc::{Channel, InvokeResponseBody};
use uuid::Uuid;

use crate::commands::model::with_document;
use crate::commands::prompt;
use crate::error::{poisoned, AppError};
use crate::state::{AppState, FbSession, SessionHandle, SessionId};

const DEFAULT_PORT: u16 = 5900;

/// What the document says about a connection, before the vault is consulted.
struct Target {
    host: String,
    port: u16,
    view_only: bool,
    credential: Option<Uuid>,
    /// `Connection.username`: the user name of VeNCrypt Plain when the vault has none.
    username: Option<String>,
    connection_name: String,
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
        let ConnectionOptions::Vnc { view_only } = &connection.options else {
            return Err(AppError::invalid_input(format!(
                "connection {connection_id} is not a VNC connection"
            )));
        };
        let host = connection.host.trim().to_owned();
        if host.is_empty() {
            return Err(AppError::invalid_input("the connection has no host"));
        }
        Ok(Target {
            host,
            port: connection.port.unwrap_or(DEFAULT_PORT),
            view_only: *view_only,
            credential: resolve_credential(doc, connection_id)?,
            username: connection.username.clone().filter(|u| !u.trim().is_empty()),
            connection_name: connection.name.clone(),
        })
    })
}

/// Reads user name and password of a vault credential. The user name is only used by
/// VeNCrypt Plain, and not when it is the placeholder of a password-only credential.
fn read_credential(state: &AppState, id: Uuid) -> Result<(Option<String>, SecretString), AppError> {
    let vault = state.vault.lock().map_err(poisoned)?;
    let summary = vault
        .list()?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| AppError::not_found(format!("credential {id} not found")))?;
    let username =
        Some(summary.username.trim().to_owned()).filter(|u| !u.is_empty() && u != SAVED_USERNAME);
    match vault.reveal_secret(id)? {
        RevealedSecret::Password { password } => Ok((username, SecretString::from(password))),
        RevealedSecret::SshKey { .. } => Err(AppError::invalid_input(
            "the credential of this connection is an SSH key, not a password",
        )),
    }
}

/// Session configuration of a connection.
fn build_config(
    state: &AppState,
    connection_id: Uuid,
    ask_requested: bool,
) -> Result<VncConfig, AppError> {
    let target = read_target(state, connection_id)?;
    let mut config = VncConfig::new(target.host, target.port);
    config.view_only = target.view_only;
    config.cert_store = Some(CertStore::in_data_dir(&state.data_dir));
    config.plain_store = Some(PlainStore::in_data_dir(&state.data_dir));
    config.username = target.username;
    match prompt::vault_credential(state, target.credential, ask_requested)? {
        Some(id) => {
            let (username, password) = read_credential(state, id)?;
            // The vault's user name wins over the connection's.
            if username.is_some() {
                config.username = username;
            }
            config.password = Some(password);
        }
        // No usable credential: the session asks if (and only if) the server wants one.
        None => {
            config.ask = Some(VncAsk {
                hooks: prompt::hooks(state, connection_id, target.connection_name),
            });
        }
    }
    Ok(config)
}

/// Registers a session for `config` that reports to `sink`.
fn start_session(
    state: &AppState,
    config: VncConfig,
    sink: EventSink,
) -> Result<SessionId, AppError> {
    tracing::info!(
        host = %config.host,
        port = config.port,
        username = config.username.is_some(),
        password = config.password.is_some(),
        asks = config.ask.is_some(),
        view_only = config.view_only,
        "opening a VNC session"
    );
    let store = config.cert_store.clone();
    let (host, port) = (config.host.clone(), config.port);
    let session = VncSession::start(config, sink);
    state
        .sessions
        .insert(SessionHandle::Fb(Arc::new(VncFbSession {
            session,
            store,
            host,
            port,
        })))
}

/// `width`/`height` (the tab size) are ignored: the server dictates the desktop size.
pub async fn open(
    state: &AppState,
    connection_id: Uuid,
    _width: u16,
    _height: u16,
    on_event: Channel<InvokeResponseBody>,
    ask_requested: bool,
) -> Result<SessionId, AppError> {
    let config = build_config(state, connection_id, ask_requested)?;
    let sink: EventSink = Arc::new(move |event: FbEvent| {
        match on_event.send(InvokeResponseBody::Raw(event.encode())) {
            Ok(()) => true,
            Err(e) => {
                // The webview is gone (reload or window closed): nobody can see the session.
                tracing::debug!("VNC event channel closed: {e}");
                false
            }
        }
    });
    start_session(state, config, sink)
}

fn forget(store: Option<&CertStore>, host: &str, port: u16) -> Result<(), AppError> {
    let Some(store) = store else {
        return Ok(());
    };
    store
        .forget(host, port)
        .map(|removed| {
            tracing::info!(host, port, removed, "stored VNC certificate removed");
        })
        .map_err(|e| {
            AppError::new(
                ErrorCode::Io,
                format!("cannot remove the stored certificate: {e}"),
            )
        })
}

/// The handle registered in the `SessionManager`.
struct VncFbSession {
    session: VncSession,
    store: Option<CertStore>,
    host: String,
    port: u16,
}

impl FbSession for VncFbSession {
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

    fn unencrypted_decision(&self, allow: bool) -> Result<(), AppError> {
        Ok(self.session.unencrypted_decision(allow)?)
    }

    fn forget_certificate(&self) -> Result<(), AppError> {
        forget(self.store.as_ref(), &self.host, self.port)
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
    use secrecy::ExposeSecret;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp10b/app");
            let dir = root.join(name);
            // Start clean; a missing directory is fine.
            if let Err(e) = fs::remove_dir_all(&dir) {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            fs::create_dir_all(&dir).expect("create dir");
            Self(dir)
        }
    }

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn vnc_connection(id: u128, parent: Option<u128>, credential: CredentialRef) -> Node {
        Node::Connection(Connection {
            id: u(id),
            parent_id: parent.map(u),
            name: format!("vnc{id}"),
            host: "127.0.0.1".into(),
            port: Some(15900),
            credential,
            username: None,
            tags: vec![],
            notes: String::new(),
            options: ConnectionOptions::Vnc { view_only: false },
        })
    }

    fn put(state: &AppState, node: Node) {
        let mut store = state.store.lock().expect("store");
        store.ensure_loaded().expect("load");
        store.upsert_node(node).expect("upsert");
    }

    fn unlocked_vault(state: &AppState, dir: &TempDir, password_credentials: &[(u128, &str)]) {
        let with_user: Vec<(u128, &str, &str)> = password_credentials
            .iter()
            .map(|(n, p)| (*n, "ignored-by-vnc", *p))
            .collect();
        unlocked_vault_with_users(state, dir, &with_user);
    }

    fn unlocked_vault_with_users(
        state: &AppState,
        dir: &TempDir,
        credentials: &[(u128, &str, &str)],
    ) {
        if let Err(e) = fs::remove_file(dir.0.join("vault.bin")) {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        }
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
        for (n, user, password) in credentials {
            vault
                .upsert(CredentialInput {
                    id: Some(u(*n)),
                    name: format!("cred{n}"),
                    username: (*user).into(),
                    domain: None,
                    secret: SecretInput::Password {
                        password: SecretString::from((*password).to_owned()),
                    },
                })
                .expect("upsert credential");
        }
        *state.vault.lock().expect("vault") = vault;
    }

    fn test_password() -> Option<String> {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        fs::read_to_string(repo.join("dev/keys/test.env"))
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("VNC_PASSWORD="))
                    .map(|v| v.trim().trim_matches('"').to_owned())
            })
    }

    async fn server_up() -> bool {
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 15900)),
        )
        .await
        .is_ok_and(|r| r.is_ok())
    }

    /// Runs the real channel path: the sink collects the raw bodies the UI would receive.
    type Bodies = Arc<Mutex<Vec<Vec<u8>>>>;

    fn collecting_channel() -> (Channel<InvokeResponseBody>, Bodies) {
        let bodies: Bodies = Arc::default();
        let sink_bodies = bodies.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink_bodies.lock().expect("bodies").push(bytes);
            }
            Ok(())
        });
        (channel, bodies)
    }

    async fn wait_for(bodies: &Bodies, tag: u8, limit: Duration) -> bool {
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

    #[test]
    fn config_resolves_the_inherited_password_in_the_backend() {
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
        put(&state, vnc_connection(2, Some(1), CredentialRef::Inherit));
        unlocked_vault(&state, &dir, &[(100, "pw-12345")]);

        let config = build_config(&state, u(2), false).expect("config");
        let password = config.password.as_ref().expect("password");
        assert_eq!(password.expose_secret(), "pw-12345");
        assert!(config.ask.is_none());
        assert!(!config.view_only);
        assert_eq!((config.host.as_str(), config.port), ("127.0.0.1", 15900));
        // The debug output of the configuration is safe to log.
        assert!(!format!("{config:?}").contains("pw-12345"));
    }

    #[test]
    fn view_only_and_the_default_port_come_from_the_connection() {
        let dir = TempDir::new("options");
        let state = AppState::new(dir.0.clone());
        let mut node = vnc_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            c.options = ConnectionOptions::Vnc { view_only: true };
            c.port = None;
        }
        put(&state, node);
        let config = build_config(&state, u(2), false).expect("config");
        assert!(config.view_only);
        assert_eq!(config.port, 5900);
    }

    #[test]
    fn open_errors_are_stable_codes() {
        let dir = TempDir::new("errors");
        let state = AppState::new(dir.0.clone());
        let code = |r: Result<VncConfig, AppError>| r.expect_err("error").code;

        assert_eq!(
            code(build_config(&state, u(99), false)),
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
            code(build_config(&state, u(1), false)),
            ErrorCode::InvalidInput
        );

        // A credential is referenced but no vault was ever created: dangling, so the
        // session asks.
        put(
            &state,
            vnc_connection(2, None, CredentialRef::Id { id: u(100) }),
        );
        let config = build_config(&state, u(2), false).expect("config");
        assert!(config.password.is_none() && config.ask.is_some());

        // A vault that exists but is locked keeps the unlock flow ...
        unlocked_vault(&state, &dir, &[(100, "p")]);
        state.vault.lock().expect("vault").lock().expect("lock");
        assert_eq!(
            code(build_config(&state, u(2), false)),
            ErrorCode::VaultLocked
        );
        // ... with "enter manually for this session" as the alternative.
        let config = build_config(&state, u(2), true).expect("manual");
        assert!(config.password.is_none() && config.ask.is_some());

        // No credential at all: the session asks (only if the server wants a password).
        put(&state, vnc_connection(3, None, CredentialRef::None));
        let config = build_config(&state, u(3), false).expect("config");
        assert!(config.password.is_none() && config.ask.is_some());
    }

    #[test]
    fn a_credential_holding_an_ssh_key_is_invalid_input() {
        use portolano_vault::SecretInput;
        let dir = TempDir::new("sshkey");
        let state = AppState::new(dir.0.clone());
        put(
            &state,
            vnc_connection(2, None, CredentialRef::Id { id: u(100) }),
        );
        unlocked_vault(&state, &dir, &[]);
        state
            .vault
            .lock()
            .expect("vault")
            .upsert(CredentialInput {
                id: Some(u(100)),
                name: "key".into(),
                username: "u".into(),
                domain: None,
                secret: SecretInput::SshKey {
                    private_key_pem: SecretString::from("not a real key".to_owned()),
                    passphrase: None,
                },
            })
            .expect("upsert key");
        let err = build_config(&state, u(2), false).expect_err("error");
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(!err.message.contains("not a real key"));
    }

    #[test]
    fn a_non_vnc_connection_is_rejected() {
        let dir = TempDir::new("kind");
        let state = AppState::new(dir.0.clone());
        let mut node = vnc_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            c.options = ConnectionOptions::Rdp {
                width: None,
                height: None,
                domain: None,
                ignore_cert: false,
                allow_without_nla: false,
            };
        }
        put(&state, node);
        let err = build_config(&state, u(2), false).expect_err("error");
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    /// The whole command path against the test server: document -> vault -> session manager ->
    /// `FbSession` -> channel bytes -> `session_close`. Skips without the server.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_and_close_through_the_session_manager() {
        let Some(password) = test_password() else {
            eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
            return;
        };
        if !server_up().await {
            eprintln!("SKIP: no VNC server on 127.0.0.1:15900");
            return;
        }
        let dir = TempDir::new("session");
        agree_to_plain_vnc(&dir);
        let state = AppState::new(dir.0.clone());
        put(
            &state,
            vnc_connection(2, None, CredentialRef::Id { id: u(100) }),
        );
        unlocked_vault(&state, &dir, &[(100, &password)]);

        let (channel, bodies) = collecting_channel();
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        assert_eq!(state.sessions.len(), 1);
        let fb = state.sessions.fb(id).expect("fb session");

        assert!(
            wait_for(&bodies, 0, Duration::from_secs(20)).await,
            "no Resize"
        );
        assert!(
            wait_for(&bodies, 1, Duration::from_secs(20)).await,
            "no Rect"
        );
        // Plain VNC says it is not encrypted (tag 7), before the size of the desktop.
        assert_eq!(
            bodies.lock().expect("bodies")[0],
            [&[7u8][..], br#"{"encrypted":false}"#].concat()
        );
        // The size is the server's, not the tab's.
        assert_eq!(
            bodies.lock().expect("bodies")[1],
            [&[0u8][..], &1280u16.to_le_bytes(), &800u16.to_le_bytes()].concat()
        );
        fb.pointer(10, 10, 0, 0).expect("pointer");
        fb.key("ShiftLeft", true).expect("key");
        fb.key("ShiftLeft", false).expect("key");
        fb.set_visible(false).expect("hide");
        fb.set_visible(true).expect("show");
        fb.resize(1000, 700).expect("resize");
        // A decision with no prompt pending is dropped, not an error.
        fb.cert_decision(true).expect("decision without a prompt");

        // No password ever appears in what the UI receives (pixel messages excluded).
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
    /// password prompt -> wrong answer -> prompt again -> `session_credential` -> frames.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_without_a_vault_prompts_then_connects_and_sends_no_secret() {
        let Some(password) = test_password() else {
            eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
            return;
        };
        if !server_up().await {
            eprintln!("SKIP: no VNC server on 127.0.0.1:15900");
            return;
        }
        let dir = TempDir::new("no-vault");
        agree_to_plain_vnc(&dir);
        let state = AppState::new(dir.0.clone());
        put(&state, vnc_connection(2, None, CredentialRef::None));

        let (channel, bodies) = collecting_channel();
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        assert!(
            wait_for(&bodies, 6, Duration::from_secs(20)).await,
            "no prompt"
        );
        let prompt: portolano_core_model::prompt::CredentialPrompt = {
            let bodies = bodies.lock().expect("bodies");
            let frame = bodies.iter().find(|b| b[0] == 6).expect("frame");
            serde_json::from_slice(&frame[1..]).expect("prompt json")
        };
        assert_eq!(
            prompt.kind,
            portolano_core_model::prompt::PromptKind::Password
        );
        assert_eq!(prompt.username_hint, None);
        assert_eq!(prompt.target, "127.0.0.1:15900");
        assert!(!prompt.can_save_to_vault);
        // Nothing but the prompt happened before the answer.
        assert!(bodies.lock().expect("bodies").iter().all(|b| b[0] == 6));

        let answer = |p: &str| CredentialAnswer {
            username: String::new(),
            password: Some(SecretString::from(p.to_owned())),
            key_path: None,
            passphrase: None,
            domain: None,
        };
        state
            .sessions
            .credential(id, answer("definitely-wrong"), false)
            .expect("wrong answer");
        // The second prompt carries the error.
        let deadline = Instant::now() + Duration::from_secs(20);
        while bodies
            .lock()
            .expect("bodies")
            .iter()
            .filter(|b| b[0] == 6)
            .count()
            < 2
            && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let second: portolano_core_model::prompt::CredentialPrompt = {
            let bodies = bodies.lock().expect("bodies");
            let frames: Vec<_> = bodies.iter().filter(|b| b[0] == 6).collect();
            serde_json::from_slice(&frames.get(1).expect("second prompt")[1..]).expect("json")
        };
        assert!(second.error.is_some());

        state
            .sessions
            .credential(id, answer(&password), false)
            .expect("answer");
        assert!(
            wait_for(&bodies, 0, Duration::from_secs(20)).await,
            "no Resize"
        );
        assert!(
            wait_for(&bodies, 1, Duration::from_secs(20)).await,
            "no Rect"
        );
        assert!(bodies
            .lock()
            .expect("bodies")
            .iter()
            .filter(|b| b[0] != 1 && b[0] != 2)
            .all(|b| {
                let text = String::from_utf8_lossy(b);
                !text.contains(&password) && !text.contains("definitely-wrong")
            }));
        assert!(!dir.0.join("vault.bin").exists());
        state.sessions.close(id).expect("close");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unreachable_server_is_reported_on_the_channel() {
        let dir = TempDir::new("unreachable");
        let state = AppState::new(dir.0.clone());
        let mut node = vnc_connection(2, None, CredentialRef::None);
        if let Node::Connection(c) = &mut node {
            // A loopback port nobody listens on.
            let port = {
                let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
                l.local_addr().expect("addr").port()
            };
            c.port = Some(port);
        }
        put(&state, node);
        let (channel, bodies) = collecting_channel();
        let id = open(&state, u(2), 800, 600, channel, false)
            .await
            .expect("open");
        assert!(
            wait_for(&bodies, 5, Duration::from_secs(15)).await,
            "no Error"
        );
        let error: portolano_core_model::AppError = {
            let bodies = bodies.lock().expect("bodies");
            let frame = bodies.iter().find(|b| b[0] == 5).expect("frame");
            serde_json::from_slice(&frame[1..]).expect("error json")
        };
        assert_eq!(error.code, ErrorCode::HostUnreachable);
        state.sessions.close(id).expect("close");
    }

    #[test]
    fn user_name_and_certificate_store_come_from_the_right_places() {
        let dir = TempDir::new("username");
        let state = AppState::new(dir.0.clone());
        let mut node = vnc_connection(2, None, CredentialRef::Id { id: u(100) });
        if let Node::Connection(c) = &mut node {
            c.username = Some("from-connection".into());
        }
        put(&state, node);

        // The vault's user name wins ...
        unlocked_vault_with_users(&state, &dir, &[(100, "from-vault", "pw")]);
        let config = build_config(&state, u(2), false).expect("config");
        assert_eq!(config.username.as_deref(), Some("from-vault"));
        assert_eq!(
            config.cert_store.as_ref().map(|s| s.path().to_path_buf()),
            Some(dir.0.join("vnc_known_certs"))
        );
        assert!(!format!("{config:?}").contains("from-vault"));

        // ... unless it is the placeholder of a password-only credential.
        unlocked_vault_with_users(&state, &dir, &[(100, SAVED_USERNAME, "pw")]);
        let config = build_config(&state, u(2), false).expect("config");
        assert_eq!(config.username.as_deref(), Some("from-connection"));
        assert!(config.password.is_some());

        // Asking: the connection's user name is the hint.
        put(&state, {
            let mut n = vnc_connection(3, None, CredentialRef::None);
            if let Node::Connection(c) = &mut n {
                c.username = Some("from-connection".into());
            }
            n
        });
        let config = build_config(&state, u(3), false).expect("config");
        assert!(config.ask.is_some() && config.password.is_none());
        assert_eq!(config.username.as_deref(), Some("from-connection"));
    }

    async fn tls_server_up() -> bool {
        tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(("127.0.0.1", 15901)),
        )
        .await
        .is_ok_and(|r| r.is_ok())
    }

    /// The user agreed before to use VNC Authentication without encryption on the test server.
    fn agree_to_plain_vnc(dir: &TempDir) {
        PlainStore::in_data_dir(&dir.0)
            .allow("127.0.0.1", 15900)
            .expect("record the agreement");
    }

    fn tls_connection(id: u128, credential: CredentialRef) -> Node {
        let mut node = vnc_connection(id, None, credential);
        if let Node::Connection(c) = &mut node {
            c.port = Some(15901);
        }
        node
    }

    /// `fb_cert_decision` reaches the session of a VeNCrypt server: prompt (tag 3) -> accept ->
    /// encrypted report (tag 7) and frames; the certificate is pinned in the data directory.
    /// Then a changed pin -> prompt with `changed` -> decline -> clean close (tag 4).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cert_decision_is_routed_to_the_vencrypt_session() {
        let Some(password) = test_password() else {
            eprintln!("SKIP: dev/keys/test.env has no VNC_PASSWORD");
            return;
        };
        if !tls_server_up().await {
            eprintln!("SKIP: no VeNCrypt server on 127.0.0.1:15901 (dev/servers.sh up)");
            return;
        }
        let dir = TempDir::new("tls");
        let state = AppState::new(dir.0.clone());
        put(&state, tls_connection(2, CredentialRef::Id { id: u(100) }));
        unlocked_vault_with_users(&state, &dir, &[(100, "tester", &password)]);

        let (channel, bodies) = collecting_channel();
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        assert!(
            wait_for(&bodies, 3, Duration::from_secs(20)).await,
            "no CertPrompt"
        );
        let prompt: serde_json::Value = {
            let bodies = bodies.lock().expect("bodies");
            let frame = bodies.iter().find(|b| b[0] == 3).expect("frame");
            serde_json::from_slice(&frame[1..]).expect("json")
        };
        assert_eq!(prompt["changed"], false);
        let fingerprint = prompt["fingerprint"]
            .as_str()
            .expect("fingerprint")
            .to_owned();
        // Until the decision nothing but the prompt reaches the UI.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(bodies.lock().expect("bodies").iter().all(|b| b[0] == 3));

        let fb = state.sessions.fb(id).expect("fb session");
        fb.cert_decision(true).expect("accept");
        assert!(
            wait_for(&bodies, 1, Duration::from_secs(20)).await,
            "no Rect"
        );
        assert!(bodies
            .lock()
            .expect("bodies")
            .iter()
            .any(|b| b == &[&[7u8][..], br#"{"encrypted":true}"#].concat()));
        let store = std::fs::read_to_string(dir.0.join("vnc_known_certs")).expect("store");
        assert!(store.contains(&fingerprint));
        // No secret reaches the UI.
        assert!(bodies
            .lock()
            .expect("bodies")
            .iter()
            .filter(|b| b[0] != 1 && b[0] != 2)
            .all(|b| !String::from_utf8_lossy(b).contains(&password)));
        state.sessions.close(id).expect("close");

        // A changed pin: the attempt ends with the changed-certificate report (both
        // fingerprints), nothing to decide, nothing else sent to the UI.
        let tampered = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
        std::fs::write(
            dir.0.join("vnc_known_certs"),
            format!("127.0.0.1\t15901\t{tampered}\n"),
        )
        .expect("tamper");
        let (channel, bodies) = collecting_channel();
        let id = open(&state, u(2), 1024, 768, channel, false)
            .await
            .expect("open");
        assert!(
            wait_for(&bodies, 3, Duration::from_secs(20)).await,
            "no CertPrompt"
        );
        let prompt: serde_json::Value = {
            let bodies = bodies.lock().expect("bodies");
            let frame = bodies.iter().find(|b| b[0] == 3).expect("frame");
            serde_json::from_slice(&frame[1..]).expect("json")
        };
        assert_eq!(prompt["changed"], true);
        assert_eq!(prompt["stored"], tampered);
        assert_eq!(prompt["fingerprint"], fingerprint);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            bodies.lock().expect("bodies").iter().all(|b| b[0] == 3),
            "nothing but the report"
        );
        let store = std::fs::read_to_string(dir.0.join("vnc_known_certs")).expect("store");
        assert!(
            store.contains(tampered),
            "a changed certificate leaves the store alone"
        );

        // The deliberate way forward: remove the stored certificate, then connect again.
        state
            .sessions
            .fb(id)
            .expect("fb session")
            .forget_certificate()
            .expect("forget");
        let store = std::fs::read_to_string(dir.0.join("vnc_known_certs")).expect("store");
        assert!(!store.contains(tampered));
        state.sessions.close(id).expect("close");
    }
}
