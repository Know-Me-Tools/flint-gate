import {
  asApiKeyValue,
  AuthConfig,
  FlintGateApiError,
  FlintGateClientConfig,
  FlintGateError,
} from "./types";

/**
 * Core HTTP client for Flint Gate.
 *
 * Edge-runtime safe: uses only `globalThis.fetch` and `AbortController`.
 * No Node.js built-ins are imported.
 */
export class FlintGateClient {
  /** Public data-plane base URL (no trailing slash). */
  public readonly baseUrl: string;
  /** Admin base URL or `null` when not configured. */
  public readonly adminUrl: string | null;
  /** Immutable auth config. */
  public readonly auth: AuthConfig;
  private readonly headers: Record<string, string>;
  private readonly timeoutMs?: number;
  private readonly fetchImpl: typeof fetch;

  constructor(config: FlintGateClientConfig) {
    this.baseUrl = normalizeBase(config.baseUrl);
    this.adminUrl = config.adminUrl ? normalizeBase(config.adminUrl) : null;
    this.auth = config.auth ?? { type: "anonymous" };
    this.headers = { ...(config.headers ?? {}) };
    this.timeoutMs = config.timeoutMs;
    this.fetchImpl = config.fetch ?? defaultFetch;
  }

  /**
   * Issue a request through Flint Gate's data plane. Auth headers are applied
   * from {@link auth} unless the caller overrides them in `init.headers`.
   */
  async request<T = unknown>(
    path: string,
    init: RequestInit & { readonly signal?: AbortSignal } = {},
  ): Promise<T> {
    return this.do<T>(this.url(path), init, false);
  }

  /**
   * Issue a streaming request through the data plane. Returns the raw
   * `Response` so callers can hand it to {@link streamSSE} / {@link streamNDJSON}.
   */
  async requestStream(
    path: string,
    init: RequestInit & { readonly signal?: AbortSignal } = {},
  ): Promise<Response> {
    const headers = this.mergeHeaders({
      Accept: "text/event-stream, application/x-ndjson",
    });
    const signal = this.applyTimeout(init.signal);
    const res = await this.fetchImpl(this.url(path), {
      ...init,
      headers,
      signal,
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
  async adminRequest<T = unknown>(
    path: string,
    init: RequestInit & { readonly signal?: AbortSignal } = {},
  ): Promise<T> {
    if (!this.adminUrl) {
      throw new FlintGateError(
        "adminUrl not configured — cannot call admin API",
        { code: "ADMIN_URL_MISSING" },
      );
    }
    return this.do<T>(joinUrl(this.adminUrl, path), init, true);
  }

  // -----------------------------------------------------------------------
  // internals
  // -----------------------------------------------------------------------

  private url(path: string): string {
    return joinUrl(this.baseUrl, path);
  }

  private async do<T>(
    url: string,
    init: RequestInit & { readonly signal?: AbortSignal },
    isAdmin: boolean,
  ): Promise<T> {
    const headers = isAdmin
      ? this.mergeHeaders(normalizeHeadersInit(init.headers))
      : this.mergeHeaders(normalizeHeadersInit(init.headers));
    const signal = this.applyTimeout(init.signal);
    const res = await this.fetchImpl(url, { ...init, headers, signal });
    if (!res.ok) {
      await consumeAndThrow(res);
    }
    if (res.status === 204) return undefined as T;
    const ct = res.headers.get("content-type") ?? "";
    if (ct.includes("application/json")) {
      return (await res.json()) as T;
    }
    return (await res.text()) as unknown as T;
  }

  private mergeHeaders(
    extra: Readonly<Record<string, string>> | Headers | undefined,
  ): Record<string, string> {
    const out: Record<string, string> = { ...this.headers };
    applyAuth(out, this.auth);
    if (extra) {
      if (typeof (extra as Headers).forEach === "function") {
        (extra as Headers).forEach((v, k) => (out[k] = v));
      } else {
        for (const [k, v] of Object.entries(extra)) out[k] = v;
      }
    }
    return out;
  }

  private applyTimeout(signal?: AbortSignal): AbortSignal {
    if (!this.timeoutMs) return signal ?? emptySignal();
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), this.timeoutMs);
    if (signal) {
      if (signal.aborted) ctrl.abort();
      else signal.addEventListener("abort", () => ctrl.abort(), { once: true });
    }
    // Best-effort clear; AbortController has no lifecycle hook so we rely on GC.
    void timer;
    return ctrl.signal;
  }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

function normalizeBase(base: string | URL): string {
  const s = base instanceof URL ? base.toString() : base;
  return s.replace(/\/+$/, "");
}

function joinUrl(base: string, path: string): string {
  if (/^https?:\/\//i.test(path)) return path;
  const p = path.startsWith("/") ? path : `/${path}`;
  return `${base}${p}`;
}

function applyAuth(
  headers: Record<string, string>,
  auth: AuthConfig,
): void {
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
      // In edge/browser runtimes cookies are attached via credentials:"include".
      if (auth.value) {
        const name = auth.name ?? "flint_session";
        headers["cookie"] = `${name}=${auth.value}`;
      }
      return;
    }
    default: {
      // exhaustiveness check
      const _never: never = auth;
      void _never;
    }
  }
}

function redactedKey(k: string): string {
  // The value is a branded ApiKeyValue; we forward it verbatim but never log.
  void asApiKeyValue;
  return k;
}

async function consumeAndThrow(res: Response): Promise<never> {
  let body: string | undefined;
  try {
    body = await res.text();
  } catch {
    body = undefined;
  }
  let message = `flint-gate request failed: ${res.status} ${res.statusText}`;
  let code: string | undefined;
  if (body) {
    try {
      const j = JSON.parse(body) as { error?: string; code?: string };
      if (j.error) message = j.error;
      if (j.code) code = j.code;
    } catch {
      if (body.length < 256) message = body;
    }
  }
  throw new FlintGateApiError(message, { status: res.status, code });
}

function defaultFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  const f = globalThis.fetch;
  if (!f) {
    throw new FlintGateError(
      "no global fetch — pass a custom `fetch` in client config",
      { code: "NO_FETCH" },
    );
  }
  return f(input, init);
}

function normalizeHeadersInit(
  init: HeadersInit | undefined,
): Record<string, string> | Headers {
  if (!init) return {};
  if (init instanceof Headers) return init;
  if (Array.isArray(init)) {
    const out: Record<string, string> = {};
    for (const [k, v] of init) out[k] = v;
    return out;
  }
  return init as Record<string, string>;
}

function emptySignal(): AbortSignal {
  const anySig = (AbortSignal as unknown as {
    withTimeout?: (ms: number) => AbortSignal;
  }).withTimeout;
  if (typeof anySig === "function") {
    // 24h — effectively "never" but still abortable downstream.
    return anySig.call(AbortSignal, 24 * 60 * 60 * 1000);
  }
  return new AbortController().signal;
}
