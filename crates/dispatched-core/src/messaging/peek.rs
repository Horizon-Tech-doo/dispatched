//! `com.microsoft:peek-message`: non-destructive browse by sequence number.

use super::connection::ServiceBusConnection;
use super::management::MgmtResponse;
use super::message::{PeekedMessage, decode_bytes};
use crate::{Error, Result};
use fe2o3_amqp::types::primitives::{OrderedMap, Value};

const OPERATION: &str = "com.microsoft:peek-message";
/// Upper bound per request; larger peeks are paged by the caller.
pub const MAX_PER_REQUEST: u32 = 250;

/// Request body for a peek.
pub fn peek_request(from_sequence: i64, count: i32) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert(
        "from-sequence-number".to_string(),
        Value::Long(from_sequence),
    );
    body.insert("message-count".to_string(), Value::Int(count));
    body
}

/// Decode a peek response. `204` (or no `messages`) means nothing to peek.
pub fn parse_peek_response(resp: MgmtResponse) -> Result<Vec<PeekedMessage>> {
    match resp.status {
        204 => return Ok(Vec::new()),
        200 => {}
        _ => return Err(resp.error(OPERATION)),
    }
    let Value::Map(map) = &resp.body else {
        return Ok(Vec::new());
    };
    let items: Vec<&Value> = match map.get(&Value::String("messages".into())) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::List(list)) => list.iter().collect(),
        Some(Value::Array(array)) => array.0.iter().collect(),
        Some(other) => {
            return Err(Error::Protocol(format!(
                "peek: unexpected `messages` value {other:?}"
            )));
        }
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::Map(entry) => match entry.get(&Value::String("message".into())) {
                Some(Value::Binary(bytes)) => decode_bytes(bytes),
                _ => Err(Error::Protocol(
                    "peek: entry without `message` binary".into(),
                )),
            },
            other => Err(Error::Protocol(format!("peek: unexpected entry {other:?}"))),
        })
        .collect()
}

/// Peek up to `max` messages from `address`, starting at `from_sequence`.
pub async fn peek(
    conn: &mut ServiceBusConnection,
    address: &str,
    from_sequence: i64,
    max: u32,
) -> Result<Vec<PeekedMessage>> {
    let count = i32::try_from(max.clamp(1, MAX_PER_REQUEST)).expect("bounded by MAX_PER_REQUEST");
    let link = conn.management(address).await?;
    let resp = link
        .call(OPERATION, peek_request(from_sequence, count))
        .await?;
    parse_peek_response(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::messaging::message::__private::Serializable;
    use fe2o3_amqp::types::messaging::{Message, MessageAnnotations};
    use serde_bytes::ByteBuf;

    fn wire_message(seq: i64) -> Value {
        let msg = Message::builder()
            .message_annotations(
                MessageAnnotations::builder()
                    .insert("x-opt-sequence-number", seq)
                    .build(),
            )
            .data(format!("m{seq}").into_bytes())
            .build();
        let bytes = serde_amqp::to_vec(&Serializable(msg)).unwrap();
        let mut entry = OrderedMap::new();
        entry.insert(
            Value::String("message".into()),
            Value::Binary(ByteBuf::from(bytes)),
        );
        Value::Map(entry)
    }

    fn response(status: u16, body: Value) -> MgmtResponse {
        MgmtResponse {
            status,
            description: None,
            body,
        }
    }

    #[test]
    fn request_body_shape() {
        let body = peek_request(5, 10);
        assert_eq!(body.get("from-sequence-number"), Some(&Value::Long(5)));
        assert_eq!(body.get("message-count"), Some(&Value::Int(10)));
    }

    #[test]
    fn peek_response_with_list_decodes_messages() {
        let mut map = OrderedMap::new();
        map.insert(
            Value::String("messages".into()),
            Value::List(vec![wire_message(1), wire_message(2)]),
        );
        let msgs = parse_peek_response(response(200, Value::Map(map))).unwrap();
        assert_eq!(
            msgs.iter().map(|m| m.sequence_number).collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn peek_response_204_is_empty() {
        assert!(
            parse_peek_response(response(204, Value::Null))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn peek_response_200_without_messages_is_empty() {
        assert!(
            parse_peek_response(response(200, Value::Map(OrderedMap::new())))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn peek_unauthorized_maps_to_auth() {
        let r = MgmtResponse {
            status: 401,
            description: Some("Unauthorized".into()),
            body: Value::Null,
        };
        assert!(matches!(
            parse_peek_response(r).unwrap_err(),
            Error::Auth(_)
        ));
    }

    #[test]
    fn peek_response_with_empty_or_null_messages_is_empty() {
        for messages in [Value::List(Vec::new()), Value::Null] {
            let mut map = OrderedMap::new();
            map.insert(Value::String("messages".into()), messages);
            assert!(
                parse_peek_response(response(200, Value::Map(map)))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn peek_response_204_with_nonempty_body_is_still_empty() {
        let mut map = OrderedMap::new();
        map.insert(
            Value::String("messages".into()),
            Value::List(vec![wire_message(1)]),
        );
        assert!(
            parse_peek_response(response(204, Value::Map(map)))
                .unwrap()
                .is_empty()
        );
    }
}
