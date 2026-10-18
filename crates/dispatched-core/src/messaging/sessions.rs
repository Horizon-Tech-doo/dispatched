//! Sessions of a session-enabled entity: the broker's list plus a peek tally.

use super::connection::ServiceBusConnection;
use super::management::MgmtResponse;
use super::scan::{PeekSource, walk_messages};
use crate::diagnostics::error_kind;
use crate::{Error, Result};
use async_trait::async_trait;
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

/// Session ids the broker listed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrokerSessions {
    /// Ids in the broker's order, at most [`MAX_SESSIONS`].
    pub ids: Vec<String>,
    /// Listing stopped at [`MAX_SESSIONS`] with the last page full, so more may exist.
    pub capped: bool,
}

/// Where session-id pages come from; the seam that lets the paging be tested without a broker.
#[async_trait]
pub trait SessionPages: Send {
    /// Up to `top` ids after the first `skip`.
    async fn page(&mut self, skip: i32, top: i32) -> Result<Vec<String>>;
}

/// Pages from the `get-message-sessions` operation of one entity.
struct BrokerPages<'a> {
    conn: &'a mut ServiceBusConnection,
    address: &'a str,
}

#[async_trait]
impl SessionPages for BrokerPages<'_> {
    async fn page(&mut self, skip: i32, top: i32) -> Result<Vec<String>> {
        let link = self.conn.management(self.address).await?;
        parse_list_response(link.call(OPERATION, list_request(skip, top)).await?)
    }
}

/// Read pages of [`PAGE`] ids until a short page, or [`MAX_SESSIONS`] ids with the last page
/// full (then `capped`).
pub async fn collect_session_ids<P: SessionPages + ?Sized>(
    pages: &mut P,
) -> Result<BrokerSessions> {
    let mut out = BrokerSessions::default();
    loop {
        let skip = i32::try_from(out.ids.len()).unwrap_or(i32::MAX);
        let page = pages.page(skip, PAGE).await?;
        let full = page.len() >= PAGE as usize;
        out.ids.extend(page);
        if out.ids.len() >= MAX_SESSIONS && full {
            out.ids.truncate(MAX_SESSIONS);
            out.capped = true;
            return Ok(out);
        }
        if !full {
            out.ids.truncate(MAX_SESSIONS);
            return Ok(out);
        }
    }
}

/// Every session id the broker lists for `address`, up to [`MAX_SESSIONS`].
pub async fn list_session_ids(
    conn: &mut ServiceBusConnection,
    address: &str,
) -> Result<BrokerSessions> {
    collect_session_ids(&mut BrokerPages { conn, address }).await
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
    /// The broker listed this id.
    pub in_broker: bool,
}

/// What the broker's session list contributed to a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum BrokerList {
    /// The broker answered with at least one id; `capped` when the list was cut at
    /// [`MAX_SESSIONS`].
    Listed {
        /// More sessions may exist than were listed.
        capped: bool,
    },
    /// The broker answered (200 or 204) with no ids. Real brokers list sessions with
    /// messages; the emulator answers this way even when sessions exist.
    Empty,
    /// The broker call failed; the sessions come from the tally alone.
    Unavailable {
        /// Error kind (`diagnostics::ERROR_KINDS`), never the raw message.
        kind: String,
    },
}

/// Sessions of an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionListing {
    /// Sorted by id.
    pub sessions: Vec<SessionSummary>,
    /// What the broker's list contributed.
    pub broker: BrokerList,
    /// Messages the tally examined.
    pub scanned: u64,
    /// The tally hit its cap: message counts are lower bounds, and sessions that only have
    /// messages beyond the cap are missing. The session set may also be incomplete when
    /// `broker` is `empty` or `unavailable`, since then only the tally names sessions.
    pub truncated: bool,
}

