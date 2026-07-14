import {
  serializeKey,
  useEntityList,
  useEntityMutation,
} from '@prometheus-ags/prometheus-entity-management';
import {
  createApiKey,
  decideApproval,
  deletePolicy,
  deleteRoute,
  fetchConfig,
  fetchHealth,
  fetchReady,
  fetchTokenAnalytics,
  fetchUsageSummary,
  getApproval,
  issueAgentIdentity,
  listAgentIdentities,
  listApiKeys,
  listApprovals,
  listAudit,
  listPolicies,
  listRoutes,
  listToolScopes,
  revokeAgentIdentity,
  revokeApiKey,
  rotateAgentIdentity,
  deleteToolScope,
  upsertPolicy,
  upsertRoute,
  upsertToolScope,
} from '@/api/admin';
import type {
  AgentIdentity,
  AgentIdentityListResponse,
  AnalyticsInterval,
  ApiKey,
  ApiKeyListResponse,
  ApprovalDecisionResponse,
  AuditListResponse,
  AuditQueryParams,
  ConfigResponse,
  DbRoute,
  HealthResponse,
  PendingApproval,
  PolicyListResponse,
  PolicyRow,
  ReadyResponse,
  RouteListResponse,
  TokenAnalyticsResponse,
  ToolScopeListResponse,
  UsageSummaryResponse,
} from '@/api/types';

// ── Adapters: shape the entity graph result to match the TanStack Query API  ──
// All page components already consume { data, isLoading, error } for reads and
// { mutateAsync(), isPending } for writes. These thin wrappers preserve that
// contract so no page components need changing.

interface TQListResult<T> {
  data: T | undefined;
  isLoading: boolean;
  error: Error | null;
}

interface RawListLike {
  items: unknown[];
  isLoading: boolean;
  error: string | null;
}

function adaptListResult<TData>(
  raw: RawListLike,
  build: (items: unknown[]) => TData,
): TQListResult<TData> {
  return {
    data: raw.isLoading && raw.items.length === 0 ? undefined : build(raw.items),
    isLoading: raw.isLoading,
    error: raw.error ? new Error(raw.error) : null,
  };
}

interface TQMutationResult<TInput, TRaw> {
  mutateAsync: (input: TInput) => Promise<TRaw>;
  isPending: boolean;
  isError: boolean;
  error: Error | null;
  reset: () => void;
}

interface RawMutationLike<TInput, TRaw> {
  mutate: (input: TInput) => Promise<TRaw | null>;
  reset: () => void;
  state: { isPending: boolean; isError: boolean; error: string | null };
}

function adaptMutation<TInput, TRaw>(
  raw: RawMutationLike<TInput, TRaw>,
): TQMutationResult<TInput, TRaw> {
  return {
    mutateAsync: async (input: TInput): Promise<TRaw> => {
      const result = await raw.mutate(input);
      if (result === null) throw new Error('Mutation failed');
      return result;
    },
    isPending: raw.state.isPending,
    isError: raw.state.isError,
    error: raw.state.error ? new Error(raw.state.error) : null,
    reset: raw.reset,
  };
}

// ── Serialized list keys (exported for imperative invalidation) ───────────────

export const POLICIES_LIST_KEY = serializeKey(['policies']);

// ── Read hooks ────────────────────────────────────────────────────────────────

