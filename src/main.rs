use std::process::ExitCode;
use std::time::Duration;

use tollgate::config::Config;
use tollgate::pipeline::{decide, Decision};
use tollgate::ratelimit::RateLimiter;
use tollgate::request::Request;
use tollgate::server::Server;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("serve") => cmd_serve(&args[1..]),
        Some("check") => cmd_check(&args[1..]),
        Some("print") => cmd_print(&args[1..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            print_usage();
            Err(format!("unknown command '{other}'"))
        }
    }
}

fn cmd_serve(args: &[String]) -> Result<(), String> {
    let mut path = None;
    let mut addr = "127.0.0.1:8080".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                i += 1;
                addr = args.get(i).ok_or("--addr needs a value")?.clone();
            }
            other if path.is_none() => path = Some(other.to_string()),
            other => return Err(format!("unexpected argument '{other}'")),
        }
        i += 1;
    }
    let path = path.ok_or("usage: tollgate serve <config> [--addr host:port]")?;
    let config = load_config(&path)?;
    let server = Server::new(config);
    server.serve(&addr).map_err(|e| format!("serve failed: {e}"))
}

/// Dry-run one request (optionally repeated) through the pipeline and print each
/// Decision with its reason. The clock stays at zero across repeats so a burst
/// visibly drains the bucket and trips 429, which is exactly the demo we want.
fn cmd_check(args: &[String]) -> Result<(), String> {
    let mut path = None;
    let mut method = "GET".to_string();
    let mut host = "localhost".to_string();
    let mut req_path = "/".to_string();
    let mut key: Option<String> = None;
    let mut client_ip = "127.0.0.1".to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut repeat: u32 = 1;

    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| args.get(i + 1).cloned().ok_or(format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "--method" => {
                method = need(i)?;
                i += 1;
            }
            "--host" => {
                host = need(i)?;
                i += 1;
            }
            "--path" => {
                req_path = need(i)?;
                i += 1;
            }
            "--key" => {
                key = Some(need(i)?);
                i += 1;
            }
            "--ip" => {
                client_ip = need(i)?;
                i += 1;
            }
            "--repeat" => {
                repeat = need(i)?.parse().map_err(|_| "--repeat needs a number")?;
                i += 1;
            }
            "--header" => {
                let raw = need(i)?;
                let (n, v) = raw.split_once(':').ok_or("--header must be name:value")?;
                headers.push((n.trim().to_string(), v.trim().to_string()));
                i += 1;
            }
            other if path.is_none() => path = Some(other.to_string()),
            other => return Err(format!("unexpected argument '{other}'")),
        }
        i += 1;
    }

    let path = path.ok_or("usage: tollgate check <config> [--method M --host H --path P ...]")?;
    let config = load_config(&path)?;

    let mut request = Request::new(&method, &host, &req_path).with_client_ip(&client_ip);
    for (n, v) in &headers {
        request.set_header(n, v);
    }
    if let Some(k) = &key {
        request.set_header("X-API-Key", k);
    }

    let mut limiter = RateLimiter::new();
    let now = Duration::ZERO;
    for n in 1..=repeat {
        let decision = decide(&config, &request, now, &mut limiter);
        print_decision(n, repeat, &decision);
    }
    Ok(())
}

fn cmd_print(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("usage: tollgate print <config>")?;
    let config = load_config(path)?;
    print!("{}", config.to_text());
    Ok(())
}

fn print_decision(n: u32, total: u32, decision: &Decision) {
    let prefix = if total > 1 { format!("[{n}/{total}] ") } else { String::new() };
    match decision {
        Decision::Allow { upstream, request } => {
            println!("{prefix}ALLOW 200 -> {upstream}  (path {})", request.path);
        }
        Decision::Reject { status, reason, headers } => {
            let extra = headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            if extra.is_empty() {
                println!("{prefix}REJECT {status}  reason: {reason}");
            } else {
                println!("{prefix}REJECT {status}  reason: {reason}  [{extra}]");
            }
        }
    }
}

fn load_config(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    Config::parse(&text).map_err(|e| e.to_string())
}

fn print_usage() {
    eprintln!(
        "tollgate - a from-scratch API gateway\n\n\
         USAGE:\n\
         \x20 tollgate serve <config> [--addr host:port]\n\
         \x20 tollgate check <config> [--method M] [--host H] [--path P]\n\
         \x20                         [--key K] [--ip IP] [--header name:value] [--repeat N]\n\
         \x20 tollgate print <config>\n"
    );
}
