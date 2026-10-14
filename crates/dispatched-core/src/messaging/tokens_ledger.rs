//! When to re-put CBS tokens, and which management links to keep.

use std::collections::{HashMap, VecDeque};
use time::OffsetDateTime;

/// Re-put a token when it has less than this left.
pub(crate) const REFRESH_MARGIN: time::Duration = time::Duration::minutes(5);

/// Token expiry per CBS audience.
#[derive(Debug, Default)]
pub(crate) struct TokenLedger {
    expiries: HashMap<String, OffsetDateTime>,
}

impl TokenLedger {
    pub(crate) fn record(&mut self, audience: &str, expires_on: OffsetDateTime) {
        self.expiries.insert(audience.to_string(), expires_on);
    }

    /// Whether a token must be put for `audience` at `now`: none recorded, or
    /// at most [`REFRESH_MARGIN`] left. Tokens that live no longer than the
    /// margin are therefore re-put on every call. Comparing against the local
    /// clock, the margin also absorbs clock skew of up to 5 minutes.
    pub(crate) fn needs_put(&self, audience: &str, now: OffsetDateTime) -> bool {
        self.expiries
            .get(audience)
            .is_none_or(|exp| *exp - now <= REFRESH_MARGIN)
    }

    pub(crate) fn forget(&mut self, audience: &str) {
        self.expiries.remove(audience);
    }
}

/// Whether a cached link must be dropped and re-attached: the broker closes a
/// link when its token expires, and re-putting a token does not revive it. So
/// any audience that needs a (re-)put means the link cannot be trusted.
pub(crate) fn must_reattach(
    ledger: &TokenLedger,
    audiences: &[String],
    now: OffsetDateTime,
) -> bool {
    audiences.iter().any(|a| ledger.needs_put(a, now))
}

/// Least-recently-used set of management-link addresses.
#[derive(Debug)]
pub(crate) struct LinkLru {
    capacity: usize,
    order: VecDeque<String>,
}

impl LinkLru {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
        }
    }

    /// Mark `address` as used. Returns an address to evict when over capacity.
    pub(crate) fn touch(&mut self, address: &str) -> Option<String> {
        if let Some(pos) = self.order.iter().position(|a| a == address) {
            self.order.remove(pos);
        }
        self.order.push_back(address.to_string());
        if self.order.len() > self.capacity {
            self.order.pop_front()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_refreshes_within_margin() {
        let now = OffsetDateTime::now_utc();
        let mut l = TokenLedger::default();
        assert!(l.needs_put("a", now), "unknown audience needs a token");
        l.record("a", now + time::Duration::minutes(60));
        assert!(!l.needs_put("a", now));
        assert!(l.needs_put("a", now + time::Duration::minutes(56)));
        l.forget("a");
        assert!(l.needs_put("a", now));
    }

    #[test]
    fn ledger_margin_boundary_is_exact_and_audiences_are_independent() {
        let now = OffsetDateTime::now_utc();
        let mut l = TokenLedger::default();
        l.record("a", now + REFRESH_MARGIN);
        l.record("b", now + REFRESH_MARGIN + time::Duration::seconds(1));
        assert!(l.needs_put("a", now), "exactly the margin left needs a put");
        assert!(
            !l.needs_put("b", now),
            "one second more than the margin is fine"
        );
        l.forget("a");
        assert!(!l.needs_put("b", now), "forgetting a leaves b alone");
        assert!(l.needs_put("c", now));
    }

    #[test]
    fn must_reattach_when_any_audience_lapsed() {
        let now = OffsetDateTime::now_utc();
        let mut l = TokenLedger::default();
        let auds = ["q".to_string(), "q/$management".to_string()];
        assert!(must_reattach(&l, &auds, now), "nothing recorded");
        l.record("q", now + time::Duration::minutes(60));
        l.record("q/$management", now + time::Duration::minutes(60));
        assert!(!must_reattach(&l, &auds, now));
        l.record("q/$management", now + time::Duration::minutes(2));
        assert!(must_reattach(&l, &auds, now), "one audience within margin");
        assert!(must_reattach(&l, &auds, now + time::Duration::minutes(56)));
        assert!(!must_reattach(&l, &[], now));
    }

    #[test]
    fn lru_capacity_one_evicts_previous() {
        let mut lru = LinkLru::new(1);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("b").as_deref(), Some("a"));
        assert_eq!(LinkLru::new(0).touch("x"), None, "capacity is at least 1");
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut lru = LinkLru::new(2);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("b"), None);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("c").as_deref(), Some("b"));
    }
}
