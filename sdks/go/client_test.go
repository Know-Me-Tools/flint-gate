package flintgate

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// TestParseSSE_SingleEvent verifies a single two-data-line event parses into
// one Event with newline-joined data.
func TestParseSSE_SingleEvent(t *testing.T) {
	body := strings.NewReader(strings.Join([]string{
		`event: message`,
		`data: {"text":"hello"}`,
		`data: continued`,
		``,
		``, // blank line dispatches
	}, "\n"))

	ch := make(chan Event, 8)
	parseSSE(context.Background(), body, 1<<20, ch)
	close(ch)

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	if len(got) != 1 {
		t.Fatalf("expected 1 event, got %d: %+v", len(got), got)
	}
	ev := got[0]
	if ev.Event != "message" {
		t.Errorf("Event = %q, want %q", ev.Event, "message")
	}
	want := `{"text":"hello"}` + "\n" + "continued"
	if ev.Data != want {
		t.Errorf("Data = %q, want %q", ev.Data, want)
	}
}

// TestParseSSE_MultipleEvents verifies dispatch happens on every blank line.
func TestParseSSE_MultipleEvents(t *testing.T) {
	body := strings.NewReader(strings.Join([]string{
		`data: one`,
		``,
		`data: two`,
		``,
		`event: done`,
		`data: {}`,
		``,
	}, "\n"))

	ch := make(chan Event, 8)
	parseSSE(context.Background(), body, 1<<20, ch)
	close(ch)

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	if len(got) != 3 {
		t.Fatalf("expected 3 events, got %d: %+v", len(got), got)
	}
	if got[0].Data != "one" {
		t.Errorf("event[0].Data = %q, want %q", got[0].Data, "one")
	}
	if got[1].Data != "two" {
		t.Errorf("event[1].Data = %q, want %q", got[1].Data, "two")
	}
	if got[2].Event != "done" || got[2].Data != "{}" {
		t.Errorf("event[2] = %+v, want event=done data={}", got[2])
	}
}

// TestParseSSE_CommentAndCRLF verifies that comment lines are skipped and
// CRLF terminators are handled per spec.
func TestParseSSE_CommentAndCRLF(t *testing.T) {
	raw := ": this is a comment\r\n" +
		"data: payload\r\n" +
		"\r\n"
	body := strings.NewReader(raw)

	ch := make(chan Event, 4)
	parseSSE(context.Background(), body, 1<<20, ch)
	close(ch)

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	if len(got) != 1 {
		t.Fatalf("expected 1 event (comment skipped), got %d: %+v", len(got), got)
	}
	if got[0].Data != "payload" {
		t.Errorf("Data = %q, want %q", got[0].Data, "payload")
	}
	if strings.Contains(got[0].Data, "\r") {
		t.Errorf("Data contains stray CR: %q", got[0].Data)
	}
}

// TestParseSSE_RetryAndID verifies the retry and id fields are surfaced.
func TestParseSSE_RetryAndID(t *testing.T) {
	body := strings.NewReader(strings.Join([]string{
		`id: 42`,
		`retry: 5000`,
		`data: x`,
		``,
	}, "\n"))

	ch := make(chan Event, 4)
	parseSSE(context.Background(), body, 1<<20, ch)
	close(ch)

	ev := (<-ch)
	if ev.ID != "42" {
		t.Errorf("ID = %q, want 42", ev.ID)
	}
	if ev.Retry != 5000 {
		t.Errorf("Retry = %d, want 5000", ev.Retry)
	}
	if ev.Pace() != 5*time.Second {
		t.Errorf("Pace() = %v, want 5s", ev.Pace())
	}
}

// TestParseSSE_ByteLimit verifies the MaxEventBytes cap is enforced.
func TestParseSSE_ByteLimit(t *testing.T) {
	oversized := strings.Repeat("a", 1000)
	body := strings.NewReader("data: " + oversized + "\n\n")

	ch := make(chan Event, 4)
	parseSSE(context.Background(), body, 64, ch)
	close(ch)

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	// Exactly one error event, no payload event.
	if len(got) != 1 || !got[0].IsError() {
		t.Fatalf("expected 1 error event, got %+v", got)
	}
	if !strings.Contains(got[0].Data, "exceeded") {
		t.Errorf("error data = %q, want substring 'exceeded'", got[0].Data)
	}
}

