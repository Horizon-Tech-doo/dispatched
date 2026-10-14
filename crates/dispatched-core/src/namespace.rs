//! A Service Bus namespace: endpoints, audiences and credentials.

use crate::connstr::ConnectionString;
use crate::tokens::{PresignedSasSource, SasKeySource, TokenSource};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Which queue of an entity to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubQueue {
    /// The entity itself.
    Active,
    /// `$DeadLetterQueue`.
    DeadLetter,
    /// `$Transfer/$DeadLetterQueue`.
    TransferDeadLetter,
}

/// What a user wants to see. Scheduled and deferred messages live in the
/// entity itself and are told apart by their state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeekView {
    /// Active messages.
    Active,
    /// Scheduled (not yet enqueued) messages.
    Scheduled,
    /// Deferred messages.
    Deferred,
    /// The dead-letter queue (all states).
    DeadLetter,
    /// The transfer dead-letter queue (all states).
    TransferDeadLetter,
}

impl PeekView {
    /// Queue to read from.
    pub fn sub_queue(self) -> SubQueue {
        match self {
            PeekView::Active | PeekView::Scheduled | PeekView::Deferred => SubQueue::Active,
            PeekView::DeadLetter => SubQueue::DeadLetter,
            PeekView::TransferDeadLetter => SubQueue::TransferDeadLetter,
        }
    }

    /// Whether a message in `state` belongs to this view.
    pub fn accepts(self, state: crate::messaging::MessageState) -> bool {
        use crate::messaging::MessageState as S;
        match self {
            PeekView::Active => state == S::Active,
            PeekView::Scheduled => state == S::Scheduled,
            PeekView::Deferred => state == S::Deferred,
            PeekView::DeadLetter | PeekView::TransferDeadLetter => true,
        }
    }

    /// Stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            PeekView::Active => "active",
            PeekView::Scheduled => "scheduled",
            PeekView::Deferred => "deferred",
            PeekView::DeadLetter => "dead-letter",
            PeekView::TransferDeadLetter => "transfer-dead-letter",
        }
    }

    /// Parse a user-facing name.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(PeekView::Active),
            "scheduled" => Ok(PeekView::Scheduled),
            "deferred" => Ok(PeekView::Deferred),
            "dlq" | "dead-letter" | "deadletter" => Ok(PeekView::DeadLetter),
            "transfer-dlq" | "transfer-dead-letter" => Ok(PeekView::TransferDeadLetter),
            other => Err(Error::Invalid(format!(
                "unknown view `{other}`; use active, scheduled, deferred, dlq or transfer-dlq"
            ))),
        }
    }
}

/// A queue, or a topic subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityPath {
    /// Queue or topic name.
    pub entity: String,
    /// Subscription name when `entity` is a topic.
    pub subscription: Option<String>,
}

impl EntityPath {
    /// A queue.
    pub fn queue(name: &str) -> Self {
        Self {
            entity: name.to_string(),
            subscription: None,
        }
    }

    /// A topic subscription.
    pub fn subscription(topic: &str, subscription: &str) -> Self {
        Self {
            entity: topic.to_string(),
            subscription: Some(subscription.to_string()),
        }
    }

    /// Parse `orders` or `events/subscriptions/all` (`Subscriptions` in any case).
    pub fn parse(input: &str) -> Result<Self> {
        let parts: Vec<&str> = input.trim().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            [q] if !q.is_empty() => Ok(Self::queue(q)),
            [t, s, sub]
                if s.eq_ignore_ascii_case("subscriptions") && !t.is_empty() && !sub.is_empty() =>
            {
                Ok(Self::subscription(t, sub))
            }
            _ => Err(Error::Invalid(format!(
                "`{input}` is not an entity; use `queue` or `topic/subscriptions/subscription`"
            ))),
        }
    }

    /// Canonical path without sub-queue.
    pub fn base(&self) -> String {
        match &self.subscription {
            None => self.entity.clone(),
            Some(sub) => format!("{}/Subscriptions/{sub}", self.entity),
        }
    }

    /// AMQP address including the sub-queue suffix.
    pub fn address(&self, sub_queue: SubQueue) -> String {
        match sub_queue {
            SubQueue::Active => self.base(),
            SubQueue::DeadLetter => format!("{}/$DeadLetterQueue", self.base()),
            SubQueue::TransferDeadLetter => format!("{}/$Transfer/$DeadLetterQueue", self.base()),
        }
    }
}

impl fmt::Display for EntityPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base())
    }
}

