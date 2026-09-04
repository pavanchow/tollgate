use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use tollgate::config::Config;
use tollgate::server::Server;

/// A tiny mock upstream: for every accepted connection it reads the request head
/// and replies 200 with a fixed body plus an echo of the shaped X-Tollgate header
/// so the test can prove shaping reached the upstream. Runs until the process ends.
fn spawn_mock_upstream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let head = read_head(&mut stream);
            let echoed = header_value(&head, "x-tollgate").unwrap_or_default();
            let body = format!("hello from upstream (x-tollgate={echoed})");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    addr
}

fn spawn_gateway(config: Config) -> String {
    // Bind first to learn the ephemeral port, then hand off to the server thread.
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap().to_string();
    drop(probe);
    let serve_addr = addr.clone();
    thread::spawn(move || {
        let server = Server::new(config);
        let _ = server.serve(&serve_addr);
    });
    wait_until_listening(&addr);
    addr
}

fn wait_until_listening(addr: &str) {
    for _ in 0..100 {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("gateway never came up on {addr}");
}

fn read_head(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while let Ok(1) = stream.read(&mut byte) {
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 16 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().to_string())
}

/// Send a raw request over TCP and return the full response as a string.
fn send_raw(addr: &str, raw: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.flush().unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    resp
}

fn status_line(resp: &str) -> &str {
    resp.lines().next().unwrap_or("")
}

fn policy(upstream: &str) -> Config {
    let text = format!(
        "\
key reader scopes read

route {{
  method GET
  host gw.local
  path /v1/thing/:id
  auth scope read
  rate 1/s burst 2
  upstream {upstream}
  strip_prefix /v1
  set_header X-Tollgate proxied
}}
"
    );
    Config::parse(&text).unwrap()
}

#[test]
fn end_to_end_over_real_tcp() {
    let upstream = spawn_mock_upstream();
    let gw = spawn_gateway(policy(&upstream));

    // 1. A routed, authed request is forwarded and the upstream body comes back
    //    with the shaped header applied (the mock echoes X-Tollgate).
    let ok = send_raw(
        &gw,
        "GET /v1/thing/42 HTTP/1.1\r\nHost: gw.local\r\nX-API-Key: reader\r\nConnection: close\r\n\r\n",
    );
    assert!(status_line(&ok).contains("200"), "status: {}", status_line(&ok));
    assert!(ok.contains("hello from upstream"), "body missing: {ok}");
    assert!(ok.contains("x-tollgate=proxied"), "shaping not applied: {ok}");

    // 2. Unknown route -> 404.
    let notfound = send_raw(
        &gw,
        "GET /nope HTTP/1.1\r\nHost: gw.local\r\nX-API-Key: reader\r\nConnection: close\r\n\r\n",
    );
    assert!(status_line(&notfound).contains("404"), "got: {}", status_line(&notfound));

    // 3. No key -> 401 with a challenge header.
    let unauth = send_raw(
        &gw,
        "GET /v1/thing/42 HTTP/1.1\r\nHost: gw.local\r\nConnection: close\r\n\r\n",
    );
    assert!(status_line(&unauth).contains("401"), "got: {}", status_line(&unauth));
    assert!(unauth.to_lowercase().contains("www-authenticate"), "no challenge: {unauth}");

    // 4. Exceed the rate limit -> 429 with Retry-After. Burst is 2, so after two
    //    accepted requests the next in the same window is limited.
    let key_req =
        "GET /v1/thing/9 HTTP/1.1\r\nHost: gw.local\r\nX-API-Key: reader\r\nConnection: close\r\n\r\n";
    let _ = send_raw(&gw, key_req);
    let _ = send_raw(&gw, key_req);
    let limited = send_raw(&gw, key_req);
    assert!(status_line(&limited).contains("429"), "got: {}", status_line(&limited));
    assert!(limited.to_lowercase().contains("retry-after"), "no retry-after: {limited}");

    // 5. A malformed request line -> 400, and the server keeps serving after it.
    let bad = send_raw(&gw, "TOTALLY NOT HTTP\r\n\r\n");
    assert!(status_line(&bad).contains("400"), "got: {}", status_line(&bad));
    let after = send_raw(
        &gw,
        "GET /nope HTTP/1.1\r\nHost: gw.local\r\nConnection: close\r\n\r\n",
    );
    assert!(status_line(&after).contains("404"), "server died after bad input: {}", status_line(&after));
}
