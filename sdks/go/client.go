package flintgate

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/rand"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// TokenSource is implemented by anything that can provide a bearer token.
// Implementations must be safe for concurrent use.
type TokenSource interface {
	GetToken(ctx context.Context) (string, error)
}

// StaticTokenSource is a TokenSource that always returns the same token.
type StaticTokenSource struct {
	Token string
}

// GetToken returns the static token, ignoring ctx.
func (s StaticTokenSource) GetToken(_ context.Context) (string, error) {
	return s.Token, nil
}

// Client is the Flint Gate admin API client. It is safe for concurrent use
// by multiple goroutines.
type Client struct {
	baseURL     *url.URL
	httpClient  *http.Client
	tokenSource TokenSource // nil means no auth
}

// Options configure a new Client.
type Options struct {
	// BaseURL is the admin server root (default: http://127.0.0.1:4457).
	BaseURL string
	// AdminToken is sent as Authorization: Bearer <token> when non-empty.
	// Backwards-compatible shorthand for TokenSource: StaticTokenSource{Token: AdminToken}.
	AdminToken string
	// TokenSource overrides AdminToken when non-nil. Use for dynamic token refresh.
	TokenSource TokenSource
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

	var ts TokenSource
	switch {
	case opts.TokenSource != nil:
		ts = opts.TokenSource
	case opts.AdminToken != "":
		ts = StaticTokenSource{Token: opts.AdminToken}
	}

	return &Client{
		baseURL:     u,
		httpClient:  hc,
		tokenSource: ts,
	}, nil
}

// BaseURL returns the configured admin base URL.
func (c *Client) BaseURL() string { return c.baseURL.String() }

// HTTPClient returns the underlying HTTP client.
func (c *Client) HTTPClient() *http.Client { return c.httpClient }

// TokenSource returns the configured TokenSource, or nil if none.
func (c *Client) TokenSource() TokenSource { return c.tokenSource }

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

// IsRateLimited reports whether err is an APIError with status 429.
func IsRateLimited(err error) bool {
	var ae *APIError
	return errors.As(err, &ae) && ae.StatusCode == http.StatusTooManyRequests
}

// IsUnauthorized reports whether err is an APIError with status 401.
func IsUnauthorized(err error) bool {
	var ae *APIError
	return errors.As(err, &ae) && ae.StatusCode == http.StatusUnauthorized
}

// IsApprovalRequired reports whether err is an APIError with status 403.
func IsApprovalRequired(err error) bool {
	var ae *APIError
	return errors.As(err, &ae) && ae.StatusCode == http.StatusForbidden
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
	if c.tokenSource != nil {
		tok, err := c.tokenSource.GetToken(ctx)
		if err != nil {
			return nil, fmt.Errorf("flintgate: get token: %w", err)
		}
		if tok != "" {
			req.Header.Set("Authorization", "Bearer "+tok)
		}
	}
	return req, nil
}

const (
	retryMax     = 3
	retryInitial = 500 * time.Millisecond
	retryFactor  = 2
	retryJitter  = 0.2 // ±20%
)

