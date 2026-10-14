//! The facade every surface uses: one namespace, lazily-opened AMQP connection.

use crate::messaging::{PeekOptions, PeekPage, ServiceBusConnection, scan_messages};
use crate::mgmt::{ManagementClient, QueueInfo, SubscriptionInfo, TopicInfo};
use crate::namespace::{EntityPath, Namespace, PeekView};
use crate::{Error, Result};
use tokio::sync::Mutex;

/// Browse and (from M2) operate one namespace.
pub struct Explorer {
    ns: Namespace,
    mgmt: ManagementClient,
    amqp: Mutex<Option<ServiceBusConnection>>,
}

impl Explorer {
    /// Explorer for `ns`. No network I/O until first use.
    pub fn new(ns: Namespace) -> Result<Self> {
        let mgmt = ManagementClient::new(&ns)?;
        Ok(Self {
            ns,
            mgmt,
            amqp: Mutex::new(None),
        })
    }

    /// The namespace.
    pub fn namespace(&self) -> &Namespace {
        &self.ns
    }

    /// Queues with counts.
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        self.mgmt.list_queues().await
    }

    /// Topics.
    pub async fn list_topics(&self) -> Result<Vec<TopicInfo>> {
        self.mgmt.list_topics().await
    }

    /// Subscriptions of `topic`.
    pub async fn list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>> {
        self.mgmt.list_subscriptions(topic).await
    }

    /// Peek `view` of `entity` according to `opts`.
    pub async fn peek(
        &self,
        entity: &EntityPath,
        view: PeekView,
        opts: &PeekOptions,
    ) -> Result<PeekPage> {
        let address = entity.address(view.sub_queue());
        with_one_retry(|_attempt| self.peek_once(&address, view, opts)).await
    }

    /// One peek attempt on the cached connection (opened on demand). A
    /// connection-level or auth failure drops the cached connection so the
    /// next attempt starts clean.
    async fn peek_once(
        &self,
        address: &str,
        view: PeekView,
        opts: &PeekOptions,
    ) -> Result<PeekPage> {
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        let conn = guard.as_mut().expect("opened above");
        let result = scan_messages(conn, address, view, opts).await;
        if result.as_ref().is_err_and(|e| retry_after(e, 0))
            && let Some(conn) = guard.take()
        {
            let _ = conn.close().await;
        }
        result
    }

    /// Close the AMQP connection, if open.
    pub async fn close(&self) {
        if let Some(conn) = self.amqp.lock().await.take() {
            let _ = conn.close().await;
        }
    }
}

/// Whether a failed operation should be retried on a fresh connection.
/// `Auth` usually means a token expired mid-session, and `Network`,
/// `Protocol` and `Timeout` mean the connection or a link died (the broker
/// closes links when their token lapses). A new connection re-puts fresh
/// tokens. Answers from a healthy service (`Forbidden`, `NotFound`, ...) are
/// never retried. These are also exactly the errors that poison the cached
/// connection.
pub fn retry_after(err: &Error, attempt: u32) -> bool {
    attempt == 0
        && matches!(
            err,
            Error::Auth(_) | Error::Network(_) | Error::Protocol(_) | Error::Timeout(_)
        )
}

/// Run `f(attempt)` and, if [`retry_after`] says so, run it once more.
/// At most two calls; the second result is returned as is.
async fn with_one_retry<T, F, Fut>(mut f: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match f(0).await {
        Err(e) if retry_after(&e, 0) => f(1).await,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_set_is_auth_and_connection_level_only() {
        assert!(!retry_after(
            &Error::Forbidden {
                needed: None,
                detail: "x".into()
            },
            0
        ));
        for e in [
            Error::Auth("x".into()),
            Error::Network("x".into()),
            Error::Protocol("x".into()),
            Error::Timeout("x".into()),
        ] {
            assert!(retry_after(&e, 0), "{e:?}");
            assert!(!retry_after(&e, 1), "{e:?}");
        }
        for e in [
            Error::NotFound("q".into()),
            Error::Invalid("x".into()),
            Error::Io("x".into()),
            Error::Cancelled,
        ] {
            assert!(!retry_after(&e, 0), "{e:?}");
        }
    }

    #[tokio::test]
    async fn one_retry_succeeds_second_time_after_protocol_error() {
        let calls = std::cell::Cell::new(0u32);
        let out = with_one_retry(|attempt| {
            calls.set(calls.get() + 1);
            async move {
                if attempt == 0 {
                    Err(Error::Protocol("link closed".into()))
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(out.unwrap(), 7);
        assert_eq!(calls.get(), 2);
    }

    #[tokio::test]
    async fn one_retry_skips_forbidden() {
        let calls = std::cell::Cell::new(0u32);
        let out: Result<()> = with_one_retry(|_| {
            calls.set(calls.get() + 1);
            async {
                Err(Error::Forbidden {
                    needed: None,
                    detail: "x".into(),
                })
            }
        })
        .await;
        assert!(matches!(out, Err(Error::Forbidden { .. })));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn one_retry_returns_second_error_and_calls_at_most_twice() {
        let calls = std::cell::Cell::new(0u32);
        let out: Result<()> = with_one_retry(|attempt| {
            calls.set(calls.get() + 1);
            async move {
                if attempt == 0 {
                    Err(Error::Auth("first".into()))
                } else {
                    Err(Error::Network("second".into()))
                }
            }
        })
        .await;
        assert!(matches!(out, Err(Error::Network(m)) if m == "second"));
        assert_eq!(calls.get(), 2);
    }
}
