//! Own known_hosts store (never the user's `~/.ssh`): one line per key,
//! `<host-pattern> <algorithm> <base64>`, in the OpenSSH layout (unhashed).
//!
//! The host pattern is `host` for port 22 and `[host]:port` otherwise (always bracketed for
//! IPv6 literals). Hosts are compared case-insensitively. Lines this module does not
//! understand (comments, hashed hosts, markers) are preserved on rewrite and ignored on read.
//!
//! Verification rule (stricter than the spike): if ANY key is stored for the host and port and
//! the presented key is not one of them, the result is `Changed`, whatever the algorithm.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use russh::keys::{HashAlg, PublicKey};

use crate::{KnownHostEntry, TermError};

/// Serializes read-modify-write cycles inside the process.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// Nothing is stored for this host and port.
    Unknown,
    /// One of the stored keys is the presented key.
    Matches,
    /// Keys are stored for this host and port and none of them is the presented key.
    Changed { stored: Vec<StoredKey> },
}

#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
}

pub(crate) fn pattern(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 && !host.contains(':') {
        host
    } else {
        format!("[{host}]:{port}")
    }
}

/// Inverse of [`pattern`]; `None` for patterns this store does not write.
fn parse_pattern(p: &str) -> Option<(String, u16)> {
    if p.is_empty() || p.starts_with('|') || p.starts_with('@') || p.contains(',') {
        return None;
    }
    if let Some(rest) = p.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(n) => n.parse().ok()?,
            None if tail.is_empty() => 22,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    Some((p.to_string(), 22))
}

/// `(algorithm, base64)` of a public key in OpenSSH wire format.
fn key_parts(key: &PublicKey) -> Result<(String, String), TermError> {
    let line = key
        .to_openssh()
        .map_err(|e| TermError::Invalid(format!("cannot encode the host key: {e}")))?;
    let mut it = line.split_whitespace();
    match (it.next(), it.next()) {
        (Some(alg), Some(b64)) => Ok((alg.to_string(), b64.to_string())),
        _ => Err(TermError::Invalid("malformed encoded host key".into())),
    }
}

/// `SHA256:...` fingerprint, as OpenSSH prints it.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn stored_fingerprint(base64: &str) -> String {
    russh::keys::parse_public_key_base64(base64)
        .map(|k| fingerprint(&k))
        .unwrap_or_else(|_| "<unreadable stored key>".to_string())
}

struct Line<'a> {
    host: String,
    port: u16,
    algorithm: &'a str,
    base64: &'a str,
}

