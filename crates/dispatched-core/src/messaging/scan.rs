//! Peek through an entity, keeping only messages that match a view and filter.

use super::connection::ServiceBusConnection;
use super::message::{MessageState, PeekedMessage};
use super::peek::{self, MAX_PER_REQUEST};
use crate::filter::MessageFilter;
use crate::namespace::PeekView;
use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::safety::CancellationToken;
use serde::Serialize;
use std::collections::{BTreeSet, VecDeque};

/// Messages examined per peek call unless the caller says otherwise.
pub const DEFAULT_SCAN_LIMIT: u32 = 10_000;

/// Anything that can return a batch of peeked messages.
#[async_trait]
pub trait PeekSource: Send {
    /// Peek up to `max` messages from `address` starting at sequence `from`.
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>>;

    /// Peek within one session. Sources that cannot answer `Unsupported`; callers then peek
    /// the whole entity and keep the session's messages themselves.
    async fn peek_session_batch(
        &mut self,
        _address: &str,
        _session_id: &str,
        _from: i64,
        _max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        Err(Error::Unsupported {
            capability: "peek within a session".into(),
        })
    }
}

#[async_trait]
impl PeekSource for ServiceBusConnection {
    async fn peek_batch(
        &mut self,
        address: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        peek::peek(self, address, from, max).await
    }

    async fn peek_session_batch(
        &mut self,
        address: &str,
        session_id: &str,
        from: i64,
        max: u32,
    ) -> Result<Vec<PeekedMessage>> {
        peek::peek_session(self, address, session_id, from, max).await
    }
}

/// Batches from `address`, within `session` while the source can peek by session.
struct Batches<'a, S: PeekSource + ?Sized> {
    src: &'a mut S,
    address: &'a str,
    session: Option<&'a str>,
}

impl<S: PeekSource + ?Sized> Batches<'_, S> {
    async fn next(&mut self, from: i64, max: u32) -> Result<Vec<PeekedMessage>> {
        if let Some(id) = self.session {
            match self
                .src
                .peek_session_batch(self.address, id, from, max)
                .await
            {
                // Not by session here: peek the whole entity from now on; the session
                // condition of the filter keeps the result correct.
                Err(Error::Unsupported { .. }) => self.session = None,
                other => return other,
            }
        }
        self.src.peek_batch(self.address, from, max).await
    }
}

/// What to peek.
#[derive(Debug, Clone)]
pub struct PeekOptions {
    /// First sequence number to look at.
    pub from_sequence: i64,
    /// Maximum matching messages to return.
    pub max: u32,
    /// Maximum messages to examine (matching or not).
    pub scan_limit: u32,
    /// Property/body filter.
    pub filter: MessageFilter,
}

impl PeekOptions {
    /// Return up to `max` matches from the start, scanning at most [`DEFAULT_SCAN_LIMIT`].
    pub fn new(max: u32) -> Self {
        Self {
            from_sequence: 0,
            max,
            scan_limit: DEFAULT_SCAN_LIMIT,
            filter: MessageFilter::default(),
        }
    }
    /// Start at `seq`.
    pub fn from_sequence(mut self, seq: i64) -> Self {
        self.from_sequence = seq;
        self
    }
    /// Examine at most `limit` messages.
    pub fn scan_limit(mut self, limit: u32) -> Self {
        self.scan_limit = limit.max(1);
        self
    }
    /// Apply `filter`.
    pub fn filter(mut self, filter: MessageFilter) -> Self {
        self.filter = filter;
        self
    }
}

/// One page of results.
#[derive(Debug, Clone, Serialize)]
pub struct PeekPage {
    /// Matching messages, in sequence order.
    pub messages: Vec<PeekedMessage>,
    /// Messages examined.
    pub scanned: u32,
    /// Where to continue, or `None` when the end of the entity was reached.
    pub next_sequence: Option<i64>,
    /// The end of the entity was reached.
    pub exhausted: bool,
}

