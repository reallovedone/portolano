//! Where a share file comes from: a path or an `https://` address. Both are read with the same
//! 5 MiB limit, checked while reading (before anything is parsed), and both report an error text
//! that names neither a path nor any file content.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use portolano_core_model::ShareLocation;
use portolano_shares::format::MAX_SHARE_BYTES;
use portolano_shares::ShareError;

/// How long one request may take, connecting and reading included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a read from a file path (often a network folder) may take.
const PATH_TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects followed before giving up (each one must stay on `https://`).
const MAX_REDIRECTS: usize = 5;

/// What identified the content last time, so that an unchanged share is not read again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Validator {
    #[default]
    None,
    /// Modification time and size of a file.
    File {
        modified: Option<SystemTime>,
        len: u64,
    },
    /// The `ETag` of an HTTP answer.
    ETag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    NotModified,
    Bytes { data: Vec<u8>, validator: Validator },
}

/// Why a share file could not be fetched. The texts are shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("The share file is larger than the 5 MiB limit.")]
    TooLarge,
    #[error("The share file could not be found or read.")]
    FileUnavailable,
    #[error("Only https:// addresses are accepted for shared folders.")]
    NotHttps,
    #[error("The address of the shared folder is not valid.")]
    InvalidAddress,
    #[error("The address must not contain a user name or password.")]
    CredentialsInUrl,
    #[error("The server redirected to an address that is not https://.")]
    InsecureRedirect,
    #[error("The server did not answer in time.")]
    Timeout,
    #[error("The server could not be reached.")]
    Unreachable,
    #[error("The server answered with an error (HTTP {0}).")]
    Http(u16),
    #[error("The transfer failed before the whole file arrived.")]
    Interrupted,
}

impl TransportError {
    /// The source could not be reached or read right now (as opposed to a refused address
    /// or file): the share is offline and keeps its last good copy.
    pub fn is_offline(&self) -> bool {
        matches!(
            self,
            Self::FileUnavailable
                | Self::Timeout
                | Self::Unreachable
                | Self::Http(_)
                | Self::Interrupted
        )
    }
}

impl From<ShareError> for TransportError {
    fn from(e: ShareError) -> Self {
        match e {
            ShareError::TooLarge => Self::TooLarge,
            _ => Self::FileUnavailable,
        }
    }
}

/// Fetches the share at `location`. `previous` is the validator of the copy already held:
/// when the source still matches it, nothing is read.
pub async fn fetch(
    location: &ShareLocation,
    previous: &Validator,
) -> Result<Fetched, TransportError> {
    match location {
        ShareLocation::Path { value } => {
            let path = PathBuf::from(value);
            let previous = previous.clone();
            // A network folder that stops answering can block a read for minutes; the sync
            // gives up after a while (the blocked read ends on its own later).
            let read = tokio::task::spawn_blocking(move || fetch_path(&path, &previous));
            tokio::time::timeout(PATH_TIMEOUT, read)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(|_| TransportError::FileUnavailable)?
        }
        ShareLocation::Url { value } => fetch_url(value, previous).await,
    }
}

fn fetch_path(path: &Path, previous: &Validator) -> Result<Fetched, TransportError> {
    let meta = fs::metadata(path).map_err(|_| TransportError::FileUnavailable)?;
    if !meta.is_file() {
        return Err(TransportError::FileUnavailable);
    }
    // Taken before reading: a change during the read makes the next sync read again.
    let validator = Validator::File {
        modified: meta.modified().ok(),
        len: meta.len(),
    };
    if validator == *previous {
        return Ok(Fetched::NotModified);
    }
    let data = read_limited(path)?;
    Ok(Fetched::Bytes { data, validator })
}

/// Reads a whole file, refusing more than [`MAX_SHARE_BYTES`] (also when it grows while read).
pub fn read_limited(path: &Path) -> Result<Vec<u8>, TransportError> {
    let file = fs::File::open(path).map_err(|_| TransportError::FileUnavailable)?;
    let len = file
        .metadata()
        .map_err(|_| TransportError::FileUnavailable)?
        .len();
    if len > MAX_SHARE_BYTES as u64 {
        return Err(TransportError::TooLarge);
    }
    let mut data = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(MAX_SHARE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|_| TransportError::FileUnavailable)?;
    if data.len() > MAX_SHARE_BYTES {
        return Err(TransportError::TooLarge);
    }
    Ok(data)
}

/// Checks the address before any network use: `https://` with a host, nothing else.
fn parse_https(text: &str) -> Result<reqwest::Url, TransportError> {
    let url = reqwest::Url::parse(text.trim()).map_err(|_| TransportError::InvalidAddress)?;
    if url.scheme() != "https" {
        return Err(TransportError::NotHttps);
    }
    if url.host_str().is_none() {
        return Err(TransportError::InvalidAddress);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(TransportError::CredentialsInUrl);
    }
    Ok(url)
}

/// Whether a redirect target may be followed: https only.
fn redirect_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
}

fn map_request_error(e: &reqwest::Error) -> TransportError {
    if e.is_redirect() {
        TransportError::InsecureRedirect
    } else if e.is_timeout() {
        TransportError::Timeout
    } else {
        TransportError::Unreachable
    }
}

