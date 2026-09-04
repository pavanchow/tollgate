use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::error::RequestError;
use crate::pipeline::{decide, Decision};
use crate::ratelimit::RateLimiter;
use crate::request::{normalize_host, Request};

const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// A running gateway. Holds the shared config and the single rate limiter whose
/// buckets are shared across all connection threads behind a mutex.
pub struct Server {
    config: Arc<Config>,
    limiter: Arc<Mutex<RateLimiter>>,
    start: Instant,
}

impl Server {
    pub fn new(config: Config) -> Server {
        Server {
            config: Arc::new(config),
            limiter: Arc::new(Mutex::new(RateLimiter::new())),
            start: Instant::now(),
        }
    }

    /// Bind and serve forever, one thread per accepted connection.
    pub fn serve(&self, addr: &str) -> io::Result<()> {
        let listener = TcpListener::bind(addr)?;
        let bound = listener.local_addr()?;
        println!("tollgate listening on {bound}");
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let config = Arc::clone(&self.config);
                    let limiter = Arc::clone(&self.limiter);
                    let start = self.start;
                    thread::spawn(move || {
                        let _ = handle_connection(stream, &config, &limiter, start);
                    });
                }
                Err(e) => eprintln!("accept error: {e}"),
            }
        }
        Ok(())
    }
}

fn handle_connection(
    mut stream: TcpStream,
    config: &Config,
    limiter: &Mutex<RateLimiter>,
    start: Instant,
) -> io::Result<()> {
    let peer_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string());

    let mut reader = BufReader::new(stream.try_clone()?);
    let request = match parse_request(&mut reader, &peer_ip) {
        Ok(r) => r,
        Err(RequestError::Empty) => return Ok(()),
        Err(e) => {
            return write_simple(&mut stream, 400, "Bad Request", &e.to_string());
        }
    };

    let now = start.elapsed();
    let decision = {
        let mut guard = limiter.lock().unwrap_or_else(|p| p.into_inner());
        decide(config, &request, now, &mut guard)
    };

    match decision {
        Decision::Allow { upstream, request } => forward(&mut stream, &upstream, &request),
        Decision::Reject { status, reason, headers } => {
            write_rejection(&mut stream, status, &reason, &headers)
        }
    }
}

/// Parse the request line and headers with hard bounds, then the body if a
/// Content-Length is present. Never trusts a claimed length past MAX_BODY_BYTES,
/// and returns a typed error (rendered as 400) instead of panicking.
fn parse_request<R: BufRead>(reader: &mut R, peer_ip: &str) -> Result<Request, RequestError> {
    let mut consumed = 0usize;
    let request_line = read_line(reader, &mut consumed)?;
    if request_line.is_empty() {
        return Err(RequestError::Empty);
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(RequestError::BadRequestLine)?;
    let target = parts.next().ok_or(RequestError::BadRequestLine)?;
    let version = parts.next().ok_or(RequestError::BadRequestLine)?;
    if parts.next().is_some() {
        return Err(RequestError::BadRequestLine);
    }
    if !version.starts_with("HTTP/1.") {
        return Err(RequestError::UnsupportedVersion);
    }
    if !target.starts_with('/') {
        return Err(RequestError::BadRequestLine);
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let line = read_line(reader, &mut consumed)?;
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(RequestError::TooLarge);
        }
        let (name, value) = line.split_once(':').ok_or(RequestError::BadHeader)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(RequestError::BadHeader);
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }

    let host = headers
        .iter()
        .find(|(k, _)| k == "host")
        .map(|(_, v)| normalize_host(v))
        .unwrap_or_default();

    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(RequestError::TooLarge);
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).map_err(|_| RequestError::BadRequestLine)?;
    }

    Ok(Request {
        method: method.to_ascii_uppercase(),
        host,
        path,
        query,
        headers,
        body,
        client_ip: peer_ip.to_string(),
    })
}

fn read_line<R: BufRead>(reader: &mut R, consumed: &mut usize) -> Result<String, RequestError> {
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                *consumed += 1;
                if *consumed > MAX_HEAD_BYTES {
                    return Err(RequestError::TooLarge);
                }
                if byte[0] == b'\n' {
                    break;
                }
                if byte[0] != b'\r' {
                    buf.push(byte[0]);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(RequestError::BadRequestLine),
        }
    }
    String::from_utf8(buf).map_err(|_| RequestError::BadHeader)
}

/// Open a connection to the upstream, replay the shaped request, and copy the
/// response back verbatim. Uses `Connection: close` so reading to EOF is safe.
fn forward(client: &mut TcpStream, upstream: &str, req: &Request) -> io::Result<()> {
    let mut up = match TcpStream::connect(upstream) {
        Ok(s) => s,
        Err(e) => {
            let body = format!("upstream {upstream} unreachable: {e}");
            return write_rejection(client, 502, &body, &[]);
        }
    };
    up.set_read_timeout(Some(Duration::from_secs(30))).ok();

    let target = match &req.query {
        Some(q) => format!("{}?{}", req.path, q),
        None => req.path.clone(),
    };
    let mut head = format!("{} {} HTTP/1.1\r\n", req.method, target);
    for (name, value) in &req.headers {
        if name == "connection" {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");

    up.write_all(head.as_bytes())?;
    if !req.body.is_empty() {
        up.write_all(&req.body)?;
    }
    up.flush()?;

    let mut response = Vec::new();
    up.read_to_end(&mut response)?;
    client.write_all(&response)?;
    client.flush()
}

fn write_rejection(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
) -> io::Result<()> {
    let phrase = reason_phrase(status);
    let body = format!("{status} {phrase}: {reason}\n");
    let mut head = format!("HTTP/1.1 {status} {phrase}\r\n");
    head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str(&format!("X-Tollgate-Reason: {}\r\n", sanitize(reason)));
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn write_simple(stream: &mut TcpStream, status: u16, phrase: &str, reason: &str) -> io::Result<()> {
    let body = format!("{status} {phrase}: {reason}\n");
    let mut head = format!("HTTP/1.1 {status} {phrase}\r\n");
    head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str(&format!("X-Tollgate-Reason: {}\r\n", sanitize(reason)));
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| *c != '\r' && *c != '\n').collect()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        _ => "Error",
    }
}
