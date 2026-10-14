//! Process-wide CLI context: profile database, keychain, Entra session.

use anyhow::Context as _;
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
pub const KEY_PREFIX: &str = "dispatched";

/// The directory holding `dispatched.db`: `DISPATCHED_CONFIG_DIR`, else the
/// platform config directory. Never falls back to the working directory.
pub fn config_dir() -> anyhow::Result<PathBuf> {
    config_dir_from(
        std::env::var_os("DISPATCHED_CONFIG_DIR"),
        dirs::config_dir(),
    )
}

fn config_dir_from(env: Option<OsString>, platform: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    platform.map(|d| d.join("dispatched")).context(
        "cannot determine the config directory (no home directory); \
         set DISPATCHED_CONFIG_DIR to a writable directory",
    )
}

type Opener = Box<dyn Fn() -> anyhow::Result<ProfileStore> + Send + Sync>;

/// Anything that can hand out the profile store (opening it on first use).
pub trait StoreSource {
    fn store(&self) -> anyhow::Result<&ProfileStore>;
}

impl StoreSource for ProfileStore {
    fn store(&self) -> anyhow::Result<&ProfileStore> {
        Ok(self)
    }
}

pub struct Ctx {
    store: OnceLock<ProfileStore>,
    opener: Opener,
    /// The OS keychain in production; an in-memory store in tests.
    pub secrets: Arc<dyn CredentialStore>,
    pub session: Arc<EntraSession>,
}

impl StoreSource for Ctx {
    fn store(&self) -> anyhow::Result<&ProfileStore> {
        if let Some(s) = self.store.get() {
            return Ok(s);
        }
        let opened = (self.opener)()?;
        // A concurrent first open would keep the winner's handle.
        let _ = self.store.set(opened);
        Ok(self.store.get().expect("store was just set"))
    }
}

impl Ctx {
    /// A context over the OS keychain and a profile store that is only
    /// opened (creating `{config_dir}/dispatched.db`, migrating the M0
    /// `cli-state.json`) when a command first needs it, so connection-string
    /// use never touches the config directory or the keychain.
    pub fn open() -> Self {
        Self::lazy(
            Box::new(|| {
                let dir = config_dir()?;
                let store = ProfileStore::open(&dir.join("dispatched.db"))?;
                store.migrate_cli_state(&dir.join("cli-state.json"))?;
                Ok(store)
            }),
            Arc::new(OsKeychain::new(KEYCHAIN_SERVICE)),
        )
    }

    /// A context whose store is produced by `opener` on first use (tests
    /// pass an opener that counts or panics).
    pub fn lazy(opener: Opener, secrets: Arc<dyn CredentialStore>) -> Self {
        let session = Arc::new(EntraSession::new(
            AzureCloud::Public,
            secrets.clone(),
            KEY_PREFIX,
        ));
        Self {
            store: OnceLock::new(),
            opener,
            secrets,
            session,
        }
    }

    /// A context over the given store and credential store (tests use
    /// in-memory ones, so they never touch the keychain or the config dir).
    pub fn from_parts(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let ctx = Self::lazy(
            Box::new(|| anyhow::bail!("the profile store was not provided")),
            secrets,
        );
        let _ = ctx.store.set(store);
        ctx
    }

    /// The profile store, opened on first use.
    pub fn store(&self) -> anyhow::Result<&ProfileStore> {
        StoreSource::store(self)
    }

    /// The current sign-in, cleared if its refresh token is gone (cleared
    /// keychain, another machine's database), so commands report "not signed
    /// in" instead of failing later.
    pub fn current_sign_in(&self) -> anyhow::Result<Option<SignIn>> {
        let store = self.store()?;
        let Some(s) = store.current_sign_in()? else {
            return Ok(None);
        };
        if self
            .session
            .is_signed_in(&s)
            .context("checking the stored sign-in")?
        {
            Ok(Some(s))
        } else {
            store.set_current_sign_in(None)?;
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
        ctx.store().unwrap().save_sign_in(&s).unwrap();
        ctx.store()
            .unwrap()
            .set_current_sign_in(Some(&s.id))
            .unwrap();
        assert!(ctx.current_sign_in().unwrap().is_none());
        assert!(ctx.store().unwrap().current_sign_in().unwrap().is_none());
        // Nothing left to clear.
        assert!(ctx.current_sign_in().unwrap().is_none());
    }

    #[test]
    fn the_store_opens_once_and_only_on_first_use() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = opens.clone();
        let ctx = Ctx::lazy(
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                ProfileStore::open_in_memory().map_err(Into::into)
            }),
            Arc::new(MemoryStore::new()),
        );
        assert_eq!(
            opens.load(Ordering::SeqCst),
            0,
            "constructing opens nothing"
        );
        ctx.store().unwrap();
        ctx.store().unwrap();
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn config_dir_prefers_the_override_and_never_uses_the_cwd() {
        let env = Some(OsString::from("/custom"));
        assert_eq!(
            config_dir_from(env, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/custom")
        );
        assert_eq!(
            config_dir_from(None, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/home/u/.config/dispatched")
        );
        // An empty override counts as unset.
        assert_eq!(
            config_dir_from(Some(OsString::new()), Some("/c".into())).unwrap(),
            PathBuf::from("/c/dispatched")
        );
        let e = config_dir_from(None, None).unwrap_err().to_string();
        assert!(e.contains("DISPATCHED_CONFIG_DIR"), "{e}");
    }
}
