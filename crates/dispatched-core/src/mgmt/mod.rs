//! Service Bus management API (ATOM over HTTPS; plain HTTP on the emulator).

mod details;
mod models;
mod rules;
pub(crate) mod xml;

pub use details::{EntityDetails, EntityKind, EntityProperty};
pub use models::{EntityCounts, QueueInfo, SubscriptionInfo, TopicInfo};
pub use rules::{CorrelationProperty, RuleFilter, RuleInfo};

use crate::namespace::Namespace;
use crate::tokens::TokenSource;
use crate::{Error, Result};
use models::{entries, queue_from_entry, subscription_from_entry, topic_from_entry};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use xml::Node;

const API_VERSION: &str = "2024-05";
const PAGE_SIZE: usize = 100;
/// Hard cap on pages per listing, so a misbehaving server cannot loop forever.
const MAX_PAGES: usize = 1000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Path segment encoding: everything except unreserved characters.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn segment(name: &str) -> String {
    utf8_percent_encode(name, SEGMENT).to_string()
}

/// Read-side management client (M0). Create/update/delete arrive in M3.
#[derive(Clone)]
pub struct ManagementClient {
    http: reqwest::Client,
    base: String,
    tokens: Arc<dyn TokenSource>,
}

impl ManagementClient {
    /// Client for `ns`.
    ///
    /// Fails if the proxy configured on `ns` is unusable (`Error::Invalid`)
    /// or the HTTP client cannot be initialised (`Error::Network`).
    pub fn new(ns: &Namespace) -> Result<Self> {
        let proxy = match ns.proxy() {
            Some(p) => Some(
                reqwest::Proxy::all(p.url_for_reqwest())
                    .map_err(|e| Error::Invalid(format!("proxy: {}", e.without_url())))?,
            ),
            None => None,
        };
        Self::build(ns.management_base(), ns.tokens().clone(), proxy)
    }

    /// Client for an explicit base URL (tests; panics if the HTTP client cannot be built).
    pub fn with_base(base: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self::build(base.into(), tokens, None).expect("reqwest client builds")
    }

