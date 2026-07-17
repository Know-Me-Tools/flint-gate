# Gateway Assessment — gate.sansabaroyalty.com
_Generated: 2026-07-14_

## Goal

Set up an NGINX Ingress (acting as a common gateway) for `gate.sansabaroyalty.com` on the `ssr` AKS cluster that:
- Routes traffic to `flint-gate` (the auth proxy / approval gate)
- Issues a Let's Encrypt TLS certificate via cert-manager
- Redirects HTTP → HTTPS automatically
- Exposes a fixed external IP for DNS configuration
- Is deployed by a GitHub Actions workflow

---

## Cluster State (as-found)

### AKS Cluster: `ssr`
- **Control plane**: `https://ssr-dns-dzsf0wcw.hcp.centralus.azmk8s.io:443`
- **Region**: Central US (Azure)
- **Nodes**: 3 (2× agentpool, 1× userpool), all `v1.33.6`

### Existing Infrastructure

| Component | Status | Notes |
|-----------|--------|-------|
| `ingress-nginx-controller` | ✅ Running | `ingress-nginx` namespace |
| External IP (LoadBalancer) | ✅ `4.249.218.170` | Single LB shared by all apps |
| `cert-manager` | ✅ Running | `cert-manager` namespace |
| `ClusterIssuer/letsencrypt-prod` | ✅ Ready | HTTP-01, solver via `nginx` IngressClass |
| `ClusterIssuer/letsencrypt-staging` | ✅ Ready | For testing |
| Kubernetes Gateway API CRDs | ❌ Not installed | No `gateway.networking.k8s.io` CRDs |
| Envoy Gateway (controller) | ❌ Not installed | No envoy pods/CRDs found |

### Existing Ingresses (reference patterns)

All current ingresses share:
- `ingressClassName: nginx`
- `cert-manager.io/cluster-issuer: letsencrypt-prod`
- `nginx.ingress.kubernetes.io/force-ssl-redirect: "true"`
- `nginx.ingress.kubernetes.io/ssl-redirect: "true"`
- Backed by external IP `4.249.218.170`

### flint-gate Deployment
- **Namespace**: Not yet created (`flint-gate` namespace absent)
- **k8s manifests**: Present in `k8s/` directory (Deployment, Service, HPA, ConfigMap, Secret, NetworkPolicy, service-admin)
- **Proxy port**: `4456`
- **Admin port**: `4457`

---

## Architecture Decision: NGINX Ingress vs. Envoy Gateway

### Recommendation: **Use NGINX Ingress** (not Envoy Gateway controller)

**Why NOT standalone Envoy Gateway:**
1. No Envoy Gateway CRDs or controller are installed on the cluster.
2. Installing a separate Envoy Gateway would require a new LoadBalancer IP or complex IP sharing — adding DNS complexity.
3. The existing `letsencrypt-prod` ClusterIssuer solver is hardcoded to `ingressClassName: nginx` — a different ingress class requires a new ClusterIssuer or solver configuration change.
4. All existing cluster services use NGINX Ingress successfully with the same cert-manager pipeline.
5. "Envoy Gateway as a common gateway" is satisfied architecturally by using NGINX as the edge proxy in front of all apps, with flint-gate placed as the upstream auth/approval layer.

**What "common gateway" means in this cluster:**
The shared NGINX Ingress controller at `4.249.218.170` already IS the common gateway. Flint-gate will be exposed through it with its own Ingress resource, just like every other application.

**If true Envoy Gateway is required in the future:**
- Install `envoy-gateway` via Helm
- Add a new `GatewayClass` + `Gateway` resource
- Update cert-manager ClusterIssuer to support the new ingress class
- This can be added as a follow-on phase

---

## Required Setup

### 1. DNS Record
Point `gate.sansabaroyalty.com` → `4.249.218.170` (A record).

The LoadBalancer IP is stable (141 days old, Azure-managed). No new IP is needed.

### 2. Kubernetes Resources (new)

#### Namespace
```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: flint-gate
```

#### Secrets (external to git — managed via GitHub Actions secrets)
- `DATABASE_URL` — Postgres connection string
- `FLINT_GATE_JWT_SECRET` — JWT signing secret

#### ConfigMap
From existing `k8s/configmap.yaml`

#### Deployment + HPA
From existing `k8s/deployment.yaml` + `k8s/hpa.yaml`

#### Services
From `k8s/service.yaml` (proxy on 4456) + `k8s/service-admin.yaml` (admin on 4457)

#### NetworkPolicy
From `k8s/network-policy.yaml`

#### Ingress (new)
```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: flint-gate
  namespace: flint-gate
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/ssl-redirect: "true"
    nginx.ingress.kubernetes.io/force-ssl-redirect: "true"
    # Increase timeouts for long-lived approval streams
    nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"
    nginx.ingress.kubernetes.io/proxy-send-timeout: "3600"
    nginx.ingress.kubernetes.io/proxy-body-size: "8m"
spec:
  ingressClassName: nginx
  rules:
  - host: gate.sansabaroyalty.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: flint-gate
            port:
              number: 4456
  tls:
  - hosts:
    - gate.sansabaroyalty.com
    secretName: flint-gate-tls
```

