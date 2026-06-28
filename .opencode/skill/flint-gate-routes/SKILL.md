---
name: flint-gate-routes
description: Create, list, update, and delete runtime routes via the flint-gate admin API on port 4457. Use when the user says "manage routes" or "add route flint gate".
version: 0.1.0
license: MIT
---

# flint-gate-routes

Manage routes through the admin API on port `4457`. DB routes take effect cluster-wide when `database.override_yaml: true`, and are broadcast to all instances via Postgres `pg_notify('flintgate_config_changed', 'routes')`. Never expose the admin port to the public internet.

## Base

```
http://<admin-host>:4457
```

Set `<admin-host>` to `localhost` for local dev, the service DNS (`flint-gate.default.svc`) in Kubernetes, or the container name in docker-compose.

## List enabled routes

```bash
curl -s http://localhost:4457/routes | jq
```

Returns a JSON array of route objects.

## Get a single route

```bash
curl -s http://localhost:4457/routes/{id} | jq
```

404 if not found.

## Create a route

```bash
curl -s -X POST http://localhost:4457/routes \
  -H 'Content-Type: application/json' \
  -d '{
    "id": "chat-stream",
    "site": "my-app",
    "match": { "path": "/api/chat/**", "methods": ["POST"] },
    "upstream": "http://llm:8000/v1/chat/completions",
    "auth": "kratos_session",
    "priority": 10,
    "enabled": true
  }' | jq
```

POST is upsert — an existing `id` is replaced.

## Update a route

```bash
curl -s -X PUT http://localhost:4457/routes/chat-stream \
  -H 'Content-Type: application/json' \
  -d '{
    "site": "my-app",
    "match": { "path": "/api/chat/**", "methods": ["POST"] },
    "upstream": "http://llm:8001/v1/chat/completions",
    "priority": 20,
    "enabled": true
  }' | jq
```

PUT requires the full route body (no patch semantics).

## Disable without deleting

```bash
curl -s -X PUT http://localhost:4457/routes/chat-stream \
  -H 'Content-Type: application/json' \
  -d '{ "site":"my-app", "match":{"path":"/api/chat/**","methods":["POST"]},
        "upstream":"http://llm:8001/v1/chat/completions",
        "priority":20, "enabled":false }' | jq
```

## Delete a route

```bash
curl -s -X DELETE http://localhost:4457/routes/chat-stream -w '\n%{http_code}\n'
```

Expects 204 on success.

## Route object schema

| field      | type           | notes                                                  |
|------------|----------------|--------------------------------------------------------|
| `id`       | string         | unique; used in URL path                               |
| `site`     | string         | must exist in `sites`                                  |
| `match`    | object         | `{ "path": "<glob>", "methods": ["GET", ...] }`; `[]` = all methods |
| `upstream` | string (url)   | optional; falls back to `site.default_upstream`        |
| `auth`     | string         | provider name from `auth_providers`                    |
| `priority` | integer        | higher matched first; negative allowed                 |
| `enabled`  | bool           | default `true`                                         |
| `hooks`    | object         | optional; same shape as YAML hooks                     |
| `stream`   | object         | optional; same shape as YAML stream block              |

## After a mutation

1. Verify the cache invalidated:
   ```bash
   curl -s http://localhost:4457/cache/stats | jq
   ```
2. Force-flush if a node missed the notify:
   ```bash
   curl -s -X POST http://localhost:4457/cache/invalidate
   ```
3. Readiness check:
   ```bash
   curl -sf http://localhost:4457/ready && echo ready
   ```

## Notes

- If `database.override_yaml` is `false`, admin-API routes still persist but will not shadow YAML routes at match time. Set the flag or remove the YAML route before relying on the DB version.
- Glob patterns compile once per reload. Invalid globs return 4xx from POST/PUT.
- There is no batching endpoint. For large imports, loop POST sequentially.