    fn build(
        base: String,
        tokens: Arc<dyn TokenSource>,
        proxy: Option<reqwest::Proxy>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT);
        if let Some(proxy) = proxy {
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|e| {
            Error::Network(format!(
                "cannot initialise the HTTP client: {}",
                e.without_url()
            ))
        })?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            tokens,
        })
    }

    /// All queues with counts.
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        self.list("$Resources/queues", queue_from_entry).await
    }

    /// All topics.
    pub async fn list_topics(&self) -> Result<Vec<TopicInfo>> {
        self.list("$Resources/topics", topic_from_entry).await
    }

    /// Subscriptions of `topic` with counts.
    pub async fn list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>> {
        self.list(&format!("{}/Subscriptions", segment(topic)), |e| {
            subscription_from_entry(topic, e)
        })
        .await
    }

    /// One queue. A missing queue is `NotFound` (the service answers 200 + empty feed).
    pub async fn get_queue(&self, name: &str) -> Result<QueueInfo> {
        let root = self.get(&segment(name), &[("enrich", "true")]).await?;
        entries(&root)
            .into_iter()
            .find_map(queue_from_entry)
            .ok_or_else(|| Error::NotFound(format!("queue `{name}`")))
    }

    /// One entity's description. `path` is a queue or topic name, or `topic/subscriptions/name`.
    /// A missing entity, or one of another kind, is `NotFound`.
    pub async fn entity_details(&self, kind: EntityKind, path: &str) -> Result<EntityDetails> {
        let (url_path, topic) = match kind {
            EntityKind::Queue | EntityKind::Topic => (segment(path), None),
            EntityKind::Subscription => {
                let e = crate::EntityPath::parse(path)?;
                let sub = e.subscription.clone().ok_or_else(|| {
                    Error::Invalid(format!(
                        "`{path}` is not a subscription; use `topic/subscriptions/name`"
                    ))
                })?;
                (
                    format!("{}/Subscriptions/{}", segment(&e.entity), segment(&sub)),
                    Some(e.entity),
                )
            }
        };
        let root = self.get(&url_path, &[("enrich", "true")]).await?;
        entries(&root)
            .into_iter()
            .find_map(|e| details::details_from_entry(kind, topic.as_deref(), e))
            .ok_or_else(|| Error::NotFound(format!("{} `{path}`", kind.as_str())))
    }

    /// The rules of `topic`'s subscription `subscription`.
    pub async fn list_rules(&self, topic: &str, subscription: &str) -> Result<Vec<RuleInfo>> {
        self.list(
            &format!(
                "{}/Subscriptions/{}/Rules",
                segment(topic),
                segment(subscription)
            ),
            rules::rule_from_entry,
        )
        .await
    }

    /// Page through a feed. Stops on a short page, on a page with no names
    /// not already seen (a server ignoring `$skip`), or fails after `MAX_PAGES`.
    async fn list<T>(&self, path: &str, map: impl Fn(&Node) -> Option<T>) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut skip = 0usize;
        for _ in 0..MAX_PAGES {
            let skip_s = skip.to_string();
            let top_s = PAGE_SIZE.to_string();
            let root = self
                .get(
                    path,
                    &[("enrich", "true"), ("$skip", &skip_s), ("$top", &top_s)],
                )
                .await?;
            let page = entries(&root);
            let n = page.len();
            let mut new_names = 0usize;
            for entry in page {
                let Some(name) = entry.text_of("title") else {
                    continue;
                };
                if !seen.insert(name.to_string()) {
                    continue;
                }
                new_names += 1;
                out.extend(map(entry));
            }
            if n < PAGE_SIZE || new_names == 0 {
                return Ok(out);
            }
            skip += n;
        }
        Err(Error::Protocol(
            "management API paging did not terminate".into(),
        ))
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Node> {
        let url = format!("{}/{path}", self.base);
        let token = self.tokens.token(&url.to_ascii_lowercase()).await?;
        let resp = self
            .http
            .get(&url)
            .query(&[("api-version", API_VERSION)])
            .query(query)
            .header(reqwest::header::AUTHORIZATION, token.http_authorization())
            .send()
            .await
            .map_err(|e| Error::Network(format!("management API {url}: {e}")))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;
        if (200..300).contains(&status) {
            return xml::parse(&body);
        }
        Err(map_http_error(status, &body, retry_after, &url))
    }
}

fn map_http_error(status: u16, body: &str, retry_after: Option<Duration>, url: &str) -> Error {
    let detail = xml::parse(body)
        .ok()
        .and_then(|n| n.text_of("Detail").map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(500).collect());
    match status {
        // Service Bus answers missing Manage claims with 401, so a 401 that
        // names claims is an authorization problem, not a rejected credential.
        401 => unauthorized(detail),
        403 => Error::Forbidden {
            needed: needed_claims(&detail),
            detail,
        },
        404 => Error::NotFound(url.to_string()),
        429 | 503 => Error::Throttled { retry_after },
        _ => Error::Protocol(format!("management API returned {status}: {detail}")),
    }
}

/// A 401-style answer: `Forbidden` when the detail names the missing claims,
/// otherwise the credential itself was rejected (`Auth`).
pub(crate) fn unauthorized(detail: String) -> Error {
    match needed_claims(&detail) {
        Some(needed) => Error::Forbidden {
            needed: Some(needed),
            detail,
        },
        None => Error::Auth(detail),
    }
}

