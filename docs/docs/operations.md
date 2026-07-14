# Operations Runbook

Practical guidance for operators running flint-gate in production or staging
environments during the beta.

---

## JWT Signing Key Rotation

Flint Gate issues and validates JWTs using a configured signing key. Rotating
the key requires a rolling restart to avoid dropping in-flight tokens.

### Steps

1. **Generate a new secret** (HS256) or key pair (RS256/ES256):
   ```sh
   openssl rand -hex 32        # HS256 secret
   openssl genpkey -algorithm RSA -out new-key.pem -pkeyopt rsa_keygen_bits:2048   # RS256
   ```

2. **Update the config** (or environment variable):
   ```yaml
   jwt:
     signing_algorithm: HS256
     signing_key_secret: <new-secret>
   ```

3. **Perform a rolling restart** — keep at least one replica running at all
   times so in-flight token validation continues on the old key. If using a
   Kubernetes Deployment, this is the default behavior when you update the
   secret and apply the config:
   ```sh
   kubectl rollout restart deployment/flint-gate
   ```

4. **Verify** — once all replicas have restarted, confirm token validation
   works with the new key before decommissioning the old secret.

> **Note:** tokens signed with the old key will be rejected after rotation.
> Plan rotation during low-traffic periods or inform clients to re-authenticate.

---

## Policy Recovery (Blocked Traffic)

If a Cedar policy blocks all traffic (e.g., a `forbid` policy with no scope
restriction), you need to remove or disable it without being locked out.

### Option 1: Delete via the Admin API (preferred)

The admin API runs on a separate port (`:4457` by default) that is not gated
by the Cedar authorization engine. Use it to list and delete the offending
policy:

```sh
# List all policies
curl http://localhost:4457/api/policies

# Delete the blocking policy
curl -X DELETE http://localhost:4457/api/policies/<policy-id>
```

The Cedar engine hot-reloads within seconds (triggered by a Postgres NOTIFY on
the `authz_policies` table).

### Option 2: Direct database delete

If the admin API is unreachable (e.g., the pod is down), connect directly to
the Postgres instance:

```sql
-- Find the blocking policy
SELECT id, policy_text, enabled FROM authz_policies WHERE enabled = true;

-- Disable or delete it
UPDATE authz_policies SET enabled = false WHERE id = '<policy-id>';
-- or
DELETE FROM authz_policies WHERE id = '<policy-id>';
```

After the database change, restart the gateway or wait for the next NOTIFY
cycle to reload.

### Option 3: Start without policies

If the database is inaccessible, start the gateway with an empty database
URL — it will serve requests with a default-deny posture but without any Cedar
policies:

```yaml
database:
  url: ""   # no database → no policies → all traffic denied by default
```

See [Cedar policies reference](./cedar-policies.md) for guidance on writing
safe policies.

---

## Approval Store Backend

Flint Gate supports two approval store backends, selected at startup:

| Backend | Durability | Cross-replica | Use case |
|---------|-----------|--------------|----------|
| `memory` (default) | Ephemeral — lost on pod restart | No | Single-replica dev/staging |
| `postgres` | Durable — survives restarts | Yes | Multi-replica production |

### Configuration

```yaml
approval:
  backend: postgres   # or "memory" (default)

database:
  url: "postgres://user:pass@host:5432/flintgate"
```

Or override at startup via environment:

```sh
FLINT_APPROVAL_BACKEND=postgres flint-gate -c config.yaml
```

If `approval.backend: postgres` is set but no `database.url` is configured,
the gateway warns and falls back to `memory` automatically.

### Database migration

The approval store requires the `pending_approvals` table. Apply the
migration before switching to the Postgres backend:

```sh
# Using the bundled migrations
sqlx migrate run --source crates/flint-gate-core/migrations
```

Or apply manually:

```sql
-- migration: 0003_pending_approvals.sql
CREATE TABLE IF NOT EXISTS pending_approvals (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_sub     TEXT        NOT NULL,
    tool_name     TEXT        NOT NULL,
    reason        TEXT        NOT NULL DEFAULT '',
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL,
    decision      TEXT        CHECK (decision IN ('approved', 'rejected')),
    decided_at    TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_pending_approvals_expires
    ON pending_approvals (expires_at)
    WHERE decision IS NULL;
```

### Admin API endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/approvals` | List pending (undecided, non-expired) approvals |
| `POST` | `/approvals` | Register a new approval request |
| `GET` | `/approvals/:id` | Status of a single approval |
| `POST` | `/approvals/:id/decide` | Record approved/rejected decision |

```sh
# Register a pending approval
curl -X POST http://localhost:4457/approvals \
  -H 'Content-Type: application/json' \
  -d '{"agent_sub":"my-agent","tool_name":"send_email","reason":"customer contact","expires_at":"2026-07-15T00:00:00Z"}'

# List pending approvals
curl http://localhost:4457/approvals

# Approve it
curl -X POST http://localhost:4457/approvals/<id>/decide \
  -H 'Content-Type: application/json' \
  -d '{"decision":"approved"}'
```

### Multi-replica note

When running two or more replicas with the Postgres backend, approvals
registered on one replica are visible and decidable from any other replica.
A `pg_notify('flintgate_approval_decided', <id>)` is sent on each decision so
replicas holding a long-poll can wake up immediately.

