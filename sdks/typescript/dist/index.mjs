// @know-me/flint-gate — Edge-runtime safe client (no Node.js built-ins)

// src/types.ts
function asRouteId(s) {
  return s;
}
function asSiteId(s) {
  return s;
}
function asApiKeyValue(s) {
  return s;
}
var FlintGateError = class extends Error {
  status;
  code;
  constructor(message, opts) {
    super(message);
    this.name = "FlintGateError";
    this.status = opts?.status;
    this.code = opts?.code;
  }
};
var FlintGateApiError = class extends FlintGateError {
};
var StreamClosedError = class extends FlintGateError {
  constructor(message = "stream closed before terminal event") {
    super(message, { code: "STREAM_CLOSED" });
    this.name = "StreamClosedError";
  }
};
var StreamProtocolError = class extends FlintGateError {
  constructor(message, code, status) {
    super(message, { code, status });
    this.name = "StreamProtocolError";
  }
};

// src/client.ts
var FlintGateClient = class {
  /** Public data-plane base URL (no trailing slash). */
  baseUrl;
  /** Admin base URL or `null` when not configured. */
  adminUrl;
  /** Immutable auth config. */
  auth;
  headers;
  timeoutMs;
  fetchImpl;
  constructor(config) {
    this.baseUrl = normalizeBase(config.baseUrl);
    this.adminUrl = config.adminUrl ? normalizeBase(config.adminUrl) : null;
    this.auth = config.auth ?? { type: "anonymous" };
    this.headers = { ...config.headers ?? {} };
    this.timeoutMs = config.timeoutMs;
    this.fetchImpl = config.fetch ?? defaultFetch;
  }
  /**
   * Issue a request through Flint Gate's data plane. Auth headers are applied
   * from {@link auth} unless the caller overrides them in `init.headers`.
   */
  async request(path, init = {}) {
    return this.do(this.url(path), init, false);
  }
  /**
   * Issue a streaming request through the data plane. Returns the raw
   * `Response` so callers can hand it to {@link streamSSE} / {@link streamNDJSON}.
   */
  async requestStream(path, init = {}) {
    const headers = this.mergeHeaders({
      Accept: "text/event-stream, application/x-ndjson"
    });
    const signal = this.applyTimeout(init.signal);
    const res = await this.fetchImpl(this.url(path), {
      ...init,
      headers,
      signal
    });
    if (!res.ok) {
      await consumeAndThrow(res);
    }
    if (!res.body) {
      throw new FlintGateError("response has no body");
    }
    return res;
  }
  /**
   * Issue a request against the admin plane. Throws if no `adminUrl` was set.
   * Admin requests never attach {@link auth} — the admin port should be
   * network-isolated and may use mTLS or loopback-only binding instead.
   */
  async adminRequest(path, init = {}) {
    if (!this.adminUrl) {
      throw new FlintGateError(
        "adminUrl not configured \u2014 cannot call admin API",
        { code: "ADMIN_URL_MISSING" }
      );
    }
    return this.do(joinUrl(this.adminUrl, path), init, true);
  }
  // -----------------------------------------------------------------------
  // internals
  // -----------------------------------------------------------------------
  url(path) {
    return joinUrl(this.baseUrl, path);
  }
  async do(url, init, isAdmin) {
    const headers = isAdmin ? this.mergeHeaders(normalizeHeadersInit(init.headers)) : this.mergeHeaders(normalizeHeadersInit(init.headers));
    const signal = this.applyTimeout(init.signal);
    const res = await this.fetchImpl(url, { ...init, headers, signal });
    if (!res.ok) {
      await consumeAndThrow(res);
    }
    if (res.status === 204) return void 0;
    const ct = res.headers.get("content-type") ?? "";
    if (ct.includes("application/json")) {
      return await res.json();
    }
    return await res.text();
  }
  mergeHeaders(extra) {
    const out = { ...this.headers };
    applyAuth(out, this.auth);
    if (extra) {
      if (typeof extra.forEach === "function") {
        extra.forEach((v, k) => out[k] = v);
      } else {
        for (const [k, v] of Object.entries(extra)) out[k] = v;
      }
    }
    return out;
  }
  applyTimeout(signal) {
    if (!this.timeoutMs) return signal ?? emptySignal();
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), this.timeoutMs);
    if (signal) {
      if (signal.aborted) ctrl.abort();
      else signal.addEventListener("abort", () => ctrl.abort(), { once: true });
    }
    void timer;
    return ctrl.signal;
  }
};
function normalizeBase(base) {
  const s = base instanceof URL ? base.toString() : base;
  return s.replace(/\/+$/, "");
}
function joinUrl(base, path) {
  if (/^https?:\/\//i.test(path)) return path;
  const p = path.startsWith("/") ? path : `/${path}`;
  return `${base}${p}`;
}
function applyAuth(headers, auth) {
  switch (auth.type) {
    case "anonymous":
      return;
    case "apiKey": {
      const h = auth.header ?? "x-api-key";
      headers[h] = redactedKey(auth.key);
      return;
    }
    case "bearer": {
      const h = auth.header ?? "authorization";
      headers[h] = `Bearer ${auth.token}`;
      return;
    }
    case "cookie": {
      if (auth.value) {
        const name = auth.name ?? "flint_session";
        headers["cookie"] = `${name}=${auth.value}`;
      }
      return;
    }
    default: {
      const _never = auth;
      void _never;
    }
  }
}
function redactedKey(k) {
  void asApiKeyValue;
  return k;
}
async function consumeAndThrow(res) {
  let body;
  try {
    body = await res.text();
  } catch {
    body = void 0;
  }
  let message = `flint-gate request failed: ${res.status} ${res.statusText}`;
  let code;
  if (body) {
    try {
      const j = JSON.parse(body);
      if (j.error) message = j.error;
      if (j.code) code = j.code;
    } catch {
      if (body.length < 256) message = body;
    }
  }
  throw new FlintGateApiError(message, { status: res.status, code });
}
function defaultFetch(input, init) {
  const f = globalThis.fetch;
  if (!f) {
    throw new FlintGateError(
      "no global fetch \u2014 pass a custom `fetch` in client config",
      { code: "NO_FETCH" }
    );
  }
  return f(input, init);
}
function normalizeHeadersInit(init) {
  if (!init) return {};
  if (init instanceof Headers) return init;
  if (Array.isArray(init)) {
    const out = {};
    for (const [k, v] of init) out[k] = v;
    return out;
  }
  return init;
}
function emptySignal() {
  const anySig = AbortSignal.withTimeout;
  if (typeof anySig === "function") {
    return anySig.call(AbortSignal, 24 * 60 * 60 * 1e3);
  }
  return new AbortController().signal;
}

// src/admin.ts
var FlintGateAdmin = class {
  constructor(client) {
    this.client = client;
  }
  client;
  // ----------------------------------------------------------------- health
  /** Liveness probe — always returns 200 if the process is up. */
  async getHealth(signal) {
    return this.client.adminRequest("/health", { signal });
  }
  /** Readiness probe — checks DB connectivity. */
  async getReady(signal) {
    return this.client.adminRequest("/ready", { signal });
  }
  // ----------------------------------------------------------------- routes
  /** List all enabled routes. */
  async getRoutes(signal) {
    const rows = await this.client.adminRequest("/routes", { signal });
    return rows.map(normalizeRoute);
  }
  /** Get a single route by id. */
  async getRoute(id, signal) {
    const row = await this.client.adminRequest(
      `/routes/${encodeURIComponent(id)}`,
      { signal }
    );
    return normalizeRoute(row);
  }
  /** Create or upsert a route. Returns the stored record. */
  async createRoute(input, signal) {
    const row = await this.client.adminRequest("/routes", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(serializeRoute(input)),
      signal
    });
    return normalizeRoute(row);
  }
  /** Update an existing route by id. */
  async updateRoute(id, input, signal) {
    const row = await this.client.adminRequest(
      `/routes/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(serializeRoute({ ...input, id: input.id ?? asRouteId(id) })),
        signal
      }
    );
    return normalizeRoute(row);
  }
  /** Delete a route by id. */
  async deleteRoute(id, signal) {
    await this.client.adminRequest(
      `/routes/${encodeURIComponent(id)}`,
      { method: "DELETE", signal }
    );
  }
  // --------------------------------------------------------------- api keys
  /** List all API keys (hashed values are never returned). */
  async getApiKeys(signal) {
    return this.client.adminRequest("/api-keys", { signal });
  }
  /**
   * Create a new API key. The plaintext `key` is returned exactly once —
   * store it immediately; the server only retains the SHA-256 hash.
   */
  async createApiKey(input, signal) {
    const res = await this.client.adminRequest("/api-keys", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        clientId: input.clientId,
        scopes: input.scopes ?? [],
        description: input.description,
        key: input.key
      }),
      signal
    });
    return {
      apiKey: res.apiKey,
      key: asApiKeyValue(res.key)
    };
  }
  /** Revoke an API key by id. */
  async revokeApiKey(id, signal) {
    await this.client.adminRequest(`/api-keys/${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal
    });
  }
};
function normalizeRoute(row) {
  const r = row;
  const id = asRouteId(String(r.id));
  const site = asSiteId(String(r.site));
  const match = r.match;
  return {
    id,
    site,
    match,
    upstream: String(r.upstream),
    priority: typeof r.priority === "number" ? r.priority : 0,
    enabled: typeof r.enabled === "boolean" ? r.enabled : true,
    auth: r.auth,
    hooks: r.hooks,
    overrideYaml: typeof r.overrideYaml === "boolean" ? r.overrideYaml : void 0
  };
}
function serializeRoute(input) {
  const out = {
    id: input.id,
    site: input.site,
    match: input.match,
    upstream: input.upstream,
    priority: input.priority ?? 0,
    enabled: input.enabled ?? true
  };
  if (input.auth) out.auth = input.auth;
  if (input.hooks) out.hooks = input.hooks;
  if (input.overrideYaml !== void 0) out.overrideYaml = input.overrideYaml;
  return out;
}

