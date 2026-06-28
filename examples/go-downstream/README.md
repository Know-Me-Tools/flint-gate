# Go Downstream Service Example

A minimal Go HTTP service that sits behind Flint Gate and uses the Go SDK middleware to rehydrate the authenticated identity from request headers.

## Setup

```bash
cd examples/go-downstream
go mod tidy
```

## Run

```bash
# Production mode: reject any request that did not come through Flint Gate
go run main.go

# Or allow direct local testing
FLINT_REQUIRE_HEADER=false go run main.go
```

## Expected behavior

When proxied through Flint Gate, the injected identity headers are parsed and attached to the request context:

```bash
curl -H "X-Flint-Identity-Provider: kratos" \
     -H "X-Flint-Identity-Subject: user-123" \
     -H "X-Flint-Identity-Scopes: chat billing" \
     http://localhost:8080/api/hello
```

Response:

```json
{
  "request_id": "svc-a1b2c3d4e5f6",
  "subject": "user-123",
  "provider": "kratos",
  "scopes": ["chat", "billing"],
  "message": "hello from downstream"
}
```

Direct request in production mode:

```bash
curl http://localhost:8080/api/hello
```

Response: `401 unauthorized: missing flint-gate identity`

The `/api/admin` route requires the `admin` scope. A request with only `chat` scope returns `403 forbidden`.
