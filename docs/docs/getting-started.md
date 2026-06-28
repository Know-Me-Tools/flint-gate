# Getting Started

This guide covers running Flint Gate locally with Docker Compose and configuring it with `config.yaml`.

## Prerequisites

- Docker and Docker Compose
- A copy of the Flint Gate repository

## Build the image

From the repository root:

```bash
docker build -t flint-gate:latest .
```

## Start with Docker Compose

The repository includes a `docker-compose.yml` that starts Flint Gate, Postgres, and an optional Ory Kratos instance:

```bash
# Use defaults
 docker compose up -d

# Or override ports and secrets
 PROXY_PORT=8080 ADMIN_PORT=8081 FLINT_GATE_JWT_SECRET=$(openssl rand -hex 32) docker compose up -d
```

Services:

| Service | Port | Notes |
|---------|------|-------|
| `flint-gate` | `4456` (proxy), `4457` (admin) | Bind only the proxy port to the public interface. |
| `postgres` | `5432` | Stores runtime routes, API keys, and usage events. |
| `kratos` | `4433`, `4434` | Optional — only needed for Kratos session auth. |

The compose file mounts `config.example.yaml` as `/app/config/config.yaml`. Copy it and edit for production:

```bash
cp config.example.yaml config.yaml
```

## `config.yaml` walkthrough

### Server block

```yaml
server:
  listen: "0.0.0.0:4456"
  admin_listen: "0.0.0.0:4457"
  tls:
    enabled: false
```

- `listen` — where the proxy accepts traffic.
- `admin_listen` — internal admin API. Keep it on a private network.
- `tls` — terminate TLS at Flint Gate or at an upstream load balancer.

### Database block

```yaml
database:
  url: "postgres://flintgate:secret@localhost:5432/flintgate"
  max_connections: 20
  override_yaml: false
```

- `url` — Postgres connection string. When empty, runtime route/API-key endpoints return HTTP 501.
- `override_yaml` — when `true`, routes stored in `gate_routes` take precedence over YAML routes.

Flint Gate applies its schema automatically at startup using `CREATE TABLE IF NOT EXISTS`.

### Cache block

```yaml
cache:
  l1:
    max_capacity: 10000
    ttl_seconds: 60
  l2:
    enabled: false
  invalidation_channel: "flintgate_config_changed"
```

- `l1` — in-memory Moka cache.
- `l2` — optional Redis tier.
- `invalidation_channel` — Postgres `LISTEN/NOTIFY` channel for cross-instance invalidation.

### Auth providers

```yaml
auth_providers:
  kratos_session:
    type: kratos
    base_url: "http://kratos:4433"
    forward_cookies: true
    session_cookie: "ory_kratos_session"

  bearer_jwt:
    type: jwt
    jwks_url: "https://auth.example.com/.well-known/jwks.json"
    issuer: "https://auth.example.com"
    audience: "flint-gate"

  api_key:
    type: api_key
    header: "X-API-Key"
    store: database

  passthrough:
    type: anonymous
    default_subject: "anonymous"
```

Providers are referenced by name in sites and routes.

### Sites

```yaml
sites:
  - id: "my-app"
    domains:
      - "app.example.com"
      - "localhost:3000"
    default_auth: kratos_session
    default_upstream: "http://app-backend:3001"
```

A site ties domains to defaults. Routes reference the site by `id`.

### Routes

```yaml
routes:
  - id: "chat-stream"
    site: "my-app"
    match:
      path: "/api/chat/**"
      methods: ["POST"]
    upstream: "http://llm-backend:8000/v1/chat/completions"
    auth: kratos_session
    priority: 10
    hooks:
      pre_request:
        - type: claims_enhancement
          config:
            inject_headers:
              X-User-Id: "{{ identity.id }}"
            mint_jwt:
              enabled: true
              additional_claims:
                scope: "chat"
        - type: body_transform
          config:
            set_fields:
              user: "{{ identity.id }}"
    stream:
      enabled: true
      protocol: sse
      ai:
        ag_ui:
          enabled: true
          validate_events: true
          allowed_events:
            - TEXT_MESSAGE_START
            - TEXT_MESSAGE_CONTENT
            - TEXT_MESSAGE_END
```

This route:

1. Matches `POST /api/chat/**` on the `my-app` site.
2. Validates the Kratos session.
3. Injects `X-User-Id` and mints a JWT with scope `chat`.
4. Adds `user` to the request body.
5. Proxies to the LLM backend.
6. Parses the SSE stream and validates AG-UI event names.

## Verify the deployment

Health:

```bash
curl http://localhost:4457/health
```

Readiness (checks DB connectivity):

```bash
curl http://localhost:4457/ready
```

List routes:

```bash
curl http://localhost:4457/routes
```

## Run the binary directly

For local development without Docker:

```bash
cargo build --release
./target/release/flint-gate --config config.yaml
```

Override ports:

```bash
./target/release/flint-gate --listen 127.0.0.1:8080 --admin-listen 127.0.0.1:8081
```

## Next steps

- Read the full [configuration reference](./configuration.md).
- Explore the [admin API](./admin-api.md).
- Pick a client SDK from the [SDK overview](./sdks/index.md).
