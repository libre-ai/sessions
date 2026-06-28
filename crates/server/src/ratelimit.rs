//! A token-bucket rate limiter guarding the open `POST /sessions` endpoint
//! against session-creation spam (which would exhaust store memory / database
//! rows). It is **global** for now: a generous burst capacity refilled at a
//! steady rate, capping total session-creation throughput regardless of source.
//!
//! Per-IP limiting (keyed on `X-Forwarded-For` behind a trusted proxy) is the
//! follow-on; a global cap protects the resource without depending on a reliable
//! client IP, which a single proxy hop would otherwise obscure.

use std::time::Instant;

use parking_lot::Mutex;

/// A monotonic-clock token bucket. `allow()` succeeds while tokens remain; tokens
/// refill continuously at `refill_per_sec` up to `capacity` (the burst size).
pub struct TokenBucket {
    capacity: f64,
    refill_per_sec: f64,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    /// A bucket starting full, with `capacity` burst and `refill_per_sec` steady
    /// rate. Built at `start` so tests can pin the clock.
    pub fn new_at(capacity: f64, refill_per_sec: f64, start: Instant) -> Self {
        Self {
            capacity,
            refill_per_sec,
            state: Mutex::new(BucketState {
                tokens: capacity,
                last: start,
            }),
        }
    }

    /// Convenience constructor anchored at the current instant.
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self::new_at(capacity, refill_per_sec, Instant::now())
    }

    /// Try to consume one token as of `now`. Returns `false` when the bucket is
    /// empty (caller should answer 429). Clock is a parameter so the refill is
    /// deterministically testable without sleeping.
    pub fn allow_at(&self, now: Instant) -> bool {
        let mut s = self.state.lock();
        // Refill for the elapsed time (saturating at `capacity`). `now` before
        // `last` (non-monotonic call order) contributes no negative refill.
        let elapsed = now.saturating_duration_since(s.last).as_secs_f64();
        s.tokens = (s.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        s.last = now;
        if s.tokens >= 1.0 {
            s.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Try to consume one token as of now.
    pub fn allow(&self) -> bool {
        self.allow_at(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn allows_up_to_capacity_then_refuses() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(3.0, 1.0, t0);
        // Three immediate calls drain the burst; the fourth is refused.
        assert!(b.allow_at(t0));
        assert!(b.allow_at(t0));
        assert!(b.allow_at(t0));
        assert!(!b.allow_at(t0));
    }

    #[test]
    fn refills_over_time() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(2.0, 1.0, t0); // 1 token/sec
        assert!(b.allow_at(t0));
        assert!(b.allow_at(t0));
        assert!(!b.allow_at(t0)); // empty
        // One second later, one token has refilled.
        assert!(b.allow_at(t0 + Duration::from_secs(1)));
        assert!(!b.allow_at(t0 + Duration::from_secs(1)));
    }

    #[test]
    fn refill_saturates_at_capacity() {
        let t0 = Instant::now();
        let b = TokenBucket::new_at(2.0, 1.0, t0);
        // A long idle does not let tokens exceed capacity.
        assert!(b.allow_at(t0 + Duration::from_secs(100)));
        assert!(b.allow_at(t0 + Duration::from_secs(100)));
        assert!(!b.allow_at(t0 + Duration::from_secs(100)));
    }
}
