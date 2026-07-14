import {
  Done,
  FlintGateApiError,
  FlintGateError,
  RawFrame,
  StreamError,
  StreamEvent,
  StreamProtocolError,
  TextDelta,
  TokenUsage,
  ToolCall,
} from "./types";

/**
 * Consume a Flint Gate SSE response body as an async iterable of typed
 * {@link StreamEvent} values.
 *
 * Accepts either an already-fetched `Response` (preferred, from
 * `client.requestStream`) or a `ReadableStream<Uint8Array>`. Pass an
 * `AbortSignal` to cancel mid-stream — the underlying body reader is
 * released and the iterator returns.
 *
 * Edge-runtime safe: uses TextDecoder only. No Node.js imports.
 */
export async function* streamSSE(
  source: Response | ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): AsyncGenerator<StreamEvent, void, unknown> {
  yield* streamSSEInternal(source, signal);
}

async function* streamSSEInternal(
  source: Response | ReadableStream<Uint8Array>,
  signal?: AbortSignal,
  onId?: (id: string) => void,
): AsyncGenerator<StreamEvent, void, unknown> {
  const lines = readLines(toStream(source), signal);
  let event = "message";
  let dataBuffer = "";

  for await (const line of lines) {
    if (signal?.aborted) return;

    if (line === "") {
      if (dataBuffer !== "") {
        const frame = decodeFrame(dataBuffer, event);
        dataBuffer = "";
        event = "message";
        if (frame) {
          const evt = normalizeFrame(frame, event);
          if (evt) {
            yield evt;
            if (evt.type === "done" || evt.type === "error") return;
          }
        }
      } else {
        event = "message";
      }
      continue;
    }

    if (line.startsWith(":")) continue; // comment / heartbeat

    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);

    if (field === "event") {
      event = value;
    } else if (field === "data") {
      dataBuffer = dataBuffer === "" ? value : `${dataBuffer}\n${value}`;
    } else if (field === "id") {
      onId?.(value);
    }
    // retry: is ignored.
  }

  // Flush any trailing frame without a blank-line terminator.
  if (dataBuffer !== "" && !signal?.aborted) {
    const frame = decodeFrame(dataBuffer, event);
    if (frame) {
      const evt = normalizeFrame(frame, event);
      if (evt) yield evt;
    }
  }
}

/**
 * Options for {@link streamSSEWithReconnect}.
 */
export interface SSEReconnectOptions {
  /** Maximum reconnect attempts after a network drop or 5xx. Default: 5. */
  maxReconnects?: number;
  /** Custom fetch implementation. Defaults to `globalThis.fetch`. */
  fetch?: typeof globalThis.fetch;
  /** Extra request headers. */
  headers?: Record<string, string>;
  /** Bearer token for `Authorization` header. */
  bearerToken?: string;
  /** AbortSignal to cancel the stream. */
  signal?: AbortSignal;
}

/**
 * Fetch an SSE endpoint with automatic reconnect on network drops or 5xx errors.
 *
 * - Tracks the last `id:` field and sends `Last-Event-ID` on reconnect.
 * - Reconnects up to `maxReconnects` times (default 5) with exponential backoff:
 *   250ms * 2^attempt, capped at 8s.
 * - HTTP 4xx responses are fatal and are not retried.
 * - Yields the same {@link StreamEvent} values as {@link streamSSE}.
 */
