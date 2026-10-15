//! Sessions of a session-enabled entity: the broker's list plus a peek tally.

use super::connection::ServiceBusConnection;
use super::management::MgmtResponse;
use super::scan::{PeekSource, walk_messages};
use crate::{Error, Result};
use fe2o3_amqp::types::primitives::{OrderedMap, Timestamp, Value};
use serde::Serialize;
use std::collections::BTreeMap;
use time::OffsetDateTime;

const OPERATION: &str = "com.microsoft:get-message-sessions";
const PAGE: i32 = 100;
/// Most session ids read from the broker.
pub const MAX_SESSIONS: usize = 10_000;

/// Request body: sessions active or with state changed since the Unix epoch (so empty
/// sessions with state are included where the broker allows it).
pub fn list_request(skip: i32, top: i32) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert(
        "last-updated-time".to_string(),
        Value::Timestamp(Timestamp::from(OffsetDateTime::UNIX_EPOCH)),
    );
    body.insert("skip".to_string(), Value::Int(skip));
    body.insert("top".to_string(), Value::Int(top));
    body
}

/// Session ids from a response; `204` means none.
pub fn parse_list_response(resp: MgmtResponse) -> Result<Vec<String>> {
    match resp.status {
        204 => return Ok(Vec::new()),
        200 => {}
        _ => return Err(resp.error(OPERATION)),
    }
    let Value::Map(map) = &resp.body else {
        return Ok(Vec::new());
    };
    let items: Vec<&Value> = match map.get(&Value::String("sessions-ids".into())) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.0.iter().collect(),
        Some(Value::List(l)) => l.iter().collect(),
        Some(other) => {
            return Err(Error::Protocol(format!(
                "sessions: unexpected `sessions-ids` value {other:?}"
            )));
        }
    };
    Ok(items
        .into_iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Symbol(s) => Some(s.0.clone()),
            _ => None,
        })
        .collect())
}

/// Every session id the broker lists for `address`, up to [`MAX_SESSIONS`].
pub async fn list_session_ids(
    conn: &mut ServiceBusConnection,
    address: &str,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    loop {
        let skip = i32::try_from(out.len()).unwrap_or(i32::MAX);
        let link = conn.management(address).await?;
        let page = parse_list_response(link.call(OPERATION, list_request(skip, PAGE)).await?)?;
        let n = page.len();
        out.extend(page);
        if n < PAGE as usize || out.len() >= MAX_SESSIONS {
            out.truncate(MAX_SESSIONS);
            return Ok(out);
        }
    }
}

/// Messages of one session seen by a tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCount {
    /// Messages (any state) in the main queue.
    pub messages: u64,
    /// Earliest enqueued time seen.
    pub first: Option<OffsetDateTime>,
    /// Latest enqueued time seen.
    pub last: Option<OffsetDateTime>,
}

/// Messages per session in the first `cap` messages of an entity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionTally {
    /// By session id.
    pub per_session: BTreeMap<String, SessionCount>,
    /// Messages examined.
    pub scanned: u64,
    /// More messages exist beyond the cap.
    pub truncated: bool,
}

/// Tally messages per session by peeking `address`, examining at most `cap` (plus one to
/// learn whether more exist). Uses the same walker as `count_messages`, so stale and
/// unsequenced messages skip or end the tally exactly as they do a count.
pub async fn tally_sessions<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    cap: u32,
) -> Result<SessionTally> {
    let mut per_session: BTreeMap<String, SessionCount> = BTreeMap::new();
    let walked = walk_messages(src, address, cap, |m| {
        if let Some(id) = &m.session_id {
            let c = per_session.entry(id.clone()).or_default();
            c.messages += 1;
            if let Some(at) = m.enqueued_time {
                c.first = Some(c.first.map_or(at, |f| f.min(at)));
                c.last = Some(c.last.map_or(at, |l| l.max(at)));
            }
        }
    })
    .await?;
    Ok(SessionTally {
        per_session,
        scanned: walked.examined,
        truncated: walked.truncated,
    })
}

/// One session in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionSummary {
    /// Session id.
    pub id: String,
    /// Messages seen by the tally (`None` when no tally ran).
    pub messages: Option<u64>,
    /// Earliest enqueued time seen.
    #[serde(with = "time::serde::rfc3339::option")]
    pub first_enqueued: Option<OffsetDateTime>,
    /// Latest enqueued time seen.
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_enqueued: Option<OffsetDateTime>,
}

/// Sessions of an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionListing {
    /// Sorted by id.
    pub sessions: Vec<SessionSummary>,
    /// The broker's list was available.
    pub from_broker: bool,
    /// Messages the tally examined.
    pub scanned: u64,
    /// The tally hit its cap; message counts are lower bounds.
    pub truncated: bool,
}

/// Combine what the broker and the tally know.
pub fn merge(broker: Option<Vec<String>>, tally: Option<SessionTally>) -> SessionListing {
    let counted = tally.is_some();
    let t = tally.unwrap_or_default();
    let mut ids: std::collections::BTreeSet<String> = t.per_session.keys().cloned().collect();
    let from_broker = broker.is_some();
    ids.extend(broker.unwrap_or_default());
    let sessions = ids
        .into_iter()
        .map(|id| {
            let c = t.per_session.get(&id).copied().unwrap_or_default();
            SessionSummary {
                messages: counted.then_some(c.messages),
                first_enqueued: c.first,
                last_enqueued: c.last,
                id,
            }
        })
        .collect();
    SessionListing {
        sessions,
        from_broker,
        scanned: t.scanned,
        truncated: t.truncated,
    }
}

