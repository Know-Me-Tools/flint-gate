import { AuthConfig, FlintGateError, StreamEvent } from "./types";

/**
 * WebSocket client for Flint Gate WS protocol routes.
 *
 * The WS layer speaks a tiny JSON envelope:
 *   → `{ "type": "subscribe", "channel": "..." }`
 *   ← `{ "type": "event", "channel": "...", "data": <StreamEvent> }`
 *
 * Because the WebSocket API is not available in all edge runtimes, this
 * module does NOT import a polyfill — it uses `globalThis.WebSocket` and
 * throws a typed error if absent. Callers in Node.js should install the
 * `ws` package and pass `WebSocketCtor` explicitly.
 */
export interface FlintGateWSOptions {
  /** Path under the WS-capable route, e.g. `/ws`. Defaults to `/`. */
  path?: string;
  /** Auth config — same shapes supported by the HTTP client. */
  auth?: AuthConfig;
  /** Protocols passed to the WebSocket constructor. */
  protocols?: string | string[];
  /** Custom WebSocket constructor (Node.js `ws` etc.). */
  WebSocketCtor?: typeof WebSocket;
  /** AbortSignal — closing it closes the socket with code 1000. */
  signal?: AbortSignal;
  /** Optional handshake headers (Node.js `ws` only; ignored in browsers). */
  headers?: Record<string, string>;
}

/**
 * Subscribe to a Flint Gate WebSocket channel as an async iterable of
 * {@link StreamEvent} values.
 *
 * Example:
 * ```ts
 * for await (const evt of streamWS("wss://gate.example.com/ws", { channel: "chat" })) {
 *   if (evt.type === "text-delta") process.stdout.write(evt.text);
 * }
 * ```
 */
export function streamWS(
  url: string | URL,
  opts: FlintGateWSOptions & { readonly channel: string } = { channel: "" },
): AsyncGenerator<StreamEvent, void, unknown> {
  return streamWSImpl(url, opts);
}

async function* streamWSImpl(
  url: string | URL,
  opts: FlintGateWSOptions & { readonly channel: string },
): AsyncGenerator<StreamEvent, void, unknown> {
  const WS = opts.WebSocketCtor ?? globalThis.WebSocket;
  if (!WS) {
    throw new FlintGateError(
      "WebSocket is not available in this runtime — pass WebSocketCtor",
      { code: "NO_WEBSOCKET" },
    );
  }

  const finalUrl = applyAuthToUrl(url, opts.auth);
  // Browser signature: (url, protocols). Node `ws` accepts (url, protocols, headers).
  // Cast to a permissive constructor type so both call shapes type-check.
  type AnyWS = new (url: string | URL, protocols?: string | string[], opts?: unknown) => WebSocket;
  const ws = new (WS as unknown as AnyWS)(finalUrl, opts.protocols, opts.headers) as WebSocket;

  const queue: StreamEvent[] = [];
  let resolveNext: ((v: IteratorResult<StreamEvent>) => void) | null = null;
  let closed = false;
  let closeError: FlintGateError | null = null;

  const push = (evt: StreamEvent): void => {
    if (resolveNext) {
      const r = resolveNext;
      resolveNext = null;
      r({ value: evt, done: false });
    } else {
      queue.push(evt);
    }
    if (evt.type === "done" || evt.type === "error") {
      close(1000);
    }
  };

  const close = (code?: number): void => {
    if (closed) return;
    closed = true;
    try {
      ws.close(code ?? 1000);
    } catch {
      /* ignore */
    }
    if (resolveNext) {
      const r = resolveNext;
      resolveNext = null;
      r(closeError ? { value: closeError, done: true } : { value: undefined, done: true });
    }
  };

  ws.onopen = () => {
    try {
      ws.send(
        JSON.stringify({ type: "subscribe", channel: opts.channel }),
      );
    } catch (err) {
      closeError = new FlintGateError(
        `failed to send subscribe: ${(err as Error).message}`,
        { code: "WS_SEND_FAILED" },
      );
      close();
    }
  };

  ws.onmessage = (ev: MessageEvent) => {
    const raw = typeof ev.data === "string" ? ev.data : "";
    if (!raw) return;
    let frame: unknown;
    try {
      frame = JSON.parse(raw);
    } catch {
      // Treat bare text frames as text deltas.
      push({ type: "text-delta", text: raw });
      return;
    }
    const env = frame as {
      type?: string;
      channel?: string;
      data?: unknown;
      error?: string;
      code?: string;
    };
    if (env.type === "error") {
      push({
        type: "error",
        message: env.error ?? "unknown ws error",
        code: env.code,
      });
      return;
    }
    if (env.type === "event" && env.data && typeof env.data === "object") {
      const inner = env.data as { type?: string };
      // Already a StreamEvent shape.
      if (
        inner.type === "text-delta" ||
        inner.type === "tool-call" ||
        inner.type === "done" ||
        inner.type === "error"
      ) {
        push(inner as StreamEvent);
        return;
      }
      // Bare text.
      if (typeof (env.data as { text?: unknown }).text === "string") {
        push({ type: "text-delta", text: (env.data as { text: string }).text });
        return;
      }
    }
    if (env.type === "ping") {
      try {
        ws.send(JSON.stringify({ type: "pong" }));
      } catch {
        /* ignore */
      }
      return;
    }
    // Unknown envelope — surface as a text delta so callers see something.
    push({ type: "text-delta", text: raw });
  };

  ws.onerror = () => {
    closeError = new FlintGateError("websocket error", { code: "WS_ERROR" });
    close();
  };

  ws.onclose = () => {
    closed = true;
    if (resolveNext) {
      const r = resolveNext;
      resolveNext = null;
      r(closeError ? { value: closeError, done: true } : { value: undefined, done: true });
    }
  };

  if (opts.signal) {
    if (opts.signal.aborted) close();
    else opts.signal.addEventListener("abort", () => close(), { once: true });
  }

  while (true) {
    if (queue.length > 0) {
      const next = queue.shift() as StreamEvent;
      yield next;
      if (next.type === "done" || next.type === "error") return;
      continue;
    }
    if (closed) {
      if (closeError) throw closeError;
      return;
    }
    const result = await new Promise<IteratorResult<StreamEvent>>((resolve) => {
      resolveNext = resolve;
    });
    if (result.done) {
      if (result.value instanceof FlintGateError) throw result.value;
      return;
    }
    yield result.value;
    if (result.value.type === "done" || result.value.type === "error") return;
  }
}

function applyAuthToUrl(url: string | URL, auth?: AuthConfig): string {
  const u = url instanceof URL ? url : new URL(url);
  if (!auth) return u.toString();
  switch (auth.type) {
    case "apiKey": {
      u.searchParams.set("key", auth.key);
      return u.toString();
    }
    case "bearer": {
      u.searchParams.set("token", auth.token);
      return u.toString();
    }
    case "anonymous":
    case "cookie":
    default:
      return u.toString();
  }
}