If you are using the `memory` backend with multiple replicas, decisions made
on one replica are invisible to others — use `sessionAffinity: ClientIP` on
the admin `Service` (`k8s/service-admin.yaml`) as a temporary band-aid, but
plan to switch to `postgres` before scaling out.

---

## Approval TTL Janitor

The approval janitor runs in the background and automatically denies pending
approvals that exceed their TTL. This prevents dead approval entries from
accumulating when the original stream has already closed.

### Configuration

```yaml
approval:
  ttl_seconds: 300            # how long a pending approval waits before expiry
  janitor_interval_seconds: 60  # how often the janitor scans for expired entries
```

**Defaults:**
- `ttl_seconds`: 300 (5 minutes)
- `janitor_interval_seconds`: derived from `ttl_seconds / 2`, clamped to
  `[10, 300]`, with a 60s fallback if TTL is not set

### Tuning guidance

| Scenario | Recommendation |
|----------|----------------|
| High-throughput with short-lived streams | Reduce `ttl_seconds` to 60–120s |
| Human reviewers need more time | Increase `ttl_seconds` to 600–1800s |
| Many replicas, frequent scans | Increase `janitor_interval_seconds` to reduce scan churn |
| Debugging — see every expiry | Set `janitor_interval_seconds: 5` temporarily |

### Log output

The janitor logs at `DEBUG` level when entries are purged:

```
DEBG authz janitor purged 3 expired pending approvals
```

Enable with `RUST_LOG=flint_gate_core=debug`.

---

## Audit Trail

Every Cedar authorization decision is recorded in the `authz_audit` table.

### Schema

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID | Unique record id |
| `principal_id` | TEXT | The identity making the request |
| `principal_type` | TEXT | `User`, `Agent`, or `Service` |
| `action` | TEXT | Cedar action (e.g. `call_tool`) |
| `resource_id` | TEXT | Route id (e.g. `my_tool`) |
| `decision` | TEXT | `Allow`, `Deny`, or `RequireApproval` |
| `policy_id` | TEXT | Id of the matched Cedar policy (nullable) |
| `approval_id` | TEXT | Approval request id (nullable, set on RequireApproval) |
| `created_at` | TIMESTAMPTZ | Timestamp of the decision |

### Common query patterns

```sql
-- All denials in the last hour
SELECT * FROM authz_audit
WHERE decision = 'Deny' AND created_at > now() - interval '1 hour'
ORDER BY created_at DESC;

-- All RequireApproval decisions for a specific agent
SELECT * FROM authz_audit
WHERE principal_id = 'my-agent' AND decision = 'RequireApproval'
ORDER BY created_at DESC;

-- Decision counts by principal in the last 24 hours
SELECT principal_id, decision, count(*)
FROM authz_audit
WHERE created_at > now() - interval '1 day'
GROUP BY principal_id, decision
ORDER BY count DESC;

-- Policies that fired most in the last week
SELECT policy_id, count(*)
FROM authz_audit
WHERE created_at > now() - interval '7 days' AND policy_id IS NOT NULL
GROUP BY policy_id
ORDER BY count DESC;
```

---

## Prometheus Metrics

Flint Gate exposes Prometheus metrics on the admin port:

```
GET http://localhost:4457/metrics
```

Six metrics are available covering token exchange throughput, exchange latency,
Cedar authorization decisions, agent budget enforcement, and governance reload
health. See the [Metrics Reference](./metrics.md) for the full catalogue,
label sets, and suggested alert thresholds.

A sample Grafana dashboard is provided at
[`grafana/flint-gate-dashboard.json`](../../grafana/flint-gate-dashboard.json).

---

## Monitoring Checklist

| Signal | What to watch | Healthy range |
|--------|---------------|---------------|
| `/health` response | `status: "ok"` | Always `ok` |
| `/ready` response | `status: "ready"` | `ready` or `degraded` (DB issues) |
| `flint_delegate_total{result!="success"}` | Delegate error rate | < 5% |
| `flint_tool_authz_total{decision="deny"}` | Cedar deny rate | Stable; spikes warrant investigation |
| `flint_agent_budget_denied_total` | Budget enforcement volume | Near 0 in steady state |
| `flint_governance_reload_rejected_total` | Governance reload rejections | 0; any nonzero warrants review |
| Authorization decisions | `authz_audit` deny rate | < 5% of total |
| Approval queue depth | `SELECT count(*) FROM authz_audit WHERE decision = 'RequireApproval' AND created_at > now() - interval '5 minutes'` | Near 0 in steady state |
| Rate limit hits | `WARN rate_limit enabled but config was degenerate` | Never |
| Reload status | `GET /api/policies/reload-status` | `ok: true` |
| Startup errors | Any `ERRO` at startup | None |

### Key log lines

```
INFO  flint_gate: database migrations applied
INFO  flint_gate: route table built  route_count=N
WARN  flint_gate: running in Kubernetes with no server.admin_auth configured
WARN  flint_gate: rate limiting enabled without Redis in a Kubernetes environment
ERRO  flint_gate: require_policies_at_startup is true but no policies are loaded
```