/// Peek through `address`, keeping messages accepted by `view` and `opts.filter`.
///
/// Stops at `opts.max` matches, `opts.scan_limit` examined messages, an empty
/// batch (the end of the entity), a batch that makes no progress, or the first
/// message without a sequence number (`< 0`).
///
/// Messages whose sequence number is below the current position (duplicates or
/// stale replays) are skipped: they are neither returned nor counted against
/// `scan_limit`.
///
/// `next_sequence` is where to continue. It is `None` when the end of the
/// entity was reached, when the scan made no progress at all (nothing examined,
/// so retrying from the same place would loop), and when `opts.max == 0`
/// (which returns immediately). If a message without a sequence number is met
/// after real progress, the continuation is the last good sequence plus one; the
/// next call then examines nothing and ends with `None`.
///
/// With `opts.filter.session_id` set, batches are peeked within that session (no session
/// lock); a source that cannot do that is asked once and then peeked whole.
pub async fn scan_messages<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    opts: &PeekOptions,
) -> Result<PeekPage> {
    let mut batches = Batches {
        src,
        address,
        session: opts.filter.session_id.as_deref(),
    };
    let mut out: Vec<PeekedMessage> = Vec::new();
    let mut scanned: u32 = 0;
    let mut next = opts.from_sequence;
    let mut exhausted = false;
    'outer: while (out.len() as u32) < opts.max && scanned < opts.scan_limit {
        let want = (opts.scan_limit - scanned).min(MAX_PER_REQUEST);
        let batch = batches.next(next, want).await?;
        if batch.is_empty() {
            exhausted = true;
            break;
        }
        let mut advanced = false;
        for m in batch {
            if m.sequence_number < 0 {
                // No sequence number: we cannot position past it.
                break 'outer;
            }
            if m.sequence_number < next {
                // Already seen or stale.
                continue;
            }
            let wanted = view.accepts(m.state) && opts.filter.matches(&m);
            if wanted && out.len() as u32 >= opts.max {
                // Leave this match for the next page.
                next = m.sequence_number;
                break 'outer;
            }
            scanned += 1;
            next = m.sequence_number + 1;
            advanced = true;
            if wanted {
                out.push(m);
            }
            if scanned >= opts.scan_limit {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }
    let next_sequence = if exhausted || scanned == 0 {
        None
    } else {
        Some(next)
    };
    Ok(PeekPage {
        messages: out,
        scanned,
        next_sequence,
        exhausted,
    })
}

/// Message tallies by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    /// Active.
    pub active: u64,
    /// Deferred.
    pub deferred: u64,
    /// Scheduled.
    pub scheduled: u64,
    /// All examined.
    pub total: u64,
    /// More messages exist beyond the cap, so the tallies are lower bounds.
    pub truncated: bool,
}

/// Count messages in `address` by peeking, examining at most `cap`.
///
/// `truncated` is set only when a message exists beyond the cap (an entity of
/// exactly `cap` messages is not truncated). Like [`scan_messages`], a message
/// without a sequence number ends the count and stale messages (below the
/// current position) are skipped and not counted; a batch that makes no
/// progress ends the count, so it always terminates.
pub async fn count_messages(
    src: &mut (impl PeekSource + ?Sized),
    address: &str,
    cap: u32,
) -> Result<StateCounts> {
    let mut counts = StateCounts::default();
    let mut next = 0i64;
    'outer: loop {
        // When the cap is reached, ask for one more message to see whether
        // anything lies beyond it.
        let want = (u64::from(cap).saturating_sub(counts.total))
            .min(u64::from(MAX_PER_REQUEST))
            .max(1) as u32;
        let batch = src.peek_batch(address, next, want).await?;
        if batch.is_empty() {
            break;
        }
        let mut advanced = false;
        for m in &batch {
            if m.sequence_number < 0 {
                break 'outer;
            }
            if m.sequence_number < next {
                continue;
            }
            if counts.total >= u64::from(cap) {
                counts.truncated = true;
                break 'outer;
            }
            counts.total += 1;
            match m.state {
                MessageState::Active => counts.active += 1,
                MessageState::Deferred => counts.deferred += 1,
                MessageState::Scheduled => counts.scheduled += 1,
            }
            next = m.sequence_number + 1;
            advanced = true;
        }
        if !advanced {
            break;
        }
    }
    Ok(counts)
}

