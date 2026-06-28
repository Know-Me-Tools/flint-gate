package flintgate

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"net/http"
	"regexp"
	"strings"
)

// Flint Gate injects the resolved identity into the headers it forwards to
// upstream services. Downstream Go services use this middleware to rehydrate
// that identity and attach it to the request context.
const (
	// HeaderIdentityProvider is the auth provider that resolved the request.
	HeaderIdentityProvider = "X-Flint-Identity-Provider"
	// HeaderIdentitySubject is the subject identifier (user id, client id, ...).
	HeaderIdentitySubject = "X-Flint-Identity-Subject"
	// HeaderIdentityScopes is a space-delimited list of granted scopes.
	HeaderIdentityScopes = "X-Flint-Identity-Scopes"
	// HeaderIdentityClientID is set when the provider is api_key.
	HeaderIdentityClientID = "X-Flint-Identity-Client-Id"
	// HeaderIdentitySessionID is set when the provider is kratos.
	HeaderIdentitySessionID = "X-Flint-Identity-Session-Id"
	// HeaderRequestID is propagated through the proxy.
	HeaderRequestID = "X-Request-Id"

	// ProviderAnonymous is the identity provider string for unauthenticated requests.
	ProviderAnonymous = "anonymous"
)

// contextKey is unexported so callers can't spoof the context value.
type contextKey struct{ name string }

var identityCtxKey = &contextKey{"flint-identity"}

// IdentityFromContext returns the Identity attached to ctx by this middleware,
// or nil if absent.
func IdentityFromContext(ctx context.Context) *Identity {
	v, _ := ctx.Value(identityCtxKey).(*Identity)
	return v
}

// IdentityFromHeaders rehydrates an Identity from the headers Flint Gate
// injected. It never returns nil. If no identity headers are present, the
// returned Identity has Provider == "anonymous".
func IdentityFromHeaders(h http.Header) *Identity {
	id := &Identity{
		Provider:  firstHeader(h, HeaderIdentityProvider, ProviderAnonymous),
		Subject:   h.Get(HeaderIdentitySubject),
		Scopes:    splitScopes(h.Get(HeaderIdentityScopes)),
		ClientID:  h.Get(HeaderIdentityClientID),
		SessionID: h.Get(HeaderIdentitySessionID),
	}
	if id.Provider == "" {
		id.Provider = ProviderAnonymous
	}
	return id
}

// RequireScope returns an http.HandlerFunc that authorizes the request against
// the given scope(s). Requests whose identity lacks any required scope receive
// 403 Forbidden. Requests with no identity at all receive 401 Unauthorized.
//
// require is OR semantics when len > 1: the identity must hold at least one
// of the listed scopes.
func RequireScope(next http.Handler, require ...string) http.Handler {
	if len(require) == 0 {
		return next
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := IdentityFromContext(r.Context())
		if id == nil || id.Provider == ProviderAnonymous {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		if !id.HasAnyScope(require...) {
			http.Error(w, "forbidden", http.StatusForbidden)
			return
		}
		next.ServeHTTP(w, r)
	})
}

// HasAnyScope reports whether the identity holds at least one of the scopes.
func (i *Identity) HasAnyScope(scopes ...string) bool {
	if i == nil || len(i.Scopes) == 0 {
		return false
	}
	owned := make(map[string]struct{}, len(i.Scopes))
	for _, s := range i.Scopes {
		owned[s] = struct{}{}
	}
	for _, want := range scopes {
		if _, ok := owned[want]; ok {
			return true
		}
	}
	return false
}

// HasAllScopes reports whether the identity holds every listed scope.
func (i *Identity) HasAllScopes(scopes ...string) bool {
	if i == nil || len(scopes) == 0 {
		return len(scopes) == 0
	}
	owned := make(map[string]struct{}, len(i.Scopes))
	for _, s := range i.Scopes {
		owned[s] = struct{}{}
	}
	for _, want := range scopes {
		if _, ok := owned[want]; !ok {
			return false
		}
	}
	return true
}

