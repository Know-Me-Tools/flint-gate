import { FlintGateError } from '@know-me/flint-gate';
import type {
  ApiKeyCreateRequest,
  ApiKeyCreatedResponse,
  ApiKeyListResponse,
  AuditListResponse,
  AuditQueryParams,
  ConfigResponse,
  DbRoute,
  HealthResponse,
  PolicyListResponse,
  PolicyRow,
  ReadyResponse,
  RouteConfig,
  RouteListResponse,
  TokenAnalyticsResponse,
  UsageSummaryResponse,
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
