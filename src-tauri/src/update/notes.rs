//! Links to the release notes on GitHub. The notes themselves are read from the changelog built
//! into the application (`changelog.rs`); the webview only ever asks to open these addresses, and
//! they are built here from fixed text and a parsed version.

use semver::Version;

const REPO: &str = "reallovedone/portolano";

/// The tag of a release: `v<major>.<minor>.<patch>[-<pre-release>]`. Build metadata is not part
/// of a tag, and dropping it keeps the text safe to put in a URL.
fn tag(version: &Version) -> String {
    let mut plain = version.clone();
    plain.build = semver::BuildMetadata::EMPTY;
    format!("v{plain}")
}

/// The page of the release on GitHub (the "Release notes" link of the update dialog).
pub fn release_page_url(version: &Version) -> String {
    format!("https://github.com/{REPO}/releases/tag/{}", tag(version))
}

/// The full changelog on GitHub (the link that replaces the oldest versions when the notes are
/// too long to show).
pub fn changelog_url() -> String {
    format!("https://github.com/{REPO}/blob/main/CHANGELOG.md")
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
            release_page_url(&v("0.2.0-rc1")),
            "https://github.com/reallovedone/portolano/releases/tag/v0.2.0-rc1"
        );
        assert_eq!(
            release_page_url(&v("0.2.0+build.7")),
            "https://github.com/reallovedone/portolano/releases/tag/v0.2.0"
        );
        assert_eq!(
            changelog_url(),
            "https://github.com/reallovedone/portolano/blob/main/CHANGELOG.md"
        );
    }
}
