// Package flintgate is a client library for Go services that sit behind
// (or administer) the Flint Gate AI auth proxy.
//
// Flint Gate exposes two HTTP servers:
//   - Proxy  :4456 — public traffic
//   - Admin  :4457 — route/key CRUD, health, cache (never expose to internet)
//
// This package provides an Admin API client, an SSE stream reader, a minimal
// WebSocket client, and an http.Handler middleware for downstream services.
package flintgate

// DefaultAdminBaseURL is the default Flint Gate admin server URL.
const DefaultAdminBaseURL = "http://127.0.0.1:4457"

// DefaultProxyBaseURL is the default Flint Gate proxy server URL.
const DefaultProxyBaseURL = "http://127.0.0.1:4456"
