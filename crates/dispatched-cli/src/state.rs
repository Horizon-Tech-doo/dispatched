//! Small non-secret CLI state (which Entra sign-in is current).

use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::OsKeychain;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
pub const KEY_PREFIX: &str = "dispatched";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CliState {
    pub sign_in: Option<SignIn>,
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("DISPATCHED_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("dispatched")
        })
}

fn state_path() -> PathBuf {
    config_dir().join("cli-state.json")
}

pub fn load_from(path: &Path) -> CliState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, state: &CliState) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

pub fn load() -> CliState {
    load_from(&state_path())
}

pub fn save(state: &CliState) -> anyhow::Result<()> {
    save_to(&state_path(), state)
}

pub fn entra_session() -> Arc<EntraSession> {
    Arc::new(EntraSession::new(
        AzureCloud::Public,
        Arc::new(OsKeychain::new(KEYCHAIN_SERVICE)),
        KEY_PREFIX,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrips_in_custom_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cli-state.json");
        assert!(load_from(&path).sign_in.is_none());
        let state = CliState {
            sign_in: Some(SignIn {
                id: "o".into(),
                user_principal: "a@x.io".into(),
                tenant_id: "t".into(),
                display_name: None,
                cloud: AzureCloud::Public,
            }),
        };
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path).sign_in.unwrap().user_principal, "a@x.io");
    }
}
