# Prior context — agent-gateway-hardening-and-exposure

Seeded from **agent-identity-and-delegation** (4/4 goals MET, 4/4 changes archived,
0 CRITICAL; 5 HIGH fail-open/escalation defects caught + fixed in review).

## What exists to harden (built last phase)
- **Admin API authentication** (fail-closed startup posture) — `add-admin-authn`.
- **RFC 8693 token-exchange** (gateway-local downscoping, act-claim delegated
  tokens; Hydra-delegate seam OFF) — `add-token-exchange`.
- **OAuth2 client-credentials + RFC 7662 introspection** (unified `/oauth/token`
  dispatcher + `/oauth/introspect`; Hydra-consume seam) — `add-client-creds-introspection`.
- **NHI as distinct Cedar Agent::/Service:: principals** + issue/rotate/revoke
  lifecycle + fail-closed revocation at per-tool AND route gates — `add-nhi-cedar-principals`.

## Debt this phase must close (from the reflection)
1. `/oauth/token` + `/oauth/introspect` are UNAUTHENTICATED + unrate-limited on the
   proxy port → endpoint auth + rate-limiting (Goal 1, do first).
2. Client secret is unsalted SHA-256 → slow salted KDF (Goal 2).
3. API-key identities never classified `Service` (escape NHI revocation); Kratos
   metadata_public kind-derivation edge; NHI audit post-mutation best-effort (Goal 3).
4. `actor_token` chained delegation + Hydra-delegate token-exchange seam (Goal 4).

Carried longer: Redis-L2-as-hard-dep for accurate cross-replica rate limiting.

See `../agent-identity-and-delegation/reflection.md` for the full record.