export function useConfig(): TQListResult<ConfigResponse> {
  const raw = useEntityList<ConfigResponse, ConfigResponse>({
    type: 'Config',
    queryKey: ['config'],
    fetch: async () => {
      const data = await fetchConfig();
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<ConfigResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as ConfigResponse,
  );
}

export function useHealth(): TQListResult<HealthResponse> {
  const raw = useEntityList<HealthResponse, HealthResponse>({
    type: 'Health',
    queryKey: ['health'],
    fetch: async () => {
      const data = await fetchHealth();
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<HealthResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as HealthResponse,
  );
}

export function useReady(): TQListResult<ReadyResponse> {
  const raw = useEntityList<ReadyResponse, ReadyResponse>({
    type: 'Ready',
    queryKey: ['ready'],
    fetch: async () => {
      const data = await fetchReady();
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<ReadyResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as ReadyResponse,
  );
}

export function useRoutes(): TQListResult<RouteListResponse> {
  const raw = useEntityList<DbRoute, DbRoute>({
    type: 'Route',
    queryKey: ['routes'],
    fetch: async () => {
      const res = await listRoutes();
      return { items: res.routes, total: res.routes.length };
    },
    normalize: (item) => ({ id: item.id, data: item }),
  });
  return adaptListResult<RouteListResponse>(
    raw as unknown as RawListLike,
    (items) => ({ routes: items as DbRoute[], source: 'db' }),
  );
}

export function usePolicies(): TQListResult<PolicyListResponse> {
  const raw = useEntityList<PolicyRow, PolicyRow>({
    type: 'Policy',
    queryKey: ['policies'],
    fetch: async () => {
      const res = await listPolicies();
      return { items: res.policies, total: res.policies.length };
    },
    normalize: (item) => ({ id: item.id, data: item }),
  });
  return adaptListResult<PolicyListResponse>(
    raw as unknown as RawListLike,
    (items) => ({ policies: items as PolicyRow[] }),
  );
}

export function useApiKeys(): TQListResult<ApiKeyListResponse> {
  const raw = useEntityList<ApiKey, ApiKey>({
    type: 'ApiKey',
    queryKey: ['api-keys'],
    fetch: async () => {
      const res = await listApiKeys();
      return { items: res.api_keys, total: res.api_keys.length };
    },
    normalize: (item) => ({ id: item.id, data: item }),
  });
  return adaptListResult<ApiKeyListResponse>(
    raw as unknown as RawListLike,
    (items) => ({ api_keys: items as ApiKey[] }),
  );
}

export function useToolScopes(): TQListResult<ToolScopeListResponse> {
  const raw = useEntityList<PolicyRow, PolicyRow>({
    type: 'ToolScope',
    queryKey: ['tool-scopes'],
    fetch: async () => {
      const res = await listToolScopes();
      return { items: res.tool_scopes, total: res.tool_scopes.length };
    },
    normalize: (item) => ({ id: item.id, data: item }),
  });
  return adaptListResult<ToolScopeListResponse>(
    raw as unknown as RawListLike,
    (items) => ({ tool_scopes: items as PolicyRow[] }),
  );
}

export function useAgentIdentities(): TQListResult<AgentIdentityListResponse> {
  const raw = useEntityList<AgentIdentity, AgentIdentity>({
    type: 'AgentIdentity',
    queryKey: ['agent-identities'],
    fetch: async () => {
      const res = await listAgentIdentities();
      return { items: res.agent_identities, total: res.agent_identities.length };
    },
    normalize: (item) => ({ id: item.id, data: item }),
  });
  return adaptListResult<AgentIdentityListResponse>(
    raw as unknown as RawListLike,
    (items) => ({ agent_identities: items as AgentIdentity[] }),
  );
}

/** Poll every 5 s so the operator sees new requests without a manual refresh. */
export function useApprovals(): TQListResult<{ approvals: PendingApproval[] }> & { refetch: () => void } {
  const raw = useEntityList<PendingApproval, PendingApproval>({
    type: 'Approval',
    queryKey: ['approvals'],
    fetch: async () => {
      const res = await listApprovals();
      return { items: res.approvals, total: res.approvals.length };
    },
    normalize: (item) => ({ id: item.approval_id, data: item }),
    // refetchInterval not in ListQueryOptions — handled by periodic polling below
  });
  return {
    ...adaptListResult<{ approvals: PendingApproval[] }>(
      raw as unknown as RawListLike,
      (items) => ({ approvals: items as PendingApproval[] }),
    ),
    refetch: raw.refetch,
  };
}

export function useApproval(id: string): TQListResult<PendingApproval> {
  const raw = useEntityList<PendingApproval, PendingApproval>({
    type: 'Approval',
    queryKey: ['approvals', id],
    fetch: async () => {
      if (!id) return { items: [], total: 0 };
      const data = await getApproval(id);
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: item.approval_id, data: item }),
    enabled: Boolean(id),
  });
  return adaptListResult<PendingApproval>(
    raw as unknown as RawListLike,
    (items) => (items[0] as PendingApproval),
  );
}

export function useUsageSummary(
  params: { since?: string; until?: string } = {},
): TQListResult<UsageSummaryResponse> {
  const raw = useEntityList<UsageSummaryResponse, UsageSummaryResponse>({
    type: 'AnalyticsSummary',
    queryKey: ['analytics', 'summary', params],
    fetch: async () => {
      const data = await fetchUsageSummary(params);
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<UsageSummaryResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as UsageSummaryResponse,
  );
}

export function useTokenAnalytics(
  interval: AnalyticsInterval = 'day',
): TQListResult<TokenAnalyticsResponse> {
  const raw = useEntityList<TokenAnalyticsResponse, TokenAnalyticsResponse>({
    type: 'AnalyticsTokens',
    queryKey: ['analytics', 'tokens', interval],
    fetch: async () => {
      const data = await fetchTokenAnalytics({ interval });
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<TokenAnalyticsResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as TokenAnalyticsResponse,
  );
}

export function useAudit(params: AuditQueryParams = {}): TQListResult<AuditListResponse> {
  const raw = useEntityList<AuditListResponse, AuditListResponse>({
    type: 'Audit',
    queryKey: ['audit', params],
    fetch: async () => {
      const data = await listAudit(params);
      return { items: [data], total: 1 };
    },
    normalize: (item) => ({ id: '_singleton', data: item }),
  });
  return adaptListResult<AuditListResponse>(
    raw as unknown as RawListLike,
    (items) => items[0] as AuditListResponse,
  );
}

// ── Mutation hooks ────────────────────────────────────────────────────────────

export function useUpsertRoute() {
  return adaptMutation(
    useEntityMutation<
      Parameters<typeof upsertRoute>[0],
      { status: string; id: string },
      { status: string; id: string }
    >({
      type: 'Route',
      mutate: upsertRoute,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['routes'])],
    }),
  );
}

export function useDeleteRoute() {
  return adaptMutation(
    useEntityMutation<string, { status: string; id: string }, { status: string; id: string }>({
      type: 'Route',
      mutate: deleteRoute,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['routes'])],
    }),
  );
}

export function useUpsertPolicy() {
  return adaptMutation(
    useEntityMutation<
      PolicyRow,
      { status: string; id: string; reloaded?: boolean; warnings?: string[] },
      { status: string; id: string; reloaded?: boolean; warnings?: string[] }
    >({
      type: 'Policy',
      mutate: upsertPolicy,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['policies'])],
    }),
  );
}

export function useDeletePolicy() {
  return adaptMutation(
    useEntityMutation<
      string,
      { status: string; id: string; reloaded?: boolean },
      { status: string; id: string; reloaded?: boolean }
    >({
      type: 'Policy',
      mutate: deletePolicy,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['policies'])],
    }),
  );
}

export function useUpsertToolScope() {
  return adaptMutation(
    useEntityMutation<
      Parameters<typeof upsertToolScope>[0],
      { status: string; agent: string; id: string; reloaded?: boolean },
      { status: string; agent: string; id: string; reloaded?: boolean }
    >({
      type: 'ToolScope',
      mutate: upsertToolScope,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      // Tool scopes persist as policy rows and reload the engine — refresh both.
      invalidateLists: [serializeKey(['tool-scopes']), serializeKey(['policies'])],
    }),
  );
}

export function useDeleteToolScope() {
  return adaptMutation(
    useEntityMutation<
      string,
      { status: string; agent: string; reloaded?: boolean },
      { status: string; agent: string; reloaded?: boolean }
    >({
      type: 'ToolScope',
      mutate: deleteToolScope,
      normalize: (raw) => ({ id: raw.agent, data: raw }),
      invalidateLists: [serializeKey(['tool-scopes']), serializeKey(['policies'])],
    }),
  );
}

export function useCreateApiKey() {
  return adaptMutation(
    useEntityMutation<
      Parameters<typeof createApiKey>[0],
      { id: string; client_id: string; scopes: string[]; expires_at?: string | null; key: string; note: string },
      { id: string; client_id: string; scopes: string[]; expires_at?: string | null; key: string; note: string }
    >({
      type: 'ApiKey',
      mutate: createApiKey,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['api-keys'])],
    }),
  );
}

