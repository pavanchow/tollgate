use tollgate::auth::{authenticate, extract_key, AuthOutcome, Principal};
use tollgate::config::{ApiKey, Config};
use tollgate::route::AuthRequirement;
use tollgate::Request;

fn config() -> Config {
    Config {
        routes: Vec::new(),
        keys: vec![
            ApiKey { key: "k-read".into(), scopes: vec!["read".into()] },
            ApiKey { key: "k-admin".into(), scopes: vec!["read".into(), "write".into()] },
        ],
    }
}

#[test]
fn extracts_from_x_api_key() {
    let req = Request::new("GET", "h", "/").with_header("X-API-Key", "abc");
    assert_eq!(extract_key(&req).as_deref(), Some("abc"));
}

#[test]
fn extracts_from_bearer() {
    let req = Request::new("GET", "h", "/").with_header("Authorization", "Bearer xyz");
    assert_eq!(extract_key(&req).as_deref(), Some("xyz"));
}

#[test]
fn no_requirement_is_anonymous() {
    let req = Request::new("GET", "h", "/");
    assert_eq!(
        authenticate(&config(), &req, &AuthRequirement::None),
        AuthOutcome::Ok(Principal::Anonymous)
    );
}

#[test]
fn missing_key_is_unauthorized() {
    let req = Request::new("GET", "h", "/");
    let out = authenticate(&config(), &req, &AuthRequirement::Key { scope: None });
    assert!(matches!(out, AuthOutcome::Unauthorized(_)));
}

#[test]
fn unknown_key_is_unauthorized() {
    let req = Request::new("GET", "h", "/").with_header("X-API-Key", "nope");
    let out = authenticate(&config(), &req, &AuthRequirement::Key { scope: None });
    assert!(matches!(out, AuthOutcome::Unauthorized(_)));
}

#[test]
fn known_key_wrong_scope_is_forbidden() {
    let req = Request::new("GET", "h", "/").with_header("X-API-Key", "k-read");
    let out = authenticate(
        &config(),
        &req,
        &AuthRequirement::Key { scope: Some("write".into()) },
    );
    assert!(matches!(out, AuthOutcome::Forbidden(_)));
}

#[test]
fn correct_key_and_scope_is_ok() {
    let req = Request::new("GET", "h", "/").with_header("X-API-Key", "k-admin");
    let out = authenticate(
        &config(),
        &req,
        &AuthRequirement::Key { scope: Some("write".into()) },
    );
    assert_eq!(out, AuthOutcome::Ok(Principal::Key("k-admin".into())));
}
