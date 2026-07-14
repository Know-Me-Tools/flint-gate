import { FlintGateError } from '@know-me/flint-gate';
import type {
  AgentIdentityListResponse,
  ApiKeyCreateRequest,
  ApiKeyCreatedResponse,
  ApiKeyListResponse,
  ApprovalDecisionRequest,
  ApprovalDecisionResponse,
  ApprovalListResponse,
  AuditListResponse,
  AuditQueryParams,
  ConfigResponse,
  DbRoute,
  HealthResponse,
  IssueAgentIdentityRequest,
  PendingApproval,
  PolicyHistoryResponse,
  PolicyListResponse,
  PolicyRow,
  ReadyResponse,
  ReloadStatus,
  RollbackResponse,
  RouteConfig,
  RouteListResponse,
  TokenAnalyticsResponse,
  ToolScopeListResponse,
  ToolScopeRequest,
  ToolScopeUpsertResponse,
  UsageSummaryResponse,
  ValidateResponse,
} from './types';

const BASE = '/api';

/**
 * Admin-plane API error. Extends the SDK's {@link FlintGateError} so admin and
 * data-plane failures share one error hierarchy — `err instanceof FlintGateError`
 * holds for both, and the workspace stays locked to the SDK's error taxonomy.
 */
export class AdminError extends FlintGateError {
  constructor(
    message: string,
    status?: number,
    public body?: unknown,
  ) {
    super(message, { status });
    this.name = 'AdminError';
  }
}

async function adminRequest<T>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    ...init,
  });

  if (!res.ok) {
    let body: unknown;
    try {
      body = await res.json();
    } catch {
      body = await res.text();
    }
    const message =
      typeof body === 'object' && body !== null && 'error' in body
        ? String((body as { error?: string }).error)
        : `Request failed: ${res.status} ${res.statusText}`;
    throw new AdminError(message, res.status, body);
  }

  if (res.status === 204) {
    return undefined as T;
  }

  return (await res.json()) as T;
}

export async function fetchConfig(): Promise<ConfigResponse> {
  return adminRequest('/config');
}

export async function fetchHealth(): Promise<HealthResponse> {
  return adminRequest('/health');
}

export async function fetchReady(): Promise<ReadyResponse> {
  return adminRequest('/ready');
}

export async function listRoutes(): Promise<RouteListResponse> {
  return adminRequest('/routes');
}

export async function getRoute(id: string): Promise<DbRoute> {
  return adminRequest(`/routes/${encodeURIComponent(id)}`);
}

export async function upsertRoute(route: RouteConfig): Promise<{ status: string; id: string }> {
  return adminRequest('/routes', {
    method: 'POST',
    body: JSON.stringify(route),
  });
}

export async function deleteRoute(id: string): Promise<{ status: string; id: string }> {
  return adminRequest(`/routes/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export async function listPolicies(): Promise<PolicyListResponse> {
  return adminRequest('/policies');
}

export async function getPolicy(id: string): Promise<PolicyRow> {
  return adminRequest(`/policies/${encodeURIComponent(id)}`);
}

export async function upsertPolicy(policy: PolicyRow): Promise<{ status: string; id: string; reloaded?: boolean; warnings?: string[] }> {
  return adminRequest('/policies', {
    method: 'POST',
    body: JSON.stringify(policy),
  });
}

export async function deletePolicy(id: string): Promise<{ status: string; id: string; reloaded?: boolean }> {
  return adminRequest(`/policies/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

export async function validatePolicy(
  policyText: string,
  schemaJson?: string,
): Promise<ValidateResponse> {
  return adminRequest('/policies/validate', {
    method: 'POST',
    body: JSON.stringify({ policy_text: policyText, schema_json: schemaJson ?? null }),
  });
}

export async function fetchReloadStatus(): Promise<ReloadStatus> {
  return adminRequest('/policies/reload-status');
}

export async function listToolScopes(): Promise<ToolScopeListResponse> {
  return adminRequest('/tool-scopes');
}

export async function upsertToolScope(scope: ToolScopeRequest): Promise<ToolScopeUpsertResponse> {
  return adminRequest('/tool-scopes', {
    method: 'POST',
    body: JSON.stringify(scope),
  });
}

export async function deleteToolScope(agent: string): Promise<{ status: string; agent: string; reloaded?: boolean }> {
  return adminRequest(`/tool-scopes/${encodeURIComponent(agent)}`, { method: 'DELETE' });
}

export async function listApiKeys(): Promise<ApiKeyListResponse> {
  return adminRequest('/api-keys');
}

export async function createApiKey(payload: ApiKeyCreateRequest): Promise<ApiKeyCreatedResponse> {
  return adminRequest('/api-keys', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function revokeApiKey(id: string): Promise<{ status: string; id: string }> {
  return adminRequest(`/api-keys/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// ── Analytics + audit (read-only) ─────────────────────────────────────────────

/** Serialize defined, non-empty query params into a `?a=b&…` suffix (or ''). */
function queryString(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== '') {
      search.set(key, String(value));
    }
  }
  const encoded = search.toString();
  return encoded ? `?${encoded}` : '';
}


export async function fetchPolicyHistory(
  id: string,
  offset = 0,
  limit = 20,
): Promise<PolicyHistoryResponse> {
  return adminRequest(
    `/policies/${encodeURIComponent(id)}/history${queryString({ offset, limit })}`,
  );
}

export async function rollbackPolicy(
  id: string,
  versionNum: number,
): Promise<RollbackResponse> {
  return adminRequest(`/policies/${encodeURIComponent(id)}/rollback`, {
    method: 'POST',
    body: JSON.stringify({ version_num: versionNum }),
  });
}

export async function fetchUsageSummary(
  params: { since?: string; until?: string } = {},
): Promise<UsageSummaryResponse> {
  return adminRequest(`/analytics/summary${queryString(params)}`);
}

export async function fetchTokenAnalytics(
  params: { since?: string; until?: string; interval?: string; limit?: number } = {},
): Promise<TokenAnalyticsResponse> {
  return adminRequest(`/analytics/tokens${queryString(params)}`);
}

export async function listAudit(params: AuditQueryParams = {}): Promise<AuditListResponse> {
  return adminRequest(
    `/audit${queryString({
      principal: params.principal,
      decision: params.decision,
      since: params.since,
      until: params.until,
      limit: params.limit,
      offset: params.offset,
    })}`,
  );
}

// ── Non-human identities ──────────────────────────────────────────────────────

export async function listAgentIdentities(): Promise<AgentIdentityListResponse> {
  return adminRequest('/agent-identities');
}

export async function issueAgentIdentity(
  payload: IssueAgentIdentityRequest,
): Promise<{ status: string; id: string; kind: string }> {
  return adminRequest('/agent-identities', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function rotateAgentIdentity(id: string): Promise<{ status: string; id: string }> {
  return adminRequest(`/agent-identities/${encodeURIComponent(id)}/rotate`, { method: 'POST' });
}

export async function revokeAgentIdentity(id: string): Promise<{ status: string; id: string }> {
  return adminRequest(`/agent-identities/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

// ── Human-in-the-loop approvals ───────────────────────────────────────────────

export async function listApprovals(): Promise<ApprovalListResponse> {
  return adminRequest('/approvals');
}

export async function getApproval(id: string): Promise<PendingApproval> {
  return adminRequest(`/approvals/${encodeURIComponent(id)}`);
}

export async function decideApproval(
  id: string,
  payload: ApprovalDecisionRequest,
): Promise<ApprovalDecisionResponse> {
  return adminRequest(`/approvals/${encodeURIComponent(id)}/decision`, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}
