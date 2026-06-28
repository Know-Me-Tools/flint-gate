package flintgate

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// Client is the Flint Gate admin API client. It is safe for concurrent use
// by multiple goroutines.
type Client struct {
	baseURL    *url.URL
	httpClient *http.Client
	adminToken string // optional bearer token for the admin API
}

// Options configure a new Client.
type Options struct {
	// BaseURL is the admin server root (default: http://127.0.0.1:4457).
	BaseURL string
	// AdminToken is sent as Authorization: Bearer <token> when non-empty.
	AdminToken string
	// HTTPClient overrides the default *http.Client. If nil, a client with
	// a 30s timeout is used.
	HTTPClient *http.Client
}

// NewClient constructs a Client. Returns an error if BaseURL is invalid.
func NewClient(opts Options) (*Client, error) {
	raw := strings.TrimRight(strings.TrimSpace(opts.BaseURL), "/")
	if raw == "" {
		raw = DefaultAdminBaseURL
	}
	u, err := url.Parse(raw)
	if err != nil {
		return nil, fmt.Errorf("flintgate: invalid base url %q: %w", opts.BaseURL, err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return nil, fmt.Errorf("flintgate: base url must be http or https, got %q", u.Scheme)
	}
	hc := opts.HTTPClient
	if hc == nil {
		hc = &http.Client{Timeout: 30 * time.Second}
	}
	return &Client{
		baseURL:    u,
		httpClient: hc,
		adminToken: opts.AdminToken,
	}, nil
}

// BaseURL returns the configured admin base URL.
func (c *Client) BaseURL() string { return c.baseURL.String() }

// HTTPClient returns the underlying HTTP client.
func (c *Client) HTTPClient() *http.Client { return c.httpClient }

// ---------------------------------------------------------------------------
// Request plumbing
// ---------------------------------------------------------------------------

// APIError is returned for non-2xx admin responses. Body is the trimmed
// response body (max 4 KiB).
type APIError struct {
	StatusCode int
	Body       string
}

func (e *APIError) Error() string {
	return fmt.Sprintf("flintgate: admin api error %d: %s", e.StatusCode, e.Body)
}

// IsNotFound reports whether err is an APIError with status 404.
func IsNotFound(err error) bool {
	var ae *APIError
	return errors.As(err, &ae) && ae.StatusCode == http.StatusNotFound
}

func (c *Client) newRequest(ctx context.Context, method, path string, body any) (*http.Request, error) {
	rel, err := url.Parse(strings.TrimLeft(path, "/"))
	if err != nil {
		return nil, err
	}
	u := c.baseURL.ResolveReference(rel)

	var r io.Reader
	if body != nil {
		buf, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("flintgate: marshal body: %w", err)
		}
		r = bytes.NewReader(buf)
	}

	req, err := http.NewRequestWithContext(ctx, method, u.String(), r)
	if err != nil {
		return nil, err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	req.Header.Set("Accept", "application/json")
	if c.adminToken != "" {
		req.Header.Set("Authorization", "Bearer "+c.adminToken)
	}
	return req, nil
}

func (c *Client) doJSON(ctx context.Context, method, path string, in, out any) error {
	req, err := c.newRequest(ctx, method, path, in)
	if err != nil {
		return err
	}
	resp, err := c.httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("flintgate: http: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
		return &APIError{StatusCode: resp.StatusCode, Body: strings.TrimSpace(string(body))}
	}
	if out == nil {
		return nil
	}
	if ct := resp.Header.Get("Content-Type"); ct != "" && !strings.Contains(ct, "application/json") {
		// Non-JSON success response (e.g. 204 No Content). Leave out untouched.
		return nil
	}
	dec := json.NewDecoder(resp.Body)
	dec.UseNumber()
	if err := dec.Decode(out); err != nil && !errors.Is(err, io.EOF) {
		return fmt.Errorf("flintgate: decode response: %w", err)
	}
	return nil
}

// ---------------------------------------------------------------------------
// Health & readiness
// ---------------------------------------------------------------------------

// GetHealth calls GET /health on the admin server.
func (c *Client) GetHealth(ctx context.Context) (HealthStatus, error) {
	var h HealthStatus
	if err := c.doJSON(ctx, http.MethodGet, "/health", nil, &h); err != nil {
		return h, err
	}
	return h, nil
}

// GetReady calls GET /ready on the admin server.
func (c *Client) GetReady(ctx context.Context) (ReadyStatus, error) {
	var r ReadyStatus
	if err := c.doJSON(ctx, http.MethodGet, "/ready", nil, &r); err != nil {
		return r, err
	}
	return r, nil
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

// GetRoutes calls GET /routes and returns all configured routes.
func (c *Client) GetRoutes(ctx context.Context) ([]RouteConfig, error) {
	var routes []RouteConfig
	if err := c.doJSON(ctx, http.MethodGet, "/routes", nil, &routes); err != nil {
		return nil, err
	}
	return routes, nil
}

// GetRoute calls GET /routes/{id}.
func (c *Client) GetRoute(ctx context.Context, id string) (RouteConfig, error) {
	var r RouteConfig
	if err := c.doJSON(ctx, http.MethodGet, "/routes/"+url.PathEscape(id), nil, &r); err != nil {
		return r, err
	}
	return r, nil
}

// CreateRoute calls POST /routes. The returned RouteConfig reflects the
// server-normalized record (including the assigned id for new routes).
func (c *Client) CreateRoute(ctx context.Context, r RouteConfig) (RouteConfig, error) {
	if r.ID == "" {
		return RouteConfig{}, errors.New("flintgate: CreateRoute: route id is required")
	}
	var out RouteConfig
	if err := c.doJSON(ctx, http.MethodPost, "/routes", r, &out); err != nil {
		return out, err
	}
	return out, nil
}

// UpsertRoute calls PUT /routes/{id}.
func (c *Client) UpsertRoute(ctx context.Context, r RouteConfig) (RouteConfig, error) {
	if r.ID == "" {
		return RouteConfig{}, errors.New("flintgate: UpsertRoute: route id is required")
	}
	var out RouteConfig
	if err := c.doJSON(ctx, http.MethodPut, "/routes/"+url.PathEscape(r.ID), r, &out); err != nil {
		return out, err
	}
	return out, nil
}

// DeleteRoute calls DELETE /routes/{id}. It is idempotent; a 404 is not
// surfaced as an error.
func (c *Client) DeleteRoute(ctx context.Context, id string) error {
	err := c.doJSON(ctx, http.MethodDelete, "/routes/"+url.PathEscape(id), nil, nil)
	if IsNotFound(err) {
		return nil
	}
	return err
}

// ---------------------------------------------------------------------------
// API keys
// ---------------------------------------------------------------------------

// ListAPIKeys calls GET /api-keys.
func (c *Client) ListAPIKeys(ctx context.Context) ([]APIKey, error) {
	var keys []APIKey
	if err := c.doJSON(ctx, http.MethodGet, "/api-keys", nil, &keys); err != nil {
		return nil, err
	}
	return keys, nil
}

// CreateAPIKey calls POST /api-keys. The server returns the full key value
// exactly once; callers must persist api.Key immediately.
func (c *Client) CreateAPIKey(ctx context.Context, api APIKeyCreate) (APIKeyWithSecret, error) {
	var out APIKeyWithSecret
	if err := c.doJSON(ctx, http.MethodPost, "/api-keys", api, &out); err != nil {
		return out, err
	}
	return out, nil
}

// DeleteAPIKey calls DELETE /api-keys/{id}.
func (c *Client) DeleteAPIKey(ctx context.Context, id string) error {
	err := c.doJSON(ctx, http.MethodDelete, "/api-keys/"+url.PathEscape(id), nil, nil)
	if IsNotFound(err) {
		return nil
	}
	return err
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// InvalidateCache calls POST /cache/invalidate with the given scope.
// scope may be empty (all), "route", "session", or "api_key".
func (c *Client) InvalidateCache(ctx context.Context, scope string) error {
	body := map[string]string{"scope": scope}
	return c.doJSON(ctx, http.MethodPost, "/cache/invalidate", body, nil)
}

// CacheStats calls GET /cache/stats.
func (c *Client) CacheStats(ctx context.Context) (CacheStats, error) {
	var s CacheStats
	if err := c.doJSON(ctx, http.MethodGet, "/cache/stats", nil, &s); err != nil {
		return s, err
	}
	return s, nil
}