func (c *Client) doJSON(ctx context.Context, method, path string, in, out any) error {
	var lastErr error
	delay := retryInitial

	for attempt := 0; attempt <= retryMax; attempt++ {
		if attempt > 0 {
			// Apply jitter: delay * [1-jitter, 1+jitter]
			jitter := 1 + retryJitter*(2*rand.Float64()-1)
			sleep := time.Duration(float64(delay) * jitter)
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(sleep):
			}
			delay *= retryFactor
		}

		req, err := c.newRequest(ctx, method, path, in)
		if err != nil {
			return err
		}
		resp, err := c.httpClient.Do(req)
		if err != nil {
			return fmt.Errorf("flintgate: http: %w", err)
		}

		if resp.StatusCode == http.StatusTooManyRequests {
			body, _ := io.ReadAll(io.LimitReader(resp.Body, 4<<10))
			resp.Body.Close()
			lastErr = &APIError{StatusCode: resp.StatusCode, Body: strings.TrimSpace(string(body))}
			continue
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

	return lastErr
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

// ---------------------------------------------------------------------------
// Policies — Cedar authorization policy management
// ---------------------------------------------------------------------------

// ListPolicies calls GET /policies and returns all Cedar authorization policies.
// NOTE: unwraps the {"policies": [...]} envelope the server returns.
func (c *Client) ListPolicies(ctx context.Context) ([]PolicyRow, error) {
	var body struct {
		Policies []PolicyRow `json:"policies"`
	}
	if err := c.doJSON(ctx, http.MethodGet, "/policies", nil, &body); err != nil {
		return nil, err
	}
	return body.Policies, nil
}

// GetPolicy calls GET /policies/{id}. Returns an error wrapping 404 when not found.
func (c *Client) GetPolicy(ctx context.Context, id string) (PolicyRow, error) {
	var out PolicyRow
	if err := c.doJSON(ctx, http.MethodGet, "/policies/"+url.PathEscape(id), nil, &out); err != nil {
		return out, err
	}
	return out, nil
}

// CreatePolicy calls POST /policies. The ID field in input is required.
func (c *Client) CreatePolicy(ctx context.Context, input UpsertPolicyInput) (UpsertPolicyResponse, error) {
	var out UpsertPolicyResponse
	if err := c.doJSON(ctx, http.MethodPost, "/policies", input, &out); err != nil {
		return out, err
	}
	return out, nil
}

// UpdatePolicy calls PUT /policies/{id}.
func (c *Client) UpdatePolicy(ctx context.Context, id string, input UpsertPolicyInput) (UpsertPolicyResponse, error) {
	var out UpsertPolicyResponse
	if err := c.doJSON(ctx, http.MethodPut, "/policies/"+url.PathEscape(id), input, &out); err != nil {
		return out, err
	}
	return out, nil
}

// DeletePolicy calls DELETE /policies/{id}. Returns a 404 error if the policy
// does not exist (policy delete is not idempotent unlike routes/keys).
func (c *Client) DeletePolicy(ctx context.Context, id string) (DeletePolicyResponse, error) {
	var out DeletePolicyResponse
	if err := c.doJSON(ctx, http.MethodDelete, "/policies/"+url.PathEscape(id), nil, &out); err != nil {
		return out, err
	}
	return out, nil
}

// GetPolicyHistory calls GET /policies/{id}/history with optional pagination.
// Pass offset=0, limit=0 to use server defaults (limit 20).
func (c *Client) GetPolicyHistory(ctx context.Context, id string, offset, limit int) (PolicyHistoryResponse, error) {
	path := "/policies/" + url.PathEscape(id) + "/history"
	if offset > 0 || limit > 0 {
		q := url.Values{}
		if offset > 0 {
			q.Set("offset", fmt.Sprintf("%d", offset))
		}
		if limit > 0 {
			q.Set("limit", fmt.Sprintf("%d", limit))
		}
		path += "?" + q.Encode()
	}
	var out PolicyHistoryResponse
	if err := c.doJSON(ctx, http.MethodGet, path, nil, &out); err != nil {
		return out, err
	}
	return out, nil
}

// RollbackPolicy calls POST /policies/{id}/rollback with the target version number.
func (c *Client) RollbackPolicy(ctx context.Context, id string, versionNum int) (RollbackPolicyResponse, error) {
	body := map[string]int{"version_num": versionNum}
	var out RollbackPolicyResponse
	if err := c.doJSON(ctx, http.MethodPost, "/policies/"+url.PathEscape(id)+"/rollback", body, &out); err != nil {
		return out, err
	}
	return out, nil
}

// ---------------------------------------------------------------------------
// Approvals — human-in-the-loop tool-call approval flow
// ---------------------------------------------------------------------------

// ListApprovals calls GET /approvals and returns all non-expired pending
// approvals on this replica.
func (c *Client) ListApprovals(ctx context.Context) ([]ApprovalStatus, error) {
	var body struct {
		Approvals []ApprovalStatus `json:"approvals"`
	}
	if err := c.doJSON(ctx, http.MethodGet, "/approvals", nil, &body); err != nil {
		return nil, err
	}
	return body.Approvals, nil
}

// GetApproval calls GET /approvals/{id}. Returns an error wrapping 404 when
// the approval is not found or has already been resolved.
func (c *Client) GetApproval(ctx context.Context, id string) (ApprovalStatus, error) {
	var out ApprovalStatus
	if err := c.doJSON(ctx, http.MethodGet, "/approvals/"+url.PathEscape(id), nil, &out); err != nil {
		return out, err
	}
	return out, nil
}

// DecideApproval calls POST /approvals/{id}/decision with the given decision.
// Returns an error wrapping 404 (not found / already resolved) or 410 (expired).
func (c *Client) DecideApproval(ctx context.Context, id string, decision ApprovalDecision) error {
	body := map[string]ApprovalDecision{"decision": decision}
	return c.doJSON(ctx, http.MethodPost, "/approvals/"+url.PathEscape(id)+"/decision", body, nil)
}
