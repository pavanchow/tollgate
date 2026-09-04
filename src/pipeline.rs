use std::time::Duration;

use crate::auth::{authenticate, AuthOutcome, Principal};
use crate::config::Config;
use crate::ratelimit::{RateLimiter, RateOutcome};
use crate::request::Request;
use crate::route::match_route;
use crate::shape::shape_request;

/// The single verdict the pipeline produces for a request. Every variant carries
/// its reason: an Allow names the upstream and the request as shaped for it, a
/// Reject names the status, a human reason, and the response headers to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow { upstream: String, request: Request },
    Reject { status: u16, reason: String, headers: Vec<(String, String)> },
}

impl Decision {
    pub fn status(&self) -> u16 {
        match self {
            Decision::Allow { .. } => 200,
            Decision::Reject { status, .. } => *status,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Decision::Allow { .. } => "allowed",
            Decision::Reject { reason, .. } => reason,
        }
    }
}

/// The whole policy, as one pure function over (config, request, clock, buckets).
///
/// The only mutable state is the rate limiter, and time only enters through
/// `now`. Given the same inputs it always returns the same decision, which is
/// what lets every stage be unit-tested without a socket or a real clock.
///
/// Stages run in order and short-circuit on the first rejection:
/// route -> auth -> rate limit -> shape -> allow.
pub fn decide(
    config: &Config,
    req: &Request,
    now: Duration,
    limiter: &mut RateLimiter,
) -> Decision {
    // 1. Routing.
    let route = match match_route(&config.routes, req) {
        Some(r) => r,
        None => {
            return Decision::Reject {
                status: 404,
                reason: "no route matches host, method, and path".to_string(),
                headers: Vec::new(),
            }
        }
    };

    // 2. Auth.
    let principal = match authenticate(config, req, &route.auth) {
        AuthOutcome::Ok(p) => p,
        AuthOutcome::Unauthorized(reason) => {
            return Decision::Reject {
                status: 401,
                reason,
                headers: vec![(
                    "WWW-Authenticate".to_string(),
                    "Bearer realm=\"tollgate\"".to_string(),
                )],
            }
        }
        AuthOutcome::Forbidden(reason) => {
            return Decision::Reject { status: 403, reason, headers: Vec::new() }
        }
    };

    // 3. Rate limit. Keyed by the authenticated key, or the client IP when anon.
    if let Some(limit) = &route.rate {
        let bucket_key = match &principal {
            Principal::Key(k) => format!("key:{k}"),
            Principal::Anonymous => format!("ip:{}", req.client_ip),
        };
        if let RateOutcome::Limited { retry_after } = limiter.check(&bucket_key, limit, now) {
            return Decision::Reject {
                status: 429,
                reason: format!("rate limit exceeded, retry after {retry_after}s"),
                headers: vec![("Retry-After".to_string(), retry_after.to_string())],
            };
        }
    }

    // 4. Shaping, then allow.
    let shaped = shape_request(req.clone(), &route.shaping);
    Decision::Allow { upstream: route.upstream.clone(), request: shaped }
}
