//! Release notes: where they come from and how they are bounded.
//!
//! The notes are fetched here, on the Rust side, from the GitHub Releases API (the webview has
//! no network permission), and handed to the UI as plain text. They are Markdown from the
//! network: the UI never injects them as HTML.

use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use super::store::MAX_NOTES_BYTES;

const REPO: &str = "reallovedone/portolano";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest API response read. A GitHub release body is capped at 125,000 characters.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

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

/// Normalises line endings, trims, bounds the size, and turns "nothing to say" into `None`.
pub fn clean_notes(text: &str) -> Option<String> {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalised.trim();
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

    #[test]
    fn long_notes_are_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_NOTES_BYTES);
        let cleaned = clean_notes(&long).expect("notes");
        assert!(cleaned.len() <= MAX_NOTES_BYTES + 8);
        assert!(cleaned.ends_with('…'));
    }
}
