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

/// What [`walk_messages`] examined.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Walked {
    /// Messages handed to the visitor.
    pub examined: u64,
    /// A message exists beyond the cap.
    pub truncated: bool,
}

/// Hand every message of `address` to `visit`, from the start, examining at most `cap` (and
/// asking for one more to learn whether anything lies beyond it). Stale messages (below the
/// current position) are skipped; a message without a sequence number or a batch that makes
/// no progress ends the walk, so it always terminates.
pub(crate) async fn walk_messages<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    cap: u32,
    mut visit: impl FnMut(&PeekedMessage) + Send,
) -> Result<Walked> {
    let mut w = Walked::default();
    let mut next = 0i64;
    'outer: loop {
        let want = (u64::from(cap).saturating_sub(w.examined))
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
            if w.examined >= u64::from(cap) {
                w.truncated = true;
                break 'outer;
            }
            w.examined += 1;
            next = m.sequence_number + 1;
            advanced = true;
            visit(m);
        }
        if !advanced {
            break;
        }
    }
    Ok(w)
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
    let walked = walk_messages(src, address, cap, |m| match m.state {
        MessageState::Active => counts.active += 1,
        MessageState::Deferred => counts.deferred += 1,
        MessageState::Scheduled => counts.scheduled += 1,
    })
    .await?;
    counts.total = walked.examined;
    counts.truncated = walked.truncated;
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
    find_last(&mut Batches {
        src,
        address,
        session,
    })
    .await
}