// Middleware is the canonical http.Handler middleware for Go services
// deployed behind Flint Gate. It:
//
//   1. Rehydrates the identity from headers and attaches it to the request
//      context.
//   2. Propagates X-Request-Id (or generates one if missing) and attaches it
//      to both the outgoing response and the request context.
//   3. Rejects requests that did not come through Flint Gate when
//      Options.RequireFlintHeader is set.
//
// Wrap any http.Handler: http.ListenAndServe(":8080", flintgate.Middleware(svc, opts))
type Middleware struct {
	next             http.Handler
	requireFlintHdr  bool
	trustProxyCIDRs  []*regexp.Regexp // future: direct-IP allowlisting
}

// MiddlewareOptions configure the flintgate middleware.
type MiddlewareOptions struct {
	// RequireFlintHeader, when true, rejects (401) any request that lacks
	// X-Flint-Identity-Provider. Use in production to ensure traffic can
	// only reach your service via Flint Gate.
	RequireFlintHeader bool
}

// NewMiddleware wraps next with the flintgate identity/request-id middleware.
func NewMiddleware(next http.Handler, opts MiddlewareOptions) http.Handler {
	m := &Middleware{
		next:            next,
		requireFlintHdr: opts.RequireFlintHeader,
	}
	return m
}

func (m *Middleware) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	h := r.Header

	// 1. Enforce "must come through Flint Gate" if configured.
	if m.requireFlintHdr && h.Get(HeaderIdentityProvider) == "" {
		http.Error(w, "unauthorized: missing flint-gate identity", http.StatusUnauthorized)
		return
	}

	// 2. Rehydrate identity into context.
	id := IdentityFromHeaders(h)
	ctx := context.WithValue(r.Context(), identityCtxKey, id)

	// 3. Request id propagation.
	rid := h.Get(HeaderRequestID)
	if rid == "" {
		rid = newRequestID()
	}
	w.Header().Set(HeaderRequestID, rid)
	ctx = WithRequestID(ctx, rid)

	m.next.ServeHTTP(w, r.WithContext(ctx))
}

// ---------------------------------------------------------------------------
// Request ID
// ---------------------------------------------------------------------------

var requestIDCtxKey = &contextKey{"flint-request-id"}

// WithRequestID attaches a request id to ctx.
func WithRequestID(ctx context.Context, id string) context.Context {
	return context.WithValue(ctx, requestIDCtxKey, id)
}

// RequestIDFromContext returns the request id attached by the middleware,
// or "" if absent.
func RequestIDFromContext(ctx context.Context) string {
	v, _ := ctx.Value(requestIDCtxKey).(string)
	return v
}

// newRequestID returns a fresh, weakly-unique request id. Production callers
// may substitute their own generator by writing the header before the
// middleware runs.
func newRequestID() string {
	return "svc-" + randHex(12)
}

// randHex returns n bytes of crypto-grade random hex (2n characters).
func randHex(n int) string {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		// rand.Read should never fail; fall back to a constant prefix so the
		// id is still non-empty and obviously degraded.
		return "000000000000"
	}
	return hex.EncodeToString(b)
}

// firstHeader returns the first non-empty value of the given headers, falling
// back to def.
func firstHeader(h http.Header, keys ...string) string {
	for _, k := range keys {
		if v := h.Get(k); v != "" {
			return v
		}
	}
	return ""
}

// splitScopes tokenizes a space-delimited scope list.
var scopeSplitter = regexp.MustCompile(`[,\s]+`)

func splitScopes(s string) []string {
	if s == "" {
		return nil
	}
	parts := scopeSplitter.Split(strings.TrimSpace(s), -1)
	out := parts[:0]
	for _, p := range parts {
		if p != "" {
			out = append(out, p)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}
