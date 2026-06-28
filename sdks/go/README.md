# Flint Gate Go SDK

Client library for Go services that sit behind (or administer) the
[Flint Gate](../../README.md) AI auth proxy.

## Install

```bash
go get github.com/know-me-tools/flint-gate/sdks/go
```

The WebSocket client depends on [`nhooyr.io/websocket`](https://pkg.go.dev/nhooyr.io/websocket),
which is already declared in `go.mod`.

## Admin client

The admin client talks to Flint Gate's admin server (`:4457` by default).
Never expose the admin server to the public internet.

```go
import "github.com/know-me-tools/flint-gate/sdks/go"

client, err := flintgate.NewClient(flintgate.Options{
    BaseURL:    "http://127.0.0.1:4457",
    AdminToken: os.Getenv("FLINT_ADMIN_TOKEN"),
})
if err != nil { log.Fatal(err) }

// Health
h, err := client.GetHealth(ctx)

// Routes
routes, err := client.GetRoutes(ctx)
created, err := client.CreateRoute(ctx, flintgate.RouteConfig{
    ID:    "chat-v1",
    Site:  "default",
    Match: flintgate.RouteMatch{Path: "/api/chat/**", Methods: []string{"POST"}},
    Stream: flintgate.StreamCfg{Mode: "passthrough"},
    Enabled: true,
})
err = client.DeleteRoute(ctx, "chat-v1") // idempotent on 404

// API keys (secret is returned exactly once)
key, err := client.CreateAPIKey(ctx, flintgate.APIKeyCreate{
    ClientID: "billing-svc",
    Scopes:   []string{"read:invoices"},
})
log.Printf("persist immediately: %s", key.Secret)
```

| Method | Endpoint |
|---|---|
| `GetHealth` / `GetReady` | `GET /health`, `GET /ready` |
| `GetRoutes` / `GetRoute` | `GET /routes`, `GET /routes/{id}` |
| `CreateRoute` / `UpsertRoute` / `DeleteRoute` | `POST`, `PUT`, `DELETE /routes[/{id}]` |
| `ListAPIKeys` / `CreateAPIKey` / `DeleteAPIKey` | `GET`, `POST`, `DELETE /api-keys[/{id}]` |
| `CacheStats` / `InvalidateCache` | `GET /cache/stats`, `POST /cache/invalidate` |

Non-2xx responses surface as `*flintgate.APIError`. `IsNotFound(err)` reports
true for HTTP 404.

## Consuming SSE streams

`StreamSSE` returns a channel of spec-compliant events parsed with
`bufio.Scanner`. The channel is closed when the stream ends or the context
is cancelled.

```go
ch := flintgate.StreamSSE(ctx, nil, "https://gate.example.com/v1/chat/stream",
    userToken, flintgate.StreamOptions{})

for ev := range ch {
    if ev.IsError() {
        log.Printf("stream error: %s", ev.Data)
        break
    }
    fmt.Println(ev.Data)
}
```

- Multi-line `data:` fields are joined with `\n` per the SSE spec.
- `event:`, `id:`, and `retry:` fields are surfaced as `Event.Event`, `.ID`,
  and `.Retry` respectively.
- `Event.Pace()` returns the retry interval as a `time.Duration`.
- `MaxEventBytes` (default 1 MiB) caps a single reassembled event to bound
  memory use against pathological upstreams.

## WebSocket client

```go
ws, _ := flintgate.NewWSClient("wss://gate.example.com/v1/realtime",
    flintgate.WSOptions{Token: userToken})

err := ws.DialWithDefaults(ctx, func(ctx context.Context, conn *websocket.Conn) error {
    for {
        msg, err := flintgate.ReceiveJSON(ctx, conn)
        if err != nil { return err }
        handle(msg)
    }
})
```

`DialWithDefaults` runs a 30 s keepalive ping loop and closes the conn with
`StatusNormalClosure` on a clean handler return.

## Downstream-service middleware

For services deployed **behind** Flint Gate, wrap your handler to rehydrate
the identity Flint Gate injected as headers:

```go
mux := http.NewServeMux()
mux.HandleFunc("/api/invoices", handleInvoices)

h := flintgate.NewMiddleware(mux, flintgate.MiddlewareOptions{
    RequireFlintHeader: true, // reject direct hits in production
})

// Scope-gated subroute:
mux.Handle("/api/admin", flintgate.RequireScope(
    http.HandlerFunc(adminHandler), "admin"),
)

http.ListenAndServe(":8080", h)
```

Inside a handler:

```go
func handleInvoices(w http.ResponseWriter, r *http.Request) {
    id := flintgate.IdentityFromContext(r.Context())
    rid := flintgate.RequestIDFromContext(r.Context())
    log.Printf("[%s] subject=%s provider=%s scopes=%v",
        rid, id.Subject, id.Provider, id.Scopes)
    // ...
}
```

| Header | Field |
|---|---|
| `X-Flint-Identity-Provider` | `Identity.Provider` (`kratos`, `jwt`, `api_key`, `anonymous`) |
| `X-Flint-Identity-Subject` | `Identity.Subject` |
| `X-Flint-Identity-Scopes` | `Identity.Scopes` (space- or comma-delimited) |
| `X-Flint-Identity-Client-Id` | `Identity.ClientID` |
| `X-Flint-Identity-Session-Id` | `Identity.SessionID` |
| `X-Request-Id` | echoed on the response and attached to ctx |

## Testing

```bash
go test ./...
```

Coverage focuses on SSE parsing semantics (CR/LF/CRLF, comments, byte caps),
admin-client error mapping, and middleware identity rehydration.
