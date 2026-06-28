/**
 * @know-me/flint-gate — type definitions.
 *
 * These types model the wire format produced by Flint Gate when proxying
 * streaming LLM traffic and the admin API surface exposed on :4457.
 *
 * All stream payload variants are discriminated unions so consumers can
 * exhaustively pattern-match in a `switch (event.type)` block.
 */
/** Opaque branded type for a Flint Gate route ID (non-empty string at runtime). */
type RouteId = string & {
    readonly __brand: "RouteId";
};
/** Opaque branded type for a Flint Gate site ID. */
type SiteId = string & {
    readonly __brand: "SiteId";
};
/** Opaque branded type for a Flint Gate API key (never logged in full). */
type ApiKeyValue = string & {
    readonly __brand: "ApiKeyValue";
};
/** Branded helper — runs at module boundaries only; not exported to end users. */
declare function asRouteId(s: string): RouteId;
declare function asSiteId(s: string): SiteId;
declare function asApiKeyValue(s: string): ApiKeyValue;
/** Selects how the SDK authenticates against Flint Gate on proxied requests. */
type AuthConfig = {
    readonly type: "anonymous";
} | {
    readonly type: "apiKey";
    /** Raw key value. The SDK redacts it from logs and console output. */
    readonly key: ApiKeyValue;
    /** Header name carrying the key. Defaults to `x-api-key`. */
    readonly header?: string;
} | {
    readonly type: "bearer";
    /** Raw bearer token (JWT or opaque). */
    readonly token: string;
    /** Header name. Defaults to `authorization`. */
    readonly header?: string;
} | {
    readonly type: "cookie";
    /** Cookie name. Defaults to `flint_session`. */
    readonly name?: string;
    /**
     * For browser/edge runtimes this is optional — the runtime attaches
     * cookies automatically when `credentials: "include"` is set. For
     * server-side use, supply the raw cookie value.
     */
    readonly value?: string;
};
/** Incremental text fragment from a TEXT_MESSAGE_CONTENT-style delta. */
interface TextDelta {
    readonly type: "text-delta";
    /** Concatenate this in arrival order to reconstruct the full message. */
    readonly text: string;
    /** Optional message id when the upstream protocol carries one. */
    readonly messageId?: string;
    /** 0-based index of this delta within its message, when known. */
    readonly index?: number;
}
/** Structured tool invocation surfaced mid-stream. */
interface ToolCall {
    readonly type: "tool-call";
    /** Stable identifier for correlating a later tool-result event. */
    readonly id: string;
    /** Tool name as registered upstream. */
    readonly name: string;
    /** Arguments object. Untyped on purpose — varies per tool. */
    readonly args: Readonly<Record<string, unknown>>;
}
/** Terminal success marker. Always emitted exactly once per stream. */
interface Done {
    readonly type: "done";
    /** Token counts when the upstream protocol reports them. */
    readonly usage?: TokenUsage;
    /** Optional request id assigned by Flint Gate. */
    readonly requestId?: string;
}
/** Recoverable or terminal error surfaced on the stream. */
interface StreamError {
    readonly type: "error";
    readonly message: string;
    /** Machine-readable error code when available. */
    readonly code?: string;
    /** HTTP status from the upstream response, if this is a terminal error. */
    readonly status?: number;
}
/** Discriminated union of every event `streamSSE` / `streamNDJSON` may yield. */
type StreamEvent = TextDelta | ToolCall | Done | StreamError;
/** Token accounting attached to {@link Done}. */
interface TokenUsage {
    readonly promptTokens?: number;
    readonly completionTokens?: number;
    readonly totalTokens?: number;
}
/** Raw SSE `data:` payload after JSON parse, before normalization to {@link StreamEvent}. */
interface RawFrame {
    readonly type: string;
    readonly [k: string]: unknown;
}
/** Path/method matcher for a route. Glob patterns compile to regex server-side. */
interface RouteMatch {
    /** Glob path, e.g. `/api/**`. */
    readonly path: string;
    /** HTTP methods. Empty/undefined means all methods. */
    readonly methods?: readonly string[];
    /** Optional host glob. */
    readonly host?: string;
}
/** Named auth provider reference on a route. */
interface AuthProviderRef {
    /** `kratos` | `jwt` | `api_key` | `anonymous` | a custom provider id. */
    readonly type: string;
    /** Provider id from the `auth_providers` map. */
    readonly id?: string;
}
/** Header injection hook config. */
interface InjectHeadersHook {
    readonly type: "inject_headers";
    /** Map of header name → template string. */
    readonly headers: Readonly<Record<string, string>>;
}
/** Body transform hook config (set JSON fields). */
interface BodyTransformHook {
    readonly type: "body_transform";
    /** JSON-pointer-ish field path → template. */
    readonly fields: Readonly<Record<string, string>>;
}
/** Outbound JWT minting hook config. */
interface MintJwtHook {
    readonly type: "mint_jwt";
    /** Signing key id from `jwt_signing_keys`. */
    readonly keyId: string;
    /** Claims template map. */
    readonly claims: Readonly<Record<string, unknown>>;
    /** TTL in seconds. */
    readonly ttlSeconds?: number;
}
type PreRequestHook = InjectHeadersHook | BodyTransformHook | MintJwtHook;
/** Full route definition as stored in `gate_routes`. */
interface RouteConfig {
    readonly id: RouteId;
    readonly site: SiteId;
    readonly match: RouteMatch;
    /** Absolute upstream URL. */
    readonly upstream: string;
    /** Higher priority wins on multi-match. Default 0. */
    readonly priority?: number;
    readonly enabled?: boolean;
    /** Per-route auth provider override. Falls back to the site default. */
    readonly auth?: AuthProviderRef;
    /** Ordered pre-request hook chain. */
    readonly hooks?: ReadonlyArray<PreRequestHook>;
    /** When true, mutate via DB; otherwise YAML-sourced. Read-only from the SDK. */
    readonly overrideYaml?: boolean;
}
/** Body for POST/PUT /routes — `id` may be omitted on POST. */
type CreateRouteInput = Omit<RouteConfig, "id"> & Partial<Pick<RouteConfig, "id">>;
/** API key record as returned by GET /api-keys. Hashed value never exposed. */
interface ApiKey {
    readonly id: string;
    /** Stable client identifier usable in templates as `{{ api_key.client_id }}`. */
    readonly clientId: string;
    /** Granted scope list. */
    readonly scopes: readonly string[];
    /** ISO-8601 creation timestamp. */
    readonly createdAt: string;
    /** ISO-8601 optional revocation timestamp. */
    readonly revokedAt?: string;
    /** Human label. */
    readonly description?: string;
}
/** Input for creating a new API key. The plaintext key is returned once. */
interface CreateApiKeyInput {
    readonly clientId: string;
    readonly scopes?: readonly string[];
    readonly description?: string;
    /** Optional explicit key value. If omitted the server generates one. */
    readonly key?: string;
}
/** Response from POST /api-keys. `key` is the plaintext and is only seen here. */
interface CreateApiKeyResponse {
    readonly apiKey: ApiKey;
    /** Plaintext key — store this immediately; the server only keeps the hash. */
    readonly key: ApiKeyValue;
}
interface HealthStatus {
    readonly status: "ok";
    readonly version?: string;
    readonly uptimeSeconds?: number;
}
interface ReadyStatus {
    readonly status: "ready" | "degraded" | "not-ready";
    readonly checks: Readonly<Record<string, {
        ok: boolean;
        detail?: string;
    }>>;
}
/** Base class for all SDK errors. Carries optional HTTP status and code. */
declare class FlintGateError extends Error {
    readonly status?: number;
    readonly code?: string;
    constructor(message: string, opts?: {
        status?: number;
        code?: string;
    });
}
/** Non-2xx response from the admin API. */
declare class FlintGateApiError extends FlintGateError {
}
/** The stream ended unexpectedly (no `done` frame). */
declare class StreamClosedError extends FlintGateError {
    constructor(message?: string);
}
/** The stream surface an `error` frame. */
declare class StreamProtocolError extends FlintGateError {
    constructor(message: string, code?: string, status?: number);
}
/** Constructor options for {@link FlintGateClient}. */
interface FlintGateClientConfig {
    /**
     * Base URL of the Flint Gate proxy (the public/data port, e.g.
     * `https://gate.example.com`). Must be absolute. No trailing slash.
     */
    readonly baseUrl: string | URL;
    /**
     * Admin base URL (the admin port, e.g. `http://gate-internal:4457`).
     * Optional — only needed for admin API methods. Should never be the same
     * host as a public-facing deployment.
     */
    readonly adminUrl?: string | URL;
    /** Auth strategy applied to proxied data-plane requests. */
    readonly auth?: AuthConfig;
    /** Extra default headers merged onto every request. */
    readonly headers?: Readonly<Record<string, string>>;
    /** Optional default AbortSignal-shortening timeout in ms. */
    readonly timeoutMs?: number;
    /** Custom fetch implementation. Defaults to `globalThis.fetch`. */
    readonly fetch?: typeof fetch;
}

