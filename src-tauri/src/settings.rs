//! Persisted application settings, kept in `settings.json` inside the data directory (so
//! `PORTOLANO_DATA_DIR` is honoured and tests never touch the user's data). One store is shared,
//! as Tauri managed state, by the update feature and by the commands that need a setting.
//!
//! The file holds no secret: whether to check for updates automatically, the version the user
//! chose to skip, the last version that ran, the release notes of an update the user accepted (so
//! that "What's new" needs no network on the first launch after it), the vault auto-lock time,
//! whether quick unlock is on, and the appearance choices (theme, colour scheme, accent, density,
//! terminal font size).

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

/// Terminal font size (in px) used when the file has none: the size the terminal always had.
pub const DEFAULT_TERMINAL_FONT_SIZE: u8 = 13;
/// Smallest and largest terminal font size the user can choose.
pub const TERMINAL_FONT_SIZE_RANGE: std::ops::RangeInclusive<u8> = 10..=24;

/// Defines a closed set of names stored in `settings.json` and sent to the UI as lowercase
/// strings, with `FromStr`, which refuses anything outside the set (and, in tests, `ALL` and `as_str`).
macro_rules! choice_enum {
    ($(#[$meta:meta])* $name:ident, $what:literal, { $($variant:ident => $text:literal),+ $(,)? }, default $default:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            #[cfg(test)]
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            #[cfg(test)]
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl std::str::FromStr for $name {
            type Err = AppError;

            fn from_str(text: &str) -> Result<Self, AppError> {
                match text {
                    $($text => Ok($name::$variant),)+
                    _ => Err(AppError::invalid_input(format!(
                        "unsupported {}: expected one of {}",
                        $what,
                        [$($text),+].join(", ")
                    ))),
                }
            }
        }
    };
}

choice_enum!(
    /// Light, dark, or follow the system.
    Theme, "theme", { System => "system", Light => "light", Dark => "dark" }, default System
);
choice_enum!(
    /// Colour scheme: the palette of surfaces, text and terminal colours (each has a light and a
    /// dark variant, so `Theme` keeps working on top of it).
    Scheme, "colour scheme",
    { Portolano => "portolano", Nord => "nord", Solarized => "solarized", RosePine => "rose-pine" },
    default Portolano
);
choice_enum!(
    /// Accent colour of the interface (selection, focus ring, primary buttons).
    Accent, "accent",
    { Blue => "blue", Teal => "teal", Purple => "purple", Orange => "orange", Graphite => "graphite" },
    default Blue
);
choice_enum!(
    /// Row height and spacing of the connection tree and the lists.
    Density, "density", { Comfortable => "comfortable", Compact => "compact" }, default Comfortable
);

/// The appearance choices as the UI sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub theme: Theme,
    pub scheme: Scheme,
    pub accent: Accent,
    pub density: Density,
    pub terminal_font_size: u8,
}

/// An appearance change as the UI sends it: plain strings and a number, checked by
/// [`AppearanceInput::validate`] so an unknown value is refused with a clear error.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceInput {
    pub theme: String,
    pub scheme: String,
    pub accent: String,
    pub density: String,
    pub terminal_font_size: u32,
}

impl AppearanceInput {
    /// Accepts only values from the fixed lists and a font size from 10 to 24.
    pub fn validate(&self) -> Result<Appearance, AppError> {
        let size = u8::try_from(self.terminal_font_size)
            .ok()
            .filter(|size| TERMINAL_FONT_SIZE_RANGE.contains(size))
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "the terminal font size must be between {} and {}",
                    TERMINAL_FONT_SIZE_RANGE.start(),
                    TERMINAL_FONT_SIZE_RANGE.end()
                ))
            })?;
        Ok(Appearance {
            theme: self.theme.parse()?,
            scheme: self.scheme.parse()?,
            accent: self.accent.parse()?,
            density: self.density.parse()?,
            terminal_font_size: size,
        })
    }
}

