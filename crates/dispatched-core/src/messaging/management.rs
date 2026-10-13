//! Request/response over an entity's `$management` node, correlated by
//! message id. Protocol: "AMQP 1.0 in Azure Service Bus: request-response
//! operations" (Microsoft Learn); request shapes cross-checked with the MIT
//! `azservicebus` crate (see NOTICE).

use crate::mgmt::needed_claims;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::link::{ReceiverAttachError, SenderAttachError};
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::definitions::{AmqpError, Error as AmqpRemoteError, ErrorCondition};
use fe2o3_amqp::types::messaging::{ApplicationProperties, Body, Message, MessageId, Properties};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Value};
use fe2o3_amqp::{Delivery, Receiver, Sender};
use fe2o3_amqp_management::error::Error as MgmtError;
use std::time::Duration;

const SERVER_TIMEOUT_MS: u32 = 60_000;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(65);
/// Bound for establishing links and for settling a sent request.
pub(crate) const SETUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Map an error the peer sent when refusing a link or operation, by its AMQP
/// condition: `amqp:unauthorized-access` is a missing right (`Forbidden`),
/// `amqp:not-found` is a missing entity (`NotFound(address)`); anything else
/// stays a `Protocol` error.
pub(crate) fn map_remote_error(what: &str, address: &str, err: &AmqpRemoteError) -> Error {
    let detail = err
        .description
        .clone()
        .unwrap_or_else(|| format!("{:?}", err.condition));
    match &err.condition {
        ErrorCondition::AmqpError(AmqpError::UnauthorizedAccess) => Error::Forbidden {
            needed: needed_claims(&detail),
            detail,
        },
        ErrorCondition::AmqpError(AmqpError::NotFound) => Error::NotFound(address.to_string()),
        ErrorCondition::Custom(sym) if sym.as_str() == "com.microsoft:entity-not-found" => {
            Error::NotFound(address.to_string())
        }
        _ => Error::Protocol(format!("{what}: {detail}")),
    }
}

/// Map a failed sender attach to `{address}`'s node (see [`map_remote_error`]).
pub(crate) fn map_sender_attach_error(node: &str, address: &str, e: SenderAttachError) -> Error {
    match &e {
        SenderAttachError::RemoteClosedWithError(err) => {
            map_remote_error(&format!("attach sender to {node}"), address, err)
        }
        _ => Error::Protocol(format!("attach sender to {node}: {e}")),
    }
}

/// Map a failed receiver attach to `{address}`'s node (see [`map_remote_error`]).
pub(crate) fn map_receiver_attach_error(
    node: &str,
    address: &str,
    e: ReceiverAttachError,
) -> Error {
    match &e {
        ReceiverAttachError::RemoteClosedWithError(err) => {
            map_remote_error(&format!("attach receiver to {node}"), address, err)
        }
        _ => Error::Protocol(format!("attach receiver to {node}: {e}")),
    }
}

