use crate::error::CommandError;
use crate::events::{Events, SignInPrompt, tracked};
use crate::state::{AppState, same_identity};
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;
use std::future::Future;
use std::sync::Arc;

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
    // The prompt's URL is kept for "Open sign-in page" only while this sign-in waits.
    let prompt = PromptUrl::new(state);
    tracked(
        ev,
        "Sign-in failed",
        async {
            let session = state.session.clone();
            let login = async {
                let r = if device_code {
                    session
                        .login_device_code(&tenant, |dc| {
                            prompt.remember(&dc.verification_uri);
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
                            prompt.remember(url);
                            ev.sign_in_prompt(SignInPrompt {
                                kind: "browser",
                                url: url.to_string(),
                                user_code: None,
                                message: "Finish signing in in your browser.".into(),
                            });
                            // The same `https` check as `open_sign_in_page`: the library's URL is not trusted blindly.
                            if let Some(page) = browser_target(url) {
                                let _ = crate::workspace::open_url(page.as_str());
                            }
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

/// A browser page the app may open for a sign-in: an `https` URL only.
fn https_url(url: &str) -> Option<url::Url> {
    url::Url::parse(url).ok().filter(|u| u.scheme() == "https")
}

/// What the browser sign-in's callback may open: the URL when it is `https`, else nothing.
fn browser_target(url: &str) -> Option<String> {
    https_url(url).map(|u| u.to_string())
}

/// The prompt URL of one running `sign_in`, kept in `AppState::last_prompt_url` for "Open sign-in
/// page" while it waits. When the sign-in returns (success, failure or cancel) the URL is cleared,
/// but only if it is still this sign-in's: a newer sign-in's prompt stays.
pub(crate) struct PromptUrl<'a> {
    state: &'a AppState,
    mine: std::sync::Mutex<Option<Arc<str>>>,
}

impl<'a> PromptUrl<'a> {
    pub(crate) fn new(state: &'a AppState) -> Self {
        Self {
            state,
            mine: std::sync::Mutex::new(None),
        }
    }

    /// Keep `url` as the waiting prompt's page. Anything but an `https` URL is ignored.
    pub(crate) fn remember(&self, url: &str) {
        if https_url(url).is_none() {
            return;
        }
        let url: Arc<str> = Arc::from(url);
        *self.mine.lock().expect("prompt url poisoned") = Some(url.clone());
        *self
            .state
            .last_prompt_url
            .lock()
            .expect("prompt url poisoned") = Some(url);
    }
}

impl Drop for PromptUrl<'_> {
    fn drop(&mut self) {
        let Some(mine) = self.mine.get_mut().ok().and_then(Option::take) else {
            return;
        };
        if let Ok(mut current) = self.state.last_prompt_url.lock()
            && current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &mine))
        {
            *current = None;
        }
    }
}

/// Open the page of the sign-in that is waiting (device code), in the default browser. Only an
/// `https` page is opened.
pub fn open_sign_in_page(state: &AppState) -> Result<(), CommandError> {
    let url = state
        .last_prompt_url
        .lock()
        .expect("prompt url poisoned")
        .clone()
        .ok_or_else(|| CommandError::new("not_found", "No sign-in is waiting."))?;
    let url = https_url(&url).ok_or_else(|| {
        CommandError::new(
            "invalid",
            "The sign-in page is not an https address, so it was not opened.",
        )
    })?;
    crate::workspace::open_url(url.as_str())
        .map_err(|e| CommandError::new("io", format!("Couldn't open the browser: {e}")))
}

/// One remembered sign-in, for the Settings list.
#[derive(Debug, Serialize)]
pub struct StoredSignIn {
    pub id: String,
    pub user_principal: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
    /// It is the current account's sign-in.
    pub current: bool,
    /// Its refresh token exists.
    pub signed_in: bool,
    /// It is the current account's identity (same object id or user principal); another
    /// account's kept tenant sign-in is not.
    pub same_account: bool,
    /// Names of the profiles that connect with this sign-in now (`bound_sign_in`), which may be
    /// the same person's tenant sign-in rather than the one the profile was saved with.
    pub profiles: Vec<String>,
}

/// Every remembered sign-in, with whose it is and which profiles connect with it.
pub fn sign_ins(state: &AppState) -> Result<Vec<StoredSignIn>, CommandError> {
    let current = state.current_sign_in()?;
    let mut users: Vec<(String, String)> = Vec::new();
    for p in state.store.list()? {
        if let Some(b) = crate::ops::profiles::bound_sign_in(state, &p)? {
            users.push((b.id, p.name));
        }
    }
    state
        .store
        .sign_ins()?
        .into_iter()
        .map(|s| {
            let profiles = users
                .iter()
                .filter(|(id, _)| *id == s.id)
                .map(|(_, name)| name.clone())
                .collect();
            Ok(StoredSignIn {
                signed_in: state.session.is_signed_in(&s)?,
                current: current.as_ref().is_some_and(|c| c.id == s.id),
                same_account: current.as_ref().is_some_and(|c| same_identity(c, &s)),
                id: s.id,
                user_principal: s.user_principal,
                display_name: s.display_name,
                tenant_id: s.tenant_id,
                profiles,
            })
        })
        .collect()
}

/// What signing out changed beyond the sign-in itself.
#[derive(Debug, Serialize)]
pub struct SignedOut {
    /// The connection that used the sign-in was closed: its profile id. The UI refreshes it.
    pub closed_profile_id: Option<String>,
}

/// Forget a sign-in: its refresh token and its record. Signing out the current one leaves no
/// current account. The active connection is closed (cancelling its jobs) when its profile is
/// bound to this sign-in or connects with it now.
pub async fn sign_out(
    state: &AppState,
    ev: &dyn Events,
    id: &str,
) -> Result<SignedOut, CommandError> {
    let s = state
        .store
        .sign_in(id)?
        .ok_or_else(|| CommandError::new("not_found", "That sign-in is already gone."))?;
    let who = s.user_principal.clone();
    tracked(
        ev,
        "Signing out failed",
        async {
            // No connect may start or finish while the sign-in goes away.
            let _serial = state.connect_lock.lock().await;
            // Decided before logging out: afterwards the sign-in no longer counts as usable.
            let closing = match state.active.read().await.as_ref() {
                Some(a) => match state.store.get_by_id(&a.profile_id)? {
                    Some(p) => uses_sign_in(state, &p, &s.id)?.then_some(p),
                    None => None,
                },
                None => None,
            };
            state.session.logout(&s)?;
            state.store.remove_sign_in(&s.id)?;
            if state.store.current_sign_in()?.is_none_or(|c| c.id == s.id) {
                state.store.set_current_sign_in(None)?;
            }
            let Some(p) = closing else {
                return Ok((
                    SignedOut {
                        closed_profile_id: None,
                    },
                    None,
                ));
            };
            let old = {
                let mut active = state.active.write().await;
                if active.as_ref().is_some_and(|a| a.profile_id == p.id) {
                    active.take()
                } else {
                    None
                }
            };
            if let Some(old) = old {
                state.close(old).await;
            }
            Ok((
                SignedOut {
                    closed_profile_id: Some(p.id),
                },
                Some(p.name),
            ))
        },
        |(_, closed)| match closed {
            Some(name) => format!("Signed out {who} and closed {name}"),
            None => format!("Signed out {who}"),
        },
    )
    .await
    .map(|(out, _)| out)
}

/// Whether profile `p` is bound to sign-in `id` or connects with it now (`usable_sign_in`).
fn uses_sign_in(
    state: &AppState,
    p: &dispatched_core::profiles::Profile,
    id: &str,
) -> Result<bool, CommandError> {
    let dispatched_core::profiles::ProfileAuth::Entra { sign_in_id, tenant } = &p.auth else {
        return Ok(false);
    };
    Ok(sign_in_id == id
        || state
            .usable_sign_in(sign_in_id, tenant.as_deref())?
            .1
            .is_some_and(|u| u.id == id))
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
    fn sign_ins_list_marks_current_signed_in_and_using_profiles() {
        let s = state();
        let home = crate::ops::test_util::remember_sign_in(&s, "oid-1", "tenant-1", true);
        crate::ops::test_util::remember_sign_in(&s, "oid-g", "tenant-guest", false);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        let mut p = dispatched_core::profiles::Profile::entra(
            "Guest NS",
            "g.servicebus.windows.net",
            "oid-g",
            Some("tenant-guest"),
        );
        p.pinned_entities.clear();
        s.store.insert(&p).unwrap();
        let list = sign_ins(&s).unwrap();
        assert_eq!(list.len(), 2);
        let h = list.iter().find(|v| v.id == "oid-1").unwrap();
        assert!(h.current && h.signed_in && h.same_account);
        let g = list.iter().find(|v| v.id == "oid-g").unwrap();
        assert!(!g.current && !g.signed_in);
        assert!(
            !g.same_account,
            "another account's kept sign-in is not the user's own"
        );
        assert_eq!(g.profiles, ["Guest NS"]);
    }

    #[test]
    fn a_profile_is_listed_under_the_sign_in_it_connects_with() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let home = remember_sign_in(&s, "oid-1", "tenant-1", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_guest_sign_in(&s, &home, "oid-1g", "tenant-guest", true);
        // Saved with the home sign-in, asking for the guest tenant: connects with the same person's
        // own sign-in there (bound_sign_in), so it is listed under that row.
        let p = dispatched_core::profiles::Profile::entra(
            "Guest NS",
            "g.servicebus.windows.net",
            "oid-1",
            Some("tenant-guest"),
        );
        s.store.insert(&p).unwrap();
        let list = sign_ins(&s).unwrap();
        let h = list.iter().find(|v| v.id == "oid-1").unwrap();
        let g = list.iter().find(|v| v.id == "oid-1g").unwrap();
        assert!(h.profiles.is_empty(), "{:?}", h.profiles);
        assert_eq!(g.profiles, ["Guest NS"]);
        assert!(g.same_account && !g.current);
    }

    #[tokio::test]
    async fn sign_out_forgets_the_sign_in_and_clears_current() {
        let s = state();
        let ev = crate::events::Recorder::default();
        let home = crate::ops::test_util::remember_sign_in(&s, "oid-1", "tenant-1", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        sign_out(&s, &ev, "oid-1").await.unwrap();
        assert!(s.store.sign_in("oid-1").unwrap().is_none());
        assert!(s.current_sign_in().unwrap().is_none());
        assert_eq!(
            sign_out(&s, &ev, "oid-1").await.unwrap_err().kind,
            "not_found"
        );
    }

    const PROMPT: &str = "https://microsoft.com/devicelogin";

    fn prompt_url(s: &AppState) -> Option<String> {
        s.last_prompt_url
            .lock()
            .unwrap()
            .as_deref()
            .map(str::to_string)
    }

    #[test]
    fn a_sign_in_s_prompt_url_is_kept_while_it_waits_and_cleared_after() {
        let s = state();
        {
            let waiting = PromptUrl::new(&s);
            waiting.remember(PROMPT);
            assert_eq!(prompt_url(&s).as_deref(), Some(PROMPT));
        }
        assert_eq!(prompt_url(&s), None);
    }

    #[test]
    fn an_older_sign_in_ending_leaves_a_newer_one_s_prompt_url() {
        let s = state();
        let first = PromptUrl::new(&s);
        first.remember(PROMPT);
        // The device-code page is the same URL every time: the newer sign-in's is still its own.
        let second = PromptUrl::new(&s);
        second.remember(PROMPT);
        drop(first);
        assert_eq!(prompt_url(&s).as_deref(), Some(PROMPT));
        drop(second);
        assert_eq!(prompt_url(&s), None);
    }

    #[test]
    fn the_browser_callback_opens_only_https_pages() {
        assert_eq!(
            browser_target("https://login.microsoftonline.com/x?a=1").as_deref(),
            Some("https://login.microsoftonline.com/x?a=1")
        );
        for bad in [
            "http://login.example",
            "file:///Applications/Calculator.app",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert_eq!(browser_target(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_https_prompt_urls_are_kept_or_opened() {
        let s = state();
        let waiting = PromptUrl::new(&s);
        for bad in [
            "http://login.example",
            "file:///Applications/Calculator.app",
            "javascript:alert(1)",
            "not a url",
            "/Applications/Calculator.app",
        ] {
            waiting.remember(bad);
            assert_eq!(prompt_url(&s), None, "{bad}");
        }
        *s.last_prompt_url.lock().unwrap() = Some(std::sync::Arc::from("file:///etc/hosts"));
        assert_eq!(open_sign_in_page(&s).unwrap_err().kind, "invalid");
    }

    #[tokio::test]
    async fn signing_out_closes_the_connection_that_uses_the_sign_in() {
        use crate::ops::test_util::{remember_guest_sign_in, remember_sign_in};
        let s = state();
        let ev = crate::events::Recorder::default();
        let home = remember_sign_in(&s, "oid-1", "tenant-1", true);
        s.store.set_current_sign_in(Some(&home.id)).unwrap();
        remember_guest_sign_in(&s, &home, "oid-1g", "tenant-guest", true);
        remember_sign_in(&s, "oid-x", "tenant-x", true);
        // Saved with the home sign-in, asking for the guest tenant: connects with the guest sign-in.
        let p = dispatched_core::profiles::Profile::entra(
            "Guest NS",
            "g.servicebus.windows.net",
            "oid-1",
            Some("tenant-guest"),
        );
        s.store.insert(&p).unwrap();
        crate::ops::connection::connect(&s, &ev, &p.id)
            .await
            .unwrap();

        // Another sign-in: the connection stays.
        let out = sign_out(&s, &ev, "oid-x").await.unwrap();
        assert_eq!(out.closed_profile_id, None);
        assert_eq!(s.active().await.unwrap().profile_id, p.id);

        // The sign-in it connects with: closed, and the result names the profile.
        let out = sign_out(&s, &ev, "oid-1g").await.unwrap();
        assert_eq!(out.closed_profile_id.as_deref(), Some(p.id.as_str()));
        assert_eq!(s.active().await.unwrap_err().kind, "not_connected");

        // Now it connects with the sign-in it was saved with; signing that out closes it too.
        crate::ops::connection::connect(&s, &ev, &p.id)
            .await
            .unwrap();
        let out = sign_out(&s, &ev, "oid-1").await.unwrap();
        assert_eq!(out.closed_profile_id.as_deref(), Some(p.id.as_str()));
        assert_eq!(s.active().await.unwrap_err().kind, "not_connected");
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Signed out oid-x@x.io\""), "{log}");
        assert!(
            log.contains("Signed out oid-1@x.io and closed Guest NS"),
            "{log}"
        );
    }

    #[test]
    fn open_sign_in_page_needs_a_waiting_prompt() {
        let s = state();
        assert_eq!(open_sign_in_page(&s).unwrap_err().kind, "not_found");
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