/// Combine what the broker and the tally know. A failed broker call is `Err(kind)`.
pub fn merge(
    broker: std::result::Result<BrokerSessions, String>,
    tally: Option<SessionTally>,
) -> SessionListing {
    let counted = tally.is_some();
    let t = tally.unwrap_or_default();
    let (state, listed) = match broker {
        Ok(b) if b.ids.is_empty() => (BrokerList::Empty, Vec::new()),
        Ok(b) => (BrokerList::Listed { capped: b.capped }, b.ids),
        Err(kind) => (BrokerList::Unavailable { kind }, Vec::new()),
    };
    let in_broker: std::collections::BTreeSet<String> = listed.into_iter().collect();
    let mut ids: std::collections::BTreeSet<String> = t.per_session.keys().cloned().collect();
    ids.extend(in_broker.iter().cloned());
    let sessions = ids
        .into_iter()
        .map(|id| {
            let c = t.per_session.get(&id).copied().unwrap_or_default();
            SessionSummary {
                messages: counted.then_some(c.messages),
                first_enqueued: c.first,
                last_enqueued: c.last,
                in_broker: in_broker.contains(&id),
                id,
            }
        })
        .collect();
    SessionListing {
        sessions,
        broker: state,
        scanned: t.scanned,
        truncated: t.truncated,
    }
}

