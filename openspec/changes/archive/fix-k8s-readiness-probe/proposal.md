# fix-k8s-readiness-probe

## Summary
Correct the Kubernetes readiness probe to use `/ready` on the admin port (4457) instead of `/health` on the proxy port (4456).

## Motivation
The readiness probe in `k8s/deployment.yaml:58-64` uses `path: /health` on `port: proxy` (4456). The `/health` endpoint is a liveness check that always returns 200. The `/ready` endpoint on the admin server (4457) actually probes Postgres connectivity and returns 503 when the DB is unreachable. Using the wrong endpoint means Kubernetes never removes pods from the Service when the database is down — clients receive 500s instead of being routed to healthy pods.

## Design
Change `k8s/deployment.yaml` readinessProbe:
- `path: /health` → `path: /ready`
- `port: proxy` → `port: admin`

No Rust code change. The `/ready` endpoint already exists at `src/admin/mod.rs:47,68-81`.

## Tasks
- [ ] Update `k8s/deployment.yaml` readinessProbe to `/ready` on `admin` port
- [ ] Verify livenessProbe remains `/health` on `proxy` port (unchanged)
