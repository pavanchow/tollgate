use tollgate::route::match_route;
use tollgate::{
    AuthRequirement, HostMatch, MethodMatch, PathPattern, Request, Route, Shaping,
};

fn route(method: MethodMatch, host: HostMatch, path: &str, upstream: &str) -> Route {
    Route {
        method,
        host,
        path: PathPattern::parse(path),
        auth: AuthRequirement::None,
        rate: None,
        upstream: upstream.to_string(),
        shaping: Shaping::default(),
    }
}

#[test]
fn no_match_is_absent() {
    let routes = vec![route(MethodMatch::Any, HostMatch::Any, "/known", "u")];
    let req = Request::new("GET", "h", "/unknown");
    assert!(match_route(&routes, &req).is_none());
}

#[test]
fn exact_beats_param_and_prefix() {
    let routes = vec![
        route(MethodMatch::Any, HostMatch::Any, "/users/*", "prefix"),
        route(MethodMatch::Any, HostMatch::Any, "/users/:id", "param"),
        route(MethodMatch::Any, HostMatch::Any, "/users/me", "exact"),
    ];
    let req = Request::new("GET", "h", "/users/me");
    assert_eq!(match_route(&routes, &req).unwrap().upstream, "exact");
}

#[test]
fn param_beats_prefix() {
    let routes = vec![
        route(MethodMatch::Any, HostMatch::Any, "/users/*", "prefix"),
        route(MethodMatch::Any, HostMatch::Any, "/users/:id", "param"),
    ];
    let req = Request::new("GET", "h", "/users/42");
    assert_eq!(match_route(&routes, &req).unwrap().upstream, "param");
}

#[test]
fn longer_prefix_wins() {
    let routes = vec![
        route(MethodMatch::Any, HostMatch::Any, "/*", "short"),
        route(MethodMatch::Any, HostMatch::Any, "/api/v1/*", "long"),
        route(MethodMatch::Any, HostMatch::Any, "/api/*", "mid"),
    ];
    let req = Request::new("GET", "h", "/api/v1/things");
    assert_eq!(match_route(&routes, &req).unwrap().upstream, "long");
}

#[test]
fn method_mismatch_does_not_match() {
    let routes = vec![route(
        MethodMatch::Exact("POST".into()),
        HostMatch::Any,
        "/thing",
        "u",
    )];
    let req = Request::new("GET", "h", "/thing");
    assert!(match_route(&routes, &req).is_none());
}

#[test]
fn host_mismatch_does_not_match() {
    let routes = vec![route(
        MethodMatch::Any,
        HostMatch::Exact("api.example.com".into()),
        "/thing",
        "u",
    )];
    let req = Request::new("GET", "other.example.com", "/thing");
    assert!(match_route(&routes, &req).is_none());
    let ok = Request::new("GET", "api.example.com", "/thing");
    assert!(match_route(&routes, &ok).is_some());
}

#[test]
fn param_requires_same_segment_count() {
    let routes = vec![route(MethodMatch::Any, HostMatch::Any, "/users/:id", "param")];
    // Too many segments.
    let deep = Request::new("GET", "h", "/users/42/posts");
    assert!(match_route(&routes, &deep).is_none());
    // Empty param segment.
    let empty = Request::new("GET", "h", "/users/");
    assert!(match_route(&routes, &empty).is_none());
}

#[test]
fn more_specific_param_wins() {
    let routes = vec![
        route(MethodMatch::Any, HostMatch::Any, "/:a/:b", "loose"),
        route(MethodMatch::Any, HostMatch::Any, "/users/:b", "tight"),
    ];
    let req = Request::new("GET", "h", "/users/42");
    assert_eq!(match_route(&routes, &req).unwrap().upstream, "tight");
}
