//! Management requests of the message plane: schedule, cancel scheduled,
//! renew lock, receive by sequence number, update disposition and session
//! state. Builders return the request body; parsers take the response.
//! Shapes: "AMQP 1.0 in Azure Service Bus: request-response operations"
//! (Microsoft Learn), cross-checked with the MIT `azservicebus` crate.

use super::management::MgmtResponse;
use super::message::{PeekedMessage, decode_bytes, timestamp_to_datetime};
use crate::{Error, Result};
use fe2o3_amqp::types::primitives::{Array, Binary, OrderedMap, Value};
use time::OffsetDateTime;
use uuid::Uuid;

/// Schedule messages for later delivery.
pub const SCHEDULE: &str = "com.microsoft:schedule-message";
/// Cancel scheduled messages by sequence number.
pub const CANCEL_SCHEDULED: &str = "com.microsoft:cancel-scheduled-message";
/// Renew message locks.
pub const RENEW_LOCK: &str = "com.microsoft:renew-lock";
/// Receive (deferred) messages by sequence number.
pub const RECEIVE_BY_SEQUENCE: &str = "com.microsoft:receive-by-sequence-number";
/// Settle peek-lock deliveries by lock token, with a status answer.
pub const UPDATE_DISPOSITION: &str = "com.microsoft:update-disposition";
/// Read a session's state.
pub const GET_SESSION_STATE: &str = "com.microsoft:get-session-state";
/// Set or clear a session's state.
pub const SET_SESSION_STATE: &str = "com.microsoft:set-session-state";
/// Renew a session lock.
pub const RENEW_SESSION_LOCK: &str = "com.microsoft:renew-session-lock";

/// The settlement an `update-disposition` request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispositionStatus {
    /// Remove the message (complete).
    Completed,
    /// Unlock the message; its delivery count goes up.
    Abandoned,
    /// Move the message to the dead-letter queue.
    Suspended,
    /// Defer the message; it is then received by sequence number only.
    Defered,
}

impl DispositionStatus {
    /// The broker's spelling (`defered` with one `r`, as the service expects).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
            Self::Suspended => "suspended",
            Self::Defered => "defered",
        }
    }
}

fn key(k: &str) -> Value {
    Value::String(k.to_string())
}

fn long_array(seqs: &[i64]) -> Value {
    Value::Array(Array(seqs.iter().map(|s| Value::Long(*s)).collect()))
}

fn uuid_array(tokens: &[Uuid]) -> Value {
    Value::Array(Array(
        tokens.iter().map(|t| Value::Uuid((*t).into())).collect(),
    ))
}

fn insert_session(body: &mut OrderedMap<String, Value>, session: Option<&str>) {
    if let Some(s) = session {
        body.insert("session-id".to_string(), Value::String(s.to_string()));
    }
}

fn session_body(session: &str) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    insert_session(&mut body, Some(session));
    body
}

/// Accept 200 (with a body to read) or 204 (`Ok(None)`: nothing); any other
/// status is mapped with [`MgmtResponse::error`].
fn success_body<'a>(
    resp: &'a MgmtResponse,
    op: &str,
) -> Result<Option<&'a OrderedMap<Value, Value>>> {
    match resp.status {
        204 => Ok(None),
        200 => match &resp.body {
            Value::Map(m) => Ok(Some(m)),
            other => Err(Error::Protocol(format!(
                "{op}: unexpected response body {other:?}"
            ))),
        },
        _ => Err(resp.error(op)),
    }
}

/// A list or array field's items; a missing or other-typed field is a protocol error.
fn items<'a>(map: &'a OrderedMap<Value, Value>, field: &str, op: &str) -> Result<Vec<&'a Value>> {
    match map.get(&key(field)) {
        Some(Value::List(list)) => Ok(list.iter().collect()),
        Some(Value::Array(array)) => Ok(array.0.iter().collect()),
        other => Err(Error::Protocol(format!(
            "{op}: expected a `{field}` list, got {other:?}"
        ))),
    }
}

