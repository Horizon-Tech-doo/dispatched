//! Plain functions behind the Tauri commands. Commands only unwrap Tauri's
//! state and handle, so everything here is testable without a window.

pub mod account;
pub mod browse;
pub mod connection;
pub mod discovery;
pub mod profiles;
pub mod settings;
pub mod tree;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::state::AppState;
    use dispatched_core::profiles::ProfileStore;
    use horizon_azure_kit::secrets::MemoryStore;
    use std::sync::Arc;

    pub const CS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0LWtleQ==";
    pub const SECRET_PART: &str = "c2VjcmV0LWtleQ";

    /// Remember a sign-in made in `tenant` and, when `with_token`, give it a refresh token in the
    /// (memory) keychain so the session counts it as signed in. Returns it.
    pub fn remember_sign_in(
        state: &AppState,
        id: &str,
        tenant: &str,
        with_token: bool,
    ) -> horizon_azure_kit::auth::SignIn {
        let s = horizon_azure_kit::auth::SignIn {
            id: id.into(),
            user_principal: format!("{id}@x.io"),
            tenant_id: tenant.into(),
            display_name: None,
            cloud: horizon_azure_kit::AzureCloud::Public,
        };
        state.store.save_sign_in(&s).unwrap();
        if with_token {
            state
                .secrets
                .put(
                    &format!("{}:entra:{id}", dispatched_core::paths::KEY_PREFIX),
                    &secrecy::SecretString::from("{}".to_string()),
                )
                .unwrap();
        }
        s
    }

    /// The same person's own sign-in in another tenant: another object id, `home`'s user principal.
    pub fn remember_guest_sign_in(
        state: &AppState,
        home: &horizon_azure_kit::auth::SignIn,
        id: &str,
        tenant: &str,
        with_token: bool,
    ) -> horizon_azure_kit::auth::SignIn {
        let s = horizon_azure_kit::auth::SignIn {
            user_principal: home.user_principal.clone(),
            ..remember_sign_in(state, id, tenant, with_token)
        };
        state.store.save_sign_in(&s).unwrap();
        s
    }

    pub fn state() -> AppState {
        AppState::new(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        )
    }
}
