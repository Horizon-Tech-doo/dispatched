use crate::error::CommandError;
use dispatched_core::Explorer;
use dispatched_core::paths::{KEY_PREFIX, KEYCHAIN_SERVICE};
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, RwLock};

/// The connected profile and its explorer.
#[derive(Clone)]
pub struct Active {
    pub profile_id: String,
    pub explorer: Arc<Explorer>,
}

// Hand-written: `Explorer` holds connections and credentials and is not `Debug`.
impl std::fmt::Debug for Active {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Active")
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

/// App-wide state.
pub struct AppState {
    pub store: ProfileStore,
    pub secrets: Arc<dyn CredentialStore>,
    pub session: Arc<EntraSession>,
    pub active: RwLock<Option<Active>>,
    /// Serialises connect, disconnect and delete, so two quick connects can't interleave their
    /// swap and close: the one that starts last finishes last and stays active.
    pub connect_lock: Mutex<()>,
    /// Wakes a running `sign_in` so it returns `cancelled`.
    pub sign_in_cancel: Notify,
    /// Why the real database could not be opened, if it couldn't.
    pub startup_error: Option<String>,
}

impl AppState {
    pub fn new(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let session = Arc::new(EntraSession::new(
            AzureCloud::Public,
            secrets.clone(),
            KEY_PREFIX,
        ));
        Self {
            store,
            secrets,
            session,
            active: RwLock::new(None),
            connect_lock: Mutex::new(()),
            sign_in_cancel: Notify::new(),
            startup_error: None,
        }
    }

    /// State over the real database and the OS keychain. If the database
    /// can't be opened the app still starts, on an empty in-memory store, and
    /// the UI shows `startup_error`.
    pub fn open() -> Self {
        let secrets: Arc<dyn CredentialStore> = Arc::new(OsKeychain::new(KEYCHAIN_SERVICE));
        match dispatched_core::paths::open_store() {
            Ok(store) => Self::new(store, secrets),
            Err(e) => {
                let mut s = Self::new(
                    ProfileStore::open_in_memory().expect("in-memory SQLite opens"),
                    secrets,
                );
                s.startup_error = Some(format!(
                    "Your saved connections could not be opened ({e}). Changes made now will not be kept."
                ));
                s
            }
        }
    }

    pub async fn active(&self) -> Result<Active, CommandError> {
        self.active
            .read()
            .await
            .clone()
            .ok_or_else(|| CommandError::new("not_connected", "Choose a connection first."))
    }

    /// The active connection, but only if it belongs to `profile_id`: a request made for a profile
    /// the user has since switched away from must not list the new one under the old one's name.
    pub async fn active_for(&self, profile_id: &str) -> Result<Active, CommandError> {
        let active = self.active().await?;
        if active.profile_id == profile_id {
            Ok(active)
        } else {
            Err(CommandError::new(
                "not_connected",
                "Choose a connection first.",
            ))
        }
    }

    /// The current sign-in, cleared if its refresh token is gone.
    pub fn current_sign_in(&self) -> Result<Option<SignIn>, CommandError> {
        let Some(s) = self.store.current_sign_in()? else {
            return Ok(None);
        };
        if self.session.is_signed_in(&s)? {
            Ok(Some(s))
        } else {
            self.store.set_current_sign_in(None)?;
            Ok(None)
        }
    }
}
