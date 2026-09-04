# Tollgate

A from-scratch API gateway. Routing, auth, and rate limiting are a pure pipeline where every allow or deny carries an explicit reason, not a YAML-configured black box.

Tollgate is the policy layer you put in front of an upstream. It answers one question per request: forward it, or reject it and say exactly why. A 404 means no route matched. A 401 means the key was missing or unknown. A 403 means the key was known but lacked the route's scope. A 429 means the token bucket was empty, and the response tells the caller how many seconds to wait.

It is built with zero external dependencies, pure Rust std. It pairs with two other from-scratch builds: Rift (a reverse proxy) and Ferryman (a load balancer). Tollgate is the piece that decides who gets through.

## The gap it closes

Most gateways are a big config file and a black box. You set a rule, traffic gets denied, and you are left guessing which rule fired. Tollgate is the opposite. The whole policy is one function:

```
decide(config, request, now, buckets) -> Decision
```

`Decision` is either `Allow { upstream, request }` (with the request already shaped for the upstream) or `Reject { status, reason, headers }` (with the status, a human reason, and the headers to send, such as `WWW-Authenticate` on a 401 or `Retry-After` on a 429).

Two properties make this readable and testable:

1. It is pure. The only mutable state is the rate limiter passed in, and time only enters through `now`. Same inputs, same decision, every time.
2. The clock is injected. Nothing in the policy reads the system clock. Rate limiting is exercised over virtual time, so the tests prove a burst trips 429 and refills correctly without a single real sleep.

The socket layer is a thin shell around that function. The pipeline can be unit-tested with no network at all, and the one integration test drives the real TCP path end to end.

## Pipeline

Each stage is its own small module and short-circuits on the first rejection.

1. Routing (`route.rs`). Match by host, method, and path. Paths can be exact, a `:param` template, or a `prefix*`. Most specific wins: exact beats param beats prefix, and a longer prefix beats a shorter one. No match is a 404.
2. Auth (`auth.rs`). A route may require an API key via `X-API-Key` or `Authorization: Bearer <key>`. Missing or unknown key is a 401. A known key without the route's required scope is a 403.
3. Rate limiting (`ratelimit.rs`). A per-key token bucket, advanced by the injected clock, with a configurable rate and burst. It falls back to a per-IP bucket when the route is anonymous. Empty bucket is a 429 with a correct `Retry-After`. Buckets are isolated per key.
4. Shaping (`shape.rs`). Per-route header add, set, and remove, plus path prefix strip and add, applied before forwarding.

Then `Allow { upstream, shaped_request }`.

## Build and test

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

## CLI

Serve a config in front of an upstream:

```
tollgate serve examples/gateway.conf --addr 127.0.0.1:8080
```

Dry-run a request through the pipeline and print the decision and its reason. Repeat it to watch the bucket drain and trip 429. The clock is held fixed across repeats so a burst is visible:

```
tollgate check examples/minimal.conf --host localhost --path /api/widgets --key demo-key --repeat 5
```

```
[1/5] ALLOW 200 -> 127.0.0.1:9000  (path /api/widgets)
[2/5] ALLOW 200 -> 127.0.0.1:9000  (path /api/widgets)
[3/5] ALLOW 200 -> 127.0.0.1:9000  (path /api/widgets)
[4/5] REJECT 429  reason: rate limit exceeded, retry after 1s  [Retry-After: 1]
[5/5] REJECT 429  reason: rate limit exceeded, retry after 1s  [Retry-After: 1]
```

Print a parsed config back out (the parser has a matching printer, and they round-trip):

```
tollgate print examples/gateway.conf
```

## Config format

A readable, line-based text format. Keys first, then `route { ... }` blocks ordered however you like (routing picks the most specific match, not the first). Comments start with `#`.

```
key svc-reader scopes read:users,read:orders

route {
  method GET
  host api.example.com
  path /v1/users/:id
  auth scope read:users
  rate 5/s burst 10
  upstream 127.0.0.1:9002
  strip_prefix /v1
  set_header X-Tollgate proxied
  remove_header X-Internal-Debug
}
```

See `examples/gateway.conf` and `examples/minimal.conf`, and `DESIGN.md` for the full grammar and the reasoning behind the design.

## Playground

`docs/index.html` is a self-contained page that ports the pipeline to JavaScript. Configure routes, keys, and rate limits, send requests, and watch each one routed, authed, and rate limited with its status and reason, plus token buckets draining and refilling on a simulated clock. It mirrors the Rust behavior. Serve it with `python3 -m http.server` from `docs/`.

## Author

Pavan Nallamothu (pavanchow).
