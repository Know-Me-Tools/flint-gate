/**
 * @know-me/flint-gate — type definitions.
 *
 * These types model the wire format produced by Flint Gate when proxying
 * streaming LLM traffic and the admin API surface exposed on :4457.
 *
 * All stream payload variants are discriminated unions so consumers can
 * exhaustively pattern-match in a `switch (event.type)` block.
 */

// ---------------------------------------------------------------------------
// Branded primitives
// ---------------------------------------------------------------------------

/** Opaque branded type for a Flint Gate route ID (non-empty string at runtime). */
export type RouteId = string & { readonly __brand: "RouteId" };

/** Opaque branded type for a Flint Gate site ID. */
export type SiteId = string & { readonly __brand: "SiteId" };

/** Opaque branded type for a Flint Gate API key (never logged in full). */
export type ApiKeyValue = string & { readonly __brand: "ApiKeyValue" };

/** Branded helper — runs at module boundaries only; not exported to end users. */
export function asRouteId(s: string): RouteId {
  return s as RouteId;
}
export function asSiteId(s: string): SiteId {
  return s as SiteId;
}
export function asApiKeyValue(s: string): ApiKeyValue {
  return s as ApiKeyValue;
}

// ---------------------------------------------------------------------------
// Auth configuration
// ---------------------------------------------------------------------------

