//! Persisted application settings, kept in `settings.json` inside the data directory (so
//! `PORTOLANO_DATA_DIR` is honoured and tests never touch the user's data). One store is shared,
//! as Tauri managed state, by the update feature and by the commands that need a setting.
//!
//! The file holds no secret: whether to check for updates automatically, the version the user
//! chose to skip, the last version that ran, the release notes of an update the user accepted (so
//! that "What's new" needs no network on the first launch after it), the vault auto-lock time
//! and whether quick unlock is on.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use portolano_core_model::private_fs::write_private_atomic;
use serde::{Deserialize, Serialize};

use crate::error::{poisoned, AppError, ErrorCode};

const FILE_NAME: &str = "settings.json";

/// Largest release-notes text kept in the file, in bytes.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Vault auto-lock times the user can choose, in minutes. `0` means never.
pub const AUTO_LOCK_CHOICES: [u32; 5] = [0, 5, 15, 30, 60];

/// Auto-lock time used when the file has none.
pub const DEFAULT_AUTO_LOCK_MINUTES: u32 = 15;

/// Accepts only the supported auto-lock times.
pub fn validate_auto_lock_minutes(minutes: u32) -> Result<u32, AppError> {
    if AUTO_LOCK_CHOICES.contains(&minutes) {
        Ok(minutes)
    } else {
        Err(AppError::invalid_input(
            "the auto-lock time must be 0 (never), 5, 15, 30 or 60 minutes",
        ))
    }
}

/// The vault inactivity timeout for `minutes`; `0` gives `Duration::ZERO` (never).
pub fn auto_lock_duration(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes) * 60)
}

/// Release notes saved when an update is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingNotes {
    pub version: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "Automatically check for updates". On by default.
    pub auto_check_updates: bool,
    /// The version the user chose to skip: not proposed again by an automatic check.
    pub skipped_version: Option<String>,
    /// The highest version that has run on this machine.
    pub last_run_version: Option<String>,
    pub pending_notes: Option<PendingNotes>,
    /// Minutes of inactivity before the vault locks itself; `0` = never.
    pub auto_lock_minutes: u32,
    /// "Quick unlock" with Touch ID / Windows Hello. Off by default; kept per device. It is
    /// only a switch: the key that makes quick unlock possible never leaves the process memory.
    pub quick_unlock: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_check_updates: true,
            skipped_version: None,
            last_run_version: None,
            pending_notes: None,
            auto_lock_minutes: DEFAULT_AUTO_LOCK_MINUTES,
            quick_unlock: false,
        }
    }
}

