//! Credential prompts of SSH sessions (WP11), against the test sshd. No vault is involved: the
//! session asks, the test answers. They skip, with a message, when the server is unreachable.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use portolano_core_model::prompt::{
    CredentialAnswer, PromptHooks, PromptKind, SavedCredential, SavedSecret,
};
use portolano_core_model::ErrorCode;
use portolano_session_term::{Session, SessionSpec};
use secrecy::{ExposeSecret, SecretString};

const T: Duration = Duration::from_secs(25);

const WRONG: &str = "definitely-not-the-password-9f3";

fn known_hosts_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("known_hosts")
}

/// Opens a session whose target asks for credentials and accepts the host key.
async fn open_asking(
    hint: &str,
    hooks: PromptHooks,
    kh: &std::path::Path,
) -> (Arc<Session>, Arc<Probe>) {
    let (session, probe) = open_ssh(ssh_spec(ask_hop(hint, hooks), kh), 80, 24);
    answer_prompt(&session, &probe, true).await;
    (session, probe)
}

#[tokio::test(flavor = "multi_thread")]
async fn no_vault_prompt_then_correct_password_gives_a_shell() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) =
        open_asking("tester", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;

    let prompt = wait_credential_prompt(&probe, 1).await;
    assert_eq!(prompt.kind, PromptKind::SshKey);
    // The connection's user name pre-fills the prompt: only the password is missing.
    assert_eq!(prompt.username_hint.as_deref(), Some("tester"));
    assert_eq!(prompt.target, format!("{SERVER_HOST}:{SERVER_PORT}"));
    assert_eq!(prompt.error, None);
    assert!(!prompt.can_save_to_vault, "no vault: nothing to save into");
    // Nothing is sent before the answer.
    assert_eq!(probe.data_len(), 0);

    let password = test_password();
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("answer");
    ready(&session, &probe).await;
    session
        .write(b"echo user-is-$(id -un)\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "user-is-tester", T).await;

    // The answer never travels back to the UI.
    assert!(!probe.raw_contains(password.expose_secret()));
    assert_eq!(probe.credential_prompts().len(), 1);
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_without_hint_has_no_username_hint() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = open_asking("", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;
    let prompt = wait_credential_prompt(&probe, 1).await;
    assert_eq!(prompt.username_hint, None);
    session.credential_cancel().expect("cancel");
    wait_until("the end", T, || probe.error().is_some()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_asks_again_with_the_error_on_the_same_session() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) =
        open_asking("tester", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;

    wait_credential_prompt(&probe, 1).await;
    let wrong = SecretString::from(WRONG.to_owned());
    session
        .credential(answer("tester", Some(&wrong)), false)
        .expect("wrong answer");
    let again = wait_credential_prompt(&probe, 2).await;
    let message = again.error.expect("the second prompt carries the reason");
    assert!(message.contains("rejected the credentials"), "{message}");
    assert!(!message.contains(WRONG), "secret echoed: {message}");
    // The user name typed the first time is offered again.
    assert_eq!(again.username_hint.as_deref(), Some("tester"));
    // The session is still alive: no error, no exit.
    assert!(probe.error().is_none() && probe.exit().is_none());

    let password = test_password();
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("right answer");
    ready(&session, &probe).await;
    assert!(!probe.raw_contains(WRONG));
    assert!(!probe.raw_contains(password.expose_secret()));
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_the_prompt_ends_with_a_cancelled_error() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) =
        open_asking("tester", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;
    wait_credential_prompt(&probe, 1).await;
    session.credential_cancel().expect("cancel");
    wait_until("the cancelled error", T, || probe.error().is_some()).await;
    assert_eq!(probe.error().expect("error").code, ErrorCode::Cancelled);
    assert_eq!(probe.data_len(), 0);
    // No second prompt, no shell: a clean end.
    assert_eq!(probe.credential_prompts().len(), 1);
    // A late answer has nothing to answer.
    assert!(session.credential_cancel().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_while_the_prompt_is_open_is_quiet() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) =
        open_asking("tester", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;
    wait_credential_prompt(&probe, 1).await;
    session.close();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(probe.error().is_none(), "{:?}", probe.error());
}

#[tokio::test(flavor = "multi_thread")]
async fn key_file_answer_works_and_its_content_never_reaches_the_ui() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) =
        open_asking("tester", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;
    wait_credential_prompt(&probe, 1).await;

    // A path that does not exist: a message, a new prompt, the session survives.
    session
        .credential(
            CredentialAnswer {
                key_path: Some(
                    dir.path()
                        .join("no-such-key")
                        .to_string_lossy()
                        .into_owned(),
                ),
                ..answer("tester", None)
            },
            false,
        )
        .expect("answer");
    let again = wait_credential_prompt(&probe, 2).await;
    assert!(
        again
            .error
            .as_deref()
            .is_some_and(|e| e.contains("key file")),
        "{:?}",
        again.error
    );

    let key_path = repo_root().join("dev/keys/id_ed25519");
    session
        .credential(
            CredentialAnswer {
                key_path: Some(key_path.to_string_lossy().into_owned()),
                ..answer("tester", None)
            },
            false,
        )
        .expect("key answer");
    ready(&session, &probe).await;
    let pem = test_key();
    let first_body_line = pem
        .expose_secret()
        .lines()
        .nth(1)
        .expect("key body")
        .to_owned();
    assert!(!probe.raw_contains(&first_body_line));
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_answer_is_asked_again_and_the_attempts_are_capped() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = open_asking("", PromptHooks::no_vault(), &known_hosts_file(&dir)).await;
    wait_credential_prompt(&probe, 1).await;

    // No user name: asked again without a server round trip.
    let password = test_password();
    session
        .credential(answer("  ", Some(&password)), false)
        .expect("answer");
    let again = wait_credential_prompt(&probe, 2).await;
    assert!(again
        .error
        .as_deref()
        .is_some_and(|e| e.contains("user name")));

    // Keep answering wrongly: the fifth failure ends the connection.
    let wrong = SecretString::from(WRONG.to_owned());
    let mut n = 2;
    while probe.error().is_none() {
        session
            .credential(answer("tester", Some(&wrong)), false)
            .expect("answer");
        n += 1;
        wait_until("the next prompt or the end", T, || {
            probe.credential_prompts().len() >= n || probe.error().is_some()
        })
        .await;
        assert!(n < 12, "the attempts were not capped");
    }
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::AuthFailed);
    assert!(err.message.contains("Too many"), "{}", err.message);
    assert!(!probe.raw_contains(WRONG));
}

#[tokio::test(flavor = "multi_thread")]
async fn save_to_vault_is_offered_per_hook_and_reported_only_after_success() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let saved: Arc<Mutex<Vec<SavedCredential>>> = Arc::default();
    let sink_saved = saved.clone();
    let hooks = PromptHooks {
        can_save: Arc::new(|| true),
        saved: Arc::new(move |c| {
            if let Ok(mut v) = sink_saved.lock() {
                v.push(c);
            }
        }),
    };
    let (session, probe) = open_asking("tester", hooks, &known_hosts_file(&dir)).await;
    let prompt = wait_credential_prompt(&probe, 1).await;
    assert!(prompt.can_save_to_vault);

    // A failed attempt with "save" ticked saves nothing.
    let wrong = SecretString::from(WRONG.to_owned());
    session
        .credential(answer("tester", Some(&wrong)), true)
        .expect("wrong");
    wait_credential_prompt(&probe, 2).await;
    assert!(saved.lock().expect("saved").is_empty());

    let password = test_password();
    session
        .credential(answer("tester", Some(&password)), true)
        .expect("right");
    ready(&session, &probe).await;
    wait_until("the saved credential", T, || {
        !saved.lock().expect("saved").is_empty()
    })
    .await;
    let saved = saved.lock().expect("saved");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].username, "tester");
    assert!(matches!(
        &saved[0].secret,
        SavedSecret::Password(p) if p.expose_secret() == password.expose_secret()
    ));
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_save_box_nothing_is_reported() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let hooks = PromptHooks {
        can_save: Arc::new(|| true),
        saved: Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    };
    let (session, probe) = open_asking("tester", hooks, &known_hosts_file(&dir)).await;
    wait_credential_prompt(&probe, 1).await;
    session
        .credential(answer("tester", Some(&test_password())), false)
        .expect("answer");
    ready(&session, &probe).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn each_hop_of_a_jump_chain_names_itself_in_its_prompt() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    // The "internal" target is the same sshd seen from inside its container: 127.0.0.1:22.
    let mut target = ask_hop("tester", PromptHooks::no_vault());
    target.port = 22;
    let mut spec = ssh_spec(target, &kh);
    spec.jumps = vec![ask_hop("tester", PromptHooks::no_vault())];
    let probe = Arc::new(Probe::default());
    let session = Arc::new(
        Session::open(SessionSpec::Ssh(Box::new(spec)), 80, 24, probe.sink()).expect("open"),
    );
    let password = test_password();

    wait_until("the first host key", T, || !probe.prompts().is_empty()).await;
    // The first host-key prompt is for the jump host, not for the target.
    assert_eq!(
        probe.prompt_hosts(),
        vec![(SERVER_HOST.to_owned(), SERVER_PORT)]
    );
    session.host_key_decision(true).expect("accept jump key");
    let first = wait_credential_prompt(&probe, 1).await;
    assert_eq!(first.target, format!("{SERVER_HOST}:{SERVER_PORT}"));
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("first hop");

    wait_until("the second host key", T, || probe.prompts().len() >= 2).await;
    assert_eq!(probe.prompt_hosts()[1], (SERVER_HOST.to_owned(), 22));
    session.host_key_decision(true).expect("accept target key");
    let second = wait_credential_prompt(&probe, 2).await;
    assert_eq!(second.target, format!("{SERVER_HOST}:22"));
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("second hop");
    ready(&session, &probe).await;
    session.close();
}