/// Map a failed CBS `put-token`. A status error is the service judging the
/// token: 401/403 with named claims is `Forbidden`, otherwise the credential
/// was rejected (`Auth`); 404 is a missing entity. Transport failures are
/// `Protocol` so the explorer drops the connection.
pub(crate) fn map_put_token_error(audience: &str, address: &str, e: MgmtError) -> Error {
    match &e {
        MgmtError::Status(status) => {
            let detail = status
                .description
                .clone()
                .unwrap_or_else(|| format!("status {}", status.code.0));
            match status.code.0.get() {
                401 | 403 => match crate::mgmt::unauthorized(detail.clone()) {
                    Error::Auth(_) => {
                        Error::Auth(format!("token for {audience} was rejected: {detail}"))
                    }
                    forbidden => forbidden,
                },
                404 => Error::NotFound(address.to_string()),
                _ => Error::Protocol(format!(
                    "putting the token for {audience} failed ({}): {detail}",
                    status.code.0
                )),
            }
        }
        _ => Error::Protocol(format!("putting the token for {audience}: {e}")),
    }
}

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
            401 => crate::mgmt::unauthorized(format!("{operation}: {detail}")),
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
        .map_err(|e| map_sender_attach_error(&node, address, e))?;
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
        .map_err(|e| map_receiver_attach_error(&node, address, e))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::primitives::Symbol;
    use fe2o3_amqp_management::error::StatusError;
    use fe2o3_amqp_management::status::StatusCode;
    use std::num::NonZeroU16;

    fn remote(condition: impl Into<ErrorCondition>, description: Option<&str>) -> AmqpRemoteError {
        AmqpRemoteError::new(condition, description.map(str::to_string), None)
    }

    #[test]
    fn unauthorized_access_condition_is_forbidden_with_needed_claims() {
        let err = remote(
            AmqpError::UnauthorizedAccess,
            Some("Unauthorized access. 'Listen' claim(s) are required to perform this operation."),
        );
        match map_remote_error("attach receiver", "orders", &err) {
            Error::Forbidden { needed, detail } => {
                assert_eq!(needed.as_deref(), Some("Listen"));
                assert!(detail.contains("Unauthorized access"));
            }
            other => panic!("expected Forbidden, got {other}"),
        }
    }

    #[test]
    fn unauthorized_access_without_claims_is_forbidden_without_needed() {
        let err = remote(AmqpError::UnauthorizedAccess, None);
        assert!(matches!(
            map_remote_error("x", "orders", &err),
            Error::Forbidden { needed: None, .. }
        ));
    }

    #[test]
    fn not_found_condition_names_the_address() {
        let err = remote(
            AmqpError::NotFound,
            Some("The messaging entity could not be found"),
        );
        assert!(matches!(
            map_remote_error("x", "orders/$DeadLetterQueue", &err),
            Error::NotFound(a) if a == "orders/$DeadLetterQueue"
        ));
        let custom = remote(
            ErrorCondition::Custom(Symbol::from("com.microsoft:entity-not-found")),
            None,
        );
        assert!(matches!(
            map_remote_error("x", "orders", &custom),
            Error::NotFound(_)
        ));
    }

    #[test]
    fn other_conditions_stay_protocol() {
        let err = remote(AmqpError::InternalError, Some("boom"));
        assert!(matches!(
            map_remote_error("attach sender to q/$management", "q", &err),
            Error::Protocol(m) if m == "attach sender to q/$management: boom"
        ));
    }

    #[test]
    fn attach_errors_map_by_remote_condition() {
        let denied = SenderAttachError::RemoteClosedWithError(remote(
            AmqpError::UnauthorizedAccess,
            Some("'Manage' claim(s) are required"),
        ));
        assert!(matches!(
            map_sender_attach_error("q/$management", "q", denied),
            Error::Forbidden { needed: Some(n), .. } if n == "Manage"
        ));
        let missing = ReceiverAttachError::RemoteClosedWithError(remote(AmqpError::NotFound, None));
        assert!(matches!(
            map_receiver_attach_error("q/$management", "q", missing),
            Error::NotFound(a) if a == "q"
        ));
        assert!(matches!(
            map_receiver_attach_error("q/$management", "q", ReceiverAttachError::IllegalState),
            Error::Protocol(_)
        ));
        assert!(matches!(
            map_sender_attach_error("q/$management", "q", SenderAttachError::IllegalState),
            Error::Protocol(_)
        ));
    }

    fn status(code: u16, description: &str) -> MgmtError {
        MgmtError::Status(StatusError {
            code: StatusCode(NonZeroU16::new(code).unwrap()),
            description: Some(description.to_string()),
        })
    }

    #[test]
    fn put_token_status_errors_are_mapped() {
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", status(401, "InvalidSignature: bad token")),
            Error::Auth(m) if m.contains("InvalidSignature")
        ));
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", status(401, "'Listen' claim(s) are required")),
            Error::Forbidden { needed: Some(n), .. } if n == "Listen"
        ));
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", status(403, "denied")),
            Error::Auth(_)
        ));
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", status(404, "gone")),
            Error::NotFound(a) if a == "q"
        ));
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", status(500, "boom")),
            Error::Protocol(_)
        ));
        assert!(matches!(
            map_put_token_error("sb://h/q", "q", MgmtError::StatusCodeNotFound),
            Error::Protocol(_)
        ));
    }
}
