use tollgate::route::Shaping;
use tollgate::shape::shape_request;
use tollgate::Request;

#[test]
fn strips_and_adds_prefix() {
    let shaping = Shaping {
        strip_prefix: Some("/v1".into()),
        add_prefix: Some("/internal".into()),
        ..Shaping::default()
    };
    let req = Request::new("GET", "h", "/v1/users/7");
    let out = shape_request(req, &shaping);
    assert_eq!(out.path, "/internal/users/7");
}

#[test]
fn strip_that_does_not_match_is_a_noop() {
    let shaping = Shaping { strip_prefix: Some("/v2".into()), ..Shaping::default() };
    let req = Request::new("GET", "h", "/v1/users/7");
    let out = shape_request(req, &shaping);
    assert_eq!(out.path, "/v1/users/7");
}

#[test]
fn set_replaces_and_remove_deletes() {
    let shaping = Shaping {
        remove_headers: vec!["X-Secret".into()],
        set_headers: vec![("X-Env".into(), "prod".into())],
        ..Shaping::default()
    };
    let req = Request::new("GET", "h", "/")
        .with_header("X-Secret", "shh")
        .with_header("X-Env", "staging");
    let out = shape_request(req, &shaping);
    assert_eq!(out.header("x-secret"), None);
    assert_eq!(out.header("x-env"), Some("prod"));
}

#[test]
fn add_appends_even_when_present() {
    let shaping = Shaping {
        add_headers: vec![("X-Trace".into(), "b".into())],
        ..Shaping::default()
    };
    let req = Request::new("GET", "h", "/").with_header("X-Trace", "a");
    let out = shape_request(req, &shaping);
    let traces: Vec<&str> = out
        .headers
        .iter()
        .filter(|(k, _)| k == "x-trace")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(traces, vec!["a", "b"]);
}
