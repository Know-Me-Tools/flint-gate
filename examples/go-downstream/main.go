package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/signal"
	"time"

	flintgate "github.com/know-me-tools/flint-gate/sdks/go"
)

func main() {
	mux := http.NewServeMux()
	mux.HandleFunc("/api/hello", handleHello)
	mux.HandleFunc("/api/admin", handleAdmin)

	opts := flintgate.MiddlewareOptions{
		RequireFlintHeader: os.Getenv("FLINT_REQUIRE_HEADER") != "false",
	}

	handler := flintgate.NewMiddleware(mux, opts)

	srv := &http.Server{
		Addr:         ":8080",
		Handler:      handler,
		ReadTimeout:  5 * time.Second,
		WriteTimeout: 10 * time.Second,
	}

	go func() {
		log.Printf("downstream service listening on %s", srv.Addr)
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("server error: %v", err)
		}
	}()

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt)
	<-stop

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := srv.Shutdown(ctx); err != nil {
		log.Fatalf("shutdown error: %v", err)
	}
}

func handleHello(w http.ResponseWriter, r *http.Request) {
	id := flintgate.IdentityFromContext(r.Context())
	rid := flintgate.RequestIDFromContext(r.Context())

	w.Header().Set("content-type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"request_id": rid,
		"subject":    id.Subject,
		"provider":   id.Provider,
		"scopes":     id.Scopes,
		"message":    "hello from downstream",
	})
}

func handleAdmin(w http.ResponseWriter, r *http.Request) {
	// RequireFlintHeader is already enforced by the middleware. This gate
	// additionally requires the admin scope.
	flintgate.RequireScope(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := flintgate.IdentityFromContext(r.Context())
		fmt.Fprintf(w, "admin ok for subject=%s\n", id.Subject)
	}), "admin").ServeHTTP(w, r)
}
