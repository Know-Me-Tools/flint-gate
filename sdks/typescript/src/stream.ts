import {
  Done,
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
    }
    // id:/retry: are ignored — Flint Gate does not use them.
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
      const { value, done } = await reader.read();
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