fn as_datetime(v: &Value, op: &str) -> Result<OffsetDateTime> {
    match v {
        Value::Timestamp(t) => timestamp_to_datetime(t)
            .ok_or_else(|| Error::Protocol(format!("{op}: expiration out of range"))),
        other => Err(Error::Protocol(format!(
            "{op}: expected a timestamp, got {other:?}"
        ))),
    }
}

/// One message to schedule: `(encoded AMQP message, message id, session id,
/// partition key)`.
pub type ScheduleEntry = (Vec<u8>, String, Option<String>, Option<String>);

/// Request body for `schedule-message`: one map per [`ScheduleEntry`].
pub fn schedule_request(messages: &[ScheduleEntry]) -> OrderedMap<String, Value> {
    let list = messages
        .iter()
        .map(|(bytes, id, session, partition)| {
            let mut m = OrderedMap::new();
            m.insert(key("message-id"), Value::String(id.clone()));
            if let Some(s) = session {
                m.insert(key("session-id"), Value::String(s.clone()));
            }
            if let Some(p) = partition {
                m.insert(key("partition-key"), Value::String(p.clone()));
            }
            m.insert(key("message"), Value::Binary(Binary::from(bytes.clone())));
            Value::Map(m)
        })
        .collect();
    let mut body = OrderedMap::new();
    body.insert("messages".to_string(), Value::List(list));
    body
}

/// The sequence numbers the broker gave the scheduled messages, in request order.
pub fn parse_schedule_response(resp: MgmtResponse) -> Result<Vec<i64>> {
    let Some(map) = success_body(&resp, SCHEDULE)? else {
        return Ok(Vec::new());
    };
    items(map, "sequence-numbers", SCHEDULE)?
        .into_iter()
        .map(|v| match v {
            Value::Long(n) => Ok(*n),
            other => Err(Error::Protocol(format!(
                "{SCHEDULE}: unexpected sequence number {other:?}"
            ))),
        })
        .collect()
}

/// Request body for `cancel-scheduled-message`.
pub fn cancel_scheduled_request(seqs: &[i64]) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert("sequence-numbers".to_string(), long_array(seqs));
    body
}

/// A response that only carries a status: 200 and 204 succeed.
pub fn parse_status_only(resp: MgmtResponse, op: &str) -> Result<()> {
    match resp.status {
        200 | 204 => Ok(()),
        _ => Err(resp.error(op)),
    }
}

/// Request body for `renew-lock`.
pub fn renew_lock_request(tokens: &[Uuid]) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert("lock-tokens".to_string(), uuid_array(tokens));
    body
}

/// The new lock expirations, in token order.
pub fn parse_renew_lock_response(resp: MgmtResponse) -> Result<Vec<OffsetDateTime>> {
    let Some(map) = success_body(&resp, RENEW_LOCK)? else {
        return Ok(Vec::new());
    };
    items(map, "expirations", RENEW_LOCK)?
        .into_iter()
        .map(|v| as_datetime(v, RENEW_LOCK))
        .collect()
}

/// Request body for `receive-by-sequence-number`: peek-lock (`1`) or
/// receive-and-delete (`0`), inside `session` for session entities.
pub fn receive_by_sequence_request(
    seqs: &[i64],
    peek_lock: bool,
    session: Option<&str>,
) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert("sequence-numbers".to_string(), long_array(seqs));
    body.insert(
        "receiver-settle-mode".to_string(),
        Value::Uint(u32::from(peek_lock)),
    );
    insert_session(&mut body, session);
    body
}

