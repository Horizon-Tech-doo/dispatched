//! Message browsing: streamed peek pages of summaries, one message's detail, filter checks.

use crate::error::CommandError;
use crate::events::{Events, tracked_failures};
use crate::state::AppState;
use dispatched_core::messaging::DETAIL_BODY_CAP;
use dispatched_core::{
    EntityPath, FilterIssue, FilterSpec, MessageDetail, MessageSummary, PeekOptions, PeekPage,
    PeekView,
};
use serde::{Deserialize, Serialize};

/// Most summaries in one page.
pub const PAGE_MAX: u32 = 500;
/// Most summaries in one "from the end" call (it has no continuation).
pub const TAIL_MAX: u32 = 5_000;
/// Most messages one page call examines.
pub const SCAN_PER_CALL: u32 = 5_000;
/// Most messages a "from the end" call examines.
pub const TAIL_BUDGET: u32 = 100_000;

/// Where a peek starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PeekFrom {
    /// The oldest message.
    Start,
    /// This sequence number.
    Sequence { sequence: i64 },
    /// The newest messages ("from the end").
    End,
}

/// One page request from the UI.
#[derive(Debug, Clone, Deserialize)]
pub struct PeekRequest {
    /// `queue` or `topic/subscriptions/name`.
    pub entity: String,
    pub view: PeekView,
    pub from: PeekFrom,
    pub max: u32,
    #[serde(default)]
    pub scan_limit: Option<u32>,
    #[serde(default)]
    pub filter: FilterSpec,
}

/// One page for the grid.
#[derive(Debug, Serialize)]
pub struct PeekPageView {
    pub messages: Vec<MessageSummary>,
    /// Messages examined by this call.
    pub scanned: u32,
    /// The scan limit this call ran with (after clamping). A "from the end" page with fewer
    /// matches than asked and `scanned >= scan_limit` stopped before reaching the oldest
    /// messages: the UI says older ones were not examined.
    pub scan_limit: u32,
    /// Where to continue; always `None` for a "from the end" page.
    pub next_sequence: Option<i64>,
    /// The end of the entity was reached. For a "from the end" page, `false` means the budget
    /// ran out inside the first window, so the matches are not the tail.
    pub exhausted: bool,
}

#[derive(Debug)]
pub struct Prepared {
    pub entity: EntityPath,
    pub view: PeekView,
    pub opts: PeekOptions,
    pub tail: bool,
}

/// Validate and bound a request. Fails (kind `invalid`) before any network work.
pub fn prepare(req: &PeekRequest) -> Result<Prepared, CommandError> {
    let entity = EntityPath::parse(&req.entity)?;
    let filter = req.filter.build()?;
    let tail = req.from == PeekFrom::End;
    let (max_cap, scan_cap) = if tail {
        (TAIL_MAX, TAIL_BUDGET)
    } else {
        (PAGE_MAX, SCAN_PER_CALL)
    };
    let from = match req.from {
        PeekFrom::Sequence { sequence } => sequence.max(0),
        _ => 0,
    };
    let opts = PeekOptions::new(req.max.clamp(1, max_cap))
        .from_sequence(from)
        .scan_limit(req.scan_limit.unwrap_or(scan_cap).clamp(1, scan_cap))
        .filter(filter);
    Ok(Prepared {
        entity,
        view: req.view,
        opts,
        tail,
    })
}

/// The grid's view of `page`, which was read with `scan_limit`.
pub fn page_view(page: &PeekPage, scan_limit: u32) -> PeekPageView {
    PeekPageView {
        messages: page.messages.iter().map(MessageSummary::from).collect(),
        scanned: page.scanned,
        scan_limit,
        next_sequence: page.next_sequence,
        exhausted: page.exhausted,
    }
}

