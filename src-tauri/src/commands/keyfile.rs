//! `read_private_key_file`: the one narrow way the UI gets text from a file on disk. It exists
//! so "Load from file…" in the credential editor does not need a general file-reading
//! permission: the webview has no filesystem plugin, and this command refuses everything that
//! is not a small, regular file holding a PEM/OpenSSH private key.
//!
//! The path comes from the native open dialog (the only place the UI gets one). The command
//! cannot prove that, so it limits what a path can ever yield instead: size, file type and
//! content are all checked, and nothing is logged. Error messages never contain file content.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::AppError;
use portolano_core_model::ErrorCode;

/// A private key file is a few KiB (RSA 4096 in OpenSSH format is about 3.4 KiB).
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

fn io_error(e: &std::io::Error) -> AppError {
    match e.kind() {
        std::io::ErrorKind::NotFound => AppError::not_found("the file was not found"),
        std::io::ErrorKind::PermissionDenied => {
            AppError::new(ErrorCode::Io, "the file cannot be read: permission denied")
        }
        kind => AppError::new(ErrorCode::Io, format!("the file cannot be read ({kind})")),
    }
}

/// Whether `text` looks like a private key: a PEM block (`-----BEGIN ... PRIVATE KEY-----`,
/// which also covers the OpenSSH format, `RSA`/`EC` and PKCS#8) with its matching end line.
fn looks_like_private_key(text: &str) -> bool {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    let Some(first) = trimmed.lines().next() else {
        return false;
    };
    let first = first.trim();
    let Some(label) = first
        .strip_prefix("-----BEGIN ")
        .and_then(|rest| rest.strip_suffix("-----"))
    else {
        return false;
    };
    label.ends_with("PRIVATE KEY") && trimmed.contains(&format!("-----END {label}-----"))
}

/// Reads `path` if it is a regular file of at most [`MAX_KEY_FILE_BYTES`] that holds a private
/// key, and returns its text.
pub(crate) fn read_private_key_file_impl(path: &str) -> Result<String, AppError> {
    if path.trim().is_empty() {
        return Err(AppError::invalid_input("no file was chosen"));
    }
    // Opening a directory fails differently per platform (Windows: permission denied): say what
    // it is before trying.
    if Path::new(path).is_dir() {
        return Err(AppError::invalid_input(
            "the chosen path is not a regular file",
        ));
    }
    let file = File::open(Path::new(path)).map_err(|e| io_error(&e))?;
    // The checks use the opened handle, so the file cannot be swapped after them.
    let meta = file.metadata().map_err(|e| io_error(&e))?;
    if !meta.is_file() {
        return Err(AppError::invalid_input(
            "the chosen path is not a regular file",
        ));
    }
    if meta.len() > MAX_KEY_FILE_BYTES {
        return Err(AppError::invalid_input(
            "the file is too large to be a private key",
        ));
    }
    // `take` bounds the read even if the file grows after the size check.
    let mut bytes = Vec::new();
    file.take(MAX_KEY_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(&e))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_KEY_FILE_BYTES {
        return Err(AppError::invalid_input(
            "the file is too large to be a private key",
        ));
    }
    let not_a_key = || AppError::invalid_input("the file does not contain a private key");
    let text = String::from_utf8(bytes).map_err(|_| not_a_key())?;
    if !looks_like_private_key(&text) {
        return Err(not_a_key());
    }
    Ok(text)
}

