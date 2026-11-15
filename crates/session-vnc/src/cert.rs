//! Server certificate trust-on-first-use for VNC (VeNCrypt X509 subtypes).
//!
//! VNC servers almost always present self-signed certificates, so chain validation is not
//! useful. Instead the SHA-256 fingerprint of the leaf certificate is pinned per `host:port`
//! the first time the user accepts it; a different fingerprint later is reported as a change.
//!
//! File format (`vnc_known_certs` in the data directory), one entry per line:
//!
//! ```text
//! <host>\t<port>\t<AA:BB:...:FF>
//! ```
//!
//! `host` is lower-cased. Lines starting with `#` and malformed lines are ignored. Writes are
//! atomic (temporary file + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// File name inside the data directory.
pub const KNOWN_CERTS_FILE: &str = "vnc_known_certs";

/// Serializes read-modify-write cycles of every store in this process.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// `AA:BB:...` upper-case hex SHA-256 of a DER certificate.
pub fn fingerprint_sha256(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut out = String::with_capacity(digest.len() * 3);
    for (i, byte) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{byte:02X}"));
    }
    out
}

/// Outcome of comparing a presented fingerprint with the stored one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertCheck {
    /// No entry for this host and port.
    Unknown,
    Matches,
    /// An entry exists with another fingerprint.
    Changed {
        stored: String,
    },
}

#[derive(Debug, Clone)]
pub struct CertStore {
    path: PathBuf,
}

fn key(host: &str, port: u16) -> (String, u16) {
    (host.trim().to_lowercase(), port)
}

impl CertStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The store of a data directory.
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self::new(data_dir.join(KNOWN_CERTS_FILE))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> io::Result<Vec<(String, u16, String)>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let host = parts.next()?.to_owned();
                let port = parts.next()?.parse().ok()?;
                let fingerprint = parts.next()?.trim().to_owned();
                (!host.is_empty() && !fingerprint.is_empty()).then_some((host, port, fingerprint))
            })
            .collect())
    }

    pub fn check(&self, host: &str, port: u16, fingerprint: &str) -> io::Result<CertCheck> {
        let wanted = key(host, port);
        let entries = self.read()?;
        Ok(
            match entries
                .iter()
                .find(|(h, p, _)| (h.as_str(), *p) == (wanted.0.as_str(), wanted.1))
            {
                None => CertCheck::Unknown,
                Some((_, _, stored)) if stored.eq_ignore_ascii_case(fingerprint) => {
                    CertCheck::Matches
                }
                Some((_, _, stored)) => CertCheck::Changed {
                    stored: stored.clone(),
                },
            },
        )
    }

    /// Whether a certificate is pinned for `host:port`: the host has been seen using TLS, so
    /// a server that stops offering it is a downgrade, not a different kind of server.
    pub fn has_pin(&self, host: &str, port: u16) -> io::Result<bool> {
        let wanted = key(host, port);
        Ok(self
            .read()?
            .iter()
            .any(|(h, p, _)| (h.as_str(), *p) == (wanted.0.as_str(), wanted.1)))
    }

    /// Stores (or replaces) the pinned fingerprint of `host:port`.
    pub fn remember(&self, host: &str, port: u16, fingerprint: &str) -> io::Result<()> {
        if !portolano_core_model::host_is_storable(host.trim()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the host contains spaces or control characters",
            ));
        }
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = key(host, port);
        let mut entries = self.read()?;
        entries.retain(|(h, p, _)| (h.as_str(), *p) != (wanted.0.as_str(), wanted.1));
        entries.push((wanted.0, wanted.1, fingerprint.to_owned()));
        self.write(&entries)
    }

    /// Removes the pinned fingerprint of `host:port`. `true` when there was one. This is the
    /// only way to trust a different certificate of a host: the user does it on purpose.
    pub fn forget(&self, host: &str, port: u16) -> io::Result<bool> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = key(host, port);
        let mut entries = self.read()?;
        let before = entries.len();
        entries.retain(|(h, p, _)| (h.as_str(), *p) != (wanted.0.as_str(), wanted.1));
        if entries.len() == before {
            return Ok(false);
        }
        self.write(&entries)?;
        Ok(true)
    }

    /// Atomic write of the whole store, readable by the user only.
    fn write(&self, entries: &[(String, u16, String)]) -> io::Result<()> {
        let mut text = String::from("# Portolano VNC server certificates: host, port, SHA-256\n");
        for (h, p, f) in entries {
            text.push_str(&format!("{h}\t{p}\t{f}\n"));
        }
        portolano_core_model::private_fs::write_private_atomic(&self.path, text.as_bytes())
    }
}

/// File name inside the data directory.
pub const UNENCRYPTED_HOSTS_FILE: &str = "vnc_unencrypted_hosts";

/// The `host:port` pairs for which the user agreed to send a VNC password without encryption
/// (VNC Authentication over plain TCP). Asked once per host, like a certificate pin.
///
/// File format: `<host>\t<port>` per line; lines starting with `#` and malformed lines are
/// ignored. Writes are atomic and the file is readable by the user only.
#[derive(Debug, Clone)]
pub struct PlainStore {
    path: PathBuf,
}