/// The listing from both attempts; an error only when both failed (the broker's).
pub fn combine(
    broker: Result<BrokerSessions>,
    tally: Result<SessionTally>,
) -> Result<SessionListing> {
    match (broker, tally) {
        (Err(b), Err(_)) => Err(b),
        (b, t) => {
            if let Err(e) = &t {
                tracing::debug!(error = %e, "session tally unavailable; using the broker list");
            }
            Ok(merge(b.map_err(|e| error_kind(&e).to_string()), t.ok()))
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
            condition: None,
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

    fn listed(ids: &[&str]) -> BrokerSessions {
        BrokerSessions {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            capped: false,
        }
    }

    #[tokio::test]
    async fn merge_keeps_broker_sessions_without_messages() {
        let mut src = Fake(vec![msg(1, Some("b"), 1)]);
        let tally = tally_sessions(&mut src, "q", 100).await.unwrap();
        let l = merge(Ok(listed(&["c", "b"])), Some(tally));
        assert_eq!(l.broker, BrokerList::Listed { capped: false });
        let ids: Vec<&str> = l.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b", "c"]);
        assert_eq!(l.sessions[0].messages, Some(1));
        assert!(l.sessions[0].in_broker && l.sessions[1].in_broker);
        assert_eq!(
            l.sessions[1].messages,
            Some(0),
            "known to the broker, no messages in the scan"
        );
        let only_broker = merge(Ok(listed(&["x"])), None);
        assert_eq!(only_broker.sessions[0].messages, None, "not counted");
    }

    #[tokio::test]
    async fn the_broker_leg_says_what_it_contributed() {
        let mut src = Fake(vec![msg(1, Some("b"), 1)]);
        // Answered with nothing (the emulator): sessions come from the tally, not the broker.
        let empty = merge(
            Ok(BrokerSessions::default()),
            Some(tally_sessions(&mut src, "q", 100).await.unwrap()),
        );
        assert_eq!(empty.broker, BrokerList::Empty);
        assert_eq!(empty.sessions.len(), 1);
        assert!(!empty.sessions[0].in_broker);
        // Failed: the error kind, not its text.
        let failed = merge(
            Err("forbidden".into()),
            Some(tally_sessions(&mut src, "q", 100).await.unwrap()),
        );
        assert_eq!(
            failed.broker,
            BrokerList::Unavailable {
                kind: "forbidden".into()
            }
        );
        // Cut at the cap.
        let capped = merge(
            Ok(BrokerSessions {
                ids: vec!["a".into()],
                capped: true,
            }),
            None,
        );
        assert_eq!(capped.broker, BrokerList::Listed { capped: true });
    }

    #[test]
    fn the_listing_has_a_stable_json_shape() {
        let l = merge(Ok(listed(&["a"])), Some(SessionTally::default()));
        let j = serde_json::to_value(&l).unwrap();
        assert_eq!(
            j["broker"],
            serde_json::json!({"state": "listed", "capped": false})
        );
        assert_eq!(j["sessions"][0]["in_broker"], true);
        assert_eq!(
            serde_json::to_value(BrokerList::Empty).unwrap(),
            serde_json::json!({"state": "empty"})
        );
        assert_eq!(
            serde_json::to_value(BrokerList::Unavailable {
                kind: "network".into()
            })
            .unwrap(),
            serde_json::json!({"state": "unavailable", "kind": "network"})
        );
    }

    #[test]
    fn list_fails_only_when_both_sources_fail() {
        let broker_err = || Err(Error::Network("down".into()));
        let tally_err = || Err(Error::Protocol("refused".into()));
        let both_ok = combine(Ok(listed(&["a"])), tally_err()).unwrap();
        assert_eq!(both_ok.broker, BrokerList::Listed { capped: false });
        assert_eq!(both_ok.sessions[0].messages, None, "no tally ran");
        let l = combine(broker_err(), Ok(SessionTally::default())).unwrap();
        assert_eq!(
            l.broker,
            BrokerList::Unavailable {
                kind: "network".into()
            }
        );
        assert!(
            matches!(
                combine(broker_err(), tally_err()).unwrap_err(),
                Error::Network(_)
            ),
            "the broker's error wins"
        );
    }

    /// Serves `total` ids and records every request.
    struct Pages {
        total: usize,
        calls: Vec<(i32, i32)>,
    }

    #[async_trait]
    impl SessionPages for Pages {
        async fn page(&mut self, skip: i32, top: i32) -> Result<Vec<String>> {
            self.calls.push((skip, top));
            let skip = skip as usize;
            let end = (skip + top as usize).min(self.total);
            Ok((skip..end.max(skip)).map(|i| format!("s{i}")).collect())
        }
    }

    #[tokio::test]
    async fn paging_advances_skip_and_stops_at_a_short_page() {
        let mut p = Pages {
            total: 250,
            calls: vec![],
        };
        let got = collect_session_ids(&mut p).await.unwrap();
        assert_eq!(got.ids.len(), 250);
        assert!(!got.capped);
        assert_eq!(p.calls, [(0, 100), (100, 100), (200, 100)]);
        // An exact multiple needs one more (empty) page to know it is the end.
        let mut p = Pages {
            total: 200,
            calls: vec![],
        };
        let got = collect_session_ids(&mut p).await.unwrap();
        assert_eq!((got.ids.len(), got.capped), (200, false));
        assert_eq!(p.calls, [(0, 100), (100, 100), (200, 100)]);
        let mut p = Pages {
            total: 0,
            calls: vec![],
        };
        assert!(collect_session_ids(&mut p).await.unwrap().ids.is_empty());
    }

    #[tokio::test]
    async fn paging_stops_at_the_cap_and_says_so() {
        let mut p = Pages {
            total: MAX_SESSIONS + 500,
            calls: vec![],
        };
        let got = collect_session_ids(&mut p).await.unwrap();
        assert_eq!(got.ids.len(), MAX_SESSIONS);
        assert!(got.capped);
        assert_eq!(
            p.calls.len(),
            MAX_SESSIONS / 100,
            "no page beyond the cap is read"
        );
        // Exactly the cap with a full last page may have more behind it.
        let mut p = Pages {
            total: MAX_SESSIONS,
            calls: vec![],
        };
        assert!(collect_session_ids(&mut p).await.unwrap().capped);
    }

    #[tokio::test]
    async fn a_failing_page_fails_the_listing() {
        struct Failing;
        #[async_trait]
        impl SessionPages for Failing {
            async fn page(&mut self, _: i32, _: i32) -> Result<Vec<String>> {
                Err(Error::Timeout("slow".into()))
            }
        }
        assert!(matches!(
            collect_session_ids(&mut Failing).await,
            Err(Error::Timeout(_))
        ));
    }
}
