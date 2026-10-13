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

pub fn state_path() -> PathBuf {
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

/// If the remembered sign-in no longer has a refresh token in the keychain
/// (cleared keychain, another machine's state file), forget it and persist
/// that, so commands report "not signed in" instead of failing later.
/// Returns whether a stale sign-in was cleared.
pub fn prune_stale_sign_in(
    session: &EntraSession,
    state: &mut CliState,
    path: &Path,
) -> anyhow::Result<bool> {
    let Some(sign_in) = &state.sign_in else {
        return Ok(false);
    };
    if session.is_signed_in(sign_in)? {
        return Ok(false);
    }
    state.sign_in = None;
    save_to(path, state)?;
    Ok(true)
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

    fn sample_sign_in() -> SignIn {
        SignIn {
            id: "o".into(),
            user_principal: "a@x.io".into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        }
    }

    #[test]
    fn stale_sign_in_is_cleared_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli-state.json");
        // An empty credential store: the remembered sign-in has no refresh token.
        let session = EntraSession::new(
            AzureCloud::Public,
            Arc::new(horizon_azure_kit::secrets::MemoryStore::new()),
            KEY_PREFIX,
        );
        let mut state = CliState {
            sign_in: Some(sample_sign_in()),
        };
        save_to(&path, &state).unwrap();
        assert!(prune_stale_sign_in(&session, &mut state, &path).unwrap());
        assert!(state.sign_in.is_none());
        assert!(load_from(&path).sign_in.is_none());
        // Nothing left to clear.
        assert!(!prune_stale_sign_in(&session, &mut state, &path).unwrap());
    }

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