/// Extract the missing rights from Service Bus error detail text. Recognises
/// the HTTP wording ("Manage,EntityRead claims required for this operation.")
/// and the AMQP wording ("'Listen' claim(s) are required to perform this
/// operation", `"Manage,EntityRead" claim(s) are required ...`).
pub fn needed_claims(detail: &str) -> Option<String> {
    for marker in [" claim(s) are required", " claims are required"] {
        if let Some(idx) = detail.find(marker) {
            let before = detail[..idx].trim_end();
            let claims = match before.chars().last() {
                Some(q @ ('\'' | '"')) => {
                    let inner = &before[..before.len() - 1];
                    inner.rfind(q).map(|i| &inner[i + 1..])
                }
                _ => before
                    .rsplit(|c: char| c.is_whitespace() || c == ':')
                    .next(),
            }
            .map(str::trim)
            .filter(|s| !s.is_empty());
            if let Some(claims) = claims {
                return Some(claims.to_string());
            }
        }
    }
    if let Some(idx) = detail.find(" claims required") {
        let before = &detail[..idx];
        let start = before.rfind([' ', ':']).map(|i| i + 1).unwrap_or(0);
        let claims = before[start..].trim();
        return (!claims.is_empty()).then(|| claims.to_string());
    }
    detail
        .split("specified action: ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{ServiceBusToken, TokenKind};
    use async_trait::async_trait;
    use mockito::Matcher;
    use secrecy::SecretString;

    struct FixedToken;
    #[async_trait]
    impl TokenSource for FixedToken {
        async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
            Ok(ServiceBusToken {
                token: SecretString::from("SharedAccessSignature sr=x&sig=y&se=9999999999&skn=k"),
                kind: TokenKind::Sas,
                expires_on: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            })
        }
        fn describe(&self) -> String {
            "fixed".into()
        }
    }

    fn client(server: &mockito::Server) -> ManagementClient {
        ManagementClient::with_base(server.url(), Arc::new(FixedToken))
    }

    #[tokio::test]
    async fn list_queues_parses_counts_and_sends_auth() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("api-version".into(), "2024-05".into()),
                Matcher::UrlEncoded("$top".into(), "100".into()),
                Matcher::UrlEncoded("enrich".into(), "true".into()),
            ]))
            .match_header(
                "authorization",
                Matcher::Regex("^SharedAccessSignature sr=".into()),
            )
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_feed.xml"))
            .create_async()
            .await;
        let queues = client(&server).list_queues().await.unwrap();
        assert_eq!(queues.len(), 2);
        let orders = &queues[0];
        assert_eq!(orders.name, "orders");
        assert_eq!(orders.counts.as_ref().unwrap().active, 3);
        assert_eq!(orders.counts.as_ref().unwrap().dead_letter, 1);
        assert_eq!(orders.max_delivery_count, Some(10));
        assert_eq!(orders.forward_to, None);
        assert!(queues[1].requires_session);
        assert_eq!(queues[1].counts, None);
    }

    #[tokio::test]
    async fn list_pages_until_short_page() {
        let mut server = mockito::Server::new_async().await;
        let entry = |i: usize| {
            format!(
                r#"<entry><title>q{i}</title><content><QueueDescription><Status>Active</Status></QueueDescription></content></entry>"#
            )
        };
        let page1 = format!("<feed>{}</feed>", (0..100).map(entry).collect::<String>());
        let page2 = format!("<feed>{}</feed>", entry(100));
        let _p1 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "0".into()))
            .with_status(200)
            .with_body(page1)
            .create_async()
            .await;
        let _p2 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "100".into()))
            .with_status(200)
            .with_body(page2)
            .create_async()
            .await;
        assert_eq!(client(&server).list_queues().await.unwrap().len(), 101);
    }

    #[tokio::test]
    async fn exact_page_then_empty_page_yields_100() {
        let mut server = mockito::Server::new_async().await;
        let entry = |i: usize| {
            format!(
                r#"<entry><title>q{i}</title><content><QueueDescription><Status>Active</Status></QueueDescription></content></entry>"#
            )
        };
        let page1 = format!("<feed>{}</feed>", (0..100).map(entry).collect::<String>());
        let _p1 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "0".into()))
            .with_status(200)
            .with_body(page1)
            .create_async()
            .await;
        let _p2 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "100".into()))
            .with_status(200)
            .with_body("<feed></feed>")
            .create_async()
            .await;
        assert_eq!(client(&server).list_queues().await.unwrap().len(), 100);
    }

    #[tokio::test]
    async fn server_ignoring_skip_terminates_without_duplicates() {
        let mut server = mockito::Server::new_async().await;
        let entry = |i: usize| {
            format!(
                r#"<entry><title>q{i}</title><content><QueueDescription><Status>Active</Status></QueueDescription></content></entry>"#
            )
        };
        let page = format!("<feed>{}</feed>", (0..100).map(entry).collect::<String>());
        let mock = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(page)
            .expect(2)
            .create_async()
            .await;
        let queues = client(&server).list_queues().await.unwrap();
        assert_eq!(queues.len(), 100);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn entity_names_are_percent_encoded_in_the_path() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a%20b%3Fc")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"<entry><title>a b?c</title><content><QueueDescription><Status>Active</Status></QueueDescription></content></entry>"#)
            .create_async()
            .await;
        let q = client(&server).get_queue("a b?c").await.unwrap();
        assert_eq!(q.name, "a b?c");
    }

    #[tokio::test]
    async fn get_missing_queue_is_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/nope")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="text">Publicly Listed Services</title></feed>"#)
            .create_async()
            .await;
        assert!(matches!(
            client(&server).get_queue("nope").await.unwrap_err(),
            Error::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn forbidden_names_missing_claims() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::Any)
            .with_status(403)
            .with_body("<Error><Code>403</Code><Detail>Unauthorized access. Manage,EntityRead claims required for this operation. TrackingId:abc</Detail></Error>")
            .create_async()
            .await;
        match client(&server).list_queues().await.unwrap_err() {
            Error::Forbidden { needed, .. } => {
                assert_eq!(needed.as_deref(), Some("Manage,EntityRead"))
            }
            other => panic!("expected Forbidden, got {other}"),
        }
    }

    #[tokio::test]
    async fn unauthorized_is_auth() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/topics")
            .match_query(Matcher::Any)
            .with_status(401)
            .with_body("<Error><Code>401</Code><Detail>MissingToken: The authorization header was not found.</Detail></Error>")
            .create_async()
            .await;
        assert!(
            matches!(client(&server).list_topics().await.unwrap_err(), Error::Auth(m) if m.contains("MissingToken"))
        );
    }

    #[test]
    fn needed_claims_variants() {
        assert_eq!(
            needed_claims(
                "Authorization failed for specified action: Manage,EntityWrite. TrackingId:x"
            )
            .as_deref(),
            Some("Manage,EntityWrite")
        );
        assert_eq!(
            needed_claims("Listen claims required").as_deref(),
            Some("Listen")
        );
        assert_eq!(needed_claims("something else"), None);
    }

    #[test]
    fn needed_claims_recognises_the_amqp_wording() {
        assert_eq!(
            needed_claims(
                "Unauthorized access. 'Listen' claim(s) are required to perform this operation. Resource: 'sb://ns.servicebus.windows.net/q'."
            )
            .as_deref(),
            Some("Listen")
        );
        assert_eq!(
            needed_claims(
                r#"Unauthorized access. "Manage,EntityRead" claim(s) are required to perform this operation."#
            )
            .as_deref(),
            Some("Manage,EntityRead")
        );
        assert_eq!(
            needed_claims("Listen claim(s) are required").as_deref(),
            Some("Listen")
        );
        assert_eq!(
            needed_claims("Manage,EntityRead claims are required").as_deref(),
            Some("Manage,EntityRead")
        );
    }

    #[tokio::test]
    async fn unauthorized_with_missing_claims_is_forbidden() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::Any)
            .with_status(401)
            .with_body("<Error><Code>401</Code><Detail>Unauthorized access. Manage,EntityRead claims required for this operation. TrackingId:abc</Detail></Error>")
            .create_async()
            .await;
        match client(&server).list_queues().await.unwrap_err() {
            Error::Forbidden { needed, .. } => {
                assert_eq!(needed.as_deref(), Some("Manage,EntityRead"))
            }
            other => panic!("expected Forbidden, got {other}"),
        }
    }

    #[tokio::test]
    async fn queue_details_list_leaf_properties_in_order() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/orders")
            .match_query(Matcher::UrlEncoded("enrich".into(), "true".into()))
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry.xml"))
            .create_async()
            .await;
        let d = client(&server)
            .entity_details(EntityKind::Queue, "orders")
            .await
            .unwrap();
        assert_eq!(
            (d.kind, d.name.as_str(), d.topic.as_deref()),
            (EntityKind::Queue, "orders", None)
        );
        assert_eq!(d.status.as_deref(), Some("Active"));
        assert_eq!(d.counts.as_ref().unwrap().active, 37);
        assert_eq!(d.size_in_bytes, Some(81_920));
        let names: Vec<&str> = d.properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            &names[..3],
            [
                "LockDuration",
                "MaxSizeInMegabytes",
                "RequiresDuplicateDetection"
            ]
        );
        assert!(!names.contains(&"CountDetails") && !names.contains(&"AuthorizationRules"));
        assert!(!names.contains(&"ForwardTo"), "empty leaves are left out");
        assert!(!serde_json::to_string(&d).unwrap().contains("secret-ish"));
    }

    #[tokio::test]
    async fn details_of_the_wrong_kind_are_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/orders")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_entry.xml"))
            .create_async()
            .await;
        let e = client(&server)
            .entity_details(EntityKind::Topic, "orders")
            .await
            .unwrap_err();
        assert!(matches!(e, Error::NotFound(_)), "{e:?}");
        let _empty = server
            .mock("GET", "/gone")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="text">Publicly Listed Services</title></feed>"#)
            .create_async()
            .await;
        assert!(matches!(
            client(&server)
                .entity_details(EntityKind::Queue, "gone")
                .await
                .unwrap_err(),
            Error::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn subscription_details_use_the_subscription_path() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title type="text">audit</title><content type="application/xml">
            <SubscriptionDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect">
            <LockDuration>PT30S</LockDuration><RequiresSession>true</RequiresSession><Status>Active</Status></SubscriptionDescription></content></entry>"#;
        let _m = server
            .mock("GET", "/events/Subscriptions/audit")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;
        let d = client(&server)
            .entity_details(EntityKind::Subscription, "events/subscriptions/audit")
            .await
            .unwrap();
        assert_eq!(
            (d.name.as_str(), d.topic.as_deref()),
            ("audit", Some("events"))
        );
        assert_eq!(d.counts, None);
        assert!(
            d.properties
                .iter()
                .any(|p| p.name == "RequiresSession" && p.value == "true")
        );
        assert!(
            client(&server)
                .entity_details(EntityKind::Subscription, "events")
                .await
                .is_err(),
            "not a subscription path"
        );
    }

    #[tokio::test]
    async fn rules_parse_sql_correlation_and_actions() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/events/Subscriptions/all/Rules")
            .match_query(Matcher::UrlEncoded("$top".into(), "100".into()))
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/rules_feed.xml"))
            .create_async()
            .await;
        let rules = client(&server).list_rules("events", "all").await.unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules[0].filter,
            RuleFilter::Sql {
                expression: "1=1".into()
            }
        );
        assert_eq!(rules[0].action, None);
        assert_eq!(rules[0].created_at.as_deref(), Some("2026-09-01T08:00:00Z"));
        assert_eq!(rules[1].action.as_deref(), Some("SET priority = 'high'"));
        let RuleFilter::Correlation {
            correlation_id,
            subject,
            properties,
            to,
            ..
        } = &rules[2].filter
        else {
            panic!("{:?}", rules[2])
        };
        assert_eq!(
            (correlation_id.as_deref(), subject.as_deref(), to.as_deref()),
            (Some("batch-7"), Some("priority"), None)
        );
        assert_eq!(
            properties,
            &[
                CorrelationProperty {
                    name: "region".into(),
                    value: "eu".into()
                },
                CorrelationProperty {
                    name: "tier".into(),
                    value: "2".into()
                },
            ]
        );
        let json = serde_json::to_value(&rules[2]).unwrap();
        assert_eq!(json["filter"]["kind"], "correlation");
    }
}