/// Emulator ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatorPorts {
    /// Plain AMQP port (5672).
    pub amqp: u16,
    /// HTTP management port (5300).
    pub management: u16,
}

/// How the AMQP connection is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// AMQP over TLS on 5671 (plain 5672 on the emulator).
    #[default]
    Amqp,
    /// AMQP over secure WebSockets on 443.
    WebSockets,
}

impl Transport {
    /// Parse `amqp` or `websockets`.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "amqp" => Ok(Transport::Amqp),
            "websockets" | "ws" | "wss" => Ok(Transport::WebSockets),
            other => Err(Error::Invalid(format!(
                "unknown transport `{other}`; use amqp or websockets"
            ))),
        }
    }
}

/// An HTTP proxy for WebSockets and HTTPS calls.
#[derive(Clone)]
pub struct ProxyConfig {
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<secrecy::SecretString>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth", &self.user.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ProxyConfig {
    /// Parse `http://[user:pass@]host:port`.
    pub fn parse(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url)
            .map_err(|_| Error::Invalid("proxy must look like http://host:port".into()))?;
        if parsed.scheme() != "http" {
            return Err(Error::Invalid(
                "only http:// proxies are supported (the tunnel itself is TLS)".into(),
            ));
        }
        // Store the dialable host: `host_str()` brackets IPv6 literals, which
        // `TcpStream::connect` cannot resolve.
        let host = match parsed.host() {
            Some(url::Host::Domain(d)) if !d.is_empty() => d.to_string(),
            Some(url::Host::Ipv4(a)) => a.to_string(),
            Some(url::Host::Ipv6(a)) => a.to_string(),
            _ => return Err(Error::Invalid("proxy URL has no host".into())),
        };
        let port = parsed.port_or_known_default().unwrap_or(80);
        let user = (!parsed.username().is_empty()).then(|| {
            percent_encoding::percent_decode_str(parsed.username())
                .decode_utf8_lossy()
                .into_owned()
        });
        let password = parsed.password().map(|p| {
            secrecy::SecretString::from(
                percent_encoding::percent_decode_str(p)
                    .decode_utf8_lossy()
                    .into_owned(),
            )
        });
        Ok(Self {
            host,
            port,
            user,
            password,
        })
    }

    /// `HTTPS_PROXY` / `https_proxy`, if set and valid.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("https_proxy"))
            .ok()?;
        match Self::parse(&value) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("ignoring HTTPS_PROXY: {e}");
                None
            }
        }
    }

    /// Proxy host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Proxy port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Basic-auth credentials, if any.
    pub fn basic_auth(&self) -> Option<(String, secrecy::SecretString)> {
        Some((
            self.user.clone()?,
            self.password
                .clone()
                .unwrap_or_else(|| secrecy::SecretString::from("")),
        ))
    }

    /// `http://host:port` without credentials, as stored in profiles.
    pub fn without_credentials_url(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("http://{host}:{}", self.port)
    }

    /// URL for reqwest (contains credentials — never log it).
    pub fn url_for_reqwest(&self) -> String {
        use secrecy::ExposeSecret;
        // IPv6 literals need brackets in an authority.
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let enc = |s: &str| {
            percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
        };
        match (&self.user, &self.password) {
            (Some(u), Some(p)) => format!(
                "http://{}:{}@{}:{}",
                enc(u),
                enc(p.expose_secret()),
                host,
                self.port
            ),
            (Some(u), None) => format!("http://{}@{}:{}", enc(u), host, self.port),
            _ => format!("http://{host}:{}", self.port),
        }
    }
}

/// A namespace plus the credentials to reach it.
#[derive(Clone)]
pub struct Namespace {
    host: String,
    emulator: Option<EmulatorPorts>,
    tokens: Arc<dyn TokenSource>,
    entity_path: Option<String>,
    transport: Transport,
    proxy: Option<ProxyConfig>,
}

impl fmt::Debug for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Namespace")
            .field("host", &self.host)
            .field("emulator", &self.emulator)
            .field("auth", &self.tokens.describe())
            .field("entity_path", &self.entity_path)
            .field("transport", &self.transport)
            .field("proxy", &self.proxy)
            .finish()
    }
}

