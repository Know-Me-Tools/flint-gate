package flintgate

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// Event is a single Server-Sent Event frame parsed from a Flint Gate
// (or upstream LLM) SSE stream. See https://html.spec.whatwg.org/#server-sent-events.
type Event struct {
	// ID is the value of the last `id:` field, if any.
	ID string
	// Event is the value of the last `event:` field, if any. Empty means
	// the default "message" event type.
	Event string
	// Data is the concatenation of all `data:` field payloads, joined by
	// "\n" per the SSE spec.
	Data string
	// Retry is the last `retry:` field value, in milliseconds, or 0 if none.
	Retry int
}

// HasData reports whether the event carries any data.
func (e Event) HasData() bool { return e.Data != "" }

// IsError reports whether the event is an SSE error frame. Flint Gate emits
// these as `event: error` with a JSON body on the error channel.
func (e Event) IsError() bool { return e.Event == "error" }

// Pace returns the Retry interval as a Duration, or zero if unspecified.
func (e Event) Pace() time.Duration {
	if e.Retry <= 0 {
		return 0
	}
	return time.Duration(e.Retry) * time.Millisecond
}

// ---------------------------------------------------------------------------
// StreamSSE
// ---------------------------------------------------------------------------

// StreamOptions configures StreamSSE.
type StreamOptions struct {
	// Headers added to the request (e.g. X-Request-Id, Last-Event-ID).
	Headers http.Header
	// MaxEventBytes caps a single reassembled event at this many bytes.
	// 0 means 1 MiB. Used to defend against pathological upstreams.
	MaxEventBytes int64
}

// StreamSSE issues a GET request to urlStr with the given bearer token and
// returns a receive-only channel of parsed SSE events. The channel is closed
// when the stream ends (clean EOF, server-side close, or non-retryable error).
//
// The first error encountered (transport, HTTP status >= 400, or parse error)
// is delivered as an Event with Event == "error" and Data containing a JSON
// object {"error": "..."} and then the channel is closed. Cancellation via
// ctx closes the channel without emitting an error.
//
// The caller MUST drain the returned channel to release resources.
func StreamSSE(
	ctx context.Context,
	httpClient *http.Client,
	urlStr string,
	bearerToken string,
	opts StreamOptions,
) <-chan Event {
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 0} // no overall timeout for streams
	}
	maxBytes := opts.MaxEventBytes
	if maxBytes <= 0 {
		maxBytes = 1 << 20 // 1 MiB
	}

	out := make(chan Event, 16)

	go func() {
		defer close(out)

		req, err := http.NewRequestWithContext(ctx, http.MethodGet, urlStr, nil)
		if err != nil {
			emitError(out, fmt.Sprintf(`{"error":%q}`, "build request: "+err.Error()))
			return
		}
		req.Header.Set("Accept", "text/event-stream")
		req.Header.Set("Cache-Control", "no-cache")
		if bearerToken != "" {
			req.Header.Set("Authorization", "Bearer "+bearerToken)
		}
		for k, vs := range opts.Headers {
			for _, v := range vs {
				req.Header.Add(k, v)
			}
		}

		resp, err := httpClient.Do(req)
		if err != nil {
			if ctx.Err() != nil {
				return // cancellation, not an error
			}
			emitError(out, fmt.Sprintf(`{"error":%q}`, "http: "+err.Error()))
			return
		}
		defer resp.Body.Close()

		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			emitError(out, fmt.Sprintf(`{"error":%q}`, fmt.Sprintf("upstream status %d", resp.StatusCode)))
			return
		}

		parseSSE(ctx, resp.Body, maxBytes, out)
	}()

	return out
}

// emitError delivers a synthetic error event. Best-effort: if the channel is
// full or context cancelled, the event is dropped.
func emitError(ch chan<- Event, dataJSON string) {
	select {
	case ch <- Event{Event: "error", Data: dataJSON}:
	default:
	}
}

// parseSSE reads a text/event-stream body using bufio.Scanner and dispatches
// events. It returns when the body is exhausted or ctx is cancelled.
func parseSSE(ctx context.Context, body interface{ Read(p []byte) (int, error) }, maxBytes int64, ch chan<- Event) {
	scanner := bufio.NewScanner(body)
	// Allow long individual lines (e.g. base64-encoded payloads).
	scanner.Buffer(make([]byte, 0, 64*1024), int(maxBytes))

	var (
		cur       Event
		dataLines []string
		dataBytes int64
	)

	dispatch := func() {
		if len(dataLines) > 0 {
			cur.Data = strings.Join(dataLines, "\n")
		}
		if cur.HasData() || cur.ID != "" || cur.Event != "" || cur.Retry > 0 {
			select {
			case <-ctx.Done():
			case ch <- cur:
			}
		}
		cur = Event{}
		dataLines = dataLines[:0]
		dataBytes = 0
	}

	for scanner.Scan() {
		if err := ctx.Err(); err != nil {
			return
		}
		line := scanner.Text()

		// Per spec: lines may end with CR, LF, or CRLF. bufio.Scanner
		// already stripped one terminator; drop a trailing CR if present.
		line = strings.TrimSuffix(line, "\r")

		// Blank line → dispatch current event.
		if line == "" {
			dispatch()
			continue
		}

		// Comment line — ignore.
		if strings.HasPrefix(line, ":") {
			continue
		}

		field, value, hadColon := splitField(line)
		if hadColon {
			value = strings.TrimPrefix(value, " ")
		}

		switch field {
		case "event":
			cur.Event = value
		case "id":
			cur.ID = value
		case "retry":
			if n, ok := atoi(value); ok {
				cur.Retry = n
			}
		case "data":
			dataLines = append(dataLines, value)
			dataBytes += int64(len(value)) + 1 // +1 for the joining \n
			if dataBytes > maxBytes {
				emitError(ch, fmt.Sprintf(`{"error":%q}`, fmt.Sprintf("event exceeded %d bytes", maxBytes)))
				return
			}
		}
		// Unknown fields are ignored per spec.
	}

	if err := scanner.Err(); err != nil {
		if !errors.Is(err, context.Canceled) {
			emitError(ch, fmt.Sprintf(`{"error":%q}`, "scan: "+err.Error()))
		}
		return
	}
	// Flush any final event at EOF.
	dispatch()
}

// splitField splits "field: value" into (field, value, hadColon).
func splitField(line string) (field, value string, hadColon bool) {
	if i := strings.IndexByte(line, ':'); i >= 0 {
		return line[:i], line[i+1:], true
	}
	return line, "", false
}

// atoi parses a non-negative decimal integer.
func atoi(s string) (int, bool) {
	if s == "" {
		return 0, false
	}
	n := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c < '0' || c > '9' {
			return 0, false
		}
		n = n*10 + int(c-'0')
	}
	return n, true
}
