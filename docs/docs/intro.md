# Introduction

Flint Gate is an authentication proxy and API gateway designed for streaming LLM workloads. It validates incoming requests, applies pre-request transformations, and forwards them to upstream services, with explicit support for Server-Sent Events (SSE), AG-UI events, A2UI intents, and token metering.

It is built as a focused replacement for [Ory Oathkeeper](https://www.ory.sh/oathkeeper/) in AI-native stacks where long-lived streams and event-level processing are common.

## What Flint Gate does

- **Authenticates requests** using Ory Kratos sessions, inbound JWT verification, API keys stored in Postgres, or anonymous access.
- **Matches routes** by host, path glob, and HTTP method.
- **Transforms requests** before they reach upstream services — injecting headers, minting outbound JWTs, and rewriting JSON body fields with a template engine.
- **Proxies upstream** while stripping hop-by-hop headers and preserving streaming semantics.
- **Processes streams** by parsing SSE frames, validating AG-UI event types, filtering A2UI intents, estimating token usage, and enforcing backpressure limits.
- **Exposes an admin API** for health checks, cache management, route CRUD, API key management, and JWT signing key rotation.

## Key concepts

### Dual-server model

Flint Gate runs two HTTP servers:

| Server | Default port | Purpose |
|--------|--------------|---------|
| Proxy server | `4456` | Receives all inbound traffic. |
| Admin server | `4457` | Internal operations only. Do not expose to the public internet. |

### Sites

A site maps one or more domains to a default auth provider and default upstream base URL. Routes belong to a site. If a route does not specify an upstream or auth provider, the site default is used.

### Routes

A route defines how a request is handled:

- `match` — path glob (for example `/api/chat/**`) and optional HTTP methods.
- `auth` — which named auth provider to use.
- `upstream` — where to send the request.
- `hooks` — pre-request transformations and post-response metering.
- `stream` — streaming protocol configuration and AI event handling.

Routes are matched by priority (higher first), then by path specificity (longer patterns first).

### Auth providers

Named providers declared in `auth_providers` are referenced by routes:

- `kratos` — calls Ory Kratos `/sessions/whoami` to validate the session cookie or bearer token.
- `jwt` — verifies an inbound `Authorization: Bearer` token against a JWKS endpoint.
- `api_key` — extracts a key from a configured header and looks up its SHA-256 hash in the `api_keys` table.
- `anonymous` — always succeeds, used for public endpoints.

### Template engine

Hooks use `{{ expression }}` placeholders resolved per request against a context containing:

- `identity.id`, `identity.traits.*`, `identity.metadata_public.*`
- `body.*` — fields from the JSON request body
- `request_id` — a UUID generated for the request
- `api_key.client_id`, `api_key.scopes`

Example:

```yaml
inject_headers:
  X-User-Id: "{{ identity.id }}"
  X-Model: "{{ coalesce(body.model, 'claude-sonnet-4-6') }}"
```

### Streaming and metering

For SSE streams, Flint Gate can parse each `data:` frame and:

- Validate AG-UI event names against an allowlist.
- Filter A2UI intents.
- Count `TEXT_MESSAGE_CONTENT` deltas to estimate token usage.
- Emit a `RUN_ERROR` event and close the stream when backpressure limits are reached.

### Configuration sources

Settings are resolved in priority order, highest first:

```
CLI flags > environment variables > config.yaml
```

Changes to `config.yaml` on disk are reloaded automatically within approximately 200 ms.

## When to use Flint Gate

Flint Gate fits when:

- Your application proxies chat/completion endpoints that return SSE streams.
- You need auth decisions before traffic reaches the upstream service.
- You want to inject trusted identity headers or mint outbound JWTs for backend services.
- You want runtime route management stored in Postgres rather than only file-based config.

It is not a general-purpose reverse proxy like NGINX or Envoy and does not replace a full service mesh.
