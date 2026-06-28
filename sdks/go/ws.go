package flintgate

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"

	"nhooyr.io/websocket"
)

// WSClient is a thin convenience wrapper around nhooyr.io/websocket that
// speaks to Flint Gate's WebSocket-routed endpoints. It is safe for one
// concurrent reader and one concurrent writer (full-duplex) as enforced by
// the underlying conn.
type WSClient struct {
	url    string
	token  string
	dialer *websocket.DialOptions
}

// WSOptions configure a WSClient.
type WSOptions struct {
	// Token is sent as a Sec-WebSocket-Protocol subprotocol "bearer.<token>"
	// if non-empty. (Flint Gate also accepts Authorization on the upgrade
	// request; both are sent when Token is set.)
	Token string
	// Headers attached to the upgrade request.
	Headers http.Header
	// DialOptions overrides the default dial options entirely when non-nil.
	DialOptions *websocket.DialOptions
}

// NewWSClient builds a WSClient for the given WebSocket URL (ws:// or wss://).
func NewWSClient(url string, opts WSOptions) (*WSClient, error) {
	u := strings.TrimSpace(url)
	if u == "" {
		return nil, errors.New("flintgate: ws url is required")
	}
	if !strings.HasPrefix(u, "ws://") && !strings.HasPrefix(u, "wss://") {
		return nil, fmt.Errorf("flintgate: ws url must be ws:// or wss://, got %q", u)
	}
	return &WSClient{
		url:   u,
		token: opts.Token,
		dialer: opts.DialOptionsOrNil(),
	}, nil
}

// DialOptionsOrNil returns a non-zero *websocket.DialOptions or nil.
func (o WSOptions) DialOptionsOrNil() *websocket.DialOptions {
	if o.DialOptions != nil {
		return o.DialOptions
	}
	d := &websocket.DialOptions{
		HTTPHeader: o.Headers,
	}
	if o.Token != "" {
		// Send bearer token both as subprotocol and as Authorization header
		// so it works regardless of which side Flint Gate inspects.
		d.Subprotocols = append(d.Subprotocols, "bearer."+o.Token)
		if d.HTTPHeader == nil {
			d.HTTPHeader = http.Header{}
		}
		d.HTTPHeader.Set("Authorization", "Bearer "+o.Token)
	}
	return d
}

// Dial connects to the WebSocket endpoint. The returned Conn must be closed
// by the caller. The default read/write deadline is none; use Conn.SetReadDeadline.
func (c *WSClient) Dial(ctx context.Context) (*websocket.Conn, error) {
	dialer := c.dialer
	if dialer == nil {
		dialer = &websocket.DialOptions{}
	}

	conn, resp, err := websocket.Dial(ctx, c.url, dialer)
	if err != nil {
		if resp != nil {
			resp.Body.Close()
		}
		return nil, fmt.Errorf("flintgate: ws dial %s: %w", c.url, err)
	}
	// Negotiated subprotocol is informational; we don't enforce it here.

	// Reasonable defaults: ping every 30s, allow 10s pong latency.
	conn.SetReadLimit(1 << 20) // 1 MiB max frame
	return conn, nil
}

// DialWithDefaults is a one-shot helper: dial, run the handler, then close.
// The handler runs in the same goroutine; it should return when it's done
// reading or when ctx is cancelled. On return, the conn is closed with
// StatusNormalClosure unless the handler returns a non-nil error (in which
// case StatusProtocolError is used).
func (c *WSClient) DialWithDefaults(
	ctx context.Context,
	handler func(ctx context.Context, conn *websocket.Conn) error,
) error {
	if handler == nil {
		return errors.New("flintgate: nil ws handler")
	}
	conn, err := c.Dial(ctx)
	if err != nil {
		return err
	}
	defer conn.Close(websocket.StatusInternalError, "shutdown")

	pingCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	go keepalive(pingCtx, conn, 30*time.Second)

	if err := handler(ctx, conn); err != nil {
		conn.Close(websocket.StatusProtocolError, err.Error())
		return err
	}
	return conn.Close(websocket.StatusNormalClosure, "bye")
}

// keepalive sends periodic pings until ctx is done.
func keepalive(ctx context.Context, conn *websocket.Conn, every time.Duration) {
	t := time.NewTicker(every)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			pingCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
			err := conn.Ping(pingCtx)
			cancel()
			if err != nil {
				return
			}
		}
	}
}

// WSMessage is a generic JSON envelope wrapper for text/binary frames.
type WSMessage struct {
	Type int    // websocket.MessageText or websocket.MessageBinary
	Data []byte // payload (owned by the caller after receive)
}

// ReceiveJSON reads the next frame and returns its raw bytes and whether it
// was text. It is a thin wrapper around conn.Read.
func ReceiveJSON(ctx context.Context, conn *websocket.Conn) (WSMessage, error) {
	msgType, data, err := conn.Read(ctx)
	if err != nil {
		return WSMessage{}, err
	}
	return WSMessage{Type: int(msgType), Data: data}, nil
}
