# observability

## ADDED Requirements

### Requirement: Gateway-local token-exchange is metered
The gateway SHALL emit a counter for every **gateway-local mint** token-exchange
outcome, symmetric with the existing Hydra-delegate counter, so operators can see
local-exchange volume and its fail-closed denials — not only the delegate path.

#### Scenario: A local exchange succeeds
- **WHEN** a token exchange is served on the local-mint path and a delegated
  token is issued
- **THEN** `flint_local_exchange_total{result="success"}` is incremented and no
  other result label is incremented for that exchange

#### Scenario: Each local denial is counted under its reason
- **WHEN** a local exchange is denied because the subject token fails
  verification, the requested scope is not a subset (downscope rejection), or the
  minter fails / is absent
- **THEN** `flint_local_exchange_total` is incremented with
  `result="deny_verify"`, `result="deny_downscope"`, or `result="mint_failed"`
  respectively, and the same fail-closed error is returned (no token issued)

#### Scenario: Metric labels are bounded
- **WHEN** any local-exchange outcome is recorded
- **THEN** the `result` label is one of a fixed, compile-time set of values (no
  request-derived or caller-influenced value becomes a label)

#### Scenario: Local metric is admin-only
- **WHEN** the metrics surface is rendered
- **THEN** `flint_local_exchange_total` is served only on the admin port, never
  on the public proxy
