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

    /// A remembered sign-in made in `tenant_id` itself (a guest tenant's own sign-in) by the current
    /// account, with its refresh token. The current account is the current sign-in's identity: the same
    /// object id, or the same user principal (any case). Another account's sign-in is never returned:
    /// without a match the caller uses the current sign-in and asks for the tenant.
    pub fn sign_in_for_tenant(&self, tenant_id: &str) -> Result<Option<SignIn>, CommandError> {
        let Some(current) = self.store.current_sign_in()? else {
            return Ok(None);
        };
        self.live_sign_in_like(tenant_id, &current)
    }

    /// A sign-in made in `tenant_id` by the same identity as `who`, whose refresh token exists; `who`
    /// itself first.
    pub fn live_sign_in_like(
        &self,
        tenant_id: &str,
        who: &SignIn,
    ) -> Result<Option<SignIn>, CommandError> {
        let mut found = None;
        for s in self.store.sign_ins_for_tenant(tenant_id)? {
            if same_identity(&s, who) && self.session.is_signed_in(&s)? {
                if s.id == who.id {
                    return Ok(Some(s));
                }
                found.get_or_insert(s);
            }
        }
        Ok(found)
    }

    /// The sign-in an Entra profile bound to `sign_in_id` (asking for `tenant`, if any) uses now. The same
    /// person's own sign-in in that tenant comes first, since it has satisfied the tenant's own sign-in
    /// rules (MFA) where the bound sign-in asking for the tenant may not; then the bound sign-in while it
    /// has its refresh token; else the same person's live sign-in in the bound sign-in's tenant. Returns
    /// the bound record too.
    pub fn usable_sign_in(
        &self,
        sign_in_id: &str,
        tenant: Option<&str>,
    ) -> Result<(Option<SignIn>, Option<SignIn>), CommandError> {
        let Some(bound) = self.store.sign_in(sign_in_id)? else {
            return Ok((None, None));
        };
        if let Some(t) = tenant.filter(|t| !t.eq_ignore_ascii_case(&bound.tenant_id))
            && let Some(own) = self.live_sign_in_like(t, &bound)?
        {
            return Ok((Some(bound), Some(own)));
        }
        if self.session.is_signed_in(&bound)? {
            return Ok((Some(bound.clone()), Some(bound)));
        }
        let usable = self.live_sign_in_like(&bound.tenant_id, &bound)?;
        Ok((Some(bound), usable))
    }
}

/// Whether two sign-ins are the same person: the same object id, or the same user principal (any case).
/// An empty user principal, or the kit's "unknown" placeholder, never matches: then only the id counts.
pub fn same_identity(a: &SignIn, b: &SignIn) -> bool {
    let known = |upn: &str| !upn.trim().is_empty() && !upn.trim().eq_ignore_ascii_case("unknown");
    a.id == b.id
        || (known(&a.user_principal) && a.user_principal.eq_ignore_ascii_case(&b.user_principal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn si(id: &str, upn: &str) -> SignIn {
        SignIn {
            id: id.into(),
            user_principal: upn.into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        }
    }

    #[test]
    fn identity_matches_by_id_or_a_real_user_principal_only() {
        assert!(same_identity(&si("a", "x"), &si("a", "y")));
        assert!(same_identity(&si("a", "Ops@X.io"), &si("b", "ops@x.io")));
        for placeholder in ["unknown", "UNKNOWN", "", "  "] {
            assert!(
                !same_identity(&si("a", placeholder), &si("b", placeholder)),
                "{placeholder:?}"
            );
            assert!(same_identity(&si("a", placeholder), &si("a", placeholder)));
        }
    }
}
