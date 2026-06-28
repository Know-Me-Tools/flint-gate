---
description: Check flint-gate health, readiness, route count, and cache stats via the admin API on :4457.
argument-hint: [admin-host]   # default: localhost
---

Run the standard health checks against the flint-gate admin API. `$1` defaults to `localhost`.

```bash
ADMIN="${1:-localhost}"
echo "== liveness =="
curl -sS -w '\n[%{http_code}]\n' "http://${ADMIN}:4457/health"
echo
echo "== readiness (DB check) =="
curl -sS -w '\n[%{http_code}]\n' "http://${ADMIN}:4457/ready"
echo
echo "== route count =="
curl -sS "http://${ADMIN}:4457/routes" | jq 'length' 2>/dev/null || echo "(jq not available)"
echo
echo "== cache stats =="
curl -sS "http://${ADMIN}:4457/cache/stats"
echo
```

Report:
- `/health` should return 200 unconditionally (liveness).
- `/ready` returns 200 only when the DB connection is healthy.
- Route count matches the expected number of enabled routes.
- Cache stats show L1 entry count; if L2 is enabled, Redis connectivity is implied by no errors.

If any step fails, suggest the user inspect `kubectl logs deploy/flint-gate` (k8s) or `docker compose logs flint-gate` (local), and confirm the admin port is reachable from where the command is run (it is internal-only in production).
