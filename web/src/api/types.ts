export interface RouteMatch {
  path: string;
  methods?: string[];
  host?: string;
}

export interface StreamConfig {
  enabled?: boolean;
  protocol?: string;
  ai?: {
    ag_ui?: { enabled?: boolean };
    a2ui?: { enabled?: boolean };
    backpressure?: { enabled?: boolean };
  };
}

export interface HooksConfig {
  pre_request: PreRequestHook[];
  post_response: PostResponseHook[];
}

export interface PreRequestHook {
  type: 'claims_enhancement' | 'body_transform' | 'max_token_budget' | 'authorize' | 'guardrail';
  config?: Record<string, unknown>;
}

export interface PostResponseHook {
  type: 'stream_meter';
  config?: Record<string, unknown>;
}

export interface RouteConfig {
  id: string;
  site: string;
  match: RouteMatch;
  upstream?: string;
  auth?: string;
  hooks?: HooksConfig;
  stream?: StreamConfig;
  priority?: number;
  enabled?: boolean;
}

export interface DbRoute {
  id: string;
  config: RouteConfig;
  priority: number;
  enabled: boolean;
}

export interface RouteListResponse {
  routes: DbRoute[];
  source: string;
  note?: string;
}

export interface PolicyRow {
  id: string;
  policy_text: string;
  schema_json?: Record<string, unknown> | null;
  entities_json?: Record<string, unknown> | null;
  enabled: boolean;
  written_by?: string | null;
}

export interface PolicyListResponse {
  policies: PolicyRow[];
}

/**
 * Structured agent tool-scope authored via the builder. Compiles server-side to
 * Cedar `permit`/`forbid` on `call_tool` for the agent — there is deliberately no
 * raw-Cedar field, so operator input reaches Cedar only through the validated
 * compiler. `deny` wins over `allow`. Values containing `*` are globs.
 */
export interface ToolScopeRequest {
  agent: string;
  allow: string[];
  deny: string[];
}

export interface ToolScopeListResponse {
  tool_scopes: PolicyRow[];
}

export interface ToolScopeUpsertResponse {
  status: string;
  agent: string;
  id: string;
  reloaded?: boolean;
}

export interface ApiKey {
  id: string;
  client_id: string;
  scopes: string[];
  expires_at?: string | null;
}

export interface ApiKeyListResponse {
  api_keys: ApiKey[];
}

export interface ApiKeyCreateRequest {
  client_id: string;
  scopes: string[];
  expires_at?: string | null;
}

export interface ApiKeyCreatedResponse extends ApiKey {
  key: string;
  note: string;
}

export type AuthProviderConfig =
  | { type: 'kratos'; base_url: string; forward_cookies?: boolean; session_cookie?: string }
  | { type: 'jwt'; jwks_url: string; issuer?: string; audience?: string; leeway_seconds?: number }
  | { type: 'mcp'; jwks_url: string; issuer?: string; audience?: string; resource: string; authorization_servers?: string[]; required_scopes?: string[]; leeway_seconds?: number }
  | { type: 'api_key'; header?: string; store?: string }
  | { type: 'anonymous'; default_subject?: string };

export interface SiteConfig {
  id: string;
  domains?: string[];
  default_auth?: string;
  default_upstream?: string;
}

export interface GateConfig {
  auth_providers: Record<string, AuthProviderConfig>;
  sites: SiteConfig[];
  routes?: RouteConfig[];
  jwt?: {
    signing_algorithm?: string;
    issuer?: string;
    default_ttl_seconds?: number;
  };
}

export interface ConfigResponse extends GateConfig {}

export interface HealthResponse {
  status: string;
  service?: string;
}

export interface ReadyResponse {
  status: string;
  db?: string;
}

// ── Analytics (read-model) ────────────────────────────────────────────────────

export interface UsageSummary {
  total_tokens: number;
  total_requests: number;
  total_duration_ms: number;
  avg_tokens_per_request: number;
  avg_duration_ms: number;
}