/// The settings in memory plus their file. Reads are cheap; every change is written atomically
/// before it becomes visible.
pub struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    /// Loads the file. A missing file gives the defaults. An unreadable or damaged file also
    /// gives the defaults (logged): losing "skipped version" must never stop the app.
    pub fn load(data_dir: &Path) -> Self {
        let path = settings_path(data_dir);
        let settings = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(settings) => settings,
                Err(e) => {
                    tracing::warn!(error = %e, "settings file is damaged, using the defaults");
                    Settings::default()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                tracing::warn!(error = %e, "settings file could not be read, using the defaults");
                Settings::default()
            }
        };
        Self {
            path,
            inner: Mutex::new(settings),
        }
    }

    /// Poisons the lock the way a panic while it was held would (tests of the fail-safe paths).
    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let _ = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = self.inner.lock();
                    panic!("simulated panic while the settings were locked");
                })
                .join()
        });
    }

    pub fn get(&self) -> Result<Settings, AppError> {
        Ok(self.inner.lock().map_err(poisoned)?.clone())
    }

    /// Applies `change` to a copy, writes it, and only then replaces the in-memory value, so
    /// memory and disk never disagree after a failed write.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, AppError> {
        let mut guard = self.inner.lock().map_err(poisoned)?;
        let mut next = guard.clone();
        change(&mut next);
        if next == *guard {
            return Ok(next);
        }
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|e| AppError::internal(format!("could not encode the settings: {e}")))?;
        write_private_atomic(&self.path, &bytes).map_err(|e| {
            AppError::new(
                ErrorCode::Io,
                format!("could not save the settings ({}): {e}", FILE_NAME),
            )
        })?;
        *guard = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under `<repo>/.tmp/settings-tests/`, never the user's data directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.tmp/settings-tests/tests")
            .join(name);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
        }
        std::fs::create_dir_all(&dir).expect("create the scratch folder");
        dir
    }

    #[test]
    fn defaults_when_there_is_no_file() {
        let store = SettingsStore::load(&scratch("defaults"));
        let settings = store.get().expect("get");
        assert!(
            settings.auto_check_updates,
            "automatic checks are on by default"
        );
        assert_eq!(settings.skipped_version, None);
        assert_eq!(settings.last_run_version, None);
        assert_eq!(settings.pending_notes, None);
    }

    #[test]
    fn changes_are_persisted_and_reloaded() {
        let dir = scratch("persist");
        let store = SettingsStore::load(&dir);
        store
            .update(|s| {
                s.auto_check_updates = false;
                s.skipped_version = Some("0.2.0".into());
                s.last_run_version = Some("0.1.0".into());
                s.pending_notes = Some(PendingNotes {
                    version: "0.2.0".into(),
                    notes: "- faster".into(),
                });
            })
            .expect("update");
        let again = SettingsStore::load(&dir).get().expect("get");
        assert!(!again.auto_check_updates);
        assert_eq!(again.skipped_version.as_deref(), Some("0.2.0"));
        assert_eq!(again.last_run_version.as_deref(), Some("0.1.0"));
        assert_eq!(again.pending_notes.expect("notes").notes, "- faster");
        assert!(dir.join(FILE_NAME).is_file());
        assert!(
            !dir.join("settings.json.tmp").exists(),
            "no temporary file is left"
        );
    }

    #[test]
    fn a_0_2_0_file_without_the_auto_lock_field_reads_as_15_minutes() {
        let dir = scratch("old-file");
        std::fs::write(
            settings_path(&dir),
            br#"{ "autoCheckUpdates": false, "skippedVersion": "0.2.0" }"#,
        )
        .expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.auto_lock_minutes, 15);
        assert!(!settings.auto_check_updates);
    }

    #[test]
    fn the_auto_lock_time_survives_a_reload() {
        let dir = scratch("auto-lock");
        SettingsStore::load(&dir)
            .update(|s| s.auto_lock_minutes = 60)
            .expect("update");
        let text = std::fs::read_to_string(settings_path(&dir)).expect("read");
        assert!(text.contains("\"autoLockMinutes\": 60"), "{text}");
        assert_eq!(
            SettingsStore::load(&dir)
                .get()
                .expect("get")
                .auto_lock_minutes,
            60
        );
    }

    #[test]
    fn quick_unlock_is_off_by_default_and_survives_a_reload() {
        let dir = scratch("quick-unlock");
        std::fs::write(settings_path(&dir), br#"{ "autoLockMinutes": 5 }"#).expect("write");
        let store = SettingsStore::load(&dir);
        assert!(!store.get().expect("get").quick_unlock);
        store.update(|s| s.quick_unlock = true).expect("update");
        let text = std::fs::read_to_string(settings_path(&dir)).expect("read");
        assert!(text.contains("\"quickUnlock\": true"), "{text}");
        assert!(SettingsStore::load(&dir).get().expect("get").quick_unlock);
    }

    #[test]
    fn only_the_five_choices_validate() {
        for ok in [0, 5, 15, 30, 60] {
            assert_eq!(validate_auto_lock_minutes(ok).expect("valid"), ok);
        }
        for bad in [1, 7, 10, 61, 120, u32::MAX] {
            let e = validate_auto_lock_minutes(bad).expect_err("invalid");
            assert_eq!(e.code, ErrorCode::InvalidInput);
        }
        assert_eq!(auto_lock_duration(0), Duration::ZERO);
        assert_eq!(auto_lock_duration(5), Duration::from_secs(300));
    }

    #[test]
    fn a_damaged_file_gives_the_defaults() {
        let dir = scratch("damaged");
        std::fs::write(settings_path(&dir), b"{ not json").expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = scratch("partial");
        std::fs::write(
            settings_path(&dir),
            br#"{ "skippedVersion": "0.3.0", "somethingNew": 1 }"#,
        )
        .expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert!(settings.auto_check_updates);
        assert_eq!(settings.skipped_version.as_deref(), Some("0.3.0"));
    }

    #[test]
    fn a_failed_write_leaves_memory_unchanged() {
        let dir = scratch("failed-write");
        let store = SettingsStore::load(&dir);
        // A directory where the file should be makes the rename fail.
        std::fs::create_dir_all(settings_path(&dir)).expect("block the file");
        let error = store
            .update(|s| s.auto_check_updates = false)
            .expect_err("write fails");
        assert_eq!(error.code, ErrorCode::Io);
        assert!(store.get().expect("get").auto_check_updates);
    }

    #[test]
    fn the_file_is_private_on_unix() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = scratch("private");
            let store = SettingsStore::load(&dir);
            store
                .update(|s| s.auto_check_updates = false)
                .expect("update");
            let mode = std::fs::metadata(settings_path(&dir))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
