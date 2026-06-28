import { FlintGateClient } from "./client";
import {
  ApiKey,
  asApiKeyValue,
  asRouteId,
  asSiteId,
  CreateApiKeyInput,
  CreateApiKeyResponse,
  CreateRouteInput,
  HealthStatus,
  ReadyStatus,
  RouteConfig,
  RouteId,
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
    const rows = await this.client.adminRequest<unknown[]>("/routes", { signal });
    return rows.map(normalizeRoute);
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
    const row = await this.client.adminRequest<unknown>("/routes", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(serializeRoute(input)),
      signal,
    });
    return normalizeRoute(row);
  }

  /** Update an existing route by id. */
  async updateRoute(
    id: RouteId | string,
    input: CreateRouteInput,
    signal?: AbortSignal,
  ): Promise<RouteConfig> {
    const row = await this.client.adminRequest<unknown>(
      `/routes/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(serializeRoute({ ...input, id: input.id ?? asRouteId(id) })),
        signal,
      },
    );
    return normalizeRoute(row);
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
    return this.client.adminRequest<ApiKey[]>("/api-keys", { signal });
  }

  /**
   * Create a new API key. The plaintext `key` is returned exactly once —
   * store it immediately; the server only retains the SHA-256 hash.
   */
  async createApiKey(
    input: CreateApiKeyInput,
    signal?: AbortSignal,
  ): Promise<CreateApiKeyResponse> {
    const res = await this.client.adminRequest<{
      apiKey: ApiKey;
      key: string;
    }>("/api-keys", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        clientId: input.clientId,
        scopes: input.scopes ?? [],
        description: input.description,
        key: input.key,
      }),
      signal,
    });
    return {
      apiKey: res.apiKey,
      key: asApiKeyValue(res.key),
    };
  }

  /** Revoke an API key by id. */
  async revokeApiKey(id: string, signal?: AbortSignal): Promise<void> {
    await this.client.adminRequest<void>(`/api-keys/${encodeURIComponent(id)}`, {
      method: "DELETE",
      signal,
    });
  }
}

// ---------------------------------------------------------------------------
// serializers / normalizers
// ---------------------------------------------------------------------------

function normalizeRoute(row: unknown): RouteConfig {
  const r = row as Record<string, unknown>;
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
