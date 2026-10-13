//! Request/response over an entity's `$management` node, correlated by
//! message id. Protocol: "AMQP 1.0 in Azure Service Bus: request-response
//! operations" (Microsoft Learn); request shapes cross-checked with the MIT
//! `azservicebus` crate (see NOTICE).

use crate::mgmt::needed_claims;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::messaging::{ApplicationProperties, Body, Message, MessageId, Properties};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Value};
use fe2o3_amqp::{Delivery, Receiver, Sender};
use std::time::Duration;

const SERVER_TIMEOUT_MS: u32 = 60_000;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(65);
/// Bound for establishing links and for settling a sent request.
pub(crate) const SETUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A decoded management response.
#[derive(Debug, Clone)]
pub struct MgmtResponse {
    /// HTTP-like status (200, 204, 401, 404, ...).
    pub status: u16,
    /// `statusDescription`, if present.
    pub description: Option<String>,
    /// AMQP-value body (`Value::Null` when absent).
    pub body: Value,
}

impl MgmtResponse {
    pub(crate) fn from_message(message: Message<Body<Value>>) -> Self {
        let props = message.application_properties.as_ref().map(|p| &p.0);
        let lookup = |a: &str, b: &str| props.and_then(|p| p.get(a).or_else(|| p.get(b)));
        let status = lookup("statusCode", "status-code")
            .and_then(simple_to_u16)
            .unwrap_or(0);
        let description = match lookup("statusDescription", "status-description") {
            Some(SimpleValue::String(s)) => Some(s.clone()),
            _ => None,
        };
        let body = match message.body {
            Body::Value(v) => v.0,
            _ => Value::Null,
        };
        Self {
            status,
            description,
            body,
        }
    }

    /// Map a non-success status to an [`Error`].
    pub fn error(&self, operation: &str) -> Error {
        let detail = self
            .description
            .clone()
            .unwrap_or_else(|| format!("status {}", self.status));
        match self.status {
            401 => Error::Auth(format!("{operation}: {detail}")),
            403 => Error::Forbidden {
                needed: needed_claims(&detail),
                detail,
            },
            404 => Error::NotFound(format!("{operation}: {detail}")),
            429 | 503 => Error::Throttled { retry_after: None },
            _ => Error::Protocol(format!("{operation} returned {}: {detail}", self.status)),
        }
    }
}

fn simple_to_u16(v: &SimpleValue) -> Option<u16> {
    match v {
        SimpleValue::Int(n) => u16::try_from(*n).ok(),
        SimpleValue::Uint(n) => u16::try_from(*n).ok(),
        SimpleValue::Long(n) => u16::try_from(*n).ok(),
        SimpleValue::Ulong(n) => u16::try_from(*n).ok(),
        SimpleValue::Short(n) => u16::try_from(*n).ok(),
        SimpleValue::Ushort(n) => Some(*n),
        _ => None,
    }
}

/// A sender/receiver pair attached to `{address}/$management`.
pub struct ManagementLink {
    sender: Sender,
    receiver: Receiver,
    reply_to: String,
    next_id: u64,
}

impl ManagementLink {
    /// Attach to `{address}/$management` on `session`.
    pub async fn attach(session: &mut SessionHandle<()>, address: &str) -> Result<Self> {
        let node = format!("{address}/$management");
        let reply_to = format!("{address}-mgmt-{}", uuid::Uuid::new_v4().simple());
        let sender = tokio::time::timeout(
            SETUP_TIMEOUT,
            Sender::builder()
                .name(format!("{reply_to}-tx"))
                .target(node.clone())
                .attach(session),
        )
        .await
        .map_err(|_| Error::Timeout(format!("attach sender to {node} timed out")))?
        .map_err(|e| Error::Protocol(format!("attach sender to {node}: {e}")))?;
        let receiver = tokio::time::timeout(
            SETUP_TIMEOUT,
            Receiver::builder()
                .name(format!("{reply_to}-rx"))
                .source(node.clone())
                .target(reply_to.clone())
                .credit_mode(CreditMode::Auto(10))
                .attach(session),
        )
        .await
        .map_err(|_| Error::Timeout(format!("attach receiver to {node} timed out")))?
        .map_err(|e| Error::Protocol(format!("attach receiver to {node}: {e}")))?;
        Ok(Self {
            sender,
            receiver,
            reply_to,
            next_id: 1,
        })
    }

    /// Send `operation` with a map body and wait for its correlated response.
    pub async fn call(
        &mut self,
        operation: &str,
        body: OrderedMap<String, Value>,
    ) -> Result<MgmtResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let mut app = OrderedMap::new();
        app.insert(
            "operation".to_string(),
            SimpleValue::String(operation.to_string()),
        );
        app.insert(
            "com.microsoft:server-timeout".to_string(),
            SimpleValue::Uint(SERVER_TIMEOUT_MS),
        );
        let request = Message::builder()
            .properties(
                Properties::builder()
                    .message_id(MessageId::Ulong(id))
                    .reply_to(self.reply_to.clone())
                    .build(),
            )
            .application_properties(ApplicationProperties(app))
            .value(body)
            .build();
        let outcome = tokio::time::timeout(SETUP_TIMEOUT, self.sender.send(request))
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "{operation}: request was not settled within {}s",
                    SETUP_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| Error::Protocol(format!("{operation}: send failed: {e}")))?;
        outcome
            .accepted_or_else(|o| o)
            .map_err(|o| Error::Protocol(format!("{operation}: request not accepted: {o:?}")))?;

        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        loop {
            let delivery: Delivery<Body<Value>> =
                tokio::time::timeout_at(deadline, self.receiver.recv())
                    .await
                    .map_err(|_| {
                        Error::Timeout(format!(
                            "{operation} got no response within {}s",
                            CLIENT_TIMEOUT.as_secs()
                        ))
                    })?
                    .map_err(|e| Error::Protocol(format!("{operation}: receive failed: {e}")))?;
            self.receiver
                .accept(&delivery)
                .await
                .map_err(|e| Error::Protocol(format!("{operation}: settle response: {e}")))?;
            let message = delivery.into_message();
            let correlated = message
                .properties
                .as_ref()
                .and_then(|p| p.correlation_id.as_ref())
                .is_some_and(|c| *c == MessageId::Ulong(id));
            if correlated {
                return Ok(MgmtResponse::from_message(message));
            }
            tracing::warn!(operation, "skipping uncorrelated management response");
        }
    }

    /// Detach both links (best effort).
    pub async fn close(self) {
        let _ = self.sender.close().await;
        let _ = self.receiver.close().await;
    }
}
