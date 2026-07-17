//go:build integration

package flintgate_test

import (
	"bufio"
	"bytes"
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	flintgate "github.com/know-me-tools/flint-gate/sdks/go"
)

// integrationClient returns a Client pointed at the running test fixture.
// INTEGRATION_GATEWAY_URL defaults to http://127.0.0.1:4457.
// The admin port is bound to loopback in config.test.yaml (AllowLoopback
// posture) so no auth token is required.
func integrationClient(t *testing.T) *flintgate.Client {
	t.Helper()
	baseURL := os.Getenv("INTEGRATION_GATEWAY_URL")
	if baseURL == "" {
		baseURL = "http://127.0.0.1:4457"
	}
	c, err := flintgate.NewClient(flintgate.Options{BaseURL: baseURL})
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}
	return c
}

// integrationProxyURL returns the agent-facing proxy base URL.
// INTEGRATION_PROXY_URL defaults to http://127.0.0.1:4456.
func integrationProxyURL() string {
	if u := os.Getenv("INTEGRATION_PROXY_URL"); u != "" {
		return u
	}
	return "http://127.0.0.1:4456"
}

// testJWT returns a minimal HS256 JWT valid for the integration test fixture.
// Uses only stdlib: crypto/hmac, crypto/sha256, encoding/base64, encoding/json.
// The signing key and issuer match config.test.yaml / docker-compose.test.yml.
func testJWT(t *testing.T) string {
	t.Helper()
	b64 := func(v any) string {
		data, _ := json.Marshal(v)
		return base64.RawURLEncoding.EncodeToString(data)
	}
	header := b64(map[string]string{"alg": "HS256", "typ": "JWT"})
	now := time.Now().Unix()
	payload := b64(map[string]any{
		"iss": "http://flint-gate:4456",
		"sub": "integ-test-user",
		"iat": now,
		"exp": now + 300,
	})
	sigInput := header + "." + payload
	mac := hmac.New(sha256.New, []byte("test-jwt-secret"))
	mac.Write([]byte(sigInput))
	sig := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	return sigInput + "." + sig
}

// pollForApproval polls ListApprovals until at least one pending approval exists,
// returning the first approval ID found. Fails the test if timeout elapses.
func pollForApproval(ctx context.Context, t *testing.T, c *flintgate.Client, timeout time.Duration) string {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		approvals, err := c.ListApprovals(ctx)
		if err != nil {
			t.Logf("pollForApproval: ListApprovals error (retrying): %v", err)
		} else if len(approvals) > 0 {
			return approvals[0].ApprovalID
		}
		time.Sleep(500 * time.Millisecond)
	}
	t.Fatalf("pollForApproval: no approval appeared within %s", timeout)
	return ""
}

// uniqueID generates a test-run-scoped ID prefix to avoid collisions between
// parallel or consecutive runs.
func uniqueID(prefix string) string {
	return fmt.Sprintf("%s-%d", prefix, time.Now().UnixMilli())
}

// ---------------------------------------------------------------------------
// Health / readiness
// ---------------------------------------------------------------------------

func TestIntegration_Health(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	h, err := c.GetHealth(ctx)
	if err != nil {
		t.Fatalf("GetHealth: %v", err)
	}
	if h.Status != "ok" {
		t.Errorf("health.Status = %q, want %q", h.Status, "ok")
	}
}

func TestIntegration_Ready(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	r, err := c.GetReady(ctx)
	if err != nil {
		t.Fatalf("GetReady: %v", err)
	}
	if !r.Ready {
		t.Errorf("ready.Ready = false, reason: %s", r.Reason)
	}
}

// ---------------------------------------------------------------------------
// Routes CRUD
// ---------------------------------------------------------------------------