/**
 * Core HTTP client for Flint Gate.
 *
 * Edge-runtime safe: uses only `globalThis.fetch` and `AbortController`.
 * No Node.js built-ins are imported.
 */
declare class FlintGateClient {
    /** Public data-plane base URL (no trailing slash). */
    readonly baseUrl: string;
    /** Admin base URL or `null` when not configured. */
    readonly adminUrl: string | null;
    /** Immutable auth config. */
    readonly auth: AuthConfig;
    private readonly headers;
    private readonly timeoutMs?;
    private readonly fetchImpl;
    constructor(config: FlintGateClientConfig);
    /**
     * Issue a request through Flint Gate's data plane. Auth headers are applied
     * from {@link auth} unless the caller overrides them in `init.headers`.
     */
    request<T = unknown>(path: string, init?: RequestInit & {
        readonly signal?: AbortSignal;
    }): Promise<T>;
    /**
     * Issue a streaming request through the data plane. Returns the raw
     * `Response` so callers can hand it to {@link streamSSE} / {@link streamNDJSON}.
     */
    requestStream(path: string, init?: RequestInit & {
        readonly signal?: AbortSignal;
    }): Promise<Response>;
    /**
     * Issue a request against the admin plane. Throws if no `adminUrl` was set.
     * Admin requests never attach {@link auth} — the admin port should be
     * network-isolated and may use mTLS or loopback-only binding instead.
     */
    adminRequest<T = unknown>(path: string, init?: RequestInit & {
        readonly signal?: AbortSignal;
    }): Promise<T>;
    private url;
    private do;
    private mergeHeaders;
    private applyTimeout;
}

