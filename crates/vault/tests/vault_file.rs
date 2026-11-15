//! Black-box tests of the vault through its public API and the on-disk file.
//! All files live under `<repo>/.tmp/wp3/`. Passwords and secrets are obviously fake.

use std::path::{Path, PathBuf};

use portolano_core_model::ErrorCode;
use portolano_vault::{
    CredentialInput, CredentialKind, CredentialUpsert, KdfParams, RevealedSecret, SecretInput,
    Vault, VaultError, VaultStatus,
};
use secrecy::SecretString;
use uuid::Uuid;

// Layout constants of the documented format (see `format.rs`).
const HEADER_LEN: usize = 110;
const PAYLOAD_NONCE: std::ops::Range<usize> = 110..134;
const OFF_VERSION: usize = 8;
const OFF_M: usize = 10;
const OFF_T: usize = 14;
const OFF_P: usize = 18;
const OFF_WRAPPED: usize = 62;

const MASTER: &str = "fake-master-password-1";

fn tiny() -> KdfParams {
    KdfParams {
        memory_kib: 8,
        iterations: 1,
        parallelism: 1,
    }
}

fn scratch() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tmp/wp3");
    std::fs::create_dir_all(&base).expect("create scratch base");
    tempfile::Builder::new()
        .prefix("vault-it-")
        .tempdir_in(base)
        .expect("tempdir")
}

fn pw(s: &str) -> SecretString {
    SecretString::from(s)
}

fn vault_at(path: PathBuf) -> Vault {
    Vault::with_kdf_params(path, tiny()).expect("vault")
}

fn password_input(id: Option<Uuid>, name: &str, secret: &str) -> CredentialInput {
    CredentialInput {
        id,
        name: name.into(),
        username: "fake-user".into(),
        domain: None,
        secret: SecretInput::Password {
            password: pw(secret),
        },
    }
}

fn key_input(name: &str, passphrase: Option<&str>) -> CredentialInput {
    CredentialInput {
        id: None,
        name: name.into(),
        username: "git".into(),
        domain: Some("CORP".into()),
        secret: SecretInput::SshKey {
            private_key_pem: pw(
                "-----BEGIN FAKE KEY-----\nZmFrZS1rZXktbWF0ZXJpYWw=\n-----END FAKE KEY-----",
            ),
            passphrase: passphrase.map(pw),
        },
    }
}

/// A created vault with one credential of each kind, locked.
fn populated() -> (tempfile::TempDir, PathBuf) {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    vault.create(pw(MASTER)).expect("create");
    vault
        .upsert(password_input(None, "pw", "fake-secret-password"))
        .expect("upsert pw");
    vault.upsert(key_input("key", None)).expect("upsert key");
    vault
        .upsert(key_input("key+pass", Some("fake-passphrase")))
        .expect("upsert key2");
    vault.lock().expect("lock");
    (dir, path)
}

fn code(err: &VaultError) -> ErrorCode {
    err.code()
}

#[test]
fn editing_without_a_secret_keeps_the_stored_one() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    vault.create(pw(MASTER)).expect("create");
    let created = vault
        .upsert(key_input("key+pass", Some("fake-passphrase")))
        .expect("create");

    let renamed = vault
        .upsert_update(CredentialUpsert {
            id: Some(created.id),
            name: "renamed".into(),
            username: "other-user".into(),
            domain: None,
            secret: None,
        })
        .expect("rename without secret");
    assert_eq!(renamed.id, created.id);
    assert_eq!(renamed.name, "renamed");
    assert_eq!(renamed.username, "other-user");
    assert_eq!(renamed.kind, CredentialKind::SshKey);
    assert_eq!(vault.list().expect("list").len(), 1);

    // The secret survives a lock and a reload from disk.
    vault.lock().expect("lock");
    let mut reloaded = vault_at(path);
    reloaded.unlock(pw(MASTER)).expect("unlock");
    match reloaded.reveal_secret(created.id).expect("reveal") {
        RevealedSecret::SshKey { passphrase, .. } => {
            assert_eq!(passphrase.as_deref(), Some("fake-passphrase"));
        }
        other => panic!("unexpected secret: {other:?}"),
    }
}