/// A TCP forwarder to the test sshd whose connections the test can cut.
struct Proxy {
    port: u16,
    cut: tokio::sync::watch::Sender<u32>,
}

async fn start_proxy() -> Proxy {
    let listener = tokio::net::TcpListener::bind((SERVER_HOST, 0))
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (cut, cut_rx) = tokio::sync::watch::channel(0u32);
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let mut cut_rx = cut_rx.clone();
            // A connection made after a cut must not see that cut.
            cut_rx.borrow_and_update();
            tokio::spawn(async move {
                let Ok(mut upstream) =
                    tokio::net::TcpStream::connect((SERVER_HOST, SERVER_PORT)).await
                else {
                    return;
                };
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {}
                    _ = cut_rx.changed() => {}
                }
            });
        }
    });
    Proxy { port, cut }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_dropped_while_the_prompt_is_open_is_reconnected_transparently() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let proxy = start_proxy().await;
    let mut target = ask_hop("tester", PromptHooks::no_vault());
    target.port = proxy.port;
    let (session, probe) = open_ssh(ssh_spec(target, &known_hosts_file(&dir)), 80, 24);
    answer_prompt(&session, &probe, true).await;
    wait_credential_prompt(&probe, 1).await;

    // The server side goes away while the user is typing.
    proxy.cut.send_modify(|n| *n += 1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let password = test_password();
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("answer");
    // The attempt fails on the dead connection; a fresh one is opened and the user is asked
    // again with an explanation. The host key is already known: no second host key prompt.
    let again = wait_credential_prompt(&probe, 2).await;
    assert!(again
        .error
        .as_deref()
        .is_some_and(|e| e.contains("closed the connection")));
    assert_eq!(probe.prompts().len(), 1);
    assert!(probe.error().is_none());
    session
        .credential(answer("tester", Some(&password)), false)
        .expect("second answer");
    ready(&session, &probe).await;
    session.close();
}