func TestIntegration_Routes(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	routeID := uniqueID("integ-route")
	route := flintgate.RouteConfig{
		ID:       routeID,
		Site:     "test",
		Match:    flintgate.RouteMatch{Path: "/integ-test/**", Methods: []string{"GET"}},
		Upstream: "http://127.0.0.1:4457/health",
		Enabled:  true,
	}

	// Create
	created, err := c.CreateRoute(ctx, route)
	if err != nil {
		t.Fatalf("CreateRoute: %v", err)
	}
	t.Cleanup(func() {
		cleanCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = c.DeleteRoute(cleanCtx, routeID)
	})
	if created.ID != routeID {
		t.Errorf("created.ID = %q, want %q", created.ID, routeID)
	}

	// List — created route must appear
	routes, err := c.GetRoutes(ctx)
	if err != nil {
		t.Fatalf("GetRoutes: %v", err)
	}
	var found bool
	for _, r := range routes {
		if r.ID == routeID {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("created route %q not found in GetRoutes result (%d routes)", routeID, len(routes))
	}

	// Get by ID
	got, err := c.GetRoute(ctx, routeID)
	if err != nil {
		t.Fatalf("GetRoute(%q): %v", routeID, err)
	}
	if got.Match.Path != "/integ-test/**" {
		t.Errorf("got.Match.Path = %q, want /integ-test/**", got.Match.Path)
	}

	// Delete
	if err := c.DeleteRoute(ctx, routeID); err != nil {
		t.Fatalf("DeleteRoute: %v", err)
	}

	// Delete idempotent — second call on a missing route must not error
	if err := c.DeleteRoute(ctx, routeID); err != nil {
		t.Errorf("DeleteRoute (idempotent) returned error: %v", err)
	}
}

// ---------------------------------------------------------------------------
// Route update lifecycle
// ---------------------------------------------------------------------------

func TestIntegration_RouteUpdate(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	routeID := uniqueID("integ-route-upd")
	route := flintgate.RouteConfig{
		ID:       routeID,
		Site:     "test",
		Match:    flintgate.RouteMatch{Path: "/integ-upd/**", Methods: []string{"GET"}},
		Upstream: "http://127.0.0.1:4457/health",
		Enabled:  true,
	}

	// Create
	if _, err := c.CreateRoute(ctx, route); err != nil {
		t.Fatalf("CreateRoute: %v", err)
	}
	t.Cleanup(func() {
		cleanCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = c.DeleteRoute(cleanCtx, routeID)
	})

	// Update — change the path
	updated := route
	updated.Match.Path = "/integ-upd-v2/**"
	got, err := c.UpsertRoute(ctx, updated)
	if err != nil {
		t.Fatalf("UpsertRoute: %v", err)
	}
	if got.Match.Path != "/integ-upd-v2/**" {
		t.Errorf("after update: Match.Path = %q, want /integ-upd-v2/**", got.Match.Path)
	}

	// Verify via GetRoute
	fetched, err := c.GetRoute(ctx, routeID)
	if err != nil {
		t.Fatalf("GetRoute after update: %v", err)
	}
	if fetched.Match.Path != "/integ-upd-v2/**" {
		t.Errorf("GetRoute after update: Match.Path = %q, want /integ-upd-v2/**", fetched.Match.Path)
	}
}

// ---------------------------------------------------------------------------
// Policy CRUD + history + rollback
// ---------------------------------------------------------------------------

func TestIntegration_PolicyCRUD(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	policyID := uniqueID("integ-policy")
	input := flintgate.UpsertPolicyInput{
		ID:         policyID,
		PolicyText: "permit(principal, action, resource);",
		Enabled:    true,
	}

	// Create (v1)
	created, err := c.CreatePolicy(ctx, input)
	if err != nil {
		t.Fatalf("CreatePolicy: %v", err)
	}
	if created.ID != policyID {
		t.Errorf("created.ID = %q, want %q", created.ID, policyID)
	}
	// Policy delete is NOT idempotent — only clean up if create succeeded
	t.Cleanup(func() {
		cleanCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = c.DeletePolicy(cleanCtx, policyID)
	})

	// List — must appear
	policies, err := c.ListPolicies(ctx)
	if err != nil {
		t.Fatalf("ListPolicies: %v", err)
	}
	var found bool
	for _, p := range policies {
		if p.ID == policyID {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("created policy %q not found in ListPolicies (%d policies)", policyID, len(policies))
	}

	// Get by ID
	got, err := c.GetPolicy(ctx, policyID)
	if err != nil {
		t.Fatalf("GetPolicy: %v", err)
	}
	if got.PolicyText != input.PolicyText {
		t.Errorf("GetPolicy.PolicyText = %q, want %q", got.PolicyText, input.PolicyText)
	}

	// Update (v2)
	v2Text := "forbid(principal, action, resource);"
	updInput := flintgate.UpsertPolicyInput{
		PolicyText: v2Text,
		Enabled:    true,
	}
	_, err = c.UpdatePolicy(ctx, policyID, updInput)
	if err != nil {
		t.Fatalf("UpdatePolicy: %v", err)
	}

	// History — should have at least 2 versions
	hist, err := c.GetPolicyHistory(ctx, policyID, 0, 0)
	if err != nil {
		t.Fatalf("GetPolicyHistory: %v", err)
	}
	if len(hist.Versions) < 2 {
		t.Errorf("expected ≥2 history versions after update, got %d", len(hist.Versions))
	}

	// Rollback to version 1
	var v1Num int
	for _, v := range hist.Versions {
		if v.PolicyText == input.PolicyText {
			v1Num = v.VersionNum
			break
		}
	}
	if v1Num == 0 {
		t.Fatal("could not find v1 version in history to roll back to")
	}
	rb, err := c.RollbackPolicy(ctx, policyID, v1Num)
	if err != nil {
		t.Fatalf("RollbackPolicy: %v", err)
	}
	if rb.PolicyID != policyID {
		t.Errorf("rollback.PolicyID = %q, want %q", rb.PolicyID, policyID)
	}

	// History should now have ≥3 entries (v1, v2, rollback-as-v3)
	hist2, err := c.GetPolicyHistory(ctx, policyID, 0, 0)
	if err != nil {
		t.Fatalf("GetPolicyHistory after rollback: %v", err)
	}
	if len(hist2.Versions) < 3 {
		t.Errorf("expected ≥3 history versions after rollback, got %d", len(hist2.Versions))
	}

	// Delete
	del, err := c.DeletePolicy(ctx, policyID)
	if err != nil {
		t.Fatalf("DeletePolicy: %v", err)
	}
	if del.Status != "deleted" {
		t.Errorf("delete.Status = %q, want deleted", del.Status)
	}

	// GetPolicy after delete must return 404
	_, err = c.GetPolicy(ctx, policyID)
	if !flintgate.IsNotFound(err) {
		t.Errorf("GetPolicy after delete: expected 404, got %v", err)
	}
}

// ---------------------------------------------------------------------------
// Approval smoke test
// ---------------------------------------------------------------------------

// TestIntegration_ApprovalExpiry verifies the TTL janitor path:
//
//  1. Create a Cedar @require_approval policy.
//  2. Send a streaming request → the buffered tool call creates an approval.
//  3. Wait longer than the approval TTL (config.test.yaml: ttl_seconds = 5).
//  4. Assert ListApprovals returns empty — janitor swept the expired entry.
//  5. Assert the streaming response body is closed (stream terminated).
//
// This test requires the test fixture to be started with config.test.yaml
// (ttl_seconds: 5, janitor_interval_seconds: 1) so TTL sweeps happen quickly.
func TestIntegration_ApprovalExpiry(t *testing.T) {
	ctx := context.Background()
	c := integrationClient(t)
	proxyBase := integrationProxyURL()

	// ── 1. Seed the @require_approval Cedar policy ────────────────────────────
	policyID := uniqueID("integ-expiry-policy")
	policyText := `@require_approval("human review required — expiry test")
permit(principal, action, resource == Route::"integ_test_tool");`

	created, err := c.CreatePolicy(ctx, flintgate.UpsertPolicyInput{
		ID:         policyID,
		PolicyText: policyText,
		Enabled:    true,
	})
	if err != nil {
		t.Fatalf("CreatePolicy: %v", err)
	}
	t.Cleanup(func() {
		cleanCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if _, err := c.DeletePolicy(cleanCtx, created.ID); err != nil {
			t.Logf("cleanup DeletePolicy %q: %v", created.ID, err)
		}
	})

	// ── 2. Send streaming request — approval is created by the gateway ────────
	reqCtx, reqCancel := context.WithTimeout(ctx, 20*time.Second)
	defer reqCancel()

	req, err := http.NewRequestWithContext(
		reqCtx,
		http.MethodPost,
		proxyBase+"/stream-test",
		bytes.NewBufferString("{}"),
	)
	if err != nil {
		t.Fatalf("http.NewRequest: %v", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/x-ndjson")
	req.Header.Set("Authorization", "Bearer "+testJWT(t))

	// Start reading the response body in the background so we can detect
	// when the stream is terminated by the janitor.
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("proxy POST /stream-test: %v", err)
	}

	bodyDone := make(chan []string, 1)
	go func() {
		var lines []string
		scanner := bufio.NewScanner(resp.Body)
		for scanner.Scan() {
			line := strings.TrimSpace(scanner.Text())
			if line != "" {
				lines = append(lines, line)
			}
		}
		resp.Body.Close()
		bodyDone <- lines
	}()

	// ── 3. Confirm the approval appeared before waiting for expiry ────────────
	_ = pollForApproval(ctx, t, c, 10*time.Second)
	t.Log("approval appeared — waiting for TTL expiry (config.test.yaml: ttl_seconds=5)")

	// ── 4. Wait longer than the TTL + janitor interval (5s TTL + 1s sweep) ────
	time.Sleep(8 * time.Second)

	listCtx, listCancel := context.WithTimeout(ctx, 5*time.Second)
	defer listCancel()
	remaining, err := c.ListApprovals(listCtx)
	if err != nil {
		t.Fatalf("ListApprovals after TTL: %v", err)
	}
	if len(remaining) != 0 {
		t.Errorf("expected 0 pending approvals after TTL expiry, got %d", len(remaining))
	}

	// ── 5. Assert the stream terminated after the approval was auto-denied ─────
	select {
	case lines := <-bodyDone:
		t.Logf("stream closed with %d lines (expected — approval auto-denied by janitor)", len(lines))
	case <-time.After(5 * time.Second):
		t.Error("stream did not close within 5s after approval TTL expiry")
	}
}

// TestIntegration_ApprovalSmoke verifies the approvals admin surface is
// reachable. The test fixture does not inject in-flight requests, so we
// cannot create a real approval — we only assert that ListApprovals returns
// without error (empty slice is valid in a fresh fixture).
func TestIntegration_ApprovalSmoke(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	approvals, err := c.ListApprovals(ctx)
	if err != nil {
		t.Fatalf("ListApprovals: %v", err)
	}
	// Nil and empty slice are both valid — just ensure no panic / error.
	t.Logf("ListApprovals returned %d approval(s)", len(approvals))
}

// ---------------------------------------------------------------------------
// API Keys CRUD
// ---------------------------------------------------------------------------

func TestIntegration_APIKeys(t *testing.T) {
	c := integrationClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	clientID := uniqueID("integ-key")
	create := flintgate.APIKeyCreate{
		ClientID: clientID,
		Scopes:   []string{"read", "write"},
	}

	// Create — secret is only returned here
	withSecret, err := c.CreateAPIKey(ctx, create)
	if err != nil {
		t.Fatalf("CreateAPIKey: %v", err)
	}
	t.Cleanup(func() {
		cleanCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = c.DeleteAPIKey(cleanCtx, withSecret.ID)
	})
	if withSecret.Secret == "" {
		t.Error("CreateAPIKey: Secret is empty — expected plaintext key on first response")
	}
	if withSecret.ClientID != clientID {
		t.Errorf("ClientID = %q, want %q", withSecret.ClientID, clientID)
	}
	// Secret must look like a non-trivial token
	if len(strings.TrimSpace(withSecret.Secret)) < 8 {
		t.Errorf("Secret looks too short (%d chars), expected a real key", len(withSecret.Secret))
	}

	// List — key must appear
	keys, err := c.ListAPIKeys(ctx)
	if err != nil {
		t.Fatalf("ListAPIKeys: %v", err)
	}
	var found bool
	for _, k := range keys {
		if k.ID == withSecret.ID {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("created key %q not found in ListAPIKeys (%d keys)", withSecret.ID, len(keys))
	}

	// Delete
	if err := c.DeleteAPIKey(ctx, withSecret.ID); err != nil {
		t.Fatalf("DeleteAPIKey: %v", err)
	}

	// Delete idempotent
	if err := c.DeleteAPIKey(ctx, withSecret.ID); err != nil {
		t.Errorf("DeleteAPIKey (idempotent) returned error: %v", err)
	}
}

// ---------------------------------------------------------------------------
// Approval full-flow
// ---------------------------------------------------------------------------

// TestIntegration_ApprovalFlow exercises the complete human-in-the-loop cycle:
//
//  1. Create a Cedar @require_approval policy for "integ_test_tool" via Admin API
//  2. POST to the /stream-test streaming route (proxied to mock-upstream)
//  3. Poll ListApprovals until the buffered tool call appears
//  4. Approve → assert the ndjson stream delivers TOOL_CALL_START through
//  5. Repeat with Deny → assert stream closes without TOOL_CALL_START
func TestIntegration_ApprovalFlow(t *testing.T) {
	ctx := context.Background()
	c := integrationClient(t)
	proxyBase := integrationProxyURL()

	// ── 1. Seed the @require_approval Cedar policy ────────────────────────────
	policyID := uniqueID("integ-approval-policy")
	policyText := `@require_approval("human review required")
permit(principal, action, resource == Route::"integ_test_tool");`

	created, err := c.CreatePolicy(ctx, flintgate.UpsertPolicyInput{
		ID:         policyID,
		PolicyText: policyText,
		Enabled:    true,
	})
	if err != nil {
		t.Fatalf("CreatePolicy: %v", err)
	}
	t.Cleanup(func() {
		if _, err := c.DeletePolicy(context.Background(), created.ID); err != nil {
			t.Logf("cleanup DeletePolicy %q: %v", created.ID, err)
		}
	})
	t.Logf("created @require_approval policy: %s (reloaded=%v)", created.ID, created.Reloaded)

	// ── Helper: send one streaming request and return the response body reader ─
	sendStreamRequest := func(t *testing.T) (*http.Response, context.CancelFunc) {
		t.Helper()
		reqCtx, cancel := context.WithTimeout(ctx, 20*time.Second)
		req, err := http.NewRequestWithContext(
			reqCtx,
			http.MethodPost,
			proxyBase+"/stream-test",
			bytes.NewBufferString("{}"),
		)
		if err != nil {
			cancel()
			t.Fatalf("http.NewRequest: %v", err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "application/x-ndjson")
		req.Header.Set("Authorization", "Bearer "+testJWT(t))

		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			cancel()
			t.Fatalf("proxy POST /stream-test: %v", err)
		}
		return resp, cancel
	}

	// ── Helper: drain ndjson lines, return all "type" values seen ─────────────
	collectTypes := func(body io.ReadCloser) []string {
		defer body.Close()
		var types []string
		scanner := bufio.NewScanner(body)
		for scanner.Scan() {
			line := strings.TrimSpace(scanner.Text())
			if line == "" {
				continue
			}
			var ev map[string]any
			if json.Unmarshal([]byte(line), &ev) == nil {
				if typ, ok := ev["type"].(string); ok {
					types = append(types, typ)
				}
			}
		}
		return types
	}

	// ── 2a. Approve path ─────────────────────────────────────────────────────
	t.Run("approve", func(t *testing.T) {
		resp, cancel := sendStreamRequest(t)
		defer cancel()

		// Collect ndjson in background while we race the approval decision.
		typesCh := make(chan []string, 1)
		go func() {
			typesCh <- collectTypes(resp.Body)
		}()

		// Poll until the approval appears (gate buffered the tool call).
		approvalID := pollForApproval(ctx, t, c, 10*time.Second)
		t.Logf("approve path: found approval %s", approvalID)

		if err := c.DecideApproval(ctx, approvalID, flintgate.ApprovalDecisionApprove); err != nil {
			t.Fatalf("DecideApproval approve: %v", err)
		}

		// The stream should now complete and include TOOL_CALL_START.
		select {
		case types := <-typesCh:
			t.Logf("approve path: received event types: %v", types)
			found := false
			for _, tp := range types {
				if tp == "TOOL_CALL_START" {
					found = true
					break
				}
			}
			if !found {
				t.Errorf("approve path: TOOL_CALL_START not found in stream events %v", types)
			}
		case <-time.After(10 * time.Second):
			t.Error("approve path: stream did not complete within 10s after approval decision")
		}
	})

	// ── 2b. Deny path ────────────────────────────────────────────────────────
	t.Run("deny", func(t *testing.T) {
		resp, cancel := sendStreamRequest(t)
		defer cancel()

		typesCh := make(chan []string, 1)
		go func() {
			typesCh <- collectTypes(resp.Body)
		}()

		approvalID := pollForApproval(ctx, t, c, 10*time.Second)
		t.Logf("deny path: found approval %s", approvalID)

		if err := c.DecideApproval(ctx, approvalID, flintgate.ApprovalDecisionDeny); err != nil {
			t.Fatalf("DecideApproval deny: %v", err)
		}

		// After a deny, the stream should close without forwarding TOOL_CALL_START.
		select {
		case types := <-typesCh:
			t.Logf("deny path: received event types: %v", types)
			for _, tp := range types {
				if tp == "TOOL_CALL_START" {
					t.Errorf("deny path: TOOL_CALL_START should not be forwarded after deny, got types: %v", types)
				}
			}
		case <-time.After(10 * time.Second):
			t.Error("deny path: stream did not close within 10s after denial")
		}
	})
}
