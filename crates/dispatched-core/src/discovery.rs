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
        location: raw.location,
        tier: raw.sku.and_then(|s| s.tier),
        local_auth_disabled: raw.properties.disable_local_auth.unwrap_or(false),
    }
}

/// All namespaces in all enabled subscriptions the token can read.
/// Subscriptions that deny access are skipped.
pub async fn discover_namespaces(arm: &ArmClient) -> Result<Vec<DiscoveredNamespace>> {
    let mut out = Vec::new();
    for sub in arm.list_subscriptions().await? {
        if sub.state.as_deref() == Some("Disabled") {
            continue;
        }
        match arm
            .list_by_provider::<RawNamespace>(
                &sub.subscription_id,
                "Microsoft.ServiceBus/namespaces",
                NAMESPACES_API,
            )
            .await
        {
            Ok(list) => out.extend(list.into_iter().map(|raw| map(&sub, raw))),
            Err(KitError::Forbidden(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;
    use secrecy::SecretString;

    #[tokio::test]
    async fn discovers_namespaces_and_skips_forbidden_subscriptions() {
        let mut server = mockito::Server::new_async().await;
        let _subs = server
            .mock("GET", "/subscriptions")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"value":[{"subscriptionId":"s1","displayName":"Prod"},{"subscriptionId":"s2","displayName":"Locked"}]}"#)
            .create_async()
            .await;
        let _ns = server
            .mock("GET", "/subscriptions/s1/providers/Microsoft.ServiceBus/namespaces")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"value":[{"id":"/subscriptions/s1/resourceGroups/rg/providers/Microsoft.ServiceBus/namespaces/SBPremium","name":"SBPremium","location":"West Europe","sku":{"name":"Premium","tier":"Premium"},"properties":{"serviceBusEndpoint":"https://SBPremium.servicebus.windows.net:443/","disableLocalAuth":true}}]}"#)
            .create_async()
            .await;
        let _denied = server
            .mock(
                "GET",
                "/subscriptions/s2/providers/Microsoft.ServiceBus/namespaces",
            )
            .match_query(Matcher::Any)
            .with_status(403)
            .with_body("AuthorizationFailed")
            .create_async()
            .await;
        let arm = ArmClient::with_base(server.url(), SecretString::from("tok"));
        let found = discover_namespaces(&arm).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].host, "sbpremium.servicebus.windows.net");
        assert_eq!(found[0].tier.as_deref(), Some("Premium"));
        assert!(found[0].local_auth_disabled);
        assert_eq!(found[0].subscription_name, "Prod");
    }
}
