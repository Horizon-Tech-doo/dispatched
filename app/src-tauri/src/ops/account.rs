use crate::error::CommandError;
use crate::events::{Events, SignInPrompt, tracked};
use crate::state::AppState;
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

pub async fn sign_in(
    state: &AppState,
    ev: &dyn Events,
    tenant: Option<String>,
    device_code: bool,
) -> Result<AccountView, CommandError> {
    let tenant = tenant
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "organizations".into());
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
            state.store.save_sign_in(&s)?;
            state.store.set_current_sign_in(Some(&s.id))?;
            Ok(AccountView::from(&s))
        },
        |a| format!("Signed in as {}", a.user_principal),
    )
    .await
}

pub fn cancel(state: &AppState) {
    state.sign_in_cancel.notify_waiters();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_util::state;

    #[test]
    fn no_current_account_without_a_sign_in() {
        assert!(current(&state()).unwrap().is_none());
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
