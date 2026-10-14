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
    pub fn new(ns: Namespace) -> Self {
        let mgmt = ManagementClient::new(&ns);
        Self {
            ns,
            mgmt,
            amqp: Mutex::new(None),
        }
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
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        let conn = guard.as_mut().expect("opened above");
        let result = scan_messages(conn, &address, view, opts).await;
        if result.as_ref().is_err_and(breaks_connection)
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

/// Only transport-level failures poison the cached connection; `Forbidden`,
/// `NotFound` and friends are answers from a healthy service.
fn breaks_connection(e: &Error) -> bool {
    matches!(
        e,
        Error::Network(_) | Error::Protocol(_) | Error::Timeout(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transport_failures_drop_the_connection() {
        assert!(breaks_connection(&Error::Network("x".into())));
        assert!(breaks_connection(&Error::Protocol("x".into())));
        assert!(breaks_connection(&Error::Timeout("x".into())));
        assert!(!breaks_connection(&Error::Forbidden {
            needed: Some("Listen".into()),
            detail: "x".into()
        }));
        assert!(!breaks_connection(&Error::NotFound("q".into())));
        assert!(!breaks_connection(&Error::Auth("x".into())));
    }
}
