# Admin API

The admin API runs on the admin server (default `:4457`). It is for internal operations only and should never be exposed to the public internet.

All endpoints return JSON. Endpoints that require a database return HTTP 501 when `database.url` is not configured.

## Health and readiness

### `GET /health`

Liveness probe. Always returns 200.

```bash
curl http://localhost:4457/health
```

Response:

```json
{
  "status": "ok",
  "service": "flint-gate"
}
```

### `GET /ready`

Readiness probe. Returns 503 if Postgres is unreachable.

```bash
curl http://localhost:4457/ready
```

Response:

```json
{
  "status": "ready",
  "db": "ok"
}
```

## Cache

### `GET /cache/stats`

Returns entry counts per cache tier.

```bash
curl http://localhost:4457/cache/stats
```

Response:

```json
{
  "l1": 42,
  "l2": 0
}
```

### `POST /cache/invalidate`

Flushes all caches immediately.

```bash
curl -X POST http://localhost:4457/cache/invalidate
```

Response:

```json
{
  "status": "invalidated"
}
```

## Routes

Routes managed through the admin API are stored in the `gate_routes` table. Mutations trigger `pg_notify('flintgate_config_changed', 'routes')` so all running instances invalidate their caches.

### `GET /routes`

Lists all routes. Returns DB routes when available, otherwise YAML-configured route ids.

```bash
curl http://localhost:4457/routes
```

Response (with database):

```json
{
  "routes": [
    {
      "id": "chat-stream",
      "site": "my-app",
      "match": { "path": "/api/chat/**", "methods": ["POST"] },
      "upstream": "http://llm-backend:8000/v1/chat/completions",
      "priority": 10,
      "enabled": true
    }
  ],
  "source": "database"
}
```

### `POST /routes`

Creates or updates a route.

```bash
curl -X POST http://localhost:4457/routes \
  -H "Content-Type: application/json" \
  -d '{
    "id": "api-completions",
    "site": "api",
    "match": { "path": "/v1/completions", "methods": ["POST"] },
    "upstream": "http://llm-backend:8000/v1/completions",
    "auth": "api_key",
    "priority": 5,
    "enabled": true
  }'
```

Response:

```json
{
  "status": "ok",
  "id": "api-completions"
}
```

### `GET /routes/{id}`

Returns a single route.

```bash
curl http://localhost:4457/routes/api-completions
```

Response (200) or `{"error": "not found"}` (404).

### `PUT /routes/{id}`

Updates a route by id. The id from the URL is inserted into the payload if missing.

```bash
curl -X PUT http://localhost:4457/routes/api-completions \
  -H "Content-Type: application/json" \
  -d '{
    "site": "api",
    "match": { "path": "/v1/completions", "methods": ["POST", "GET"] },
    "upstream": "http://llm-backend:8000/v1/completions",
    "priority": 10,
    "enabled": true
  }'
```

### `DELETE /routes/{id}`

Deletes a route.

```bash
curl -X DELETE http://localhost:4457/routes/api-completions
```

Response:

```json
{
  "status": "deleted",
  "id": "api-completions"
}
```

## API keys

API keys are SHA-256 hashed before storage. The raw key is returned only once, on creation.

### `GET /api-keys`

Lists active API keys. Key hashes are not returned.

```bash
curl http://localhost:4457/api-keys
```

Response:

```json
{
  "api_keys": [
    {
      "id": "...",
      "client_id": "billing-svc",
      "scopes": ["read:invoices"],
      "expires_at": "2026-01-01T00:00:00Z"
    }
  ]
}
```

### `POST /api-keys`

Creates a new API key.

```bash
curl -X POST http://localhost:4457/api-keys \
  -H "Content-Type: application/json" \
  -d '{
    "client_id": "mobile-app",
    "scopes": ["chat", "embed"],
    "expires_at": "2026-01-01T00:00:00Z"
  }'
```

Response:

```json
{
  "id": "...",
  "client_id": "mobile-app",
  "scopes": ["chat", "embed"],
  "expires_at": "2026-01-01T00:00:00Z",
  "key": "flint_...",
  "note": "Store this key securely — it will not be shown again."
}
```

### `DELETE /api-keys/{id}`

Revokes an API key by id.

```bash
curl -X DELETE http://localhost:4457/api-keys/550e8400-e29b-41d4-a716-446655440000
```

Response:

```json
{
  "status": "revoked",
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

## Signing keys

### `GET /signing-keys`

Lists JWT signing keys.

```bash
curl http://localhost:4457/signing-keys
```

### `POST /signing-keys`

Inserts a new signing key and deactivates all prior keys.

```bash
curl -X POST http://localhost:4457/signing-keys \
  -H "Content-Type: application/json" \
  -d '{
    "id": "key-2026",
    "algorithm": "RS256",
    "public_key": "-----BEGIN PUBLIC KEY-----...",
    "private_key": "-----BEGIN PRIVATE KEY-----..."
  }'
```

Response:

```json
{
  "status": "activated",
  "id": "key-2026",
  "algorithm": "RS256",
  "note": "All prior signing keys deactivated."
}
```

### `DELETE /signing-keys/{id}`

Deactivates a signing key.

```bash
curl -X DELETE http://localhost:4457/signing-keys/key-2026
```

Response:

```json
{
  "status": "deactivated",
  "id": "key-2026"
}
```

## Errors

| Status | Meaning |
|--------|---------|
| 400 | Bad request, for example missing route id |
| 404 | Resource not found |
| 501 | Database not configured |
| 503 | Service not ready (DB unreachable) |
