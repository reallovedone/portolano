//! Release notes: where they come from and how they are bounded.
//!
//! The notes come from the update manifest (`latest.json`, its `notes` field), which holds the
//! changelog section only. When the manifest has none, they are fetched here, on the Rust side,
//! from the GitHub Releases API (the webview has no network permission). Either way they reach
//! the UI as plain text. They are Markdown from the network: the UI never injects them as HTML.
//!
//! The body of the GitHub release also ends with a paragraph for first-time installers, which
//! the release workflow puts after [`INSTALL_NOTE_MARKER`]: the app cuts it off.

use std::future::Future;
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use crate::settings::MAX_NOTES_BYTES;

const REPO: &str = "reallovedone/portolano";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest API response read. A GitHub release body is capped at 125,000 characters.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// The line the release workflow writes (an HTML comment, invisible on GitHub) right before the
/// install paragraph of the release body. Everything from the marker on is not release notes.
pub const INSTALL_NOTE_MARKER: &str = "<!-- portolano:install-note -->";

/// The tag of a release: `v<major>.<minor>.<patch>[-<pre-release>]`. Build metadata is not part
/// of a tag, and dropping it keeps the text safe to put in a URL.
fn tag(version: &Version) -> String {
    let mut plain = version.clone();
    plain.build = semver::BuildMetadata::EMPTY;
    format!("v{plain}")
}

/// The page of the release on GitHub (the "See the release notes" link).
pub fn release_page_url(version: &Version) -> String {
    format!("https://github.com/{REPO}/releases/tag/{}", tag(version))
}

pub fn release_api_url(version: &Version) -> String {
    format!(
        "https://api.github.com/repos/{REPO}/releases/tags/{}",
        tag(version)
    )
}

#[derive(Deserialize)]
struct ReleaseBody {
    body: Option<String>,
}

/// The `body` of a GitHub release JSON, cleaned.
pub fn extract_body(json: &[u8]) -> Option<String> {
    let release: ReleaseBody = serde_json::from_slice(json).ok()?;
    clean_notes(release.body.as_deref()?)
}

/// Normalises line endings, cuts the install paragraph (everything from [`INSTALL_NOTE_MARKER`]
/// on; text without the marker is used as it is), trims, bounds the size, and turns "nothing to
/// say" into `None`.
pub fn clean_notes(text: &str) -> Option<String> {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let kept = match normalised.find(INSTALL_NOTE_MARKER) {
        Some(at) => &normalised[..at],
        None => normalised.as_str(),
    };
    let trimmed = kept.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= MAX_NOTES_BYTES {
        return Some(trimmed.to_owned());
    }
    let mut end = MAX_NOTES_BYTES;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}\n\n…", trimmed[..end].trim_end()))
}

/// The notes to show for `version`: the manifest's when it has any, so the normal case makes no
/// request; otherwise the GitHub release body (see [`fetch_release_notes`]).
pub async fn release_notes(manifest: Option<&str>, version: &Version) -> Option<String> {
    notes_or_else(manifest, || fetch_release_notes(version)).await
}

async fn notes_or_else<F, Fut>(manifest: Option<&str>, fallback: F) -> Option<String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<String>>,
{
    match manifest.and_then(clean_notes) {
        Some(text) => Some(text),
        None => fallback().await,
    }
}

/// Notes of the release `version` from GitHub, or `None` for any failure (logged): the caller
/// falls back to the update manifest and then to a link.
pub async fn fetch_release_notes(version: &Version) -> Option<String> {
    match fetch(version).await {
        Ok(notes) => notes,
        Err(reason) => {
            tracing::warn!(%version, %reason, "could not fetch the release notes");
            None
        }
    }
}