/// Most one-message peeks [`last_sequence`] makes.
pub const MAX_PROBES: u32 = 200;

async fn first_at<S: PeekSource + ?Sized>(
    b: &mut Batches<'_, S>,
    from: i64,
    probes: &mut u32,
) -> Result<Option<i64>> {
    *probes += 1;
    if *probes > MAX_PROBES {
        return Err(Error::Protocol(
            "could not find the end of the entity: sequence numbers kept moving".into(),
        ));
    }
    Ok(b.next(from, 1)
        .await?
        .into_iter()
        .map(|m| m.sequence_number)
        .find(|&s| s >= from))
}

/// The highest sequence number in `address` (within `session` when given), or `None` when it
/// is empty. Uses one-message peeks: from the first message, the step doubles until a peek
/// finds nothing, then the gap is bisected (about 2·log2(span) peeks, at most [`MAX_PROBES`]).
/// Messages that arrive meanwhile can only raise the answer.
pub async fn last_sequence<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    session: Option<&str>,
) -> Result<Option<i64>> {
    let mut b = Batches {
        src,
        address,
        session,
    };
    let mut probes = 0;
    let Some(mut lo) = first_at(&mut b, 0, &mut probes).await? else {
        return Ok(None);
    };
    let mut step: i64 = 1;
    let mut hi = loop {
        let at = lo.saturating_add(step);
        if at == lo {
            return Ok(Some(lo));
        }
        match first_at(&mut b, at, &mut probes).await? {
            Some(s) => {
                lo = s;
                step = step.saturating_mul(2);
            }
            None => break at,
        }
    };
    // A message at `lo`, nothing at or after `hi`.
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        match first_at(&mut b, mid, &mut probes).await? {
            Some(s) => {
                lo = s;
                if lo >= hi {
                    hi = lo.saturating_add(1);
                }
            }
            None => hi = mid,
        }
    }
    Ok(Some(lo))
}

fn tail_page(kept: VecDeque<PeekedMessage>, scanned: u32) -> PeekPage {
    PeekPage {
        messages: kept.into(),
        scanned,
        next_sequence: None,
        exhausted: true,
    }
}

/// Most matches one page of a [`peek_tail`] window holds (besides the kept tail).
const TAIL_PAGE: u32 = 1_000;

/// One window of [`peek_tail`].
struct Window {
    /// The last `want` matches seen, oldest first.
    kept: VecDeque<PeekedMessage>,
    /// Messages examined.
    scanned: u32,
    /// The end of the entity was reached within the budget.
    complete: bool,
}

/// Scan from `start` to the end of `view` within `budget`, keeping only the last `want` matches
/// in a ring. A window over large bodies therefore holds at most `want` plus one page of
/// matches, however many messages match in it.
async fn scan_window<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    filter: &MessageFilter,
    start: i64,
    budget: u32,
    want: usize,
) -> Result<Window> {
    let mut w = Window {
        kept: VecDeque::with_capacity(want),
        scanned: 0,
        complete: false,
    };
    let mut opts = PeekOptions {
        from_sequence: start,
        max: TAIL_PAGE,
        scan_limit: budget,
        filter: filter.clone(),
    };
    while w.scanned < budget {
        opts.scan_limit = budget - w.scanned;
        let page = scan_messages(src, address, view, &opts).await?;
        w.scanned = w.scanned.saturating_add(page.scanned);
        for m in page.messages {
            if w.kept.len() == want {
                w.kept.pop_front();
            }
            w.kept.push_back(m);
        }
        match page.next_sequence {
            None => {
                w.complete = true;
                break;
            }
            Some(n) if n > opts.from_sequence => opts.from_sequence = n,
            // No progress: the end was not confirmed.
            Some(_) => break,
        }
    }
    Ok(w)
}

