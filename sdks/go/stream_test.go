package flintgate

import (
	"bufio"
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

// TestStreamSSE_Reconnect verifies that the client reconnects after a 503
// (retryable 5xx) and delivers events from the second connection.
func TestStreamSSE_Reconnect(t *testing.T) {
	var connCount int32

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn := atomic.AddInt32(&connCount, 1)

		if conn == 1 {
			// First connection: return 503 to trigger reconnect.
			http.Error(w, "temporarily unavailable", http.StatusServiceUnavailable)
			return
		}
		// Second connection: send an event and clean close.
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		flusher, _ := w.(http.Flusher)
		bw := bufio.NewWriter(w)
		_, _ = bw.WriteString("data: hello-from-reconnect\n\n")
		_ = bw.Flush()
		if flusher != nil {
			flusher.Flush()
		}
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	ch := StreamSSE(ctx, srv.Client(), srv.URL+"/stream", "", StreamOptions{})

	var got []Event
	for ev := range ch {
		if ev.IsError() {
			t.Logf("error event: %s", ev.Data)
			break
		}
		got = append(got, ev)
		cancel() // got the reconnected event, done
	}

	if len(got) < 1 {
		t.Fatalf("expected at least 1 event after reconnect, got 0")
	}
	if got[0].Data != "hello-from-reconnect" {
		t.Errorf("event[0].Data = %q, want %q", got[0].Data, "hello-from-reconnect")
	}
	if atomic.LoadInt32(&connCount) < 2 {
		t.Errorf("connCount = %d, want >= 2 (reconnect must have happened)", atomic.LoadInt32(&connCount))
	}
}

// TestStreamSSE_ReconnectExhausted verifies that after all retries fail the
// client emits an error event.
func TestStreamSSE_ReconnectExhausted(t *testing.T) {
	var connCount int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		atomic.AddInt32(&connCount, 1)
		// Always return 503 (retryable 5xx).
		http.Error(w, "unavailable", http.StatusServiceUnavailable)
	}))
	defer srv.Close()

	fastClient := &http.Client{Timeout: 2 * time.Second}

	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	ch := StreamSSE(ctx, fastClient, srv.URL+"/stream", "", StreamOptions{})

	var got []Event
	for ev := range ch {
		got = append(got, ev)
	}

	if len(got) == 0 || !got[len(got)-1].IsError() {
		t.Fatalf("expected final error event after exhaustion, got %+v", got)
	}
}

// TestStreamSSE_LastEventID verifies that the Last-Event-ID header is sent on
// reconnect after a transport error.
func TestStreamSSE_LastEventID(t *testing.T) {
	var connCount int32
	var lastIDOnSecond string

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn := atomic.AddInt32(&connCount, 1)

		if conn == 1 {
			hj, ok := w.(http.Hijacker)
			if !ok {
				t.Errorf("ResponseWriter does not support hijacking")
				return
			}
			rawConn, bufrw, err := hj.Hijack()
			if err != nil {
				t.Errorf("Hijack failed: %v", err)
				return
			}
			_, _ = bufrw.WriteString("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
			_, _ = bufrw.WriteString("1e\r\nid: event-42\ndata: ping\n\n\r\n")
			_ = bufrw.Flush()
			_ = rawConn.(*net.TCPConn).SetLinger(0)
			rawConn.Close()
			return
		}
		lastIDOnSecond = r.Header.Get("Last-Event-ID")
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		flusher, _ := w.(http.Flusher)
		_, _ = bufio.NewWriter(w).WriteString("data: pong\n\n")
		if flusher != nil {
			flusher.Flush()
		}
	}))
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	ch := StreamSSE(ctx, srv.Client(), srv.URL+"/stream", "", StreamOptions{})

	var got []Event
	for ev := range ch {
		if ev.IsError() {
			break
		}
		got = append(got, ev)
		if len(got) == 2 {
			cancel()
		}
	}

	if lastIDOnSecond != "event-42" {
		t.Errorf("Last-Event-ID on reconnect = %q, want %q", lastIDOnSecond, "event-42")
	}
}