async fn fetch(version: &Version) -> Result<Option<String>, String> {
    // No cookies, no credentials, nothing about the user: a fixed User-Agent (GitHub rejects
    // requests without one) and the public release URL.
    let client = reqwest::Client::builder()
        .user_agent("Portolano-updater")
        .https_only(true)
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let mut response = client
        .get(release_api_url(version))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("status {}", response.status()));
    }
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("body: {}", e.without_url()))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("the response is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(extract_body(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("valid version in the test")
    }

    #[test]
    fn urls_are_built_from_the_parsed_version_only() {
        assert_eq!(
            release_api_url(&v("0.2.0")),
            "https://api.github.com/repos/reallovedone/portolano/releases/tags/v0.2.0"
        );
        assert_eq!(
            release_page_url(&v("0.2.0-rc1")),
            "https://github.com/reallovedone/portolano/releases/tag/v0.2.0-rc1"
        );
        assert_eq!(
            release_page_url(&v("0.2.0+build.7")),
            "https://github.com/reallovedone/portolano/releases/tag/v0.2.0"
        );
    }

    #[test]
    fn the_body_field_is_extracted_and_cleaned() {
        let json = br###"{"tag_name":"v0.2.0","body":"## New\r\n- one\r\n- two\r\n\r\n"}"###;
        assert_eq!(extract_body(json).as_deref(), Some("## New\n- one\n- two"));
    }

    #[test]
    fn missing_null_or_blank_bodies_give_nothing() {
        assert_eq!(extract_body(br#"{"tag_name":"v0.2.0"}"#), None);
        assert_eq!(extract_body(br#"{"body":null}"#), None);
        assert_eq!(extract_body(br#"{"body":"  \n "}"#), None);
        assert_eq!(extract_body(b"not json"), None);
        assert_eq!(extract_body(br#"{"body":42}"#), None);
    }

    fn run<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    #[test]
    fn the_manifest_notes_win_and_the_api_is_not_asked() {
        let asked = std::cell::Cell::new(false);
        let notes = run(notes_or_else(Some("## 0.2.0\r\n- one\n"), || {
            asked.set(true);
            async { Some("from the API".to_owned()) }
        }));
        assert_eq!(notes.as_deref(), Some("## 0.2.0\n- one"));
        assert!(!asked.get(), "no request when the manifest has notes");
    }

    #[test]
    fn missing_or_blank_manifest_notes_fall_back_to_the_api() {
        for manifest in [None, Some(""), Some("  \n "), Some(INSTALL_NOTE_MARKER)] {
            let notes = run(notes_or_else(manifest, || async {
                Some("from the API".to_owned())
            }));
            assert_eq!(notes.as_deref(), Some("from the API"), "{manifest:?}");
        }
        assert_eq!(run(notes_or_else(None, || async { None })), None);
    }

    #[test]
    fn the_install_paragraph_is_cut_at_the_marker() {
        let body = format!(
            "## 0.2.0\n- one\n\n{INSTALL_NOTE_MARKER}\n---\n\nFirst install? These builds are not signed."
        );
        let json = serde_json::json!({ "body": body }).to_string();
        assert_eq!(
            extract_body(json.as_bytes()).as_deref(),
            Some("## 0.2.0\n- one")
        );
        // CRLF bodies (as GitHub may store them) are cut too, and so are saved notes.
        let crlf = body.replace('\n', "\r\n");
        assert_eq!(clean_notes(&crlf).as_deref(), Some("## 0.2.0\n- one"));
        // Only the marker line starts the cut: the first marker wins.
        let twice = format!("a\n{INSTALL_NOTE_MARKER}\nb\n{INSTALL_NOTE_MARKER}\nc");
        assert_eq!(clean_notes(&twice).as_deref(), Some("a"));
    }

    #[test]
    fn the_release_workflow_writes_the_marker_the_app_cuts_at() {
        let workflow = include_str!("../../../.github/workflows/release.yml");
        assert!(workflow.contains(INSTALL_NOTE_MARKER));
    }

    #[test]
    fn a_body_without_the_marker_is_used_as_it_is() {
        let body = "## 0.2.0\n- one\n\n---\n\nFirst install? These builds are not signed.";
        assert_eq!(clean_notes(body).as_deref(), Some(body));
    }

    #[test]
    fn long_notes_are_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_NOTES_BYTES);
        let cleaned = clean_notes(&long).expect("notes");
        assert!(cleaned.len() <= MAX_NOTES_BYTES + 8);
        assert!(cleaned.ends_with('…'));
    }
}