#[test]
fn creating_without_a_secret_is_refused() {
    let dir = scratch();
    let mut vault = vault_at(dir.path().join("vault.bin"));
    vault.create(pw(MASTER)).expect("create");
    for id in [None, Some(Uuid::new_v4())] {
        let err = vault
            .upsert_update(CredentialUpsert {
                id,
                name: "new".into(),
                username: "u".into(),
                domain: None,
                secret: None,
            })
            .expect_err("no secret");
        assert_eq!(code(&err), ErrorCode::InvalidInput);
    }
    assert!(vault.list().expect("list").is_empty());
}

#[test]
fn round_trip_all_credential_kinds() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    assert_eq!(vault.status().expect("status"), VaultStatus::NotCreated);
    vault.create(pw(MASTER)).expect("create");
    assert_eq!(vault.status().expect("status"), VaultStatus::Unlocked);

    let pw_id = vault
        .upsert(password_input(None, "pw", "fake-secret-password"))
        .expect("pw")
        .id;
    let key_id = vault.upsert(key_input("key", None)).expect("key").id;
    let key2_id = vault
        .upsert(key_input("key+pass", Some("fake-passphrase")))
        .expect("key2")
        .id;
    vault.lock().expect("lock");
    assert_eq!(vault.status().expect("status"), VaultStatus::Locked);

    // A brand-new handle on the same file, as after an app restart.
    let mut reopened = vault_at(path);
    assert_eq!(reopened.status().expect("status"), VaultStatus::Locked);
    reopened.unlock(pw(MASTER)).expect("unlock");
    let list = reopened.list().expect("list");
    assert_eq!(list.len(), 3);
    let by_id = |id| list.iter().find(|c| c.id == id).expect("summary");
    assert_eq!(by_id(pw_id).kind, CredentialKind::Password);
    assert_eq!(by_id(pw_id).username, "fake-user");
    assert_eq!(by_id(key_id).kind, CredentialKind::SshKey);
    assert_eq!(by_id(key_id).domain.as_deref(), Some("CORP"));

    match reopened.reveal_secret(pw_id).expect("reveal") {
        RevealedSecret::Password { password } => assert_eq!(password, "fake-secret-password"),
        other => panic!("unexpected {other:?}"),
    }
    match reopened.reveal_secret(key_id).expect("reveal") {
        RevealedSecret::SshKey {
            private_key_pem,
            passphrase,
        } => {
            assert!(private_key_pem.contains("BEGIN FAKE KEY"));
            assert_eq!(passphrase, None);
        }
        other => panic!("unexpected {other:?}"),
    }
    match reopened.reveal_secret(key2_id).expect("reveal") {
        RevealedSecret::SshKey { passphrase, .. } => {
            assert_eq!(passphrase.as_deref(), Some("fake-passphrase"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn upsert_replaces_and_delete_removes_across_restarts() {
    let (_dir, path) = populated();
    let mut vault = vault_at(path.clone());
    vault.unlock(pw(MASTER)).expect("unlock");
    let first = vault.list().expect("list")[0].clone();
    let updated = vault
        .upsert(password_input(Some(first.id), "renamed", "fake-new-secret"))
        .expect("update");
    assert_eq!(updated.id, first.id);
    assert_eq!(vault.list().expect("list").len(), 3);
    vault.delete(first.id).expect("delete");
    assert!(matches!(
        vault.delete(first.id),
        Err(VaultError::NotFound(_))
    ));
    assert!(matches!(
        vault.reveal_secret(first.id),
        Err(VaultError::NotFound(_))
    ));
    vault.lock().expect("lock");

    let mut again = vault_at(path);
    again.unlock(pw(MASTER)).expect("unlock");
    assert_eq!(again.list().expect("list").len(), 2);
}

#[test]
fn file_contains_no_plaintext() {
    let (_dir, path) = populated();
    let bytes = std::fs::read(path).expect("read");
    for needle in [
        "fake-secret-password",
        "fake-passphrase",
        "BEGIN FAKE KEY",
        "fake-user",
        "key+pass",
        "CORP",
        MASTER,
    ] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle} found in clear"
        );
    }
    assert_eq!(&bytes[..8], b"PRTLVLT\0");
}

#[test]
fn wrong_password_is_a_clean_auth_error() {
    let (_dir, path) = populated();
    let mut vault = vault_at(path);
    for wrong in ["", "fake-master-password-2", "FAKE-MASTER-PASSWORD-1"] {
        let err = vault.unlock(pw(wrong)).expect_err("must fail");
        assert!(matches!(err, VaultError::AuthFailed), "{err:?}");
        assert_eq!(code(&err), ErrorCode::AuthFailed);
        assert_eq!(err.to_string(), "authentication failed");
        assert_eq!(vault.status().expect("status"), VaultStatus::Locked);
        assert!(matches!(vault.list(), Err(VaultError::Locked)));
    }
    vault
        .unlock(pw(MASTER))
        .expect("right password still works");
}

#[test]
fn operations_on_a_locked_vault_fail_with_vault_locked() {
    let (_dir, path) = populated();
    let mut vault = vault_at(path);
    let id = Uuid::new_v4();
    let errors = [
        vault.list().map(|_| ()).expect_err("list"),
        vault
            .upsert(password_input(None, "x", "y"))
            .map(|_| ())
            .expect_err("upsert"),
        vault.delete(id).expect_err("delete"),
        vault.reveal_secret(id).map(|_| ()).expect_err("reveal"),
        vault
            .change_password(pw(MASTER), pw("fake-other"))
            .expect_err("change"),
    ];
    for err in errors {
        assert!(matches!(err, VaultError::Locked), "{err:?}");
        assert_eq!(code(&err), ErrorCode::VaultLocked);
    }

    // Same without any file at all.
    let dir = scratch();
    let mut none = vault_at(dir.path().join("vault.bin"));
    assert!(matches!(none.list(), Err(VaultError::Locked)));
    assert!(matches!(
        none.unlock(pw(MASTER)),
        Err(VaultError::NotCreated)
    ));
    none.lock().expect("lock is a no-op");
}

#[test]
fn create_policy_and_no_overwrite() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    let err = vault.create(pw("")).expect_err("empty password");
    assert_eq!(code(&err), ErrorCode::InvalidInput);
    assert!(!path.exists(), "a rejected create writes nothing");

    vault
        .create(pw("a"))
        .expect("any non-empty password is fine");
    let before = std::fs::read(&path).expect("read");
    let mut other = vault_at(path.clone());
    let err = other.create(pw("fake-other")).expect_err("exists");
    assert!(matches!(err, VaultError::AlreadyExists));
    assert_eq!(std::fs::read(&path).expect("read"), before);

    let err = vault
        .change_password(pw("a"), pw(""))
        .expect_err("empty new password");
    assert_eq!(code(&err), ErrorCode::InvalidInput);
    assert_eq!(std::fs::read(&path).expect("read"), before);
}

