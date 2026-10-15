use crate::error::CommandError;
use crate::events::{Events, SignInPrompt, tracked};
use crate::state::{AppState, same_identity};
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;
use std::future::Future;

#[derive(Debug, Serialize)]
pub struct AccountView {
    pub id: String,
    pub user_principal: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
}

impl From<&SignIn> for AccountView {
    fn from(s: &SignIn) -> Self {
        Self {
            id: s.id.clone(),
            user_principal: s.user_principal.clone(),
            display_name: s.display_name.clone(),
            tenant_id: s.tenant_id.clone(),
        }
    }
}

pub fn current(state: &AppState) -> Result<Option<AccountView>, CommandError> {
    Ok(state.current_sign_in()?.as_ref().map(AccountView::from))
}

/// Await `login`, or return `cancelled` when `cancel` is called.
pub(crate) async fn wait_or_cancel(
    state: &AppState,
    login: impl Future<Output = Result<SignIn, CommandError>>,
) -> Result<SignIn, CommandError> {
    tokio::select! {
        r = login => r,
        () = state.sign_in_cancel.notified() => Err(CommandError::new("cancelled", "Sign-in cancelled.")),
    }
}

/// Whether a finished sign-in becomes the current one. Signing in without naming a tenant does, and
/// so does signing in to the current sign-in's own tenant. Signing in to another tenant is saved but
/// leaves the home sign-in current, so the account the user works under does not change.
pub(crate) fn becomes_current(
    requested: Option<&str>,
    current: Option<&SignIn>,
    new: &SignIn,
) -> bool {
    let Some(current) = current else { return true };
    match requested {
        None => true,
        Some(t) => {
            t.eq_ignore_ascii_case("organizations")
                || new.id == current.id
                || t.eq_ignore_ascii_case(&current.tenant_id)
        }
    }
}

/// A sign-in made in a tenant that did not become the current account: the tenant's own sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TenantSignIn {
    pub tenant_id: String,
    pub user_principal: String,
    /// It is the current account's identity, so that tenant is listed and connected with it. Another
    /// account's tenant sign-in is kept but never used for the current account.
    pub same_account: bool,
}

/// What `sign_in` returns: the current account, and the tenant sign-in when one was made.
#[derive(Debug, Serialize)]
pub struct SignInView {
    pub account: AccountView,
    pub tenant_sign_in: Option<TenantSignIn>,
}

/// The bookkeeping after a finished sign-in, `s`, asked for `requested`: remember it, decide whether
/// it becomes current, and say what happened for the activity log.
pub(crate) fn record_sign_in(
    state: &AppState,
    requested: Option<&str>,
    s: &SignIn,
) -> Result<(SignInView, String), CommandError> {
    let current = state.current_sign_in()?;
    state.store.save_sign_in(s)?;
    match current {
        Some(current) if !becomes_current(requested, Some(&current), s) => {
            let tenant = requested.unwrap_or(&s.tenant_id);
            let activity = format!("Signed in to tenant {tenant} as {}", s.user_principal);
            Ok((
                SignInView {
                    account: AccountView::from(&current),
                    tenant_sign_in: Some(TenantSignIn {
                        tenant_id: s.tenant_id.clone(),
                        user_principal: s.user_principal.clone(),
                        same_account: same_identity(&current, s),
                    }),
                },
                activity,
            ))
        }
        _ => {
            state.store.set_current_sign_in(Some(&s.id))?;
            Ok((
                SignInView {
                    account: AccountView::from(s),
                    tenant_sign_in: None,
                },
                format!("Signed in as {}", s.user_principal),
            ))
        }
    }
}

pub async fn sign_in(
    state: &AppState,
    ev: &dyn Events,
    tenant: Option<String>,
    device_code: bool,
) -> Result<SignInView, CommandError> {
    let requested = tenant
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let tenant = requested.clone().unwrap_or_else(|| "organizations".into());
    tracked(
        ev,
        "Sign-in failed",
        async {
            let session = state.session.clone();
            let login = async {
                let r = if device_code {
                    session
                        .login_device_code(&tenant, |dc| {
                            ev.sign_in_prompt(SignInPrompt {
                                kind: "device-code",
                                url: dc.verification_uri.clone(),
                                user_code: Some(dc.user_code.clone()),
                                message: dc.message.clone(),
                            })
                        })
                        .await
                } else {
                    session
                        .login_browser(&tenant, |url| {
                            ev.sign_in_prompt(SignInPrompt {
                                kind: "browser",
                                url: url.to_string(),
                                user_code: None,
                                message: "Finish signing in in your browser.".into(),
                            });
                            let _ = open::that(url);
                        })
                        .await
                };
                r.map_err(CommandError::from)
            };
            let s = wait_or_cancel(state, login).await?;
            record_sign_in(state, requested.as_deref(), &s)
        },
        |(_, activity)| activity.clone(),
    )
    .await
    .map(|(view, _)| view)
}

