use crate::error::CommandError;
use dispatched_core::Explorer;
use dispatched_core::paths::{KEY_PREFIX, KEYCHAIN_SERVICE};
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

/// The connected profile and its explorer.
#[derive(Clone)]
pub struct Active {
    pub profile_id: String,
    pub explorer: Arc<Explorer>,
}

/// App-wide state.
pub struct AppState {
    pub store: ProfileStore,
    pub secrets: Arc<dyn CredentialStore>,
    pub session: Arc<EntraSession>,
    pub active: RwLock<Option<Active>>,
    // Read by the sign-in commands added in the next task.
    #[allow(dead_code)]
    /// Wakes a running `sign_in` so it returns `cancelled`.
    pub sign_in_cancel: Notify,
    /// Why the real database could not be opened, if it couldn't.
    pub startup_error: Option<String>,
}

impl AppState {
    pub fn new(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let session = Arc::new(EntraSession::new(AzureCloud::Public, secrets.clone(), KEY_PREFIX));
        Self {
            store,
            secrets,
            session,
            active: RwLock::new(None),
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
                    "Your saved profiles could not be opened ({e}). Changes made now will not be kept."
                ));
                s
            }
        }
    }

    // Used by the connection and tree commands added in the next task.
    #[allow(dead_code)]
    pub async fn active(&self) -> Result<Active, CommandError> {
        self.active
            .read()
            .await
            .clone()
            .ok_or_else(|| CommandError::new("not_connected", "Choose a connection first."))
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
