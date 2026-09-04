use std::time::Duration;

use tollgate::{RateLimit, RateLimiter, RateOutcome};

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

#[test]
fn burst_is_allowed_then_next_is_limited() {
    let mut rl = RateLimiter::new();
    let limit = RateLimit { rate: 1.0, burst: 3.0 };

    for _ in 0..3 {
        assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    }
    // Fourth request at the same instant: bucket is empty.
    assert_eq!(
        rl.check("k", &limit, secs(0.0)),
        RateOutcome::Limited { retry_after: 1 }
    );
}

#[test]
fn tokens_refill_after_the_right_virtual_time() {
    let mut rl = RateLimiter::new();
    let limit = RateLimit { rate: 1.0, burst: 3.0 };

    for _ in 0..3 {
        assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    }
    assert!(matches!(
        rl.check("k", &limit, secs(0.0)),
        RateOutcome::Limited { .. }
    ));

    // Still short of a full token half a second later.
    assert!(matches!(
        rl.check("k", &limit, secs(0.5)),
        RateOutcome::Limited { .. }
    ));
    // One second in, exactly one token has refilled.
    assert_eq!(rl.check("k", &limit, secs(1.0)), RateOutcome::Allowed);
    // And it is spent again.
    assert!(matches!(
        rl.check("k", &limit, secs(1.0)),
        RateOutcome::Limited { .. }
    ));
}

#[test]
fn fractional_refill_is_continuous() {
    let mut rl = RateLimiter::new();
    let limit = RateLimit { rate: 2.0, burst: 2.0 };

    assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    assert!(matches!(
        rl.check("k", &limit, secs(0.0)),
        RateOutcome::Limited { .. }
    ));
    // At 2 tokens/sec, half a second buys exactly one token.
    assert_eq!(rl.check("k", &limit, secs(0.5)), RateOutcome::Allowed);
}

#[test]
fn retry_after_is_correct_for_slow_rate() {
    let mut rl = RateLimiter::new();
    // One token every 10 seconds.
    let limit = RateLimit { rate: 0.1, burst: 1.0 };

    assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    // Empty now; a full token is 10 seconds away.
    assert_eq!(
        rl.check("k", &limit, secs(0.0)),
        RateOutcome::Limited { retry_after: 10 }
    );
    // Three seconds later, 0.3 tokens present, 0.7 to go -> ceil(7s) = 7.
    assert_eq!(
        rl.check("k", &limit, secs(3.0)),
        RateOutcome::Limited { retry_after: 7 }
    );
}

#[test]
fn buckets_are_isolated_per_key() {
    let mut rl = RateLimiter::new();
    let limit = RateLimit { rate: 1.0, burst: 1.0 };

    assert_eq!(rl.check("a", &limit, secs(0.0)), RateOutcome::Allowed);
    // Draining key "a" must not touch key "b".
    assert!(matches!(
        rl.check("a", &limit, secs(0.0)),
        RateOutcome::Limited { .. }
    ));
    assert_eq!(rl.check("b", &limit, secs(0.0)), RateOutcome::Allowed);
}

#[test]
fn tokens_never_exceed_burst() {
    let mut rl = RateLimiter::new();
    let limit = RateLimit { rate: 5.0, burst: 2.0 };

    // Idle for a long time; the bucket caps at burst, not rate*time.
    assert_eq!(rl.check("k", &limit, secs(0.0)), RateOutcome::Allowed);
    assert_eq!(rl.check("k", &limit, secs(1000.0)), RateOutcome::Allowed);
    assert_eq!(rl.check("k", &limit, secs(1000.0)), RateOutcome::Allowed);
    assert!(matches!(
        rl.check("k", &limit, secs(1000.0)),
        RateOutcome::Limited { .. }
    ));
}
