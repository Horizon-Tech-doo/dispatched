//! Where Dispatched keeps its database and secrets. Shared by the CLI and the
//! desktop app so both see the same profiles and sign-ins.

use crate::profiles::ProfileStore;
use crate::{Error, Result};
use std::ffi::OsString;
use std::path::PathBuf;

/// OS keychain service holding profile secrets and Entra refresh tokens.
pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
/// Key prefix for the kit's Entra session entries in the keychain.
pub const KEY_PREFIX: &str = "dispatched";
/// Database file inside [`config_dir`].
pub const DB_FILE: &str = "dispatched.db";
/// M0's CLI state file, migrated into the database on first open.
pub const LEGACY_CLI_STATE: &str = "cli-state.json";

/// The directory holding `dispatched.db`: `DISPATCHED_CONFIG_DIR`, else the
/// platform config directory. Never falls back to the working directory.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_from(
        std::env::var_os("DISPATCHED_CONFIG_DIR"),
        dirs::config_dir(),
    )
}

/// [`config_dir`] with its inputs passed in (tests).
pub fn config_dir_from(env: Option<OsString>, platform: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    platform.map(|d| d.join("dispatched")).ok_or_else(|| {
        Error::Io(
            "cannot determine the config directory (no home directory); \
             set DISPATCHED_CONFIG_DIR to a writable directory"
                .into(),
        )
    })
}

/// Open the profile database at its standard location, migrating M0's CLI
/// state file once.
pub fn open_store() -> Result<ProfileStore> {
    let dir = config_dir()?;
    let store = ProfileStore::open(&dir.join(DB_FILE))?;
    store.migrate_cli_state(&dir.join(LEGACY_CLI_STATE))?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_prefers_the_override_and_never_uses_the_cwd() {
        let env = Some(OsString::from("/tmp/dispatched-test"));
        assert_eq!(
            config_dir_from(env, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/tmp/dispatched-test")
        );
        assert_eq!(
            config_dir_from(None, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/home/u/.config/dispatched")
        );
        assert_eq!(
            config_dir_from(Some(OsString::new()), Some("/c".into())).unwrap(),
            PathBuf::from("/c/dispatched")
        );
        let e = config_dir_from(None, None).unwrap_err().to_string();
        assert!(e.contains("DISPATCHED_CONFIG_DIR"), "{e}");
    }
}
