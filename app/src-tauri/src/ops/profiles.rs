use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::AppState;
use dispatched_core::diagnostics::{Check, ConnectionReport, test_connection};
use dispatched_core::profiles::{Profile, ProfileAuth, ProfileDraft, namespace_for};
use dispatched_core::{Error, Explorer, Namespace};

pub fn list(state: &AppState) -> Result<Vec<Profile>, CommandError> {
    Ok(state.store.list()?)
}

/// The sign-in a new Entra profile binds to: the current sign-in for its home tenant (no tenant, or the
/// current sign-in's own tenant); for another tenant, the sign-in made in that tenant when there is one
/// (a guest tenant's own sign-in), else the current sign-in.
fn entra_sign_in(
    state: &AppState,
    tenant: Option<&str>,
) -> Result<Option<horizon_azure_kit::auth::SignIn>, CommandError> {
    let current = state.current_sign_in()?;
    let Some(t) = tenant.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(current);
    };
    if current
        .as_ref()
        .is_some_and(|c| c.tenant_id.eq_ignore_ascii_case(t))
    {
        return Ok(current);
    }
    if let Some(own) = state.sign_in_for_tenant(t)? {
        return Ok(Some(own));
    }
    Ok(current)
}

pub async fn save(
    state: &AppState,
    ev: &dyn Events,
    draft: ProfileDraft,
) -> Result<Profile, CommandError> {
    tracked(
        ev,
        "Saving the connection failed",
        async {
            let sign_in = match &draft.auth {
                dispatched_core::profiles::DraftAuth::Entra { tenant, .. } => {
                    entra_sign_in(state, tenant.as_deref())?
                }
                _ => None,
            };
            Ok(state
                .store
                .save_draft(&draft, state.secrets.as_ref(), sign_in.as_ref())?)
        },
        |p| format!("Saved connection {}", p.name),
    )
    .await
}

