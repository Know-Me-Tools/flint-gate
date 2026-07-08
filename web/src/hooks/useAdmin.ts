import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createApiKey,
  deletePolicy,
  deleteRoute,
  fetchConfig,
  fetchHealth,
  fetchReady,
  fetchTokenAnalytics,
  fetchUsageSummary,
  issueAgentIdentity,
  listAgentIdentities,
  listApiKeys,
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
import type { AnalyticsInterval, AuditQueryParams } from '@/api/types';

export function useConfig() {
  return useQuery({ queryKey: ['config'], queryFn: fetchConfig });
}

export function useHealth() {
  return useQuery({ queryKey: ['health'], queryFn: fetchHealth });
}

export function useReady() {
  return useQuery({ queryKey: ['ready'], queryFn: fetchReady });
}

export function useRoutes() {
  return useQuery({ queryKey: ['routes'], queryFn: listRoutes });
}

export function usePolicies() {
  return useQuery({ queryKey: ['policies'], queryFn: listPolicies });
}

export function useApiKeys() {
  return useQuery({ queryKey: ['api-keys'], queryFn: listApiKeys });
}

export function useUpsertRoute() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: upsertRoute,
    onSuccess: () => client.invalidateQueries({ queryKey: ['routes'] }),
  });
}

export function useDeleteRoute() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: deleteRoute,
    onSuccess: () => client.invalidateQueries({ queryKey: ['routes'] }),
  });
}

export function useUpsertPolicy() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: upsertPolicy,
    onSuccess: () => client.invalidateQueries({ queryKey: ['policies'] }),
  });
}

export function useDeletePolicy() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: deletePolicy,
    onSuccess: () => client.invalidateQueries({ queryKey: ['policies'] }),
  });
}

export function useToolScopes() {
  return useQuery({ queryKey: ['tool-scopes'], queryFn: listToolScopes });
}

export function useUpsertToolScope() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: upsertToolScope,
    // Tool scopes persist as policy rows and reload the engine — refresh both.
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ['tool-scopes'] });
      client.invalidateQueries({ queryKey: ['policies'] });
    },
  });
}

export function useDeleteToolScope() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: deleteToolScope,
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ['tool-scopes'] });
      client.invalidateQueries({ queryKey: ['policies'] });
    },
  });
}

export function useCreateApiKey() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: createApiKey,
    onSuccess: () => client.invalidateQueries({ queryKey: ['api-keys'] }),
  });
}

export function useRevokeApiKey() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: revokeApiKey,
    onSuccess: () => client.invalidateQueries({ queryKey: ['api-keys'] }),
  });
}

// ── Analytics + audit ─────────────────────────────────────────────────────────

export function useUsageSummary(params: { since?: string; until?: string } = {}) {
  return useQuery({
    queryKey: ['analytics', 'summary', params],
    queryFn: () => fetchUsageSummary(params),
  });
}

export function useTokenAnalytics(interval: AnalyticsInterval = 'day') {
  return useQuery({
    queryKey: ['analytics', 'tokens', interval],
    queryFn: () => fetchTokenAnalytics({ interval }),
  });
}

export function useAudit(params: AuditQueryParams = {}) {
  return useQuery({
    queryKey: ['audit', params],
    queryFn: () => listAudit(params),
  });
}

// ── Non-human identities ──────────────────────────────────────────────────────

export function useAgentIdentities() {
  return useQuery({ queryKey: ['agent-identities'], queryFn: listAgentIdentities });
}

export function useIssueAgentIdentity() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: issueAgentIdentity,
    onSuccess: () => client.invalidateQueries({ queryKey: ['agent-identities'] }),
  });
}

export function useRotateAgentIdentity() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: rotateAgentIdentity,
    onSuccess: () => client.invalidateQueries({ queryKey: ['agent-identities'] }),
  });
}

export function useRevokeAgentIdentity() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: revokeAgentIdentity,
    onSuccess: () => client.invalidateQueries({ queryKey: ['agent-identities'] }),
  });
}
