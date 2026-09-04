use crate::request::Request;

/// How a route matches the request method: any verb, or one specific verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodMatch {
    Any,
    Exact(String),
}

impl MethodMatch {
    fn matches(&self, method: &str) -> bool {
        match self {
            MethodMatch::Any => true,
            MethodMatch::Exact(m) => m.eq_ignore_ascii_case(method),
        }
    }
}

/// How a route matches the request host: any host, or one specific host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMatch {
    Any,
    Exact(String),
}

impl HostMatch {
    fn matches(&self, host: &str) -> bool {
        match self {
            HostMatch::Any => true,
            HostMatch::Exact(h) => h.eq_ignore_ascii_case(host),
        }
    }
}

/// One path segment in a `:param` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    Param(String),
}

/// The three ways a route can match a path. This is the whole routing model:
/// an exact string, a `:param` template, or a prefix ending in `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathPattern {
    Exact(String),
    Param(Vec<Segment>),
    Prefix(String),
}

impl PathPattern {
    /// Parse a path spec from config. Trailing `*` means prefix, a `:name`
    /// segment means a param template, otherwise the path is matched exactly.
    pub fn parse(spec: &str) -> PathPattern {
        if let Some(prefix) = spec.strip_suffix('*') {
            return PathPattern::Prefix(prefix.to_string());
        }
        if spec.split('/').any(|s| s.starts_with(':')) {
            let segs = spec
                .split('/')
                .map(|s| {
                    if let Some(name) = s.strip_prefix(':') {
                        Segment::Param(name.to_string())
                    } else {
                        Segment::Literal(s.to_string())
                    }
                })
                .collect();
            return PathPattern::Param(segs);
        }
        PathPattern::Exact(spec.to_string())
    }

    /// Render back to the config spec form. Inverse of `parse`.
    pub fn to_spec(&self) -> String {
        match self {
            PathPattern::Exact(p) => p.clone(),
            PathPattern::Prefix(p) => format!("{p}*"),
            PathPattern::Param(segs) => segs
                .iter()
                .map(|s| match s {
                    Segment::Literal(l) => l.clone(),
                    Segment::Param(p) => format!(":{p}"),
                })
                .collect::<Vec<_>>()
                .join("/"),
        }
    }

    fn score(&self, path: &str) -> Option<MatchScore> {
        match self {
            PathPattern::Exact(p) => {
                if p == path {
                    // Exact is the most specific tier; specificity ties broken by length.
                    Some(MatchScore { tier: 3, specificity: p.len() })
                } else {
                    None
                }
            }
            PathPattern::Param(segs) => {
                let parts: Vec<&str> = path.split('/').collect();
                if parts.len() != segs.len() {
                    return None;
                }
                let mut literals = 0usize;
                for (seg, part) in segs.iter().zip(parts.iter()) {
                    match seg {
                        Segment::Literal(l) => {
                            if l != part {
                                return None;
                            }
                            literals += 1;
                        }
                        Segment::Param(_) => {
                            if part.is_empty() {
                                return None;
                            }
                        }
                    }
                }
                // More literal segments means a more specific param template.
                Some(MatchScore { tier: 2, specificity: literals })
            }
            PathPattern::Prefix(prefix) => {
                if path.starts_with(prefix.as_str()) {
                    // Longer prefixes win over shorter ones at the same tier.
                    Some(MatchScore { tier: 1, specificity: prefix.len() })
                } else {
                    None
                }
            }
        }
    }
}

/// Ranks a matched route. Higher tier wins first (exact > param > prefix), then
/// higher specificity within a tier (longer prefix, more literal segments).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MatchScore {
    pub tier: u8,
    pub specificity: usize,
}

/// What a route demands of the caller before it may pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthRequirement {
    /// No key needed; the caller is anonymous and rate limited by IP.
    None,
    /// A valid key is required. `scope` (when set) must be one of the key's scopes.
    Key { scope: Option<String> },
}

/// Per-route token bucket settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimit {
    /// Tokens added per second.
    pub rate: f64,
    /// Bucket capacity (the largest instantaneous burst).
    pub burst: f64,
}

/// Header and path rewrites applied to an allowed request before forwarding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shaping {
    pub remove_headers: Vec<String>,
    pub set_headers: Vec<(String, String)>,
    pub add_headers: Vec<(String, String)>,
    pub strip_prefix: Option<String>,
    pub add_prefix: Option<String>,
}

/// A single routing rule and everything the pipeline needs once it matches.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub method: MethodMatch,
    pub host: HostMatch,
    pub path: PathPattern,
    pub auth: AuthRequirement,
    pub rate: Option<RateLimit>,
    pub upstream: String,
    pub shaping: Shaping,
}

impl Route {
    fn score(&self, req: &Request) -> Option<MatchScore> {
        if !self.method.matches(&req.method) {
            return None;
        }
        if !self.host.matches(&req.host) {
            return None;
        }
        self.path.score(&req.path)
    }
}

/// Find the single most specific route for a request, or `None` for a 404.
pub fn match_route<'a>(routes: &'a [Route], req: &Request) -> Option<&'a Route> {
    let mut best: Option<(&Route, MatchScore)> = None;
    for route in routes {
        if let Some(score) = route.score(req) {
            match best {
                Some((_, best_score)) if best_score >= score => {}
                _ => best = Some((route, score)),
            }
        }
    }
    best.map(|(r, _)| r)
}