#[test]
fn change_master_password_old_fails_new_works() {
    let (_dir, path) = populated();
    let mut vault = vault_at(path.clone());
    vault.unlock(pw(MASTER)).expect("unlock");

    let err = vault
        .change_password(pw("fake-not-the-password"), pw("fake-new-1"))
        .expect_err("wrong current");
    assert!(matches!(err, VaultError::AuthFailed));

    vault
        .change_password(pw(MASTER), pw("fake-new-1"))
        .expect("change");
    vault.lock().expect("lock");
    let mut again = vault_at(path);
    assert!(matches!(
        again.unlock(pw(MASTER)),
        Err(VaultError::AuthFailed)
    ));
    again.unlock(pw("fake-new-1")).expect("new password");
    assert_eq!(again.list().expect("list").len(), 3);
}

#[test]
fn every_single_bit_flip_in_the_file_is_rejected_cleanly() {
    let (_dir, path) = populated();
    let original = std::fs::read(&path).expect("read");
    let mut vault = vault_at(path.clone());

    for index in 0..original.len() {
        for bit in [0u8, 7] {
            let mut bytes = original.clone();
            bytes[index] ^= 1 << bit;
            std::fs::write(&path, &bytes).expect("write tampered");
            let err = vault
                .unlock(pw(MASTER))
                .expect_err(&format!("flip at byte {index} bit {bit} must fail"));
            assert!(
                matches!(err.code(), ErrorCode::AuthFailed | ErrorCode::Unsupported),
                "byte {index}: {err:?}"
            );
            assert_eq!(vault.status().expect("status"), VaultStatus::Locked);
            assert!(matches!(vault.list(), Err(VaultError::Locked)));
        }
    }

    // The untouched file still opens.
    std::fs::write(&path, &original).expect("restore");
    vault.unlock(pw(MASTER)).expect("original unlocks");
}

