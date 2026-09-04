use tollgate::config::Config;
use tollgate::route::{AuthRequirement, HostMatch, MethodMatch, PathPattern};

#[test]
fn parses_keys_routes_and_shaping() {
    let text = "\
key svc scopes read,write

route {
  method POST
  host api.example.com
  path /v1/users/:id
  auth scope write
  rate 0.5/s burst 3
  upstream 10.0.0.5:8080
  strip_prefix /v1
  add_prefix /internal
  set_header X-Gateway tollgate
  add_header X-Trace abc
  remove_header X-Secret
}
";
    let config = Config::parse(text).unwrap();
    assert_eq!(config.keys.len(), 1);
    assert_eq!(config.keys[0].key, "svc");
    assert_eq!(config.keys[0].scopes, vec!["read", "write"]);

    assert_eq!(config.routes.len(), 1);
    let r = &config.routes[0];
    assert_eq!(r.method, MethodMatch::Exact("POST".into()));
    assert_eq!(r.host, HostMatch::Exact("api.example.com".into()));
    assert!(matches!(r.path, PathPattern::Param(_)));
    assert_eq!(r.auth, AuthRequirement::Key { scope: Some("write".into()) });
    let rate = r.rate.as_ref().unwrap();
    assert_eq!(rate.rate, 0.5);
    assert_eq!(rate.burst, 3.0);
    assert_eq!(r.upstream, "10.0.0.5:8080");
    assert_eq!(r.shaping.strip_prefix.as_deref(), Some("/v1"));
    assert_eq!(r.shaping.add_prefix.as_deref(), Some("/internal"));
    assert_eq!(r.shaping.set_headers, vec![("X-Gateway".into(), "tollgate".into())]);
    assert_eq!(r.shaping.add_headers, vec![("X-Trace".into(), "abc".into())]);
    assert_eq!(r.shaping.remove_headers, vec!["X-Secret".to_string()]);
}

#[test]
fn print_then_parse_round_trips() {
    let text = "\
key a scopes read
key b

route {
  method GET
  host h.example.com
  path /users/:id
  auth scope read
  rate 5/s burst 10
  upstream 127.0.0.1:9000
  strip_prefix /v1
  set_header X-A one
}

route {
  path /public/*
  upstream 127.0.0.1:9001
}
";
    let first = Config::parse(text).unwrap();
    let printed = first.to_text();
    let second = Config::parse(&printed).unwrap();
    assert_eq!(first, second, "printed config must parse back to an equal value");
}

#[test]
fn auth_without_scope_requires_only_a_key() {
    let text = "\
route {
  path /x
  auth
  upstream 127.0.0.1:9000
}
";
    let config = Config::parse(text).unwrap();
    assert_eq!(config.routes[0].auth, AuthRequirement::Key { scope: None });
}

#[test]
fn missing_upstream_is_an_error() {
    let text = "route {\n  path /x\n}\n";
    let err = Config::parse(text).unwrap_err();
    assert!(err.to_string().contains("upstream"), "got: {err}");
}

#[test]
fn missing_path_is_an_error() {
    let text = "route {\n  upstream 127.0.0.1:9000\n}\n";
    let err = Config::parse(text).unwrap_err();
    assert!(err.to_string().contains("path"), "got: {err}");
}

#[test]
fn unknown_directive_is_an_error() {
    let err = Config::parse("wat foo\n").unwrap_err();
    assert!(err.to_string().contains("unknown directive"), "got: {err}");
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let text = "\
# a comment
key k   # trailing comment

route {
  # inside the block
  path /x
  upstream 127.0.0.1:9000
}
";
    let config = Config::parse(text).unwrap();
    assert_eq!(config.keys.len(), 1);
    assert_eq!(config.routes.len(), 1);
}
