//! Restrictive permissions for the files Portolano keeps in its data directory.
//!
//! The documents are not secret, but they name hosts, user names and notes, so other local
//! users have no business reading them: the data directory is `0700` and the files in it are
//! `0600` on Unix. On Windows nothing is set explicitly: the directory inherits the ACL of the
//! user profile, which already restricts it to the user.
//!
//! Permissions are applied when a file or folder is created and corrected again at start-up
//! ([`harden_data_dir`]), so data written by an older version is tightened as well.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Mode of the data directory.
pub const DIR_MODE: u32 = 0o700;
/// Mode of every file in the data directory.
pub const FILE_MODE: u32 = 0o600;

/// Creates `dir` (and its parents) and restricts `dir` itself to the user.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    restrict_dir(dir)
}

/// Restricts an existing directory to the user. A no-op on Windows.
pub fn restrict_dir(dir: &Path) -> io::Result<()> {
    set_mode(dir, DIR_MODE)
}

/// Restricts an existing file to the user. A missing file is not an error.
pub fn restrict_file(path: &Path) -> io::Result<()> {
    match set_mode(path, FILE_MODE) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let current = fs::metadata(path)?.permissions().mode() & 0o7777;
    if current == mode {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// `OpenOptions` that create a file readable by the user only.
pub fn private_create_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    options
}

/// Writes `bytes` to `path` through a temporary file in the same directory and a rename. The
/// file is created `0600` before any byte is written, so there is no window with wider access.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    let tmp = PathBuf::from(name);
    let written = (|| -> io::Result<()> {
        let mut file = private_create_options().open(&tmp)?;
        // An older, wider temporary file would keep its mode: correct it explicitly.
        restrict_file(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // Windows cannot rename a file that is still open here: close it first.
        drop(file);
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        if let Err(e) = fs::remove_file(&tmp) {
            if e.kind() != io::ErrorKind::NotFound {
                tracing::warn!(error = %e, "could not remove a temporary file");
            }
        }
    }
    written
}

/// The files of the data directory that exist: their modes are corrected.
const DATA_FILES: &[&str] = &[
    "connections.json",
    "connections.json.bak",
    "connections.json.tmp",
    "connections.json.bak.tmp",
    "vault.bin",
    "vault.bin.tmp",
    "known_hosts",
    "known_hosts.tmp",
    "rdp_known_certs",
    "rdp_known_certs.tmp",
    "vnc_known_certs",
    "vnc_known_certs.tmp",
    "vnc_unencrypted_hosts",
    "vnc_unencrypted_hosts.tmp",
    "settings.json",
    "settings.json.tmp",
];

/// Creates the data directory `0700` and corrects the mode of the data files already there.
/// Failures on single files are logged and do not stop the start-up; the directory failing is
/// an error.
pub fn harden_data_dir(dir: &Path) -> io::Result<()> {
    create_private_dir(dir)?;
    for name in DATA_FILES {
        if let Err(e) = restrict_file(&dir.join(name)) {
            tracing::warn!(file = name, error = %e, "could not restrict the permissions of a data file");
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp/secfix/private_fs")
            .join(name);
        if let Err(e) = fs::remove_dir_all(&dir) {
            assert_eq!(e.kind(), io::ErrorKind::NotFound);
        }
        fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn private_dir_is_0700_even_when_it_exists_wide() {
        let base = scratch("dir");
        let dir = base.join("data");
        fs::create_dir(&dir).expect("create");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");
        create_private_dir(&dir).expect("private");
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn atomic_write_creates_0600_and_replaces() {
        let base = scratch("write");
        let file = base.join("known");
        write_private_atomic(&file, b"one").expect("write");
        assert_eq!(mode(&file), 0o600);
        write_private_atomic(&file, b"two").expect("rewrite");
        assert_eq!(fs::read(&file).expect("read"), b"two");
        assert_eq!(mode(&file), 0o600);
        assert!(!base.join("known.tmp").exists());
    }

    #[test]
    fn hardening_fixes_what_an_older_version_left_wide() {
        let base = scratch("harden");
        let dir = base.join("data");
        fs::create_dir(&dir).expect("create");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");
        for name in ["connections.json", "rdp_known_certs", "vault.bin"] {
            fs::write(dir.join(name), b"x").expect("seed");
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o644)).expect("chmod");
        }
        fs::write(dir.join("unrelated.txt"), b"x").expect("seed");
        fs::set_permissions(dir.join("unrelated.txt"), fs::Permissions::from_mode(0o644))
            .expect("chmod");

        harden_data_dir(&dir).expect("harden");

        assert_eq!(mode(&dir), 0o700);
        for name in ["connections.json", "rdp_known_certs", "vault.bin"] {
            assert_eq!(mode(&dir.join(name)), 0o600, "{name}");
        }
        // Only the files Portolano owns are touched.
        assert_eq!(mode(&dir.join("unrelated.txt")), 0o644);
    }
}

/// Runs on every platform (the module above is unix-only): replacing an existing file twice
/// must work, which on Windows needs the temporary file closed before the rename.
#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn write_private_atomic_replaces_an_existing_file_repeatedly() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp/wp12")
            .join(format!("private-fs-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("data.bin");
        for round in 0..3u8 {
            write_private_atomic(&path, &[round; 8]).expect("write");
            assert_eq!(fs::read(&path).expect("read"), vec![round; 8]);
        }
        assert!(!dir.join("data.bin.tmp").exists());
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
