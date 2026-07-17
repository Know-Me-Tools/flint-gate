---
license: MIT
name: flint-gate-config
description: Edit and validate flint-gate config.yaml — sites, routes, auth providers, hooks, JWT, cache, and stream/AI options. Use when the user says "configure flint gate", "flint gate config", or "edit gateway config".
---

# flint-gate-config

Authoring and validation guidance for `config.yaml` (default path `/app/config/config.yaml`, override with `FLINT_GATE_CONFIG`). Flint Gate hot-reloads this file within ~200ms.

## Top-level schema

```yaml
server:        # required — bind addresses and TLS
database:      # required — Postgres connection + override_yaml flag
cache:         # required — L1/L2 tiers + invalidation channel
auth_providers:# map<name, provider> — referenced by routes/sites
jwt:           # outbound JWT signing config (for claims_enhancement hook)
sites:         # list<site> — domain → upstream/auth mapping
routes:        # list<route> — match rules, evaluated by priority then specificity
```

## server

```yaml
server:
  listen: "0.0.0.0:4456"        # proxy port — public-facing
  admin_listen: "0.0.0.0:4457"  # admin port — NEVER expose to internet
  tls:
    enabled: false
    cert_path: "/etc/flint-gate/tls/cert.pem"
    key_path:  "/etc/flint-gate/tls/key.pem"
```

Overrides: `--listen`, `--admin-listen` (env: `FLINT_GATE_LISTEN`, `FLINT_GATE_ADMIN_LISTEN`).

## database

```yaml
database:
  url: "postgres://flintgate:secret@localhost:5432/flintgate"
  max_connections: 20
  override_yaml: false   # when true, DB-stored routes win over YAML routes
```

## cache

```yaml
cache:
  l1:
    max_capacity: 10000   # in-memory entry cap
    ttl_seconds: 60
  l2:
    enabled: false
    redis_url: "redis://localhost:6379"
  invalidation_channel: "flintgate_config_changed"  # Postgres LISTEN channel
```

## auth_providers

Each provider has a `type`. Types: `kratos`, `jwt`, `api_key`, `anonymous`.

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
    leeway_seconds: 5

  api_key:
    type: api_key
    header: "X-API-Key"
    store: database          # currently the only store

  passthrough:
    type: anonymous
    default_subject: "anonymous"
```

Behavior:
- `kratos` — calls `GET /sessions/whoami` on `base_url`, forwards `session_cookie`.
- `jwt` — verifies `Authorization: Bearer <token>` against JWKS, checks `iss`/`aud` with clock `leeway_seconds`.
- `api_key` — reads key from `header`, SHA-256 hashes it, looks it up in `api_keys` table. (Phase 3.)
- `anonymous` — always authenticates; sets subject to `default_subject`.

## jwt (outbound minting)

Used only by the `claims_enhancement` hook when `mint_jwt.enabled: true`.

```yaml
jwt:
  signing_algorithm: "HS256"   # HS256|HS384|HS512|RS256|RS384|RS512|ES256|ES384
  signing_key_secret: ""       # for HS* — put in env FLINT_GATE_JWT_SECRET in prod
  signing_key_path:   ""       # for RS*/ES* — PEM file path (env FLINT_GATE_JWT_KEY_PATH)
  issuer: "https://gate.example.com"
  default_ttl_seconds: 300
```

## sites

```yaml
sites:
  - id: "my-app"
    domains: ["app.example.com", "localhost:3000"]
    default_auth: kratos_session
    default_upstream: "http://app-backend:3001"
```

A site without an explicit `upstream` on a matched route falls back to `default_upstream`.

## routes

Matches sorted by `priority` desc, then path specificity (longer globs first). Glob patterns compile to regex once at startup/hot-reload.

```yaml
routes:
  - id: "chat-stream"            # unique string
    site: "my-app"               # references sites[].id
    match:
      path: "/api/chat/**"       # glob
      methods: ["POST"]          # [] means all methods
    upstream: "http://llm:8000/v1/chat/completions"
    auth: kratos_session         # provider name; omit for site default
    priority: 10                 # higher = first; negative allowed
    enabled: true                # default true
    hooks:
      pre_request:  [ ... ]
      post_response: [ ... ]
    stream:
      enabled: true
      protocol: sse
      ai: { ... }
```

### hooks

```yaml
hooks:
  pre_request:
    - type: claims_enhancement
      config:
        inject_headers:
          X-User-Id: "{{ identity.id }}"
          X-User-Email: "{{ identity.traits.email }}"
        mint_jwt:
          enabled: true
          additional_claims:
            scope: "chat"
            org_id: "{{ identity.metadata_public.org_id }}"
    - type: body_transform
      config:
        set_fields:
          user: "{{ identity.id }}"
          model: "{{ coalesce(body.model, 'Codex-sonnet-4-6') }}"
  post_response:
    - type: stream_meter
      config:
        log_to_db: true
```

Template context: `identity.*` (from auth provider), `body.*` (parsed request body for transforms), `api_key.*` (`client_id`, `scopes`), `request_id`. Helpers: `coalesce(...)`.

### stream (AI/streaming routes)

```yaml
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
        - TOOL_CALL_START
        - TOOL_CALL_ARGS
        - TOOL_CALL_END
        - RUN_STARTED
        - RUN_FINISHED
        - RUN_ERROR
        - STEP_STARTED
        - STEP_FINISHED
    a2ui:
      enabled: true
      allowed_intents: [render_component, show_toast, stream_content]
      theme: { mode: dark, primary: "#2563eb" }
    session_watchdog:
      enabled: true
      check_interval_seconds: 30
    backpressure:
      max_stream_duration_seconds: 300
      max_events: 10000
```

## Workflow

1. Locate `config.yaml` (env `FLINT_GATE_CONFIG`, otherwise `/app/config/config.yaml`). In dev repos this is `config.example.yaml` mounted read-only.
2. Make the smallest possible edit. Preserve field order and indentation — YAML matters.
3. If `database.override_yaml: true`, routes authored here are shadowed by DB routes. Tell the user to use the admin API instead (see `flint-gate-routes` skill).
4. After saving, check `/health` and `/ready` on the admin port and tail logs for hot-reload confirmation. No restart required.
5. Never put secrets in `signing_key_secret`. Reference them via env (`FLINT_GATE_JWT_SECRET`, `FLINT_GATE_JWT_KEY_PATH`).

## Common mistakes

- Forgetting `methods: []` on a catch-all — empty list means all methods, omitting the key may match nothing.
- Referencing a provider name in a route that isn't defined under `auth_providers`.
- Exposing `admin_listen` (`:4457`) to the public internet. It must stay ClusterIP/internal.
- Setting `priority` without considering specificity — two routes at the same priority resolve by path length.
- Enabling `stream.enabled` on a non-SSE upstream; protocol must match the upstream response.
