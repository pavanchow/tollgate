use std::fmt::Write as _;

use crate::error::ConfigError;
use crate::route::{
    AuthRequirement, HostMatch, MethodMatch, PathPattern, RateLimit, Route, Shaping,
};

/// A configured API key and the scopes it is allowed to exercise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    pub key: String,
    pub scopes: Vec<String>,
}

/// The full gateway policy: the ordered route table plus the known keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub routes: Vec<Route>,
    pub keys: Vec<ApiKey>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        parse(text)
    }

    pub fn to_text(&self) -> String {
        print_config(self)
    }
}

// ----- parsing -----------------------------------------------------------

/// Parse the line-based config format. See DESIGN.md for the grammar; briefly:
/// `key <id> [scopes a,b]` lines at the top level, and `route { ... }` blocks
/// whose lines set method/host/path/auth/rate/upstream and shaping directives.
fn parse(text: &str) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    let mut lines = text.lines().enumerate();
    let mut block_count = 0usize;

    while let Some((idx, raw)) = lines.next() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let lineno = idx + 1;
        let (head, rest) = split_first(line);

        match head {
            "key" => config.keys.push(parse_key(rest, lineno)?),
            "route" => {
                if rest.trim() != "{" {
                    return Err(ConfigError::Parse {
                        line: lineno,
                        message: "expected 'route {' to open a route block".to_string(),
                    });
                }
                block_count += 1;
                let route = parse_route(&mut lines, block_count)?;
                config.routes.push(route);
            }
            other => {
                return Err(ConfigError::Parse {
                    line: lineno,
                    message: format!("unknown directive '{other}'"),
                })
            }
        }
    }

    Ok(config)
}

fn parse_key(rest: &str, lineno: usize) -> Result<ApiKey, ConfigError> {
    let (id, tail) = split_first(rest);
    if id.is_empty() {
        return Err(ConfigError::Parse {
            line: lineno,
            message: "key directive needs an identifier".to_string(),
        });
    }
    let mut scopes = Vec::new();
    let tail = tail.trim();
    if !tail.is_empty() {
        let (kw, list) = split_first(tail);
        if kw != "scopes" {
            return Err(ConfigError::Parse {
                line: lineno,
                message: format!("expected 'scopes' after key id, found '{kw}'"),
            });
        }
        scopes = list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    Ok(ApiKey { key: id.to_string(), scopes })
}

fn parse_route<'a, I>(lines: &mut I, block: usize) -> Result<Route, ConfigError>
where
    I: Iterator<Item = (usize, &'a str)>,
{
    let mut method = MethodMatch::Any;
    let mut host = HostMatch::Any;
    let mut path: Option<PathPattern> = None;
    let mut auth = AuthRequirement::None;
    let mut rate: Option<RateLimit> = None;
    let mut upstream: Option<String> = None;
    let mut shaping = Shaping::default();

    for (idx, raw) in lines.by_ref() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let lineno = idx + 1;
        if line == "}" {
            let path = path.ok_or(ConfigError::Missing { block, field: "path" })?;
            let upstream = upstream.ok_or(ConfigError::Missing { block, field: "upstream" })?;
            return Ok(Route { method, host, path, auth, rate, upstream, shaping });
        }

        let (key, value) = split_first(line);
        let value = value.trim();
        match key {
            "method" => method = MethodMatch::Exact(value.to_ascii_uppercase()),
            "host" => {
                host = if value == "*" {
                    HostMatch::Any
                } else {
                    HostMatch::Exact(value.to_ascii_lowercase())
                }
            }
            "path" => path = Some(PathPattern::parse(value)),
            "auth" => auth = parse_auth(value)?,
            "rate" => rate = Some(parse_rate(value, lineno)?),
            "upstream" => upstream = Some(value.to_string()),
            "strip_prefix" => shaping.strip_prefix = Some(value.to_string()),
            "add_prefix" => shaping.add_prefix = Some(value.to_string()),
            "set_header" => shaping.set_headers.push(parse_header(value, lineno)?),
            "add_header" => shaping.add_headers.push(parse_header(value, lineno)?),
            "remove_header" => shaping.remove_headers.push(value.to_string()),
            other => {
                return Err(ConfigError::Parse {
                    line: lineno,
                    message: format!("unknown route field '{other}'"),
                })
            }
        }
    }

    Err(ConfigError::Missing { block, field: "closing '}'" })
}

fn parse_auth(value: &str) -> Result<AuthRequirement, ConfigError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(AuthRequirement::Key { scope: None });
    }
    let (kw, scope) = split_first(value);
    if kw != "scope" {
        return Err(ConfigError::Parse {
            line: 0,
            message: format!("expected 'auth' or 'auth scope <name>', found 'auth {value}'"),
        });
    }
    Ok(AuthRequirement::Key { scope: Some(scope.trim().to_string()) })
}

