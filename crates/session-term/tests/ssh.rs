//! Integration tests against the test sshd (127.0.0.1:12222, started by `dev/servers.sh up`).
//! They skip, with a message, when the server is unreachable. Credentials are read from
//! `dev/keys` at run time.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use portolano_core_model::ErrorCode;
use portolano_session_term::{
    known_hosts_list, known_hosts_remove, KnownHosts, Session, SessionSpec, SshAuth,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

const T: Duration = Duration::from_secs(20);

fn known_hosts_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("known_hosts")
}

/// A trusted-host file for the test server (accepts the key once through a throwaway session).
async fn trust_server(path: &std::path::Path) {
    let (session, probe) = open_ssh(ssh_spec(password_hop(), path), 80, 24);
    answer_prompt(&session, &probe, true).await;
    wait_until("the shell", T, || probe.data_len() > 0).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_key_prompts_accept_persists_then_connects_silently() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);

    let (session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    let (fingerprint, algorithm) = answer_prompt(&session, &probe, true).await;
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    assert_eq!(algorithm, "ssh-ed25519");
    ready(&session, &probe).await;
    session.close();

    // Persisted, in the OpenSSH layout, under the non-default port pattern.
    let text = std::fs::read_to_string(&kh).expect("known_hosts written");
    assert!(text.starts_with("[127.0.0.1]:12222 ssh-ed25519 "), "{text}");
    let entries = known_hosts_list(&kh).expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "127.0.0.1");
    assert_eq!(entries[0].port, 12222);
    assert_eq!(entries[0].fingerprint, fingerprint);

    // Second connection: matching key, no prompt.
    let (session, probe) = connected_shell(&kh).await;
    assert!(probe.prompts().is_empty(), "a known key must not prompt");
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn rejecting_an_unknown_key_fails_cleanly_and_stores_nothing() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    let (session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    answer_prompt(&session, &probe, false).await;
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::AuthFailed);
    assert!(err.message.contains("host key"), "{}", err.message);
    assert_eq!(probe.data_len(), 0);
    assert!(!kh.exists() || known_hosts_list(&kh).expect("list").is_empty());
    // The decision is no longer pending.
    assert_eq!(
        session
            .host_key_decision(true)
            .expect_err("nothing pending")
            .code,
        ErrorCode::InvalidInput
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_key_fails_with_both_fingerprints_and_never_prompts() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    // A different ed25519 key stored for the same host:port, as if the server had been replaced.
    let stored = "AAAAC3NzaC1lZDI1NTE5AAAAICsdM+rC22ibB4bJRNCBtV0QHH/ONxI5/URcdlsjaF2V";
    std::fs::write(&kh, format!("[127.0.0.1]:12222 ssh-ed25519 {stored}\n")).expect("seed");
    let stored_fp = known_hosts_list(&kh).expect("list")[0].fingerprint.clone();

    let (_session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::HostKeyMismatch, "{}", err.message);
    assert!(
        err.message.contains(&stored_fp),
        "stored fingerprint missing: {}",
        err.message
    );
    assert!(
        err.message.contains("Presented: ssh-ed25519 SHA256:"),
        "{}",
        err.message
    );
    assert!(
        probe.prompts().is_empty(),
        "a changed key must never prompt"
    );
    assert_eq!(probe.data_len(), 0);
    // Nothing was overwritten.
    assert_eq!(
        known_hosts_list(&kh).expect("list")[0].fingerprint,
        stored_fp
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_host_with_a_key_of_another_algorithm_is_a_mismatch_not_unknown() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    // ECDSA P-384: the test server has no key of that type, so it presents another algorithm
    // than the stored one. Under the spike's rule that was "unknown"; now it must be a mismatch.
    let key = dir.path().join("ec384");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ecdsa", "-b", "384", "-N", "", "-C", "", "-f"])
        .arg(&key)
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => {
            eprintln!("SKIPPED: ssh-keygen is not usable ({other:?})");
            return;
        }
    }
    let pubkey = std::fs::read_to_string(dir.path().join("ec384.pub")).expect("pub key");
    let kh = known_hosts_file(&dir);
    std::fs::write(&kh, format!("[127.0.0.1]:12222 {}\n", pubkey.trim())).expect("seed");

    let (_session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::HostKeyMismatch, "{}", err.message);
    assert!(probe.prompts().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn stored_rsa_key_that_differs_from_the_servers_is_a_mismatch() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let key = dir.path().join("rsa");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "rsa", "-b", "2048", "-N", "", "-C", "", "-f"])
        .arg(&key)
        .status();
    match status {
        Ok(s) if s.success() => {}
        other => {
            eprintln!("SKIPPED: ssh-keygen is not usable ({other:?})");
            return;
        }
    }
    let pubkey = std::fs::read_to_string(dir.path().join("rsa.pub")).expect("pub key");
    let kh = known_hosts_file(&dir);
    std::fs::write(&kh, format!("[127.0.0.1]:12222 {}\n", pubkey.trim())).expect("seed");
    let (_session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::HostKeyMismatch);
    // The stored algorithm was offered first, so the server presented its RSA key.
    assert!(
        err.message.contains("Presented: ssh-rsa"),
        "{}",
        err.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_auth_failed() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    let hop = hop(SshAuth::Password(SecretString::from(
        "definitely-not-the-password",
    )));
    let (session, probe) = open_ssh(ssh_spec(hop, &kh), 80, 24);
    answer_prompt(&session, &probe, true).await;
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::AuthFailed);
    assert!(
        !err.message.contains("definitely-not"),
        "secret leaked: {}",
        err.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn private_key_authentication_works() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    trust_server(&kh).await;
    let (session, probe) = open_ssh(ssh_spec(key_hop(), &kh), 80, 24);
    ready(&session, &probe).await;
    session
        .write(b"echo user-is-$(id -un)\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "user-is-tester", T).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn encrypted_key_with_wrong_passphrase_is_reported_without_reaching_the_server_auth() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    trust_server(&kh).await;
    let key = dir.path().join("enc");
    let ok = std::process::Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "right-passphrase",
            "-C",
            "",
            "-f",
        ])
        .arg(&key)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("SKIPPED: ssh-keygen is not usable");
        return;
    }
    let pem = std::fs::read_to_string(&key).expect("key");
    let mk = |pass: Option<&str>| {
        hop(SshAuth::Key {
            private_key: SecretString::from(pem.clone()),
            passphrase: pass.map(|p| SecretString::from(p.to_string())),
        })
    };

    // Wrong passphrase and missing passphrase: auth_failed with a message about the key.
    for pass in [Some("zzz-bad-pass"), None] {
        let (_s, probe) = open_ssh(ssh_spec(mk(pass), &kh), 80, 24);
        wait_until("an error", T, || probe.error().is_some()).await;
        let err = probe.error().expect("error");
        assert_eq!(err.code, ErrorCode::AuthFailed);
        assert!(err.message.contains("private key"), "{}", err.message);
        assert!(!err.message.contains("zzz-bad-pass") && !err.message.contains("right-passphrase"));
    }
    // Right passphrase: the key decodes and is offered; the server (which does not know this
    // key) rejects it, which is a different message than the one above.
    let (_s, probe) = open_ssh(ssh_spec(mk(Some("right-passphrase")), &kh), 80, 24);
    wait_until("an error", T, || probe.error().is_some()).await;
    let err = probe.error().expect("error");
    assert_eq!(err.code, ErrorCode::AuthFailed);
    assert!(
        err.message.contains("rejected the credentials"),
        "{}",
        err.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_host_is_host_unreachable() {
    let _serial = serial().await;
    let dir = scratch();
    // Needs neither the server nor its credentials: a dummy password for a closed port.
    let mut target = hop(SshAuth::Password(SecretString::from("unused")));
    target.port = 1; // nothing listens there on loopback
    let (_s, probe) = open_ssh(ssh_spec(target, &known_hosts_file(&dir)), 80, 24);
    wait_until("an error", T, || probe.error().is_some()).await;
    assert_eq!(
        probe.error().expect("error").code,
        ErrorCode::HostUnreachable
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_output_and_utf8_locale() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    session
        .write(b"echo hello-$((6*7))\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "hello-42", T).await;
    // The LANG sent before the shell is visible in the shell's initial environment (the
    // server's profile then overrides it for the shell itself).
    session
        .write(b"tr '\\0' '\\n' < /proc/$$/environ | grep '^LANG=' | sed 's/^/INIT-/'\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "INIT-LANG=en_US.UTF-8", T).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn resize_reaches_the_remote_pty() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    session.write(b"stty size\n".to_vec()).await.expect("write");
    wait_text(&probe, "24 80", T).await;
    session.resize(132, 50).expect("resize");
    tokio::time::sleep(Duration::from_millis(300)).await;
    session.write(b"stty size\n".to_vec()).await.expect("write");
    wait_text(&probe, "50 132", T).await;
    assert_eq!(
        session.resize(0, 10).expect_err("zero").code,
        ErrorCode::InvalidInput
    );
    session.close();
}

fn expected_seq_digest(n: u32) -> (String, usize) {
    let mut h = Sha256::new();
    let mut len = 0;
    for i in 1..=n {
        let line = format!("{i}\r\n");
        len += line.len();
        h.update(line.as_bytes());
    }
    (hex(&h.finalize()), len)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn between(text: &[u8], start: &[u8], end: &[u8]) -> Vec<u8> {
    let find = |hay: &[u8], needle: &[u8], from: usize| {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    };
    let s = find(text, start, 0).expect("start marker") + start.len();
    let e = find(text, end, s).expect("end marker");
    text[s..e].to_vec()
}

#[tokio::test(flavor = "multi_thread")]
async fn large_output_arrives_intact() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    let _acker = Acker::start(&session, &probe);
    session
        .write(b"echo BEGIN-$((3+4)); seq 1 1500000; echo END-$((3+4))\n".to_vec())
        .await
        .expect("write");
    wait_text(&probe, "END-7", Duration::from_secs(60)).await;
    let got = between(&probe.data(), b"BEGIN-7\r\n", b"END-7");
    let (digest, len) = expected_seq_digest(1_500_000);
    assert_eq!(got.len(), len, "output length");
    assert_eq!(hex(&Sha256::digest(&got)), digest, "output checksum");
    assert!(
        probe.max_frame() <= 64 * 1024,
        "frame of {} bytes",
        probe.max_frame()
    );
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_megabyte_paste_with_echo_is_intact() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    let _acker = Acker::start(&session, &probe);

    // ~1 MiB of short lines (the tty line discipline truncates lines over 4096 bytes).
    let mut paste = Vec::new();
    let mut i = 0u32;
    while paste.len() < 1_048_576 {
        paste.extend_from_slice(format!("line {i:07} {}\n", "x".repeat(46)).as_bytes());
        i += 1;
    }
    let expected = hex(&Sha256::digest(&paste));
    let before = probe.data_len();

    // Echo is on: sha256sum reads the paste from the terminal, the tty echoes all of it back.
    session.write(b"sha256sum\n".to_vec()).await.expect("write");
    for chunk in paste.chunks(16 * 1024) {
        session.write(chunk.to_vec()).await.expect("paste chunk");
    }
    session.write(vec![0x04]).await.expect("eof");
    wait_text(&probe, &expected, Duration::from_secs(60)).await;
    let echoed = probe.data_len() - before;
    // The remote tty may coalesce or drop echo under load; the checksum above is the proof of
    // integrity, this only shows that a large echo flowed back.
    assert!(
        echoed >= paste.len() / 2,
        "echo too small: {echoed} of {} bytes",
        paste.len()
    );
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_status_is_reported_after_the_output() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    session
        .write(b"echo bye-$((1+1)); exit 7\n".to_vec())
        .await
        .expect("write");
    wait_until("the exit", T, || probe.exit().is_some()).await;
    assert_eq!(probe.exit(), Some(Some(7)));
    assert!(probe.text().contains("bye-2"));
    assert!(probe.error().is_none());
    // After the exit the session is over: writes fail explicitly.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        session.write(b"x".to_vec()).await.expect_err("ended").code,
        ErrorCode::Io
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flow_control_stops_the_stream_until_the_ui_acknowledges() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    let window = portolano_session_term::MAX_UNACKED_BYTES;

    let start = probe.data_len();
    session
        .write(b"echo BEGIN-$((3+4)); seq 1 2000000; echo END-$((3+4))\n".to_vec())
        .await
        .expect("write");
    // No acknowledgements: the backend must stop at the window (~13.8 MB are on their way).
    wait_until("the window to fill", T, || {
        probe.data_len() - start >= window - 64 * 1024
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let outstanding = probe.data_len() - start;
    assert!(
        outstanding <= window,
        "{outstanding} bytes sent without acknowledgement"
    );
    assert!(session.unacked() <= window as u64);
    assert!(!probe.text().contains("END-7"));

    // Acknowledging resumes the stream, and the window is never exceeded on the way.
    let session2 = session.clone();
    let probe2 = probe.clone();
    let watcher = tokio::spawn(async move {
        let mut max = 0u64;
        loop {
            probe2.ack_new(&session2);
            max = max.max(session2.unacked());
            tokio::time::sleep(Duration::from_millis(2)).await;
            if probe2.text().contains("END-7") {
                return max;
            }
        }
    });
    let max = tokio::time::timeout(Duration::from_secs(90), watcher)
        .await
        .expect("stream completes")
        .expect("watcher");
    assert!(max <= window as u64, "unacked peaked at {max}");
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn close_is_immediate_and_idempotent() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let (session, probe) = connected_shell(&known_hosts_file(&dir)).await;
    let started = std::time::Instant::now();
    session.close();
    session.close();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(
        session.write(b"x".to_vec()).await.expect_err("closed").code,
        ErrorCode::Io
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let len = probe.data_len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(probe.data_len(), len, "no output after close");
}

#[tokio::test(flavor = "multi_thread")]
async fn known_hosts_list_and_remove_round_trip() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    trust_server(&kh).await;
    assert_eq!(known_hosts_list(&kh).expect("list").len(), 1);
    known_hosts_remove(&kh, "127.0.0.1", 12222).expect("remove");
    assert!(known_hosts_list(&kh).expect("list").is_empty());
    // Removing again is not an error; the host is unknown again and prompts.
    known_hosts_remove(&kh, "127.0.0.1", 12222).expect("remove again");
    assert_eq!(
        KnownHosts::new(&kh)
            .algorithms("127.0.0.1", 12222)
            .expect("algs"),
        Vec::<String>::new()
    );
    let (session, probe) = open_ssh(ssh_spec(password_hop(), &kh), 80, 24);
    answer_prompt(&session, &probe, true).await;
    wait_until("the shell", T, || probe.data_len() > 0).await;
    session.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn jump_host_reaches_the_target_through_a_direct_tcpip_channel() {
    require_server!();
    let _serial = serial().await;
    let dir = scratch();
    let kh = known_hosts_file(&dir);
    // The "internal" target is the same sshd, seen from inside its container: 127.0.0.1:22.
    let mut target = password_hop();
    target.port = 22;
    let mut spec = ssh_spec(target, &kh);
    spec.jumps = vec![password_hop()];

    let probe = Arc::new(Probe::default());
    let session = Arc::new(
        Session::open(SessionSpec::Ssh(Box::new(spec)), 80, 24, probe.sink()).expect("open"),
    );
    // Two prompts, one per hop (distinct host:port entries).
    for expected in 1..=2 {
        wait_until("a prompt", T, || probe.prompts().len() >= expected).await;
        session.host_key_decision(true).expect("accept");
    }
    ready(&session, &probe).await;
    // Each prompt says which hop it is about: the jump host first, then the target.
    assert_eq!(
        probe.prompt_hosts(),
        vec![
            (SERVER_HOST.to_owned(), SERVER_PORT),
            (SERVER_HOST.to_owned(), 22)
        ]
    );
    let entries = known_hosts_list(&kh).expect("list");
    assert_eq!(entries.len(), 2, "{entries:?}");
    session.close();
}
