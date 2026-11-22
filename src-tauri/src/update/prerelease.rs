//! Opt-in pre-release updates. GitHub's `releases/latest` never lists pre-releases, so when the
//! user asks for them the newest published release (pre-releases included) is found through the
//! GitHub releases API, and the update endpoint is the `latest.json` of that release.
//!
//! Choosing the release is pure (`newest_release`); only `fetch_newest` touches the network. The
//! endpoint is built from the parsed version, never from text of the API answer.

use std::time::Duration;

use semver::Version;
use serde::Deserialize;

use super::logic::parse_version;

const API_URL: &str = "https://api.github.com/repos/reallovedone/portolano/releases?per_page=10";
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
/// A page of ten releases with long notes is well under this; anything bigger is refused.
const MAX_API_BYTES: usize = 2 * 1024 * 1024;

/// The fields of a release in the API answer that matter here.
#[derive(Debug, Deserialize)]
struct ApiRelease {
    tag_name: Option<String>,
    #[serde(default)]
    draft: bool,
}

/// A tag is accepted only as `vX.Y.Z` or `vX.Y.Z-pre`: no other spelling, no build metadata.
fn parse_tag(tag: &str) -> Option<Version> {
    let rest = tag.strip_prefix('v')?;
    let version = parse_version(rest)?;
    (version.build.is_empty() && tag == format!("v{version}")).then_some(version)
}

/// The highest published version among the releases of an API answer (`json` is the body).
/// Drafts and tags that are not `vX.Y.Z[-pre]` are ignored. `None` when nothing qualifies or the
/// body is not the expected JSON.
pub fn newest_release(json: &[u8]) -> Option<Version> {
    let releases: Vec<ApiRelease> = serde_json::from_slice(json).ok()?;
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| r.tag_name.as_deref().and_then(parse_tag))
        .max()
}

/// The `latest.json` of the release `version`, built only from the parsed version.
pub fn manifest_url(version: &Version) -> String {
    format!("https://github.com/reallovedone/portolano/releases/download/v{version}/latest.json")
}

/// Asks GitHub for the newest published release. Any failure is a short reason for the log.
pub async fn fetch_newest() -> Result<Version, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(LOOKUP_TIMEOUT)
        .user_agent(concat!("Portolano/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("could not build the HTTP client: {}", e.without_url()))?;
    let response = client
        .get(API_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "the releases list answered HTTP {}",
            status.as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_API_BYTES as u64)
    {
        return Err("the releases list is too large".to_owned());
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("could not read the answer: {}", e.without_url()))?;
    if body.len() > MAX_API_BYTES {
        return Err("the releases list is too large".to_owned());
    }
    newest_release(&body).ok_or_else(|| "no usable release in the releases list".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn newest(json: &str) -> Option<String> {
        newest_release(json.as_bytes()).map(|v| v.to_string())
    }

    #[test]
    fn picks_the_highest_version_whatever_the_order() {
        let json = r#"[
            {"tag_name":"v0.3.1","draft":false,"prerelease":false},
            {"tag_name":"v0.4.0-rc2","draft":false,"prerelease":true},
            {"tag_name":"v0.4.0-rc1","draft":false,"prerelease":true}
        ]"#;
        assert_eq!(newest(json).as_deref(), Some("0.4.0-rc2"));
    }

    #[test]
    fn a_stable_release_beats_its_own_release_candidates() {
        let json = r#"[
            {"tag_name":"v0.4.0-rc2","draft":false},
            {"tag_name":"v0.4.0","draft":false},
            {"tag_name":"v0.4.0-rc10","draft":false}
        ]"#;
        assert_eq!(newest(json).as_deref(), Some("0.4.0"));
    }

    #[test]
    fn a_newer_stable_beats_an_older_candidate() {
        let json =
            r#"[{"tag_name":"v0.3.2","draft":false},{"tag_name":"v0.3.2-rc1","draft":false}]"#;
        assert_eq!(newest(json).as_deref(), Some("0.3.2"));
    }

    #[test]
    fn drafts_are_skipped() {
        let json = r#"[
            {"tag_name":"v0.5.0","draft":true},
            {"tag_name":"v0.4.0-rc1","draft":false}
        ]"#;
        assert_eq!(newest(json).as_deref(), Some("0.4.0-rc1"));
    }

    #[test]
    fn garbage_tags_are_ignored() {
        let json = r#"[
            {"tag_name":"nightly","draft":false},
            {"tag_name":"0.9.0","draft":false},
            {"tag_name":"v1.0","draft":false},
            {"tag_name":"v1.0.0+build5","draft":false},
            {"tag_name":"vv1.0.0","draft":false},
            {"tag_name":"v1.0.0/../x","draft":false},
            {"tag_name":" v1.0.0","draft":false},
            {"tag_name":null,"draft":false},
            {"draft":false},
            {"tag_name":"v0.4.0-rc1","draft":false}
        ]"#;
        assert_eq!(newest(json).as_deref(), Some("0.4.0-rc1"));
    }

    #[test]
    fn nothing_usable_gives_none() {
        assert_eq!(newest("[]"), None);
        assert_eq!(newest(r#"[{"tag_name":"v1.0.0","draft":true}]"#), None);
        assert_eq!(newest("not json"), None);
        assert_eq!(newest(r#"{"message":"API rate limit exceeded"}"#), None);
        assert_eq!(newest(r#"[1,2,3]"#), None);
    }

    #[test]
    fn the_manifest_url_comes_from_the_version() {
        let v = Version::parse("0.4.0-rc2").expect("version");
        assert_eq!(
            manifest_url(&v),
            "https://github.com/reallovedone/portolano/releases/download/v0.4.0-rc2/latest.json"
        );
    }
}
