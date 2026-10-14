//! Process-wide CLI context: profile database, keychain, Entra session.

use anyhow::Context as _;
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::path::PathBuf;
use std::sync::Arc;

pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
pub const KEY_PREFIX: &str = "dispatched";

pub fn config_dir() -> PathBuf {
    std::env::var_os("DISPATCHED_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("dispatched")
        })
}

pub struct Ctx {
    pub store: ProfileStore,
    /// The OS keychain in production; an in-memory store in tests.
    pub secrets: Arc<dyn CredentialStore>,
    pub session: Arc<EntraSession>,
}

impl Ctx {
    /// Open `{config_dir}/dispatched.db` (migrating the M0 `cli-state.json`)
    /// and the OS keychain.
    pub fn open() -> anyhow::Result<Self> {
        let dir = config_dir();
        let store = ProfileStore::open(&dir.join("dispatched.db"))?;
        store.migrate_cli_state(&dir.join("cli-state.json"))?;
        Ok(Self::from_parts(
            store,
            Arc::new(OsKeychain::new(KEYCHAIN_SERVICE)),
        ))
    }

    /// A context over the given store and credential store (tests use
    /// in-memory ones, so they never touch the keychain or the config dir).
    pub fn from_parts(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let session = Arc::new(EntraSession::new(
            AzureCloud::Public,
            secrets.clone(),
            KEY_PREFIX,
        ));
        Self {
            store,
            secrets,
            session,
        }
    }

    /// The current sign-in, cleared if its refresh token is gone (cleared
    /// keychain, another machine's database), so commands report "not signed
    /// in" instead of failing later.
    pub fn current_sign_in(&self) -> anyhow::Result<Option<SignIn>> {
        let Some(s) = self.store.current_sign_in()? else {
            return Ok(None);
        };
        if self
            .session
            .is_signed_in(&s)
            .context("checking the stored sign-in")?
        {
            Ok(Some(s))
        } else {
            self.store.set_current_sign_in(None)?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_azure_kit::secrets::MemoryStore;

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
    fn stale_sign_in_is_cleared() {
        // An empty credential store: the remembered sign-in has no refresh token.
        let ctx = Ctx::from_parts(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        );
        let s = sample_sign_in();
        ctx.store.save_sign_in(&s).unwrap();
        ctx.store.set_current_sign_in(Some(&s.id)).unwrap();
        assert!(ctx.current_sign_in().unwrap().is_none());
        assert!(ctx.store.current_sign_in().unwrap().is_none());
        // Nothing left to clear.
        assert!(ctx.current_sign_in().unwrap().is_none());
    }
}
