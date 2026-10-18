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
/// `amqp:not-found` is a missing entity (`NotFound(address)`),
/// `com.microsoft:session-cannot-be-locked` is a session held by another
/// receiver (`Conflict`), a lost message or session lock is `LockLost`, and
/// `com.microsoft:timeout` is `Timeout`; anything else stays a `Protocol` error.
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
        ErrorCondition::Custom(sym) if sym.as_str() == TIMEOUT => {
            Error::Timeout(format!("{what}: {detail}"))
        }
        ErrorCondition::Custom(sym) => condition_error(what, sym.as_str(), &detail)
            .unwrap_or_else(|| Error::Protocol(format!("{what}: {detail}"))),
        _ => Error::Protocol(format!("{what}: {detail}")),
    }
}

/// The outcome of a next-available session attach: a `Timeout` means no
/// session was available, which is `Ok(None)`, not an error.
pub(crate) fn next_available_outcome<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::Timeout(_)) => Ok(None),
        Err(e) => Err(e),
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
    /// `errorCondition` (or `error-condition`), if present: Service Bus puts the
    /// error condition there, not in `statusDescription`.
    pub condition: Option<String>,
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
        let condition = match lookup("errorCondition", "error-condition") {
            Some(SimpleValue::String(s)) => Some(s.clone()),
            Some(SimpleValue::Symbol(s)) => Some(s.as_str().to_string()),
            _ => None,
        };
        let body = match message.body {
            Body::Value(v) => v.0,
            _ => Value::Null,
        };
        Self {
            status,
            description,
            condition,
            body,
        }
    }

    /// Map a non-success response to an [`Error`]: the `errorCondition` first,
    /// then the status (410 lock lost, 409 conflict), then the condition names
    /// in the description (for brokers that only put them there), otherwise by
    /// status.
    pub fn error(&self, operation: &str) -> Error {
        let detail = self
            .description
            .clone()
            .unwrap_or_else(|| format!("status {}", self.status));
        if let Some(e) = self
            .condition
            .as_deref()
            .and_then(|c| condition_error(operation, c, &detail))
        {
            return e;
        }
        // 410 and 409 apply to every operation; today only the lock and
        // session operations return them (a missing entity is 404).
        match self.status {
            410 => return lock_lost(operation, &detail),
            409 => return Error::Conflict(format!("{operation}: {detail}")),
            _ => {}
        }
        if let Some(e) = self
            .description
            .as_deref()
            .and_then(|d| condition_error(operation, d, &detail))
        {
            return e;
        }
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

const MESSAGE_LOCK_LOST: &str = "com.microsoft:message-lock-lost";
const SESSION_LOCK_LOST: &str = "com.microsoft:session-lock-lost";
const SESSION_CANNOT_BE_LOCKED: &str = "com.microsoft:session-cannot-be-locked";
const TIMEOUT: &str = "com.microsoft:timeout";

fn lock_lost(operation: &str, detail: &str) -> Error {
    Error::LockLost(format!(
        "{operation}: the lock expired or was lost ({detail}); the message is available again, run the operation again"
    ))
}

/// The error for a lock or session condition named in `text` (an
/// `errorCondition`, or a description containing one); `None` for others.
fn condition_error(operation: &str, text: &str, detail: &str) -> Option<Error> {
    if text.contains(MESSAGE_LOCK_LOST) || text.contains(SESSION_LOCK_LOST) {
        Some(lock_lost(operation, detail))
    } else if text.contains(SESSION_CANNOT_BE_LOCKED) {
        Some(Error::Conflict(format!(
            "{operation}: the session is locked by another receiver; stop that receiver or wait for its lock to expire"
        )))
    } else {
        None
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

/// Application properties of a management request.
fn request_properties(
    operation: &str,
    associated_link: Option<&str>,
) -> OrderedMap<String, SimpleValue> {
    let mut app = OrderedMap::new();
    app.insert(
        "operation".to_string(),
        SimpleValue::String(operation.to_string()),
    );
    app.insert(
        "com.microsoft:server-timeout".to_string(),
        SimpleValue::Uint(SERVER_TIMEOUT_MS),
    );
    if let Some(link) = associated_link {
        app.insert(
            "associated-link-name".to_string(),
            SimpleValue::String(link.to_string()),
        );
    }
    app
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
        self.call_for_link(operation, body, None).await
    }

    /// [`call`](Self::call) on behalf of a receiver link: `associated_link`
    /// (the receiver link's name) is sent as `associated-link-name`, which
    /// lock and session operations need.
    pub async fn call_for_link(
        &mut self,
        operation: &str,
        body: OrderedMap<String, Value>,
        associated_link: Option<&str>,
    ) -> Result<MgmtResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let app = request_properties(operation, associated_link);
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
    use fe2o3_amqp::types::messaging::AmqpValue;
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

    // The real carrier for a locked session is the attach refusal.
    #[test]
    fn attach_refusals_map_to_conflict_lock_lost_and_no_session() {
        let remote = |c: &str| {
            AmqpRemoteError::new(
                ErrorCondition::Custom(Symbol::from(c)),
                Some("x".to_string()),
                None,
            )
        };
        assert!(
            matches!(map_remote_error("attach receiver to s", "s", &remote("com.microsoft:session-cannot-be-locked")), Error::Conflict(m) if m.contains("locked by another receiver"))
        );
        assert!(matches!(
            map_remote_error("x", "s", &remote("com.microsoft:message-lock-lost")),
            Error::LockLost(_)
        ));
        assert!(matches!(
            map_remote_error("x", "s", &remote("com.microsoft:session-lock-lost")),
            Error::LockLost(_)
        ));
        let timeout = map_remote_error(
            "attach receiver to s",
            "s",
            &remote("com.microsoft:timeout"),
        );
        assert!(matches!(timeout, Error::Timeout(_)));
        assert!(
            matches!(next_available_outcome::<()>(Err(timeout)), Ok(None)),
            "next-available: no session is not an error"
        );
        assert!(matches!(
            next_available_outcome::<()>(Err(Error::Network("down".into()))),
            Err(Error::Network(_))
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
    fn requests_for_a_link_name_it() {
        let plain = request_properties("com.microsoft:peek-message", None);
        assert_eq!(
            plain.get("operation"),
            Some(&SimpleValue::String("com.microsoft:peek-message".into()))
        );
        assert_eq!(
            plain.get("com.microsoft:server-timeout"),
            Some(&SimpleValue::Uint(SERVER_TIMEOUT_MS))
        );
        assert!(plain.get("associated-link-name").is_none());
        let linked = request_properties("com.microsoft:renew-lock", Some("rx-1"));
        assert_eq!(
            linked.get("associated-link-name"),
            Some(&SimpleValue::String("rx-1".into()))
        );
    }

    #[test]
    fn lock_lost_and_locked_sessions_map_to_their_errors() {
        // The real carrier: the condition is in the `errorCondition` application property.
        let with_condition = |status: u16, condition: &str| {
            let msg = Message::builder()
                .application_properties(
                    ApplicationProperties::builder()
                        .insert("statusCode", status as i32)
                        .insert("statusDescription", "The lock supplied is invalid.")
                        .insert("errorCondition", Symbol::from(condition))
                        .build(),
                )
                .body(Body::Value(AmqpValue(Value::Null)))
                .build();
            MgmtResponse::from_message(msg)
        };
        let lost = with_condition(400, "com.microsoft:message-lock-lost");
        assert_eq!(
            lost.condition.as_deref(),
            Some("com.microsoft:message-lock-lost")
        );
        assert!(
            matches!(lost.error("update-disposition"), Error::LockLost(_)),
            "condition wins over status 400"
        );
        assert!(matches!(
            with_condition(400, "com.microsoft:session-lock-lost").error("renew-session-lock"),
            Error::LockLost(_)
        ));
        assert!(matches!(
            with_condition(400, "com.microsoft:session-cannot-be-locked").error("get-session-state"),
            Error::Conflict(m) if m.contains("locked by another receiver")
        ));
        // Status only, no condition.
        assert!(matches!(
            MgmtResponse {
                status: 410,
                description: Some("gone".into()),
                condition: None,
                body: Value::Null
            }
            .error("renew-lock"),
            Error::LockLost(_)
        ));
        assert!(matches!(
            MgmtResponse {
                status: 409,
                description: None,
                condition: None,
                body: Value::Null
            }
            .error("x"),
            Error::Conflict(_)
        ));
        // Description only, as a last resort.
        assert!(matches!(
            MgmtResponse {
                status: 400,
                description: Some("com.microsoft:message-lock-lost: x".into()),
                condition: None,
                body: Value::Null
            }
            .error("x"),
            Error::LockLost(_)
        ));
        // The condition wins over a 410 or 409 status.
        assert!(matches!(
            with_condition(409, "com.microsoft:message-lock-lost").error("x"),
            Error::LockLost(_)
        ));
        assert!(matches!(
            with_condition(410, "com.microsoft:session-cannot-be-locked").error("x"),
            Error::Conflict(_)
        ));
        // A condition string arrives as a String too; other conditions keep the status mapping.
        let mut app = OrderedMap::new();
        app.insert("statusCode".to_string(), SimpleValue::Int(404));
        app.insert(
            "error-condition".to_string(),
            SimpleValue::String("amqp:not-found".into()),
        );
        let r = MgmtResponse::from_message(
            Message::builder()
                .application_properties(ApplicationProperties(app))
                .body(Body::Value(AmqpValue(Value::Null)))
                .build(),
        );
        assert_eq!(r.condition.as_deref(), Some("amqp:not-found"));
        assert!(matches!(r.error("x"), Error::NotFound(_)));
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
