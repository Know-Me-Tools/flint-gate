# Refinement log — add-admin-authn

**Mode:** code-artifact constraint validation (not generative refinement)
**Date:** 2026-07-04

## Summary
- 5/5 tasks complete. Adds authentication to the previously-unauthenticated admin
  API (reuses JWT/Kratos providers via a tower middleware). Fail-closed startup
  posture: unauth allowed only on loopback; non-loopback without auth refuses to start.
- Workspace green: 299 core lib + 5 mcp-e2e + 11 + 16 + 4 + 1 doc, 0 failed, 4 ignored.
  clippy --workspace -D warnings clean; fmt clean.
- 10 new tests: 4 admin_auth_status mapping, 2 middleware integration (deny→401,
  accept→identity-attached), 4 posture (enforce/allow-loopback/refuse-start/unparseable-fails-safe).

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets/keys/creds committed | BLOCK | PASS (config.example uses placeholder JWKS URL only) |
| Never expose admin (4457) to public internet | BLOCK | **ADVANCED** — change makes exposure *safe & opt-in*; non-loopback bind without `admin_auth` now REFUSES to start (was: silently open) |
| Existing tests not broken | BLOCK | PASS (all prior tests pass; added `admin_auth: None` to one test literal) |
| Config priority CLI>env>YAML unchanged | BLOCK | PASS (untouched) |
| anyhow/thiserror error style | WARN | PASS (RefuseStart uses anyhow::bail!) |
| No unwrap/expect outside tests/init | WARN | PASS (one `.expect("Enforce implies admin_auth is Some")` is a proven-unreachable invariant guarded by the posture match) |
| Proxy(4456)/admin(4457) separation | WARN | PASS (admin-only middleware; proxy untouched) |
| Follow existing module structure | WARN | PASS (new `admin/auth.rs` submodule; shared `build_authenticator` factored in `auth/mod.rs`) |

## Notes
- Security-review (security-reviewer agent) run separately — see change QA gate.
- Fail-closed `degrades_to_deny` covered: posture refuses-start off-loopback-without-auth
  (incl. unparseable-host-fails-safe); middleware returns 401 on authenticator denial.

## Security review (security-reviewer agent)
Focus: authentication-bypass / fail-open. Verified against axum 0.8.9 source + IpAddr edge cases.

**No CRITICAL/HIGH found.** Key confirmations:
- **No auth bypass via merge/fallback/layer.** `Router::layer` wraps path_router,
  fallback_router, default_fallback, AND catch_all_fallback; `public.merge(protected)`
  preserves protected's auth-wrapped custom fallback. The SPA static fallback, `/docs`,
  and `/openapi.json` are all authenticated. (Verified in axum 0.8.9 routing/mod.rs.)
- **Loopback detection sound.** `0.0.0.0`→false, `::`→false, `::ffff:127.0.0.1`→false
  (not treated as loopback), decimal/octal/`127.0.0.1.evil.com`→parse-error→non-loopback
  (fail-safe RefuseStart). No host string was found where parsed `.is_loopback()` is true
  while the real bind is non-loopback (no bypass). `127.0.0.0/8` correctly all loopback.

**Gap closed during review:** added a composed-router integration test
(`spa_fallback_is_protected_when_auth_denies`, `health_probe_stays_open_in_composed_router`,
`protected_and_fallback_pass_when_auth_accepts`) proving the auth layer actually covers the
SPA fallback (401 without creds) while `/health` stays open. This was the one real test gap.

**Verdict: PASS** (fail-closed boundary verified from two angles: source analysis + integration test).
