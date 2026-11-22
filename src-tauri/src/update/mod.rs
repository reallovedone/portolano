//! In-app updates (WP13a) and "What's new" after an update.
//!
//! The updater is driven from here, on the Rust side: the webview only calls the commands of
//! `commands/update.rs`, so it needs no network permission and no updater plugin permission.
//! `tauri-plugin-updater` downloads the package and verifies its minisign signature against the
//! public key of `tauri.conf.json` before anything is installed; nothing here bypasses it.

pub mod changelog;
pub mod logic;
pub mod notes;
pub mod prerelease;

use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use semver::Version;
use serde::Serialize;
use tauri::{AppHandle, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{poisoned, AppError, ErrorCode};
use crate::settings::{Settings, SettingsStore};
use logic::{is_skipped, parse_version, run_transition, should_offer, RunTransition};

/// Environment variable that replaces the update endpoint (testing release candidates, which
/// `releases/latest` hides). HTTPS only; signature verification stays mandatory.
pub const ENDPOINT_ENV: &str = "PORTOLANO_UPDATER_ENDPOINT";

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The progress channel is fed at most once per this many downloaded bytes.
const PROGRESS_STEP: u64 = 64 * 1024;

/// What the UI shows about an available update.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
}

/// Result of `update_check`.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UpdateCheck {
    UpToDate,
    /// An automatic check while "Automatically check for updates" is off.
    Disabled,
    /// An automatic check found the version the user chose to skip.
    Skipped {
        version: String,
    },
    Available {
        update: UpdateInfo,
    },
}

/// Progress of `update_install`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UpdateEvent {
    Progress {
        downloaded: u64,
        total: Option<u64>,
    },
    /// The package is downloaded and its signature checked; installing.
    Installing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub include_prereleases: bool,
    pub current_version: String,
}

/// "What's new" after an update: the changelog sections of every version since the one that ran
/// last, newest first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    /// The running version.
    pub version: String,
    pub sections: Vec<changelog::Section>,
    /// Older versions were left out because the notes are long: the UI offers the link to the
    /// full changelog (`update_open_changelog`).
    pub truncated: bool,
}

/// Every released version of the changelog built into this binary, for the "Release notes"
/// window.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotesList {
    /// Newest first.
    pub sections: Vec<changelog::Section>,
    /// Older versions were left out because the text passed `MAX_ALL_NOTES_BYTES`.
    pub truncated: bool,
}

/// The release notes of every version, newest first, from the embedded changelog.
pub fn release_notes() -> ReleaseNotesList {
    release_notes_from(changelog::EMBEDDED)
}

fn release_notes_from(changelog_text: &str) -> ReleaseNotesList {
    let (sections, truncated) = changelog::bound_to(
        changelog::all_sections(changelog_text),
        changelog::MAX_ALL_NOTES_BYTES,
    );
    ReleaseNotesList {
        sections,
        truncated,
    }
}

struct PendingUpdate {
    update: Update,
}

pub struct UpdateService {
    store: Arc<SettingsStore>,
    current: Version,
    endpoint_override: Option<Url>,
    pending: Mutex<Option<PendingUpdate>>,
    /// One check or installation at a time.
    busy: tokio::sync::Mutex<()>,
}

/// The endpoint override: `Ok(None)` when unset, an error text when it must be refused.
pub fn parse_endpoint_override(value: Option<OsString>) -> Result<Option<Url>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = value
        .into_string()
        .map_err(|_| "the value is not valid text".to_owned())?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let url = Url::parse(text).map_err(|e| format!("the value is not a URL: {e}"))?;
    if url.scheme() != "https" {
        return Err("only https:// endpoints are accepted".to_owned());
    }
    if url.host_str().is_none() {
        return Err("the URL has no host".to_owned());
    }
    Ok(Some(url))
}

