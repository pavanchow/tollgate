use std::time::Duration;

use tollgate::{decide, Config, Decision, RateLimiter, Request};

const POLICY: &str = "\
key reader scopes read:users
key writer scopes read:users,write:users

route {
  method GET
  host api.example.com
  path /v1/users/:id
  auth scope read:users
  rate 2/s burst 2
  upstream 127.0.0.1:9000
  strip_prefix /v1
  set_header X-Tollgate proxied
}

route {
  method GET
  host api.example.com
  path /health
  upstream 127.0.0.1:9001
}
";

fn config() -> Config {
    Config::parse(POLICY).expect("policy parses")
}

fn get(path: &str) -> Request {
    Request::new("GET", "api.example.com", path).with_client_ip("10.0.0.1")
}

#[test]
fn unknown_route_is_404() {
    let d = decide(&config(), &get("/nope"), Duration::ZERO, &mut RateLimiter::new());
    assert_eq!(d.status(), 404);
}

#[test]
fn missing_key_is_401_with_challenge() {
    let d = decide(&config(), &get("/v1/users/1"), Duration::ZERO, &mut RateLimiter::new());
    match d {
        Decision::Reject { status, headers, .. } => {
            assert_eq!(status, 401);
            assert!(headers.iter().any(|(k, _)| k == "WWW-Authenticate"));
        }
        _ => panic!("expected 401"),
    }
}

#[test]
fn unknown_key_is_401() {
    let req = get("/v1/users/1").with_header("X-API-Key", "bogus");
    let d = decide(&config(), &req, Duration::ZERO, &mut RateLimiter::new());
    assert_eq!(d.status(), 401);
}

#[test]
fn wrong_scope_is_403() {
    // "reader" has read:users but this simulates a route needing more: use a
    // key that exists but lacks the scope by pointing at a write-only need.
    let policy = POLICY.replace("auth scope read:users", "auth scope write:users");
    let config = Config::parse(&policy).unwrap();
    let req = get("/v1/users/1").with_header("Authorization", "Bearer reader");
    let d = decide(&config, &req, Duration::ZERO, &mut RateLimiter::new());
    assert_eq!(d.status(), 403);
}

#[test]
fn correct_key_is_allowed_and_shaped() {
    let req = get("/v1/users/7").with_header("X-API-Key", "reader");
    let d = decide(&config(), &req, Duration::ZERO, &mut RateLimiter::new());
    match d {
        Decision::Allow { upstream, request } => {
            assert_eq!(upstream, "127.0.0.1:9000");
            // strip_prefix removed /v1.
            assert_eq!(request.path, "/users/7");
            assert_eq!(request.header("x-tollgate"), Some("proxied"));
        }
        _ => panic!("expected allow"),
    }
}

#[test]
fn rate_limit_trips_after_burst_with_retry_after() {
    let config = config();
    let req = get("/v1/users/7").with_header("X-API-Key", "reader");
    let mut rl = RateLimiter::new();

    assert_eq!(decide(&config, &req, Duration::ZERO, &mut rl).status(), 200);
    assert_eq!(decide(&config, &req, Duration::ZERO, &mut rl).status(), 200);
    let third = decide(&config, &req, Duration::ZERO, &mut rl);
    match third {
        Decision::Reject { status, headers, .. } => {
            assert_eq!(status, 429);
            let retry = headers.iter().find(|(k, _)| k == "Retry-After");
            assert_eq!(retry.map(|(_, v)| v.as_str()), Some("1"));
        }
        _ => panic!("expected 429"),
    }
}

#[test]
fn two_keys_do_not_share_a_bucket() {
    let config = config();
    let mut rl = RateLimiter::new();
    let reader = get("/v1/users/7").with_header("X-API-Key", "reader");
    let writer = get("/v1/users/7").with_header("X-API-Key", "writer");

    // Drain reader's bucket.
    assert_eq!(decide(&config, &reader, Duration::ZERO, &mut rl).status(), 200);
    assert_eq!(decide(&config, &reader, Duration::ZERO, &mut rl).status(), 200);
    assert_eq!(decide(&config, &reader, Duration::ZERO, &mut rl).status(), 429);
    // writer is untouched.
    assert_eq!(decide(&config, &writer, Duration::ZERO, &mut rl).status(), 200);
}

#[test]
fn health_route_needs_no_key() {
    let d = decide(&config(), &get("/health"), Duration::ZERO, &mut RateLimiter::new());
    assert_eq!(d.status(), 200);
}