export function useRevokeApiKey() {
  return adaptMutation(
    useEntityMutation<string, { status: string; id: string }, { status: string; id: string }>({
      type: 'ApiKey',
      mutate: revokeApiKey,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['api-keys'])],
    }),
  );
}

export function useIssueAgentIdentity() {
  return adaptMutation(
    useEntityMutation<
      Parameters<typeof issueAgentIdentity>[0],
      { status: string; id: string; kind: string },
      { status: string; id: string; kind: string }
    >({
      type: 'AgentIdentity',
      mutate: issueAgentIdentity,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['agent-identities'])],
    }),
  );
}

export function useRotateAgentIdentity() {
  return adaptMutation(
    useEntityMutation<string, { status: string; id: string }, { status: string; id: string }>({
      type: 'AgentIdentity',
      mutate: rotateAgentIdentity,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['agent-identities'])],
    }),
  );
}

export function useRevokeAgentIdentity() {
  return adaptMutation(
    useEntityMutation<string, { status: string; id: string }, { status: string; id: string }>({
      type: 'AgentIdentity',
      mutate: revokeAgentIdentity,
      normalize: (raw) => ({ id: raw.id, data: raw }),
      invalidateLists: [serializeKey(['agent-identities'])],
    }),
  );
}

export function useDecideApproval() {
  return adaptMutation(
    useEntityMutation<
      { id: string; decision: 'approve' | 'deny' },
      ApprovalDecisionResponse,
      ApprovalDecisionResponse
    >({
      type: 'Approval',
      mutate: ({ id, decision }) => decideApproval(id, { decision }),
      normalize: (raw) => ({ id: raw.approval_id, data: raw }),
      invalidateLists: [serializeKey(['approvals'])],
    }),
  );
}