export interface UsageSummaryResponse {
  summary: UsageSummary;
}

export interface UsageTimeSeriesPoint {
  /** RFC3339 bucket start. */
  bucket: string;
  tokens: number;
  requests: number;
}

export interface RouteUsage {
  route_id: string;
  tokens: number;
  requests: number;
}

export interface UserUsage {
  user_id: string;
  tokens: number;
  requests: number;
}

export type AnalyticsInterval = 'hour' | 'day';

export interface TokenAnalyticsResponse {
  interval: AnalyticsInterval;
  timeseries: UsageTimeSeriesPoint[];
  by_route: RouteUsage[];
  by_user: UserUsage[];
}

// ── Authorization audit trail (read-only) ─────────────────────────────────────

export type AuthzDecision = 'allow' | 'deny' | 'step_up' | 'approval';

export interface AuditRow {
  id: string;
  request_id?: string | null;
  principal?: string | null;
  action?: string | null;
  resource?: string | null;
  decision: AuthzDecision;
  reason?: string | null;
  context?: Record<string, unknown> | null;
  created_at: string;
}

export interface AuditListResponse {
  audit: AuditRow[];
}

export interface AuditQueryParams {
  principal?: string;
  decision?: AuthzDecision;
  since?: string;
  until?: string;
  limit?: number;
  offset?: number;
}

// ── Non-human identities (agent / service) ────────────────────────────────────

export type AgentIdentityKind = 'agent' | 'service';
export type AgentIdentityStatus = 'active' | 'revoked';

export interface AgentIdentity {
  id: string;
  kind: AgentIdentityKind;
  status: AgentIdentityStatus;
  label?: string | null;
  rotated_at?: string | null;
  created_at: string;
}

export interface AgentIdentityListResponse {
  agent_identities: AgentIdentity[];
}

export interface IssueAgentIdentityRequest {
  id: string;
  kind: AgentIdentityKind;
  label?: string;
}

// ── Human-in-the-loop approvals ───────────────────────────────────────────────

export type ApprovalDecision = 'approve' | 'deny';

/** A single pending approval request surfaced by the admin list/get endpoints. */
export interface PendingApproval {
  approval_id: string;
  principal_id: string;
  action: string;
  resource_id: string;
  reason?: string;
  expires_at: string;
  expired: boolean;
}

export interface ApprovalListResponse {
  approvals: PendingApproval[];
}

export interface ApprovalDecisionRequest {
  decision: ApprovalDecision;
}

export interface ApprovalDecisionResponse {
  status: string;
  approval_id: string;
  decision: ApprovalDecision;
}

// ── Policy version history + rollback ────────────────────────────────────────

export interface PolicyVersionRow {
  id: number;
  policy_id: string;
  version_num: number;
  policy_text: string;
  schema_json?: unknown | null;
  entities_json?: unknown | null;
  written_by?: string | null;
  written_at: string;
}

export interface PolicyHistoryResponse {
  policy_id: string;
  total_hint: number | null;
  offset: number;
  limit: number;
  versions: PolicyVersionRow[];
}

export interface RollbackResponse {
  status: string;
  policy_id: string;
  from_version: number;
  to_version: number;
  reloaded: boolean;
}

// ── Cedar policy validation ───────────────────────────────────────────────────

export interface PolicyParseError {
  line: number;
  column: number;
  length: number;
  message: string;
}

export interface ValidateResponse {
  valid: boolean;
  errors: PolicyParseError[];
}

// ── Admin SSE events ──────────────────────────────────────────────────────────

export type AdminEvent =
  | { type: 'policy_reload_ok'; policy_count: number }
  | { type: 'policy_reload_error'; skipped_count: number; db_error?: string | null };

// ── Hot-reload status ─────────────────────────────────────────────────────────

export interface ReloadStatus {
  ok: boolean;
  policy_count: number;
  last_error?: string | null;
  last_reload_at?: string | null;
}
