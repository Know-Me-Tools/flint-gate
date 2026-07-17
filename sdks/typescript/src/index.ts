/**
 * @know-me/flint-gate — TypeScript SDK for the Flint Gate AI auth proxy.
 *
 * Public surface:
 *   - {@link FlintGateClient}      — HTTP data-plane + admin-plane client
 *   - {@link FlintGateAdmin}       — admin API methods (routes, api keys, health)
 *   - {@link streamSSE}            — async-iterable SSE consumer
 *   - {@link streamNDJSON}         — async-iterable NDJSON consumer
 *   - {@link streamWS}             — async-iterable WebSocket consumer
 *   - {@link createFlintGateMiddleware} — Next.js middleware helper
 *   - {@link expressFlintGateAdapter}   — Express adapter
 *   - Type unions: {@link StreamEvent}, {@link RouteConfig}, {@link ApiKey}
 *
 * Edge-runtime safe. No Node.js built-ins. Works in browsers, workers,
 * Next.js Edge / Node runtimes, Deno, and Bun.
 *
 * @packageDocumentation
 */

export {
  FlintGateClient,
  isRateLimited,
  isUnauthorized,
  isApprovalRequired,
  isNotFound,
} from "./client";
export type { TokenProvider } from "./client";

export {
  FlintGateAdmin,
} from "./admin";

export {
  streamSSE,
  streamNDJSON,
  streamSSEWithReconnect,
} from "./stream";
export type { SSEReconnectOptions } from "./stream";

export {
  streamWS,
} from "./ws";
export type { FlintGateWSOptions } from "./ws";

export {
  createFlintGateMiddleware,
  expressFlintGateAdapter,
  readFlintIdentity,
  forwardApiKey,
} from "./middleware";
export type {
  FlintGateMiddlewareConfig,
  FlintIdentity,
} from "./middleware";

export {
  // Brands + brand helpers
  asRouteId,
  asSiteId,
  asApiKeyValue,
  // Errors
  FlintGateError,
  FlintGateApiError,
  StreamClosedError,
  StreamProtocolError,
} from "./types";

export type {
  RouteId,
  SiteId,
  ApiKeyValue,
  AuthConfig,
  TokenProvider,
  StreamEvent,
  TextDelta,
  ToolCall,
  Done,
  StreamError,
  TokenUsage,
  RawFrame,
  RouteMatch,
  AuthProviderRef,
  InjectHeadersHook,
  BodyTransformHook,
  MintJwtHook,
  PreRequestHook,
  RouteConfig,
  CreateRouteInput,
  ApiKey,
  CreateApiKeyInput,
  CreateApiKeyResponse,
  HealthStatus,
  ReadyStatus,
  FlintGateClientConfig,
  PolicyRow,
  UpsertPolicyInput,
  UpsertPolicyResponse,
  DeletePolicyResponse,
  PolicyVersion,
  PolicyVersionRow,
  PolicyHistoryResponse,
  PolicyHistoryOptions,
  RollbackResponse,
  ApprovalStatus,
  ApprovalDecision,
} from "./types";