/** Selects how the SDK authenticates against Flint Gate on proxied requests. */
export type AuthConfig =
  | { readonly type: "anonymous" }
  | {
      readonly type: "apiKey";
      /** Raw key value. The SDK redacts it from logs and console output. */
      readonly key: ApiKeyValue;
      /** Header name carrying the key. Defaults to `x-api-key`. */
      readonly header?: string;
    }
  | {
      readonly type: "bearer";
      /** Raw bearer token (JWT or opaque). */
      readonly token: string;
      /** Header name. Defaults to `authorization`. */
      readonly header?: string;
    }
  | {
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

// ---------------------------------------------------------------------------
// Stream events (AG-UI compatible subset)
// ---------------------------------------------------------------------------

/** Incremental text fragment from a TEXT_MESSAGE_CONTENT-style delta. */
export interface TextDelta {
  readonly type: "text-delta";
  /** Concatenate this in arrival order to reconstruct the full message. */
  readonly text: string;
  /** Optional message id when the upstream protocol carries one. */
  readonly messageId?: string;
  /** 0-based index of this delta within its message, when known. */
  readonly index?: number;
}

/** Structured tool invocation surfaced mid-stream. */
export interface ToolCall {
  readonly type: "tool-call";
  /** Stable identifier for correlating a later tool-result event. */
  readonly id: string;
  /** Tool name as registered upstream. */
  readonly name: string;
  /** Arguments object. Untyped on purpose — varies per tool. */
  readonly args: Readonly<Record<string, unknown>>;
}

/** Terminal success marker. Always emitted exactly once per stream. */
export interface Done {
  readonly type: "done";
  /** Token counts when the upstream protocol reports them. */
  readonly usage?: TokenUsage;
  /** Optional request id assigned by Flint Gate. */
  readonly requestId?: string;
}

/** Recoverable or terminal error surfaced on the stream. */
export interface StreamError {
  readonly type: "error";
  readonly message: string;
  /** Machine-readable error code when available. */
  readonly code?: string;
  /** HTTP status from the upstream response, if this is a terminal error. */
  readonly status?: number;
}

/** Discriminated union of every event `streamSSE` / `streamNDJSON` may yield. */
export type StreamEvent = TextDelta | ToolCall | Done | StreamError;

/** Token accounting attached to {@link Done}. */
export interface TokenUsage {
  readonly promptTokens?: number;
  readonly completionTokens?: number;
  readonly totalTokens?: number;
}

// ---------------------------------------------------------------------------
// Raw frame shapes (pre-decoded)
// ---------------------------------------------------------------------------

/** Raw SSE `data:` payload after JSON parse, before normalization to {@link StreamEvent}. */
export interface RawFrame {
  readonly type: string;
  readonly [k: string]: unknown;
}

// ---------------------------------------------------------------------------
// Admin API — policies
// ---------------------------------------------------------------------------

/** A Cedar authorization policy stored in Flint Gate. */
export interface PolicyRow {
  readonly id: string;
  readonly policy_text: string;
  readonly schema_json?: Record<string, unknown> | null;
  readonly entities_json?: Record<string, unknown> | null;
  readonly enabled: boolean;
  /** JWT `sub` of the operator who last modified this policy. */
  readonly written_by?: string | null;
}

/** A single version snapshot in a policy's history. */
export interface PolicyVersionRow {
  readonly id: string;
  readonly policy_id: string;
  readonly version_num: number;
  readonly policy_text: string;
  readonly schema_json?: Record<string, unknown> | null;
  readonly entities_json?: Record<string, unknown> | null;
  readonly written_by?: string | null;
  readonly written_at: string;
}

/** Response from GET /policies/{id}/history. */
export interface PolicyHistoryResponse {
  readonly policy_id: string;
  /** Server-side row count hint. May be null when count is unavailable. */
  readonly total_hint: number | null;
  readonly versions: readonly PolicyVersionRow[];
}

/** Response from POST /policies/{id}/rollback. */
export interface RollbackResponse {
  readonly status: string;
  readonly policy_id: string;
  readonly from_version: number;
  readonly to_version: number;
}

/** Input for creating or updating a Cedar policy. */
export interface UpsertPolicyInput {
  /** Policy id. Required on create; matched against path param on update. */
  readonly id: string;
  readonly policy_text: string;
  readonly schema_json?: Record<string, unknown> | null;
  readonly entities_json?: Record<string, unknown> | null;
  readonly enabled?: boolean;
}

/** Response from POST /policies or PUT /policies/{id}. */
export interface UpsertPolicyResponse {
  readonly status: string;
  readonly id: string;
  readonly reloaded?: boolean;
  readonly warnings?: readonly string[];
}

// ---------------------------------------------------------------------------
// Admin API — routes
// ---------------------------------------------------------------------------

/** Path/method matcher for a route. Glob patterns compile to regex server-side. */
export interface RouteMatch {
  /** Glob path, e.g. `/api/**`. */
  readonly path: string;
  /** HTTP methods. Empty/undefined means all methods. */
  readonly methods?: readonly string[];
  /** Optional host glob. */
  readonly host?: string;
}

/** Named auth provider reference on a route. */
export interface AuthProviderRef {
  /** `kratos` | `jwt` | `api_key` | `anonymous` | a custom provider id. */
  readonly type: string;
  /** Provider id from the `auth_providers` map. */
  readonly id?: string;
}

/** Header injection hook config. */
export interface InjectHeadersHook {
  readonly type: "inject_headers";
  /** Map of header name → template string. */
  readonly headers: Readonly<Record<string, string>>;
}

/** Body transform hook config (set JSON fields). */
export interface BodyTransformHook {
  readonly type: "body_transform";
  /** JSON-pointer-ish field path → template. */
  readonly fields: Readonly<Record<string, string>>;
}

/** Outbound JWT minting hook config. */
export interface MintJwtHook {
  readonly type: "mint_jwt";
  /** Signing key id from `jwt_signing_keys`. */
  readonly keyId: string;
  /** Claims template map. */
  readonly claims: Readonly<Record<string, unknown>>;
  /** TTL in seconds. */
  readonly ttlSeconds?: number;
}

export type PreRequestHook =
  | InjectHeadersHook
  | BodyTransformHook
  | MintJwtHook;

/** Full route definition as stored in `gate_routes`. */
export interface RouteConfig {
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
export type CreateRouteInput = Omit<RouteConfig, "id"> &
  Partial<Pick<RouteConfig, "id">>;

// ---------------------------------------------------------------------------
// Admin API — api keys
// ---------------------------------------------------------------------------

/** API key record as returned by GET /api-keys. Hashed value never exposed. */
export interface ApiKey {
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
export interface CreateApiKeyInput {
  readonly clientId: string;
  readonly scopes?: readonly string[];
  readonly description?: string;
  /** Optional explicit key value. If omitted the server generates one. */
  readonly key?: string;
}

/** Response from POST /api-keys. `key` is the plaintext and is only seen here. */
export interface CreateApiKeyResponse {
  readonly apiKey: ApiKey;
  /** Plaintext key — store this immediately; the server only keeps the hash. */
  readonly key: ApiKeyValue;
}

// ---------------------------------------------------------------------------
// Admin API — approvals (human-in-the-loop tool-call flow)
// ---------------------------------------------------------------------------

/**
 * Read-only view of a pending human-in-the-loop approval returned by
 * GET /approvals and GET /approvals/{id}.
 */
export interface ApprovalStatus {
  /** Opaque approval request id. */
  readonly approvalId: string;
  /** Principal (agent / user) that triggered the tool call. */
  readonly principalId: string;
  /** Action string identifying the tool call (e.g. "tool:read_file"). */
  readonly action: string;
  /** Resource id the action targets. */
  readonly resourceId: string;
  /** Optional human-readable reason the call was flagged for approval. */
  readonly reason?: string;
  /** ISO-8601 timestamp after which the approval request expires. */
  readonly expiresAt: string;
  /** True when the request has already expired (included for polling convenience). */
  readonly expired: boolean;
}

/** Decision value for POST /approvals/{id}/decision. */
export type ApprovalDecision = "approve" | "deny";

// ---------------------------------------------------------------------------
// Admin API — health
// ---------------------------------------------------------------------------

export interface HealthStatus {
  readonly status: "ok";
  readonly version?: string;
  readonly uptimeSeconds?: number;
}

export interface ReadyStatus {
  readonly status: "ready" | "degraded" | "not-ready";
  readonly checks: Readonly<Record<string, { ok: boolean; detail?: string }>>;
}

// ---------------------------------------------------------------------------
// Admin API — policies
// ---------------------------------------------------------------------------

/** Response from DELETE /policies/{id}. */
export interface DeletePolicyResponse {
  readonly status: "deleted" | string;
  readonly id: string;
}

/** One historical revision of a policy. */
export interface PolicyVersion {
  readonly version_num: number;
  readonly policy_text: string;
  readonly created_at: string;
}

/** Options for getPolicyHistory pagination. */
export interface PolicyHistoryOptions {
  readonly offset?: number;
  readonly limit?: number;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/** Base class for all SDK errors. Carries optional HTTP status and code. */
export class FlintGateError extends Error {
  public readonly status?: number;
  public readonly code?: string;
  constructor(message: string, opts?: { status?: number; code?: string }) {
    super(message);
    this.name = "FlintGateError";
    this.status = opts?.status;
    this.code = opts?.code;
  }
}

/** Non-2xx response from the admin API. */
export class FlintGateApiError extends FlintGateError {}

/** The stream ended unexpectedly (no `done` frame). */
export class StreamClosedError extends FlintGateError {
  constructor(message = "stream closed before terminal event") {
    super(message, { code: "STREAM_CLOSED" });
    this.name = "StreamClosedError";
  }
}

/** The stream surface an `error` frame. */
export class StreamProtocolError extends FlintGateError {
  constructor(message: string, code?: string, status?: number) {
    super(message, { code, status });
    this.name = "StreamProtocolError";
  }
}

// ---------------------------------------------------------------------------
// Token provider
// ---------------------------------------------------------------------------

/**
 * An async function that returns a bearer token string.
 *
 * Use this for dynamic token sources — JWTs that need periodic refresh,
 * tokens from an in-memory credential cache, etc. The SDK calls this before
 * every request (or on 401 retry) so the implementation should cache
 * internally when appropriate.
 *
 * For static tokens use {@link FlintGateClientConfig.token} — it is wrapped
 * in a `StaticTokenProvider` automatically and is backwards-compatible.
 */
export type TokenProvider = () => Promise<string>;

// ---------------------------------------------------------------------------
// Client config
// ---------------------------------------------------------------------------

/** Constructor options for {@link FlintGateClient}. */
export interface FlintGateClientConfig {
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
  /**
   * Static bearer token for the data plane. Convenience shorthand for
   * `auth: { type: "bearer", token: "..." }`. Takes precedence over `auth`
   * when both are supplied. Backwards-compatible with previous `token` field.
   */
  readonly token?: string;
  /**
   * Async token provider for dynamic/refreshable bearer tokens.
   * When supplied, the SDK calls this before every request and sets the
   * `Authorization: Bearer <token>` header with the returned value.
   * Takes precedence over both `token` and `auth.token` (for bearer auth).
   */
  readonly tokenProvider?: TokenProvider;
  /** Extra default headers merged onto every request. */
  readonly headers?: Readonly<Record<string, string>>;
  /** Optional default AbortSignal-shortening timeout in ms. */
  readonly timeoutMs?: number;
  /** Custom fetch implementation. Defaults to `globalThis.fetch`. */
  readonly fetch?: typeof fetch;
  /**
   * Maximum number of automatic retries on HTTP 429 (Rate Limited) responses.
   * Default: 3. Set to 0 to disable.
   */
  readonly maxRetries?: number;
}