impl UpdateService {
    /// Uses the shared settings `store` and reads the endpoint override of the environment.
    pub fn new(store: Arc<SettingsStore>, current: Version) -> Self {
        let endpoint_override = match parse_endpoint_override(std::env::var_os(ENDPOINT_ENV)) {
            Ok(Some(url)) => {
                tracing::info!(
                    host = url.host_str().unwrap_or(""),
                    "{ENDPOINT_ENV} is set: using that update endpoint"
                );
                Some(url)
            }
            Ok(None) => None,
            Err(reason) => {
                tracing::warn!(%reason, "{ENDPOINT_ENV} ignored, using the default endpoint");
                None
            }
        };
        Self::with_parts(store, current, endpoint_override)
    }

    fn with_parts(
        store: Arc<SettingsStore>,
        current: Version,
        endpoint_override: Option<Url>,
    ) -> Self {
        Self {
            store,
            current,
            endpoint_override,
            pending: Mutex::new(None),
            busy: tokio::sync::Mutex::new(()),
        }
    }

    pub fn settings(&self) -> Result<UpdateSettings, AppError> {
        let settings = self.store.get()?;
        Ok(UpdateSettings {
            auto_check: settings.auto_check_updates,
            include_prereleases: settings.include_prereleases,
            current_version: self.current.to_string(),
        })
    }

    pub fn set_auto_check(&self, enabled: bool) -> Result<(), AppError> {
        self.store.update(|s| s.auto_check_updates = enabled)?;
        Ok(())
    }

    pub fn set_include_prereleases(&self, enabled: bool) -> Result<(), AppError> {
        self.store.update(|s| s.include_prereleases = enabled)?;
        Ok(())
    }

    /// Remembers that the user skipped `version`.
    pub fn skip(&self, version: &str) -> Result<(), AppError> {
        let parsed = parse_version(version)
            .ok_or_else(|| AppError::invalid_input("not a valid version number"))?;
        self.store
            .update(|s| s.skipped_version = Some(parsed.to_string()))?;
        Ok(())
    }

    /// Checks for a newer version. `manual` checks ignore both the automatic-check setting and
    /// the skipped version: they always answer.
    pub async fn check(&self, app: &AppHandle, manual: bool) -> Result<UpdateCheck, AppError> {
        let settings = self.store.get()?;
        if !manual && !settings.auto_check_updates {
            return Ok(UpdateCheck::Disabled);
        }
        let _busy = self.busy.try_lock().map_err(|_| busy_error())?;

        // The endpoint override (testing) has priority. Otherwise, with "Include pre-releases",
        // the newest published release is looked up and its own manifest is used; if the lookup
        // fails the default endpoint answers, which never lists pre-releases.
        let mut endpoint = self.endpoint_override.clone();
        if endpoint.is_none() && settings.include_prereleases {
            endpoint = prerelease_endpoint().await;
        }
        // Pre-releases are offered to stable users only when asked for (setting or override).
        let allow_prerelease = endpoint.is_some();
        let mut builder = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .version_comparator(move |current, release| {
                should_offer(&current, &release.version, allow_prerelease)
            });
        if let Some(url) = &endpoint {
            builder = builder
                .endpoints(vec![url.clone()])
                .map_err(|e| map_updater_error(e, "check"))?;
        }
        let updater = builder.build().map_err(|e| map_updater_error(e, "check"))?;
        let found = match updater.check().await {
            Ok(found) => found,
            Err(e) => return check_failure_outcome(e),
        };
        let Some(update) = found else {
            return Ok(UpdateCheck::UpToDate);
        };

        let version = parse_version(&update.version).ok_or_else(|| {
            tracing::warn!("the update manifest announced a version that is not valid semver");
            AppError::internal("the update server announced an invalid version number")
        })?;
        // The comparator already did this; checked again so no code path can offer a downgrade.
        if !should_offer(&self.current, &version, allow_prerelease) {
            return Ok(UpdateCheck::UpToDate);
        }
        if !manual && is_skipped(settings.skipped_version.as_deref(), &version) {
            return Ok(UpdateCheck::Skipped {
                version: version.to_string(),
            });
        }

        let info = UpdateInfo {
            current_version: self.current.to_string(),
            version: version.to_string(),
        };
        *self.pending.lock().map_err(poisoned)? = Some(PendingUpdate { update });
        tracing::info!(current = %self.current, available = %version, manual, "an update is available");
        Ok(UpdateCheck::Available { update: info })
    }

