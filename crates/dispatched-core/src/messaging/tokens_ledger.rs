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

    pub(crate) fn needs_put(&self, audience: &str, now: OffsetDateTime) -> bool {
        self.expiries
            .get(audience)
            .is_none_or(|exp| *exp - now <= REFRESH_MARGIN)
    }

    pub(crate) fn forget(&mut self, audience: &str) {
        self.expiries.remove(audience);
    }
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
    fn lru_evicts_least_recently_used() {
        let mut lru = LinkLru::new(2);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("b"), None);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("c").as_deref(), Some("b"));
    }
}
