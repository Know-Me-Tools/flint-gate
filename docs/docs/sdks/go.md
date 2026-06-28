# Go SDK

The Go SDK is at `github.com/know-me-tools/flint-gate/sdks/go`. It provides an admin client, SSE stream consumer, WebSocket client, and downstream service middleware for rehydrating identity headers.

## Install

```bash
go get github.com/know-me-tools/flint-gate/sdks/go
```

The WebSocket client depends on [`nhooyr.io/websocket`](https://pkg.go.dev/nhooyr.io/websocket), declared in `go.mod`.

## Admin client

```go
import "github.com/know-me-tools/flint-gate/sdks/go"

client, err := flintgate.NewClient(flintgate.Options{
    BaseURL:    "http://127.0.0.1:4457",
    AdminToken: os.Getenv("FLINT_ADMIN_TOKEN"),
})
if err != nil { log.Fatal(err) }

// Health / readiness
h, err := client.GetHealth(ctx)
r, err := client.GetReady(ctx)

// Routes
routes, err := client.GetRoutes(ctx)
created, err := client.CreateRoute(ctx, flintgate.RouteConfig{
    ID:      "chat-v1",
    Site:    "default",
    Match:   flintgate.RouteMatch{Path: "/api/chat/**", Methods: []string{"POST"}},
    Stream:  flintgate.StreamCfg{Mode: "passthrough"},
    Enabled: true,
})
err = client.DeleteRoute(ctx, "chat-v1") // idempotent on 404

// API keys — the secret is returned exactly once
key, err := client.CreateAPIKey(ctx, flintgate.APIKeyCreate{
    ClientID: "billing-svc",
    Scopes:   []string{"read:invoices"},
})
log.Printf("persist immediately: %s", key.Secret)
```

## SSE streams

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

The SSE parser joins multi-line `data:` fields with `\n` per the SSE spec and exposes `event:`, `id:`, and `retry:` fields.

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

`DialWithDefaults` runs a 30-second keepalive ping loop.

## Downstream middleware

For services deployed behind Flint Gate, wrap handlers to rehydrate injected identity headers:

```go
mux := http.NewServeMux()
mx.HandleFunc("/api/invoices", handleInvoices)

h := flintgate.NewMiddleware(mx, flintgate.MiddlewareOptions{
    RequireFlintHeader: true,
})

http.ListenAndServe(":8080", h)
```

Inside a handler:

```go
func handleInvoices(w http.ResponseWriter, r *http.Request) {
    id := flintgate.IdentityFromContext(r.Context())
    rid := flintgate.RequestIDFromContext(r.Context())
    log.Printf("[%s] subject=%s provider=%s", rid, id.Subject, id.Provider)
}
```

## Testing

```bash
go test ./...
```
