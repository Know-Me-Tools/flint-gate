# Refinement Log — add-delegate-observability

_Change 3/4 of `agent-gateway-exposure-operability` (Goal G2 · cand-001 + build-003)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets; metric labels are static literals, render output carries no token/URL/subject. |
| Never expose the admin server (4457) to public internet | PASS | `/metrics` is on the ADMIN router only (never proxy_app); admin bind is posture-gated (RefuseStart on non-loopback without auth). |
| Never break existing unit tests without updating them | PASS | 406 core tests green; delegate + admin tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | No config fields added; recorder install + route only. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 406 core tests, 0 failed (8 ignored).
- New deps: `metrics@0.24.6` + `metrics-exporter-prometheus@0.18.3`
  (default-features=false — no http-listener/push-gateway).
- New-code coverage: recorder install-idempotent + render, every delegate `result`
  label + latency exposed, delegate-success metered-and-rendered end-to-end.
  Admin-port-only verified structurally (`/metrics` appears only in admin/mod.rs).

## Adoption + decision recorded

- **cand-001 adopted:** `metrics` facade + `metrics-exporter-prometheus`; `/metrics`
  on the admin port; `flint_delegate_total{result}` + `flint_delegate_latency_seconds`.
- **build-003 (no re-stamp) encoded:** documented in README + config.example that
  delegate-mode tokens carry Hydra's claims, are NOT flint_kind=agent-classified,
  and escape gateway-side agent budget — intentional (federate, never an IdP);
  the delegate metric surfaces the bypass volume.

## Separated security review (security-reviewer agent)

Verdict: **APPROVE** — no CRITICAL/HIGH; both design invariants hold; all 5 focus
areas PASS.

- **/metrics exposure PASS** — admin-router only (never proxy_app); admin bind
  posture-gated; render output limited to `flint_delegate_*` (no process/host
  metrics, no secrets).
- **Label cardinality/injection PASS** — `record_delegate(&'static str)` structurally
  forbids a runtime/attacker value as a label; `result` label bounded at 5; metric
  names const. Hydra error detail flows to the OAuth JSON error, never a label.
- **Recorder global-install PASS** — `OnceLock::get_or_init` means `expect` fires
  at most once, only on genuine first-install failure at startup (fail-fast);
  confirmed the only production install site is main.rs:145 (no competing recorder).
- **Timing/DoS PASS** — monotonic `Instant`, timeout-bounded delegate client.
- **Secrets PASS** — `metrics_handler` reads no request data; render carries none.

LOW/informational only (default histogram buckets; local-mint path not metered
by design) — nothing actionable.

## Outcome

**PASS** — all constraints satisfied; the metrics surface is admin-only with
static labels; the no-re-stamp decision is encoded + documented; review APPROVE
with no actionable findings. Proceed to archive.
