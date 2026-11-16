//! Persisted settings and state of the update feature, kept in `settings.json` inside the data
//! directory (so `PORTOLANO_DATA_DIR` is honoured and tests never touch the user's data).
//!
//! The file holds no secret: whether to check automatically, the version the user chose to skip,
//! the last version that ran, and the release notes of an update the user accepted (so that
//! "What's new" needs no network on the first launch after it).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use portolano_core_model::private_fs::write_private_atomic;
use serde::{Deserialize, Serialize};

use crate::error::{poisoned, AppError, ErrorCode};

const FILE_NAME: &str = "settings.json";

/// Largest release-notes text kept in the file, in bytes.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_check_updates: true,
            skipped_version: None,
            last_run_version: None,
            pending_notes: None,
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

    /// A fresh folder under `<repo>/.tmp/wp13a/`, never the user's data directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.tmp/wp13a/tests")
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
