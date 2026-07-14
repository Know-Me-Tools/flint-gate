export interface Policy {
  id: string;
  policy_text: string;
  enabled: boolean;
  written_by?: string | null;
  version_num?: number;
  created_at?: string;
  updated_at?: string;
}

export interface PolicyVersion {
  id: number;
  policy_id: string;
  version_num: number;
  policy_text: string;
  schema_json: null;
  entities_json: null;
  written_by?: string | null;
  written_at: string;
}

export interface PolicyHistoryResponse {
  policy_id: string;
  total_hint: number | null;
  offset: number;
  limit: number;
  versions: PolicyVersion[];
}

export interface PoliciesResponse {
  policies: Policy[];
}

export interface UpsertPolicyResponse {
  status: string;
  id: string;
  reloaded?: boolean;
}

export interface ValidationResponse {
  valid: boolean;
  errors?: string[];
}

export interface PendingApproval {
  approval_id: string;
  principal_id: string;
  action: string;
  resource_id: string;
  reason?: string | null;
  expires_at: string;
}

export interface ApprovalsResponse {
  approvals: PendingApproval[];
}

export interface ApprovalCountsResponse {
  counts: Record<string, number>;
}