/**
 * Admin API surface for Flint Gate.
 *
 * All methods hit the admin port (default :4457) via
 * {@link FlintGateClient.adminRequest}. The admin port must be
 * network-isolated from the public internet.
 */
declare class FlintGateAdmin {
    private readonly client;
    constructor(client: FlintGateClient);
    /** Liveness probe — always returns 200 if the process is up. */
    getHealth(signal?: AbortSignal): Promise<HealthStatus>;
    /** Readiness probe — checks DB connectivity. */
    getReady(signal?: AbortSignal): Promise<ReadyStatus>;
    /** List all enabled routes. */
    getRoutes(signal?: AbortSignal): Promise<RouteConfig[]>;
    /** Get a single route by id. */
    getRoute(id: RouteId | string, signal?: AbortSignal): Promise<RouteConfig>;
    /** Create or upsert a route. Returns the stored record. */
    createRoute(input: CreateRouteInput, signal?: AbortSignal): Promise<RouteConfig>;
    /** Update an existing route by id. */
    updateRoute(id: RouteId | string, input: CreateRouteInput, signal?: AbortSignal): Promise<RouteConfig>;
    /** Delete a route by id. */
    deleteRoute(id: RouteId | string, signal?: AbortSignal): Promise<void>;
    /** List all API keys (hashed values are never returned). */
    getApiKeys(signal?: AbortSignal): Promise<ApiKey[]>;
    /**
     * Create a new API key. The plaintext `key` is returned exactly once —
     * store it immediately; the server only retains the SHA-256 hash.
     */
    createApiKey(input: CreateApiKeyInput, signal?: AbortSignal): Promise<CreateApiKeyResponse>;
    /** Revoke an API key by id. */
    revokeApiKey(id: string, signal?: AbortSignal): Promise<void>;
}

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
declare function streamSSE(source: Response | ReadableStream<Uint8Array>, signal?: AbortSignal): AsyncGenerator<StreamEvent, void, unknown>;
/**
 * Consume an NDJSON (newline-delimited JSON) stream as an async iterable of
 * typed {@link StreamEvent} values. Each non-empty line is parsed as JSON
 * and normalized through the same pipeline as {@link streamSSE}.
 */
