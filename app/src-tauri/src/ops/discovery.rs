//! Find the signed-in user's tenants and Service Bus namespaces, so the Entra
//! profile form can offer a pick list instead of asking for typed names.

use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::ops::tree::count;
use crate::state::AppState;
use dispatched_core::discovery::{DiscoveredNamespace, SkippedSubscription, discover_namespaces};
use horizon_azure_kit::arm::{ArmClient, Tenant};
use horizon_azure_kit::{AzureCloud, KitError};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TenantView {
    pub tenant_id: String,
    pub display_name: Option<String>,
    pub default_domain: Option<String>,
    /// The tenant the account signed in to.
    pub home: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscoveryView {
    pub namespaces: Vec<DiscoveredNamespace>,
    pub skipped: Vec<SkippedSubscription>,
}

/// What the list shows: the home tenant first, then by display name (falling back to the domain,
/// then the id), ignoring case.
pub fn tenant_views(tenants: Vec<Tenant>, home_tenant_id: &str) -> Vec<TenantView> {
    let mut views: Vec<TenantView> = tenants
        .into_iter()
        .map(|t| TenantView {
            home: t.tenant_id.eq_ignore_ascii_case(home_tenant_id),
            tenant_id: t.tenant_id,
            display_name: t.display_name.filter(|n| !n.trim().is_empty()),
            default_domain: t.default_domain.filter(|d| !d.trim().is_empty()),
        })
        .collect();
    views.sort_by_cached_key(|t| {
        let label = t
            .display_name
            .as_deref()
            .or(t.default_domain.as_deref())
            .unwrap_or(&t.tenant_id)
            .to_lowercase();
        (!t.home, label)
    });
    views
}

/// Whether a token failure means the tenant wants an interactive sign-in (MFA, Conditional Access).
fn wants_interaction(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "aadsts50076",
        "aadsts50079",
        "aadsts53003",
        "aadsts700082",
        "interaction_required",
        "multi-factor",
    ]
    .iter()
    .any(|marker| m.contains(marker))
}

/// Map a token failure for `guest` (not the home tenant) to a command error. The kit reports
/// "this tenant needs an interactive sign-in" as `NotSignedIn`; for the home tenant that really
/// means signing in again, but for a guest tenant it means MFA or Conditional Access, which this
/// needs its own interactive sign-in (`sign_in` with that tenant).
pub fn token_error(e: KitError, guest: bool) -> CommandError {
    let interactive = guest
        && match &e {
            // The token endpoint's interactive-sign-in answers carry "sign in again (code: ...)";
            // "signed out" and "different client" are about the account, not the tenant.
            KitError::NotSignedIn(m) => m.contains("sign in again (") || wants_interaction(m),
            KitError::AuthFailed(m) => wants_interaction(m),
            _ => false,
        };
    if interactive {
        CommandError::new(
            "mfa_required",
            "This tenant requires multi-factor sign-in. Sign in to it to list its namespaces.",
        )
    } else {
        e.into()
    }
}

fn signed_in(state: &AppState) -> Result<horizon_azure_kit::auth::SignIn, CommandError> {
    state.current_sign_in()?.ok_or_else(|| {
        CommandError::new(
            "not_signed_in",
            "Sign in with Entra ID to list your namespaces.",
        )
    })
}

pub async fn list_tenants(
    state: &AppState,
    ev: &dyn Events,
) -> Result<Vec<TenantView>, CommandError> {
    let sign_in = signed_in(state)?;
    tracked(
        ev,
        "Listing tenants failed",
        async {
            let token = state
                .session
                .token_for(&sign_in, None, &AzureCloud::Public.arm_scope())
                .await?;
            let tenants = ArmClient::new(AzureCloud::Public, token.token)
                .list_tenants()
                .await?;
            Ok(tenant_views(tenants, &sign_in.tenant_id))
        },
        |t| format!("Listed {}", count(t.len(), "tenant", "tenants")),
    )
    .await
}

/// Namespaces in one tenant (`None` is the home tenant).
pub async fn discover(
    state: &AppState,
    ev: &dyn Events,
    tenant: Option<String>,
) -> Result<DiscoveryView, CommandError> {
    let sign_in = signed_in(state)?;
    let tenant = tenant
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(&sign_in.tenant_id));
    let label = tenant_label(tenant.as_deref());
    tracked(
        ev,
        &format!("Finding namespaces in {label} failed"),
        async {
            let token = state
                .session
                .token_for(&sign_in, tenant.as_deref(), &AzureCloud::Public.arm_scope())
                .await
                .map_err(|e| token_error(e, tenant.is_some()))?;
            let found =
                discover_namespaces(&ArmClient::new(AzureCloud::Public, token.token)).await?;
            Ok(DiscoveryView {
                namespaces: found.namespaces,
                skipped: found.skipped,
            })
        },
        |d| {
            format!(
                "Found {} in {label}",
                count(d.namespaces.len(), "namespace", "namespaces")
            )
        },
    )
    .await
}