/// Returns the text of the private key file at `path` (chosen with the native dialog), for the
/// credential editor to store in the vault. Refuses anything else; see the module docs.
#[tauri::command]
pub async fn read_private_key_file(path: String) -> Result<String, AppError> {
    read_private_key_file_impl(&path)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Command;

    use super::*;

    /// Scratch directory under `<repo>/.tmp/wp12/`, removed on drop.
    fn scratch() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/wp12");
        std::fs::create_dir_all(&base).expect("scratch base");
        tempfile::tempdir_in(base).expect("scratch dir")
    }

    /// Generates a throwaway key with `ssh-keygen` (no passphrase); returns its path.
    /// Only the tests that need real keys use it, and they run on macOS and Linux (the Windows
    /// runner's OpenSSH client is not guaranteed).
    #[cfg(unix)]
    fn keygen(dir: &Path, kind: &str, bits: Option<&str>) -> PathBuf {
        let path = dir.join(format!("key_{kind}"));
        let mut cmd = Command::new("ssh-keygen");
        cmd.args(["-q", "-t", kind, "-N", "", "-C", "test", "-f"])
            .arg(&path);
        if let Some(bits) = bits {
            cmd.args(["-b", bits]);
        }
        let status = cmd.status().expect("run ssh-keygen");
        assert!(status.success(), "ssh-keygen {kind} failed");
        path
    }

    fn code(result: Result<String, AppError>) -> ErrorCode {
        result.expect_err("must fail").code
    }

    #[cfg(unix)]
    #[test]
    fn valid_keys_are_returned_verbatim() {
        let dir = scratch();
        for (kind, bits) in [("ed25519", None), ("rsa", Some("2048"))] {
            let path = keygen(dir.path(), kind, bits);
            let expected = std::fs::read_to_string(&path).expect("read key");
            let got = read_private_key_file_impl(path.to_str().expect("utf8")).expect(kind);
            assert_eq!(got, expected, "{kind}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_public_key_is_not_a_private_key() {
        let dir = scratch();
        let path = keygen(dir.path(), "ed25519", None);
        let public = PathBuf::from(format!("{}.pub", path.display()));
        assert_eq!(
            code(read_private_key_file_impl(public.to_str().expect("utf8"))),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn oversize_files_are_refused() {
        let dir = scratch();
        let path = dir.path().join("big");
        let mut text = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        text.push_str(&"A".repeat(70 * 1024));
        text.push_str("\n-----END OPENSSH PRIVATE KEY-----\n");
        std::fs::write(&path, text).expect("write");
        assert_eq!(
            code(read_private_key_file_impl(path.to_str().expect("utf8"))),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn non_key_files_are_refused_without_echoing_their_content() {
        let dir = scratch();
        let cases: [(&str, &[u8]); 5] = [
            ("text", b"just some secret-looking text\n"),
            ("empty", b""),
            ("binary", &[0xff, 0xfe, 0x00, 0x01]),
            (
                "unterminated",
                b"-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n",
            ),
            (
                "certificate",
                b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
            ),
        ];
        for (name, content) in cases {
            let path = dir.path().join(name);
            std::fs::write(&path, content).expect("write");
            let err = read_private_key_file_impl(path.to_str().expect("utf8")).expect_err(name);
            assert_eq!(err.code, ErrorCode::InvalidInput, "{name}");
            assert!(!err.message.contains("secret-looking"), "{name}");
            assert!(
                !err.message.contains(dir.path().to_str().expect("utf8")),
                "{name}"
            );
        }
    }

    #[test]
    fn directories_are_refused() {
        let dir = scratch();
        assert_eq!(
            code(read_private_key_file_impl(
                dir.path().to_str().expect("utf8")
            )),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn missing_and_empty_paths_have_stable_codes() {
        let dir = scratch();
        let missing = dir.path().join("nope");
        assert_eq!(
            code(read_private_key_file_impl(missing.to_str().expect("utf8"))),
            ErrorCode::NotFound
        );
        assert_eq!(
            code(read_private_key_file_impl("")),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(read_private_key_file_impl("  ")),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn the_key_text_is_accepted_with_pem_variants() {
        for label in [
            "RSA PRIVATE KEY",
            "EC PRIVATE KEY",
            "PRIVATE KEY",
            "ENCRYPTED PRIVATE KEY",
        ] {
            let text = format!("-----BEGIN {label}-----\nAAAA\n-----END {label}-----\n");
            assert!(looks_like_private_key(&text), "{label}");
        }
        assert!(!looks_like_private_key(
            "-----BEGIN PUBLIC KEY-----\nA\n-----END PUBLIC KEY-----"
        ));
        assert!(!looks_like_private_key(
            "-----BEGIN RSA PRIVATE KEY-----\nA\n-----END EC PRIVATE KEY-----"
        ));
    }
}
