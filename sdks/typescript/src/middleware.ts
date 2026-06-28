import {
  asApiKeyValue,
  AuthConfig,
  FlintGateError,
} from "./types";

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
export interface FlintIdentity {
  readonly subject?: string;
  readonly sessionId?: string;
  readonly clientId?: string;
  readonly scopes?: readonly string[];
  readonly [k: string]: unknown;
}

export interface FlintGateMiddlewareConfig {
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

/** Normalize a Headers-like input to a plain lookup function. */
function headerGetter(headers: Headers | Record<string, string | string[] | undefined>): (name: string) => string | null {
  if (typeof (headers as Headers).get === "function") {
    return (name: string) => (headers as Headers).get(name);
  }
  const h = headers as Record<string, string | string[] | undefined>;
  return (name: string) => {
    const v = h[name] ?? h[name.toLowerCase()];
    if (v == null) return null;
    return Array.isArray(v) ? (v[0] ?? null) : v;
  };
}

function parseIdentity(get: (n: string) => string | null, identityHeader: string): FlintIdentity | null {
  const raw = get(identityHeader);
  if (!raw) return null;
  try {
    return JSON.parse(decodeURIComponent(raw)) as FlintIdentity;
  } catch {
    try {
      return JSON.parse(raw) as FlintIdentity;
    } catch {
      return null;
    }
  }
}

function checkScopes(identity: FlintIdentity | null, required: readonly string[]): boolean {
  if (required.length === 0) return true;
  const have = new Set(identity?.scopes ?? []);
  for (const s of required) {
    if (!have.has(s)) return false;
  }
  return true;
}

function checkRequest(
  get: (n: string) => string | null,
  cfg: FlintGateMiddlewareConfig,
): { ok: true; identity: FlintIdentity | null } | { ok: false; status: number; message: string } {
  const authHeader = cfg.authenticatedHeader ?? "x-flint-authenticated";
  const identityHeader = cfg.identityHeader ?? "x-flint-identity";

  const val = get(authHeader);
  if (val == null) {
    return { ok: false, status: 401, message: "missing flint-gate auth header" };
  }
  if (cfg.sharedSecret !== undefined && val !== cfg.sharedSecret) {
    return { ok: false, status: 401, message: "invalid flint-gate secret" };
  }

  const identity = parseIdentity(get, identityHeader);
  if (!checkScopes(identity, cfg.requiredScopes ?? [])) {
    return { ok: false, status: 403, message: "insufficient scope" };
  }
  return { ok: true, identity };
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
export function createFlintGateMiddleware(
  cfg: FlintGateMiddlewareConfig = {},
): (req: { headers: Headers | Record<string, string | string[] | undefined>; nextUrl?: { pathname?: string } }) =>
  Response | { ok: true; identity: FlintIdentity | null } {
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
      { status: result.status, headers: { "content-type": "application/json" } },
    );
  };
}

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
export function expressFlintGateAdapter(
  cfg: FlintGateMiddlewareConfig = {},
): (req: ExpressReq, res: ExpressRes, next: ExpressNext) => void {
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
      // Attach identity to the request for downstream handlers.
      (req as ExpressReq & { flintIdentity?: FlintIdentity | null }).flintIdentity =
        result.identity;
      next();
      return;
    }
    void (res.status(result.status) as ExpressRes).json({ error: result.message });
  };
}

/**
 * Helper for route handlers: read the identity Flint Gate injected, or throw
 * a typed error if absent. Pair with middleware that has already verified
 * the `authenticatedHeader`.
 */
export function readFlintIdentity(
  headers: Headers | Record<string, string | string[] | undefined>,
  identityHeader = "x-flint-identity",
): FlintIdentity {
  const get = headerGetter(headers);
  const identity = parseIdentity(get, identityHeader);
  if (!identity) {
    throw new FlintGateError("flint-gate identity header missing or invalid", {
      code: "IDENTITY_MISSING",
      status: 401,
    });
  }
  return identity;
}

/**
 * Build an {@link AuthConfig} for forwarding an inbound request's identity
 * downstream through Flint Gate (e.g. service-to-service). The plaintext key
 * is preserved verbatim and never logged.
 */
export function forwardApiKey(headerValue: string, headerName = "x-api-key"): AuthConfig {
  return { type: "apiKey", key: asApiKeyValue(headerValue), header: headerName };
}