/// The received messages with their lock tokens. `204` means none of the
/// sequence numbers exist. A missing `lock-token` (receive-and-delete) is the
/// nil uuid.
pub fn parse_receive_by_sequence_response(
    resp: MgmtResponse,
) -> Result<Vec<(PeekedMessage, Uuid)>> {
    let Some(map) = success_body(&resp, RECEIVE_BY_SEQUENCE)? else {
        return Ok(Vec::new());
    };
    items(map, "messages", RECEIVE_BY_SEQUENCE)?
        .into_iter()
        .map(|item| {
            let Value::Map(entry) = item else {
                return Err(Error::Protocol(format!(
                    "{RECEIVE_BY_SEQUENCE}: unexpected entry {item:?}"
                )));
            };
            let message = match entry.get(&key("message")) {
                Some(Value::Binary(bytes)) => decode_bytes(bytes)?,
                _ => {
                    return Err(Error::Protocol(format!(
                        "{RECEIVE_BY_SEQUENCE}: entry without `message` binary"
                    )));
                }
            };
            let token = match entry.get(&key("lock-token")) {
                Some(Value::Uuid(u)) => Uuid::from(u.clone()),
                None | Some(Value::Null) => Uuid::nil(),
                Some(other) => {
                    return Err(Error::Protocol(format!(
                        "{RECEIVE_BY_SEQUENCE}: unexpected lock token {other:?}"
                    )));
                }
            };
            Ok((message, token))
        })
        .collect()
}

/// Request body for `update-disposition`; `dead_letter` is `(reason,
/// description)` for [`DispositionStatus::Suspended`].
pub fn update_disposition_request(
    tokens: &[Uuid],
    status: DispositionStatus,
    dead_letter: Option<(&str, &str)>,
    session: Option<&str>,
) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert(
        "disposition-status".to_string(),
        Value::String(status.as_str().to_string()),
    );
    body.insert("lock-tokens".to_string(), uuid_array(tokens));
    if let Some((reason, description)) = dead_letter {
        body.insert(
            "deadletter-reason".to_string(),
            Value::String(reason.to_string()),
        );
        body.insert(
            "deadletter-description".to_string(),
            Value::String(description.to_string()),
        );
    }
    insert_session(&mut body, session);
    body
}

/// Request body for `get-session-state`.
pub fn get_session_state_request(session: &str) -> OrderedMap<String, Value> {
    session_body(session)
}

/// The session's state bytes; `None` when it has none.
pub fn parse_session_state_response(resp: MgmtResponse) -> Result<Option<Vec<u8>>> {
    let Some(map) = success_body(&resp, GET_SESSION_STATE)? else {
        return Ok(None);
    };
    match map.get(&key("session-state")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Binary(bytes)) => Ok(Some(bytes.to_vec())),
        Some(other) => Err(Error::Protocol(format!(
            "{GET_SESSION_STATE}: unexpected session state {other:?}"
        ))),
    }
}

/// Request body for `set-session-state`; `None` clears the state.
pub fn set_session_state_request(session: &str, state: Option<&[u8]>) -> OrderedMap<String, Value> {
    let mut body = session_body(session);
    body.insert(
        "session-state".to_string(),
        state.map_or(Value::Null, |s| Value::Binary(Binary::from(s.to_vec()))),
    );
    body
}

/// Request body for `renew-session-lock`.
pub fn renew_session_lock_request(session: &str) -> OrderedMap<String, Value> {
    session_body(session)
}