impl PlainStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self::new(data_dir.join(UNENCRYPTED_HOSTS_FILE))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> io::Result<Vec<(String, u16)>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let host = parts.next()?.to_owned();
                let port = parts.next()?.trim().parse().ok()?;
                (!host.is_empty()).then_some((host, port))
            })
            .collect())
    }

    fn write(&self, entries: &[(String, u16)]) -> io::Result<()> {
        let mut text =
            String::from("# Portolano VNC hosts allowed to authenticate without encryption\n");
        for (h, p) in entries {
            text.push_str(&format!("{h}\t{p}\n"));
        }
        portolano_core_model::private_fs::write_private_atomic(&self.path, text.as_bytes())
    }

    /// Whether the user agreed for `host:port`.
    pub fn allowed(&self, host: &str, port: u16) -> io::Result<bool> {
        let wanted = key(host, port);
        Ok(self
            .read()?
            .iter()
            .any(|(h, p)| (h, *p) == (&wanted.0, wanted.1)))
    }

    /// Records the agreement for `host:port`.
    pub fn allow(&self, host: &str, port: u16) -> io::Result<()> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = key(host, port);
        let mut entries = self.read()?;
        if entries
            .iter()
            .any(|(h, p)| (h, *p) == (&wanted.0, wanted.1))
        {
            return Ok(());
        }
        entries.push(wanted);
        self.write(&entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_colon_separated_upper_hex() {
        // SHA-256 of the empty input.
        assert_eq!(
            fingerprint_sha256(b""),
            "E3:B0:C4:42:98:FC:1C:14:9A:FB:F4:C8:99:6F:B9:24:27:AE:41:E4:64:9B:93:4C:A4:95:99:1B:78:52:B8:55"
        );
    }

    #[test]
    fn tofu_cycle_unknown_match_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CertStore::in_data_dir(dir.path());
        assert_eq!(
            store.check("Host", 5900, "AA").expect("check"),
            CertCheck::Unknown
        );

        store.remember("Host", 5900, "AA").expect("remember");
        assert_eq!(
            store.check("host", 5900, "aa").expect("check"),
            CertCheck::Matches
        );
        assert_eq!(
            store.check("host", 5900, "BB").expect("check"),
            CertCheck::Changed {
                stored: "AA".into()
            }
        );
        // Another port is another entry.
        assert_eq!(
            store.check("host", 3390, "AA").expect("check"),
            CertCheck::Unknown
        );

        store.remember("host", 5900, "BB").expect("replace");
        assert_eq!(
            store.check("host", 5900, "BB").expect("check"),
            CertCheck::Matches
        );
        let text = fs::read_to_string(store.path()).expect("read");
        assert_eq!(text.lines().filter(|l| l.starts_with("host\t")).count(), 1);
    }

    #[test]
    fn forget_removes_only_that_entry_and_the_store_stays_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CertStore::in_data_dir(dir.path());
        store.remember("a", 1, "AA").expect("remember");
        store.remember("b", 1, "BB").expect("remember");
        assert!(store.forget("A", 1).expect("forget"));
        assert!(!store.forget("a", 1).expect("forget again"));
        assert_eq!(
            store.check("a", 1, "AA").expect("check"),
            CertCheck::Unknown
        );
        assert_eq!(
            store.check("b", 1, "BB").expect("check"),
            CertCheck::Matches
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn pins_are_reported_and_the_plain_store_remembers_per_host() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CertStore::in_data_dir(dir.path());
        assert!(!store.has_pin("h", 5901).expect("pin"));
        store.remember("H", 5901, "AA").expect("remember");
        assert!(store.has_pin("h", 5901).expect("pin"));
        assert!(!store.has_pin("h", 5900).expect("pin"));

        let plain = PlainStore::in_data_dir(dir.path());
        assert!(!plain.allowed("h", 5900).expect("allowed"));
        plain.allow("H", 5900).expect("allow");
        plain.allow("h", 5900).expect("allow twice");
        assert!(plain.allowed("h", 5900).expect("allowed"));
        assert!(!plain.allowed("h", 5901).expect("another port"));
        assert_eq!(
            fs::read_to_string(plain.path())
                .expect("read")
                .lines()
                .filter(|l| l.starts_with("h\t"))
                .count(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(plain.path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_host_that_would_corrupt_the_line_format_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CertStore::in_data_dir(dir.path());
        for host in ["a b", "a\tb", "a\nb", "a\rb"] {
            let err = store.remember(host, 1, "AA").expect_err(host);
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!store.path().exists());
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CertStore::in_data_dir(dir.path());
        fs::write(store.path(), "garbage\nh\tnotaport\tAA\nh\t1\tAA\n").expect("write");
        assert_eq!(
            store.check("h", 1, "AA").expect("check"),
            CertCheck::Matches
        );
        assert_eq!(
            store.check("h", 2, "AA").expect("check"),
            CertCheck::Unknown
        );
    }
}