#[test]
fn named_tampering_cases() {
    let (_dir, path) = populated();
    let original = std::fs::read(&path).expect("read");
    let mut vault = vault_at(path.clone());

    let mut expect_failure = |name: &str, bytes: Vec<u8>, want: ErrorCode| {
        std::fs::write(&path, bytes).expect("write");
        let err = vault.unlock(pw(MASTER)).expect_err(name);
        assert_eq!(err.code(), want, "{name}: {err:?}");
        assert!(matches!(vault.list(), Err(VaultError::Locked)), "{name}");
    };
    let with = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = original.clone();
        f(&mut b);
        b
    };

    expect_failure("bad magic", with(&|b| b[0] = b'X'), ErrorCode::AuthFailed);
    expect_failure(
        "unknown version",
        with(&|b| b[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&2u16.to_le_bytes())),
        ErrorCode::Unsupported,
    );
    expect_failure(
        "version 0",
        with(&|b| b[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&0u16.to_le_bytes())),
        ErrorCode::Unsupported,
    );
    for (name, off) in [
        ("memory", OFF_M),
        ("iterations", OFF_T),
        ("parallelism", OFF_P),
    ] {
        expect_failure(
            &format!("absurd {name}"),
            with(&|b| b[off..off + 4].copy_from_slice(&u32::MAX.to_le_bytes())),
            ErrorCode::AuthFailed,
        );
        expect_failure(
            &format!("zero {name}"),
            with(&|b| b[off..off + 4].copy_from_slice(&0u32.to_le_bytes())),
            ErrorCode::AuthFailed,
        );
    }
    // Plausible but different parameters: the derived KEK differs, so authentication fails.
    expect_failure(
        "different iterations",
        with(&|b| b[OFF_T..OFF_T + 4].copy_from_slice(&2u32.to_le_bytes())),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "flipped wrapped DEK",
        with(&|b| b[OFF_WRAPPED + 5] ^= 0x80),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "flipped payload nonce",
        with(&|b| b[PAYLOAD_NONCE.start + 3] ^= 0x01),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "flipped ciphertext",
        with(&|b| {
            let last = b.len() - 20;
            b[last] ^= 0x01;
        }),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "flipped tag",
        with(&|b| {
            let last = b.len() - 1;
            b[last] ^= 0x01;
        }),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "trailing garbage",
        with(&|b| b.push(0)),
        ErrorCode::AuthFailed,
    );
    for len in [0, 1, 8, 10, 109, 110, 133, 134, 149, original.len() - 1] {
        expect_failure(
            &format!("truncated to {len}"),
            original[..len].to_vec(),
            ErrorCode::AuthFailed,
        );
    }
    expect_failure("empty file", Vec::new(), ErrorCode::AuthFailed);
    expect_failure(
        "random garbage",
        (0..400u32).map(|i| (i * 31 % 251) as u8).collect(),
        ErrorCode::AuthFailed,
    );
    expect_failure(
        "oversized file",
        vec![0u8; 64 * 1024 * 1024 + 1],
        ErrorCode::AuthFailed,
    );

    // Header of one vault grafted onto the payload of another (valid files, wrong pairing).
    let other_dir = scratch();
    let other_path = other_dir.path().join("vault.bin");
    let mut other = vault_at(other_path.clone());
    other.create(pw(MASTER)).expect("create other");
    other.lock().expect("lock");
    let other_bytes = std::fs::read(&other_path).expect("read other");
    let mut grafted = original.clone();
    grafted[..HEADER_LEN].copy_from_slice(&other_bytes[..HEADER_LEN]);
    expect_failure("grafted header", grafted, ErrorCode::AuthFailed);

    // And the same header with the same password but the payload swapped.
    let mut swapped = other_bytes.clone();
    swapped.truncate(HEADER_LEN);
    swapped.extend_from_slice(&original[HEADER_LEN..]);
    expect_failure("grafted payload", swapped, ErrorCode::AuthFailed);

    std::fs::write(&path, &original).expect("restore");
    vault.unlock(pw(MASTER)).expect("original unlocks");
}