// TestStreamSSE_EndToEnd drives the public API against an httptest.Server
// that emits two SSE events and then closes.
func TestStreamSSE_EndToEnd(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		flusher, _ := w.(http.Flusher)
		_, _ = io.WriteString(w, "data: first\n\n")
		if flusher != nil {
			flusher.Flush()
		}
		_, _ = io.WriteString(w, "event: custom\ndata: {\"k\":2}\n\n")
		if flusher != nil {
			flusher.Flush()
		}
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	ch := StreamSSE(ctx, srv.Client(), srv.URL+"/stream", "tok-abc", StreamOptions{})

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	if len(got) != 2 {
		t.Fatalf("expected 2 events, got %d: %+v", len(got), got)
	}
	if got[0].Data != "first" {
		t.Errorf("event[0].Data = %q, want %q", got[0].Data, "first")
	}
	if got[1].Event != "custom" || got[1].Data != `{"k":2}` {
		t.Errorf("event[1] = %+v, want event=custom data={\"k\":2}", got[1])
	}
}

// TestStreamSSE_BadStatus verifies an HTTP error is surfaced as an error event.
func TestStreamSSE_BadStatus(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "nope", http.StatusUnauthorized)
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	ch := StreamSSE(ctx, srv.Client(), srv.URL+"/stream", "", StreamOptions{})

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	if len(got) != 1 || !got[0].IsError() {
		t.Fatalf("expected 1 error event, got %+v", got)
	}
	if !strings.Contains(got[0].Data, "401") {
		t.Errorf("error data = %q, want substring '401'", got[0].Data)
	}
}

// TestStreamSSE_ContextCancel verifies that cancelling the context closes
// the channel without emitting an error event.
func TestStreamSSE_ContextCancel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		// Block forever; rely on client cancellation.
		<-r.Context().Done()
	}))
	defer srv.Close()

	ctx, cancel := context.WithCancel(context.Background())
	ch := StreamSSE(ctx, nil, srv.URL+"/stream", "", StreamOptions{})

	// Read one tick to ensure the request is in flight, then cancel.
	time.AfterFunc(50*time.Millisecond, cancel)

	start := time.Now()
	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}
	elapsed := time.Since(start)
	if len(got) != 0 {
		t.Errorf("expected no events on cancel, got %+v", got)
	}
	if elapsed > 2*time.Second {
		t.Errorf("channel did not close promptly after cancel: %v", elapsed)
	}
}

// TestSplitField covers the field/value splitter directly.
func TestSplitField(t *testing.T) {
	cases := []struct {
		in       string
		field    string
		value    string
		colon    bool
	}{
		{"data: hello", "data", " hello", true},
		{"data:hello", "data", "hello", true},
		{"data", "data", "", false},
		{":comment", "", "comment", true},
		{"id: 7", "id", " 7", true},
	}
	for _, c := range cases {
		f, v, k := splitField(c.in)
		if f != c.field || v != c.value || k != c.colon {
			t.Errorf("splitField(%q) = (%q,%q,%v), want (%q,%q,%v)",
				c.in, f, v, k, c.field, c.value, c.colon)
		}
	}
}

// TestAtoi covers the integer parser.
func TestAtoi(t *testing.T) {
	cases := []struct {
		in   string
		want int
		ok   bool
	}{
		{"0", 0, true},
		{"123", 123, true},
		{"", 0, false},
		{"-1", 0, false},
		{"12a", 0, false},
	}
	for _, c := range cases {
		got, ok := atoi(c.in)
		if got != c.want || ok != c.ok {
			t.Errorf("atoi(%q) = (%d,%v), want (%d,%v)", c.in, got, ok, c.want, c.ok)
		}
	}
}

// TestClient_NewClient_InvalidURL guards the constructor.
func TestClient_NewClient_InvalidURL(t *testing.T) {
	_, err := NewClient(Options{BaseURL: "://bad"})
	if err == nil {
		t.Fatal("expected error for invalid url")
	}
	_, err = NewClient(Options{BaseURL: "ftp://example.com"})
	if err == nil {
		t.Fatal("expected error for non-http scheme")
	}
}

