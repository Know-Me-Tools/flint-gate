# Configuration Reference

Flint Gate configuration is resolved in priority order:

```
CLI flags > environment variables > config.yaml
```

Changes to `config.yaml` are reloaded automatically within approximately 200 ms. CLI and environment overrides are reapplied on top of the reloaded file.

## Environment variables

| Variable | Overrides | Default | Description |
|----------|-----------|---------|-------------|
| `FLINT_GATE_CONFIG` | `--config` | `config.yaml` | Path to YAML config file |
| `FLINT_GATE_LISTEN` | `--listen` | From YAML | Proxy listen address |
| `FLINT_GATE_ADMIN_LISTEN` | `--admin-listen` | From YAML | Admin listen address |
| `DATABASE_URL` | `--database-url` | From YAML | Postgres DSN |
| `RUST_LOG` | `--log` | `info,flint_gate=debug` | Tracing filter |
| `FLINT_GATE_JWT_SECRET` | `--jwt-secret` | From YAML | HS256 HMAC secret |
| `FLINT_GATE_JWT_KEY_PATH` | `--jwt-key-path` | From YAML | PEM key path for RS256/ES256 |

## CLI flags

```
Usage: flint-gate [OPTIONS]

Options:
  -c, --config <PATH>           Path to YAML config file
      --listen <HOST:PORT>      Proxy listen address
      --admin-listen <HOST:PORT> Admin API listen address
      --database-url <URL>      Postgres connection URL
      --log <FILTER>            Tracing filter (EnvFilter syntax)
      --jwt-secret <SECRET>      HMAC secret for HS256
      --jwt-key-path <PATH>     PEM private key for RS256/ES256
  -h, --help                    Print help
  -V, --version                 Print version
```

## YAML reference

### `server`

```yaml
server:
  listen: "0.0.0.0:4456"
  admin_listen: "0.0.0.0:4457"
  tls:
    enabled: false
    cert_path: "/etc/flint-gate/tls/cert.pem"
    key_path: "/etc/flint-gate/tls/key.pem"
```

| Field | Type | Description |
|-------|------|-------------|
| `listen` | string | Proxy bind address |
| `admin_listen` | string | Admin API bind address (internal only) |
| `tls.enabled` | bool | Enable TLS termination |
| `tls.cert_path` | string? | TLS certificate file |
| `tls.key_path` | string? | TLS private key file |

### `database`

```yaml
database:
  url: "postgres://user:pass@localhost:5432/flintgate"
  max_connections: 20
  override_yaml: false
```

| Field | Type | Description |
|-------|------|-------------|
| `url` | string | Postgres DSN; empty disables DB-backed features |
| `max_connections` | u32 | Connection pool size |
| `override_yaml` | bool | When true, DB routes take precedence over YAML routes |

### `cache`

```yaml
cache:
  l1:
    max_capacity: 10000
    ttl_seconds: 60
  l2:
    enabled: false
    redis_url: "redis://localhost:6379"
  invalidation_channel: "flintgate_config_changed"
```

| Field | Type | Description |
|-------|------|-------------|
| `l1.max_capacity` | u64 | Max entries in the in-memory cache |
| `l1.ttl_seconds` | u64 | Entry TTL |
| `l2.enabled` | bool | Enable Redis L2 cache |
| `l2.redis_url` | string? | Redis connection URL |
| `invalidation_channel` | string | Postgres `LISTEN` channel for invalidation |

### `auth_providers`

Named providers referenced by routes and sites.

#### Kratos

```yaml
auth_providers:
  my_kratos:
    type: kratos
    base_url: "http://kratos:4433"
    forward_cookies: true
    session_cookie: "ory_kratos_session"
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `kratos` |
| `base_url` | string | Ory Kratos public base URL |
| `forward_cookies` | bool | Forward session cookie to Kratos |
| `session_cookie` | string | Cookie name to read |

#### JWT

```yaml
auth_providers:
  my_jwt:
    type: jwt
    jwks_url: "https://auth.example.com/.well-known/jwks.json"
    issuer: "https://auth.example.com"
    audience: "flint-gate"
    leeway_seconds: 5
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `jwt` |
| `jwks_url` | string | JWKS endpoint |
| `issuer` | string? | Expected token issuer |
| `audience` | string? | Expected token audience |
| `leeway_seconds` | u64 | Clock skew tolerance |

#### API key

```yaml
auth_providers:
  my_api_key:
    type: api_key
    header: "X-API-Key"
    store: database
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `api_key` |
| `header` | string | Header to extract the key from |
| `store` | string | `database` only currently |

#### Anonymous

```yaml
auth_providers:
  public:
    type: anonymous
    default_subject: "anonymous"