    /// Downloads (the plugin verifies the signature), installs, and restarts the application.
    /// On Windows the installer ends this process itself.
    pub async fn install(
        &self,
        app: &AppHandle,
        on_event: impl Fn(UpdateEvent) + Send + 'static,
    ) -> Result<(), AppError> {
        let _busy = self.busy.try_lock().map_err(|_| busy_error())?;
        let update = {
            let guard = self.pending.lock().map_err(poisoned)?;
            let pending = guard.as_ref().ok_or_else(|| {
                AppError::invalid_input("there is no update to install: check again")
            })?;
            pending.update.clone()
        };
        let version = update.version.clone();

        match self.download_and_install(update, on_event).await {
            Ok(()) => {
                tracing::info!(%version, "update installed, restarting");
                // Triggers the normal exit path: sessions are closed and the vault is locked.
                app.request_restart();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn download_and_install(
        &self,
        update: Update,
        on_event: impl Fn(UpdateEvent) + Send + 'static,
    ) -> Result<(), AppError> {
        let mut downloaded: u64 = 0;
        let mut reported: u64 = 0;
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    let finished = total.is_some_and(|t| downloaded >= t);
                    if finished || downloaded.saturating_sub(reported) >= PROGRESS_STEP {
                        reported = downloaded;
                        on_event(UpdateEvent::Progress { downloaded, total });
                    }
                },
                || {},
            )
            .await
            .map_err(|e| map_updater_error(e, "download"))?;
        on_event(UpdateEvent::Installing);
        // `install` verifies the signature once more before touching the installed application.
        let installing = update.clone();
        tokio::task::spawn_blocking(move || installing.install(&bytes))
            .await
            .map_err(|e| AppError::internal(format!("the installation task failed: {e}")))?
            .map_err(|e| map_updater_error(e, "install"))
    }

    /// Decides at start-up whether to show "What's new" and gathers its text.
    ///
    /// The notes are the sections of the changelog built into this binary for every version
    /// after the last one that ran, up to this one. The last-run version is recorded first, so
    /// the dialog is shown once per version even if the app is closed meanwhile. Nothing is shown
    /// on a first install, when the version is unchanged, or after a downgrade.
    pub async fn whats_new(&self) -> Result<Option<WhatsNew>, AppError> {
        self.whats_new_from(changelog::EMBEDDED)
    }

    fn whats_new_from(&self, changelog_text: &str) -> Result<Option<WhatsNew>, AppError> {
        let settings = self.store.get()?;
        let transition = run_transition(settings.last_run_version.as_deref(), &self.current);
        let current = self.current.to_string();
        match transition {
            RunTransition::Same | RunTransition::Downgraded => return Ok(None),
            RunTransition::FreshInstall => {
                self.record_run(&current)?;
                return Ok(None);
            }
            RunTransition::Upgraded => {}
        }
        // `Upgraded` means the record is a valid version.
        let last = settings.last_run_version.as_deref().and_then(parse_version);
        // Recorded first; a failure to write only means the dialog may come again next time.
        if let Err(e) = self.record_run(&current) {
            tracing::warn!(error = %e, "could not record the running version");
        }
        let Some(last) = last else {
            return Ok(None);
        };
        let (sections, truncated) = changelog::bound(changelog::skipped_sections(
            changelog_text,
            &last,
            &self.current,
        ));
        if sections.is_empty() {
            return Ok(None);
        }
        Ok(Some(WhatsNew {
            version: current,
            sections,
            truncated,
        }))
    }

    fn record_run(&self, version: &str) -> Result<Settings, AppError> {
        self.store.update(|s| {
            s.last_run_version = Some(version.to_owned());
        })
    }

    /// Opens the full changelog on GitHub in the browser (a fixed address).
    pub fn open_changelog(&self) -> Result<(), AppError> {
        open_in_browser(notes::changelog_url())
    }

    /// Opens the release page of `version` in the browser. The URL is built here from the parsed
    /// version, so the webview cannot make the application open an arbitrary address.
    pub fn open_release_page(&self, version: &str) -> Result<(), AppError> {
        let parsed = parse_version(version)
            .ok_or_else(|| AppError::invalid_input("not a valid version number"))?;
        open_in_browser(notes::release_page_url(&parsed))
    }
}

/// The manifest of the newest published release, pre-releases included; `None` (after a warning)
/// when the lookup fails, so the default endpoint is used.
async fn prerelease_endpoint() -> Option<Url> {
    match prerelease::fetch_newest().await {
        Ok(version) => match Url::parse(&prerelease::manifest_url(&version)) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!(error = %e, "pre-release manifest address rejected, using the default endpoint");
                None
            }
        },
        Err(reason) => {
            tracing::warn!(%reason, "could not look up pre-releases, using the default endpoint");
            None
        }
    }
}