/// [`last_sequence`] over `b`, so a caller keeps one session fallback for a whole operation.
async fn find_last<S: PeekSource + ?Sized>(b: &mut Batches<'_, S>) -> Result<Option<i64>> {
    let mut probes = 0;
    let Some(mut lo) = first_at(b, 0, &mut probes).await? else {
        return Ok(None);
    };
    let mut step: i64 = 1;
    let mut hi = loop {
        let at = lo.saturating_add(step);
        if at == lo {
            return Ok(Some(lo));
        }
        match first_at(b, at, &mut probes).await? {
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
        match first_at(b, mid, &mut probes).await? {
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

/// A [`peek_tail`] page. `reached_end` is false when the budget ran out before the first
/// window was read to the end, so the messages are not the tail.
fn tail_page(kept: VecDeque<PeekedMessage>, scanned: u32, reached_end: bool) -> PeekPage {
    PeekPage {
        messages: kept.into(),
        scanned,
        next_sequence: None,
        exhausted: reached_end,
    }
}

/// Most matches a [`peek_tail`] ring allocates up front, so a large `max` costs memory only
/// as matches are found.
const TAIL_PREALLOC: usize = 1_000;

/// One range of a [`peek_tail`] window.
struct Range {
    /// The last `want` matches in the range, oldest first.
    kept: VecDeque<PeekedMessage>,
    /// Messages examined.
    scanned: u32,
    /// The whole range was read (or the entity ended inside it).
    complete: bool,
}

/// Read sequences `start..=end` of `view` within `budget`, keeping only the last `want` matches
/// in a ring. Nothing past `end` is read into the result, so messages that arrive during the
/// search are never chased. A batch without progress leaves the range incomplete.
async fn scan_range<S: PeekSource + ?Sized>(
    b: &mut Batches<'_, S>,
    view: PeekView,
    filter: &MessageFilter,
    (start, end): (i64, i64),
    budget: u32,
    want: usize,
) -> Result<Range> {
    let mut r = Range {
        kept: VecDeque::with_capacity(want.min(TAIL_PREALLOC)),
        scanned: 0,
        complete: false,
    };
    let mut next = start;
    'outer: while r.scanned < budget {
        // No more than the range can hold: sequence numbers are unique.
        let span = u32::try_from(end.saturating_sub(next).saturating_add(1)).unwrap_or(u32::MAX);
        let max = (budget - r.scanned).min(MAX_PER_REQUEST).min(span).max(1);
        let batch = b.next(next, max).await?;
        if batch.is_empty() {
            // The entity ends inside the range (messages before `end` were received).
            r.complete = true;
            break;
        }
        let mut advanced = false;
        for m in batch {
            if m.sequence_number < 0 {
                // No sequence number: we cannot position past it.
                break 'outer;
            }
            if m.sequence_number < next {
                continue;
            }
            if m.sequence_number > end {
                r.complete = true;
                break 'outer;
            }
            r.scanned += 1;
            next = m.sequence_number.saturating_add(1);
            advanced = true;
            let at_end = m.sequence_number == end;
            if view.accepts(m.state) && filter.matches(&m) {
                if r.kept.len() == want {
                    r.kept.pop_front();
                }
                r.kept.push_back(m);
            }
            if at_end {
                r.complete = true;
                break 'outer;
            }
            if r.scanned >= budget {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(r)
}

/// The last `opts.max` messages of `view` matching `opts.filter`, oldest first ("from the end").
///
/// Finds the end once with [`last_sequence`] and never reads past it, so messages that arrive
/// during the search are not chased. Then reads windows that start further back (×4 each time)
/// until it has enough matches, starts at sequence 0, or has examined `opts.scan_limit`
/// messages in total. Each wider window reads only the range in front of the previous one and
/// merges it with what that one found, so every message is examined at most once. When the
/// budget runs out inside a range, that range is dropped in favour of what the previous windows
/// found. Only the last `opts.max` matches are held, so memory stays bounded on large-body
/// entities. Within a session, the session peek is tried once for the whole search.
///
/// The page has no continuation (`next_sequence: None`). `exhausted` is true when the result is
/// the tail as of the end that was found. It is false only when the budget ran out inside the
/// first window: the page then holds that window's partial matches, which are not the last
/// ones, and callers should say the scan did not reach the end.
pub async fn peek_tail<S: PeekSource + ?Sized>(
    src: &mut S,
    address: &str,
    view: PeekView,
    opts: &PeekOptions,
) -> Result<PeekPage> {
    let want = opts.max as usize;
    if want == 0 {
        return Ok(tail_page(VecDeque::new(), 0, true));
    }
    let mut b = Batches {
        src,
        address,
        session: opts.filter.session_id.as_deref(),
    };
    let Some(last) = find_last(&mut b).await? else {
        return Ok(tail_page(VecDeque::new(), 0, true));
    };
    let mut window = (want as i64).max(64);
    let mut scanned: u32 = 0;
    // Every match in `end + 1..=last` (fewer than `want`), oldest first; `None` before the first
    // window is complete.
    let mut found: Option<VecDeque<PeekedMessage>> = None;
    let mut end = last;
    loop {
        let start = last.saturating_sub(window - 1).max(0);
        let budget = opts.scan_limit.saturating_sub(scanned).max(1);
        let r = scan_range(&mut b, view, &opts.filter, (start, end), budget, want).await?;
        scanned = scanned.saturating_add(r.scanned);
        if !r.complete {
            // Out of budget (or stuck) before the range was read: keep the complete part. When
            // even the first window is incomplete, its matches are from the front of the window,
            // not the tail, and the page says the end was not reached.
            return Ok(match found {
                Some(tail) => tail_page(tail, scanned, true),
                None => tail_page(r.kept, scanned, false),
            });
        }
        let mut merged = r.kept;
        merged.extend(found.take().unwrap_or_default());
        while merged.len() > want {
            merged.pop_front();
        }
        if merged.len() >= want || start == 0 || scanned >= opts.scan_limit {
            return Ok(tail_page(merged, scanned, true));
        }
        found = Some(merged);
        end = start - 1;
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

/// The messages of `view` with the given sequence numbers, read from the smallest to the
/// largest. Each batch starts at the next wanted sequence number, so messages between wanted
/// ones are skipped rather than read. Examines at most `budget` messages; stops early when all
/// are found.
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
    'outer: while out.scanned < budget {
        // Wanted sequences behind the position were skipped over (received, expired or never
        // there); drop them so they cannot hold back the jump. `missing` is computed below.
        wanted = wanted.split_off(&next);
        // Skip the gap to the next wanted message, so the cost follows the selection.
        let Some(&target) = wanted.first() else {
            break;
        };
        next = next.max(target);
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
        // The gallop's step saturates at i64::MAX, where the source cannot go further.
        assert_eq!(r.unwrap(), Some(i64::MAX));
        assert!(src.calls <= 129, "{} probes", src.calls);
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
        // A dense selection (no gap to jump over): the budget ends the read at 500.
        let wanted: Vec<i64> = (1..=600).collect();
        let c = collect_sequences(&mut src, "q", PeekView::Active, &wanted, 500, None)
            .await
            .unwrap();
        assert_eq!(c.messages.len(), 500);
        assert_eq!(
            c.missing,
            (501..=600).collect::<Vec<i64>>(),
            "not reached within the budget"
        );
        assert_eq!(c.scanned, 500);
        let token = CancellationToken::new();
        token.cancel();
        let e = collect_sequences(&mut src, "q", PeekView::Active, &[1], 500, Some(&token))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Cancelled));
    }

    #[tokio::test]
    async fn collect_sequences_jumps_to_the_next_wanted_sequence() {
        let mut src = entity(100_000, 0);
        let c = collect_sequences(
            &mut src,
            "q",
            PeekView::Active,
            &[10, 95_000],
            1_000_000,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            c.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [10, 95_000]
        );
        assert!(c.missing.is_empty());
        assert!(
            src.calls <= 2,
            "cost follows the selection, not the span: {} batches",
            src.calls
        );
        assert!(
            c.scanned <= 2 * u64::from(MAX_PER_REQUEST),
            "scanned {}",
            c.scanned
        );
    }

    #[tokio::test]
    async fn collect_sequences_reports_a_wanted_message_in_another_view_as_missing() {
        // 5 and 10 are scheduled.
        let mut src = entity(10, 5);
        let c = collect_sequences(&mut src, "q", PeekView::Active, &[4, 5, 6], 1_000, None)
            .await
            .unwrap();
        assert_eq!(
            c.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [4, 6]
        );
        assert_eq!(c.missing, [5]);
    }

    /// One-message probes see the entity as it was; every wider peek first lets `burst` new
    /// messages arrive, faster than a scan can read them.
    struct Arriving {
        inner: Fake,
        burst: i64,
    }

    #[async_trait]
    impl PeekSource for Arriving {
        async fn peek_batch(&mut self, a: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>> {
            if max > 1 {
                let end = self.inner.messages.last().map_or(0, |m| m.sequence_number);
                self.inner
                    .messages
                    .extend((end + 1..=end + self.burst).map(test_message));
            }
            self.inner.peek_batch(a, from, max).await
        }
    }

    #[tokio::test]
    async fn tail_never_chases_messages_that_arrive_during_the_scan() {
        let mut src = Arriving {
            inner: entity(1_000, 0),
            burst: 500,
        };
        let opts = PeekOptions::new(3).scan_limit(100_000);
        let page = peek_tail(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(
            seq_numbers(&page),
            [998, 999, 1000],
            "the tail as of the end that was found"
        );
        assert!(page.scanned <= 64, "scanned {}", page.scanned);
    }

    #[tokio::test]
    async fn tail_window_ending_exactly_on_the_last_message_is_complete() {
        // Windows of 64 and 256 fit the budget of 320 exactly; the third runs out of budget.
        let mut src = entity(1_000, 100);
        let opts = PeekOptions::new(64).scan_limit(320);
        let page = peek_tail(&mut src, "q", PeekView::Scheduled, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [800, 900, 1000]);
        assert!(page.scanned <= 320, "scanned {}", page.scanned);
    }

    #[tokio::test]
    async fn tail_widening_reads_each_message_once() {
        // Nothing matches, so every window widens; the new range of each window is read, not
        // the part the previous window already covered.
        let mut src = entity(1_000, 0);
        let page = peek_tail(
            &mut src,
            "q",
            PeekView::Scheduled,
            &PeekOptions::new(5).scan_limit(100_000),
        )
        .await
        .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.scanned, 1_000, "each message examined once");
    }

    /// Probes see the entity; wider peeks only answer a stale message, so a window cannot move.
    struct StaleWide {
        inner: Fake,
        wide_calls: u32,
    }

    #[async_trait]
    impl PeekSource for StaleWide {
        async fn peek_batch(&mut self, a: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>> {
            if max == 1 {
                return self.inner.peek_batch(a, from, max).await;
            }
            self.wide_calls += 1;
            Ok(seqs(&[1]))
        }
    }

    #[tokio::test]
    async fn tail_window_without_progress_is_not_complete() {
        let mut src = StaleWide {
            inner: entity(1_000, 0),
            wide_calls: 0,
        };
        let page = peek_tail(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(3).scan_limit(100_000),
        )
        .await
        .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(
            src.wide_calls, 1,
            "no progress ends the search instead of widening"
        );
    }

    #[tokio::test]
    async fn tail_peeks_within_the_session() {
        let mut src = SessionFake {
            messages: sessions(10),
            session_calls: 0,
            plain_calls: 0,
            supports_sessions: true,
        };
        let opts = PeekOptions {
            max: 3,
            ..in_session("even")
        };
        let page = peek_tail(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(seq_numbers(&page), [6, 8, 10]);
        assert_eq!(src.plain_calls, 0);
    }

    #[tokio::test]
    async fn tail_asks_for_a_session_peek_once_when_the_source_cannot() {
        let mut src = SessionFake {
            messages: sessions(1_000),
            session_calls: 0,
            plain_calls: 0,
            supports_sessions: false,
        };
        // Odd session, last 40: needs more than one window (64 wide holds 32 odd messages).
        let opts = PeekOptions {
            max: 40,
            ..in_session("odd")
        };
        let page = peek_tail(&mut src, "q", PeekView::Active, &opts)
            .await
            .unwrap();
        assert_eq!(
            seq_numbers(&page),
            (921..=999).step_by(2).collect::<Vec<i64>>()
        );
        assert_eq!(
            src.session_calls, 1,
            "the refusal is remembered for the whole search"
        );
    }

    #[tokio::test]
    async fn collect_sequences_keeps_jumping_past_a_missing_sequence() {
        // 4 was received: it is skipped over and must not hold back the next jump.
        let mut src = entity(100_000, 0);
        src.messages.retain(|m| m.sequence_number != 4);
        let c = collect_sequences(
            &mut src,
            "q",
            PeekView::Active,
            &[4, 95_000],
            1_000_000,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            c.messages
                .iter()
                .map(|m| m.sequence_number)
                .collect::<Vec<_>>(),
            [95_000]
        );
        assert_eq!(c.missing, [4]);
        assert!(src.calls <= 3, "{} batches", src.calls);
    }

    #[tokio::test]
    async fn tail_that_runs_out_of_budget_in_its_first_window_is_not_exhausted() {
        // A budget of 10 cannot read the first window (64 messages) to the end.
        let mut src = entity(1_000, 0);
        let page = peek_tail(
            &mut src,
            "q",
            PeekView::Active,
            &PeekOptions::new(3).scan_limit(10),
        )
        .await
        .unwrap();
        assert!(!page.exhausted, "the scan did not reach the end");
        assert_eq!(page.next_sequence, None, "a tail has no continuation");
        assert!(page.scanned <= 10, "scanned {}", page.scanned);
    }
}