/// Reads a field that must never stop the file from loading: a value that is not one of the
/// known names (a file written by a newer version, or edited by hand) gives the default.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Same, for the terminal font size: outside 10 to 24 gives the default.
fn lenient_font_size<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let size: u8 = lenient(deserializer)?;
    Ok(if TERMINAL_FONT_SIZE_RANGE.contains(&size) {
        size
    } else {
        DEFAULT_TERMINAL_FONT_SIZE
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "Automatically check for updates". On by default.
    pub auto_check_updates: bool,
    /// "Include pre-releases": update checks also offer release candidates. Off by default.
    pub include_prereleases: bool,
    /// The version the user chose to skip: not proposed again by an automatic check.
    pub skipped_version: Option<String>,
    /// The highest version that has run on this machine.
    pub last_run_version: Option<String>,
    /// Minutes of inactivity before the vault locks itself; `0` = never.
    pub auto_lock_minutes: u32,
    /// "Quick unlock" with Touch ID / Windows Hello. Off by default; kept per device. It is
    /// only a switch: the key that makes quick unlock possible never leaves the process memory.
    pub quick_unlock: bool,
    /// Light, dark or follow the system.
    #[serde(deserialize_with = "lenient")]
    pub theme: Theme,
    /// Colour scheme (palette of surfaces, text and terminal).
    #[serde(deserialize_with = "lenient")]
    pub scheme: Scheme,
    /// Accent colour.
    #[serde(deserialize_with = "lenient")]
    pub accent: Accent,
    /// Row height and spacing of the tree and the lists.
    #[serde(deserialize_with = "lenient")]
    pub density: Density,
    /// Terminal font size in px, 10 to 24.
    #[serde(deserialize_with = "lenient_font_size")]
    pub terminal_font_size: u8,
}

impl Settings {
    /// The appearance choices.
    pub fn appearance(&self) -> Appearance {
        Appearance {
            theme: self.theme,
            scheme: self.scheme,
            accent: self.accent,
            density: self.density,
            terminal_font_size: self.terminal_font_size,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_check_updates: true,
            include_prereleases: false,
            skipped_version: None,
            last_run_version: None,
            auto_lock_minutes: DEFAULT_AUTO_LOCK_MINUTES,
            quick_unlock: false,
            theme: Theme::default(),
            scheme: Scheme::default(),
            accent: Accent::default(),
            density: Density::default(),
            terminal_font_size: DEFAULT_TERMINAL_FONT_SIZE,
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
        assert!(!settings.include_prereleases, "pre-releases are opt-in");
    }

    #[test]
    fn a_settings_file_without_include_prereleases_loads_as_off() {
        let dir = scratch("no-prereleases");
        std::fs::write(
            dir.join(FILE_NAME),
            r#"{"autoCheckUpdates":true,"lastRunVersion":"0.3.1","autoLockMinutes":15}"#,
        )
        .expect("write the old file");
        let store = SettingsStore::load(&dir);
        assert!(!store.get().expect("get").include_prereleases);
        store
            .update(|s| s.include_prereleases = true)
            .expect("update");
        assert!(
            SettingsStore::load(&dir)
                .get()
                .expect("get")
                .include_prereleases
        );
    }

    #[test]
    fn a_settings_file_of_0_3_1_with_saved_notes_still_loads() {
        let dir = scratch("old-notes");
        std::fs::write(
            dir.join(FILE_NAME),
            r#"{"autoCheckUpdates":false,"skippedVersion":"0.3.0","lastRunVersion":"0.3.1","pendingNotes":{"version":"0.3.1","notes":"- old"},"autoLockMinutes":30,"quickUnlock":true}"#,
        )
        .expect("write the old file");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert!(!settings.auto_check_updates);
        assert_eq!(settings.last_run_version.as_deref(), Some("0.3.1"));
        assert_eq!(settings.auto_lock_minutes, 30);
        assert!(settings.quick_unlock);
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
            })
            .expect("update");
        let again = SettingsStore::load(&dir).get().expect("get");
        assert!(!again.auto_check_updates);
        assert_eq!(again.skipped_version.as_deref(), Some("0.2.0"));
        assert_eq!(again.last_run_version.as_deref(), Some("0.1.0"));
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
    fn a_0_3_x_file_without_the_appearance_fields_gets_the_defaults() {
        let dir = scratch("old-appearance");
        std::fs::write(
            settings_path(&dir),
            br#"{"autoCheckUpdates":false,"lastRunVersion":"0.3.1","autoLockMinutes":30,"quickUnlock":true}"#,
        )
        .expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(
            settings.appearance(),
            Appearance {
                theme: Theme::System,
                scheme: Scheme::Portolano,
                accent: Accent::Blue,
                density: Density::Comfortable,
                terminal_font_size: 13,
            }
        );
        assert_eq!(settings.auto_lock_minutes, 30);
        assert!(!settings.auto_check_updates);
    }

    #[test]
    fn a_task_8_file_without_a_scheme_gets_the_default_scheme() {
        let dir = scratch("no-scheme");
        std::fs::write(
            settings_path(&dir),
            br#"{"theme":"dark","accent":"teal","density":"compact","terminalFontSize":16}"#,
        )
        .expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.scheme, Scheme::Portolano);
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.accent, Accent::Teal);
        assert_eq!(settings.terminal_font_size, 16);
    }

    #[test]
    fn the_appearance_survives_a_reload_in_camel_case() {
        let dir = scratch("appearance-persist");
        let input = AppearanceInput {
            theme: "dark".into(),
            scheme: "rose-pine".into(),
            accent: "teal".into(),
            density: "compact".into(),
            terminal_font_size: 16,
        };
        let appearance = input.validate().expect("valid");
        SettingsStore::load(&dir)
            .update(|s| {
                s.theme = appearance.theme;
                s.scheme = appearance.scheme;
                s.accent = appearance.accent;
                s.density = appearance.density;
                s.terminal_font_size = appearance.terminal_font_size;
            })
            .expect("update");
        let text = std::fs::read_to_string(settings_path(&dir)).expect("read");
        assert!(text.contains("\"theme\": \"dark\""), "{text}");
        assert!(text.contains("\"scheme\": \"rose-pine\""), "{text}");
        assert!(text.contains("\"terminalFontSize\": 16"), "{text}");
        assert_eq!(
            SettingsStore::load(&dir).get().expect("get").appearance(),
            appearance
        );
    }

    #[test]
    fn values_outside_the_lists_are_refused_on_save() {
        let ok = || AppearanceInput {
            theme: "light".into(),
            scheme: "nord".into(),
            accent: "purple".into(),
            density: "comfortable".into(),
            terminal_font_size: 10,
        };
        assert!(ok().validate().is_ok());
        for bad in [
            AppearanceInput {
                theme: "sepia".into(),
                ..ok()
            },
            AppearanceInput {
                theme: "Dark".into(),
                ..ok()
            },
            AppearanceInput {
                scheme: "dracula".into(),
                ..ok()
            },
            AppearanceInput {
                scheme: "Nord".into(),
                ..ok()
            },
            AppearanceInput {
                scheme: "".into(),
                ..ok()
            },
            AppearanceInput {
                accent: "red".into(),
                ..ok()
            },
            AppearanceInput {
                accent: "".into(),
                ..ok()
            },
            AppearanceInput {
                density: "tiny".into(),
                ..ok()
            },
            AppearanceInput {
                terminal_font_size: 9,
                ..ok()
            },
            AppearanceInput {
                terminal_font_size: 25,
                ..ok()
            },
            AppearanceInput {
                terminal_font_size: 0,
                ..ok()
            },
            AppearanceInput {
                terminal_font_size: 269,
                ..ok()
            },
            AppearanceInput {
                terminal_font_size: u32::MAX,
                ..ok()
            },
        ] {
            let e = bad.validate().expect_err("refused");
            assert_eq!(e.code, ErrorCode::InvalidInput, "{bad:?}");
        }
        for size in [10, 13, 24] {
            let valid = AppearanceInput {
                terminal_font_size: size,
                ..ok()
            };
            assert_eq!(
                valid.validate().expect("valid").terminal_font_size,
                size as u8
            );
        }
    }

    #[test]
    fn every_listed_name_parses_back_to_itself() {
        for theme in Theme::ALL {
            assert_eq!(theme.as_str().parse::<Theme>().expect("parse"), *theme);
        }
        for scheme in Scheme::ALL {
            assert_eq!(scheme.as_str().parse::<Scheme>().expect("parse"), *scheme);
        }
        for accent in Accent::ALL {
            assert_eq!(accent.as_str().parse::<Accent>().expect("parse"), *accent);
        }
        for density in Density::ALL {
            assert_eq!(
                density.as_str().parse::<Density>().expect("parse"),
                *density
            );
        }
    }

    #[test]
    fn a_bad_appearance_in_the_file_gives_defaults_without_losing_the_other_settings() {
        let dir = scratch("bad-appearance");
        std::fs::write(
            settings_path(&dir),
            br#"{"skippedVersion":"0.3.0","theme":"sepia","scheme":"dracula","accent":7,"density":null,"terminalFontSize":99}"#,
        )
        .expect("write");
        let settings = SettingsStore::load(&dir).get().expect("get");
        assert_eq!(settings.skipped_version.as_deref(), Some("0.3.0"));
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.scheme, Scheme::Portolano);
        assert_eq!(settings.accent, Accent::Blue);
        assert_eq!(settings.density, Density::Comfortable);
        assert_eq!(settings.terminal_font_size, 13);
    }

    #[test]
    fn an_older_build_reads_a_file_written_with_the_appearance_fields() {
        // Builds before this change have no `deny_unknown_fields`: the new keys are ignored. This
        // mirrors their struct (the fields they know) reading what the current build writes.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        #[allow(dead_code)]
        struct Old {
            auto_check_updates: bool,
            auto_lock_minutes: u32,
            quick_unlock: bool,
        }
        impl Default for Old {
            fn default() -> Self {
                Self {
                    auto_check_updates: true,
                    auto_lock_minutes: 15,
                    quick_unlock: false,
                }
            }
        }
        let dir = scratch("old-reader");
        SettingsStore::load(&dir)
            .update(|s| {
                s.theme = Theme::Dark;
                s.auto_lock_minutes = 60;
            })
            .expect("update");
        let bytes = std::fs::read(settings_path(&dir)).expect("read");
        let old: Old = serde_json::from_slice(&bytes).expect("an older build still reads it");
        assert_eq!(old.auto_lock_minutes, 60);
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