/// The session lock's new expiration.
pub fn parse_renew_session_lock_response(resp: MgmtResponse) -> Result<OffsetDateTime> {
    let Some(map) = success_body(&resp, RENEW_SESSION_LOCK)? else {
        return Err(Error::Protocol(format!(
            "{RENEW_SESSION_LOCK}: no expiration in the response"
        )));
    };
    match map.get(&key("expiration")) {
        Some(v) => as_datetime(v, RENEW_SESSION_LOCK),
        None => Err(Error::Protocol(format!(
            "{RENEW_SESSION_LOCK}: no expiration in the response"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::messaging::message::__private::Serializable;
    use fe2o3_amqp::types::messaging::{Message, MessageAnnotations};
    use fe2o3_amqp::types::primitives::Timestamp;
    use serde_bytes::ByteBuf;

    fn resp(status: u16, body: Value) -> MgmtResponse {
        MgmtResponse {
            status,
            description: None,
            condition: None,
            body,
        }
    }

    fn map<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::Map(OrderedMap::from_iter(
            entries.into_iter().map(|(k, v)| (key(k), v)),
        ))
    }

    #[test]
    fn schedule_request_wraps_each_encoded_message() {
        let body = schedule_request(&[(vec![1, 2], "m1".into(), Some("s".into()), None)]);
        let Some(Value::List(items)) = body.get("messages") else {
            panic!()
        };
        let Value::Map(m) = &items[0] else { panic!() };
        assert_eq!(
            m.get(&Value::String("message-id".into())),
            Some(&Value::String("m1".into()))
        );
        assert_eq!(
            m.get(&Value::String("session-id".into())),
            Some(&Value::String("s".into()))
        );
        assert!(m.get(&Value::String("partition-key".into())).is_none());
        assert!(
            matches!(m.get(&Value::String("message".into())), Some(Value::Binary(b)) if b.as_slice() == [1, 2])
        );
    }

    #[test]
    fn schedule_response_gives_sequence_numbers() {
        let r = resp(
            200,
            map([(
                "sequence-numbers",
                Value::Array(Array(vec![Value::Long(5), Value::Long(6)])),
            )]),
        );
        assert_eq!(parse_schedule_response(r).unwrap(), [5, 6]);
        assert!(
            parse_schedule_response(resp(204, Value::Null))
                .unwrap()
                .is_empty()
        );
        assert!(parse_schedule_response(resp(200, map([]))).is_err());
        assert!(matches!(
            parse_schedule_response(resp(404, Value::Null)),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn cancel_and_status_only() {
        let b = cancel_scheduled_request(&[3, 4]);
        assert!(
            matches!(b.get("sequence-numbers"), Some(Value::Array(a)) if a.0 == vec![Value::Long(3), Value::Long(4)])
        );
        assert!(parse_status_only(resp(200, Value::Null), CANCEL_SCHEDULED).is_ok());
        assert!(parse_status_only(resp(204, Value::Null), CANCEL_SCHEDULED).is_ok());
        assert!(matches!(
            parse_status_only(resp(410, Value::Null), UPDATE_DISPOSITION),
            Err(Error::LockLost(_))
        ));
    }

    #[test]
    fn renew_lock_sends_tokens_and_reads_expirations() {
        let t = Uuid::from_u128(7);
        let b = renew_lock_request(&[t]);
        assert!(
            matches!(b.get("lock-tokens"), Some(Value::Array(a)) if a.0 == vec![Value::Uuid(t.into())])
        );
        let ms = 1_700_000_000_000i64;
        let r = resp(
            200,
            map([(
                "expirations",
                Value::Array(Array(vec![Value::Timestamp(Timestamp::from(ms))])),
            )]),
        );
        let got = parse_renew_lock_response(r).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].unix_timestamp(), ms / 1000);
        assert!(
            parse_renew_lock_response(resp(204, Value::Null))
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            parse_renew_lock_response(resp(410, Value::Null)),
            Err(Error::LockLost(_))
        ));
    }

    #[test]
    fn receive_by_sequence_request_shape() {
        let b = receive_by_sequence_request(&[9], true, Some("s"));
        assert!(
            matches!(b.get("sequence-numbers"), Some(Value::Array(a)) if a.0 == vec![Value::Long(9)])
        );
        assert_eq!(b.get("receiver-settle-mode"), Some(&Value::Uint(1)));
        assert_eq!(b.get("session-id"), Some(&Value::String("s".into())));
        let d = receive_by_sequence_request(&[9], false, None);
        assert_eq!(d.get("receiver-settle-mode"), Some(&Value::Uint(0)));
        assert!(d.get("session-id").is_none());
    }

    #[test]
    fn update_disposition_uses_the_broker_spelling_and_reason() {
        let b = update_disposition_request(
            &[Uuid::nil()],
            DispositionStatus::Suspended,
            Some(("Bad", "why")),
            None,
        );
        assert_eq!(
            b.get("disposition-status"),
            Some(&Value::String("suspended".into()))
        );
        assert_eq!(
            b.get("deadletter-reason"),
            Some(&Value::String("Bad".into()))
        );
        assert_eq!(
            b.get("deadletter-description"),
            Some(&Value::String("why".into()))
        );
        assert!(b.get("session-id").is_none());
        assert_eq!(
            update_disposition_request(&[], DispositionStatus::Defered, None, None)
                .get("disposition-status"),
            Some(&Value::String("defered".into()))
        );
        let c = update_disposition_request(&[], DispositionStatus::Completed, None, Some("S"));
        assert_eq!(c.get("session-id"), Some(&Value::String("S".into())));
        assert!(c.get("deadletter-reason").is_none());
        assert_eq!(DispositionStatus::Abandoned.as_str(), "abandoned");
    }

    #[test]
    fn receive_by_sequence_returns_messages_with_lock_tokens() {
        let msg = Message::builder()
            .message_annotations(
                MessageAnnotations::builder()
                    .insert("x-opt-sequence-number", 9i64)
                    .build(),
            )
            .data(b"x".to_vec())
            .build();
        let bytes = serde_amqp::to_vec(&Serializable(msg)).unwrap();
        let token = Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);
        let entry = Value::Map(OrderedMap::from_iter([
            (
                Value::String("message".into()),
                Value::Binary(ByteBuf::from(bytes)),
            ),
            (
                Value::String("lock-token".into()),
                Value::Uuid(token.into()),
            ),
        ]));
        let got = parse_receive_by_sequence_response(resp(
            200,
            map([("messages", Value::List(vec![entry]))]),
        ))
        .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.sequence_number, 9);
        assert_eq!(got[0].1, token);
        assert!(
            parse_receive_by_sequence_response(resp(204, map([])))
                .unwrap()
                .is_empty(),
            "204: none of them exist"
        );
        assert!(
            parse_receive_by_sequence_response(resp(200, map([("messages", Value::Null)])))
                .is_err(),
            "a malformed body is a protocol error"
        );
    }

    #[test]
    fn session_state_null_is_none_and_bytes_are_kept() {
        assert_eq!(
            parse_session_state_response(resp(200, map([("session-state", Value::Null)]))).unwrap(),
            None
        );
        assert_eq!(
            parse_session_state_response(resp(
                200,
                map([("session-state", Value::Binary(ByteBuf::from(vec![7])))])
            ))
            .unwrap(),
            Some(vec![7])
        );
        assert_eq!(
            parse_session_state_response(resp(204, Value::Null)).unwrap(),
            None
        );
        assert!(matches!(
            set_session_state_request("s", None).get("session-state"),
            Some(Value::Null)
        ));
        let set = set_session_state_request("s", Some(&[1, 2]));
        assert!(
            matches!(set.get("session-state"), Some(Value::Binary(b)) if b.as_slice() == [1, 2])
        );
        assert_eq!(set.get("session-id"), Some(&Value::String("s".into())));
        assert_eq!(
            get_session_state_request("s").get("session-id"),
            Some(&Value::String("s".into()))
        );
    }

    #[test]
    fn renew_session_lock_reads_the_expiration() {
        assert_eq!(
            renew_session_lock_request("s").get("session-id"),
            Some(&Value::String("s".into()))
        );
        let ms = 1_700_000_000_000i64;
        let r = resp(
            200,
            map([("expiration", Value::Timestamp(Timestamp::from(ms)))]),
        );
        assert_eq!(
            parse_renew_session_lock_response(r)
                .unwrap()
                .unix_timestamp(),
            ms / 1000
        );
        assert!(parse_renew_session_lock_response(resp(200, map([]))).is_err());
        assert!(matches!(
            parse_renew_session_lock_response(resp(410, Value::Null)),
            Err(Error::LockLost(_))
        ));
    }
}
