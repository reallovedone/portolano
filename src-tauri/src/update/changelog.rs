//! The changelog built into the application, and what it says about the versions a user skipped.
//!
//! `CHANGELOG.md` is embedded at compile time, so "What's new" needs no network and works for
//! any jump between versions. The sections are read with the same rules as
//! `.github/scripts/changelog-notes.py` (which still writes the body of the GitHub release): a
//! section starts at `## [X.Y.Z] - <date>` and ends at the next `## [` heading or at the link
//! references at the bottom. `[Unreleased]` is never a version, so it is never shown.

use semver::Version;
use serde::Serialize;

use super::logic::parse_version;
use crate::settings::MAX_NOTES_BYTES;

/// The repository changelog, as it was when this binary was built.
pub const EMBEDDED: &str = include_str!("../../../CHANGELOG.md");

/// One released version of the changelog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub version: String,
    /// `YYYY-MM-DD`; `None` while the heading still says "Unreleased" (a release candidate).
    pub date: Option<String>,
    /// Markdown text, trimmed, without the heading.
    pub notes: String,
}

/// A parsed section, with its version in a comparable form.
struct Parsed {
    version: Version,
    section: Section,
}

/// `## [name]` or `## [name] - rest`: the name and the rest, as the script's `SECTION` pattern.
fn heading(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("##")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('[')?;
    let close = rest.find(']')?;
    let name = &rest[..close];
    if name.is_empty() {
        return None;
    }
    let after = rest[close + 1..].trim_end();
    if after.is_empty() {
        return Some((name, ""));
    }
    let after = after.strip_prefix(char::is_whitespace)?.trim_start();
    let date = after.strip_prefix('-')?;
    if !date.starts_with(char::is_whitespace) {
        return None;
    }
    Some((name, date.trim()))
}

/// `[label]: url ...`, a link reference of the bottom of the file.
fn is_link_reference(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('[') else {
        return false;
    };
    let Some(close) = rest.find(']') else {
        return false;
    };
    let Some(after) = rest[close + 1..].strip_prefix(':') else {
        return false;
    };
    after.starts_with(char::is_whitespace) && !after.trim().is_empty()
}

/// Headings alone ("### Added" with nothing under it) are not notes.
fn has_content(body: &[&str]) -> bool {
    body.iter()
        .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
}

fn valid_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [y, m, d] = parts[..] else {
        return false;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// Every released-looking section of `changelog` (a heading whose name is a version and that has
/// some content), in file order, which is newest first.
fn parse(changelog: &str) -> Vec<Parsed> {
    let text = changelog.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let headings: Vec<(usize, &str, &str)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| heading(l).map(|(name, date)| (i, name, date)))
        .collect();
    let mut sections = Vec::new();
    for (n, &(at, name, date)) in headings.iter().enumerate() {
        let Some(version) = parse_version(name.trim()) else {
            continue; // "[Unreleased]" and anything that is not a version
        };
        // A version listed twice (a slip while editing): the first one wins.
        if sections
            .iter()
            .any(|p: &Parsed| p.version.cmp_precedence(&version) == std::cmp::Ordering::Equal)
        {
            tracing::warn!(%version, "CHANGELOG.md lists this version twice; the later section is ignored");
            continue;
        }
        let end = headings.get(n + 1).map_or(lines.len(), |next| next.0);
        let mut body = lines[at + 1..end].to_vec();
        // The link references sit after the last section.
        while body
            .last()
            .is_some_and(|l| l.trim().is_empty() || is_link_reference(l))
        {
            body.pop();
        }
        if !has_content(&body) {
            continue;
        }
        sections.push(Parsed {
            section: Section {
                version: version.to_string(),
                date: valid_date(date).then(|| date.to_owned()),
                notes: body.join("\n").trim().to_owned(),
            },
            version,
        });
    }
    sections
}

/// The sections to show after an update from `last_run` to `current`: every version `v` with
/// `last_run < v <= current`, newest first. A pre-release `X.Y.Z-rcN` has no section of its own
/// and counts as `X.Y.Z`, so its own final section is included even while it still says
/// "Unreleased".
pub fn skipped_sections(changelog: &str, last_run: &Version, current: &Version) -> Vec<Section> {
    let ceiling = Version::new(current.major, current.minor, current.patch);
    let mut found: Vec<Parsed> = parse(changelog)
        .into_iter()
        .filter(|p| {
            last_run.cmp_precedence(&p.version) == std::cmp::Ordering::Less
                && p.version.cmp_precedence(&ceiling) != std::cmp::Ordering::Greater
        })
        .collect();
    found.sort_by(|a, b| b.version.cmp_precedence(&a.version));
    found.into_iter().map(|p| p.section).collect()
}