pub fn cancel(state: &AppState) {
    state.sign_in_cancel.notify_waiters();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_util::state;

    fn si(id: &str, tenant: &str) -> SignIn {
        SignIn {
            id: id.into(),
            user_principal: format!("{id}@x.io"),
            tenant_id: tenant.into(),
            display_name: None,
            cloud: horizon_azure_kit::AzureCloud::Public,
        }
    }

    #[test]
    fn only_a_sign_in_without_a_tenant_or_for_the_home_tenant_becomes_current() {
        let home = si("home", "tenant-home");
        let guest = si("guest", "tenant-guest");
        // Nothing current yet: whatever signs in becomes current.
        assert!(becomes_current(None, None, &home));
        assert!(becomes_current(Some("tenant-guest"), None, &guest));
        // No tenant named, or the default: the account the user picked becomes current.
        assert!(becomes_current(
            None,
            Some(&home),
            &si("other", "tenant-other")
        ));
        assert!(becomes_current(
            Some("organizations"),
            Some(&home),
            &si("other", "x")
        ));
        // The home tenant again, in any case, or the same account.
        assert!(becomes_current(Some("TENANT-HOME"), Some(&home), &home));
        assert!(becomes_current(Some("whatever"), Some(&home), &home));
        // Another tenant: saved, but the home sign-in stays current.
        assert!(!becomes_current(Some("tenant-guest"), Some(&home), &guest));
    }

    #[test]
    fn a_sign_in_without_a_current_one_becomes_current_and_says_who() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        let (view, activity) = record_sign_in(&s, None, &home).unwrap();
        assert_eq!(view.account.id, "home-oid");
        assert!(view.tenant_sign_in.is_none());
        assert_eq!(activity, "Signed in as home-oid@x.io");
        assert_eq!(s.current_sign_in().unwrap().unwrap().id, "home-oid");
    }

    #[test]
    fn a_tenant_sign_in_keeps_the_account_and_names_the_identity_used() {
        use crate::ops::test_util::remember_sign_in;
        let s = state();
        let home = remember_sign_in(&s, "home-oid", "tenant-home", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        // The same person's guest sign-in (another object id, the same user principal).
        let guest = SignIn {
            user_principal: "HOME-OID@x.io".into(),
            ..si("guest-oid", "tenant-guest")
        };
        let (view, activity) = record_sign_in(&s, Some("tenant-guest"), &guest).unwrap();
        assert_eq!(view.account.id, "home-oid");
        assert_eq!(
            view.tenant_sign_in,
            Some(TenantSignIn {
                tenant_id: "tenant-guest".into(),
                user_principal: "HOME-OID@x.io".into(),
                same_account: true
            })
        );
        assert_eq!(
            activity,
            "Signed in to tenant tenant-guest as HOME-OID@x.io"
        );
        assert_eq!(s.current_sign_in().unwrap().unwrap().id, "home-oid");
        assert!(s.store.sign_in("guest-oid").unwrap().is_some());
        // Another person in that tenant: saved, reported as another account, still not current.
        let (view, activity) =
            record_sign_in(&s, Some("tenant-guest"), &si("stranger", "tenant-guest")).unwrap();
        assert!(!view.tenant_sign_in.unwrap().same_account);
        assert_eq!(
            activity,
            "Signed in to tenant tenant-guest as stranger@x.io"
        );
        assert_eq!(s.current_sign_in().unwrap().unwrap().id, "home-oid");
    }

    #[test]
    fn no_current_account_without_a_sign_in() {
        assert!(current(&state()).unwrap().is_none());
    }

    #[tokio::test]
    async fn a_cancelled_sign_in_is_logged_as_cancelled_not_failed() {
        let s = state();
        let ev = crate::events::Recorder::default();
        let pending = tracked(
            &ev,
            "Sign-in failed",
            wait_or_cancel(&s, std::future::pending::<Result<SignIn, CommandError>>()),
            |a| format!("Signed in as {}", a.user_principal),
        );
        let (r, ()) = tokio::join!(pending, async {
            tokio::task::yield_now().await;
            cancel(&s);
        });
        assert_eq!(r.unwrap_err().kind, "cancelled");
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Sign-in cancelled");
        assert!(log[0].error.is_none());
    }

    #[tokio::test]
    async fn cancel_ends_a_pending_sign_in() {
        let s = state();
        let ev = crate::events::Recorder::default();
        // A sign-in that never finishes stands in for the browser flow.
        let pending = wait_or_cancel(&s, std::future::pending::<Result<SignIn, CommandError>>());
        let (r, ()) = tokio::join!(pending, async {
            tokio::task::yield_now().await;
            cancel(&s);
        });
        assert_eq!(r.unwrap_err().kind, "cancelled");
        drop(ev);
    }
}
