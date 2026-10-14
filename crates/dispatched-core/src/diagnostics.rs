//! "Test connection": which auth path is used and which rights work.

use crate::explorer::Explorer;
use crate::messaging::PeekOptions;
use crate::namespace::{EntityPath, Namespace, PeekView, Transport};
use crate::{Error, Result};
use async_trait::async_trait;
use serde::Serialize;

/// Stable error category (shared with the desktop app and JSON output).
/// The match is exhaustive on purpose: a new `Error` variant must pick a kind.
pub fn error_kind(e: &Error) -> &'static str {
    match e {
        Error::Auth(_) => "auth",
        Error::NotSignedIn(_) => "not_signed_in",
        Error::Forbidden { .. } => "forbidden",
        Error::NotFound(_) => "not_found",
        Error::Throttled { .. } => "throttled",
        Error::Timeout(_) => "timeout",
        Error::Cancelled => "cancelled",
        Error::Unsupported { .. } => "unsupported",
        Error::Invalid(_) => "invalid",
        Error::Network(_) => "network",
        Error::Protocol(_) => "protocol",
        Error::Io(_) => "io",
    }
}

/// Every kind `error_kind` can return (the frontend's error-kind union).
pub const ERROR_KINDS: [&str; 12] = [
    "auth",
    "not_signed_in",
    "forbidden",
    "not_found",
    "throttled",
    "timeout",
    "cancelled",
    "unsupported",
    "invalid",
    "network",
    "protocol",
    "io",
];

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Check {
    /// Worked.
    Ok {
        /// What was verified.
        detail: String,
    },
    /// Failed.
    Failed {
        /// Error category.
        kind: &'static str,
        /// Error text.
        message: String,
    },
    /// Not attempted.
    Skipped {
        /// Why.
        reason: String,
    },
}

/// Result of testing a connection.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionReport {
    /// Namespace host.
    pub host: String,
    /// Emulator.
    pub emulator: bool,
    /// Transport.
    pub transport: Transport,
    /// Auth path, e.g. "SAS key RootManageSharedAccessKey" or "Entra ID a@x.io".
    pub auth: String,
    /// Listing entities (needs Manage).
    pub management: Check,
    /// Peeking an entity (needs Listen).
    pub messaging: Check,
}

/// What `test_connection` needs (implemented by `Explorer`; faked in tests).
#[async_trait]
pub trait Probe: Sync {
    /// The namespace under test.
    fn namespace(&self) -> &Namespace;
    /// Names of queues.
    async fn list_queue_names(&self) -> Result<Vec<String>>;
    /// Peek one active message; returns how many were seen.
    async fn peek_one(&self, entity: &EntityPath) -> Result<usize>;
}

#[async_trait]
impl Probe for Explorer {
    fn namespace(&self) -> &Namespace {
        Explorer::namespace(self)
    }
    async fn list_queue_names(&self) -> Result<Vec<String>> {
        Ok(self
            .management_client()
            .list_queues()
            .await?
            .into_iter()
            .map(|q| q.name)
            .collect())
    }
    async fn peek_one(&self, entity: &EntityPath) -> Result<usize> {
        Ok(self
            .peek(entity, PeekView::Active, &PeekOptions::new(1))
            .await?
            .messages
            .len())
    }
}

fn failed(e: &Error) -> Check {
    Check::Failed {
        kind: error_kind(e),
        message: e.to_string(),
    }
}