pub async fn delete(state: &AppState, ev: &dyn Events, id: &str) -> Result<(), CommandError> {
    let profile = state
        .store
        .get_by_id(id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    let name = profile.name.clone();
    tracked(
        ev,
        "Removing the connection failed",
        async {
            // No connect may start or finish while the profile goes away.
            let _serial = state.connect_lock.lock().await;
            // Check and take under one write lock, so only this profile's connection is closed.
            let old = {
                let mut active = state.active.write().await;
                if active.as_ref().is_some_and(|a| a.profile_id == id) {
                    active.take()
                } else {
                    None
                }
            };
            if let Some(old) = old {
                old.explorer.close().await;
            }
            state.store.delete(&profile.name, state.secrets.as_ref())?;
            state
                .store
                .clear_app_settings_with_prefix(&format!("shell:{id}"))?;
            Ok(())
        },
        |_| format!("Removed connection {name}"),
    )
    .await
}

/// The namespace a profile points at, with errors that name the profile and the fix.
pub fn namespace_for_profile(state: &AppState, p: &Profile) -> Result<Namespace, CommandError> {
    let sign_in = match &p.auth {
        ProfileAuth::ConnectionString => None,
        ProfileAuth::Entra { sign_in_id, .. } => {
            let s = state.store.sign_in(sign_in_id)?;
            match s {
                Some(s) if state.session.is_signed_in(&s)? => Some(s),
                _ => {
                    return Err(Error::NotSignedIn(format!(
                        "the sign-in used by profile `{}` has expired; sign in again",
                        p.name
                    ))
                    .into());
                }
            }
        }
    };
    Ok(namespace_for(
        p,
        state.secrets.as_ref(),
        &state.session,
        sign_in.as_ref(),
    )?)
}

pub async fn test(
    state: &AppState,
    ev: &dyn Events,
    id: &str,
) -> Result<ConnectionReport, CommandError> {
    let p = state
        .store
        .get_by_id(id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    tracked(
        ev,
        "Testing the connection failed",
        async {
            let explorer = Explorer::new(namespace_for_profile(state, &p)?)?;
            let report = test_connection(&explorer, None).await;
            explorer.close().await;
            Ok(report)
        },
        |r| {
            format!(
                "Tested {}: management {}, messaging {}",
                p.name,
                check_word(&r.management),
                check_word(&r.messaging)
            )
        },
    )
    .await
}

fn check_word(c: &Check) -> &'static str {
    match c {
        Check::Ok { .. } => "ok",
        Check::Failed { .. } => "failed",
        Check::Skipped { .. } => "skipped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::{CS, SECRET_PART, state};
    use dispatched_core::namespace::Transport;
    use dispatched_core::profiles::{DraftAuth, ProfileDraft};

    fn draft(name: &str, cs: Option<&str>) -> ProfileDraft {
        ProfileDraft {
            id: None,
            name: name.into(),
            auth: DraftAuth::ConnectionString {
                connection_string: cs.map(str::to_string),
            },
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: vec![],
        }
    }

    #[tokio::test]
    async fn save_response_and_activity_never_contain_the_secret() {
        let s = state();
        let ev = Recorder::default();
        let p = save(&s, &ev, draft("Logistics", Some(CS))).await.unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains(SECRET_PART));
        assert!(
            !serde_json::to_string(&list(&s).unwrap())
                .unwrap()
                .contains(SECRET_PART)
        );
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Saved connection Logistics"), "{log}");
        assert!(!log.contains(SECRET_PART), "{log}");
    }

    #[tokio::test]
    async fn bad_connection_string_error_does_not_echo_it() {
        let s = state();
        let ev = Recorder::default();
        let bad = "Endpoint=sb://x/;SharedAccessKey=TOPSECRET;SharedAccessKeyName";
        let e = save(&s, &ev, draft("x", Some(bad))).await.unwrap_err();
        assert!(!e.message.contains("TOPSECRET"));
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Saving the connection failed"), "{log}");
        assert!(!log.contains("TOPSECRET"), "{log}");
    }

    #[tokio::test]
    async fn delete_profile_clears_its_secret_and_shell_state() {
        let s = state();
        let ev = Recorder::default();
        let p = save(&s, &ev, draft("Logistics", Some(CS))).await.unwrap();
        s.store
            .set_app_setting(&format!("shell:{}", p.id), Some("{}"))
            .unwrap();
        s.store.set_app_setting("ui", Some("{}")).unwrap();
        delete(&s, &ev, &p.id).await.unwrap();
        assert!(list(&s).unwrap().is_empty());
        assert!(
            s.secrets
                .get(&dispatched_core::profiles::secret_key(&p.id))
                .unwrap()
                .is_none()
        );
        assert!(
            s.store
                .app_setting(&format!("shell:{}", p.id))
                .unwrap()
                .is_none()
        );
        assert!(s.store.app_setting("ui").unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_closes_only_its_own_connection() {
        let s = state();
        let ev = Recorder::default();
        let a = save(&s, &ev, draft("A", Some(CS))).await.unwrap();
        let b = save(&s, &ev, draft("B", Some(CS))).await.unwrap();
        crate::ops::connection::connect(&s, &ev, &b.id)
            .await
            .unwrap();
        delete(&s, &ev, &a.id).await.unwrap();
        assert_eq!(s.active().await.unwrap().profile_id, b.id);
        delete(&s, &ev, &b.id).await.unwrap();
        assert_eq!(s.active().await.unwrap_err().kind, "not_connected");
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Removed connection A"), "{log}");
    }

    #[tokio::test]
    async fn delete_missing_profile_is_not_found() {
        let s = state();
        let e = delete(&s, &Recorder::default(), "nope").await.unwrap_err();
        assert_eq!(e.kind, "not_found");
    }

    #[tokio::test]
    async fn connect_reports_missing_secret_with_the_fix() {
        let s = state();
        // A profile row whose keychain entry is gone (cleared keychain, copied DB).
        let p = dispatched_core::profiles::Profile::connection_string(
            "Orphan",
            &dispatched_core::ConnectionString::parse(CS).unwrap(),
        );
        s.store.insert(&p).unwrap();
        let e = namespace_for_profile(&s, &p).unwrap_err();
        assert_eq!(e.kind, "invalid");
        assert!(
            e.message.contains("Orphan") && e.message.contains("connection string"),
            "{}",
            e.message
        );
    }

    fn entra_draft(name: &str, tenant: Option<&str>) -> ProfileDraft {
        ProfileDraft {
            auth: DraftAuth::Entra {
                namespace: "sb-ops".into(),
                tenant: tenant.map(str::to_string),
            },
            ..draft(name, None)
        }
    }

    #[tokio::test]
    async fn an_entra_profile_binds_to_the_sign_in_made_in_its_own_tenant() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_sign_in(&s, "guest-oid", "tenant-guest", true);
        let ev = Recorder::default();
        let guest = save(&s, &ev, entra_draft("Guest ns", Some("tenant-guest")))
            .await
            .unwrap();
        assert_eq!(
            guest.auth,
            ProfileAuth::Entra {
                sign_in_id: "guest-oid".into(),
                tenant: Some("tenant-guest".into())
            }
        );
        // No tenant, or one with no sign-in of its own: the current (home) sign-in.
        let plain = save(&s, &ev, entra_draft("Home ns", None)).await.unwrap();
        assert!(
            matches!(&plain.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid")
        );
        let other = save(&s, &ev, entra_draft("Other ns", Some("tenant-other")))
            .await
            .unwrap();
        assert!(
            matches!(&other.auth, ProfileAuth::Entra { sign_in_id, tenant }
            if sign_in_id == "home-oid" && tenant.as_deref() == Some("tenant-other"))
        );
    }

    #[tokio::test]
    async fn the_current_sign_in_s_own_tenant_is_home_and_binds_to_it() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        // Another account signed in to the same tenant, sorting before the current one.
        remember_sign_in(&s, "a-other-oid", "tenant-home", true);
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        let ev = Recorder::default();
        let p = save(&s, &ev, entra_draft("Home ns", Some("TENANT-HOME")))
            .await
            .unwrap();
        assert!(
            matches!(&p.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid"),
            "{:?}",
            p.auth
        );
    }

    #[test]
    fn a_tenant_s_own_sign_in_prefers_the_current_account() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        // Two accounts signed in to the guest tenant; the second is the current account's guest sign-in.
        remember_sign_in(&s, "a-stranger-oid", "tenant-guest", true);
        let mine = horizon_azure_kit::auth::SignIn {
            user_principal: home.user_principal.clone(),
            ..remember_sign_in(&s, "b-guest-oid", "tenant-guest", true)
        };
        s.store.save_sign_in(&mine).unwrap();
        assert_eq!(
            s.sign_in_for_tenant("tenant-guest").unwrap().unwrap().id,
            "b-guest-oid"
        );
        // Without a current sign-in the store's order decides.
        s.store.set_current_sign_in(None).unwrap();
        assert_eq!(
            s.sign_in_for_tenant("tenant-guest").unwrap().unwrap().id,
            "a-stranger-oid"
        );
    }

    #[tokio::test]
    async fn a_tenant_sign_in_without_its_token_is_not_used_for_a_profile() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_sign_in(&s, "guest-oid", "tenant-guest", false);
        let p = save(
            &s,
            &Recorder::default(),
            entra_draft("Guest ns", Some("tenant-guest")),
        )
        .await
        .unwrap();
        assert!(
            matches!(&p.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid")
        );
    }
}