/// The last `opts.max` messages of `view` matching `opts.filter`, oldest first ("from the end").
/// Finds the end with [`last_sequence`], then scans windows that start further back (×4 each
/// time) until it has enough matches, starts at sequence 0, or has examined `opts.scan_limit`
/// messages in total. A window that runs out of budget before the end is discarded in favour
/// of the previous complete one. Each window keeps only its last `opts.max` matches, so memory
/// stays bounded on large-body entities. The page has no continuation.
pub async fn peek_tail<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    opts: &PeekOptions,
) -> Result<PeekPage> {
    let want = opts.max as usize;
    if want == 0 {
        return Ok(tail_page(VecDeque::new(), 0));
    }
    let Some(last) = last_sequence(src, address, opts.filter.session_id.as_deref()).await? else {
        return Ok(tail_page(VecDeque::new(), 0));
    };
    let mut window = (want as i64).max(64);
    let mut scanned: u32 = 0;
    let mut best: Option<VecDeque<PeekedMessage>> = None;
    loop {
        let start = last.saturating_sub(window - 1).max(0);
        let budget = opts.scan_limit.saturating_sub(scanned).max(1);
        let w = scan_window(src, address, view, &opts.filter, start, budget, want).await?;
        scanned = scanned.saturating_add(w.scanned);
        if !w.complete {
            // Out of budget before reaching the end: this window is incomplete.
            return Ok(tail_page(best.unwrap_or(w.kept), scanned));
        }
        if w.kept.len() >= want || start == 0 || scanned >= opts.scan_limit {
            return Ok(tail_page(w.kept, scanned));
        }
        best = Some(w.kept);
        window = window.saturating_mul(4);
    }
}

/// The message with sequence number `sequence` in `view` of `address`, if it is still there.
pub async fn peek_one<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    sequence: i64,
) -> Result<Option<PeekedMessage>> {
    let batch = src.peek_batch(address, sequence, 1).await?;
    Ok(batch
        .into_iter()
        .find(|m| m.sequence_number == sequence && view.accepts(m.state)))
}

/// Messages found by [`collect_sequences`].
#[derive(Debug, Clone, Default)]
pub struct SequenceCollection {
    /// Found, in sequence order.
    pub messages: Vec<PeekedMessage>,
    /// Asked for but not found (received, expired, not in this view, or beyond the budget).
    pub missing: Vec<i64>,
    /// Messages examined.
    pub scanned: u64,
}

