//! Find Service Bus namespaces visible to an Entra sign-in (via ARM).

use crate::Result;
use horizon_azure_kit::KitError;
use horizon_azure_kit::arm::{ArmClient, Subscription};
use serde::{Deserialize, Serialize};

/// ARM api-version for `Microsoft.ServiceBus/namespaces`.
pub const NAMESPACES_API: &str = "2024-01-01";

/// A namespace found in a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredNamespace {
    /// Namespace name.
    pub name: String,
    /// Host, e.g. `myns.servicebus.windows.net`.
    pub host: String,
    /// ARM resource id.
    pub resource_id: String,
    /// Subscription id.
    pub subscription_id: String,
    /// Subscription display name.
    pub subscription_name: String,
    /// Tenant that owns the subscription.
    pub tenant_id: Option<String>,
    /// Azure region.
    pub location: Option<String>,
    /// `Basic`, `Standard` or `Premium`.
    pub tier: Option<String>,
    /// SAS (local auth) disabled: Entra only.
    pub local_auth_disabled: bool,
}

#[derive(Deserialize)]
struct RawNamespace {
    id: String,
    name: String,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    sku: Option<RawSku>,
    #[serde(default)]
    properties: RawProperties,
}

#[derive(Deserialize)]
struct RawSku {
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawProperties {
    #[serde(default)]
    service_bus_endpoint: Option<String>,
    #[serde(default)]
    disable_local_auth: Option<bool>,
}

fn host_from(endpoint: Option<&str>, name: &str) -> String {
    endpoint
        .and_then(|e| url::Url::parse(e).ok())
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| format!("{}.servicebus.windows.net", name.to_ascii_lowercase()))
}

fn map(sub: &Subscription, raw: RawNamespace) -> DiscoveredNamespace {
    DiscoveredNamespace {
        host: host_from(raw.properties.service_bus_endpoint.as_deref(), &raw.name),
        name: raw.name,
        resource_id: raw.id,
        subscription_id: sub.subscription_id.clone(),
        subscription_name: sub.display_name.clone(),
        tenant_id: sub.tenant_id.clone(),
        location: raw.location,
        tier: raw.sku.and_then(|s| s.tier),
        local_auth_disabled: raw.properties.disable_local_auth.unwrap_or(false),
    }
}

/// A subscription that could not be listed (for example no access).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedSubscription {
    /// Subscription id.
    pub subscription_id: String,
    /// Subscription display name.
    pub subscription_name: String,
    /// Why it was skipped.
    pub reason: String,
}

/// Result of a discovery run: what was found and what was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Discovery {
    /// Namespaces found, sorted by name.
    pub namespaces: Vec<DiscoveredNamespace>,
    /// Subscriptions that failed to list.
    pub skipped: Vec<SkippedSubscription>,
}

fn is_usable(state: Option<&str>) -> bool {
    match state {
        None => true,
        Some(s) => ["enabled", "warned", "pastdue"]
            .iter()
            .any(|u| s.eq_ignore_ascii_case(u)),
    }
}