/// Run the checks.
pub async fn test_connection(
    probe: &(impl Probe + ?Sized),
    entity: Option<&EntityPath>,
) -> ConnectionReport {
    let ns = probe.namespace();
    let listed = probe.list_queue_names().await;
    let management = match &listed {
        Ok(names) => Check::Ok {
            detail: format!("listed {} queue(s) (Manage)", names.len()),
        },
        Err(e) => failed(e),
    };
    let target = entity
        .cloned()
        .or_else(|| ns.entity_path().and_then(|p| EntityPath::parse(p).ok()))
        .or_else(|| {
            listed
                .as_ref()
                .ok()
                .and_then(|n| n.first())
                .map(|q| EntityPath::queue(q))
        });
    let messaging = match target {
        None => Check::Skipped {
            reason: "no entity to probe; pass one".into(),
        },
        Some(e) => match probe.peek_one(&e).await {
            Ok(n) => Check::Ok {
                detail: format!("peeked {e} ({n} message(s) visible; Listen)"),
            },
            Err(err) => failed(&err),
        },
    };
    ConnectionReport {
        host: ns.host().to_string(),
        emulator: ns.is_emulator(),
        transport: ns.transport(),
        auth: ns.tokens().describe(),
        management,
        messaging,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionString;

    struct Fake {
        ns: Namespace,
        list: std::result::Result<Vec<String>, &'static str>,
        peek: std::result::Result<usize, &'static str>,
    }

    #[async_trait]
    impl Probe for Fake {
        fn namespace(&self) -> &Namespace {
            &self.ns
        }
        async fn list_queue_names(&self) -> Result<Vec<String>> {
            self.list.clone().map_err(|d| Error::Forbidden {
                needed: Some("Manage,EntityRead".into()),
                detail: d.into(),
            })
        }
        async fn peek_one(&self, _e: &EntityPath) -> Result<usize> {
            self.peek.map_err(|d| Error::Forbidden {
                needed: Some("Listen".into()),
                detail: d.into(),
            })
        }
    }

    fn ns(cs: &str) -> Namespace {
        Namespace::from_connection_string(&ConnectionString::parse(cs).unwrap()).unwrap()
    }

    #[test]
    fn every_error_variant_maps_to_a_distinct_documented_kind() {
        let all = [
            Error::Auth(String::new()),
            Error::NotSignedIn(String::new()),
            Error::Forbidden {
                needed: None,
                detail: String::new(),
            },
            Error::NotFound(String::new()),
            Error::Throttled { retry_after: None },
            Error::Timeout(String::new()),
            Error::Cancelled,
            Error::Unsupported {
                capability: String::new(),
            },
            Error::Invalid(String::new()),
            Error::Network(String::new()),
            Error::Protocol(String::new()),
            Error::Io(String::new()),
        ];
        let kinds: Vec<&str> = all.iter().map(error_kind).collect();
        let distinct: std::collections::HashSet<_> = kinds.iter().collect();
        assert_eq!(
            distinct.len(),
            all.len(),
            "kinds must be distinct: {kinds:?}"
        );
        for k in &kinds {
            assert!(ERROR_KINDS.contains(k), "undocumented kind {k}");
        }
        assert_eq!(ERROR_KINDS.len(), distinct.len());
        assert_eq!(error_kind(&Error::Io("x".into())), "io");
        assert_eq!(error_kind(&Error::Protocol("x".into())), "protocol");
    }

    #[tokio::test]
    async fn failure_message_has_no_secret_material() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=supersecretkey",
            ),
            list: Err("claims required"),
            peek: Err("claims required"),
        };
        let r = test_connection(&f, Some(&EntityPath::queue("q"))).await;
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("supersecretkey"));
        assert!(matches!(
            r.messaging,
            Check::Failed {
                kind: "forbidden",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn full_rights_probe_first_queue() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=k",
            ),
            list: Ok(vec!["orders".into()]),
            peek: Ok(1),
        };
        let r = test_connection(&f, None).await;
        assert!(matches!(r.management, Check::Ok { .. }));
        assert!(matches!(&r.messaging, Check::Ok { detail } if detail.contains("orders")));
        assert_eq!(r.auth, "SAS key Root");
    }

    #[tokio::test]
    async fn listen_only_key_uses_entity_path() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=listen;SharedAccessKey=k;EntityPath=orders",
            ),
            list: Err("claims required"),
            peek: Ok(0),
        };
        let r = test_connection(&f, None).await;
        assert!(matches!(
            r.management,
            Check::Failed {
                kind: "forbidden",
                ..
            }
        ));
        assert!(matches!(r.messaging, Check::Ok { .. }));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["management"]["status"], "failed");
    }

    #[tokio::test]
    async fn nothing_to_probe_is_skipped() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=s;SharedAccessKey=k",
            ),
            list: Err("no"),
            peek: Ok(0),
        };
        assert!(matches!(
            test_connection(&f, None).await.messaging,
            Check::Skipped { .. }
        ));
    }
}
