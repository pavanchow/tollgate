use crate::config::Config;
use crate::request::Request;
use crate::route::AuthRequirement;

/// The identity a request carries once auth has run. `Anonymous` means the
/// route did not require a key, so rate limiting later falls back to the IP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Anonymous,
    Key(String),
}

/// Outcome of the auth stage. A rejection names exactly why: missing/unknown key
/// is a 401, a known key without the required scope is a 403.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Ok(Principal),
    Unauthorized(String),
    Forbidden(String),
}

/// Pull the presented key from `X-API-Key` or an `Authorization: Bearer <key>`.
pub fn extract_key(req: &Request) -> Option<String> {
    if let Some(k) = req.header("x-api-key") {
        let k = k.trim();
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    if let Some(auth) = req.header("authorization") {
        if let Some(rest) = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")) {
            let k = rest.trim();
            if !k.is_empty() {
                return Some(k.to_string());
            }
        }
    }
    None
}

/// Apply a route's auth requirement to a request against the configured key set.
pub fn authenticate(config: &Config, req: &Request, requirement: &AuthRequirement) -> AuthOutcome {
    let scope = match requirement {
        AuthRequirement::None => return AuthOutcome::Ok(Principal::Anonymous),
        AuthRequirement::Key { scope } => scope,
    };

    let presented = match extract_key(req) {
        Some(k) => k,
        None => return AuthOutcome::Unauthorized("missing api key".to_string()),
    };

    let key = match config.keys.iter().find(|k| k.key == presented) {
        Some(k) => k,
        None => return AuthOutcome::Unauthorized("unknown api key".to_string()),
    };

    if let Some(required) = scope {
        if !key.scopes.iter().any(|s| s == required) {
            return AuthOutcome::Forbidden(format!("key lacks required scope '{required}'"));
        }
    }

    AuthOutcome::Ok(Principal::Key(key.key.clone()))
}
