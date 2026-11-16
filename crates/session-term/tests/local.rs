//! Local shell tests (macOS/Linux). They run `/bin/sh` in a scratch directory so no user
//! profile or dotfile is read.
#![cfg(unix)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use portolano_core_model::ErrorCode;
use portolano_session_term::{LocalSpec, Session, SessionSpec};

// Generous: a loaded CI runner can be slow to start a shell; waits are condition-based, so a
// fast machine never waits this long.
const T: Duration = Duration::from_secs(30);

/// An interactive `/bin/sh` that never writes a history file: on macOS `sh` is bash and
/// would otherwise create `~/.bash_history` in the real home folder on exit.
fn spec(cwd: &std::path::Path) -> LocalSpec {
    LocalSpec {
        program: Some("/usr/bin/env".into()),
        args: vec!["HISTFILE=/dev/null".into(), "/bin/sh".into()],
        cwd: Some(cwd.to_string_lossy().into_owned()),
    }
}

fn open(spec: LocalSpec) -> (Arc<Session>, Arc<Probe>) {
    let probe = Arc::new(Probe::default());
    let session = Session::open(SessionSpec::Local(spec), 80, 24, probe.sink()).expect("open");
    (Arc::new(session), probe)
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread")]
async fn echo_and_environment() {
    let dir = scratch();
    let (session, probe) = open(spec(dir.path()));
    ready(&session, &probe).await;
    session
        .write(b"echo hello-$((6*7)) $TERM $COLORTERM; pwd\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "hello-42 xterm-256color truecolor", T).await;
    // Started in the requested directory (resolve symlinks such as /var -> /private/var).
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    wait_text(&probe, real.to_str().expect("utf8"), T).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn resize_reaches_the_pty() {
    let dir = scratch();
    let (session, probe) = open(spec(dir.path()));
    ready(&session, &probe).await;
    session.write(b"stty size\n".to_vec()).await.expect("write");
    wait_text(&probe, "24 80", T).await;
    session.resize(132, 50).expect("resize");
    session.write(b"stty size\n".to_vec()).await.expect("write");
    wait_text(&probe, "50 132", T).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_code_follows_the_output() {
    let dir = scratch();
    let (session, probe) = open(spec(dir.path()));
    ready(&session, &probe).await;
    session
        .write(b"echo last-$((1+1)); exit 3\n".to_vec())
        .await
        .expect("write");
    wait_until("the exit", T, || probe.exit().is_some()).await;
    assert_eq!(probe.exit(), Some(Some(3)));
    assert!(probe.text().contains("last-2"));
    assert!(probe.error().is_none());
}

/// The number between `prefix` and `suffix` in the first place where it is all digits.
///
/// The PTY echoes what is typed, so the command line itself (`...PID[%s]...` or `...$$...`)
/// is part of the output. Requiring digits between two delimiters makes sure only the
/// expanded value matches, never the echoed command.
fn number_between(text: &str, prefix: &str, suffix: &str) -> Option<u32> {
    text.split(prefix).skip(1).find_map(|rest| {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || !rest[digits.len()..].starts_with(suffix) {
            return None;
        }
        digits.parse().ok()
    })
}

#[test]
fn number_between_ignores_the_echoed_command() {
    let echoed = "printf 'SHELL-PID[%s]\\n' $$; sleep 1000\r\n";
    assert_eq!(number_between(echoed, "SHELL-PID[", "]"), None);
    let both = format!("{echoed}SHELL-PID[4242]\r\n");
    assert_eq!(number_between(&both, "SHELL-PID[", "]"), Some(4242));
    // A half-received value is not a match yet.
    assert_eq!(number_between("SHELL-PID[42", "SHELL-PID[", "]"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn close_kills_the_shell_and_its_foreground_job() {
    let dir = scratch();
    let (session, probe) = open(spec(dir.path()));
    // Wait for an interactive shell first: input typed before it is ready may be dropped.
    ready(&session, &probe).await;
    session
        .write(b"printf 'SHELL-PID[%s]\\n' $$; sleep 1000\n".to_vec())
        .await
        .expect("write");
    // Condition-based: the value arrives after the echo of the command line.
    wait_until("the expanded shell pid", T, || {
        number_between(&probe.text(), "SHELL-PID[", "]").is_some()
    })
    .await;
    let pid = number_between(&probe.text(), "SHELL-PID[", "]").expect("pid in the output");
    assert!(
        process_alive(pid),
        "the shell must be running before the close"
    );
    session.close();
    wait_until("the shell to die", T, || !process_alive(pid)).await;
    assert_eq!(
        session.write(b"x".to_vec()).await.expect_err("closed").code,
        ErrorCode::Io
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flow_control_applies_to_local_output_too() {
    let dir = scratch();
    let (session, probe) = open(spec(dir.path()));
    ready(&session, &probe).await;
    let window = portolano_session_term::MAX_UNACKED_BYTES;
    session
        .write(b"seq 1 1000000; echo END-$((3+4))\n".to_vec())
        .await
        .expect("write");
    wait_until("the window to fill", T, || {
        probe.data_len() >= window - 64 * 1024
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        probe.data_len() <= window,
        "{} bytes without acknowledgement",
        probe.data_len()
    );
    assert!(!probe.text().contains("END-7"));
    let _acker = Acker::start(&session, &probe);
    wait_text(&probe, "END-7", Duration::from_secs(30)).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_errors_are_explicit() {
    let dir = scratch();
    let probe = Arc::new(Probe::default());
    let missing = LocalSpec {
        program: Some("/nonexistent/portolano-shell".into()),
        args: vec![],
        cwd: Some(dir.path().to_string_lossy().into_owned()),
    };
    let err = Session::open(SessionSpec::Local(missing), 80, 24, probe.sink())
        .err()
        .expect("must fail");
    assert!(
        matches!(err.code, ErrorCode::NotFound | ErrorCode::Io),
        "{err}"
    );

    let bad_cwd = LocalSpec {
        program: Some("/bin/sh".into()),
        args: vec![],
        cwd: Some("/nonexistent/portolano-dir".into()),
    };
    let err = Session::open(SessionSpec::Local(bad_cwd), 80, 24, probe.sink())
        .err()
        .expect("must fail");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_program_and_args_are_honoured() {
    let dir = scratch();
    let (_session, probe) = open(LocalSpec {
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "echo args-ok-$((1+1)); exit 5".into()],
        cwd: Some(dir.path().to_string_lossy().into_owned()),
    });
    wait_until("the exit", T, || probe.exit().is_some()).await;
    assert_eq!(probe.exit(), Some(Some(5)));
    assert!(probe.text().contains("args-ok-2"));
}