export async function* streamSSEWithReconnect(
  urlStr: string,
  opts: SSEReconnectOptions = {},
): AsyncGenerator<StreamEvent, void, unknown> {
  const maxReconnects = opts.maxReconnects ?? 5;
  const fetchImpl = opts.fetch ?? globalThis.fetch;
  const signal = opts.signal;

  let lastEventId: string | undefined;
  let attempt = 0;

  while (attempt <= maxReconnects) {
    if (signal?.aborted) return;

    const headers: Record<string, string> = { ...opts.headers };
    if (opts.bearerToken) {
      headers["Authorization"] = `Bearer ${opts.bearerToken}`;
    }
    if (lastEventId !== undefined) {
      headers["Last-Event-ID"] = lastEventId;
    }
    headers["Accept"] = "text/event-stream";
    headers["Cache-Control"] = "no-cache";

    let res: Response;
    try {
      res = await fetchImpl(urlStr, { headers, signal });
    } catch (err) {
      if (signal?.aborted) return;
      if (attempt >= maxReconnects) {
        throw new FlintGateError(
          `SSE connection failed after ${attempt} retries: ${String(err)}`,
          { code: "SSE_CONNECT_FAILED" },
        );
      }
      await sleep(sseBackoff(attempt));
      attempt++;
      continue;
    }

    // 4xx = fatal, do not retry
    if (res.status >= 400 && res.status < 500) {
      let body = "";
      try { body = await res.text(); } catch { /* ignore */ }
      throw new FlintGateApiError(
        body || `SSE endpoint returned ${res.status}`,
        { status: res.status },
      );
    }

    // 5xx or unexpected = retryable
    if (!res.ok) {
      if (attempt >= maxReconnects) {
        throw new FlintGateApiError(
          `SSE endpoint returned ${res.status} after ${attempt} retries`,
          { status: res.status },
        );
      }
      await sleep(sseBackoff(attempt));
      attempt++;
      continue;
    }

    // Successful connection — reset attempt counter, stream events
    attempt = 0;
    let streamError: unknown = null;
    let receivedDone = false;

    try {
      const idTracker = (id: string) => { lastEventId = id; };
      for await (const evt of streamSSEInternal(res, signal, idTracker)) {
        if (signal?.aborted) return;
        yield evt;
        if (evt.type === "done") {
          receivedDone = true;
          return;
        }
      }
      // Clean EOF with a done frame already yielded — normal termination
      if (receivedDone) return;
      // Clean EOF without a done frame — connection dropped, try to reconnect
    } catch (err) {
      if (signal?.aborted) return;
      streamError = err;
    }

    if (signal?.aborted) return;

    // Stream broke or dropped — retry if we have reconnects left
    if (attempt >= maxReconnects) {
      if (streamError) throw streamError;
      return;
    }
    await sleep(sseBackoff(attempt));
    attempt++;
  }
}

function sseBackoff(attempt: number): number {
  return Math.min(250 * Math.pow(2, attempt), 8000);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Consume an NDJSON (newline-delimited JSON) stream as an async iterable of
 * typed {@link StreamEvent} values. Each non-empty line is parsed as JSON
 * and normalized through the same pipeline as {@link streamSSE}.
 */
export async function* streamNDJSON(
  source: Response | ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): AsyncGenerator<StreamEvent, void, unknown> {
  for await (const line of readLines(toStream(source), signal)) {
    if (signal?.aborted) return;
    if (line === "") continue;
    const frame = decodeFrame(line, "message");
    if (!frame) continue;
    const evt = normalizeFrame(frame, "message");
    if (evt) {
      yield evt;
      if (evt.type === "done" || evt.type === "error") return;
    }
  }
}

// ---------------------------------------------------------------------------
// Line reader — handles CRLF, LF, and partial chunks across read boundaries.
// ---------------------------------------------------------------------------

function toStream(source: Response | ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
  if (source instanceof ReadableStream) return source;
  return (
    source.body ??
    new ReadableStream<Uint8Array>({ start(c) { c.close(); } })
  );
}

async function* readLines(
  stream: ReadableStream<Uint8Array>,
  signal?: AbortSignal,
): AsyncGenerator<string, void, unknown> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      if (signal?.aborted) return;
      // Race the read against the abort signal so we don't block forever on
      // a stream that never closes (e.g. test stubs or stalled connections).
      let readResult: ReadableStreamReadResult<Uint8Array>;
      if (signal) {
        const abortPromise = new Promise<ReadableStreamReadResult<Uint8Array>>((resolve) => {
          const onAbort = () => resolve({ done: true, value: undefined });
          if (signal.aborted) { onAbort(); return; }
          signal.addEventListener("abort", onAbort, { once: true });
        });
        readResult = await Promise.race([reader.read(), abortPromise]);
      } else {
        readResult = await reader.read();
      }
      const { value, done } = readResult;
      if (done) break;
      if (value == null) continue;
      buffer += decoder.decode(value, { stream: true });

      let nl: number;
      while ((nl = indexOfNewline(buffer)) !== -1) {
        const lineEnd = nl;
        // Handle CRLF.
        const ch = buffer.charCodeAt(nl);
        let consumeTo = nl + 1;
        if (ch === 13 /* \r */ && buffer.charCodeAt(nl + 1) === 10 /* \n */) {
          // line ends at \r; consume \r\n
        } else if (ch === 13) {
          // lone \r — line ends at \r
        }
        const line = buffer.slice(0, lineEnd);
        buffer = buffer.slice(consumeTo);
        yield line;
      }
    }
    buffer += decoder.decode();
    if (buffer !== "") yield buffer;
  } finally {
    try {
      reader.releaseLock();
    } catch {
      /* ignore */
    }
  }
}