// src/stream.ts
async function* streamSSE(source, signal) {
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
    if (line.startsWith(":")) continue;
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    if (field === "event") {
      event = value;
    } else if (field === "data") {
      dataBuffer = dataBuffer === "" ? value : `${dataBuffer}
${value}`;
    }
  }
  if (dataBuffer !== "" && !signal?.aborted) {
    const frame = decodeFrame(dataBuffer, event);
    if (frame) {
      const evt = normalizeFrame(frame, event);
      if (evt) yield evt;
    }
  }
}
async function* streamNDJSON(source, signal) {
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
function toStream(source) {
  if (source instanceof ReadableStream) return source;
  return source.body ?? new ReadableStream({ start(c) {
    c.close();
  } });
}
async function* readLines(stream, signal) {
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
      let nl;
      while ((nl = indexOfNewline(buffer)) !== -1) {
        const lineEnd = nl;
        const ch = buffer.charCodeAt(nl);
        let consumeTo = nl + 1;
        if (ch === 13 && buffer.charCodeAt(nl + 1) === 10) {
        } else if (ch === 13) {
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
    }
  }
}
function indexOfNewline(s) {
  const lf = s.indexOf("\n");
  const cr = s.indexOf("\r");
  if (lf === -1) return cr;
  if (cr === -1) return lf;
  return Math.min(lf, cr);
}
function decodeFrame(data, event) {
  const trimmed = data.trim();
  if (trimmed === "") return null;
  if (trimmed === "[DONE]") {
    return { type: "done" };
  }
  try {
    return JSON.parse(trimmed);
  } catch {
    if (event === "message") {
      return { type: "text-delta", text: data };
    }
    return { type: "error", message: `unparseable frame: ${trimmed.slice(0, 80)}` };
  }
}
function normalizeFrame(frame, sseEvent) {
  const t = frame.type ?? sseEvent;
  if (t === "text-delta" || t === "TEXT_MESSAGE_CONTENT" || t === "delta") {
    const text = frame.text ?? frame.textDelta ?? frame.delta ?? frame.content;
    if (typeof text !== "string") return null;
    const out = {
      type: "text-delta",
      text
    };
    if (typeof frame.messageId === "string") out.messageId = frame.messageId;
    if (typeof frame.index === "number") out.index = frame.index;
    return out;
  }
  if (t === "tool-call" || t === "TOOL_CALL" || t === "function_call") {
    const id = frame.id ?? frame.toolCallId;
    const name = frame.name ?? frame.toolName;
    if (!id || !name) return null;
    const args = frame.args ?? frame.arguments ?? frame.input ?? {};
    const out = { type: "tool-call", id, name, args };
    return out;
  }
  if (t === "done" || t === "finish" || t === "DONE" || t === "[DONE]") {
    const usage = readUsage(frame);
    const out = { type: "done" };
    if (usage) out.usage = usage;
    if (typeof frame.requestId === "string") out.requestId = frame.requestId;
    return out;
  }
  if (t === "error" || t === "ERROR") {
    const message = frame.message ?? frame.error ?? "unknown stream error";
    const code = typeof frame.code === "string" ? frame.code : void 0;
    const status = typeof frame.status === "number" ? frame.status : void 0;
    throw new StreamProtocolError(message, code, status);
  }
  return null;
}
function readUsage(frame) {
  const metadata = frame.metadata;
  const u = frame.usage ?? metadata?.usage;
  if (!u) return void 0;
  const out = {};
  if (typeof u.promptTokens === "number") out.promptTokens = u.promptTokens;
  if (typeof u.completionTokens === "number") out.completionTokens = u.completionTokens;
  if (typeof u.totalTokens === "number") out.totalTokens = u.totalTokens;
  if (out.promptTokens === void 0 && out.completionTokens === void 0 && out.totalTokens === void 0) {
    return void 0;
  }
  return out;
}

// src/ws.ts
function streamWS(url, opts = { channel: "" }) {
  return streamWSImpl(url, opts);
}
async function* streamWSImpl(url, opts) {
  const WS = opts.WebSocketCtor ?? globalThis.WebSocket;
  if (!WS) {
    throw new FlintGateError(
      "WebSocket is not available in this runtime \u2014 pass WebSocketCtor",
      { code: "NO_WEBSOCKET" }
    );
  }
  const finalUrl = applyAuthToUrl(url, opts.auth);
  const ws = new WS(finalUrl, opts.protocols, opts.headers);
  const queue = [];
  let resolveNext = null;
  let closed = false;
  let closeError = null;
  const push = (evt) => {
    if (resolveNext) {
      const r = resolveNext;
      resolveNext = null;
      r({ value: evt, done: false });
    } else {
      queue.push(evt);
    }
    if (evt.type === "done" || evt.type === "error") {
      close(1e3);
    }
  };
  const close = (code) => {
    if (closed) return;
    closed = true;
    try {
      ws.close(code ?? 1e3);
    } catch {
    }
    if (resolveNext) {
      const r = resolveNext;
      resolveNext = null;
      r(closeError ? { value: closeError, done: true } : { value: void 0, done: true });
    }
  };
  ws.onopen = () => {
    try {
      ws.send(
        JSON.stringify({ type: "subscribe", channel: opts.channel })
      );
    } catch (err) {
      closeError = new FlintGateError(
        `failed to send subscribe: ${err.message}`,
        { code: "WS_SEND_FAILED" }
      );
      close();
    }
  };
  ws.onmessage = (ev) => {
    const raw = typeof ev.data === "string" ? ev.data : "";
    if (!raw) return;
    let frame;
    try {
      frame = JSON.parse(raw);
    } catch {
      push({ type: "text-delta", text: raw });
      return;
    }
    const env = frame;
    if (env.type === "error") {
      push({
        type: "error",
        message: env.error ?? "unknown ws error",
        code: env.code
      });
      return;
    }
    if (env.type === "event" && env.data && typeof env.data === "object") {
      const inner = env.data;
      if (inner.type === "text-delta" || inner.type === "tool-call" || inner.type === "done" || inner.type === "error") {
        push(inner);
        return;
      }
      if (typeof env.data.text === "string") {
        push({ type: "text-delta", text: env.data.text });
        return;
      }
    }
    if (env.type === "ping") {
      try {
        ws.send(JSON.stringify({ type: "pong" }));
      } catch {
      }
      return;
    }
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
      r(closeError ? { value: closeError, done: true } : { value: void 0, done: true });
    }
  };
  if (opts.signal) {
    if (opts.signal.aborted) close();
    else opts.signal.addEventListener("abort", () => close(), { once: true });
  }
  while (true) {
    if (queue.length > 0) {
      const next = queue.shift();
      yield next;
      if (next.type === "done" || next.type === "error") return;
      continue;
    }
    if (closed) {
      if (closeError) throw closeError;
      return;
    }
    const result = await new Promise((resolve) => {
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
function applyAuthToUrl(url, auth) {
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

// src/middleware.ts
function headerGetter(headers) {
  if (typeof headers.get === "function") {
    return (name) => headers.get(name);
  }
  const h = headers;
  return (name) => {
    const v = h[name] ?? h[name.toLowerCase()];
    if (v == null) return null;
    return Array.isArray(v) ? v[0] ?? null : v;
  };
}
function parseIdentity(get, identityHeader) {
  const raw = get(identityHeader);
  if (!raw) return null;
  try {
    return JSON.parse(decodeURIComponent(raw));
  } catch {
    try {
      return JSON.parse(raw);
    } catch {
      return null;
    }
  }
}
function checkScopes(identity, required) {
  if (required.length === 0) return true;
  const have = new Set(identity?.scopes ?? []);
  for (const s of required) {
    if (!have.has(s)) return false;
  }
  return true;
}
function checkRequest(get, cfg) {
  const authHeader = cfg.authenticatedHeader ?? "x-flint-authenticated";
  const identityHeader = cfg.identityHeader ?? "x-flint-identity";
  const val = get(authHeader);
  if (val == null) {
    return { ok: false, status: 401, message: "missing flint-gate auth header" };
  }
  if (cfg.sharedSecret !== void 0 && val !== cfg.sharedSecret) {
    return { ok: false, status: 401, message: "invalid flint-gate secret" };
  }
  const identity = parseIdentity(get, identityHeader);
  if (!checkScopes(identity, cfg.requiredScopes ?? [])) {
    return { ok: false, status: 403, message: "insufficient scope" };
  }
  return { ok: true, identity };
}
function createFlintGateMiddleware(cfg = {}) {
  const bypass = cfg.bypassPrefixes ?? [];
  return (req) => {
    const path = req.nextUrl?.pathname ?? "/";
    for (const p of bypass) {
      if (path === p || path.startsWith(p)) {
        return { ok: true, identity: null };
      }
    }
    const get = headerGetter(req.headers);
    const result = checkRequest(get, cfg);
    if (result.ok) return result;
    return new Response(
      JSON.stringify({ error: result.message }),
      { status: result.status, headers: { "content-type": "application/json" } }
    );
  };
}
function expressFlintGateAdapter(cfg = {}) {
  const bypass = cfg.bypassPrefixes ?? [];
  return (req, res, next) => {
    const path = req.path ?? req.url ?? "/";
    for (const p of bypass) {
      if (path === p || path.startsWith(p)) {
        next();
        return;
      }
    }
    const get = headerGetter(req.headers);
    const result = checkRequest(get, cfg);
    if (result.ok) {
      req.flintIdentity = result.identity;
      next();
      return;
    }
    void res.status(result.status).json({ error: result.message });
  };
}
function readFlintIdentity(headers, identityHeader = "x-flint-identity") {
  const get = headerGetter(headers);
  const identity = parseIdentity(get, identityHeader);
  if (!identity) {
    throw new FlintGateError("flint-gate identity header missing or invalid", {
      code: "IDENTITY_MISSING",
      status: 401
    });
  }
  return identity;
}
function forwardApiKey(headerValue, headerName = "x-api-key") {
  return { type: "apiKey", key: asApiKeyValue(headerValue), header: headerName };
}
export {
  FlintGateAdmin,
  FlintGateApiError,
  FlintGateClient,
  FlintGateError,
  StreamClosedError,
  StreamProtocolError,
  asApiKeyValue,
  asRouteId,
  asSiteId,
  createFlintGateMiddleware,
  expressFlintGateAdapter,
  forwardApiKey,
  readFlintIdentity,
  streamNDJSON,
  streamSSE,
  streamWS
};
//# sourceMappingURL=index.mjs.map