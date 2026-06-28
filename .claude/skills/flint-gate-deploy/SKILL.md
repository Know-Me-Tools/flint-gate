---
license: MIT
name: flint-gate-deploy
description: Deploy flint-gate via docker-compose or Kubernetes. Includes compose file, ConfigMap/Secret/Deployment/Service structure, probes, and env wiring. Use when the user says "deploy flint gate", "flint gate kubernetes", or needs to run the gateway locally or in a cluster.
---

# flint-gate-deploy

Flint Gate is a single Rust binary exposing two ports:
- **4456** — proxy (public)
- **4457** — admin API (internal only; never exposed to the internet)

It requires Postgres (routes, API keys, stream meter logs) and optional Redis for L2 cache. Config is a single YAML file plus env overrides for secrets.

## Local: docker-compose

Reference file (matches repo `docker-compose.yml`):

```yaml
version: "3.9"
services:
  flint-gate:
    build: .
    ports:
      - "${PROXY_PORT:-4456}:4456"
      - "${ADMIN_PORT:-4457}:4457"
    environment:
      DATABASE_URL: "postgres://flintgate:${POSTGRES_PASSWORD:-flintgate}@postgres:5432/flintgate"
      FLINT_GATE_JWT_SECRET: "${FLINT_GATE_JWT_SECRET:-change-me-in-production}"
      FLINT_GATE_CONFIG: "/app/config/config.yaml"
      RUST_LOG: "${RUST_LOG:-info,flint_gate=debug}"
    volumes:
      - ./config.example.yaml:/app/config/config.yaml:ro
    depends_on:
      postgres: { condition: service_healthy }
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "printf '' > /dev/tcp/localhost/4456 2>/dev/null && echo ok || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 3
      start_period: 15s

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: flintgate
      POSTGRES_PASSWORD: "${POSTGRES_PASSWORD:-flintgate}"
      POSTGRES_DB: flintgate
    volumes:
      - postgres_data:/var/lib/postgresql/data
    ports:
      - "${POSTGRES_PORT:-5432}:5432"
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U flintgate -d flintgate"]
      interval: 5s
      timeout: 5s
      retries: 10

  # Optional — only if using Kratos session auth
  kratos:
    image: oryd/kratos:v1.2
    command: serve all --dev
    environment:
      DSN: "postgres://flintgate:${POSTGRES_PASSWORD:-flintgate}@postgres:5432/kratos"
      LOG_LEVEL: info
      SERVE_PUBLIC_BASE_URL: "http://localhost:4433/"
      SERVE_ADMIN_BASE_URL: "http://kratos:4434/"
    ports: ["4433:4433", "4434:4434"]
    depends_on: { postgres: { condition: service_healthy } }
    restart: unless-stopped

volumes:
  postgres_data:
```

Run:

```bash
cp .env.example .env   # edit POSTGRES_PASSWORD, FLINT_GATE_JWT_SECRET
docker compose up -d --build
curl -sf http://localhost:4457/ready && echo ready
```

The healthcheck uses bash `/dev/tcp` because the slim runtime image has no curl.

## Kubernetes

Five manifests live under `k8s/`: `configmap.yaml`, `secret.yaml`, `deployment.yaml`, `service.yaml`, `hpa.yaml`. Structure:

### Secret (`secret.yaml`)

```yaml
apiVersion: v1
kind: Secret
metadata: { name: flint-gate-secrets }
type: Opaque
stringData:
  database-url: "postgres://flintgate:...@postgres:5432/flintgate"
  jwt-secret:   "<strong-hs256-secret>"
```

### ConfigMap (`configmap.yaml`)

Holds the rendered `config.yaml`. Mount read-only at `/app/config`.

### Deployment (`deployment.yaml`) — key fields

```yaml
spec:
  replicas: 2                       # stateless; scale horizontally
  strategy: { type: RollingUpdate, rollingUpdate: { maxSurge: 1, maxUnavailable: 0 } }
  template:
    spec:
      terminationGracePeriodSeconds: 60   # drain in-flight SSE streams
      containers:
        - name: flint-gate
          image: flint-gate:latest
          ports:
            - { name: proxy,  containerPort: 4456 }
            - { name: admin,  containerPort: 4457 }
          env:
            - { name: FLINT_GATE_CONFIG, value: /app/config/config.yaml }
            - name: DATABASE_URL
              valueFrom: { secretKeyRef: { name: flint-gate-secrets, key: database-url } }
            - name: FLINT_GATE_JWT_SECRET
              valueFrom: { secretKeyRef: { name: flint-gate-secrets, key: jwt-secret } }
            - { name: RUST_LOG, value: "info,flint_gate=debug" }
          volumeMounts:
            - { name: config, mountPath: /app/config, readOnly: true }
          livenessProbe:
            httpGet: { path: /health, port: proxy }
            initialDelaySeconds: 10
            periodSeconds: 15
          readinessProbe:
            httpGet: { path: /ready,  port: admin }
            initialDelaySeconds: 5
            periodSeconds: 10
          resources:
            requests: { cpu: 100m, memory: 128Mi }
            limits:   { cpu: 500m, memory: 512Mi }
      volumes:
        - name: config
          configMap: { name: flint-gate-config }
```

### Service (`service.yaml`)

Two Services, never one combined:
- **public** — `port: 4456`, type LoadBalancer / ClusterIP behind ingress.
- **admin** — `port: 4457`, type ClusterIP only, restricted by NetworkPolicy to internal callers.

### HPA (`hpa.yaml`)

Scales on CPU; pair with the `resources` block above.

## Image build

`Dockerfile` in repo root produces a slim image. Build and push:

```bash
docker build -t flint-gate:latest .
# remote registry
docker tag  flint-gate:latest <registry>/flint-gate:v0.1.0
docker push <registry>/flint-gate:v0.1.0
```

Update `image:` in the Deployment and `kubectl rollout restart deployment/flint-gate`.

## Applying

```bash
kubectl apply -f k8s/secret.yaml
kubectl apply -f k8s/configmap.yaml
kubectl apply -f k8s/deployment.yaml -f k8s/service.yaml -f k8s/hpa.yaml
kubectl rollout status deployment/flint-gate
```

## Post-deploy checks

```bash
# From a pod in the same namespace (admin port is internal-only):
kubectl exec -it deploy/flint-gate -- \
  curl -sf http://localhost:4457/ready && echo

kubectl exec -it deploy/flint-gate -- \
  curl -sf http://localhost:4457/routes | jq length
```

## Operational notes

- **Admin port isolation** — if a LoadBalancer accidentally exposes 4457, anyone can read/modify routes. Use a NetworkPolicy and a separate Service.
- **In-flight streams on rollout** — `terminationGracePeriodSeconds: 60` gives SSE/AG-UI streams time to drain; lower only if you accept dropped streams.
- **Hot-reload vs restart** — `config.yaml` edits hot-reload (~200ms). Env changes (secrets, `RUST_LOG`) require a restart.
- **Postgres** — the gateway runs `CREATE TABLE IF NOT EXISTS` migrations at startup; grant `CREATE` on the DB to the gateway user or pre-create the schema.
- **ConfigMap rollout** — editing the ConfigMap does not restart pods. Use `kubectl rollout restart` or mount via a tool that watches for changes.
