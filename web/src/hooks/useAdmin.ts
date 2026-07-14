import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSearchParams } from 'react-router-dom';
import { adminApi } from '@/api/admin';

export function usePolicies() {
  return useQuery({
    queryKey: ['policies'],
    queryFn: () => adminApi.listPolicies(),
  });
}

export function usePolicyHistory(_policyId: string | null) {
  return useQuery({
    queryKey: ['policy-history', _policyId],
    queryFn: () => adminApi.getPolicyHistory(_policyId!, 0, 20),
    enabled: !!_policyId,
  });
}

export function useApprovalCount() {
  return useQuery({
    queryKey: ['approval-counts'],
    queryFn: () => adminApi.getApprovalCounts(),
    refetchInterval: 10_000,
  });
}

export function useCreatePolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { policy_text: string; enabled: boolean }) =>
      adminApi.createPolicy(body),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['policies'] }),
  });
}

export function useUpdatePolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string; policy_text: string; enabled: boolean }) =>
      adminApi.updatePolicy(id, body),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['policies'] }),
  });
}

export function useDeletePolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => adminApi.deletePolicy(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['policies'] }),
  });
}

export function useValidatePolicy() {
  return useMutation({
    mutationFn: ({ policy_text }: { policy_text: string }) =>
      adminApi.validatePolicy(policy_text),
  });
}

export function useApprovals() {
  const [searchParams] = useSearchParams();
  const policyId = searchParams.get('policy') ?? undefined;
  return useQuery({
    queryKey: ['approvals', policyId],
    queryFn: () => adminApi.listApprovals(policyId),
    refetchInterval: 5_000,
  });
}

export function useDecideApproval() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, decision }: { id: string; decision: 'approve' | 'deny' }) =>
      adminApi.decideApproval(id, decision === 'approve' ? 'approved' : 'rejected'),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['approvals'] }),
  });
}
