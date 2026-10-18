//! Process-wide CLI context: profile database, activity log, keychain, Entra session.

use anyhow::Context as _;
use dispatched_core::ActivityLog;
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::sync::{Arc, OnceLock};

pub use dispatched_core::paths::{KEY_PREFIX, KEYCHAIN_SERVICE};

type Opener = Box<dyn Fn() -> anyhow::Result<ProfileStore> + Send + Sync>;
type ActivityOpener = Box<dyn Fn() -> anyhow::Result<ActivityLog> + Send + Sync>;

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
    activity: OnceLock<Arc<ActivityLog>>,
    activity_opener: ActivityOpener,
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
            Box::new(|| Ok(dispatched_core::paths::open_store()?)),
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
            activity: OnceLock::new(),
            activity_opener: Box::new(|| Ok(dispatched_core::paths::open_activity_log()?)),
            secrets,
            session,
        }
    }

    #[cfg(test)]
    /// A context over the given store and credential store (tests use
    /// in-memory ones, so they never touch the keychain or the config dir).
    /// The activity log is in memory too.
    pub fn from_parts(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let mut ctx = Self::lazy(
            Box::new(|| anyhow::bail!("the profile store was not provided")),
            secrets,
        );
        ctx.activity_opener = Box::new(|| Ok(ActivityLog::open_in_memory()?));
        let _ = ctx.store.set(store);
        ctx
    }

    #[cfg(test)]
    /// An in-memory context whose store holds these profiles: `(name, host, production)`.
    pub fn for_test_with_profiles(profiles: &[(&str, &str, bool)]) -> Self {
        let store = ProfileStore::open_in_memory().expect("in-memory store");
        for (name, host, production) in profiles {
            let mut p = dispatched_core::profiles::Profile::entra(name, host, "o", None);
            p.production = *production;
            store.insert(&p).expect("profile saved");
        }
        Self::from_parts(
            store,
            Arc::new(horizon_azure_kit::secrets::MemoryStore::new()),
        )
    }

    /// The activity log, opened (creating `{config_dir}/activity.db`) on first use, so
    /// read-only commands never touch it.
    pub fn activity(&self) -> anyhow::Result<&Arc<ActivityLog>> {
        if let Some(log) = self.activity.get() {
            return Ok(log);
        }
        let opened = (self.activity_opener)().context("opening the activity log")?;
        // A concurrent first open would keep the winner's handle.
        let _ = self.activity.set(Arc::new(opened));
        Ok(self.activity.get().expect("activity log was just set"))
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
}
