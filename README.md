# Flint Gate

**AI-native auth proxy and API gateway — by [KnowMe, LLC](https://know-me.tools)**

> Strike an idea. Watch it build.

Flint Gate is a purpose-built replacement for [Ory Oathkeeper](https://www.ory.sh/oathkeeper/) designed for **streaming LLM workloads**. It is not a general-purpose reverse proxy — it is an enforcement point that authenticates, authorizes, enriches, and passes through requests to upstream services, with first-class support for Server-Sent Events, AG-UI events, A2UI intents, and token metering.

---

## Why Flint Gate?

Ory Oathkeeper has critical gaps for modern AI application architectures:

| Problem | Oathkeeper | Flint Gate |
|---|---|---|
| Config updates | Requires process restart | Hot-reload via filesystem watcher + Postgres LISTEN/NOTIFY |
| Streaming | Buffers full response body | SSE passthrough — never buffers |
| AI protocols | No awareness | AG-UI event validation/filtering, A2UI intent filtering |
| Runtime routing | File-only | CRUD via Admin API, stored in Postgres |
| Token metering | None | Counts tokens mid-stream from TEXT_MESSAGE_CONTENT deltas |
| Session expiry mid-stream | None | Session watchdog terminates active streams on expiry |

---

## Architecture

```
                    ┌──────────────────────────────────────────┐
                    │              Flint Gate                   │
    Request ──────► │                                          │
                    │  ┌────────────┐  ┌────────────────────┐  │
                    │  │   Route    │  │   Auth Pipeline    │  │
                    │  │  Matcher   │──│ • Kratos session   │  │
                    │  │(glob+site) │  │ • JWT verification │  │
                    │  └────────────┘  │ • API key (DB)     │  │
                    │                  │ • Anonymous        │  │
                    │                  └────────┬───────────┘  │
                    │                           │              │
                    │  ┌────────────────────────▼───────────┐  │
                    │  │        Pre-Request Hooks           │  │
                    │  │  • claims_enhancement              │  │
                    │  │    – inject_headers (templates)    │  │
                    │  │    – mint outbound JWT             │  │
                    │  │  • body_transform                  │  │
                    │  │    – set JSON fields (templates)   │  │
                    │  └──────────────────┬─────────────────┘  │
                    │                     │                     │
                    │  ┌──────────────────▼─────────────────┐  │
                    │  │         Upstream Proxy             │  │──► Backend
                    │  │  • SSE stream passthrough          │  │    Services
                    │  │  • AG-UI event processing          │  │
                    │  │  • A2UI intent filtering           │  │
                    │  │  • Token metering mid-stream       │  │
                    │  │  • Backpressure limits             │  │
                    │  └────────────────────────────────────┘  │
                    │                                          │
                    │  Proxy :4456          Admin :4457        │
                    └──────────────────────────────────────────┘
```

### Dual-Server Model

- **Proxy server** (`:4456`) — All inbound traffic. Route match → auth → hooks → upstream proxy.
- **Admin server** (`:4457`) — Health checks, route CRUD, cache management. **Never expose to the internet.**

### Request Pipeline

1. **Route Matching** — match `(host, path, method)` against the compiled route table. Glob patterns (`/api/**`) are compiled to regex once at startup and on hot-reload.
2. **Authentication** — resolve the auth provider for the route (route-level override → site default → anonymous). Produce an `Identity` struct.
3. **Template Context** — build per-request context with `identity.*`, `body.*`, `request_id`, `api_key.*`.
4. **Pre-Request Hooks** — execute the route's `hooks.pre_request` chain: inject headers, transform body, mint JWT.
5. **Upstream Proxy** — forward the mutated request. Hop-by-hop headers are stripped.
6. **Response** — stream through `SseStreamProcessor` (AG-UI / A2UI filtering, token counting, backpressure) or buffer and forward.

---

## Configuration

Configuration is resolved in priority order (highest first):

```
CLI flags  >  Environment variables  >  config.yaml
```

All three sources support the same settings. A value supplied by a higher-priority source always wins, regardless of what the file says. When the YAML file changes on disk, Flint Gate reloads it automatically and re-applies any CLI / env overrides on top.

---

### 1 — YAML file (base configuration)

Copy `config.example.yaml` to `config.yaml` and edit it. The file is watched for changes; edits are applied within ~200 ms without restarting the process.

```yaml
# config.yaml

server:
  listen:       "0.0.0.0:4456"   # proxy port
  admin_listen: "0.0.0.0:4457"   # admin port (keep private)

database:
  url:             "postgres://user:pass@localhost:5432/flintgate"
  max_connections: 20
  override_yaml:   false          # when true, DB routes take precedence

cache:
  l1:
    max_capacity: 10000
    ttl_seconds:  60
  invalidation_channel: "flintgate_config_changed"

auth_providers:
  kratos:
    type:     kratos
    base_url: "http://kratos:4433"
  anon:
    type:            anonymous
    default_subject: "anonymous"

jwt:
  signing_algorithm: "HS256"
  signing_key_secret: "change-me-in-production"
  issuer:             "https://gate.example.com"
  default_ttl_seconds: 300

sites:
  - id:             "my-app"
    domains:        ["app.example.com", "localhost:3000"]
    default_auth:   kratos
    default_upstream: "http://app-backend:3001"

routes:
  - id:   "chat-stream"
    site: "my-app"
    match:
      path:    "/api/chat/**"
      methods: ["POST"]
    upstream: "http://llm:8000/v1/chat/completions"
    auth:     kratos
    hooks:
      pre_request:
        - type: claims_enhancement
          config:
            inject_headers:
              X-User-Id:    "{{ identity.id }}"
              X-User-Email: "{{ identity.traits.email }}"
            mint_jwt:
              enabled: true
              additional_claims: { scope: "chat" }
        - type: body_transform
          config:
            set_fields:
              user:  "{{ identity.id }}"
              model: "{{ coalesce(body.model, 'claude-sonnet-4-6') }}"
    stream:
      enabled:  true
      protocol: sse
      ai:
        ag_ui:
          enabled:        true
          validate_events: true
          allowed_events:
            - TEXT_MESSAGE_START
            - TEXT_MESSAGE_CONTENT
            - TEXT_MESSAGE_END
            - RUN_STARTED
            - RUN_FINISHED
            - RUN_ERROR
        backpressure:
          max_stream_duration_seconds: 300
          max_events: 10000
```

#### Full YAML reference

<details>
<summary>Click to expand complete field reference</summary>

```yaml
server:
  listen:       "0.0.0.0:4456"     # string  — proxy bind address
  admin_listen: "0.0.0.0:4457"     # string  — admin bind address
  tls:
    enabled:   false               # bool
    cert_path: "/path/to/cert.pem" # string?
    key_path:  "/path/to/key.pem"  # string?

database:
  url:             ""              # string  — Postgres DSN; empty = disabled
  max_connections: 20              # u32
  override_yaml:   false           # bool    — DB routes win over YAML routes

cache:
  l1:
    max_capacity: 10000            # u64     — max entries in moka cache
    ttl_seconds:  60               # u64     — entry TTL
  l2:
    enabled:   false               # bool    — Redis L2 cache tier
    redis_url: ""                  # string? — Redis connection URL
  invalidation_channel: "flintgate_config_changed"  # string

# Named auth providers — referenced by id in sites/routes
auth_providers:
  <name>:
    # Ory Kratos
    type:            kratos
    base_url:        "http://kratos:4433"
    forward_cookies: true           # bool — forward session cookie
    session_cookie:  "ory_kratos_session"

    # Inbound JWT verification (Phase 3)
    type:            jwt
    jwks_url:        "https://auth.example.com/.well-known/jwks.json"
    issuer:          "https://auth.example.com"   # string?
    audience:        "flint-gate"                 # string?
    leeway_seconds:  5                            # u64

    # API key lookup (Phase 3)
    type:   api_key
    header: "X-API-Key"   # string — header to extract key from
    store:  database       # string — "database" only currently

    # Anonymous / passthrough
    type:            anonymous
    default_subject: "anonymous"   # string

jwt:
  signing_algorithm:  "HS256"      # HS256 | HS384 | HS512 | RS256 | RS384 | RS512 | ES256 | ES384
  signing_key_secret: ""           # string — HMAC secret (HS*)
  signing_key_path:   ""           # string — PEM file path (RS*/ES*)
  issuer:             "flint-gate" # string
  default_ttl_seconds: 300         # u64

sites:
  - id:              "site-id"     # string — referenced by routes
    domains:         []            # [string] — matched against Host header
    default_auth:    null          # string? — provider name
    default_upstream: null         # string? — base URL

routes:
  - id:      "route-id"            # string — unique
    site:    "site-id"             # string — must match a site id
    enabled: true                  # bool
    priority: 0                    # i32 — higher = matched first
    match:
      path:    "/api/**"           # string — glob pattern
      methods: []                  # [string] — empty = all methods
      host:    null                # string? — additional host filter
    upstream: null                 # string? — full URL; null = site default + path
    auth:     null                 # string? — provider name; null = site default
    hooks:
      pre_request:
        - type: claims_enhancement
          config:
            inject_headers:        # map<string, template>
              X-Header: "{{ identity.id }}"
            mint_jwt:
              enabled: false
              additional_claims: {}  # JSON object merged into JWT payload
        - type: body_transform
          config:
            set_fields:            # map<string, template>
              field: "{{ body.other_field }}"
      post_response:
        - type: stream_meter
          config:
            log_to_db: true
    stream:
      enabled:  false
      protocol: sse                # sse | websocket | ndjson
      ai:
        ag_ui:
          enabled:         false
          validate_events: false
          allowed_events:  []      # [string] — AG-UI event type names
          inject_metadata: {}      # map<string, template> → _gate_metadata
        a2ui:
          enabled:          false
          allowed_intents:  []     # [string] — A2UI intent names
        session_watchdog:
          enabled:                 false
          check_interval_seconds:  30
        backpressure:
          max_stream_duration_seconds: null  # u64?
          max_events:                  null  # u64?
```

</details>

---

### 2 — Environment variables

Environment variables are read by `clap` before CLI flags are applied. They are useful for secrets and per-deployment settings that should not live in version-controlled YAML files.

| Variable | Overrides | Default | Description |
|---|---|---|---|
| `FLINT_GATE_CONFIG` | `--config` | `config.yaml` | Path to YAML config file |
| `FLINT_GATE_LISTEN` | `--listen` | *(from YAML)* | Proxy listen address |
| `FLINT_GATE_ADMIN_LISTEN` | `--admin-listen` | *(from YAML)* | Admin listen address |
| `DATABASE_URL` | `--database-url` | *(from YAML)* | Postgres connection URL |
| `RUST_LOG` | `--log` | `info,flint_gate=debug` | Tracing filter string |
| `FLINT_GATE_JWT_SECRET` | `--jwt-secret` | *(from YAML)* | HS256 HMAC signing secret |
| `FLINT_GATE_JWT_KEY_PATH` | `--jwt-key-path` | *(from YAML)* | PEM key file for RS256/ES256 |

**Example — twelve-factor style `.env`:**

```bash
FLINT_GATE_CONFIG=/etc/flint-gate/config.yaml
DATABASE_URL=postgres://flintgate:s3cr3t@db.internal:5432/flintgate
FLINT_GATE_JWT_SECRET=a-very-long-random-string-at-least-32-chars
RUST_LOG=info
```

**Example — Docker:**

```bash
docker run \
  -e DATABASE_URL="postgres://..." \
  -e FLINT_GATE_JWT_SECRET="..." \
  -e RUST_LOG="debug" \
  -v $(pwd)/config.yaml:/app/config/config.yaml \
  -p 4456:4456 \
  flint-gate:latest
```

---

### 3 — CLI flags

CLI flags are the highest-priority configuration source. Any flag supplied here overrides both env vars and YAML.

```
Usage: flint-gate [OPTIONS]

Options:
  -c, --config <PATH>
          Path to the YAML configuration file
          [env: FLINT_GATE_CONFIG]
          [default: config.yaml]

      --listen <HOST:PORT>
          Proxy server listen address. Overrides server.listen in config.yaml
          [env: FLINT_GATE_LISTEN]

      --admin-listen <HOST:PORT>
          Admin API listen address. Overrides server.admin_listen in config.yaml
          [env: FLINT_GATE_ADMIN_LISTEN]

      --database-url <URL>
          Postgres connection URL. Overrides database.url in config.yaml
          [env: DATABASE_URL]

      --log <FILTER>
          Tracing filter (EnvFilter syntax)
          [env: RUST_LOG]
          [default: info,flint_gate=debug]

      --jwt-secret <SECRET>
          HMAC secret for HS256 JWT signing. Overrides jwt.signing_key_secret
          [env: FLINT_GATE_JWT_SECRET]

      --jwt-key-path <PATH>
          Path to PEM private key for RS256/ES256 signing. Overrides jwt.signing_key_path
          [env: FLINT_GATE_JWT_KEY_PATH]

  -h, --help     Print help
  -V, --version  Print version
```

**Common invocations:**

```bash
# Default — reads ./config.yaml
flint-gate

# Custom config file
flint-gate --config /etc/flint-gate/config.yaml

# Override listen address for local testing
flint-gate --listen 127.0.0.1:8080 --admin-listen 127.0.0.1:8081

# Supply secrets at runtime without touching config files
flint-gate \
  --database-url "postgres://prod-host/flintgate" \
  --jwt-secret   "$(vault kv get -field=secret secret/flintgate/jwt)"

# Verbose logging for a debugging session
flint-gate --log "debug,sqlx=warn,hyper=warn"
```

---

## Auth Providers

### Kratos session

Validates Ory Kratos sessions by calling `GET /sessions/whoami`. Forwards the session cookie and/or `Authorization: Bearer` header from the incoming request.

```yaml
auth_providers:
  my_kratos:
    type:            kratos
    base_url:        "http://kratos:4433"
    forward_cookies: true
    session_cookie:  "ory_kratos_session"
```

On success, produces an `Identity` with `id`, `traits`, `metadata_public`, `schema_id`, `session_id`, and `aal` populated from the Kratos response.

### JWT Bearer (Phase 3)

Verifies inbound `Authorization: Bearer <token>` against a JWKS endpoint.

```yaml
auth_providers:
  my_jwt:
    type:            jwt
    jwks_url:        "https://auth.example.com/.well-known/jwks.json"
    issuer:          "https://auth.example.com"
    audience:        "flint-gate"
    leeway_seconds:  5
```

### API Key (Phase 3)

Extracts a key from the configured header, SHA-256 hashes it, and looks it up in the `api_keys` table.

```yaml
auth_providers:
  my_api_key:
    type:   api_key
    header: "X-API-Key"
    store:  database
```

### Anonymous

Always succeeds. Used for public endpoints.

```yaml
auth_providers:
  public:
    type:            anonymous
    default_subject: "anonymous"
```

---

## Template Engine

Hook configurations use `{{ expression }}` placeholders resolved against a per-request context:

| Expression | Resolves to |
|---|---|
| `{{ identity.id }}` | Authenticated user ID |
| `{{ identity.traits.email }}` | Dot-path into identity traits |
| `{{ identity.metadata_public.org_id }}` | Public metadata field |
| `{{ body.model }}` | Field from the JSON request body |
| `{{ body.messages.0.content }}` | Indexed array access |
| `{{ request_id }}` | UUID generated for this request |
| `{{ api_key.client_id }}` | API key client ID |
| `{{ api_key.scopes }}` | Comma-joined scope list |
| `{{ coalesce(body.model, 'claude-sonnet-4-6') }}` | First non-empty value |
| `{{ coalesce(identity.traits.name, identity.id) }}` | Coalesce identity fields |

Unknown expressions resolve to an empty string. Nested dot-paths walk both objects and arrays.

---

## Hooks

### `claims_enhancement`

Injects HTTP headers into the upstream request and optionally mints an outbound JWT.

```yaml
- type: claims_enhancement
  config:
    inject_headers:
      X-User-Id:    "{{ identity.id }}"
      X-User-Email: "{{ identity.traits.email }}"
      X-Org-Id:     "{{ identity.metadata_public.org_id }}"
      X-Request-Id: "{{ request_id }}"
    mint_jwt:
      enabled: true
      additional_claims:
        scope:  "chat"
        org_id: "{{ identity.metadata_public.org_id }}"
```

When `mint_jwt.enabled` is `true`, the minted token is injected as `Authorization: Bearer <token>`, replacing any existing Authorization header forwarded to upstream.

### `body_transform`

Modifies or adds fields in the JSON request body before forwarding.

```yaml
- type: body_transform
  config:
    set_fields:
      user:        "{{ identity.id }}"
      model:       "{{ coalesce(body.model, 'claude-sonnet-4-6') }}"
      temperature: "0.7"
```

Non-JSON request bodies are passed through unchanged.

### `stream_meter` (post-response)

Records token counts and stream duration to the `usage_events` table for billing.

```yaml
post_response:
  - type: stream_meter
    config:
      log_to_db: true
```

---

## Streaming (AG-UI / A2UI)

### AG-UI (CopilotKit)

When `stream.ai.ag_ui.enabled` is `true`, the SSE stream processor parses each `data:` frame as an AG-UI event and can:

- **Validate** — drop events not in `allowed_events`
- **Meter** — count `TEXT_MESSAGE_CONTENT` deltas for token estimation

```yaml
stream:
  enabled:  true
  protocol: sse
  ai:
    ag_ui:
      enabled:         true
      validate_events: true
      allowed_events:
        - TEXT_MESSAGE_START
        - TEXT_MESSAGE_CONTENT
        - TEXT_MESSAGE_END
        - TOOL_CALL_START
        - TOOL_CALL_ARGS
        - TOOL_CALL_END
        - RUN_STARTED
        - RUN_FINISHED
        - RUN_ERROR
        - STEP_STARTED
        - STEP_FINISHED
        - STATE_SNAPSHOT
        - STATE_DELTA
        - RAW
```

### A2UI (intent-driven UI)

When `stream.ai.a2ui.enabled` is `true`, the processor inspects events with an `intent` field and can:

- **Filter by intent** — drop events with intents not in `allowed_intents`
- **Filter by scope** — check `a2ui_scopes` claim against required scope per intent
- **Inject theme** — add `_theme` object to `render_component` payloads

```yaml
ai:
  a2ui:
    enabled: true
    allowed_intents:
      - render_component
      - show_toast
      - stream_content
      - navigate
      - show_modal
      - request_input
      - update_state
```

### Backpressure

```yaml
ai:
  backpressure:
    max_stream_duration_seconds: 300   # terminate after 5 minutes
    max_events: 10000                   # terminate after 10k events
```

When a limit is hit, Flint Gate emits a `RUN_ERROR` SSE event and closes the stream.

---

## Admin API

All endpoints are on the admin port (`:4457`).

**Authentication.** The admin API is unauthenticated by default and, in that
state, is only permitted on a **loopback** bind — flint-gate **refuses to start**
if `admin_listen` is non-loopback while `server.admin_auth` is unset (fail-safe
against exposing an unauthenticated control plane). To expose the admin API (and
the web UI) beyond loopback, set `server.admin_auth` to an auth provider
(`type: jwt` — e.g. an Ory Hydra-issued Bearer token — or `type: kratos` session;
any JWKS-backed JWT provider works). When enabled, every admin request is
authenticated except the `/health` and `/ready` probes. See `config.example.yaml`.

### Health & readiness

```bash
# Liveness probe — always 200
GET :4457/health

# Readiness — checks DB connectivity
GET :4457/ready
```

### Cache

```bash
# Entry counts per cache tier
GET  :4457/cache/stats

# Flush all caches immediately
POST :4457/cache/invalidate
```

### Routes (requires database)

```bash
# List all enabled routes
GET :4457/routes

# Create or update a route
POST :4457/routes
Content-Type: application/json
{
  "id": "my-route",
  "site": "my-app",
  "match": { "path": "/api/**", "methods": ["GET"] },
  "upstream": "http://backend:3000",
  "priority": 10,
  "enabled": true
}

# Get a single route
GET :4457/routes/{id}

# Update by ID
PUT :4457/routes/{id}

# Delete
DELETE :4457/routes/{id}
```

Route mutations automatically send `SELECT pg_notify('flintgate_config_changed', 'routes')` so all running instances invalidate their caches within milliseconds.

---

## Non-Human Identities (agents & services)

Flint Gate authorizes **non-human identities** as first-class Cedar principals,
distinct from human users. A delegated token (from RFC 8693 token exchange,
carrying an `act` claim) authorizes as an **`Agent`**; a client-credentials
service token (carrying a `client_id`) authorizes as a **`Service`**; everything
else is a **`User`**. Because the Cedar entity *type* differs, a policy can grant
an agent something a user must not have — and vice-versa:

```cedar
// Agents may call the deploy tool; a human user with the same id may not.
permit(principal == Agent::"ci-bot", action == Action::"call_tool", resource == Route::"deploy");

// A service identity may read metrics.
permit(principal == Service::"metrics-scraper", action, resource == Route::"metrics");

// Humans keep their own policies.
permit(principal == User::"alice", action, resource);
```

### Lifecycle (Admin API)

```bash
GET    :4457/agent-identities              # list all NHIs
POST   :4457/agent-identities              # issue { "id": "...", "kind": "agent"|"service", "label"? }
POST   :4457/agent-identities/{id}/rotate  # stamp rotated_at
DELETE :4457/agent-identities/{id}         # revoke
```

Every issue / rotate / revoke is written to the authz **audit trail**. **Revocation
is fail-closed**: once revoked, the identity is denied on its **next authorize** —
the check runs per request and denies on a lookup error rather than letting a
revoked agent through. Manage identities from the **Agents** tab in the web UI.

---

## Database Schema

Flint Gate applies its own schema at startup (`migrate()` — idempotent, uses `CREATE TABLE IF NOT EXISTS`).

```sql
gate_routes     -- runtime-managed route configs (JSONB)
gate_sites      -- site definitions
api_keys        -- SHA-256 hashed API keys with scopes
usage_events    -- per-request token/duration metering
jwt_signing_keys -- key rotation (future)
```

---

## Running

### Local (binary)

```bash
# Build
cargo build --release

# Run with a config file
./target/release/flint-gate --config config.yaml

# Override listen address
./target/release/flint-gate --listen 127.0.0.1:8080 --admin-listen 127.0.0.1:8081

# No config file — use defaults + CLI flags
./target/release/flint-gate \
  --listen       0.0.0.0:4456 \
  --database-url postgres://localhost/flintgate \
  --jwt-secret   my-secret
```

### Docker

```bash
docker build -t flint-gate:latest .

docker run \
  -p 4456:4456 \
  -p 4457:4457 \
  -v $(pwd)/config.yaml:/app/config/config.yaml \
  -e DATABASE_URL="postgres://user:pass@host/flintgate" \
  -e FLINT_GATE_JWT_SECRET="change-me" \
  flint-gate:latest
```

### Docker Compose

```yaml
services:
  flint-gate:
    image: flint-gate:latest
    ports:
      - "4456:4456"
      # Admin port: do NOT expose to the internet
    environment:
      DATABASE_URL: "postgres://flintgate:secret@postgres/flintgate"
      FLINT_GATE_JWT_SECRET: "${JWT_SECRET}"
      RUST_LOG: "info"
    volumes:
      - ./config.yaml:/app/config/config.yaml:ro
    depends_on:
      - postgres
    restart: unless-stopped

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: flintgate
      POSTGRES_PASSWORD: secret
      POSTGRES_DB: flintgate
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  pgdata:
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: flint-gate
spec:
  replicas: 2
  selector:
    matchLabels: { app: flint-gate }
  template:
    metadata:
      labels: { app: flint-gate }
    spec:
      containers:
        - name: flint-gate
          image: flint-gate:latest
          ports:
            - containerPort: 4456  # proxy
            - containerPort: 4457  # admin (ClusterIP only)
          env:
            - name: DATABASE_URL
              valueFrom:
                secretKeyRef: { name: flintgate-secrets, key: database-url }
            - name: FLINT_GATE_JWT_SECRET
              valueFrom:
                secretKeyRef: { name: flintgate-secrets, key: jwt-secret }
            - name: RUST_LOG
              value: "info"
          livenessProbe:
            httpGet: { path: /health, port: 4457 }
            initialDelaySeconds: 5
          readinessProbe:
            httpGet: { path: /ready, port: 4457 }
            initialDelaySeconds: 5
          volumeMounts:
            - name: config
              mountPath: /app/config
              readOnly: true
      volumes:
        - name: config
          configMap: { name: flintgate-config }
```

All replicas connect to the same Postgres. `LISTEN/NOTIFY` keeps caches in sync across pods automatically.

---

## Logging

Flint Gate uses [`tracing`](https://docs.rs/tracing) with structured JSON-compatible output.

```bash
# Default
RUST_LOG="info,flint_gate=debug"

# Verbose — all debug output
RUST_LOG="debug"

# Quiet — warnings only, suppress sqlx and hyper noise
RUST_LOG="warn,flint_gate=info,sqlx=warn,hyper=warn"

# Per-module granularity
RUST_LOG="info,flint_gate::middleware=debug,flint_gate::auth=trace"
```

Or via CLI flag: `flint-gate --log "debug,sqlx=warn"`

---

## Development

```bash
# Check for errors
cargo check

# Run all tests (54 tests)
cargo test

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt

# Build release binary
cargo build --release
```

### Project layout

```
src/
├── main.rs                  # CLI, startup wiring
├── config/
│   ├── types.rs             # GateConfig and all nested structs (the schema)
│   ├── template.rs          # {{ expression }} engine
│   └── loader.rs            # YAML load + notify hot-reload
├── auth/
│   ├── mod.rs               # Authenticator trait, factory, AnonymousAuthenticator
│   ├── identity.rs          # Universal Identity struct
│   ├── kratos.rs            # Kratos /sessions/whoami
│   └── jwt_mint.rs          # Outbound JWT minting (HS*/RS*/ES*)
├── stream/
│   ├── ag_ui.rs             # AG-UI event types, validation, token counting
│   ├── a2ui.rs              # A2UI intent filtering, scope checking, theme injection
│   └── processor.rs         # SseStreamProcessor (line-buffered SSE engine)
├── proxy/
│   └── router.rs            # Route compiler (glob→regex), matcher, upstream resolver
├── cache/
│   └── mod.rs               # GateCache (moka), Postgres LISTEN/NOTIFY listener
├── db/
│   └── mod.rs               # Database (sqlx PgPool), schema DDL, CRUD
├── admin/
│   └── mod.rs               # Admin Axum router (:4457)
└── middleware/
    └── pipeline.rs          # proxy_handler — the full 10-step request pipeline
```

---

## License

MIT — Copyright 2025 KnowMe, LLC
