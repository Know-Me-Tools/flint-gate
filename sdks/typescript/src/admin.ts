import { FlintGateClient } from "./client";
import {
  ApiKey,
  ApprovalDecision,
  ApprovalStatus,
  asApiKeyValue,
  asRouteId,
  asSiteId,
  CreateApiKeyInput,
  CreateApiKeyResponse,
  CreateRouteInput,
  HealthStatus,
  PolicyHistoryResponse,
  PolicyRow,
  ReadyStatus,
  RollbackResponse,
  RouteConfig,
  RouteId,
  UpsertPolicyInput,
  UpsertPolicyResponse,
} from "./types";

/**
 * Admin API surface for Flint Gate.
 *
 * All methods hit the admin port (default :4457) via
 * {@link FlintGateClient.adminRequest}. The admin port must be
 * network-isolated from the public internet.
 */
export class FlintGateAdmin {
  constructor(private readonly client: FlintGateClient) {}

  // ----------------------------------------------------------------- health

  /** Liveness probe — always returns 200 if the process is up. */
  async getHealth(signal?: AbortSignal): Promise<HealthStatus> {
    return this.client.adminRequest<HealthStatus>("/health", { signal });
  }

  /** Readiness probe — checks DB connectivity. */
  async getReady(signal?: AbortSignal): Promise<ReadyStatus> {
    return this.client.adminRequest<ReadyStatus>("/ready", { signal });
  }

  // ----------------------------------------------------------------- routes

  /** List all enabled routes. */
  async getRoutes(signal?: AbortSignal): Promise<RouteConfig[]> {
    const res = await this.client.adminRequest<{ routes: unknown[] }>("/routes", { signal });
    return res.routes.map(normalizeRoute);
  }

  /** Get a single route by id. */
  async getRoute(id: RouteId | string, signal?: AbortSignal): Promise<RouteConfig> {
    const row = await this.client.adminRequest<unknown>(
      `/routes/${encodeURIComponent(id)}`,
      { signal },
    );
    return normalizeRoute(row);
  }

  /** Create or upsert a route. Returns the stored record. */
  async createRoute(input: CreateRouteInput, signal?: AbortSignal): Promise<RouteConfig> {
    // The server acknowledges a write with `{ status, id }`; read the record back.
    const ack = await this.client.adminRequest<{ id: string }>("/routes", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(serializeRoute(input)),
      signal,
    });
    return this.getRoute(ack.id, signal);
  }

  /** Update an existing route by id. */
  async updateRoute(
    id: RouteId | string,
    input: CreateRouteInput,
    signal?: AbortSignal,
  ): Promise<RouteConfig> {
    const ack = await this.client.adminRequest<{ id: string }>(
      `/routes/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(serializeRoute({ ...input, id: input.id ?? asRouteId(id) })),
        signal,
      },
    );
    return this.getRoute(ack.id, signal);
  }

  /** Delete a route by id. */
  async deleteRoute(id: RouteId | string, signal?: AbortSignal): Promise<void> {
    await this.client.adminRequest<void>(
      `/routes/${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
    );
  }

  // --------------------------------------------------------------- api keys

  /** List all API keys (hashed values are never returned). */
  async getApiKeys(signal?: AbortSignal): Promise<ApiKey[]> {
    const res = await this.client.adminRequest<{ api_keys: unknown[] }>("/api-keys", { signal });
    return res.api_keys.map(normalizeApiKey);
  }

  /**
   * Create a new API key. The plaintext `key` is returned exactly once —
   * store it immediately; the server only retains the SHA-256 hash.
   */
  async createApiKey(
    input: CreateApiKeyInput,
    signal?: AbortSignal,
  ): Promise<CreateApiKeyResponse> {
    // The server speaks snake_case and returns the record fields flat,
    // alongside the one-time plaintext `key`.
    const res = await this.client.adminRequest<Record<string, unknown>>("/api-keys", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        client_id: input.clientId,
        scopes: input.scopes ?? [],
      }),
      signal,
    });
    return {
      apiKey: normalizeApiKey(res),
      key: asApiKeyValue(String(res.key)),
    };
  }

  /** Revoke an API key by id. */
  async revokeApiKey(id: string, signal?: AbortSignal): Promise<void> {
    await this.client.adminRequest<void>(`/api-keys/${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    });
  }

  // ------------------------------------------------------------ policies

  /** List all Cedar authorization policies (enabled and disabled). */
  async listPolicies(signal?: AbortSignal): Promise<PolicyRow[]> {
    const body = await this.client.adminRequest<{ policies: PolicyRow[] }>(
      "/policies",
      { signal },
    );
    return body.policies ?? [];
  }

  /** Fetch a single policy by id. */
  async getPolicy(id: string, signal?: AbortSignal): Promise<PolicyRow> {
    return this.client.adminRequest<PolicyRow>(
      `/policies/${encodeURIComponent(id)}`,
      { signal },
    );
  }

  /** Create a new Cedar policy. */
  async createPolicy(
    input: UpsertPolicyInput,
    signal?: AbortSignal,
  ): Promise<UpsertPolicyResponse> {
    return this.client.adminRequest<UpsertPolicyResponse>("/policies", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
  }

  /** Update (upsert) an existing Cedar policy by id. */
  async updatePolicy(
    id: string,
    input: UpsertPolicyInput,
    signal?: AbortSignal,
  ): Promise<UpsertPolicyResponse> {
    return this.client.adminRequest<UpsertPolicyResponse>(
      `/policies/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input),
        signal,
      },
    );
  }

  /** Delete a Cedar policy by id. */
  async deletePolicy(
    id: string,
    signal?: AbortSignal,
  ): Promise<{ status: string; id: string }> {
    return this.client.adminRequest<{ status: string; id: string }>(
      `/policies/${encodeURIComponent(id)}`,
      { method: "DELETE", signal },
    );
  }

  /**
   * Fetch version history for a policy.
   * @param opts.offset - Page offset (default 0).
   * @param opts.limit  - Page size (default 20).
   */
  async getPolicyHistory(
    id: string,
    opts?: { offset?: number; limit?: number },
    signal?: AbortSignal,
  ): Promise<PolicyHistoryResponse> {
    const params = new URLSearchParams();
    if (opts?.offset !== undefined) params.set("offset", String(opts.offset));
    if (opts?.limit !== undefined) params.set("limit", String(opts.limit));
    const qs = params.size > 0 ? `?${params.toString()}` : "";
    return this.client.adminRequest<PolicyHistoryResponse>(
      `/policies/${encodeURIComponent(id)}/history${qs}`,
      { signal },
    );
  }

  /**
   * Roll back a policy to a prior version.
   * Creates a new version row equal to the target — the rollback is fully
   * auditable in the version history.
   */
  async rollbackPolicy(
    id: string,
    versionNum: number,
    signal?: AbortSignal,
  ): Promise<RollbackResponse> {
    return this.client.adminRequest<RollbackResponse>(
      `/policies/${encodeURIComponent(id)}/rollback`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ version_num: versionNum }),
        signal,
      },
    );
  }

  // --------------------------------------------------------------- approvals

  /** List all non-expired pending human-in-the-loop approvals on this replica. */
  async listApprovals(signal?: AbortSignal): Promise<ApprovalStatus[]> {
    const body = await this.client.adminRequest<{ approvals: unknown[] }>(
      "/approvals",
      { signal },
    );
    return (body.approvals ?? []).map(normalizeApproval);
  }

  /**
   * Get a single pending approval by id.
   * Throws {@link FlintGateApiError} with status 404 when not found or already resolved.
   */
  async getApproval(id: string, signal?: AbortSignal): Promise<ApprovalStatus> {
    const raw = await this.client.adminRequest<unknown>(
      `/approvals/${encodeURIComponent(id)}`,
      { signal },
    );
    return normalizeApproval(raw);
  }

  /**
   * Approve or deny a pending approval request.
   * Throws {@link FlintGateApiError} with status 404 (not found / already resolved)
   * or 410 (expired).
   */
  async decideApproval(
    id: string,
    decision: ApprovalDecision,
    signal?: AbortSignal,
  ): Promise<void> {
    await this.client.adminRequest<unknown>(
      `/approvals/${encodeURIComponent(id)}/decision`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ decision }),
        signal,
      },
    );
  }
}