/// How the activity log names a tenant: the id, since a name needs a second lookup.
fn tenant_label(tenant: Option<&str>) -> String {
    match tenant {
        None => "the home tenant".into(),
        Some(id) => format!("tenant {id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::state;

    fn tenant(id: &str, name: Option<&str>, domain: Option<&str>) -> Tenant {
        Tenant {
            tenant_id: id.into(),
            display_name: name.map(Into::into),
            default_domain: domain.map(Into::into),
        }
    }

    #[test]
    fn the_home_tenant_comes_first_then_names_without_regard_to_case() {
        let views = tenant_views(
            vec![
                tenant("t-zeta", Some("Zeta"), None),
                tenant(
                    "t-home",
                    Some("Horizon Tech"),
                    Some("horizon.onmicrosoft.com"),
                ),
                tenant("t-alpha", Some("alpha labs"), None),
                tenant("t-bare", None, Some("bare.example")),
            ],
            "T-HOME",
        );
        let ids: Vec<_> = views.iter().map(|t| t.tenant_id.as_str()).collect();
        assert_eq!(ids, ["t-home", "t-alpha", "t-bare", "t-zeta"]);
        assert!(views[0].home);
        assert!(views[1..].iter().all(|t| !t.home));
    }

    #[test]
    fn a_tenant_without_a_name_sorts_by_domain_then_id_and_blank_names_are_dropped() {
        let views = tenant_views(
            vec![
                tenant("b-id", Some("  "), None),
                tenant("a-id", None, None),
                tenant("x", Some("Mid"), None),
            ],
            "nobody",
        );
        let ids: Vec<_> = views.iter().map(|t| t.tenant_id.as_str()).collect();
        assert_eq!(ids, ["a-id", "b-id", "x"]);
        assert_eq!(views[1].display_name, None);
    }

    #[test]
    fn a_guest_tenant_that_needs_interaction_is_mfa_required() {
        let e = token_error(
            KitError::NotSignedIn(
                "a@x.io: sign in again (interaction_required: AADSTS50076)".into(),
            ),
            true,
        );
        assert_eq!(e.kind, "mfa_required");
        assert!(e.message.contains("multi-factor"));
        let e = token_error(
            KitError::AuthFailed("invalid_request: AADSTS53003 blocked".into()),
            true,
        );
        assert_eq!(e.kind, "mfa_required");
    }

    #[test]
    fn the_same_failure_for_the_home_tenant_stays_a_sign_in_problem() {
        let e = token_error(
            KitError::NotSignedIn("a@x.io: sign in again (invalid_grant: AADSTS50173)".into()),
            false,
        );
        assert_eq!(e.kind, "not_signed_in");
        let e = token_error(KitError::NotSignedIn("a@x.io signed out".into()), true);
        assert_eq!(e.kind, "not_signed_in");
    }

    #[test]
    fn other_guest_failures_keep_their_own_kind() {
        let e = token_error(KitError::AuthFailed("invalid_client: bad".into()), true);
        assert_eq!(e.kind, "auth");
        let e = token_error(KitError::Network("dns".into()), true);
        assert_eq!(e.kind, "network");
        // The raw token error is not echoed for the interactive case.
        let e = token_error(KitError::NotSignedIn("AADSTS50076 trace abc".into()), true);
        assert!(!e.message.contains("abc"));
    }

    #[tokio::test]
    async fn listing_needs_a_sign_in_and_logs_the_failure() {
        let s = state();
        let ev = Recorder::default();
        assert_eq!(
            list_tenants(&s, &ev).await.unwrap_err().kind,
            "not_signed_in"
        );
        assert_eq!(
            discover(&s, &ev, None).await.unwrap_err().kind,
            "not_signed_in"
        );
        // Nothing was attempted, so nothing is logged.
        assert!(ev.activity.lock().unwrap().is_empty());
    }

    #[test]
    fn tenants_and_namespaces_get_real_plurals() {
        assert_eq!(count(1, "tenant", "tenants"), "1 tenant");
        assert_eq!(count(2, "tenant", "tenants"), "2 tenants");
        assert_eq!(count(0, "namespace", "namespaces"), "0 namespaces");
        assert_eq!(tenant_label(None), "the home tenant");
        assert_eq!(tenant_label(Some("abc")), "tenant abc");
    }
}