// TestClient_GetHealth_OK exercises the admin GET plumbing against a stub server.
func TestClient_GetHealth_OK(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/health" {
			t.Errorf("path = %q, want /health", r.URL.Path)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer admin-tok" {
			t.Errorf("Authorization = %q, want Bearer admin-tok", got)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"status":"ok","uptime_seconds":1.5,"checked_at":"2026-01-01T00:00:00Z"}`)
	}))
	defer srv.Close()

	c, err := NewClient(Options{BaseURL: srv.URL, AdminToken: "admin-tok"})
	if err != nil {
		t.Fatal(err)
	}
	h, err := c.GetHealth(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if h.Status != "ok" {
		t.Errorf("Status = %q, want ok", h.Status)
	}
	if h.UptimeSec != 1.5 {
		t.Errorf("UptimeSec = %v, want 1.5", h.UptimeSec)
	}
}

// TestClient_GetHealth_Error verifies non-2xx is converted to APIError.
func TestClient_GetHealth_Error(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "server melting", http.StatusInternalServerError)
	}))
	defer srv.Close()

	c, _ := NewClient(Options{BaseURL: srv.URL})
	_, err := c.GetHealth(context.Background())
	if err == nil {
		t.Fatal("expected error")
	}
	ae, ok := err.(*APIError)
	if !ok {
		t.Fatalf("expected *APIError, got %T (%v)", err, err)
	}
	if ae.StatusCode != 500 {
		t.Errorf("StatusCode = %d, want 500", ae.StatusCode)
	}
	if !strings.Contains(ae.Body, "melting") {
		t.Errorf("Body = %q, want substring 'melting'", ae.Body)
	}
}

// TestClient_DeleteRoute_Idempotent verifies 404 is swallowed.
func TestClient_DeleteRoute_Idempotent(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "not found", http.StatusNotFound)
	}))
	defer srv.Close()

	c, _ := NewClient(Options{BaseURL: srv.URL})
	if err := c.DeleteRoute(context.Background(), "missing"); err != nil {
		t.Errorf("DeleteRoute on 404 should be nil, got %v", err)
	}
}

// TestMiddleware_Identity verifies header rehydration and context attachment.
func TestMiddleware_Identity(t *testing.T) {
	var seen *Identity
	inner := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen = IdentityFromContext(r.Context())
		w.WriteHeader(http.StatusOK)
	})
	srv := httptest.NewServer(NewMiddleware(inner, MiddlewareOptions{}))
	defer srv.Close()

	req, _ := http.NewRequest(http.MethodGet, srv.URL+"/", nil)
	req.Header.Set(HeaderIdentityProvider, "api_key")
	req.Header.Set(HeaderIdentitySubject, "user-123")
	req.Header.Set(HeaderIdentityScopes, "read write admin")
	req.Header.Set(HeaderIdentityClientID, "client-7")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()

	if resp.Header.Get(HeaderRequestID) == "" {
		t.Error("expected X-Request-Id to be set on response")
	}
	if seen == nil {
		t.Fatal("identity not attached to context")
	}
	if seen.Subject != "user-123" || seen.Provider != "api_key" {
		t.Errorf("identity = %+v", seen)
	}
	if !seen.HasAllScopes("read", "write") || !seen.HasAnyScope("nope", "admin") {
		t.Errorf("scope checks failed for %+v", seen)
	}
}

// TestMiddleware_RequireFlintHeader verifies the rejection path.
func TestMiddleware_RequireFlintHeader(t *testing.T) {
	inner := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("inner handler should not be called")
	})
	srv := httptest.NewServer(NewMiddleware(inner, MiddlewareOptions{RequireFlintHeader: true}))
	defer srv.Close()

	resp, err := http.Get(srv.URL + "/")
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("status = %d, want 401", resp.StatusCode)
	}
}

// TestRequireScope verifies the scope-guard wrapper.
func TestRequireScope(t *testing.T) {
	cases := []struct {
		name     string
		provider string
		scopes   string
		require  []string
		want     int
	}{
		{"anonymous_denied", "anonymous", "", []string{"read"}, 401},
		{"no_identity", "", "", []string{"read"}, 401},
		{"has_required", "jwt", "read", []string{"read"}, 200},
		{"missing_required", "jwt", "read", []string{"admin"}, 403},
		{"any_of_required", "jwt", "write", []string{"read", "write"}, 200},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			inner := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(http.StatusOK)
			})
			h := NewMiddleware(RequireScope(inner, c.require...), MiddlewareOptions{})
			srv := httptest.NewServer(h)
			defer srv.Close()

			req, _ := http.NewRequest(http.MethodGet, srv.URL+"/", nil)
			if c.provider != "" {
				req.Header.Set(HeaderIdentityProvider, c.provider)
			}
			if c.scopes != "" {
				req.Header.Set(HeaderIdentityScopes, c.scopes)
			}

			resp, err := http.DefaultClient.Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			if resp.StatusCode != c.want {
				t.Errorf("status = %d, want %d", resp.StatusCode, c.want)
			}
		})
	}
}

// Compile-time guard: ensure *bytes.Reader satisfies the reader interface
// parseSSE expects.
var _ interface{ Read(p []byte) (int, error) } = (*bytes.Reader)(nil)