// ---------------------------------------------------------------------------
// serializers / normalizers
// ---------------------------------------------------------------------------

function normalizeApproval(raw: unknown): ApprovalStatus {
  const r = raw as Record<string, unknown>;
  return {
    approvalId: String(r["approval_id"]),
    principalId: String(r["principal_id"]),
    action: String(r["action"]),
    resourceId: String(r["resource_id"]),
    reason: typeof r["reason"] === "string" ? r["reason"] : undefined,
    expiresAt: String(r["expires_at"]),
    expired: r["expired"] === true,
  };
}

function normalizeApiKey(row: unknown): ApiKey {
  const k = row as Record<string, unknown>;
  return {
    id: String(k.id),
    clientId: String(k.client_id),
    scopes: Array.isArray(k.scopes) ? (k.scopes as string[]) : [],
    createdAt: typeof k.created_at === "string" ? k.created_at : undefined,
    revokedAt: typeof k.revoked_at === "string" ? k.revoked_at : undefined,
  };
}

function normalizeRoute(row: unknown): RouteConfig {
  // Stored routes come back as `{ id, config, priority, enabled }`; the route
  // itself lives in `config`, with priority/enabled kept on the record.
  const record = row as Record<string, unknown>;
  const r =
    record.config && typeof record.config === "object"
      ? { ...(record.config as Record<string, unknown>), priority: record.priority, enabled: record.enabled }
      : record;
  const id = asRouteId(String(r.id));
  const site = asSiteId(String(r.site));
  const match = r.match as RouteConfig["match"];
  return {
    id,
    site,
    match,
    upstream: String(r.upstream),
    priority: typeof r.priority === "number" ? r.priority : 0,
    enabled: typeof r.enabled === "boolean" ? r.enabled : true,
    auth: r.auth as RouteConfig["auth"] | undefined,
    hooks: r.hooks as RouteConfig["hooks"] | undefined,
    overrideYaml: typeof r.overrideYaml === "boolean" ? r.overrideYaml : undefined,
  };
}

function serializeRoute(input: CreateRouteInput): Record<string, unknown> {
  const out: Record<string, unknown> = {
    id: input.id,
    site: input.site,
    match: input.match,
    upstream: input.upstream,
    priority: input.priority ?? 0,
    enabled: input.enabled ?? true,
  };
  if (input.auth) out.auth = input.auth;
  if (input.hooks) out.hooks = input.hooks;
  if (input.overrideYaml !== undefined) out.overrideYaml = input.overrideYaml;
  return out;
}
