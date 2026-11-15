//! Location of user data. User data (connections JSON, `vault.bin`, `known_hosts`) lives in
//! the OS data directory, never in the repository.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::ModelError;

/// Environment variable that overrides the data directory (development and tests).
pub const DATA_DIR_ENV: &str = "PORTOLANO_DATA_DIR";

const APP_DIR: &str = "portolano";

/// Pure resolution rule: a non-empty override wins, otherwise `<os data dir>/portolano`.
pub fn resolve_data_dir(
    env_override: Option<OsString>,
    os_data_dir: Option<PathBuf>,
) -> Result<PathBuf, ModelError> {
    if let Some(value) = env_override {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    os_data_dir
        .map(|dir| dir.join(APP_DIR))
        .ok_or(ModelError::NoDataDir)
}

/// The data directory of the running process. `PORTOLANO_DATA_DIR` is honoured in development
/// and test builds only: a release build always uses the OS data directory, so a variable that
/// leaked into the environment cannot redirect where a user's vault and connections are read
/// from. The directory is not created here.
pub fn data_dir() -> Result<PathBuf, ModelError> {
    resolve_data_dir(
        override_from_env(std::env::var_os(DATA_DIR_ENV), cfg!(debug_assertions)),
        dirs::data_dir(),
    )
}

/// `value` when overrides are allowed, nothing otherwise.
fn override_from_env(value: Option<OsString>, allowed: bool) -> Option<OsString> {
    value.filter(|_| allowed)
}

pub fn connections_path(data_dir: &Path) -> PathBuf {
    data_dir.join("connections.json")
}

pub fn vault_path(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.bin")
}

pub fn known_hosts_path(data_dir: &Path) -> PathBuf {
    data_dir.join("known_hosts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        let dir = resolve_data_dir(Some("/tmp/x".into()), Some("/os".into())).expect("dir");
        assert_eq!(dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn a_release_build_ignores_the_override() {
        assert_eq!(
            override_from_env(Some("/tmp/x".into()), false),
            None,
            "release builds never take the directory from the environment"
        );
        assert_eq!(
            override_from_env(Some("/tmp/x".into()), true),
            Some(OsString::from("/tmp/x"))
        );
    }

    #[test]
    fn empty_override_is_ignored() {
        let dir = resolve_data_dir(Some(OsString::new()), Some("/os".into())).expect("dir");
        assert_eq!(dir, PathBuf::from("/os/portolano"));
    }

    #[test]
    fn os_default_gets_app_folder() {
        let dir = resolve_data_dir(None, Some("/os".into())).expect("dir");
        assert_eq!(dir, PathBuf::from("/os/portolano"));
    }

    #[test]
    fn missing_everything_is_an_error() {
        assert!(matches!(
            resolve_data_dir(None, None),
            Err(ModelError::NoDataDir)
        ));
    }

    #[test]
    fn file_names_are_stable() {
        let base = Path::new("/d");
        assert_eq!(connections_path(base), PathBuf::from("/d/connections.json"));
        assert_eq!(vault_path(base), PathBuf::from("/d/vault.bin"));
        assert_eq!(known_hosts_path(base), PathBuf::from("/d/known_hosts"));
    }
}