fn parse_line(line: &str) -> Option<Line<'_>> {
    let mut it = line.split_whitespace();
    let (p, algorithm, base64) = (it.next()?, it.next()?, it.next()?);
    if p.starts_with('#') {
        return None;
    }
    let (host, port) = parse_pattern(p)?;
    Some(Line {
        host,
        port,
        algorithm,
        base64,
    })
}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<String, TermError> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes `content` through a temporary file in the same directory and a rename.
    fn write(&self, content: &str) -> Result<(), TermError> {
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        portolano_core_model::private_fs::restrict_file(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<KnownHostEntry>, TermError> {
        let content = self.read()?;
        Ok(content
            .lines()
            .filter_map(parse_line)
            .map(|l| KnownHostEntry {
                host: l.host,
                port: l.port,
                algorithm: l.algorithm.to_string(),
                fingerprint: stored_fingerprint(l.base64),
            })
            .collect())
    }

    /// Key algorithms stored for `host:port` (used to order the algorithms offered to the
    /// server, so that a known key is the one it presents).
    pub fn algorithms(&self, host: &str, port: u16) -> Result<Vec<String>, TermError> {
        let host = host.to_ascii_lowercase();
        let content = self.read()?;
        Ok(content
            .lines()
            .filter_map(parse_line)
            .filter(|l| l.host == host && l.port == port)
            .map(|l| l.algorithm.to_string())
            .collect())
    }

    pub fn check(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Result<HostKeyStatus, TermError> {
        let (_, presented) = key_parts(key)?;
        let host = host.to_ascii_lowercase();
        let content = self.read()?;
        let mut stored = Vec::new();
        for line in content.lines().filter_map(parse_line) {
            if line.host != host || line.port != port {
                continue;
            }
            if line.base64 == presented {
                return Ok(HostKeyStatus::Matches);
            }
            stored.push(StoredKey {
                algorithm: line.algorithm.to_string(),
                fingerprint: stored_fingerprint(line.base64),
            });
        }
        if stored.is_empty() {
            Ok(HostKeyStatus::Unknown)
        } else {
            Ok(HostKeyStatus::Changed { stored })
        }
    }

    /// Stores `key` for `host:port` (no-op when it is already stored).
    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), TermError> {
        if !portolano_core_model::host_is_storable(host) {
            return Err(TermError::Invalid(
                "the host contains spaces or control characters".into(),
            ));
        }
        let (algorithm, base64) = key_parts(key)?;
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut content = self.read()?;
        let host_l = host.to_ascii_lowercase();
        let stored: Vec<_> = content
            .lines()
            .filter_map(parse_line)
            .filter(|l| l.host == host_l && l.port == port)
            .collect();
        if stored.iter().any(|l| l.base64 == base64) {
            return Ok(());
        }
        // The key was unknown when the prompt was shown. A different one on file now means
        // another session stored its own in the meantime: two answers for one host cannot both
        // be right, so this one is refused (the next attempt reports a changed key).
        if !stored.is_empty() {
            return Err(TermError::Invalid(
                "a different host key for this host was stored in the meantime".into(),
            ));
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!("{} {algorithm} {base64}\n", pattern(host, port)));
        self.write(&content)
    }

    /// Removes every key stored for `host:port`; returns how many lines were removed.
    pub fn remove(&self, host: &str, port: u16) -> Result<usize, TermError> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let host = host.to_ascii_lowercase();
        let content = self.read()?;
        let mut removed = 0;
        let mut kept = String::with_capacity(content.len());
        for line in content.lines() {
            match parse_line(line) {
                Some(l) if l.host == host && l.port == port => removed += 1,
                _ => {
                    kept.push_str(line);
                    kept.push('\n');
                }
            }
        }
        if removed > 0 {
            self.write(&kept)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway public keys generated with ssh-keygen for these tests.
    const ED_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAR+B36fVH0lW5H4UYc/s59RiDAh+akCM35NyILC1L2a";
    const ED_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAICsdM+rC22ibB4bJRNCBtV0QHH/ONxI5/URcdlsjaF2V";
    const EC_A: &str = "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBJXoJIxw/tYyRakF30h/IxxCrSkt/wb7NDzy6iVq5UeXyuYdEBCoCyEVttHTeB/uLF2Wm5joiJvAuNLuWA4UZ+8=";

    fn key(base64: &str) -> PublicKey {
        russh::keys::parse_public_key_base64(base64).expect("parse key")
    }

    fn store() -> (tempfile::TempDir, KnownHosts) {
        let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.tmp/wp5");
        std::fs::create_dir_all(base).expect("create base dir");
        let dir = tempfile::tempdir_in(base).expect("temp dir");
        let kh = KnownHosts::new(dir.path().join("known_hosts"));
        (dir, kh)
    }

    #[test]
    fn pattern_round_trip() {
        assert_eq!(pattern("Example.COM", 22), "example.com");
        assert_eq!(pattern("h", 2222), "[h]:2222");
        assert_eq!(pattern("::1", 22), "[::1]:22");
        assert_eq!(
            parse_pattern("example.com"),
            Some(("example.com".into(), 22))
        );
        assert_eq!(parse_pattern("[h]:2222"), Some(("h".into(), 2222)));
        assert_eq!(parse_pattern("[::1]:22"), Some(("::1".into(), 22)));
        assert_eq!(parse_pattern("|1|abc|def"), None);
        assert_eq!(parse_pattern("a,b"), None);
        assert_eq!(parse_pattern("[h]:notaport"), None);
    }

    #[test]
    fn unknown_then_matches_after_add() {
        let (_d, kh) = store();
        let k = key(ED_A);
        assert_eq!(
            kh.check("h", 22, &k).expect("check"),
            HostKeyStatus::Unknown
        );
        kh.add("h", 22, &k).expect("add");
        assert_eq!(
            kh.check("H", 22, &k).expect("check"),
            HostKeyStatus::Matches
        );
        // Another port is another host.
        assert_eq!(
            kh.check("h", 2222, &k).expect("check"),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn same_algorithm_different_key_is_changed() {
        let (_d, kh) = store();
        let stored = key(ED_A);
        kh.add("h", 2222, &stored).expect("add");
        let other = key(ED_B);
        match kh.check("h", 2222, &other).expect("check") {
            HostKeyStatus::Changed { stored: s } => {
                assert_eq!(s.len(), 1);
                assert_eq!(s[0].fingerprint, fingerprint(&stored));
            }
            other => panic!("expected Changed, got {other:?}"),
        }
    }

    #[test]
    fn different_algorithm_for_a_known_host_is_changed_not_unknown() {
        let (_d, kh) = store();
        let stored = key(ED_A);
        kh.add("h", 22, &stored).expect("add");
        let presented = key(EC_A);
        assert!(matches!(
            kh.check("h", 22, &presented).expect("check"),
            HostKeyStatus::Changed { .. }
        ));
    }

    #[test]
    fn a_second_key_for_the_same_host_is_refused_not_added() {
        let (_d, kh) = store();
        kh.add("h", 22, &key(ED_A)).expect("add");
        // Two sessions asked about the same new host; the second answer arrives after the first
        // one was stored.
        let err = kh.add("h", 22, &key(ED_B)).expect_err("second key");
        assert!(err.to_string().contains("in the meantime"), "{err}");
        let entries = kh.list().expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].fingerprint, fingerprint(&key(ED_A)));
        // The same key again is still a no-op, and another host is unaffected.
        kh.add("h", 22, &key(ED_A)).expect("same key");
        kh.add("other", 22, &key(ED_B)).expect("another host");
    }

    #[test]
    fn hosts_that_would_corrupt_the_file_are_refused() {
        let (_d, kh) = store();
        for host in ["a b", "a\nb", "a\tb"] {
            assert!(kh.add(host, 22, &key(ED_A)).is_err(), "{host:?}");
        }
        assert!(!kh.path().exists());
    }

    #[test]
    fn list_remove_and_foreign_lines_are_preserved() {
        let (_d, kh) = store();
        let a = key(ED_A);
        let b = key(ED_B);
        std::fs::write(kh.path(), "# my comment\n|1|salt|hash ssh-ed25519 AAAA\n").expect("seed");
        kh.add("a.example", 22, &a).expect("add");
        kh.add("b.example", 2222, &b).expect("add");
        let list = kh.list().expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].host, "a.example");
        assert_eq!(list[0].port, 22);
        assert_eq!(list[0].algorithm, "ssh-ed25519");
        assert_eq!(list[0].fingerprint, fingerprint(&a));
        assert_eq!(list[1].port, 2222);

        assert_eq!(kh.remove("a.example", 22).expect("remove"), 1);
        assert_eq!(kh.remove("a.example", 22).expect("remove again"), 0);
        let text = std::fs::read_to_string(kh.path()).expect("read");
        assert!(text.contains("# my comment"));
        assert!(text.contains("|1|salt|hash"));
        assert_eq!(kh.list().expect("list").len(), 1);
    }

    #[test]
    fn missing_file_lists_empty_and_add_is_idempotent() {
        let (_d, kh) = store();
        assert!(kh.list().expect("list").is_empty());
        let k = key(ED_A);
        kh.add("h", 22, &k).expect("add");
        kh.add("h", 22, &k).expect("add again");
        assert_eq!(kh.list().expect("list").len(), 1);
        assert_eq!(kh.algorithms("h", 22).expect("algs"), vec!["ssh-ed25519"]);
    }
}
