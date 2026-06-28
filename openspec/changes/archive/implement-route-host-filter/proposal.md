# implement-route-host-filter

## Summary
Implement the route-level `host` filter that is parsed in `RouteMatch.host` but never checked in `match_route`.

## Motivation
`RouteMatch.host: Option<String>` is defined at `src/config/types.rs:315-316` and documented as "Optional host pattern to restrict this route to a specific subdomain." However, `match_route` (`src/proxy/router.rs:138-181`) only checks site-level domains (`router.rs:146-150`) and completely ignores the route-level host field. A route configured with `host: "api.example.com"` silently matches all hosts — a config knob that does nothing.

## Design
1. Add a `host_matches(pattern: &str, host_header: &str) -> bool` function in `router.rs`.
   - Strip the port from `host_header` (split on `:` take first part).
   - Exact match: `pattern == host_no_port`.
   - Wildcard suffix: if pattern starts with `*.`, check `host_no_port.ends_with(&pattern[1..])` (the `[1..]` includes the `.`, so `*.example.com` matches `api.example.com`).
   - Case-insensitive comparison.
2. Insert the check inside `match_route` loop at `router.rs:144`, between the site check (line 146-150) and the path check (line 153):
   ```rust
   if let Some(route_host) = &route.config.route_match.host {
       if !host_matches(route_host, host) { continue; }
   }
   ```
3. Do NOT reuse `glob_to_regex` (`router.rs:221`) — it is path-oriented and escapes `.`; hostnames need a different wildcard semantic.

## Tasks
- [ ] Implement `host_matches(pattern, host_header) -> bool` in `router.rs`
- [ ] Insert route-level host check in `match_route` loop
- [ ] Add unit tests: exact match, `*.example.com` wildcard, port-stripping, case-insensitivity, mismatch continues loop
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