/// All namespaces in all usable subscriptions (state absent, Enabled, Warned
/// or PastDue). A subscription that fails to list is recorded in `skipped`;
/// only when every usable subscription failed is the first error returned.
pub async fn discover_namespaces(arm: &ArmClient) -> Result<Discovery> {
    let mut namespaces = Vec::new();
    let mut skipped = Vec::new();
    let mut first_error: Option<KitError> = None;
    let mut usable = 0usize;
    let mut succeeded = 0usize;
    for sub in arm.list_subscriptions().await? {
        if !is_usable(sub.state.as_deref()) {
            continue;
        }
        usable += 1;
        match arm
            .list_by_provider::<RawNamespace>(
                &sub.subscription_id,
                "Microsoft.ServiceBus/namespaces",
                NAMESPACES_API,
            )
            .await
        {
            Ok(list) => {
                succeeded += 1;
                namespaces.extend(list.into_iter().map(|raw| map(&sub, raw)));
            }
            Err(e) => {
                skipped.push(SkippedSubscription {
                    subscription_id: sub.subscription_id.clone(),
                    subscription_name: sub.display_name.clone(),
                    reason: e.to_string(),
                });
                first_error.get_or_insert(e);
            }
        }
    }
    if usable > 0
        && succeeded == 0
        && let Some(e) = first_error
    {
        return Err(e.into());
    }
    namespaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Discovery {
        namespaces,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;
    use secrecy::SecretString;

    const NS_BODY: &str = r#"{"value":[{"id":"/subscriptions/s1/resourceGroups/rg/providers/Microsoft.ServiceBus/namespaces/SBPremium","name":"SBPremium","location":"West Europe","sku":{"name":"Premium","tier":"Premium"},"properties":{"serviceBusEndpoint":"https://SBPremium.servicebus.windows.net:443/","disableLocalAuth":true}}]}"#;

    async fn subs_mock(server: &mut mockito::ServerGuard, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/subscriptions")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(body)
            .create_async()
            .await
    }

    async fn ns_mock(
        server: &mut mockito::ServerGuard,
        sub: &str,
        status: usize,
        body: &str,
        expect: usize,
    ) -> mockito::Mock {
        server
            .mock(
                "GET",
                format!("/subscriptions/{sub}/providers/Microsoft.ServiceBus/namespaces").as_str(),
            )
            .match_query(Matcher::Any)
            .with_status(status)
            .with_body(body)
            .expect(expect)
            .create_async()
            .await
    }

    fn arm(server: &mockito::ServerGuard) -> ArmClient {
        ArmClient::with_base(server.url(), SecretString::from("tok"))
    }

    #[tokio::test]
    async fn discovers_namespaces_and_records_forbidden_subscriptions() {
        let mut server = mockito::Server::new_async().await;
        let _subs = subs_mock(&mut server, r#"{"value":[{"subscriptionId":"s1","displayName":"Prod","tenantId":"t1"},{"subscriptionId":"s2","displayName":"Locked"}]}"#).await;
        let _ns = ns_mock(&mut server, "s1", 200, NS_BODY, 1).await;
        let _denied = ns_mock(&mut server, "s2", 403, "AuthorizationFailed", 1).await;
        let found = discover_namespaces(&arm(&server)).await.unwrap();
        assert_eq!(found.namespaces.len(), 1);
        let ns = &found.namespaces[0];
        assert_eq!(ns.host, "sbpremium.servicebus.windows.net");
        assert_eq!(ns.tier.as_deref(), Some("Premium"));
        assert!(ns.local_auth_disabled);
        assert_eq!(ns.subscription_name, "Prod");
        assert_eq!(ns.tenant_id.as_deref(), Some("t1"));
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.skipped[0].subscription_id, "s2");
        assert_eq!(found.skipped[0].subscription_name, "Locked");
        assert!(
            found.skipped[0].reason.contains("forbidden"),
            "{}",
            found.skipped[0].reason
        );
    }

    #[tokio::test]
    async fn other_failures_in_one_subscription_do_not_abort() {
        let mut server = mockito::Server::new_async().await;
        let _subs = subs_mock(&mut server, r#"{"value":[{"subscriptionId":"s1","displayName":"Prod"},{"subscriptionId":"s2","displayName":"Flaky"},{"subscriptionId":"s3","displayName":"Conflict"}]}"#).await;
        let _ns = ns_mock(&mut server, "s1", 200, NS_BODY, 1).await;
        let _boom = ns_mock(&mut server, "s2", 500, "boom", 1).await;
        let _conflict = ns_mock(&mut server, "s3", 409, "conflict", 1).await;
        let found = discover_namespaces(&arm(&server)).await.unwrap();
        assert_eq!(found.namespaces.len(), 1);
        assert_eq!(found.skipped.len(), 2);
    }

    #[tokio::test]
    async fn all_subscriptions_failing_is_an_error() {
        let mut server = mockito::Server::new_async().await;
        let _subs = subs_mock(&mut server, r#"{"value":[{"subscriptionId":"s1","displayName":"A"},{"subscriptionId":"s2","displayName":"B"}]}"#).await;
        let _a = ns_mock(&mut server, "s1", 500, "boom", 1).await;
        let _b = ns_mock(&mut server, "s2", 403, "denied", 1).await;
        assert!(discover_namespaces(&arm(&server)).await.is_err());
    }

    #[tokio::test]
    async fn unusable_subscriptions_are_not_queried() {
        let mut server = mockito::Server::new_async().await;
        let _subs = subs_mock(
            &mut server,
            r#"{"value":[{"subscriptionId":"s1","displayName":"Prod","state":"Enabled"},{"subscriptionId":"s2","displayName":"Gone","state":"Deleted"},{"subscriptionId":"s3","displayName":"Off","state":"disabled"}]}"#,
        )
        .await;
        let _ns = ns_mock(&mut server, "s1", 200, NS_BODY, 1).await;
        let gone = ns_mock(&mut server, "s2", 200, r#"{"value":[]}"#, 0).await;
        let off = ns_mock(&mut server, "s3", 200, r#"{"value":[]}"#, 0).await;
        let found = discover_namespaces(&arm(&server)).await.unwrap();
        assert_eq!(found.namespaces.len(), 1);
        assert!(found.skipped.is_empty());
        gone.assert_async().await;
        off.assert_async().await;
    }
}
