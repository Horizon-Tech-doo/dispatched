use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::{AppState, same_identity};
use dispatched_core::diagnostics::{Check, ConnectionReport, test_connection};
use dispatched_core::profiles::store::normalize_tenant;
use dispatched_core::profiles::{DraftAuth, Profile, ProfileAuth, ProfileDraft, namespace_for};
use dispatched_core::{Error, Explorer, Namespace};
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;

/// The sign-in an Entra profile uses, as the UI needs it: whose it is and whether it still works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundSignIn {
    pub id: String,
    pub user_principal: String,
    /// The sign-in's own (home) tenant.
    pub tenant_id: String,
    /// Its refresh token exists, so connecting can get tokens without signing in again.
    pub live: bool,
    /// It is the current account's identity (same object id or user principal).
    pub same_account: bool,
    /// It is a tenant's own sign-in, made outside the current account's home tenant: the profile's
    /// tenant is that one even though none is stored.
    pub own_tenant: bool,
}

/// A profile as the UI sees it: the stored profile plus, for Entra, the sign-in it uses.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    #[serde(flatten)]
    pub profile: Profile,
    pub bound_sign_in: Option<BoundSignIn>,
}

impl std::ops::Deref for ProfileView {
    type Target = Profile;
    fn deref(&self) -> &Profile {
        &self.profile
    }
}

/// The sign-in an Entra profile uses now (see `AppState::usable_sign_in`), described for the UI. A
/// bound sign-in with no usable replacement is reported as not live.
pub fn bound_sign_in(state: &AppState, p: &Profile) -> Result<Option<BoundSignIn>, CommandError> {
    let ProfileAuth::Entra { sign_in_id, tenant } = &p.auth else {
        return Ok(None);
    };
    let (record, usable) = state.usable_sign_in(sign_in_id, tenant.as_deref())?;
    let Some(s) = usable.clone().or(record) else {
        return Ok(None);
    };
    let current = state.current_sign_in()?;
    Ok(Some(BoundSignIn {
        live: usable.is_some(),
        same_account: current.as_ref().is_some_and(|c| same_identity(c, &s)),
        own_tenant: current
            .as_ref()
            .is_some_and(|c| !c.tenant_id.eq_ignore_ascii_case(&s.tenant_id)),
        id: s.id,
        user_principal: s.user_principal,
        tenant_id: s.tenant_id,
    }))
}

pub fn view(state: &AppState, profile: Profile) -> Result<ProfileView, CommandError> {
    Ok(ProfileView {
        bound_sign_in: bound_sign_in(state, &profile)?,
        profile,
    })
}

pub fn list(state: &AppState) -> Result<Vec<ProfileView>, CommandError> {
    state
        .store
        .list()?
        .into_iter()
        .map(|p| view(state, p))
        .collect()
}