/// One page of `req.view` of `req.entity` in `profile_id`, which must be the connected profile.
pub async fn peek_page(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
    req: PeekRequest,
) -> Result<PeekPageView, CommandError> {
    let p = prepare(&req)?;
    let active = state.active_for(profile_id).await?;
    // A load is up to 20 page calls: only failures are logged, never each page.
    tracked_failures(ev, &format!("Peeking {} failed", p.entity), async {
        let ex = &active.explorer;
        let page = if p.tail {
            ex.peek_tail(&p.entity, p.view, &p.opts).await?
        } else {
            ex.peek(&p.entity, p.view, &p.opts).await?
        };
        Ok(page_view(&page, p.opts.scan_limit))
    })
    .await
}

/// The full message (body capped at [`DETAIL_BODY_CAP`]), or `None` when it is no longer in the view.
/// `session_id` is the session the row was peeked in (a per-session view): the message is read
/// through it, falling back to the whole entity where the broker can't peek by session.
pub async fn message_detail(
    state: &AppState,
    ev: &dyn Events,
    profile_id: &str,
    entity: &str,
    view: PeekView,
    sequence: i64,
    session_id: Option<&str>,
) -> Result<Option<MessageDetail>, CommandError> {
    let session_id = session_id.filter(|s| !s.is_empty());
    let path = EntityPath::parse(entity)?;
    let active = state.active_for(profile_id).await?;
    tracked_failures(
        ev,
        &format!("Reading message {sequence} of {path} failed"),
        async {
            Ok(active
                .explorer
                .peek_message(&path, view, sequence, session_id)
                .await?
                .map(|m| MessageDetail::new(m, DETAIL_BODY_CAP)))
        },
    )
    .await
}

