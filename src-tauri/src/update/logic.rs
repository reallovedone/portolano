//! Pure decisions of the update feature: which versions may be offered, whether a version was
//! skipped, and what the first launch after an update should do. No I/O, so every rule is
//! unit tested.

use semver::Version;

/// Parses a version as written in a manifest or a tag (`1.2.3` or `v1.2.3`).
pub fn parse_version(text: &str) -> Option<Version> {
    Version::parse(text.trim().trim_start_matches('v')).ok()
}

/// Whether `candidate` may be offered to a user running `current`.
///
/// - Only versions strictly newer than the running one (semver precedence, build metadata
///   ignored): never a downgrade, never the same version again.
/// - A pre-release (`0.2.0-rc1`) is offered to a user on a stable version only when
///   `allow_prerelease` is set (the testing endpoint override). A user already running a
///   pre-release may be offered any newer version. Semver already orders `0.2.0-rc1` before
///   `0.2.0`, so a user on `0.2.0` is never offered the candidate `0.2.0-rc1`.
pub fn should_offer(current: &Version, candidate: &Version, allow_prerelease: bool) -> bool {
    if candidate.cmp_precedence(current) != std::cmp::Ordering::Greater {
        return false;
    }
    if !candidate.pre.is_empty() && current.pre.is_empty() && !allow_prerelease {
        return false;
    }
    true
}

/// Whether the user chose to skip exactly this version.
pub fn is_skipped(skipped: Option<&str>, candidate: &Version) -> bool {
    match skipped {
        None => false,
        Some(text) => match parse_version(text) {
            Some(version) => version.cmp_precedence(candidate) == std::cmp::Ordering::Equal,
            None => false,
        },
    }
}

/// How this run relates to the last version that ran on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTransition {
    /// Nothing recorded (or unreadable): first run ever. "What's new" is not shown.
    FreshInstall,
    /// Same version as last time.
    Same,
    /// A newer version than the last run: show "What's new".
    Upgraded,
    /// An older version than the last run (a manual downgrade): nothing to show, and the
    /// record keeps the highest version so that coming back to it does not repeat its notes.
    Downgraded,
}

pub fn run_transition(last_run: Option<&str>, current: &Version) -> RunTransition {
    let Some(text) = last_run else {
        return RunTransition::FreshInstall;
    };
    let Some(last) = parse_version(text) else {
        // A damaged record is treated like a first run: record the current version, say nothing.
        return RunTransition::FreshInstall;
    };
    match current.cmp_precedence(&last) {
        std::cmp::Ordering::Greater => RunTransition::Upgraded,
        std::cmp::Ordering::Equal => RunTransition::Same,
        std::cmp::Ordering::Less => RunTransition::Downgraded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        parse_version(text).expect("valid version in the test")
    }

    #[test]
    fn parses_plain_and_tag_spellings() {
        assert_eq!(parse_version("1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_version("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_version(" v0.2.0-rc1 "), Some(v("0.2.0-rc1")));
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn only_strictly_newer_versions_are_offered() {
        assert!(should_offer(&v("0.1.0"), &v("0.2.0"), false));
        assert!(should_offer(&v("0.1.0"), &v("0.1.1"), false));
        assert!(
            !should_offer(&v("0.2.0"), &v("0.2.0"), false),
            "same version"
        );
        assert!(!should_offer(&v("0.2.0"), &v("0.1.9"), false), "downgrade");
        assert!(
            !should_offer(&v("1.0.0"), &v("0.9.9"), true),
            "downgrade, even with pre-releases"
        );
        assert!(
            !should_offer(&v("0.2.0"), &v("0.2.0+build5"), false),
            "build metadata is ignored"
        );
    }

    #[test]
    fn a_release_candidate_is_not_offered_to_its_own_final_version() {
        assert!(!should_offer(&v("0.2.0"), &v("0.2.0-rc1"), false));
        assert!(!should_offer(&v("0.2.0"), &v("0.2.0-rc1"), true));
    }

    #[test]
    fn pre_releases_are_hidden_from_stable_users_unless_allowed() {
        assert!(!should_offer(&v("0.1.0"), &v("0.2.0-rc1"), false));
        assert!(
            should_offer(&v("0.1.0"), &v("0.2.0-rc1"), true),
            "testing override"
        );
    }

    #[test]
    fn a_pre_release_user_follows_pre_releases_and_the_final_version() {
        assert!(should_offer(&v("0.2.0-rc1"), &v("0.2.0-rc2"), false));
        assert!(should_offer(&v("0.2.0-rc1"), &v("0.2.0"), false));
        assert!(!should_offer(&v("0.2.0-rc2"), &v("0.2.0-rc1"), false));
        assert!(!should_offer(&v("0.2.0-rc1"), &v("0.2.0-rc1"), false));
    }

    #[test]
    fn skip_matches_only_the_skipped_version() {
        assert!(is_skipped(Some("0.2.0"), &v("0.2.0")));
        assert!(is_skipped(Some("v0.2.0"), &v("0.2.0")));
        assert!(
            !is_skipped(Some("0.2.0"), &v("0.2.1")),
            "a newer version is proposed again"
        );
        assert!(!is_skipped(Some("0.2.0"), &v("0.2.0-rc1")));
        assert!(!is_skipped(None, &v("0.2.0")));
        assert!(!is_skipped(Some("garbage"), &v("0.2.0")));
    }

    #[test]
    fn first_launch_decisions() {
        let current = v("0.2.0");
        assert_eq!(run_transition(None, &current), RunTransition::FreshInstall);
        assert_eq!(run_transition(Some("0.2.0"), &current), RunTransition::Same);
        assert_eq!(
            run_transition(Some("0.1.0"), &current),
            RunTransition::Upgraded
        );
        assert_eq!(
            run_transition(Some("0.3.0"), &current),
            RunTransition::Downgraded
        );
        assert_eq!(
            run_transition(Some("0.2.0-rc1"), &current),
            RunTransition::Upgraded
        );
        assert_eq!(
            run_transition(Some("0.2.0"), &v("0.2.0-rc1")),
            RunTransition::Downgraded
        );
        assert_eq!(
            run_transition(Some("not a version"), &current),
            RunTransition::FreshInstall
        );
    }
}
