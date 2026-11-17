//! In-app updates (WP13a) and "What's new" after an update.
//!
//! The updater is driven from here, on the Rust side: the webview only calls the commands of
//! `commands/update.rs`, so it needs no network permission and no updater plugin permission.
//! `tauri-plugin-updater` downloads the package and verifies its minisign signature against the
//! public key of `tauri.conf.json` before anything is installed; nothing here bypasses it.

pub mod logic;
pub mod notes;
pub mod store;

use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use semver::Version;
use serde::Serialize;
use tauri::{AppHandle, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{poisoned, AppError, ErrorCode};
use logic::{is_skipped, parse_version, run_transition, should_offer, RunTransition};
use store::{PendingNotes, Settings, SettingsStore};

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
    /// Markdown text from the network, to be rendered as text, never as HTML. `None`: the UI
    /// offers the link to the release page instead.
    pub notes: Option<String>,
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
    pub current_version: String,
}

/// "What's new in <version>". `notes: None` means the UI shows the link to the release page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    pub notes: Option<String>,
}

struct PendingUpdate {
    update: Update,
    notes: Option<String>,
}

pub struct UpdateService {
    store: SettingsStore,
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
    /// Reads the settings file of `data_dir` and the endpoint override of the environment.
    pub fn new(data_dir: &Path, current: Version) -> Self {
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
        Self::with_parts(SettingsStore::load(data_dir), current, endpoint_override)
    }

    fn with_parts(store: SettingsStore, current: Version, endpoint_override: Option<Url>) -> Self {
        Self {
            store,
            current,
            endpoint_override,
            pending: Mutex::new(None),
            busy: tokio::sync::Mutex::new(()),
        }
    }

    pub fn settings(&self) -> Result<UpdateSettings, AppError> {
        Ok(UpdateSettings {
            auto_check: self.store.get()?.auto_check_updates,
            current_version: self.current.to_string(),
        })
    }

    pub fn set_auto_check(&self, enabled: bool) -> Result<(), AppError> {
        self.store.update(|s| s.auto_check_updates = enabled)?;
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

        // Pre-releases are offered to stable users only when the endpoint was overridden for
        // testing; `releases/latest` never lists them anyway.
        let allow_prerelease = self.endpoint_override.is_some();
        let mut builder = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .version_comparator(move |current, release| {
                should_offer(&current, &release.version, allow_prerelease)
            });
        if let Some(url) = &self.endpoint_override {
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

        // The manifest carries the changelog section only; GitHub is asked just without it.
        let notes = notes::release_notes(update.body.as_deref(), &version).await;
        let info = UpdateInfo {
            current_version: self.current.to_string(),
            version: version.to_string(),
            notes: notes.clone(),
        };
        *self.pending.lock().map_err(poisoned)? = Some(PendingUpdate { update, notes });
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
        let (update, notes) = {
            let guard = self.pending.lock().map_err(poisoned)?;
            let pending = guard.as_ref().ok_or_else(|| {
                AppError::invalid_input("there is no update to install: check again")
            })?;
            (pending.update.clone(), pending.notes.clone())
        };
        let version = update.version.clone();

        // Saved before installing (on Windows the installer ends this process): the first launch
        // of the new version shows these notes without a network request.
        if let Some(text) = notes {
            if let Err(e) = self.store.update(|s| {
                s.pending_notes = Some(PendingNotes {
                    version: version.clone(),
                    notes: text,
                });
            }) {
                tracing::warn!(error = %e, "could not save the release notes for after the update");
            }
        }

        match self.download_and_install(update, on_event).await {
            Ok(()) => {
                tracing::info!(%version, "update installed, restarting");
                // Triggers the normal exit path: sessions are closed and the vault is locked.
                app.request_restart();
                Ok(())
            }
            Err(e) => {
                if let Err(clear) = self.store.update(|s| s.pending_notes = None) {
                    tracing::warn!(error = %clear, "could not clear the saved release notes");
                }
                Err(e)
            }
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
    /// The last-run version is recorded before any network request, so a notes dialog is shown
    /// once per version even if the request hangs or the app is closed meanwhile. Nothing is
    /// shown on a first install, when the version is unchanged, or after a downgrade.
    pub async fn whats_new(&self) -> Result<Option<WhatsNew>, AppError> {
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
        let saved = settings
            .pending_notes
            .filter(|p| parse_version(&p.version).is_some_and(|v| v == self.current))
            .map(|p| p.notes);
        // Recorded first; a failure to write only means the dialog may come again next time.
        if let Err(e) = self.record_run(&current) {
            tracing::warn!(error = %e, "could not record the running version");
        }
        let notes = notes::release_notes(saved.as_deref(), &self.current).await;
        Ok(Some(WhatsNew {
            version: current,
            notes,
        }))
    }

    fn record_run(&self, version: &str) -> Result<Settings, AppError> {
        self.store.update(|s| {
            s.last_run_version = Some(version.to_owned());
            s.pending_notes = None;
        })
    }

    /// Opens the release page of `version` in the browser. The URL is built here from the parsed
    /// version, so the webview cannot make the application open an arbitrary address.
    pub fn open_release_page(&self, version: &str) -> Result<(), AppError> {
        let parsed = parse_version(version)
            .ok_or_else(|| AppError::invalid_input("not a valid version number"))?;
        let url = notes::release_page_url(&parsed);
        tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| {
            tracing::warn!(error = %e, "could not open the release page");
            AppError::new(
                ErrorCode::Io,
                "could not open the web browser; the release notes are on the GitHub releases page of the project",
            )
        })
    }
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
            SettingsStore::load(&dir),
            Version::parse(current).expect("version"),
            None,
        );
        (service, dir)
    }

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
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
        let reloaded =
            UpdateService::with_parts(SettingsStore::load(&dir), Version::new(0, 1, 0), None);
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
        assert!(run(service.whats_new()).expect("whats new").is_none());
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
        assert!(run(service.whats_new()).expect("whats new").is_none());
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
        assert!(run(service.whats_new()).expect("whats new").is_none());
        assert_eq!(
            SettingsStore::load(&dir)
                .get()
                .expect("get")
                .last_run_version
                .as_deref(),
            Some("0.3.0")
        );
    }

    #[test]
    fn an_upgrade_uses_the_saved_notes_once_without_the_network() {
        let (service, dir) = service("upgrade", "0.2.0");
        service
            .store
            .update(|s| {
                s.last_run_version = Some("0.1.0".into());
                s.pending_notes = Some(PendingNotes {
                    version: "0.2.0".into(),
                    notes: "## 0.2.0\n- automatic updates".into(),
                });
            })
            .expect("seed");
        let shown = run(service.whats_new())
            .expect("whats new")
            .expect("the dialog is due");
        assert_eq!(shown.version, "0.2.0");
        assert_eq!(
            shown.notes.as_deref(),
            Some("## 0.2.0\n- automatic updates")
        );
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.last_run_version.as_deref(), Some("0.2.0"));
        assert_eq!(settings.pending_notes, None, "the notes are used once");
        // The second start of the same version shows nothing.
        assert!(run(service.whats_new()).expect("whats new").is_none());
    }
}
