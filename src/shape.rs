use crate::request::Request;
use crate::route::Shaping;

/// Rewrite a request per a route's shaping rules before it is forwarded.
///
/// Order is deliberate and easy to reason about: strip prefix, then add prefix,
/// then remove headers, then set (replace-or-insert), then add (append even if
/// present). The returned request is what the upstream actually receives.
pub fn shape_request(mut req: Request, shaping: &Shaping) -> Request {
    if let Some(strip) = &shaping.strip_prefix {
        if let Some(rest) = req.path.strip_prefix(strip.as_str()) {
            req.path = ensure_leading_slash(rest);
        }
    }
    if let Some(add) = &shaping.add_prefix {
        req.path = format!("{}{}", add, req.path);
    }

    for name in &shaping.remove_headers {
        req.remove_header(name);
    }
    for (name, value) in &shaping.set_headers {
        req.set_header(name, value);
    }
    for (name, value) in &shaping.add_headers {
        req.headers.push((name.to_ascii_lowercase(), value.clone()));
    }

    req
}

fn ensure_leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}
