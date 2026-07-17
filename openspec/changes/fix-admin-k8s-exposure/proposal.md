# fix-admin-k8s-exposure

**Phase:** beta-release-readiness / Phase 1 (Blocker B-2)
**Priority:** BLOCKER — must close before any external beta

## Problem

In a Kubernetes deployment, the admin port (4457) is bound to loopback
(`127.0.0.1:4457`) but the K8s service exposes it to all pods in the cluster
via the pod IP. Any pod can reach any other pod's admin API with zero credential.
The existing posture guard only fires when the admin bind is non-loopback — it
does not account for K8s network topology.

## Solution

1. **K8s NetworkPolicy** — add `k8s/network-policy.yaml` that denies all ingress
   to port 4457 by default. Operators who need cross-pod admin access must
   explicitly allow specific source pods.

2. **Remove admin from the K8s service** — the service at `k8s/service.yaml`
   must not expose port 4457 in the service's `ports:` block. Pod-direct access
   (via `kubectl port-forward`) is the intended access pattern.

3. **K8s startup warning** — in `main.rs`, detect `KUBERNETES_SERVICE_HOST`
   env var (standard k8s injection) and warn when `admin_auth` is not configured,
   regardless of the bind address.

4. **Documentation** — add an "Admin API security" section to `getting-started.md`
   with a red-box warning and the NetworkPolicy YAML.

## Files to change

- `k8s/network-policy.yaml` (new)
- `k8s/service.yaml` — remove admin port from service `ports:`
- `crates/flint-gate/src/main.rs` — k8s-aware posture guard
- `docs/docs/getting-started.md` — security warning section
