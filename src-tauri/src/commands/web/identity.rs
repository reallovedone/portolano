//! Validation and identifiers of web sessions. Pure functions, no Tauri types.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tauri::Url;
use uuid::Uuid;

use crate::error::AppError;

/// Prefix of every child webview label. No capability may match it (see the test in
/// `commands/web.rs`): a page loaded in a child webview must not have Tauri IPC.
pub const LABEL_PREFIX: &str = "web-";

/// Folder (under the app data dir) holding the WebView2 profiles on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub const PROFILES_DIR: &str = "web-profiles";

/// Parses a URL typed by the user or stored in a connection. Only `http` and `https` with a
/// host are accepted: `file:`, `javascript:`, `data:`, `tauri:` and the like never load.
pub fn validate_url(raw: &str) -> Result<Url, AppError> {
    let trimmed = raw.trim();
    let url =
        Url::parse(trimmed).map_err(|e| AppError::invalid_input(format!("invalid URL: {e}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AppError::invalid_input(format!(
                "unsupported URL scheme \"{other}\": only http and https are allowed"
            )))
        }
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(AppError::invalid_input("invalid URL: missing host"));
    }
    if is_app_origin(&url) {
        return Err(AppError::invalid_input(
            "invalid URL: this address belongs to the application itself",
        ));
    }
    Ok(url)
}

/// The origins the application serves its own UI from: on Windows Tauri treats
/// `http(s)://tauri.localhost` (and any `*.localhost` name, which resolves to the loopback
/// address anyway) as local, and a page loaded there would run with the capability of the main
/// window. They are never a valid web connection.
fn is_app_origin(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        host == "tauri.localhost" || host.ends_with(".localhost")
    })
}

/// Whether a page may navigate (links, redirects, `location`) to `url`. Same rule as
/// [`validate_url`] plus `about:blank`; everything else (`file:`, `tauri:`, `mailto:`...) is
/// blocked so a page cannot reach local resources or the app protocol.
pub fn navigation_allowed(url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" => url.host_str().is_some_and(|h| !h.is_empty()) && !is_app_origin(url),
        "about" => url.as_str() == "about:blank",
        _ => false,
    }
}

/// Label of a new child webview: unique per session, never reused.
pub fn new_label() -> String {
    format!("{LABEL_PREFIX}{}", Uuid::new_v4())
}

/// 16-byte `data_store_identifier` of a connection's isolated profile (macOS 14+).
/// Deterministic and reversible: the bytes of the connection id. The same connection always
/// gets the same store, across tabs and restarts, and a stored identifier can be mapped back
/// to its connection to find orphans. Only macOS names a store this way (Windows uses a profile
/// folder, other platforms have no isolated store), so the function exists only there.
#[cfg(target_os = "macos")]
pub fn store_identifier(connection_id: Uuid) -> [u8; 16] {
    *connection_id.as_bytes()
}

/// Inverse of [`store_identifier`].
#[cfg(target_os = "macos")]
pub fn connection_of_identifier(identifier: [u8; 16]) -> Uuid {
    Uuid::from_bytes(identifier)
}

/// WebView2 user data folder of a connection's isolated profile (Windows).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn profile_dir(data_dir: &Path, connection_id: Uuid) -> PathBuf {
    data_dir.join(PROFILES_DIR).join(connection_id.to_string())
}

/// Connections whose isolated store exists but is neither wanted (the connection no longer
/// exists or no longer asks for isolation) nor in use by an open session.
pub fn orphaned(
    found: impl IntoIterator<Item = Uuid>,
    wanted: &HashSet<Uuid>,
    in_use: &HashSet<Uuid>,
) -> Vec<Uuid> {
    found
        .into_iter()
        .filter(|id| !wanted.contains(id) && !in_use.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use portolano_core_model::ErrorCode;

    use super::*;

    #[test]
    fn the_applications_own_origin_is_never_a_web_connection() {
        for raw in [
            "http://tauri.localhost/",
            "https://tauri.localhost/index.html",
            "http://TAURI.LOCALHOST./",
            "http://anything.localhost:8080/",
        ] {
            let err = validate_url(raw).expect_err(raw);
            assert_eq!(err.code, ErrorCode::InvalidInput, "{raw}");
            let url = Url::parse(raw).expect("url");
            assert!(!navigation_allowed(&url), "{raw}");
        }
        // Plain localhost (a development server) stays allowed.
        assert!(validate_url("http://localhost:5173/").is_ok());
        assert!(navigation_allowed(
            &Url::parse("http://localhost:5173/").expect("url")
        ));
    }

    #[test]
    fn http_and_https_are_accepted() {
        assert!(validate_url("http://localhost:8080/x?y=1#z").is_ok());
        assert!(validate_url("  https://example.com ").is_ok());
        assert!(validate_url("https://192.168.1.10").is_ok());
    }

    #[test]
    fn other_schemes_are_invalid_input() {
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hi",
            "tauri://localhost/index.html",
            "ftp://example.com",
            "about:blank",
            "ssh://host",
        ] {
            assert_eq!(
                validate_url(bad).expect_err(bad).code,
                ErrorCode::InvalidInput,
                "{bad}"
            );
        }
    }

    #[test]
    fn malformed_urls_are_invalid_input() {
        for bad in ["", "example.com", "http://", "not a url"] {
            assert_eq!(
                validate_url(bad).expect_err(bad).code,
                ErrorCode::InvalidInput,
                "{bad}"
            );
        }
    }

    #[test]
    fn navigation_rule() {
        let ok = |s: &str| navigation_allowed(&Url::parse(s).expect("url"));
        assert!(ok("https://a.b/c"));
        assert!(ok("http://a.b"));
        assert!(ok("about:blank"));
        assert!(!ok("about:srcdoc"));
        assert!(!ok("file:///x"));
        assert!(!ok("tauri://localhost"));
        assert!(!ok("mailto:a@b.c"));
        assert!(!ok("javascript:1"));
    }

    #[test]
    fn labels_are_unique_and_prefixed() {
        let a = new_label();
        let b = new_label();
        assert_ne!(a, b);
        assert!(a.starts_with(LABEL_PREFIX));
        assert!(Uuid::parse_str(&a[LABEL_PREFIX.len()..]).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn store_identifier_is_stable_distinct_and_reversible() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(store_identifier(a), store_identifier(a));
        assert_ne!(store_identifier(a), store_identifier(b));
        assert_eq!(connection_of_identifier(store_identifier(a)), a);
    }

    #[test]
    fn profile_dir_is_named_after_the_connection() {
        let id = Uuid::from_u128(7);
        let dir = profile_dir(Path::new("/data"), id);
        assert_eq!(
            dir,
            Path::new("/data").join("web-profiles").join(id.to_string())
        );
    }

    #[test]
    fn orphans_exclude_wanted_and_in_use() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let wanted = HashSet::from([a]);
        let in_use = HashSet::from([b]);
        assert_eq!(orphaned([a, b, c], &wanted, &in_use), vec![c]);
        assert!(orphaned([], &wanted, &in_use).is_empty());
    }
}