fn open_in_browser(url: String) -> Result<(), AppError> {
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| {
        tracing::warn!(error = %e, "could not open the release notes page");
        AppError::new(
            ErrorCode::Io,
            "could not open the web browser; the release notes are on the GitHub releases page of the project",
        )
    })
}

fn busy_error() -> AppError {
    AppError::invalid_input("another update operation is already in progress")
}

/// What a failed *check* means. `ReleaseNotFound` is the plugin's answer when the manifest
/// of the latest release cannot be downloaded (HTTP 404 and any other non-success status): no
/// update manifest is published, which is not something the user can act on, so it counts as
/// "no update available". Every other error stays an error. Only the check uses this: a missing
/// release while installing is still an error (`map_updater_error`).
fn check_failure_outcome(error: tauri_plugin_updater::Error) -> Result<UpdateCheck, AppError> {
    if matches!(error, tauri_plugin_updater::Error::ReleaseNotFound) {
        tracing::info!(
            "no update manifest is published for the latest release: treating it as up to date"
        );
        return Ok(UpdateCheck::UpToDate);
    }
    Err(map_updater_error(error, "check"))
}

/// A message the user can act on. The detail is the plugin's own text (no secrets in it).
fn map_updater_error(error: tauri_plugin_updater::Error, stage: &str) -> AppError {
    use tauri_plugin_updater::Error as E;
    tracing::warn!(stage, %error, "update operation failed");
    match error {
        E::Reqwest(e) => AppError::new(
            ErrorCode::HostUnreachable,
            format!(
                "could not reach the update server; check the internet connection ({})",
                e.without_url()
            ),
        ),
        E::Network(detail) => AppError::new(
            ErrorCode::HostUnreachable,
            format!("the update server did not deliver the update ({detail})"),
        ),
        E::ReleaseNotFound => AppError::new(
            ErrorCode::HostUnreachable,
            "the update server did not return valid release information",
        ),
        e @ (E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion) => AppError::internal(format!(
            "the update could not be verified and was NOT installed ({e})"
        )),
        e @ (E::TargetNotFound(_)
        | E::TargetsNotFound(_)
        | E::UnsupportedOs
        | E::UnsupportedArch) => AppError::new(
            ErrorCode::Unsupported,
            format!("this release has no update package for this system ({e})"),
        ),
        E::Io(e) => AppError::new(
            ErrorCode::Io,
            format!("the update could not be written to disk ({e})"),
        ),
        e => AppError::internal(format!("the update failed ({e})")),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn release_notes_list_every_version_and_serialise_for_the_ui() {
        let text = "## [Unreleased]\n\n- wip\n\n## [0.2.0] - 2026-11-17\n\n- b\n\n## [0.1.0] - 2026-11-16\n\n- a\n";
        let list = release_notes_from(text);
        assert!(!list.truncated);
        let versions: Vec<&str> = list.sections.iter().map(|s| s.version.as_str()).collect();
        assert_eq!(versions, ["0.2.0", "0.1.0"]);
        let json = serde_json::to_value(&list).expect("serialises");
        assert_eq!(json["truncated"], false);
        assert_eq!(json["sections"][0]["version"], "0.2.0");
        assert_eq!(json["sections"][0]["date"], "2026-11-17");
        // The real file, as the command returns it.
        let real = release_notes();
        assert!(real.sections.iter().any(|s| s.version == "0.1.0"));
    }

    #[test]
    fn release_notes_over_the_cap_drop_the_oldest() {
        let part = "x".repeat(changelog::MAX_ALL_NOTES_BYTES / 2 + 1);
        let text = format!(
            "## [0.3.0] - 2026-11-19\n\n{part}\n\n## [0.2.0] - 2026-11-18\n\n{part}\n\n## [0.1.0] - 2026-11-17\n\nold\n"
        );
        let list = release_notes_from(&text);
        assert!(list.truncated);
        assert_eq!(list.sections.len(), 1);
        assert_eq!(list.sections[0].version, "0.3.0");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.tmp/wp13a/service")
            .join(name);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        }
        std::fs::create_dir_all(&dir).expect("create the scratch folder");
        dir
    }

    fn service(name: &str, current: &str) -> (UpdateService, PathBuf) {
        let dir = scratch(name);
        let service = UpdateService::with_parts(
            Arc::new(SettingsStore::load(&dir)),
            Version::parse(current).expect("version"),
            None,
        );
        (service, dir)
    }

    #[test]
    fn a_missing_release_manifest_is_not_an_update_error_when_checking() {
        use tauri_plugin_updater::Error as E;
        let outcome = check_failure_outcome(E::ReleaseNotFound).expect("not an error");
        assert!(matches!(outcome, UpdateCheck::UpToDate));
    }

    #[test]
    fn other_check_failures_stay_errors() {
        use tauri_plugin_updater::Error as E;
        let network = check_failure_outcome(E::Network("boom".into())).expect_err("error");
        assert_eq!(network.code, ErrorCode::HostUnreachable);
        let unsupported = check_failure_outcome(E::UnsupportedOs).expect_err("error");
        assert_eq!(unsupported.code, ErrorCode::Unsupported);
        let unverified = check_failure_outcome(E::MissingSignedVersion).expect_err("error");
        assert_eq!(unverified.code, ErrorCode::Internal);
    }

    #[test]
    fn a_missing_release_is_still_an_error_outside_the_check() {
        let error = map_updater_error(tauri_plugin_updater::Error::ReleaseNotFound, "download");
        assert_eq!(error.code, ErrorCode::HostUnreachable);
    }

    #[test]
    fn the_endpoint_override_must_be_https() {
        assert_eq!(parse_endpoint_override(None), Ok(None));
        assert_eq!(parse_endpoint_override(Some(OsString::new())), Ok(None));
        assert_eq!(parse_endpoint_override(Some("  ".into())), Ok(None));
        let ok = parse_endpoint_override(Some("https://example.org/rc/latest.json".into()))
            .expect("https accepted")
            .expect("a url");
        assert_eq!(ok.host_str(), Some("example.org"));
        for bad in [
            "http://example.org/latest.json",
            "ftp://example.org/x",
            "file:///tmp/latest.json",
            "example.org/latest.json",
            "https://",
            "not a url",
        ] {
            assert!(
                parse_endpoint_override(Some(bad.into())).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn auto_check_defaults_on_and_persists() {
        let (service, dir) = service("auto", "0.1.0");
        assert!(service.settings().expect("settings").auto_check);
        service.set_auto_check(false).expect("set");
        assert!(!service.settings().expect("settings").auto_check);
        let reloaded = UpdateService::with_parts(
            Arc::new(SettingsStore::load(&dir)),
            Version::new(0, 1, 0),
            None,
        );
        assert!(!reloaded.settings().expect("settings").auto_check);
    }

    #[test]
    fn skip_is_persisted_in_canonical_form_and_rejects_garbage() {
        let (service, dir) = service("skip", "0.1.0");
        service.skip("v0.2.0").expect("skip");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.skipped_version.as_deref(), Some("0.2.0"));
        assert_eq!(
            service.skip("../../etc").expect_err("invalid").code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn a_fresh_install_records_the_version_and_shows_nothing() {
        let (service, dir) = service("fresh", "0.1.0");
        assert!(service
            .whats_new_from(CHANGELOG)
            .expect("whats new")
            .is_none());
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.last_run_version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn the_same_version_shows_nothing() {
        let (service, dir) = service("same", "0.1.0");
        service
            .store
            .update(|s| s.last_run_version = Some("0.1.0".into()))
            .expect("seed");
        assert!(service
            .whats_new_from(CHANGELOG)
            .expect("whats new")
            .is_none());
        assert_eq!(
            SettingsStore::load(&dir)
                .get()
                .expect("get")
                .last_run_version
                .as_deref(),
            Some("0.1.0")
        );
    }

    #[test]
    fn a_downgrade_shows_nothing_and_keeps_the_highest_version() {
        let (service, dir) = service("downgrade", "0.1.0");
        service
            .store
            .update(|s| s.last_run_version = Some("0.3.0".into()))
            .expect("seed");
        assert!(service
            .whats_new_from(CHANGELOG)
            .expect("whats new")
            .is_none());
        assert_eq!(
            SettingsStore::load(&dir)
                .get()
                .expect("get")
                .last_run_version
                .as_deref(),
            Some("0.3.0")
        );
    }

    const CHANGELOG: &str = "## [Unreleased]\n\n## [0.3.2] - 2026-12-01\n\n- c\n\n## [0.3.1] - 2026-11-18\n\n- b\n\n## [0.3.0] - 2026-11-18\n\n- a\n\n## [0.2.0] - 2026-11-17\n\n- z\n";

    fn versions_shown(last: &str, current: &str) -> (Vec<String>, SettingsStore) {
        let (service, dir) = service(&format!("shown-{last}-{current}"), current);
        service
            .store
            .update(|s| s.last_run_version = Some(last.into()))
            .expect("seed");
        let shown = service
            .whats_new_from(CHANGELOG)
            .expect("whats new")
            .map(|w| w.sections.into_iter().map(|s| s.version).collect())
            .unwrap_or_default();
        (shown, SettingsStore::load(&dir))
    }

    #[test]
    fn an_upgrade_shows_every_skipped_version_once() {
        let (shown, settings) = versions_shown("0.2.0", "0.3.2");
        assert_eq!(shown, ["0.3.2", "0.3.1", "0.3.0"]);
        let settings = settings.get().expect("get");
        assert_eq!(settings.last_run_version.as_deref(), Some("0.3.2"));
        let (service, _) = service("again", "0.3.2");
        assert!(service
            .whats_new_from(CHANGELOG)
            .expect("whats new")
            .is_none());
    }

    #[test]
    fn one_step_and_release_candidate_upgrades() {
        assert_eq!(versions_shown("0.3.1", "0.3.2").0, ["0.3.2"]);
        assert_eq!(versions_shown("0.3.2-rc1", "0.3.2").0, ["0.3.2"]);
        assert_eq!(versions_shown("0.3.1", "0.3.2-rc1").0, ["0.3.2"]);
    }

    #[test]
    fn nothing_to_show_still_records_the_version() {
        let (shown, settings) = versions_shown("0.3.2", "0.3.2");
        assert!(shown.is_empty());
        assert_eq!(
            settings.get().expect("get").last_run_version.as_deref(),
            Some("0.3.2")
        );
        // An upgrade whose versions have no section: nothing to show, but recorded.
        let (shown, settings) = versions_shown("0.3.2", "0.9.0");
        assert!(shown.is_empty());
        assert_eq!(
            settings.get().expect("get").last_run_version.as_deref(),
            Some("0.9.0")
        );
    }

    #[test]
    fn too_long_notes_say_so() {
        let (service, _) = service("long", "0.3.2");
        service
            .store
            .update(|s| s.last_run_version = Some("0.1.0".into()))
            .expect("seed");
        let big = "x".repeat(crate::settings::MAX_NOTES_BYTES / 2 + 1);
        let text = format!(
            "## [0.3.2] - 2026-12-01\n\n{big}\n\n## [0.3.1] - 2026-11-18\n\n{big}\n\n## [0.2.0] - 2026-11-17\n\n- z\n"
        );
        let shown = service
            .whats_new_from(&text)
            .expect("whats new")
            .expect("due");
        assert_eq!(shown.sections.len(), 1);
        assert!(shown.truncated);
    }
}