async fn fetch_url(text: &str, previous: &Validator) -> Result<Fetched, TransportError> {
    let url = parse_https(text)?;
    // The platform certificate verifier is what reqwest's `rustls` feature uses. The initial
    // address was checked above; the policy refuses any redirect that leaves https, with an
    // error of the redirect kind so it is reported as such.
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if !redirect_allowed(attempt.url()) {
                attempt.error("redirect to an address that is not https")
            } else if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(concat!("Portolano/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| TransportError::Unreachable)?;
    let mut request = client.get(url);
    if let Validator::ETag(tag) = previous {
        request = request.header(reqwest::header::IF_NONE_MATCH, tag.as_str());
    }
    let mut response = request.send().await.map_err(|e| map_request_error(&e))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    if !status.is_success() {
        return Err(TransportError::Http(status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_SHARE_BYTES as u64)
    {
        return Err(TransportError::TooLarge);
    }
    let validator = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map_or(Validator::None, |tag| Validator::ETag(tag.to_owned()));
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        if e.is_timeout() {
            TransportError::Timeout
        } else {
            TransportError::Interrupted
        }
    })? {
        if data.len() + chunk.len() > MAX_SHARE_BYTES {
            return Err(TransportError::TooLarge);
        }
        data.extend_from_slice(&chunk);
    }
    Ok(Fetched::Bytes { data, validator })
}

/// Writes `bytes` to `path` through a temporary file in the same directory and a rename, so a
/// reader never sees half a file. The file gets the permissions of a normal new file (other
/// people have to be able to read a share): the private variant of the data directory is for
/// the local cache only.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = path.as_os_str().to_owned();
    // A random suffix: nobody can predict (or pre-create) the name in a shared directory.
    name.push(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let tmp = PathBuf::from(name);
    let written = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // Windows cannot rename a file that is still open.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/shares");
        fs::create_dir_all(&base).expect("base");
        tempfile::tempdir_in(base).expect("scratch")
    }

    fn url(text: &str) -> ShareLocation {
        ShareLocation::Url { value: text.into() }
    }

    #[tokio::test]
    async fn url_with_embedded_credentials_is_refused() {
        for text in [
            "https://user:secret@example.net/x",
            "https://user@example.net/x",
        ] {
            assert_eq!(
                fetch(&url(text), &Validator::None).await,
                Err(TransportError::CredentialsInUrl),
                "{text}"
            );
        }
    }

    #[test]
    fn only_https_redirects_are_followed() {
        let allowed = |t: &str| redirect_allowed(&reqwest::Url::parse(t).expect("url"));
        assert!(allowed("https://example.net/x"));
        assert!(!allowed("http://example.net/x"));
        assert_eq!(
            TransportError::InsecureRedirect.to_string(),
            "The server redirected to an address that is not https://."
        );
    }

    #[tokio::test]
    async fn url_location_refuses_http() {
        // Refused before any connection is attempted.
        for text in [
            "http://example.net/team.portolano-share",
            "HTTP://example.net/x",
            "ftp://example.net/x",
            "file:///etc/passwd",
        ] {
            assert_eq!(
                fetch(&url(text), &Validator::None).await,
                Err(TransportError::NotHttps),
                "{text}"
            );
        }
        assert_eq!(
            fetch(&url("not a url"), &Validator::None).await,
            Err(TransportError::InvalidAddress)
        );
        assert!(parse_https("https://example.net/team.portolano-share").is_ok());
    }

    #[tokio::test]
    async fn path_fetch_returns_bytes_then_not_modified() {
        let dir = scratch();
        let file = dir.path().join("a.portolano-share");
        fs::write(&file, b"hello").expect("write");
        let location = ShareLocation::Path {
            value: file.to_string_lossy().into_owned(),
        };
        let Fetched::Bytes { data, validator } =
            fetch(&location, &Validator::None).await.expect("first")
        else {
            panic!("bytes expected");
        };
        assert_eq!(data, b"hello");
        assert_eq!(fetch(&location, &validator).await, Ok(Fetched::NotModified));
        fs::write(&file, b"hello!").expect("rewrite");
        assert!(matches!(
            fetch(&location, &validator).await,
            Ok(Fetched::Bytes { .. })
        ));
    }

    #[tokio::test]
    async fn path_fetch_refuses_more_than_5_mib_and_missing_files() {
        let dir = scratch();
        let big = dir.path().join("big");
        fs::write(&big, vec![b' '; MAX_SHARE_BYTES + 1]).expect("write");
        let at = |p: &Path| ShareLocation::Path {
            value: p.to_string_lossy().into_owned(),
        };
        assert_eq!(
            fetch(&at(&big), &Validator::None).await,
            Err(TransportError::TooLarge)
        );
        let exact = dir.path().join("exact");
        fs::write(&exact, vec![b' '; MAX_SHARE_BYTES]).expect("write");
        assert!(fetch(&at(&exact), &Validator::None).await.is_ok());
        let missing = dir.path().join("missing");
        assert_eq!(
            fetch(&at(&missing), &Validator::None).await,
            Err(TransportError::FileUnavailable)
        );
        // A directory is not a share file.
        assert_eq!(
            fetch(&at(dir.path()), &Validator::None).await,
            Err(TransportError::FileUnavailable)
        );
        // The message names no path.
        assert!(!TransportError::FileUnavailable
            .to_string()
            .contains(&*dir.path().to_string_lossy()));
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temporary_file() {
        let dir = scratch();
        let file = dir.path().join("share.portolano-share");
        // The old fixed temporary name is not used any more, so whatever sits there stays.
        let squatter = dir.path().join("share.portolano-share.tmp");
        fs::write(&squatter, b"mine").expect("squatter");
        write_atomic(&file, b"one").expect("first");
        write_atomic(&file, b"two").expect("second");
        assert_eq!(fs::read(&file).expect("read"), b"two");
        let names: Vec<_> = fs::read_dir(dir.path())
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names.len(), 2);
        assert_eq!(fs::read(&squatter).expect("squatter"), b"mine");
        // A missing directory is an error, not a panic.
        assert!(write_atomic(&dir.path().join("none/x"), b"x").is_err());
    }
}
