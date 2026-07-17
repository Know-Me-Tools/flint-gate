# add-admin-authn

## Why
The admin API (routes/policies/api-keys/signing-keys/approvals CRUD + analytics +
the embedded web UI) has **zero request-level authentication** — its only
protection is `admin_listen` defaulting to `127.0.0.1`. This blocks safely
exposing the control plane (and the new web UI) beyond loopback for remote /
multi-operator use. Prior-phase tech-debt #1; the CRITICAL gate for this phase.

## What Changes
Add an admin-auth **tower middleware** on `admin_router()` that verifies each
request against a configured auth provider — **reusing the existing
`JwtAuthConfig` / `KratosAuthConfig`** (the Ory-standard path; any JWKS-backed
JWT provider also works). No new crate, no new identity model (D-B01).

**Default posture (per operator decision):**
- `admin_auth` unset **and** `admin_listen` is loopback → allow unauthenticated (dev).
- `admin_auth` unset **and** `admin_listen` is non-loopback → **refuse to start** (fail-safe).
- `admin_auth` set → always enforce, regardless of bind address.

## Design
- New `admin_auth: Option<AdminAuthConfig>` in `ServerConfig`, referencing an
  `AuthProviderConfig` (JWT or Kratos) by inline config or provider id.
- `admin::auth` middleware (`axum::middleware::from_fn_with_state`) verifies the
  bearer JWT (via `auth/jwt_verify.rs`) or Kratos session (via `auth/kratos.rs`),
  attaching the resolved `Identity` to request extensions; 401 on failure.
- Startup guard in the server bootstrap enforces the loopback/non-loopback posture.

## Depends on
- Nothing new — reuses existing verification primitives. **Built first** (gates the phase).

## Scope
IN: admin-auth middleware, `AdminAuthConfig`, loopback/off-loopback startup guard,
reuse of JWT/Kratos verification, unit + integration tests, docs update.
OUT: RBAC on individual admin routes (single admin principal for now); a bespoke
admin credential store (reuse providers).

## Tasks
- [ ] Add `AdminAuthConfig` + `ServerConfig.admin_auth`; loopback/off-loopback startup guard (refuse non-loopback without auth)
- [ ] `admin::auth` middleware: verify JWT (jwt_verify) or Kratos session (kratos), attach Identity, 401 on failure
- [ ] Wire the middleware onto `admin_router()`; keep `/health` (+ `/ready`) unauthenticated for probes
- [ ] Tests: authed request passes, missing/invalid token 401, loopback-dev bypass, off-loopback-without-auth refuses start (fail-closed `degrades_to_deny`)
- [ ] Docs: config.example.yaml `admin_auth` block + README admin-exposure note; `cargo check/clippy/test --workspace` green