/// The listing from both attempts; an error only when both failed (the broker's).
pub fn combine(broker: Result<Vec<String>>, tally: Result<SessionTally>) -> Result<SessionListing> {
    match (broker, tally) {
        (Err(b), Err(_)) => Err(b),
        (b, t) => {
            if let Err(e) = &b {
                tracing::debug!(error = %e, "broker session list unavailable; using the peek tally");
            }
            if let Err(e) = &t {
                tracing::debug!(error = %e, "session tally unavailable; using the broker list");
            }
            Ok(merge(b.ok(), t.ok()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{PeekedMessage, test_message};
    use async_trait::async_trait;
    use fe2o3_amqp::types::primitives::Array;

    fn resp(status: u16, body: Value) -> MgmtResponse {
        MgmtResponse {
            status,
            description: None,
            body,
        }
    }

    #[test]
    fn request_asks_from_the_epoch_with_paging() {
        let b = list_request(100, 100);
        assert!(
            matches!(b.get("last-updated-time"), Some(Value::Timestamp(t)) if t.milliseconds() == 0)
        );
        assert_eq!(b.get("skip"), Some(&Value::Int(100)));
        assert_eq!(b.get("top"), Some(&Value::Int(100)));
    }

    #[test]
    fn response_ids_come_from_an_array_or_a_list() {
        let mut map = OrderedMap::new();
        map.insert(
            Value::String("sessions-ids".into()),
            Value::Array(Array(vec![
                Value::String("a".into()),
                Value::String("b".into()),
            ])),
        );
        assert_eq!(
            parse_list_response(resp(200, Value::Map(map))).unwrap(),
            ["a", "b"]
        );
        let mut map = OrderedMap::new();
        map.insert(
            Value::String("sessions-ids".into()),
            Value::List(vec![Value::String("c".into())]),
        );
        assert_eq!(
            parse_list_response(resp(200, Value::Map(map))).unwrap(),
            ["c"]
        );
        assert!(
            parse_list_response(resp(204, Value::Null))
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            parse_list_response(resp(403, Value::Null)).unwrap_err(),
            Error::Forbidden { .. }
        ));
    }

    struct Fake(Vec<PeekedMessage>);

    #[async_trait]
    impl PeekSource for Fake {
        async fn peek_batch(
            &mut self,
            _a: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            Ok(self
                .0
                .iter()
                .filter(|m| m.sequence_number >= from)
                .take(max as usize)
                .cloned()
                .collect())
        }
    }

    fn msg(seq: i64, session: Option<&str>, minute: i64) -> PeekedMessage {
        let mut m = test_message(seq);
        m.session_id = session.map(str::to_string);
        m.enqueued_time = Some(time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(minute));
        m
    }

    #[tokio::test]
    async fn tally_counts_per_session_with_first_and_last_times() {
        let mut src = Fake(vec![
            msg(1, Some("b"), 5),
            msg(2, Some("a"), 1),
            msg(3, None, 2),
            msg(4, Some("b"), 9),
        ]);
        let t = tally_sessions(&mut src, "q", 100).await.unwrap();
        assert_eq!(t.scanned, 4);
        assert!(!t.truncated);
        let b = &t.per_session["b"];
        assert_eq!(b.messages, 2);
        assert_eq!(b.first.unwrap().minute(), 5);
        assert_eq!(b.last.unwrap().minute(), 9);
        let capped = tally_sessions(&mut src, "q", 2).await.unwrap();
        assert!(capped.truncated);
    }

    #[tokio::test]
    async fn merge_keeps_broker_sessions_without_messages() {
        let mut src = Fake(vec![msg(1, Some("b"), 1)]);
        let tally = tally_sessions(&mut src, "q", 100).await.unwrap();
        let l = merge(Some(vec!["c".into(), "b".into()]), Some(tally));
        assert!(l.from_broker);
        let ids: Vec<&str> = l.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert_eq!(l.sessions[0].messages, Some(1));
        assert_eq!(
            l.sessions[1].messages,
            Some(0),
            "known to the broker, no messages in the scan"
        );
        let only_tally = merge(
            None,
            Some(tally_sessions(&mut src, "q", 100).await.unwrap()),
        );
        assert!(!only_tally.from_broker);
        let only_broker = merge(Some(vec!["x".into()]), None);
        assert_eq!(only_broker.sessions[0].messages, None, "not counted");
    }

    #[test]
    fn list_fails_only_when_both_sources_fail() {
        let broker_err = || Err(Error::Network("down".into()));
        let tally_err = || Err(Error::Protocol("refused".into()));
        assert!(
            combine(Ok(vec!["a".into()]), tally_err())
                .unwrap()
                .from_broker
        );
        assert!(
            !combine(broker_err(), Ok(SessionTally::default()))
                .unwrap()
                .from_broker
        );
        assert!(
            matches!(
                combine(broker_err(), tally_err()).unwrap_err(),
                Error::Network(_)
            ),
            "the broker's error wins"
        );
    }
}