/// Problems with `spec`, one per field; empty when it compiles.
pub fn check_filter(spec: &FilterSpec) -> Vec<FilterIssue> {
    spec.compile().err().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::connection::connect;
    use crate::ops::test_util::{CS, state};
    use dispatched_core::{MessageBody, MessageState, PeekedMessage};

    fn req(from: PeekFrom, max: u32, filter: FilterSpec) -> PeekRequest {
        PeekRequest {
            entity: "orders".into(),
            view: PeekView::DeadLetter,
            from,
            max,
            scan_limit: None,
            filter,
        }
    }

    #[test]
    fn prepare_clamps_sizes_and_budgets() {
        let p = prepare(&req(PeekFrom::Start, 0, FilterSpec::default())).unwrap();
        assert_eq!(
            (p.opts.max, p.opts.scan_limit, p.opts.from_sequence, p.tail),
            (1, SCAN_PER_CALL, 0, false)
        );
        let p = prepare(&req(
            PeekFrom::Sequence { sequence: -5 },
            9_999,
            FilterSpec::default(),
        ))
        .unwrap();
        assert_eq!((p.opts.max, p.opts.from_sequence), (PAGE_MAX, 0));
        let p = prepare(&req(PeekFrom::End, 9_999, FilterSpec::default())).unwrap();
        assert_eq!(
            (p.opts.max, p.opts.scan_limit, p.tail),
            (TAIL_MAX, TAIL_BUDGET, true)
        );
        let mut r = req(PeekFrom::Start, 10, FilterSpec::default());
        r.scan_limit = Some(10_000_000);
        assert_eq!(prepare(&r).unwrap().opts.scan_limit, SCAN_PER_CALL);
    }

    #[test]
    fn prepare_rejects_bad_entities_and_filters_as_invalid() {
        let mut r = req(PeekFrom::Start, 10, FilterSpec::default());
        r.entity = "/".into();
        assert_eq!(prepare(&r).unwrap_err().kind, "invalid");
        let r = req(
            PeekFrom::Start,
            10,
            FilterSpec {
                property: Some("a =".into()),
                ..Default::default()
            },
        );
        let e = prepare(&r).unwrap_err();
        assert_eq!(e.kind, "invalid");
        assert!(e.message.contains("column"), "{}", e.message);
    }

    #[test]
    fn page_view_carries_summaries_not_bodies() {
        let mut m = PeekedMessage {
            sequence_number: 1,
            state: MessageState::Active,
            message_id: None,
            correlation_id: None,
            session_id: None,
            subject: None,
            content_type: None,
            to: None,
            reply_to: None,
            reply_to_session_id: None,
            partition_key: None,
            expires_at: None,
            enqueued_time: None,
            scheduled_enqueue_time: None,
            locked_until: None,
            delivery_count: 0,
            time_to_live: None,
            dead_letter_reason: None,
            dead_letter_description: None,
            dead_letter_source: None,
            application_properties: Default::default(),
            body: MessageBody::Empty,
        };
        m.body = MessageBody::Text("x".repeat(50_000));
        let page = dispatched_core::PeekPage {
            messages: vec![m],
            scanned: 1,
            next_sequence: Some(2),
            exhausted: false,
        };
        let v = page_view(&page, SCAN_PER_CALL);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.len() < 2_000, "{} bytes", json.len());
        assert_eq!(v.next_sequence, Some(2));
    }

    #[test]
    fn page_view_reports_the_effective_scan_limit_so_a_short_tail_can_say_it_stopped_early() {
        let page = dispatched_core::PeekPage {
            messages: vec![],
            scanned: 100_000,
            next_sequence: None,
            exhausted: true,
        };
        let p = prepare(&req(PeekFrom::End, 50, FilterSpec::default())).unwrap();
        let v = page_view(&page, p.opts.scan_limit);
        assert_eq!((v.scanned, v.scan_limit), (100_000, TAIL_BUDGET));
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["scan_limit"], TAIL_BUDGET);
        assert_eq!(json["next_sequence"], serde_json::Value::Null);
    }

    #[test]
    fn check_filter_reports_each_bad_field() {
        let issues = check_filter(&FilterSpec {
            body_regex: Some("(".into()),
            ..Default::default()
        });
        assert_eq!(issues.len(), 1);
        assert!(check_filter(&FilterSpec::default()).is_empty());
        let json = serde_json::to_value(&issues).unwrap();
        assert_eq!(json[0]["field"], "body_regex");
    }

    #[test]
    fn peek_from_and_request_deserialize_in_the_shapes_the_ui_sends() {
        let r: PeekRequest = serde_json::from_value(serde_json::json!({
            "entity": "orders", "view": "dead-letter", "from": {"kind": "sequence", "sequence": 7},
            "max": 100,
        }))
        .unwrap();
        assert_eq!(r.from, PeekFrom::Sequence { sequence: 7 });
        assert_eq!(r.view, PeekView::DeadLetter);
        assert!(r.filter.is_empty() && r.scan_limit.is_none());
        let r: PeekRequest = serde_json::from_value(serde_json::json!({
            "entity": "t/subscriptions/s", "view": "active", "from": {"kind": "end"},
            "max": 5, "scan_limit": 10, "filter": {"body_contains": "x"},
        }))
        .unwrap();
        assert_eq!(r.from, PeekFrom::End);
        assert_eq!(r.scan_limit, Some(10));
    }

    #[tokio::test]
    async fn peek_page_for_another_profile_is_not_connected_and_not_logged() {
        let s = state();
        let ev = Recorder::default();
        let p = s
            .store
            .add_connection_string("A", CS, s.secrets.as_ref(), false)
            .unwrap();
        connect(&s, &ev, &p.id).await.unwrap();
        let before = ev.activity.lock().unwrap().len();
        let e = peek_page(
            &s,
            &ev,
            "someone-else",
            req(PeekFrom::Start, 10, FilterSpec::default()),
        )
        .await
        .unwrap_err();
        assert_eq!(e.kind, "not_connected");
        let e = message_detail(&s, &ev, "someone-else", "orders", PeekView::Active, 1, None)
            .await
            .unwrap_err();
        assert_eq!(e.kind, "not_connected");
        assert_eq!(
            ev.activity.lock().unwrap().len(),
            before,
            "refusals are not engine calls"
        );
    }
}
