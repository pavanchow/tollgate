use std::collections::HashMap;
use std::time::Duration;

use crate::route::RateLimit;

/// Result of charging one request against a bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateOutcome {
    Allowed,
    /// Limited, with the number of whole seconds to advertise in `Retry-After`.
    Limited { retry_after: u64 },
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: Duration,
}

/// A set of per-key token buckets advanced by an injected clock.
///
/// Nothing here reads the system clock. Every call passes `now` (time since the
/// gateway started, in the wire path; an arbitrary value in tests), which is
/// what makes rate limiting deterministic and unit-testable without sleeping.
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter { buckets: HashMap::new() }
    }

    /// Charge one token to `key`'s bucket at time `now`. A fresh bucket starts
    /// full at `burst`. Tokens refill continuously at `rate` per second and are
    /// capped at `burst`. When under one token, the request is limited and the
    /// advertised `Retry-After` is the ceiling of the seconds until one token.
    pub fn check(&mut self, key: &str, limit: &RateLimit, now: Duration) -> RateOutcome {
        let bucket = self
            .buckets
            .entry(key.to_string())
            .or_insert(Bucket { tokens: limit.burst, last: now });

        let elapsed = now.saturating_sub(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * limit.rate).min(limit.burst);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            RateOutcome::Allowed
        } else {
            let deficit = 1.0 - bucket.tokens;
            let seconds = if limit.rate > 0.0 {
                (deficit / limit.rate).ceil() as u64
            } else {
                u64::MAX
            };
            RateOutcome::Limited { retry_after: seconds.max(1) }
        }
    }
}