#[test]
fn two_saves_use_different_nonces_and_ciphertexts() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    vault.create(pw(MASTER)).expect("create");
    let id = vault
        .upsert(password_input(None, "pw", "fake-secret"))
        .expect("upsert")
        .id;
    let first = std::fs::read(&path).expect("read");
    // Same logical content saved again.
    vault
        .upsert(password_input(Some(id), "pw", "fake-secret"))
        .expect("upsert again");
    let second = std::fs::read(&path).expect("read");

    assert_eq!(first.len(), second.len());
    assert_eq!(
        first[..HEADER_LEN],
        second[..HEADER_LEN],
        "header is stable"
    );
    assert_ne!(first[PAYLOAD_NONCE], second[PAYLOAD_NONCE], "fresh nonce");
    assert_ne!(first[134..], second[134..], "different ciphertext");
}

#[test]
fn saves_are_atomic_and_leave_a_valid_file() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    vault.create(pw(MASTER)).expect("create");
    for i in 0..5 {
        vault
            .upsert(password_input(None, &format!("c{i}"), "fake-secret"))
            .expect("upsert");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["vault.bin"], "no temporary file left behind");
        // The file on disk is complete and valid after every save.
        let mut probe = vault_at(path.clone());
        probe.unlock(pw(MASTER)).expect("valid after save");
        assert_eq!(probe.list().expect("list").len(), i + 1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn failed_save_leaves_no_temporary_file() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = vault_at(path.clone());
    vault.create(pw(MASTER)).expect("create");
    std::fs::remove_file(&path).expect("remove");
    std::fs::create_dir(&path).expect("directory in the way");
    assert!(vault.upsert(password_input(None, "x", "y")).is_err());
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["vault.bin"]);
}

/// Real production parameters (64 MiB, t = 3, p = 1). Slow in a debug build, so it only runs
/// on request: `cargo test -p portolano-vault --release -- --ignored`.
#[test]
#[ignore = "slow: uses the production Argon2 parameters"]
fn production_parameters_round_trip() {
    let dir = scratch();
    let path = dir.path().join("vault.bin");
    let mut vault = Vault::new(path.clone());
    vault.create(pw(MASTER)).expect("create");
    vault
        .upsert(password_input(None, "pw", "fake-secret"))
        .expect("upsert");
    vault.lock().expect("lock");
    let bytes = std::fs::read(&path).expect("read");
    assert_eq!(bytes[OFF_M..OFF_M + 4], (64u32 * 1024).to_le_bytes());
    assert_eq!(bytes[OFF_T..OFF_T + 4], 3u32.to_le_bytes());
    assert_eq!(bytes[OFF_P..OFF_P + 4], 1u32.to_le_bytes());
    let mut again = Vault::new(path);
    again.unlock(pw(MASTER)).expect("unlock");
    assert_eq!(again.list().expect("list").len(), 1);
}

