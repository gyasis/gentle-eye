//! Per-key token-bucket rate limiting for the `analyze_*` tools.
//!
//! Implements **NFR-002 / T103**: limit vision-API calls to a configurable rate
//! (default 10 requests / 60 s) and return a clear error when the budget is
//! exhausted. A token bucket smooths bursts while enforcing the long-run rate.
//!
//! Authored 2026-05-28 from PRD NFR-002 (GAP-4 resolved here: token bucket
//! chosen over sliding window) — the recovered source was a 1-byte stub.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Returned when a caller exceeds its rate budget.
#[derive(Debug, Error, PartialEq)]
#[error("rate limit exceeded for '{key}'; retry in {retry_after_secs:.1}s")]
pub struct RateLimitExceeded {
    /// The bucket key (e.g. tool name or client id) that was throttled.
    pub key: String,
    /// Seconds until at least one token is available again.
    pub retry_after_secs: f64,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// A per-key token-bucket limiter. Cheap to clone the handle via `Arc` at the
/// call site; the internal state is behind a `Mutex` so it is `Send + Sync`.
pub struct RateLimiter {
    capacity: f64,
    /// Tokens replenished per second.
    refill_per_sec: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    /// Create a limiter allowing `max_per_minute` requests per key per 60 s.
    pub fn per_minute(max_per_minute: u32) -> Self {
        Self::new(max_per_minute as f64, Duration::from_secs(60))
    }

    /// Create a limiter of `capacity` tokens that fully refills over `window`.
    pub fn new(capacity: f64, window: Duration) -> Self {
        let window_secs = window.as_secs_f64().max(f64::MIN_POSITIVE);
        Self {
            capacity,
            refill_per_sec: capacity / window_secs,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Try to consume one token for `key`. `Ok(())` if permitted, otherwise an
    /// error carrying the retry-after hint.
    pub fn check(&self, key: &str) -> Result<(), RateLimitExceeded> {
        self.check_at(key, Instant::now())
    }

    /// Clock-injectable form of [`RateLimiter::check`] (used by tests).
    pub fn check_at(&self, key: &str, now: Instant) -> Result<(), RateLimitExceeded> {
        let mut buckets = self.buckets.lock().expect("rate-limiter mutex poisoned");
        let bucket = buckets.entry(key.to_string()).or_insert_with(|| Bucket {
            tokens: self.capacity,
            last_refill: now,
        });

        // Refill based on elapsed time, capped at capacity.
        let elapsed = now.saturating_duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let needed = 1.0 - bucket.tokens;
            Err(RateLimitExceeded {
                key: key.to_string(),
                retry_after_secs: needed / self.refill_per_sec,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_burst_up_to_capacity_then_blocks() {
        let rl = RateLimiter::per_minute(10);
        let t0 = Instant::now();
        for _ in 0..10 {
            assert!(rl.check_at("analyze_video", t0).is_ok());
        }
        // 11th within the same instant is rejected.
        let err = rl.check_at("analyze_video", t0).unwrap_err();
        assert_eq!(err.key, "analyze_video");
        assert!(err.retry_after_secs > 0.0);
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::per_minute(1);
        let t0 = Instant::now();
        assert!(rl.check_at("a", t0).is_ok());
        assert!(rl.check_at("a", t0).is_err());
        // Different key has its own bucket.
        assert!(rl.check_at("b", t0).is_ok());
    }

    #[test]
    fn refills_over_time() {
        let rl = RateLimiter::per_minute(60); // 1 token/sec
        let t0 = Instant::now();
        for _ in 0..60 {
            rl.check_at("k", t0).unwrap();
        }
        assert!(rl.check_at("k", t0).is_err());
        // After ~2 seconds, ~2 tokens are back.
        let t2 = t0 + Duration::from_secs(2);
        assert!(rl.check_at("k", t2).is_ok());
        assert!(rl.check_at("k", t2).is_ok());
    }
}