/// The messages of `view` with the given sequence numbers, read by scanning from the smallest
/// to the largest. Examines at most `budget` messages; stops early when all are found.
pub async fn collect_sequences<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    sequences: &[i64],
    budget: u64,
    cancel: Option<&CancellationToken>,
) -> Result<SequenceCollection> {
    let mut wanted: BTreeSet<i64> = sequences.iter().copied().filter(|&s| s >= 0).collect();
    let mut out = SequenceCollection::default();
    let (Some(&first), Some(&last)) = (wanted.first(), wanted.last()) else {
        return Ok(out);
    };
    let mut next = first;
    'outer: while !wanted.is_empty() && out.scanned < budget {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(Error::Cancelled);
        }
        let batch = src.peek_batch(address, next, MAX_PER_REQUEST).await?;
        let mut advanced = false;
        for m in batch {
            if m.sequence_number < next {
                continue;
            }
            if m.sequence_number > last {
                break 'outer;
            }
            out.scanned += 1;
            next = m.sequence_number + 1;
            advanced = true;
            if wanted.remove(&m.sequence_number) && view.accepts(m.state) {
                out.messages.push(m);
            }
            if out.scanned >= budget {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }
    // A wanted message in the wrong state was removed from `wanted` but not collected: report it.
    let found: BTreeSet<i64> = out.messages.iter().map(|m| m.sequence_number).collect();
    out.missing = sequences
        .iter()
        .copied()
        .filter(|s| *s >= 0 && !found.contains(s))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::filter::PropertyFilter;
    use crate::messaging::{MessageState, test_message};
    use horizon_azure_kit::safety::CancellationToken;

    struct Fake {
        messages: Vec<PeekedMessage>,
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Fake {
        async fn peek_batch(
            &mut self,
            _address: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(self
                .messages
                .iter()
                .filter(|m| m.sequence_number >= from)
                .take(max as usize)
                .cloned()
                .collect())
        }
    }

    fn entity(n: i64, scheduled_every: i64) -> Fake {
        let messages = (1..=n)
            .map(|i| {
                let mut m = test_message(i);
                if scheduled_every > 0 && i % scheduled_every == 0 {
                    m.state = MessageState::Scheduled;
                }
                m
            })
            .collect();
        Fake { messages, calls: 0 }
    }

    #[tokio::test]
    async fn views_keep_only_their_state() {
        let mut src = entity(10, 5);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &PeekOptions::new(50))
            .await
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [5, 10]
        );
        assert!(page.exhausted);
        assert_eq!(page.next_sequence, None);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(50))
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 8);
    }

    #[tokio::test]
    async fn max_leaves_the_next_match_for_the_continuation() {
        let mut src = entity(10, 0);
        let first = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3))
            .await
            .unwrap();
        assert_eq!(
            first
                .messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        let next = first.next_sequence.unwrap();
        let second = scan_messages(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(3).from_sequence(next),
        )
        .await
        .unwrap();
        assert_eq!(
            second
                .messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );
    }

    #[tokio::test]
    async fn scan_stops_at_limit_with_continuation() {
        let mut src = entity(100_000, 0);
        let opts = PeekOptions::new(10).scan_limit(1_000);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &opts)
            .await
            .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.scanned, 1_000);
        assert_eq!(page.next_sequence, Some(1_001));
        assert!(!page.exhausted);
        assert!(src.calls <= 4, "1000 / 250 batches, got {}", src.calls);
    }

    /// A misbehaving source that always returns messages without sequence numbers.
    struct Stuck {
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Stuck {
        async fn peek_batch(
            &mut self,
            _address: &str,
            _from: i64,
            _max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok((0..5).map(|_| test_message(-1)).collect())
        }
    }

    #[tokio::test]
    async fn scan_stops_when_sequence_does_not_advance() {
        let mut src = Stuck { calls: 0 };
        let page = scan_messages(&mut src, "q", PeekView::DeadLetter, &PeekOptions::new(100))
            .await
            .unwrap();
        assert_eq!(src.calls, 1, "must not re-read the same range");
        assert!(
            page.messages.is_empty(),
            "unsequenced messages are not emitted"
        );
        assert_eq!(page.scanned, 0);
        assert_eq!(
            page.next_sequence, None,
            "no progress means no continuation"
        );
        assert!(!page.exhausted);
    }

    /// Returns scripted batches in order, then empty batches.
    struct Scripted {
        batches: std::collections::VecDeque<Vec<PeekedMessage>>,
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Scripted {
        async fn peek_batch(
            &mut self,
            _address: &str,
            _from: i64,
            _max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(self.batches.pop_front().unwrap_or_default())
        }
    }

    fn seqs(list: &[i64]) -> Vec<PeekedMessage> {
        list.iter().map(|&i| test_message(i)).collect()
    }

    fn seq_numbers(page: &PeekPage) -> Vec<i64> {
        page.messages.iter().map(|m| m.sequence_number).collect()
    }

    #[tokio::test]
    async fn duplicate_and_stale_messages_are_skipped() {
        let mut src = Scripted {
            batches: [seqs(&[5, 5, 6])].into(),
            calls: 0,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [5, 6]);
        assert_eq!(
            page.scanned, 2,
            "stale messages do not count toward the scan limit"
        );
        assert!(page.exhausted);
    }

    #[tokio::test]
    async fn stale_only_batch_makes_no_progress() {
        let mut src = Scripted {
            batches: [seqs(&[3, 4]), seqs(&[1, 2])].into(),
            calls: 0,
        };
        let opts = PeekOptions::new(10).from_sequence(10);
        let page = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(src.calls, 1);
        assert!(page.messages.is_empty());
        assert_eq!(page.next_sequence, None);
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn gapped_sequences_continue_correctly() {
        let mut src = Fake {
            messages: seqs(&[1, 2, 10, 11]),
            calls: 0,
        };
        let first = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&first), [1, 2, 10]);
        assert_eq!(first.next_sequence, Some(11));
        let opts = PeekOptions::new(3).from_sequence(11);
        let second = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&second), [11]);
        assert!(second.exhausted);
    }

    #[tokio::test]
    async fn max_mid_batch_points_at_the_first_unexamined_message() {
        // Even sequence numbers are scheduled, so Active matches 1, 3, 5, ...
        let mut src = entity(10, 2);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(2))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1, 3]);
        assert_eq!(page.scanned, 4, "1, 2, 3 and the non-match 4 were examined");
        assert_eq!(page.next_sequence, Some(5));
        let opts = PeekOptions::new(2).from_sequence(5);
        let second = scan_messages(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&second), [5, 7]);
    }

    #[tokio::test]
    async fn deferred_view_keeps_only_deferred_messages() {
        let mut src = entity(9, 0);
        for m in src
            .messages
            .iter_mut()
            .filter(|m| m.sequence_number % 3 == 0)
        {
            m.state = MessageState::Deferred;
        }
        let page = scan_messages(&mut src, "q", PeekView::Deferred, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [3, 6, 9]);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 6);
    }

    #[tokio::test]
    async fn unsequenced_message_after_progress_keeps_the_continuation() {
        let mut batch = seqs(&[1, 2]);
        batch.push(test_message(-1));
        let mut src = Scripted {
            batches: [batch].into(),
            calls: 0,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1, 2]);
        assert_eq!(page.next_sequence, Some(3));
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn zero_max_returns_immediately_without_a_continuation() {
        let mut src = entity(10, 0);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(0))
            .await
            .unwrap();
        assert_eq!(src.calls, 0);
        assert!(page.messages.is_empty());
        assert_eq!(page.next_sequence, None);
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn filters_apply_during_the_scan() {
        let mut src = entity(20, 0);
        for m in src.messages.iter_mut() {
            m.delivery_count = (m.sequence_number % 3) as u32;
        }
        let filter = MessageFilter {
            property: Some(PropertyFilter::parse("sys.DeliveryCount = 2").unwrap()),
            body: vec![],
            ..Default::default()
        };
        let page = scan_messages(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(100).filter(filter),
        )
        .await
        .unwrap();
        assert!(page.messages.iter().all(|m| m.delivery_count == 2));
        assert_eq!(page.messages.len(), 7);
    }

    #[tokio::test]
    async fn empty_entity_is_exhausted_immediately() {
        let mut src = entity(0, 0);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10))
            .await
            .unwrap();
        assert!(page.exhausted && page.messages.is_empty() && page.scanned == 0);
    }

    #[tokio::test]
    async fn count_messages_tallies_states_and_respects_cap() {
        let mut src = entity(10, 5);
        src.messages[0].state = MessageState::Deferred;
        let c = count_messages(&mut src, "q", 1_000).await.unwrap();
        assert_eq!(
            (c.active, c.deferred, c.scheduled, c.total, c.truncated),
            (7, 1, 2, 10, false)
        );
        let capped = count_messages(&mut src, "q", 4).await.unwrap();
        assert_eq!(capped.total, 4);
        assert!(capped.truncated);
    }

    #[tokio::test]
    async fn count_at_exactly_the_cap_is_not_truncated() {
        let mut src = entity(10, 0);
        let c = count_messages(&mut src, "q", 10).await.unwrap();
        assert_eq!((c.total, c.truncated), (10, false));
        let c = count_messages(&mut src, "q", 9).await.unwrap();
        assert_eq!((c.total, c.truncated), (9, true));
        let mut empty = entity(0, 0);
        let c = count_messages(&mut empty, "q", 0).await.unwrap();
        assert_eq!((c.total, c.truncated), (0, false));
    }

    #[tokio::test]
    async fn count_pages_past_one_request() {
        let mut src = entity(600, 0);
        let c = count_messages(&mut src, "q", 10_000).await.unwrap();
        assert_eq!((c.total, c.truncated), (600, false));
        assert!(src.calls <= 4, "got {}", src.calls);
    }

    #[tokio::test]
    async fn count_terminates_on_a_stuck_source() {
        let mut src = Stuck { calls: 0 };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(src.calls, 1);
        assert_eq!((c.total, c.truncated), (0, false));
        // Stale replays only: no progress, must not loop.
        let mut src = Scripted {
            batches: [seqs(&[1, 2]), seqs(&[1, 2]), seqs(&[1, 2])].into(),
            calls: 0,
        };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(c.total, 2);
        assert_eq!(src.calls, 2, "second batch is all stale and ends the count");
    }

    #[tokio::test]
    async fn count_skips_duplicates_and_stops_at_unsequenced() {
        let mut batch = seqs(&[5, 5, 6]);
        batch.push(test_message(-1));
        batch.push(test_message(7));
        let mut src = Scripted {
            batches: [batch].into(),
            calls: 0,
        };
        let c = count_messages(&mut src, "q", 100).await.unwrap();
        assert_eq!(c.total, 2);
    }

    /// A source that peeks within a session (or refuses to).
    struct SessionFake {
        messages: Vec<PeekedMessage>,
        session_calls: u32,
        plain_calls: u32,
        supports_sessions: bool,
    }

    #[async_trait]
    impl PeekSource for SessionFake {
        async fn peek_batch(
            &mut self,
            _a: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.plain_calls += 1;
            Ok(self
                .messages
                .iter()
                .filter(|m| m.sequence_number >= from)
                .take(max as usize)
                .cloned()
                .collect())
        }
        async fn peek_session_batch(
            &mut self,
            _a: &str,
            s: &str,
            from: i64,
            max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.session_calls += 1;
            if !self.supports_sessions {
                return Err(Error::Unsupported {
                    capability: "peek within a session".into(),
                });
            }
            Ok(self
                .messages
                .iter()
                .filter(|m| m.sequence_number >= from && m.session_id.as_deref() == Some(s))
                .take(max as usize)
                .cloned()
                .collect())
        }
    }

    fn sessions(n: i64) -> Vec<PeekedMessage> {
        (1..=n)
            .map(|i| {
                let mut m = test_message(i);
                m.session_id = Some(if i % 2 == 0 { "even" } else { "odd" }.into());
                m
            })
            .collect()
    }

    fn in_session(id: &str) -> PeekOptions {
        PeekOptions::new(100).filter(MessageFilter {
            session_id: Some(id.into()),
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn session_scan_asks_the_broker_for_that_session() {
        let mut src = SessionFake {
            messages: sessions(10),
            session_calls: 0,
            plain_calls: 0,
            supports_sessions: true,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &in_session("even"))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [2, 4, 6, 8, 10]);
        assert_eq!(src.plain_calls, 0);
        assert_eq!(page.scanned, 5, "only the session's messages were examined");
    }

    #[tokio::test]
    async fn session_scan_falls_back_when_the_source_cannot_peek_by_session() {
        let mut src = SessionFake {
            messages: sessions(10),
            session_calls: 0,
            plain_calls: 0,
            supports_sessions: false,
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &in_session("odd"))
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1, 3, 5, 7, 9]);
        assert_eq!(src.session_calls, 1, "asked once, then fell back for good");
        assert!(page.exhausted);
    }

    #[tokio::test]
    async fn last_sequence_finds_the_end_of_sparse_sequences() {
        for list in [
            vec![1, 2, 3],
            vec![5],
            vec![1, 10, 1_000, 1_000_000],
            vec![3, 4, (1i64 << 48) + 7],
        ] {
            let mut src = Fake {
                messages: seqs(&list),
                calls: 0,
            };
            assert_eq!(
                last_sequence(&mut src, "q", None).await.unwrap(),
                list.last().copied(),
                "{list:?}"
            );
            assert!(src.calls <= 2 * 64 + 2, "{} probes for {list:?}", src.calls);
        }
        let mut empty = Fake {
            messages: vec![],
            calls: 0,
        };
        assert_eq!(last_sequence(&mut empty, "q", None).await.unwrap(), None);
    }

    /// Answers every peek with a message just past the request, as if messages kept arriving.
    struct Endless {
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Endless {
        async fn peek_batch(
            &mut self,
            _a: &str,
            from: i64,
            _max: u32,
        ) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(vec![test_message(from.saturating_add(1))])
        }
    }

    #[tokio::test]
    async fn last_sequence_terminates_on_a_source_that_never_ends() {
        let mut src = Endless { calls: 0 };
        let r = last_sequence(&mut src, "q", None).await;
        assert!(src.calls <= MAX_PROBES, "{} probes", src.calls);
        assert!(matches!(r, Ok(Some(_)) | Err(Error::Protocol(_))), "{r:?}");
    }

    #[tokio::test]
    async fn tail_returns_the_last_matches_oldest_first() {
        let mut src = entity(1_000, 0);
        let opts = PeekOptions::new(3).scan_limit(100_000);
        let page = peek_tail(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [998, 999, 1000]);
        assert!(page.exhausted && page.next_sequence.is_none());
    }

    #[tokio::test]
    async fn tail_widens_until_the_filter_has_enough_matches() {
        // Only every 100th message is scheduled; ask for the last 4 scheduled ones.
        let mut src = entity(2_000, 100);
        let opts = PeekOptions::new(4).scan_limit(100_000);
        let page = peek_tail(&mut src, "q", PeekView::Scheduled, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [1700, 1800, 1900, 2000]);
    }

    #[tokio::test]
    async fn tail_window_spanning_several_pages_keeps_the_last_matches() {
        // 1,500 wanted: the window is read in pages of at most 1,000 matches into a ring.
        let mut src = entity(3_000, 0);
        let page = peek_tail(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(1_500).scan_limit(100_000),
        )
        .await
        .unwrap();
        assert_eq!(seq_numbers(&page), (1_501..=3_000).collect::<Vec<i64>>());
    }

    #[tokio::test]
    async fn tail_of_a_short_entity_is_all_of_it() {
        let mut src = entity(5, 0);
        let page = peek_tail(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(50).scan_limit(1_000),
        )
        .await
        .unwrap();
        assert_eq!(seq_numbers(&page), [1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn tail_is_bounded_by_the_scan_budget() {
        // Nothing matches; the search must stop near the budget, not read 50,000 messages.
        let mut src = entity(50_000, 0);
        let opts = PeekOptions::new(10).scan_limit(2_000);
        let page = peek_tail(&mut src, "q", PeekView::Scheduled, &opts)
            .await
            .unwrap();
        assert!(page.messages.is_empty());
        assert!(page.scanned <= 2_000, "scanned {}", page.scanned);
    }

    #[tokio::test]
    async fn peek_one_returns_the_message_or_none_when_the_sequence_is_gone() {
        let mut src = Fake {
            messages: seqs(&[1, 2, 5]),
            calls: 0,
        };
        assert_eq!(
            peek_one(&mut src, "q", PeekView::Active, 2)
                .await
                .unwrap()
                .unwrap()
                .sequence_number,
            2
        );
        assert!(
            peek_one(&mut src, "q", PeekView::Active, 3)
                .await
                .unwrap()
                .is_none(),
            "3 was received; 5 is not it"
        );
        assert!(
            peek_one(&mut src, "q", PeekView::Scheduled, 1)
                .await
                .unwrap()
                .is_none(),
            "wrong view"
        );
    }

    #[tokio::test]
    async fn collect_sequences_returns_found_and_reports_missing() {
        let mut src = Fake {
            messages: seqs(&[1, 2, 3, 5, 8, 13, 21]),
            calls: 0,
        };
        let c = collect_sequences(
            &mut src,
            "q",
            PeekView::Active,
            &[21, 3, 4, 3, 8],
            1_000,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            c.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [3, 8, 21]
        );
        assert_eq!(c.missing, [4]);
    }

    #[tokio::test]
    async fn collect_sequences_stops_at_the_budget_and_on_cancel() {
        let mut src = entity(10_000, 0);
        let c = collect_sequences(&mut src, "q", PeekView::Active, &[1, 9_999], 500, None)
            .await
            .unwrap();
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.missing, [9_999], "not reached within the budget");
        let token = CancellationToken::new();
        token.cancel();
        let e = collect_sequences(&mut src, "q", PeekView::Active, &[1], 500, Some(&token))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Cancelled));
    }
}