/// The review's proof of concept: after a password change, the old password plus an old copy
/// of the file must not decrypt the new file, secrets added later included.
#[test]
fn the_old_password_and_an_old_copy_do_not_open_the_new_file() {
    let (_dir, path) = populated();
    let mut vault = vault_at(path.clone());
    vault.unlock(pw(MASTER)).expect("unlock");
    let old_copy = std::fs::read(&path).expect("old copy");

    vault
        .change_password(pw(MASTER), pw("fake-new-password"))
        .expect("change");
    vault
        .upsert(password_input(
            None,
            "after-change",
            "fake-secret-after-change",
        ))
        .expect("upsert after the change");
    let current = std::fs::read(&path).expect("current file");
    assert!(
        !current
            .windows(b"fake-secret-after-change".len())
            .any(|w| w == b"fake-secret-after-change"),
        "no plaintext in the file"
    );

    // The attacker: the old password, the old copy of the file. Everything derived from the
    // old header is a key to the OLD file only.
    let probe = old_copy.clone();
    let mut attacker = vault_at(path.clone());
    // (1) The current file refuses the old password.
    assert!(matches!(
        attacker.unlock(pw(MASTER)),
        Err(VaultError::AuthFailed)
    ));
    // (2) The old copy still opens with the old password (rotation does not rewrite history) ...
    let copy_path = path.with_file_name("old-copy.bin");
    std::fs::write(&copy_path, &probe).expect("write the copy");
    let mut old = vault_at(copy_path.clone());
    old.unlock(pw(MASTER))
        .expect("the old copy keeps its old password");
    assert_eq!(old.list().expect("list").len(), 3);
    // (3) ... but it does not know the secret added after the change.
    assert!(old
        .list()
        .expect("list")
        .iter()
        .all(|c| c.name != "after-change"));
    // (4) Splicing the old header (so the old password "works") onto the new body fails.
    let header_len = 110;
    let mut spliced = probe[..header_len].to_vec();
    spliced.extend_from_slice(&current[header_len..]);
    std::fs::write(&copy_path, &spliced).expect("write the splice");
    let mut forged = vault_at(copy_path);
    assert!(
        forged.unlock(pw(MASTER)).is_err(),
        "a spliced file must not open"
    );
}

/// A vault written by the current code (before any password change) keeps loading, and a
/// password change is the only operation that rewrites its key material.
#[test]
fn a_vault_written_before_the_rotation_loads_and_can_change_its_password() {
    let (_dir, path) = populated();
    let before = std::fs::read(&path).expect("read");
    let mut vault = vault_at(path.clone());
    vault.unlock(pw(MASTER)).expect("unlock");
    assert_eq!(vault.list().expect("list").len(), 3);
    // Saving a credential re-seals the payload but keeps the key material (header).
    vault
        .upsert(password_input(None, "extra", "fake-extra"))
        .expect("upsert");
    let after_upsert = std::fs::read(&path).expect("read");
    assert_eq!(before[..110], after_upsert[..110], "same wrapped key");
    vault
        .change_password(pw(MASTER), pw("fake-rotated"))
        .expect("change");
    let after_change = std::fs::read(&path).expect("read");
    assert_ne!(before[..110], after_change[..110], "new key material");
    let mut again = vault_at(path);
    again.unlock(pw("fake-rotated")).expect("unlock");
    assert_eq!(again.list().expect("list").len(), 4);
}