function indexOfNewline(s: string): number {
  const lf = s.indexOf("\n");
  const cr = s.indexOf("\r");
  if (lf === -1) return cr;
  if (cr === -1) return lf;
  return Math.min(lf, cr);
}

// ---------------------------------------------------------------------------
// Frame decode + normalize
// ---------------------------------------------------------------------------

function decodeFrame(data: string, event: string): RawFrame | null {
  const trimmed = data.trim();
  if (trimmed === "") return null;
  if (trimmed === "[DONE]") {
    return { type: "done" };
  }
  try {
    return JSON.parse(trimmed) as RawFrame;
  } catch {
    if (event === "message") {
      return { type: "text-delta", text: data };
    }
    return { type: "error", message: `unparseable frame: ${trimmed.slice(0, 80)}` };
  }
}

function normalizeFrame(frame: RawFrame, sseEvent: string): StreamEvent | null {
  const t = (frame.type as string | undefined) ?? sseEvent;

  if (t === "text-delta" || t === "TEXT_MESSAGE_CONTENT" || t === "delta") {
    const text =
      (frame.text as string | undefined) ??
      (frame.textDelta as string | undefined) ??
      (frame.delta as string | undefined) ??
      (frame.content as string | undefined);
    if (typeof text !== "string") return null;
    const out: { type: "text-delta"; text: string; messageId?: string; index?: number } = {
      type: "text-delta",
      text,
    };
    if (typeof frame.messageId === "string") out.messageId = frame.messageId;
    if (typeof frame.index === "number") out.index = frame.index;
    return out;
  }

  if (t === "tool-call" || t === "TOOL_CALL" || t === "function_call") {
    const id = (frame.id as string | undefined) ?? (frame.toolCallId as string | undefined);
    const name = (frame.name as string | undefined) ?? (frame.toolName as string | undefined);
    if (!id || !name) return null;
    const args =
      (frame.args as Record<string, unknown> | undefined) ??
      (frame.arguments as Record<string, unknown> | undefined) ??
      (frame.input as Record<string, unknown> | undefined) ??
      {};
    const out: ToolCall = { type: "tool-call", id, name, args };
    return out;
  }

  if (t === "done" || t === "finish" || t === "DONE" || t === "[DONE]") {
    const usage = readUsage(frame);
    const out: { type: "done"; usage?: TokenUsage; requestId?: string } = { type: "done" };
    if (usage) out.usage = usage;
    if (typeof frame.requestId === "string") out.requestId = frame.requestId;
    return out;
  }

  if (t === "error" || t === "ERROR") {
    const message =
      (frame.message as string | undefined) ??
      (frame.error as string | undefined) ??
      "unknown stream error";
    const code = typeof frame.code === "string" ? frame.code : undefined;
    const status = typeof frame.status === "number" ? frame.status : undefined;
    throw new StreamProtocolError(message, code, status);
  }

  // Unknown frame type — ignore rather than fail.
  return null;
}

function readUsage(frame: RawFrame): TokenUsage | undefined {
  const metadata = frame.metadata as { usage?: Record<string, unknown> } | undefined;
  const u =
    (frame.usage as Record<string, unknown> | undefined) ??
    (metadata?.usage as Record<string, unknown> | undefined);
  if (!u) return undefined;
  const out: {
    promptTokens?: number;
    completionTokens?: number;
    totalTokens?: number;
  } = {};
  if (typeof u.promptTokens === "number") out.promptTokens = u.promptTokens;
  if (typeof u.completionTokens === "number") out.completionTokens = u.completionTokens;
  if (typeof u.totalTokens === "number") out.totalTokens = u.totalTokens;
  if (
    out.promptTokens === undefined &&
    out.completionTokens === undefined &&
    out.totalTokens === undefined
  ) {
    return undefined;
  }
  return out;
}

// Re-exported for type narrowing in consumers that import here.
export type { StreamError, Done, TextDelta };