```

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `anonymous` |
| `default_subject` | string | Subject assigned to the identity |

### `jwt`

Configures outbound JWT minting via the `claims_enhancement` hook.

```yaml
jwt:
  signing_algorithm: "HS256"
  signing_key_secret: "change-me-in-production"
  signing_key_path: ""
  issuer: "https://gate.example.com"
  default_ttl_seconds: 300
```

| Field | Type | Description |
|-------|------|-------------|
| `signing_algorithm` | string | `HS256`, `HS384`, `HS512`, `RS256`, `RS384`, `RS512`, `ES256`, `ES384` |
| `signing_key_secret` | string | HMAC secret for HS* |
| `signing_key_path` | string | PEM private key path for RS*/ES* |
| `issuer` | string | JWT issuer claim |
| `default_ttl_seconds` | u64 | Default token lifetime |

### `sites`

```yaml
sites:
  - id: "my-app"
    domains:
      - "app.example.com"
      - "localhost:3000"
    default_auth: kratos_session
    default_upstream: "http://app-backend:3001"
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique site identifier |
| `domains` | [string] | Host values matched against the `Host` header |
| `default_auth` | string? | Default auth provider for routes |
| `default_upstream` | string? | Default base URL for routes |

### `routes`

```yaml
routes:
  - id: "chat-stream"
    site: "my-app"
    enabled: true
    priority: 10
    match:
      path: "/api/chat/**"
      methods: ["POST"]
      host: null
    upstream: "http://llm-backend:8000/v1/chat/completions"
    auth: kratos_session
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
      post_response:
        - type: stream_meter
          config:
            log_to_db: true
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
        a2ui:
          enabled: true
          allowed_intents:
            - render_component
            - show_toast
        session_watchdog:
          enabled: true
          check_interval_seconds: 30
        backpressure:
          max_stream_duration_seconds: 300
          max_events: 10000
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique route identifier |
| `site` | string | Site id this route belongs to |
| `enabled` | bool | Whether the route is active |
| `priority` | i32 | Higher values match first |
| `match.path` | string | Glob pattern |
| `match.methods` | [string] | Allowed methods; empty = all |
| `match.host` | string? | Additional host filter |
| `upstream` | string? | Full upstream URL; null uses site default + request path |
| `auth` | string? | Auth provider name; null uses site default |
| `hooks.pre_request` | [object] | Claims enhancement and body transform hooks |
| `hooks.post_response` | [object] | Stream metering hooks |
| `stream.enabled` | bool | Enable streaming processing |
| `stream.protocol` | string | `sse`, `websocket`, or `ndjson` |
| `stream.ai.ag_ui.enabled` | bool | Enable AG-UI processing |
| `stream.ai.ag_ui.validate_events` | bool | Drop events not in `allowed_events` |
| `stream.ai.ag_ui.allowed_events` | [string] | AG-UI event names |
| `stream.ai.a2ui.enabled` | bool | Enable A2UI intent filtering |
| `stream.ai.a2ui.allowed_intents` | [string] | A2UI intent names |
| `stream.ai.session_watchdog.enabled` | bool | Terminate streams when sessions expire |
| `stream.ai.session_watchdog.check_interval_seconds` | u64 | How often to check session validity |
| `stream.ai.backpressure.max_stream_duration_seconds` | u64? | Hard stream duration limit |
| `stream.ai.backpressure.max_events` | u64? | Hard event count limit |

### `hooks` detail

#### `claims_enhancement`

Injects headers and optionally mints a JWT.

```yaml
- type: claims_enhancement
  config:
    inject_headers:
      X-User-Id: "{{ identity.id }}"
      X-Org-Id: "{{ identity.metadata_public.org_id }}"
    mint_jwt:
      enabled: true
      additional_claims:
        scope: "chat"
```

#### `body_transform`

Adds or replaces fields in a JSON request body.

```yaml
- type: body_transform
  config:
    set_fields:
      user: "{{ identity.id }}"
      model: "{{ coalesce(body.model, 'claude-sonnet-4-6') }}"
```

#### `stream_meter`

Records per-request token counts and stream duration to `usage_events`.

```yaml
post_response:
  - type: stream_meter
    config:
      log_to_db: true
```

## Logging

Controlled via `RUST_LOG` or `--log`:

```bash
# Default
RUST_LOG="info,flint_gate=debug"

# Verbose
RUST_LOG="debug"

# Quiet
RUST_LOG="warn,flint_gate=info,sqlx=warn,hyper=warn"
```
