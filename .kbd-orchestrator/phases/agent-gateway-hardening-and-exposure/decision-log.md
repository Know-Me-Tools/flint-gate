# Decision Log — agent-gateway-hardening-and-exposure

### 2026-07-04 — Analyze: hardening build-vs-adopt calls (stack-specified)

- **D-C01 · OAuth endpoint auth+rate-limit → REUSE client store + governor** (hand-wire). [analyze · 2026-07-04]
  TL;DR: authenticate /oauth/introspect (+ optionally /oauth/token) with OAuth client credentials via the existing oauth_clients store; apply the existing build_governor_layer per-endpoint.
  Why: RFC 7662 §2.1 MANDATES introspection auth to stop token scanning; client-credentials is standard and already verified; governor layer already exists. Zero new crates.
  Provenance: research (RFC 7662 §2.1) + codebase.
- **D-C02 · Slow-KDF secrets → ADOPT bcrypt@0.19** (the phase's one new crate). [analyze · 2026-07-04]
  TL;DR: replace unsalted sha256 for oauth_clients.secret_hash with bcrypt (stable, pure-Rust, internal salt); verify = fetch-by-id then bcrypt::verify; format-sniff cutover.
  Why: client secret is 256-bit CSPRNG so bcrypt work-factor suffices; argon2 crate is mid-churn (0.6.0-rc) and memory-hardness is overkill. KISS. Breaks the all-reuse streak — deliberate, security-justified.
  Alternatives: argon2 (rc churn, rejected), scrypt (overkill, rejected). Provenance: research (cargo search).
- **D-C03 · Classification edges → EXTEND existing** (3 fixes). [analyze · 2026-07-04]
  TL;DR: api_key build_result kind=Service; derived_kind gates the act-fallback off session_id (Kratos); NHI revoke+audit in one transaction (mirror db:834).
  Why: targeted correctness/robustness on existing code, no deps. Provenance: codebase assessment.
- **D-C04 · Chained delegation + Hydra-delegate → HAND-ROLL reject-first + wire seam**. [analyze · 2026-07-04]
  TL;DR: verify actor_token OR reject present-but-unsupported (fail-closed, vs today's silent-ignore); wire delegate_to_hydra to proxy to the configured Hydra token endpoint.
  Why: silently ignoring a security-relevant param is the fail-open pattern this phase kills. Hydra external-aud caveat (ory/hydra#3723).
  Open: verify-and-chain now vs reject-if-present (lean: reject-if-present). Provenance: codebase + prior research.
