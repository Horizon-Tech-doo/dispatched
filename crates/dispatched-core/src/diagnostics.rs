//! "Test connection": which auth path is used and which rights work.

use crate::explorer::Explorer;
use crate::messaging::PeekOptions;
use crate::namespace::{EntityPath, Namespace, PeekView, Transport};
use crate::{Error, Result};
use async_trait::async_trait;
use serde::Serialize;
use std::future::Future;
use std::time::Duration;

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

/// Every kind `error_kind` can return. The desktop app and the CLI branch on
/// these exact strings, so they are a stable contract.
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

/// Longest one probe call may take before its check is reported as timed out.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// "1 queue", "4 queues".
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Bound one probe call so a single hang cannot stall the whole report.
async fn bounded<T>(what: &str, fut: impl Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout(CHECK_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(Error::Timeout(format!(
            "{what} did not answer within {} s",
            CHECK_TIMEOUT.as_secs()
        ))),
    }
}

/// Run the checks.
pub async fn test_connection(
    probe: &(impl Probe + ?Sized),
    entity: Option<&EntityPath>,
) -> ConnectionReport {
    let ns = probe.namespace();
    let listed = bounded("listing queues", probe.list_queue_names()).await;
    let management = match &listed {
        Ok(names) => Check::Ok {
            detail: format!("listed {} (Manage)", count(names.len(), "queue")),
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
            reason: if listed.is_err() {
                "no queue could be discovered because the management check failed; pass an entity to test Listen"
            } else {
                "no entity to probe; pass one"
            }
            .into(),
        },
        Some(e) => match bounded("peeking", probe.peek_one(&e)).await {
            Ok(n) => Check::Ok {
                detail: format!("peeked {e} ({} visible; Listen)", count(n, "message")),
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
        hang_list: bool,
        hang_peek: bool,
        peeked: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl Probe for Fake {
        fn namespace(&self) -> &Namespace {
            &self.ns
        }
        async fn list_queue_names(&self) -> Result<Vec<String>> {
            if self.hang_list {
                std::future::pending::<()>().await;
            }
            self.list.clone().map_err(|d| Error::Forbidden {
                needed: Some("Manage,EntityRead".into()),
                detail: d.into(),
            })
        }
        async fn peek_one(&self, e: &EntityPath) -> Result<usize> {
            *self.peeked.lock().unwrap() = Some(e.to_string());
            if self.hang_peek {
                std::future::pending::<()>().await;
            }
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
    fn every_error_variant_maps_to_its_pinned_kind() {
        let table: [(Error, &str); 12] = [
            (Error::Auth(String::new()), "auth"),
            (Error::NotSignedIn(String::new()), "not_signed_in"),
            (
                Error::Forbidden {
                    needed: None,
                    detail: String::new(),
                },
                "forbidden",
            ),
            (Error::NotFound(String::new()), "not_found"),
            (Error::Throttled { retry_after: None }, "throttled"),
            (Error::Timeout(String::new()), "timeout"),
            (Error::Cancelled, "cancelled"),
            (
                Error::Unsupported {
                    capability: String::new(),
                },
                "unsupported",
            ),
            (Error::Invalid(String::new()), "invalid"),
            (Error::Network(String::new()), "network"),
            (Error::Protocol(String::new()), "protocol"),
            (Error::Io(String::new()), "io"),
        ];
        for (e, expected) in &table {
            assert_eq!(error_kind(e), *expected, "{e:?}");
        }
        let kinds: std::collections::HashSet<&str> = table.iter().map(|(_, k)| *k).collect();
        assert_eq!(kinds.len(), table.len(), "kinds must be distinct");
        let documented: std::collections::HashSet<&str> = ERROR_KINDS.iter().copied().collect();
        assert_eq!(kinds, documented);
    }

    #[tokio::test]
    async fn failure_message_has_no_secret_material() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=supersecretkey",
            ),
            list: Err("claims required"),
            peek: Err("claims required"),
            hang_list: false,
            hang_peek: false,
            peeked: Default::default(),
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

    #[test]
    fn counts_are_pluralised() {
        assert_eq!(count(0, "queue"), "0 queues");
        assert_eq!(count(1, "queue"), "1 queue");
        assert_eq!(count(4, "message"), "4 messages");
    }

    #[tokio::test]
    async fn full_rights_probe_first_queue() {
        let f = Fake {
            ns: ns(
                "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=k",
            ),
            list: Ok(vec!["orders".into()]),
            peek: Ok(1),
            hang_list: false,
            hang_peek: false,
            peeked: Default::default(),
        };
        let r = test_connection(&f, None).await;
        assert!(
            matches!(&r.management, Check::Ok { detail } if detail == "listed 1 queue (Manage)")
        );
        assert!(
            matches!(&r.messaging, Check::Ok { detail } if detail == "peeked orders (1 message visible; Listen)")
        );
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
            hang_list: false,
            hang_peek: false,
            peeked: Default::default(),
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
            hang_list: false,
            hang_peek: false,
            peeked: Default::default(),
        };
        assert!(matches!(
            test_connection(&f, None).await.messaging,
            Check::Skipped { .. }
        ));
    }

    const SAS: &str =
        "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=k";

    fn fake(ns_: Namespace) -> Fake {
        Fake {
            ns: ns_,
            list: Ok(vec!["first".into()]),
            peek: Ok(0),
            hang_list: false,
            hang_peek: false,
            peeked: Default::default(),
        }
    }

    #[tokio::test]
    async fn manage_ok_but_listen_forbidden() {
        let mut f = fake(ns(SAS));
        f.peek = Err("no listen claim");
        let r = test_connection(&f, None).await;
        assert!(matches!(r.management, Check::Ok { .. }));
        assert!(matches!(
            &r.messaging,
            Check::Failed { kind: "forbidden", message } if message.contains("needs Listen")
        ));
    }

    #[tokio::test]
    async fn explicit_entity_wins_over_namespace_path_and_first_queue() {
        let f = fake(ns(&format!("{SAS};EntityPath=from-conn-string")));
        let given = EntityPath::queue("explicit");
        test_connection(&f, Some(&given)).await;
        assert_eq!(f.peeked.lock().unwrap().as_deref(), Some("explicit"));

        // Without an argument the namespace EntityPath beats the first queue.
        test_connection(&f, None).await;
        assert_eq!(
            f.peeked.lock().unwrap().as_deref(),
            Some("from-conn-string")
        );
    }

    #[tokio::test]
    async fn skipped_reason_names_the_failed_management_check() {
        let mut f = fake(ns(SAS));
        f.list = Err("claims required");
        let r = test_connection(&f, None).await;
        assert!(matches!(
            &r.messaging,
            Check::Skipped { reason } if reason.contains("management check failed") && reason.contains("pass an entity")
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_listing_times_out_without_blocking_the_report() {
        let mut f = fake(ns(SAS));
        f.hang_list = true;
        let r = test_connection(&f, Some(&EntityPath::queue("q"))).await;
        assert!(matches!(
            &r.management,
            Check::Failed { kind: "timeout", message } if message.contains("did not answer within 30 s")
        ));
        assert!(matches!(r.messaging, Check::Ok { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_peek_times_out() {
        let mut f = fake(ns(SAS));
        f.hang_peek = true;
        let r = test_connection(&f, None).await;
        assert!(matches!(r.management, Check::Ok { .. }));
        assert!(matches!(
            &r.messaging,
            Check::Failed { kind: "timeout", message } if message.contains("peeking did not answer within 30 s")
        ));
    }
}