impl Namespace {
    /// A cloud namespace (`myns.servicebus.windows.net`).
    pub fn new(host: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            host: host.into().to_ascii_lowercase(),
            emulator: None,
            tokens,
            entity_path: None,
            transport: Transport::Amqp,
            proxy: None,
        }
    }

    /// The local Service Bus Emulator.
    pub fn emulator(
        host: impl Into<String>,
        ports: EmulatorPorts,
        tokens: Arc<dyn TokenSource>,
    ) -> Self {
        Self {
            host: host.into().to_ascii_lowercase(),
            emulator: Some(ports),
            tokens,
            entity_path: None,
            transport: Transport::Amqp,
            proxy: None,
        }
    }

    /// Build from a parsed connection string.
    pub fn from_connection_string(cs: &ConnectionString) -> Result<Self> {
        let tokens: Arc<dyn TokenSource> = match (
            &cs.shared_access_signature,
            &cs.shared_access_key_name,
            &cs.shared_access_key,
        ) {
            (Some(sas), _, _) => Arc::new(PresignedSasSource::parse(sas.clone())?),
            (None, Some(name), Some(key)) => Arc::new(SasKeySource::new(name.clone(), key.clone())),
            _ => {
                return Err(Error::Invalid(
                    "connection string has no usable credentials".into(),
                ));
            }
        };
        let mut ns = if cs.use_development_emulator {
            Self::emulator(
                cs.host.clone(),
                EmulatorPorts {
                    amqp: 5672,
                    management: cs.port.unwrap_or(5300),
                },
                tokens,
            )
        } else {
            Self::new(cs.host.clone(), tokens)
        };
        ns.entity_path = cs.entity_path.clone();
        Ok(ns)
    }

    /// Choose the transport. WebSockets are not available on the emulator.
    pub fn with_transport(mut self, transport: Transport) -> Result<Self> {
        if self.is_emulator() && transport == Transport::WebSockets {
            return Err(Error::Unsupported {
                capability: "AMQP over WebSockets on the Service Bus Emulator".into(),
            });
        }
        if self.proxy.is_some() && transport == Transport::Amqp {
            return Err(Error::Invalid(
                "a proxy requires the websockets transport".into(),
            ));
        }
        self.transport = transport;
        Ok(self)
    }

    /// Route through an HTTP proxy (forces WebSockets).
    pub fn with_proxy(mut self, proxy: Option<ProxyConfig>) -> Result<Self> {
        if let Some(p) = proxy {
            if self.is_emulator() {
                return Err(Error::Unsupported {
                    capability: "a proxy with the Service Bus Emulator".into(),
                });
            }
            self.proxy = Some(p);
            self.transport = Transport::WebSockets;
        }
        Ok(self)
    }

    /// Transport in use.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Proxy, if any.
    pub fn proxy(&self) -> Option<&ProxyConfig> {
        self.proxy.as_ref()
    }

    /// Namespace host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether this is the local emulator.
    pub fn is_emulator(&self) -> bool {
        self.emulator.is_some()
    }

    /// Credential source.
    pub fn tokens(&self) -> &Arc<dyn TokenSource> {
        &self.tokens
    }

    /// `EntityPath` from an entity-scoped connection string.
    pub fn entity_path(&self) -> Option<&str> {
        self.entity_path.as_deref()
    }

    /// AMQP connection URL.
    pub fn amqp_url(&self) -> String {
        match self.emulator {
            Some(p) => format!("amqp://{}:{}", self.host, p.amqp),
            None => format!("amqps://{}:5671", self.host),
        }
    }

    /// CBS audience / link address URI for `address` (e.g. `orders/$DeadLetterQueue`).
    pub fn amqp_audience(&self, address: &str) -> String {
        let scheme = if self.is_emulator() { "amqp" } else { "amqps" };
        format!("{scheme}://{}/{address}", self.host)
    }

    /// Base URL of the ATOM management API, without trailing slash.
    pub fn management_base(&self) -> String {
        match self.emulator {
            Some(p) => format!("http://{}:{}", self.host, p.management),
            None => format!("https://{}", self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator_ns() -> Namespace {
        let cs = ConnectionString::parse(
            "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;",
        )
        .unwrap();
        Namespace::from_connection_string(&cs).unwrap()
    }

    #[test]
    fn emulator_endpoints() {
        let ns = emulator_ns();
        assert!(ns.is_emulator());
        assert_eq!(ns.amqp_url(), "amqp://localhost:5672");
        assert_eq!(ns.management_base(), "http://localhost:5300");
        assert_eq!(ns.amqp_audience("orders"), "amqp://localhost/orders");
    }

    #[test]
    fn cloud_endpoints() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://MyNs.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v",
        )
        .unwrap();
        let ns = Namespace::from_connection_string(&cs).unwrap();
        assert_eq!(ns.amqp_url(), "amqps://myns.servicebus.windows.net:5671");
        assert_eq!(ns.management_base(), "https://myns.servicebus.windows.net");
        assert_eq!(
            ns.amqp_audience("orders/$DeadLetterQueue"),
            "amqps://myns.servicebus.windows.net/orders/$DeadLetterQueue"
        );
    }

    #[test]
    fn entity_path_parse_and_addresses() {
        let q = EntityPath::parse("orders").unwrap();
        assert_eq!(q.address(SubQueue::DeadLetter), "orders/$DeadLetterQueue");
        let s = EntityPath::parse("events/subscriptions/all").unwrap();
        assert_eq!(s.base(), "events/Subscriptions/all");
        assert_eq!(
            s.address(SubQueue::TransferDeadLetter),
            "events/Subscriptions/all/$Transfer/$DeadLetterQueue"
        );
        assert!(EntityPath::parse("a/b").is_err());
        assert!(EntityPath::parse("").is_err());
    }

    #[test]
    fn peek_views_map_to_sub_queues_and_states() {
        use crate::messaging::MessageState as S;
        assert_eq!(PeekView::Scheduled.sub_queue(), SubQueue::Active);
        assert_eq!(PeekView::parse("DLQ").unwrap(), PeekView::DeadLetter);
        assert!(PeekView::Scheduled.accepts(S::Scheduled));
        assert!(!PeekView::Active.accepts(S::Deferred));
        assert!(PeekView::DeadLetter.accepts(S::Deferred));
        assert!(
            PeekView::parse("nope")
                .unwrap_err()
                .to_string()
                .contains("unknown view")
        );
    }

    #[test]
    fn debug_never_prints_secrets() {
        let dbg = format!("{:?}", emulator_ns());
        assert!(!dbg.contains("SAS_KEY_VALUE"));
        assert!(dbg.contains("SAS key RootManageSharedAccessKey"));
    }

    #[test]
    fn emulator_rejects_websockets_and_proxy() {
        let ns = emulator_ns();
        assert!(matches!(
            ns.clone().with_transport(Transport::WebSockets),
            Err(Error::Unsupported { .. })
        ));
        let p = ProxyConfig::parse("http://proxy:3128").unwrap();
        assert!(matches!(
            ns.with_proxy(Some(p)),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn proxy_forces_websockets_and_redacts() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v",
        )
        .unwrap();
        let p = ProxyConfig::parse("http://alice:s3cr%40t@proxy.corp:8080").unwrap();
        assert_eq!(p.basic_auth().unwrap().0, "alice");
        let ns = Namespace::from_connection_string(&cs)
            .unwrap()
            .with_proxy(Some(p))
            .unwrap();
        assert_eq!(ns.transport(), Transport::WebSockets);
        let dbg = format!("{ns:?}");
        assert!(!dbg.contains("s3cr"), "{dbg}");
        assert!(ProxyConfig::parse("https://proxy:443").is_err());
        assert!(ProxyConfig::parse("socks5://p:1").is_err());
    }

    #[test]
    fn ipv6_proxy_host_is_stored_unbracketed() {
        let p = ProxyConfig::parse("http://[::1]:3128").unwrap();
        assert_eq!(p.host(), "::1");
        assert_eq!(p.port(), 3128);
        assert_eq!(p.url_for_reqwest(), "http://[::1]:3128");
        let p = ProxyConfig::parse("http://u:p@[2001:db8::2]:8080").unwrap();
        assert_eq!(p.host(), "2001:db8::2");
        assert_eq!(p.url_for_reqwest(), "http://u:p@[2001:db8::2]:8080");
        let p = ProxyConfig::parse("http://proxy.corp:3128").unwrap();
        assert_eq!(p.host(), "proxy.corp");
    }

    #[test]
    fn without_credentials_url_drops_auth_and_brackets_ipv6() {
        for (input, expected) in [
            ("http://u:p@proxy.corp:3128", "http://proxy.corp:3128"),
            ("http://[::1]:3128", "http://[::1]:3128"),
            ("http://u:p@[2001:db8::2]:8080", "http://[2001:db8::2]:8080"),
            ("http://10.0.0.1:80", "http://10.0.0.1:80"),
        ] {
            let url = ProxyConfig::parse(input).unwrap().without_credentials_url();
            assert_eq!(url, expected);
            assert_eq!(
                ProxyConfig::parse(&url).unwrap().host(),
                ProxyConfig::parse(input).unwrap().host()
            );
        }
    }
}
