use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::AppState;
use dispatched_core::diagnostics::{Check, ConnectionReport, test_connection};
use dispatched_core::profiles::{Profile, ProfileAuth, ProfileDraft, namespace_for};
use dispatched_core::{Error, Explorer, Namespace};

pub fn list(state: &AppState) -> Result<Vec<Profile>, CommandError> {
    Ok(state.store.list()?)
}

pub async fn save(state: &AppState, ev: &dyn Events, draft: ProfileDraft) -> Result<Profile, CommandError> {
    tracked(
        ev,
        "Saving the profile failed",
        async {
            let sign_in = match draft.auth {
                dispatched_core::profiles::DraftAuth::Entra { .. } => state.current_sign_in()?,
                _ => None,
            };
            Ok(state.store.save_draft(&draft, state.secrets.as_ref(), sign_in.as_ref())?)
        },
        |p| format!("Saved profile {}", p.name),
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
        "Removing the profile failed",
        async {
            let was_active = state.active.read().await.as_ref().is_some_and(|a| a.profile_id == id);
            if was_active
                && let Some(old) = state.active.write().await.take()
            {
                old.explorer.close().await;
            }
            state.store.delete(&profile.name, state.secrets.as_ref())?;
            state.store.clear_app_settings_with_prefix(&format!("shell:{id}"))?;
            Ok(())
        },
        |_| format!("Removed profile {name}"),
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
    Ok(namespace_for(p, state.secrets.as_ref(), &state.session, sign_in.as_ref())?)
}

pub async fn test(state: &AppState, ev: &dyn Events, id: &str) -> Result<ConnectionReport, CommandError> {
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
        |r| format!("Tested {}: management {}, messaging {}", p.name, check_word(&r.management), check_word(&r.messaging)),
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
            auth: DraftAuth::ConnectionString { connection_string: cs.map(str::to_string) },
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
        assert!(!serde_json::to_string(&list(&s).unwrap()).unwrap().contains(SECRET_PART));
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Saved profile Logistics"), "{log}");
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
        assert!(log.contains("Saving the profile failed"), "{log}");
        assert!(!log.contains("TOPSECRET"), "{log}");
    }

    #[tokio::test]
    async fn delete_profile_clears_its_secret_and_shell_state() {
        let s = state();
        let ev = Recorder::default();
        let p = save(&s, &ev, draft("Logistics", Some(CS))).await.unwrap();
        s.store.set_app_setting(&format!("shell:{}", p.id), Some("{}")).unwrap();
        s.store.set_app_setting("ui", Some("{}")).unwrap();
        delete(&s, &ev, &p.id).await.unwrap();
        assert!(list(&s).unwrap().is_empty());
        assert!(s.secrets.get(&dispatched_core::profiles::secret_key(&p.id)).unwrap().is_none());
        assert!(s.store.app_setting(&format!("shell:{}", p.id)).unwrap().is_none());
        assert!(s.store.app_setting("ui").unwrap().is_some());
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
        assert!(e.message.contains("Orphan") && e.message.contains("connection string"), "{}", e.message);
    }
}
