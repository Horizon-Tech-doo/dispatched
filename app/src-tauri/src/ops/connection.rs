use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::ops::profiles::namespace_for_profile;
use crate::state::{Active, AppState};
use dispatched_core::{Explorer, Transport};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct ConnectionView {
    pub profile_id: String,
    pub profile_name: String,
    pub host: String,
    pub emulator: bool,
    /// e.g. "SAS key reader" or "Entra ID ops@contoso.example".
    pub auth: String,
    pub transport: Transport,
    pub production: bool,
}

pub async fn connect(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
) -> Result<ConnectionView, CommandError> {
    let p = state
        .store
        .get_by_id(profile_id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    let name = p.name.clone();
    tracked(
        ev,
        &format!("Connecting to {name} failed"),
        async {
            // Held across the swap and the close of the previous explorer.
            let _serial = state.connect_lock.lock().await;
            let ns = namespace_for_profile(state, &p)?;
            let view = ConnectionView {
                profile_id: p.id.clone(),
                profile_name: p.name.clone(),
                host: ns.host().to_string(),
                emulator: ns.is_emulator(),
                auth: ns.tokens().describe(),
                transport: ns.transport(),
                production: p.production,
            };
            let explorer = Arc::new(Explorer::new(ns)?);
            let previous = state.active.write().await.replace(Active {
                profile_id: p.id.clone(),
                explorer,
            });
            if let Some(old) = previous {
                old.explorer.close().await;
            }
            Ok(view)
        },
        |_| format!("Connected to {name}"),
    )
    .await
}

pub async fn disconnect(state: &AppState, ev: &dyn Events) -> Result<(), CommandError> {
    tracked(
        ev,
        "Disconnecting failed",
        async {
            let _serial = state.connect_lock.lock().await;
            // Take it out first so the write lock is not held while closing.
            let old = state.active.write().await.take();
            if let Some(old) = old {
                old.explorer.close().await;
            }
            Ok(())
        },
        |_| "Disconnected".to_string(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::{CS, state};
    use dispatched_core::profiles::Profile;
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::auth::SignIn;

    #[tokio::test]
    async fn connect_sets_the_active_profile_and_disconnect_clears_it() {
        let s = state();
        let ev = Recorder::default();
        let p = s
            .store
            .add_connection_string("Logistics", CS, s.secrets.as_ref(), true)
            .unwrap();
        let view = connect(&s, &ev, &p.id).await.unwrap();
        assert_eq!(view.host, "myns.servicebus.windows.net");
        assert!(view.production);
        assert_eq!(s.active().await.unwrap().profile_id, p.id);
        disconnect(&s, &ev).await.unwrap();
        assert_eq!(s.active().await.unwrap_err().kind, "not_connected");
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Connected to Logistics");
        assert_eq!(log[1].action, "Disconnected");
    }

    #[tokio::test]
    async fn connect_with_lost_sign_in_says_sign_in_again() {
        let s = state();
        let si = SignIn {
            id: "oid-1".into(),
            user_principal: "ops@contoso.example".into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        };
        s.store.save_sign_in(&si).unwrap();
        let p = Profile::entra("Ops", "myns.servicebus.windows.net", "oid-1", None);
        s.store.insert(&p).unwrap();
        // No refresh token in the (memory) keychain.
        let e = connect(&s, &Recorder::default(), &p.id).await.unwrap_err();
        assert_eq!(e.kind, "not_signed_in");
        assert!(e.message.contains("sign in again"), "{}", e.message);
        assert!(s.active().await.is_err());
    }

    #[tokio::test]
    async fn concurrent_connects_leave_the_last_one_active() {
        let s = state();
        let ev = Recorder::default();
        let a = s
            .store
            .add_connection_string("A", CS, s.secrets.as_ref(), false)
            .unwrap();
        let b = s
            .store
            .add_connection_string("B", &CS.replace("myns", "other"), s.secrets.as_ref(), false)
            .unwrap();
        let (ra, rb) = tokio::join!(connect(&s, &ev, &a.id), connect(&s, &ev, &b.id));
        assert_eq!(ra.unwrap().profile_id, a.id);
        assert_eq!(rb.unwrap().profile_id, b.id);
        assert_eq!(s.active().await.unwrap().profile_id, b.id);
        // Listing for the profile that is no longer active is refused.
        assert_eq!(s.active_for(&a.id).await.unwrap_err().kind, "not_connected");
        assert_eq!(s.active_for(&b.id).await.unwrap().profile_id, b.id);
    }

    #[tokio::test]
    async fn connect_to_a_missing_profile_is_not_found() {
        let s = state();
        assert_eq!(
            connect(&s, &Recorder::default(), "nope")
                .await
                .unwrap_err()
                .kind,
            "not_found"
        );
    }
}
