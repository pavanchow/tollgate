//! Tollgate: a from-scratch API gateway built as a pure, testable pipeline.
//!
//! The policy is one function, [`pipeline::decide`], over a config, a request,
//! an injected clock, and a rate limiter. It returns a [`pipeline::Decision`]
//! that is either an Allow (with the upstream and the shaped request) or a
//! Reject (with the status, an explicit reason, and the response headers). The
//! `server` module is the only place that touches sockets; everything the
//! decision depends on can be unit-tested without one.

pub mod auth;
pub mod config;
pub mod error;
pub mod pipeline;
pub mod ratelimit;
pub mod request;
pub mod route;
pub mod server;
pub mod shape;

pub use config::{ApiKey, Config};
pub use pipeline::{decide, Decision};
pub use ratelimit::{RateLimiter, RateOutcome};
pub use request::Request;
pub use route::{
    AuthRequirement, HostMatch, MethodMatch, PathPattern, RateLimit, Route, Segment, Shaping,
};
pub use server::Server;
