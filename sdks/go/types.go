package flintgate

import "time"

// HealthStatus is the body returned by GET /health on the admin server.
type HealthStatus struct {
	Status     string    `json:"status"`               // "ok" | "degraded" | "down"
	Version    string    `json:"version,omitempty"`    // flint-gate semver
	Commit     string    `json:"commit,omitempty"`     // build commit sha
	UptimeSec  float64   `json:"uptime_seconds"`       // process uptime
	CheckedAt  time.Time `json:"checked_at"`           // server time of check
	Components map[string]ComponentHealth `json:"components,omitempty"`
}

// ComponentHealth is the status of an internal subsystem (db, cache, etc).
type ComponentHealth struct {
	Status  string        `json:"status"`            // "ok" | "degraded" | "down"
	Latency time.Duration `json:"latency,omitempty"` // last probe latency
	Error   string        `json:"error,omitempty"`   // populated when status != "ok"
}

// ReadyStatus is the body returned by GET /ready.
type ReadyStatus struct {
	Ready    bool   `json:"ready"`
	Reason   string `json:"reason,omitempty"` // populated when not ready
	Revision string `json:"revision,omitempty"`
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

// RouteMatch describes how a route is matched against an inbound request.
// The `match` field is a reserved Go keyword so it carries a JSON tag.
type RouteMatch struct {
	Host    string   `json:"host,omitempty"`    // glob, e.g. "*.example.com"
	Path    string   `json:"path"`              // glob, e.g. "/api/v1/**"
	Methods []string `json:"methods,omitempty"` // empty/nil means all methods
}

// RouteConfig is the Flint Gate route definition (admin API shape).
type RouteConfig struct {
	ID       string     `json:"id"`                 // stable identifier
	Site     string     `json:"site"`               // owning site id
	Match    RouteMatch `json:"match"`              // match criteria
	Upstream string     `json:"upstream,omitempty"` // full upstream URL (overrides site)
	Auth     string     `json:"auth,omitempty"`     // auth provider id (overrides site)
	Hooks    Hooks      `json:"hooks,omitempty"`
	Stream   StreamCfg  `json:"stream,omitempty"`
	Priority int        `json:"priority,omitempty"`
	Enabled  bool       `json:"enabled"`
}

// Hooks mirrors the Flint Gate pre-request hook configuration.
type Hooks struct {
	PreRequest []PreRequestHook `json:"pre_request,omitempty"`
}

// PreRequestHook is a single named hook with a typed payload.
type PreRequestHook struct {
	Type string                 `json:"type"` // "claims_enhancement" | "body_transform"
	Name string                 `json:"name,omitempty"`
	With map[string]interface{} `json:"with,omitempty"`
}

// StreamCfg controls SSE/streaming behavior for a route.
type StreamCfg struct {
	Mode              string `json:"mode,omitempty"` // "passthrough" | "agui" | "a2ui" | "off"
	MaxConcurrent     int    `json:"max_concurrent,omitempty"`
	BufferBytes       int    `json:"buffer_bytes,omitempty"`
	CountTokens       bool   `json:"count_tokens,omitempty"`
	TerminateOnExpire bool   `json:"terminate_on_expire,omitempty"`
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

// APIKey is the persisted shape returned by GET /api-keys (no secret).
type APIKey struct {
	ID        string     `json:"id"`
	ClientID  string     `json:"client_id"`
	Scopes    []string   `json:"scopes,omitempty"`
	ExpiresAt *time.Time `json:"expires_at,omitempty"`
	CreatedAt time.Time  `json:"created_at"`
}

// APIKeyCreate is the body of POST /api-keys.
type APIKeyCreate struct {
	ClientID  string     `json:"client_id"`
	Scopes    []string   `json:"scopes,omitempty"`
	ExpiresAt *time.Time `json:"expires_at,omitempty"`
}

// APIKeyWithSecret is returned exactly once by POST /api-keys. The Secret
// field is the only time the raw key is visible to the caller.
type APIKeyWithSecret struct {
	APIKey
	Secret string `json:"secret"` // raw key — persist immediately, never log
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// CacheStats is returned by GET /cache/stats.
type CacheStats struct {
	RouteEntries  int `json:"route_entries"`
	SessionEntries int `json:"session_entries"`
	ApiKeyEntries int `json:"api_key_entries"`
	HitRate       float64 `json:"hit_rate"`
}

// ---------------------------------------------------------------------------
// Identity — populated by Flint Gate on inbound requests and forwarded to
// downstream services as headers (see middleware.IdentityFromHeaders).
// ---------------------------------------------------------------------------

// Identity is the authenticated subject resolved by Flint Gate.
type Identity struct {
	Subject   string   `json:"subject"`
	Provider  string   `json:"provider"`            // "kratos" | "jwt" | "api_key" | "anonymous"
	Scopes    []string `json:"scopes,omitempty"`
	ClientID  string   `json:"client_id,omitempty"` // api_key provider
	SessionID string   `json:"session_id,omitempty"` // kratos session
	Issuer    string   `json:"issuer,omitempty"`     // jwt issuer
	Audience  []string `json:"audience,omitempty"`   // jwt audiences
}