HTTP→HTTPS redirect is handled automatically by the two nginx annotations (`ssl-redirect` + `force-ssl-redirect`). cert-manager will use HTTP-01 challenge over port 80 via the existing NGINX controller to obtain the certificate.

### 3. GitHub Actions Workflow (new: `.github/workflows/deploy-gateway.yml`)

**Triggers**: push to `main` on paths `k8s/**` or `flint-gate.yaml` or manual dispatch.

**Secrets needed in GitHub repo**:
| Secret Name | Value |
|-------------|-------|
| `KUBE_CONFIG` | base64-encoded kubeconfig for `ssr` context |
| `DATABASE_URL` | Postgres connection string |
| `FLINT_GATE_JWT_SECRET` | JWT signing secret |
| `REGISTRY_URL` | Container registry (if pushing image) |

**Workflow steps**:
1. Checkout
2. Set up `kubectl` with `KUBE_CONFIG`
3. Build and push Docker image (or use pre-built image from CI)
4. Apply namespace + secrets (secrets injected from GH secrets, never from git)
5. Apply ConfigMap, Deployment, Services, HPA, NetworkPolicy
6. Apply Ingress
7. Wait for rollout: `kubectl rollout status deployment/flint-gate -n flint-gate`
8. Verify certificate: `kubectl get certificate -n flint-gate`

---

## External IP and DNS Configuration

### Gateway IP (current and authoritative)
```
4.249.218.170
```
This is the Azure-managed public IP for the ingress-nginx LoadBalancer service in the `ssr` cluster. It has been stable for 141 days.

### DNS Action Required
Create an **A record** in the `sansabaroyalty.com` DNS zone:
```
gate.sansabaroyalty.com  →  4.249.218.170  (TTL: 300)
```
This must be done **before** the certificate issuance step or the HTTP-01 ACME challenge will fail.

### Certificate Flow
1. Ingress created with `cert-manager.io/cluster-issuer: letsencrypt-prod`
2. cert-manager creates a `CertificateRequest` and HTTP-01 `Challenge`
3. NGINX routes `/.well-known/acme-challenge/...` to cert-manager's solver pod
4. Let's Encrypt verifies the domain and issues the certificate
5. cert-manager stores it in `secret/flint-gate-tls` in the `flint-gate` namespace
6. NGINX picks up the TLS secret and serves HTTPS

**Time to cert**: typically 30–90 seconds after DNS propagates.

---

## Gaps and Open Questions

| Gap | Severity | Notes |
|-----|----------|-------|
| DNS record not yet created for `gate.sansabaroyalty.com` | **Blocker** | Must be done before deploy or ACME challenge fails |
| `flint-gate` namespace not created | **High** | Created by workflow |
| Docker image not published to a registry accessible by `ssr` cluster | **High** | Need registry URL + pull secret or use GHCR |
| `DATABASE_URL` value for production Postgres not known | **High** | Must be provided as GitHub Actions secret |
| `FLINT_GATE_JWT_SECRET` production value not known | **High** | Must be provided as GitHub Actions secret |
| Container image pull secret (if using private registry) | **Medium** | Add `imagePullSecrets` to Deployment if needed |
| `configmap.yaml` has empty `sites: []` / `routes: []` | **Medium** | Production config needs to define the apps protected by flint-gate |
| No Envoy Gateway controller installed | **Low** | If true Envoy Gateway semantics needed, install separately as follow-on |

---

## Recommended Implementation Order

1. **Provide DNS record**: `gate.sansabaroyalty.com → 4.249.218.170`
2. **Set GitHub Actions secrets**: `KUBE_CONFIG`, `DATABASE_URL`, `FLINT_GATE_JWT_SECRET`
3. **Create `k8s/ingress.yaml`** with the Ingress spec above
4. **Create `.github/workflows/deploy-gateway.yml`** with deploy steps
5. **Push to main** → workflow deploys namespace, secrets, manifests
6. **Wait for cert** (~60s) → `kubectl get cert -n flint-gate`
7. **Verify**: `curl -I https://gate.sansabaroyalty.com/health`

---

## Summary

The `ssr` cluster already has all required infrastructure:
- **External IP**: `4.249.218.170` (stable, Azure LB)
- **TLS automation**: cert-manager + letsencrypt-prod ClusterIssuer (HTTP-01 via nginx)
- **HTTP→HTTPS redirect**: nginx annotations (`force-ssl-redirect: "true"`)

The only new Kubernetes resource required is an **Ingress** pointing `gate.sansabaroyalty.com` at the `flint-gate` proxy service. The GitHub Actions workflow deploys the full stack. The one external prerequisite is the DNS A record.

No new LoadBalancer IP, no new TLS infrastructure, no Envoy Gateway controller installation is required to meet the stated goal.