/// Parse `rate <n>/s burst <b>`, e.g. `rate 5/s burst 10` or `rate 0.5/s burst 1`.
fn parse_rate(value: &str, lineno: usize) -> Result<RateLimit, ConfigError> {
    let err = |m: String| ConfigError::Parse { line: lineno, message: m };
    let mut it = value.split_whitespace();
    let rate_tok = it.next().ok_or_else(|| err("rate needs a value".to_string()))?;
    let per = rate_tok
        .strip_suffix("/s")
        .ok_or_else(|| err(format!("rate '{rate_tok}' must end in '/s'")))?;
    let rate: f64 = per
        .parse()
        .map_err(|_| err(format!("rate '{per}' is not a number")))?;

    let burst_kw = it.next().ok_or_else(|| err("rate needs 'burst <n>'".to_string()))?;
    if burst_kw != "burst" {
        return Err(err(format!("expected 'burst', found '{burst_kw}'")));
    }
    let burst_tok = it.next().ok_or_else(|| err("burst needs a value".to_string()))?;
    let burst: f64 = burst_tok
        .parse()
        .map_err(|_| err(format!("burst '{burst_tok}' is not a number")))?;

    if rate < 0.0 || burst < 0.0 {
        return Err(err("rate and burst must be non-negative".to_string()));
    }
    Ok(RateLimit { rate, burst })
}

fn parse_header(value: &str, lineno: usize) -> Result<(String, String), ConfigError> {
    let (name, val) = split_first(value);
    if name.is_empty() {
        return Err(ConfigError::Parse {
            line: lineno,
            message: "header directive needs a name".to_string(),
        });
    }
    Ok((name.to_string(), val.trim().to_string()))
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn split_first(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

// ----- printing ----------------------------------------------------------

fn print_config(config: &Config) -> String {
    let mut out = String::new();
    for key in &config.keys {
        if key.scopes.is_empty() {
            let _ = writeln!(out, "key {}", key.key);
        } else {
            let _ = writeln!(out, "key {} scopes {}", key.key, key.scopes.join(","));
        }
    }
    if !config.keys.is_empty() {
        out.push('\n');
    }

    for route in &config.routes {
        print_route(&mut out, route);
        out.push('\n');
    }
    out
}

fn print_route(out: &mut String, route: &Route) {
    let _ = writeln!(out, "route {{");
    if let MethodMatch::Exact(m) = &route.method {
        let _ = writeln!(out, "  method {m}");
    }
    if let HostMatch::Exact(h) = &route.host {
        let _ = writeln!(out, "  host {h}");
    }
    let _ = writeln!(out, "  path {}", route.path.to_spec());
    match &route.auth {
        AuthRequirement::None => {}
        AuthRequirement::Key { scope: None } => {
            let _ = writeln!(out, "  auth");
        }
        AuthRequirement::Key { scope: Some(s) } => {
            let _ = writeln!(out, "  auth scope {s}");
        }
    }
    if let Some(rate) = &route.rate {
        let _ = writeln!(out, "  rate {}/s burst {}", fmt_num(rate.rate), fmt_num(rate.burst));
    }
    let _ = writeln!(out, "  upstream {}", route.upstream);
    if let Some(p) = &route.shaping.strip_prefix {
        let _ = writeln!(out, "  strip_prefix {p}");
    }
    if let Some(p) = &route.shaping.add_prefix {
        let _ = writeln!(out, "  add_prefix {p}");
    }
    for n in &route.shaping.remove_headers {
        let _ = writeln!(out, "  remove_header {n}");
    }
    for (n, v) in &route.shaping.set_headers {
        let _ = writeln!(out, "  set_header {n} {v}");
    }
    for (n, v) in &route.shaping.add_headers {
        let _ = writeln!(out, "  add_header {n} {v}");
    }
    out.push_str("}\n");
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}
