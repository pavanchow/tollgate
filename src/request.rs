/// An inbound request reduced to exactly what the policy pipeline needs.
///
/// Header names are stored lowercased so lookups are case-insensitive, which is
/// what HTTP requires. The `host` is the authority with any port stripped, and
/// `path` never contains the query string (routing matches on the path only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub host: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub client_ip: String,
}

impl Request {
    /// Convenience builder used by tests and the `check` CLI. Header names are
    /// lowercased and the host is normalized the same way the wire parser does.
    pub fn new(method: &str, host: &str, path: &str) -> Request {
        let (path_only, query) = split_query(path);
        Request {
            method: method.to_ascii_uppercase(),
            host: normalize_host(host),
            path: path_only,
            query,
            headers: Vec::new(),
            body: Vec::new(),
            client_ip: "0.0.0.0".to_string(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Request {
        self.set_header(name, value);
        self
    }

    pub fn with_client_ip(mut self, ip: &str) -> Request {
        self.client_ip = ip.to_string();
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        let name = name.to_ascii_lowercase();
        if let Some(slot) = self.headers.iter_mut().find(|(k, _)| *k == name) {
            slot.1 = value.to_string();
        } else {
            self.headers.push((name, value.to_string()));
        }
    }

    pub fn remove_header(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        self.headers.retain(|(k, _)| *k != name);
    }
}

pub fn normalize_host(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    match host.split_once(':') {
        Some((h, _port)) => h.to_string(),
        None => host,
    }
}

fn split_query(target: &str) -> (String, Option<String>) {
    match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    }
}