/// The sign-in an Entra profile binds to: the current sign-in for its home tenant (no tenant, or the
/// current sign-in's own tenant); for another tenant, the current account's own sign-in in that tenant
/// when there is one, else the current sign-in (asking for the tenant). Never another account's.
fn entra_sign_in(state: &AppState, tenant: Option<&str>) -> Result<Option<SignIn>, CommandError> {
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

/// The sign-in an edited Entra profile is rebound to, if any: the one `entra_sign_in` resolves for its
/// tenant, but only when that is the same identity as the sign-in the profile uses now, or the profile
/// has no working sign-in. An edit never moves a profile to another account.
fn rebind_on_edit(
    state: &AppState,
    existing: &Profile,
    tenant: Option<&str>,
) -> Result<Option<SignIn>, CommandError> {
    let ProfileAuth::Entra {
        sign_in_id,
        tenant: stored,
    } = &existing.auth
    else {
        return Ok(None);
    };
    let Some(resolved) = entra_sign_in(state, tenant)? else {
        return Ok(None);
    };
    let (record, usable) = state.usable_sign_in(sign_in_id, stored.as_deref())?;
    Ok(match usable.or(record) {
        Some(now) if !same_identity(&now, &resolved) && state.session.is_signed_in(&now)? => None,
        _ => Some(resolved),
    })
}

pub async fn save(
    state: &AppState,
    ev: &dyn Events,
    draft: ProfileDraft,
) -> Result<ProfileView, CommandError> {
    tracked(
        ev,
        "Saving the connection failed",
        async {
            let sign_in = match (&draft.auth, &draft.id) {
                (DraftAuth::Entra { tenant, .. }, None) => entra_sign_in(state, tenant.as_deref())?,
                (DraftAuth::Entra { tenant, .. }, Some(id)) => match state.store.get_by_id(id)? {
                    Some(existing) => rebind_on_edit(state, &existing, tenant.as_deref())?,
                    None => None,
                },
                _ => None,
            };
            let p = state
                .store
                .save_draft(&draft, state.secrets.as_ref(), sign_in.as_ref())?;
            view(state, p)
        },
        |p| format!("Saved connection {}", p.profile.name),
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

/// The namespace a profile points at, with errors that name the profile and the fix. An Entra profile
/// uses the sign-in `AppState::usable_sign_in` finds, which may be the same account's newer sign-in in
/// the profile's tenant (made after the profile was saved); it then asks that sign-in's own tenant.
pub fn namespace_for_profile(state: &AppState, p: &Profile) -> Result<Namespace, CommandError> {
    let (profile, sign_in) = match &p.auth {
        ProfileAuth::ConnectionString => (p.clone(), None),
        ProfileAuth::Entra { sign_in_id, tenant } => {
            let Some(s) = state.usable_sign_in(sign_in_id, tenant.as_deref())?.1 else {
                return Err(Error::NotSignedIn(format!(
                    "the sign-in used by profile `{}` has expired; sign in again",
                    p.name
                ))
                .into());
            };
            let mut used = p.clone();
            if s.id != *sign_in_id {
                used.auth = ProfileAuth::Entra {
                    sign_in_id: s.id.clone(),
                    tenant: normalize_tenant(
                        Some(tenant.as_deref().unwrap_or(&s.tenant_id)),
                        Some(&s.tenant_id),
                    ),
                };
            }
            (used, Some(s))
        }
    };
    Ok(namespace_for(
        &profile,
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
    async fn an_entra_profile_binds_to_the_current_account_s_sign_in_in_its_tenant() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_guest_sign_in(&s, &home, "guest-oid", "tenant-guest", true);
        let ev = Recorder::default();
        let guest = save(&s, &ev, entra_draft("Guest ns", Some("tenant-guest")))
            .await
            .unwrap();
        // That sign-in's home is the tenant, so no tenant is stored.
        assert_eq!(
            guest.profile.auth,
            ProfileAuth::Entra {
                sign_in_id: "guest-oid".into(),
                tenant: None
            }
        );
        let bound = guest.bound_sign_in.unwrap();
        assert!(bound.live && bound.same_account && bound.own_tenant);
        assert_eq!(bound.tenant_id, "tenant-guest");
        // No tenant, or one with no sign-in of its own: the current (home) sign-in.
        let plain = save(&s, &ev, entra_draft("Home ns", None)).await.unwrap();
        assert!(
            matches!(&plain.profile.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid")
        );
        let other = save(&s, &ev, entra_draft("Other ns", Some("tenant-other")))
            .await
            .unwrap();
        assert!(
            matches!(&other.profile.auth, ProfileAuth::Entra { sign_in_id, tenant }
            if sign_in_id == "home-oid" && tenant.as_deref() == Some("tenant-other"))
        );
    }

    #[tokio::test]
    async fn another_account_s_sign_in_is_never_bound() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        // Another account signed in to the home tenant (sorting first) and to the guest tenant.
        remember_sign_in(&s, "a-other-oid", "tenant-home", true);
        remember_sign_in(&s, "a-stranger-oid", "tenant-guest", true);
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        let ev = Recorder::default();
        let p = save(&s, &ev, entra_draft("Home ns", Some("TENANT-HOME")))
            .await
            .unwrap();
        assert_eq!(
            p.profile.auth,
            ProfileAuth::Entra {
                sign_in_id: "home-oid".into(),
                tenant: None
            }
        );
        let p = save(&s, &ev, entra_draft("Guest ns", Some("tenant-guest")))
            .await
            .unwrap();
        assert_eq!(
            p.profile.auth,
            ProfileAuth::Entra {
                sign_in_id: "home-oid".into(),
                tenant: Some("tenant-guest".into())
            }
        );
        // Without a current sign-in nothing in the tenant counts as the current account's.
        s.store.set_current_sign_in(None).unwrap();
        assert!(s.sign_in_for_tenant("tenant-guest").unwrap().is_none());
    }

    #[tokio::test]
    async fn a_tenant_sign_in_without_its_token_is_not_used_for_a_profile() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_guest_sign_in(&s, &home, "guest-oid", "tenant-guest", false);
        let p = save(
            &s,
            &Recorder::default(),
            entra_draft("Guest ns", Some("tenant-guest")),
        )
        .await
        .unwrap();
        assert!(
            matches!(&p.profile.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid")
        );
    }

    #[tokio::test]
    async fn a_guest_bound_profile_is_the_current_account_s_and_not_a_different_one() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_guest_sign_in(&s, &home, "guest-oid", "tenant-guest", true);
        let ev = Recorder::default();
        let p = save(&s, &ev, entra_draft("Guest ns", Some("tenant-guest")))
            .await
            .unwrap();
        // Editing it (the form shows the tenant the profile uses) keeps the binding, live and the same account.
        let mut edit = entra_draft("Guest ns renamed", Some("tenant-guest"));
        edit.id = Some(p.profile.id.clone());
        let e = save(&s, &ev, edit).await.unwrap();
        assert!(
            matches!(&e.profile.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "guest-oid")
        );
        let listed = list(&s).unwrap();
        let bound = listed[0].bound_sign_in.clone().unwrap();
        assert!(bound.live && bound.same_account, "{bound:?}");
        // Signed in as someone else: the profile's sign-in belongs to another identity.
        let other = remember_sign_in(&s, "other-oid", "tenant-other", true);
        s.store.set_current_sign_in(Some(&other.id)).unwrap();
        let bound = list(&s).unwrap()[0].bound_sign_in.clone().unwrap();
        assert!(bound.live && !bound.same_account, "{bound:?}");
        // ... and an edit does not move it to that account.
        let mut edit = entra_draft("Guest ns", Some("tenant-guest"));
        edit.id = Some(p.profile.id.clone());
        let e = save(&s, &ev, edit).await.unwrap();
        assert!(
            matches!(&e.profile.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "guest-oid")
        );
    }

    #[tokio::test]
    async fn an_edit_rebinds_a_profile_made_before_its_tenant_sign_in() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        let ev = Recorder::default();
        let p = save(&s, &ev, entra_draft("Guest ns", Some("tenant-guest")))
            .await
            .unwrap();
        assert!(
            matches!(&p.profile.auth, ProfileAuth::Entra { sign_in_id, .. } if sign_in_id == "home-oid")
        );
        // The user then signs in to the tenant (MFA) and saves the profile again.
        remember_guest_sign_in(&s, &home, "guest-oid", "tenant-guest", true);
        let mut edit = entra_draft("Guest ns", Some("tenant-guest"));
        edit.id = Some(p.profile.id.clone());
        let e = save(&s, &ev, edit).await.unwrap();
        assert_eq!(
            e.profile.auth,
            ProfileAuth::Entra {
                sign_in_id: "guest-oid".into(),
                tenant: None
            }
        );
    }

    #[test]
    fn an_expired_bound_sign_in_falls_back_to_the_same_person_s_tenant_sign_in() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        // Bound to a home sign-in whose token is gone, asking for the guest tenant.
        let home = remember_sign_in(&s, "home-oid", "tenant-home", false);
        let p = Profile::entra(
            "G",
            "g.servicebus.windows.net",
            "home-oid",
            Some("tenant-guest"),
        );
        s.store.insert(&p).unwrap();
        let bound = bound_sign_in(&s, &p).unwrap().unwrap();
        assert!(!bound.live);
        assert_eq!(
            namespace_for_profile(&s, &p).unwrap_err().kind,
            "not_signed_in"
        );
        // Another person's tenant sign-in does not help; the same person's does.
        remember_sign_in(&s, "stranger-oid", "tenant-guest", true);
        assert!(!bound_sign_in(&s, &p).unwrap().unwrap().live);
        remember_guest_sign_in(&s, &home, "guest-oid", "tenant-guest", true);
        let bound = bound_sign_in(&s, &p).unwrap().unwrap();
        assert!(bound.live);
        assert_eq!(bound.id, "guest-oid");
        assert!(namespace_for_profile(&s, &p).is_ok());
    }
}
