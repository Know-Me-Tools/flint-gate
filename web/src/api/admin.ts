import type {
  ApprovalsResponse,
  ApprovalCountsResponse,
  PoliciesResponse,
  PolicyHistoryResponse,
  UpsertPolicyResponse,
  ValidationResponse,
} from './types';

const BASE = '/api';

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Content-Type': 'application/json', ...init?.headers },
    ...init,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(`${res.status}: ${text}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

export const adminApi = {
  listPolicies: () => req<PoliciesResponse>('/policies'),

  createPolicy: (body: { policy_text: string; enabled: boolean }) =>
    req<UpsertPolicyResponse>('/policies', { method: 'POST', body: JSON.stringify(body) }),

  updatePolicy: (id: string, body: { policy_text: string; enabled: boolean }) =>
    req<UpsertPolicyResponse>(`/policies/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify(body),
    }),

  deletePolicy: (id: string) =>
    req<{ status: string; id: string }>(`/policies/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  validatePolicy: (policy_text: string) =>
    req<ValidationResponse>('/policies/validate', {
      method: 'POST',
      body: JSON.stringify({ policy_text }),
    }),

  getPolicyHistory: (id: string, offset = 0, limit = 20) =>
    req<PolicyHistoryResponse>(
      `/policies/${encodeURIComponent(id)}/history?offset=${offset}&limit=${limit}`,
    ),

  listApprovals: (policyId?: string) => {
    const qs = policyId ? `?policy=${encodeURIComponent(policyId)}` : '';
    return req<ApprovalsResponse>(`/approvals${qs}`);
  },

  getApprovalCounts: () => req<ApprovalCountsResponse>('/approvals/counts'),

  decideApproval: (id: string, decision: 'approved' | 'rejected') =>
    req<{ id: string; decision: string }>(`/approvals/${encodeURIComponent(id)}/decide`, {
      method: 'POST',
      body: JSON.stringify({ decision }),
    }),
};
