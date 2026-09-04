# Tollgate design

## The point

An API gateway you can read and unit-test. Routing, auth, and rate limiting are a pure pipeline, and every allow or deny carries an explicit reason. A 404 is no route. A 401 is a missing or invalid key. A 403 is a forbidden scope. A 429 is a drained token bucket, with a `Retry-After`. None of it is buried in a YAML-configured black box.

Tollgate is the policy layer in front of an upstream. It pairs with Rift (a reverse proxy) and Ferryman (a load balancer). Those move bytes. Tollgate decides whether the bytes are allowed to move at all, and it can always tell you why.

## The core idea: one pure function

The whole policy is:

```
decide(config, request, now, buckets) -> Decision
```

`Decision` is a two-variant enum:

- `Allow { upstream, request }` where `request` is already shaped for the upstream.
- `Reject { status, reason, headers }` where `status` is the HTTP code, `reason` is a human sentence, and `headers` are the response headers that belong with that rejection (a `WWW-Authenticate` challenge on a 401, a `Retry-After` on a 429).

Two decisions make this design what it is.

### It is pure

`decide` reads its inputs and returns a value. The only mutable state is the `RateLimiter` passed in by reference, and that state is explicit rather than hidden in a global. Given the same config, request, clock, and bucket state, it always returns the same decision. That is what makes each stage testable in isolation with no sockets.

### The clock is injected

No code inside the policy calls `Instant::now()` or reads a system clock. Time enters only as the `now: Duration` argument, measured as time since the gateway started. In the wire path the server computes `start.elapsed()` and passes it in. In tests we pass whatever value we want.

This is the whole trick behind deterministic rate limiting. To prove that a burst of N is allowed and the next request is 429, we call `decide` N+1 times with `now` held constant. To prove tokens refill after the right amount of time, we advance `now` by an exact `Duration` and assert. There is no `thread::sleep` anywhere in the tests, so they are fast and they never flake on timing.

## Module layout

Each stage is a small module with one job.

- `request.rs`: the inbound request reduced to what the policy needs (method, host with port stripped, path without query, headers lowercased for case-insensitive lookup, body, client IP).
- `config.rs`: the `Config` (routes plus keys), a parser for the text format, and a matching printer. The two round-trip.
- `route.rs`: the routing model (method, host, and the three path pattern kinds) and `match_route`, which returns the single most specific route.
- `auth.rs`: key extraction and the auth outcome (anonymous, ok, unauthorized, forbidden).
- `ratelimit.rs`: the token bucket store, advanced by the injected clock.
- `shape.rs`: request rewriting before forwarding.
- `pipeline.rs`: `decide`, which wires the stages together in order and defines `Decision`.
- `server.rs`: the only module that touches sockets. Bounded HTTP/1.1 parsing, the thread-per-connection accept loop, upstream forwarding, and rejection responses.
- `main.rs`: the `serve`, `check`, and `print` CLI commands.
- `error.rs`: config and request error types.

## Routing precedence

A path spec is one of three kinds, decided by its shape:

- Ends in `*`: a prefix match. `/api/*` matches any path starting with `/api/`.
- Contains a `:name` segment: a param template. `/users/:id` matches `/users/42` but not `/users/42/posts` (segment counts must match) and not `/users/` (a param segment cannot be empty).
- Otherwise: an exact match.

When more than one route matches a request, the most specific wins. The ranking is a tuple `(tier, specificity)` compared in order:

- tier: exact is 3, param is 2, prefix is 1. So exact beats param beats prefix.
- specificity within a tier: for a prefix it is the prefix length, so `/api/v1/*` beats `/api/*`. For a param template it is the number of literal (non-param) segments, so `/users/:id` beats `/:a/:id`. For exact it is the path length, though only one exact route can match a given path anyway.

Method and host must match before the path is even scored. A method mismatch or host mismatch means the route does not apply. If nothing matches, the decision is a 404.

## Auth

A route declares its requirement:

- `auth` with no scope: any valid key is enough.
- `auth scope <name>`: the key must exist and list that scope.
- no `auth` line: the route is anonymous.

The presented key is read from `X-API-Key`, or from `Authorization: Bearer <key>`. A missing or unknown key is a 401 and carries a `WWW-Authenticate` challenge. A known key that lacks the required scope is a 403. The distinction matters. A 401 says "who are you", a 403 says "you cannot do this", and Tollgate never blurs the two.

## Rate limiting

Each principal gets a token bucket. The principal is the authenticated key when the route required one, and the client IP otherwise, so anonymous routes are still throttled per caller.

A bucket has a `rate` (tokens per second) and a `burst` (capacity). It starts full. On each request the bucket refills continuously for the elapsed virtual time, capped at `burst`, then tries to spend one token. Fractional tokens are fine, which keeps refill smooth rather than stepped.

When the bucket has less than one token, the request is a 429. The `Retry-After` is the ceiling of the seconds until one whole token is available, `ceil((1 - tokens) / rate)`, with a floor of one second. So a bucket at 5 tokens per second advertises 1, and a bucket at one token per 10 seconds advertises up to 10. Buckets are keyed independently, so draining one key never touches another.

Ordering is deliberate: auth runs before rate limiting, so the bucket is keyed by the authenticated principal, and an unauthorized request never consumes a token.

## Request shaping

An allowed request is rewritten before forwarding, in a fixed order that is easy to reason about: strip the configured path prefix, add the configured path prefix, remove headers, set headers (replace or insert), then add headers (append even if present). The result is the exact request the upstream receives.

## The socket layer

`server.rs` is a thin shell around `decide`. It parses the request line and headers with hard bounds (a byte cap on the head, a header count cap, and a body cap that refuses an oversized `Content-Length`), so a malformed or hostile request returns a 400 and never panics or hangs the accept loop. It builds a `Request`, calls `decide` with `start.elapsed()` and the shared rate limiter, and then either opens a `TcpStream` to the upstream and relays the response, or writes the rejection with its status line, reason, and headers. Connections are handled one per thread, and the rate limiter is shared behind a mutex.

Forwarding uses `Connection: close` on the upstream request so reading the response to EOF is correct without parsing chunked framing. That keeps the relay simple and is enough for a policy gateway whose job is the decision, not connection pooling.

## Testing

The correctness gate lives in `tests/` and needs no network except the one integration test.

- `ratelimit.rs`: burst then limit, refill after exact virtual time, fractional refill, correct `Retry-After` at slow rates, per-key isolation, and the burst cap. All over the injected clock, no sleeping.
- `routing.rs`: exact over param over prefix, longer prefix wins, more specific param wins, method and host mismatch, param segment-count rules, and no-match.
- `auth.rs`: key extraction from both header forms, and the missing, unknown, wrong-scope, and correct-key outcomes.
- `config.rs`: parsing keys, routes, and shaping, the print-then-parse round trip, and the required-field errors.
- `shape.rs`: prefix strip and add, and header set, add, and remove semantics.
- `pipeline.rs`: the four rejection reasons and the allow path end to end through `decide`.
- `integration.rs`: starts a real mock upstream and the real gateway on ephemeral `127.0.0.1:0` ports, then over real TCP asserts a forwarded request with shaped headers, a 404, a 401 with a challenge, a 429 with `Retry-After`, and a 400 on a malformed request line after which the server keeps serving.

## Non-goals

This is a focused, readable gateway, not a production edge proxy. It does not do TLS termination, HTTP/2, chunked upstream streaming, connection pooling, or hot config reload. The design leaves room for those, but the point here is a policy engine you can read in an afternoon and trust because every decision is explicit and every stage is tested.
