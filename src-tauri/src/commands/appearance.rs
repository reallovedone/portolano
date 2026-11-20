//! IPC commands of the appearance settings (theme, colour scheme, accent, density, terminal font size). Not part
//! of the §4.3 table: they are additive. The values live in `settings.json`; the UI keeps a copy in
//! `localStorage` so the theme is applied before the first paint.

use std::sync::Arc;

use tauri::State;

use crate::error::AppError;
use crate::settings::{Appearance, AppearanceInput, SettingsStore};

fn get(settings: &SettingsStore) -> Result<Appearance, AppError> {
    Ok(settings.get()?.appearance())
}

/// Validates and saves a new appearance; nothing changes when a value is refused or the file
/// cannot be written. Returns what is now saved.
fn set(settings: &SettingsStore, input: &AppearanceInput) -> Result<Appearance, AppError> {
    let appearance = input.validate()?;
    let saved = settings.update(|s| {
        s.theme = appearance.theme;
        s.scheme = appearance.scheme;
        s.accent = appearance.accent;
        s.density = appearance.density;
        s.terminal_font_size = appearance.terminal_font_size;
    })?;
    Ok(saved.appearance())
}

#[tauri::command]
pub async fn appearance_get(
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<Appearance, AppError> {
    get(&settings)
}

#[tauri::command]
pub async fn appearance_set(
    settings: State<'_, Arc<SettingsStore>>,
    appearance: AppearanceInput,
) -> Result<Appearance, AppError> {
    let settings = Arc::clone(&settings);
    tauri::async_runtime::spawn_blocking(move || set(&settings, &appearance))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "appearance task failed");
            AppError::internal("appearance task failed")
        })?
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::error::ErrorCode;
    use crate::settings::{Accent, Density, Scheme, Theme};

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.tmp/settings-tests/appearance")
            .join(name);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        }
        std::fs::create_dir_all(&dir).expect("create the scratch folder");
        dir
    }

    fn input(theme: &str, size: u32) -> AppearanceInput {
        AppearanceInput {
            theme: theme.into(),
            scheme: "solarized".into(),
            accent: "orange".into(),
            density: "compact".into(),
            terminal_font_size: size,
        }
    }

    #[test]
    fn a_valid_change_is_saved_and_returned() {
        let dir = scratch("valid");
        let store = SettingsStore::load(&dir);
        let saved = set(&store, &input("dark", 18)).expect("set");
        assert_eq!(saved.theme, Theme::Dark);
        assert_eq!(saved.scheme, Scheme::Solarized);
        assert_eq!(saved.accent, Accent::Orange);
        assert_eq!(saved.density, Density::Compact);
        assert_eq!(saved.terminal_font_size, 18);
        assert_eq!(get(&SettingsStore::load(&dir)).expect("get"), saved);
    }

    #[test]
    fn a_refused_change_leaves_the_saved_appearance_alone() {
        let dir = scratch("refused");
        let store = SettingsStore::load(&dir);
        let before = get(&store).expect("get");
        for bad in [input("neon", 13), input("dark", 30)] {
            let e = set(&store, &bad).expect_err("refused");
            assert_eq!(e.code, ErrorCode::InvalidInput);
        }
        assert_eq!(get(&store).expect("get"), before);
        assert!(!dir.join("settings.json").exists(), "nothing was written");
    }
}