declare function streamNDJSON(source: Response | ReadableStream<Uint8Array>, signal?: AbortSignal): AsyncGenerator<StreamEvent, void, unknown>;

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
interface FlintGateWSOptions {
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
declare function streamWS(url: string | URL, opts?: FlintGateWSOptions & {
    readonly channel: string;
}): AsyncGenerator<StreamEvent, void, unknown>;

/**
 * Framework adapters for verifying that a request was authenticated by
 * Flint Gate. Two shapes are provided:
 *
 *   - {@link createFlintGateMiddleware} — Next.js Edge / Node middleware
 *   - {@link expressFlintGateAdapter}   — Express 4/5 middleware
 *
 * Both adapters verify the upstream-injected `x-flint-authenticated` header
 * and (optionally) parse `x-flint-identity` JSON. Configure Flint Gate's
 * `inject_headers` hook to set these on every protected route.
 */
/** Identity JSON injected by Flint Gate's `inject_headers` template engine. */
interface FlintIdentity {
    readonly subject?: string;
    readonly sessionId?: string;
    readonly clientId?: string;
    readonly scopes?: readonly string[];
    readonly [k: string]: unknown;
}
interface FlintGateMiddlewareConfig {
    /**
     * Header that signals Flint Gate already authenticated this request.
     * Default: `x-flint-authenticated`.
     */
    readonly authenticatedHeader?: string;
    /** Header carrying the identity JSON. Default: `x-flint-identity`. */
    readonly identityHeader?: string;
    /**
     * Shared secret Flint Gate injects into `authenticatedHeader` value.
     * When set, requests whose header does not match are rejected with 401.
     * When omitted, only the header's presence is checked.
     */
    readonly sharedSecret?: string;
    /**
     * Required scope(s). The request's identity (from `identityHeader`)
     * must include all listed scopes, otherwise 403.
     */
    readonly requiredScopes?: readonly string[];
    /** Paths that skip verification (e.g. `/_next/static`). Glob is not supported — exact prefix match. */
    readonly bypassPrefixes?: readonly string[];
}
/**
 * Create a Next.js (Edge or Node) middleware function.
 *
 * Usage in `middleware.ts`:
 * ```ts
 * export const middleware = createFlintGateMiddleware({ sharedSecret: process.env.FLINT_SECRET });
 * export const config = { matcher: ["/api/:path*"] };
 * ```
 */
declare function createFlintGateMiddleware(cfg?: FlintGateMiddlewareConfig): (req: {
    headers: Headers | Record<string, string | string[] | undefined>;
    nextUrl?: {
        pathname?: string;
    };
}) => Response | {
    ok: true;
    identity: FlintIdentity | null;
};
/** Minimal Express-style handler signature. */
type ExpressReq = {
    headers: Record<string, string | string[] | undefined>;
    path?: string;
    url?: string;
};
type ExpressRes = {
    status(code: number): unknown;
    json(body: unknown): unknown;
};
type ExpressNext = (err?: unknown) => void;
/**
 * Express 4/5 adapter. Mount with `app.use("/api", expressFlintGateAdapter(cfg))`.
 */
declare function expressFlintGateAdapter(cfg?: FlintGateMiddlewareConfig): (req: ExpressReq, res: ExpressRes, next: ExpressNext) => void;
/**
 * Helper for route handlers: read the identity Flint Gate injected, or throw
 * a typed error if absent. Pair with middleware that has already verified
 * the `authenticatedHeader`.
 */
declare function readFlintIdentity(headers: Headers | Record<string, string | string[] | undefined>, identityHeader?: string): FlintIdentity;
/**
 * Build an {@link AuthConfig} for forwarding an inbound request's identity
 * downstream through Flint Gate (e.g. service-to-service). The plaintext key
 * is preserved verbatim and never logged.
 */
declare function forwardApiKey(headerValue: string, headerName?: string): AuthConfig;

export { type ApiKey, type ApiKeyValue, type AuthConfig, type AuthProviderRef, type BodyTransformHook, type CreateApiKeyInput, type CreateApiKeyResponse, type CreateRouteInput, type Done, FlintGateAdmin, FlintGateApiError, FlintGateClient, type FlintGateClientConfig, FlintGateError, type FlintGateMiddlewareConfig, type FlintGateWSOptions, type FlintIdentity, type HealthStatus, type InjectHeadersHook, type MintJwtHook, type PreRequestHook, type RawFrame, type ReadyStatus, type RouteConfig, type RouteId, type RouteMatch, type SiteId, StreamClosedError, type StreamError, type StreamEvent, StreamProtocolError, type TextDelta, type TokenUsage, type ToolCall, asApiKeyValue, asRouteId, asSiteId, createFlintGateMiddleware, expressFlintGateAdapter, forwardApiKey, readFlintIdentity, streamNDJSON, streamSSE, streamWS };
