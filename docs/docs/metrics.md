# Metrics Reference

Flint Gate exposes Prometheus metrics on the **admin port** (default `4457`) at:

```
GET http://localhost:4457/metrics
```

This endpoint is served on the private control-plane surface only — never the
public proxy port (`4456`). Do not expose port `4457` to the public internet.

---

## Scraping the endpoint

Add a scrape config to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: flint-gate
    static_configs:
      - targets: ['<admin-host>:4457']
    metrics_path: /metrics
```

---

## Metric catalogue

### `flint_delegate_total`

| Property | Value |
|----------|-------|
| **Type** | Counter |
| **Labels** | `result` |
| **Description** | Counts delegate-exchange outcomes (RFC 8693 token exchange forwarded to Hydra). One increment per exchange attempt. |

**`result` label values:**

| Value | Meaning |
|-------|---------|
| `success` | Hydra returned a scoped token |
| `deny_transport` | HTTP error reaching Hydra |
| `deny_non2xx` | Hydra returned a non-2xx response |

**Alert suggestion:** alert when `rate(flint_delegate_total{result!="success"}[5m]) > 0.05`
(error rate above 5% for 5 minutes).

---

### `flint_local_exchange_total`

| Property | Value |
|----------|-------|
| **Type** | Counter |
| **Labels** | `result` |
| **Description** | Counts gateway-local token-mint outcomes (RFC 8693 exchange handled locally without delegating to Hydra). Symmetric counterpart to `flint_delegate_total` — together they make both exchange modes observable. |

**`result` label values:**

| Value | Meaning |
|-------|---------|
| `success` | Local mint succeeded |
| `deny_verify` | Incoming token failed verification |
| `deny_downscope` | Scope reduction failed |
| `mint_failed` | Signing or serialisation error |

**Alert suggestion:** alert when `rate(flint_local_exchange_total{result!="success"}[5m]) > 0.05`.

---

### `flint_delegate_latency_seconds`

| Property | Value |
|----------|-------|
| **Type** | Histogram |
| **Labels** | _(none)_ |
| **Description** | End-to-end latency of delegate-exchange requests to Hydra, in seconds. Useful for detecting Hydra slowdowns that affect agent token issuance. |

**Alert suggestion:** alert when `histogram_quantile(0.99, rate(flint_delegate_latency_seconds_bucket[5m])) > 1.0`
(P99 latency above 1 second for 5 minutes).

---

### `flint_tool_authz_total`

| Property | Value |
|----------|-------|
| **Type** | Counter |
| **Labels** | `decision` |
| **Description** | Counts per-tool-call Cedar authorization decisions. The tool name is deliberately **not** a label — it is operator/attacker-influenced and would explode cardinality. Tool-level detail is in the `authz_audit` table (see the [Operations Runbook](./operations.md#audit-trail)). |

**`decision` label values:**

| Value | Meaning |
|-------|---------|
| `allow` | Cedar policy permitted the call |
| `deny` | Cedar policy denied the call |
| `deny_shadow` | Denied by shadow mode (would-have-denied under strict policy) |

**Alert suggestion:** alert when `rate(flint_tool_authz_total{decision="deny"}[1m]) > 10`
(more than 10 denials per second — may indicate a misconfigured policy or an abuse attempt).

---

### `flint_agent_budget_denied_total`

| Property | Value |
|----------|-------|
| **Type** | Counter |
| **Labels** | _(none)_ |
| **Description** | Counts the number of agent tool calls rejected because the agent exceeded its configured token or call budget. Surfaces the volume of spend-cap enforcements — both over-limit and fail-closed enforcements on backend outage. |

**Alert suggestion:** alert when `increase(flint_agent_budget_denied_total[5m]) > 50`
(spike in budget enforcement may indicate a runaway agent or a misconfigured budget).

---

### `flint_governance_reload_rejected_total`

| Property | Value |
|----------|-------|
| **Type** | Counter |
| **Labels** | _(none)_ |
| **Description** | Counts route configuration hot-reloads rejected by the strict agent-governance lint. When `strict_agent_governance: true`, any reload containing an ungoverned agent-reachable route is rejected and the last known-good configuration is retained. This metric makes those silent retain-last-good events observable without log-grepping. |

**Alert suggestion:** alert when `increase(flint_governance_reload_rejected_total[10m]) > 0`
(any governance rejection in a 10-minute window warrants operator review).

---

## Grafana dashboard

A sample Grafana 10 dashboard definition is provided at
[`grafana/flint-gate-dashboard.json`](../../grafana/flint-gate-dashboard.json).

Import it via **Dashboards → Import → Upload JSON file** in the Grafana UI,
or via the Grafana API:

```bash
curl -X POST http://localhost:3000/api/dashboards/import \
  -H 'Content-Type: application/json' \
  -d @grafana/flint-gate-dashboard.json
```

---

## See also

- [Operations Runbook](./operations.md) — monitoring checklist, approval janitor tuning, audit trail
- [Cedar Policies Reference](./cedar-policies.md) — authorization decision audit trail