/// Keeps the newest sections that fit in `MAX_NOTES_BYTES`. The newest one is always kept, even
/// alone over the cap (it is the running version: it is always shown), but then it is cut on a
/// character boundary. The flag tells that older ones were left out, so the UI links to the full
/// changelog.
pub fn bound(sections: Vec<Section>) -> (Vec<Section>, bool) {
    let mut kept: Vec<Section> = Vec::new();
    let mut total = 0usize;
    let all = sections.len();
    for section in sections {
        total += section.notes.len();
        if !kept.is_empty() && total > MAX_NOTES_BYTES {
            break;
        }
        kept.push(section);
    }
    if let Some(first) = kept.first_mut() {
        if first.notes.len() > MAX_NOTES_BYTES {
            let mut end = MAX_NOTES_BYTES;
            while !first.notes.is_char_boundary(end) {
                end -= 1;
            }
            first.notes = format!("{}\n\n…", first.notes[..end].trim_end());
        }
    }
    let truncated = kept.len() < all;
    (kept, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("valid version in the test")
    }

    fn section_of(version: &str) -> Section {
        parse(EMBEDDED)
            .into_iter()
            .map(|p| p.section)
            .find(|s| s.version == version)
            .unwrap_or_else(|| panic!("no section {version} in CHANGELOG.md"))
    }

    #[test]
    fn the_embedded_changelog_has_every_released_version_with_its_date() {
        let all: Vec<(String, Option<String>)> = parse(EMBEDDED)
            .into_iter()
            .map(|p| (p.section.version, p.section.date))
            .collect();
        for (version, date) in [
            ("0.3.1", "2026-11-18"),
            ("0.3.0", "2026-11-18"),
            ("0.2.0", "2026-11-17"),
            ("0.1.0", "2026-11-16"),
        ] {
            assert!(
                all.contains(&(version.to_owned(), Some(date.to_owned()))),
                "{version} {date} not in {all:?}"
            );
        }
        assert!(
            !all.iter().any(|(version, _)| version == "Unreleased"),
            "[Unreleased] is never a section"
        );
    }

    /// The body `changelog-notes.py 0.3.1` prints (checked by hand against the script).
    #[test]
    fn the_body_of_a_version_is_what_the_release_script_prints() {
        assert_eq!(
            section_of("0.3.1").notes,
            "### ✨ Added\n\n- RDP connections to Active Directory servers can sign in with Kerberos, needed where NTLM is disabled; Portolano uses it automatically with a full server name and a domain account, and an optional KDC can be set per connection."
        );
        let first = section_of("0.1.0").notes;
        assert!(
            !first.starts_with('#') || first.contains("Added"),
            "{first}"
        );
        assert!(
            !first.contains("]: https://"),
            "link references are cut: {first}"
        );
        assert!(
            !first.contains("## ["),
            "the next heading is not part of it"
        );
    }

    const FIXTURE: &str = "# Changelog\n\n## [Unreleased]\n\n- not shown\n\n## [0.3.0] - Unreleased\n\n### Added\n\n- C\n\n## [0.2.0] - 2026-11-20\r\n\r\n### Fixed\r\n\r\n- B. See [x](https://e.org).\r\n\r\n## [0.1.5] - 2026-11-19\n\n### Added\n\n## [0.1.0] - 2026-11-16\n\nFirst.\n\n[Unreleased]: https://e.org/x\n[0.1.0]: https://e.org/y\n";

    #[test]
    fn sections_follow_the_script_rules() {
        let parsed: Vec<Section> = parse(FIXTURE).into_iter().map(|p| p.section).collect();
        let versions: Vec<&str> = parsed.iter().map(|s| s.version.as_str()).collect();
        assert_eq!(
            versions,
            ["0.3.0", "0.2.0", "0.1.0"],
            "empty 0.1.5 is skipped"
        );
        assert_eq!(parsed[0].date, None, "Unreleased is not a date");
        assert_eq!(parsed[1].date.as_deref(), Some("2026-11-20"));
        assert_eq!(parsed[1].notes, "### Fixed\n\n- B. See [x](https://e.org).");
        assert_eq!(
            parsed[2].notes, "First.",
            "link references and blanks are cut"
        );
    }

    #[test]
    fn headings_need_the_script_shape() {
        assert_eq!(
            heading("## [1.0.0] - 2026-01-02"),
            Some(("1.0.0", "2026-01-02"))
        );
        assert_eq!(heading("##  [1.0.0]  "), Some(("1.0.0", "")));
        assert_eq!(heading("### [1.0.0] - 2026-01-02"), None);
        assert_eq!(heading("## [1.0.0] 2026"), None);
        assert_eq!(heading("## 1.0.0"), None);
        assert!(valid_date("2024-02-29") && !valid_date("2026-02-29"));
        assert!(!valid_date("Unreleased") && !valid_date("2026-1-02"));
    }

    fn shown(last: &str, current: &str) -> Vec<String> {
        skipped_sections(EMBEDDED, &v(last), &v(current))
            .into_iter()
            .map(|s| s.version)
            .collect()
    }

    // The real CHANGELOG.md may have a 0.3.2 section by now; these use a stable fixture.
    const RANGE: &str = "## [0.3.2] - 2026-12-01\n\n- c\n\n## [0.3.1] - 2026-11-18\n\n- b\n\n## [0.3.0] - 2026-11-18\n\n- a\n\n## [0.2.0] - 2026-11-17\n\n- z\n";

    fn range(last: &str, current: &str) -> Vec<String> {
        skipped_sections(RANGE, &v(last), &v(current))
            .into_iter()
            .map(|s| s.version)
            .collect()
    }

    #[test]
    fn every_skipped_version_is_shown_newest_first() {
        assert_eq!(range("0.2.0", "0.3.2"), ["0.3.2", "0.3.1", "0.3.0"]);
        assert_eq!(range("0.3.1", "0.3.2"), ["0.3.2"]);
        assert_eq!(range("0.3.2-rc1", "0.3.2"), ["0.3.2"]);
    }

    #[test]
    fn a_release_candidate_uses_the_section_of_its_final_version() {
        assert_eq!(range("0.3.1", "0.3.2-rc1"), ["0.3.2"]);
        assert_eq!(range("0.2.0", "0.3.2-rc2"), ["0.3.2", "0.3.1", "0.3.0"]);
    }

    #[test]
    fn nothing_is_shown_without_a_newer_version() {
        assert!(range("0.3.2", "0.3.2").is_empty());
        assert!(range("0.3.2", "0.3.1").is_empty(), "downgrade");
        // Only the real file: a version it does not have yet gives nothing.
        assert!(shown("0.3.1", "0.3.1").is_empty());
    }

    #[test]
    fn long_notes_keep_the_newest_and_flag_the_rest() {
        let big = |version: &str, size: usize| Section {
            version: version.into(),
            date: None,
            notes: "x".repeat(size),
        };
        let (kept, cut) = bound(vec![
            big("0.3.0", MAX_NOTES_BYTES / 2),
            big("0.2.0", MAX_NOTES_BYTES / 2),
            big("0.1.0", 10),
        ]);
        assert_eq!(kept.len(), 2);
        assert!(cut);
        let (kept, cut) = bound(vec![big("0.3.0", MAX_NOTES_BYTES * 2), big("0.2.0", 1)]);
        assert_eq!(kept.len(), 1, "the newest is always kept");
        assert!(cut);
        let (kept, cut) = bound(vec![big("0.3.0", 10)]);
        assert_eq!((kept.len(), cut), (1, false));
    }

    #[test]
    fn a_version_listed_twice_keeps_the_first_section() {
        let text = "## [0.2.0] - 2026-11-17\n\n- first\n\n## [0.2.0] - 2026-11-18\n\n- second\n\n## [0.1.0] - 2026-11-16\n\n- z\n";
        let all = parse(text);
        let versions: Vec<&str> = all.iter().map(|p| p.section.version.as_str()).collect();
        assert_eq!(versions, ["0.2.0", "0.1.0"]);
        assert_eq!(all[0].section.notes, "- first");
    }

    #[test]
    fn a_new_candidate_shows_the_grown_section_again() {
        // Intended: testers of rc2 see the [0.3.2] section again, as it may have grown.
        let text = "## [0.3.2] - Unreleased\n\n- c\n";
        let shown = skipped_sections(text, &v("0.3.2-rc1"), &v("0.3.2-rc2"));
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].version, "0.3.2");
    }

    #[test]
    fn a_single_oversized_section_is_cut_on_a_character_boundary() {
        let section = Section {
            version: "0.3.0".into(),
            date: None,
            notes: "é".repeat(MAX_NOTES_BYTES),
        };
        let (kept, cut) = bound(vec![section]);
        assert!(!cut);
        assert!(kept[0].notes.len() <= MAX_NOTES_BYTES + 8);
        assert!(kept[0].notes.ends_with('…'));
    }
}
