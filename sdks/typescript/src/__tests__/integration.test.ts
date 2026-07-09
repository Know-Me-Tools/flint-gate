/**
 * SDK integration tests — hit a real flint-gate admin API.
 *
 * Skipped automatically when INTEGRATION_GATEWAY_URL is unset so they
 * never break unit-only CI runs. Run them with:
 *
 *   INTEGRATION_GATEWAY_URL=http://localhost:4457 pnpm test:integration
 *
 * The test fixture (docker-compose.test.yml) binds the admin port to
 * loopback inside the container (config.test.yaml: admin_listen: 127.0.0.1:4457)
 * and the docker port mapping exposes it on host :4457 — no auth required
 * (AllowLoopback posture, admin_auth not configured).
 */
import { createHmac } from "node:crypto";
import { beforeAll, describe, expect, it } from "vitest";
import { FlintGateAdmin } from "../admin";
import { FlintGateClient } from "../client";
import { asRouteId, asSiteId } from "../types";

const gatewayUrl = process.env["INTEGRATION_GATEWAY_URL"];
const proxyUrl =
  process.env["INTEGRATION_PROXY_URL"] ?? "http://127.0.0.1:4456";

function uniqueId(prefix: string): string {
  return `${prefix}-${Date.now()}`;
}

/**
 * Minimal HS256 JWT signed with the fixture secret ("test-jwt-secret").
 * Uses only Node.js built-in `node:crypto` — no external dependency.
 * Claims match config.test.yaml: iss = http://flint-gate:4456.
 */
function testJWT(): string {
  const b64url = (obj: unknown): string =>
    Buffer.from(JSON.stringify(obj))
      .toString("base64url");

  const header = b64url({ alg: "HS256", typ: "JWT" });
  const nowSecs = Math.floor(Date.now() / 1000);
  const payload = b64url({
    iss: "http://flint-gate:4456",
    sub: "integ-test-user",
    iat: nowSecs,
    exp: nowSecs + 300,
  });

  const sigInput = `${header}.${payload}`;
  const sig = createHmac("sha256", "test-jwt-secret")
    .update(sigInput)
    .digest("base64url");

  return `${sigInput}.${sig}`;
}

/**
 * Poll listApprovals until at least one pending approval appears.
 * Returns the first approval ID found, or throws if the timeout elapses.
 */
async function pollForApproval(
  admin: FlintGateAdmin,
  timeoutMs: number,
): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const approvals = await admin.listApprovals();
    if (approvals.length > 0) {
      return approvals[0].approvalId;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`No approval appeared within ${timeoutMs}ms`);
}

describe.skipIf(!gatewayUrl)("FlintGateAdmin integration", () => {
  let admin: FlintGateAdmin;

  beforeAll(() => {
    const client = new FlintGateClient({
      baseUrl: gatewayUrl!,
      adminUrl: gatewayUrl!,
    });
    admin = new FlintGateAdmin(client);
  });

  // ── Health / readiness ─────────────────────────────────────────────────────

  it("getHealth returns status ok", async () => {
    const health = await admin.getHealth();
    expect(health.status).toBe("ok");
  });

  it("getReady returns a ready response", async () => {
    const ready = await admin.getReady();
    expect(ready.status).toMatch(/^(ready|degraded)$/);
  });

  // ── Routes CRUD ────────────────────────────────────────────────────────────

  it("createRoute → getRoutes → getRoute → deleteRoute round-trip", async () => {
    const rawId = uniqueId("integ-route");

    // Create
    const created = await admin.createRoute({
      id: asRouteId(rawId),
      site: asSiteId("test"),
      match: { path: "/integ-ts/**", methods: ["GET"] },
      upstream: "http://127.0.0.1:4457/health",
      enabled: true,
    });
    expect(created.id).toBe(rawId);

    try {
      // List
      const routes = await admin.getRoutes();
      expect(routes.some((r) => r.id === rawId)).toBe(true);

      // Get by id
      const fetched = await admin.getRoute(rawId);
      expect(fetched.match.path).toBe("/integ-ts/**");
    } finally {
      // Always clean up — server may surface 404 on second delete
      await admin.deleteRoute(rawId);
    }

    // Idempotent delete — 404 is acceptable
    try {
      await admin.deleteRoute(rawId);
    } catch {
      // 404 is fine
    }
  });

  // ── Route update lifecycle ─────────────────────────────────────────────────

  it("updateRoute changes the stored route", async () => {
    const rawId = uniqueId("integ-route-upd");

    await admin.createRoute({
      id: asRouteId(rawId),
      site: asSiteId("test"),
      match: { path: "/integ-ts-upd/**", methods: ["GET"] },
      upstream: "http://127.0.0.1:4457/health",
      enabled: true,
    });

    try {
      const updated = await admin.updateRoute(rawId, {
        id: asRouteId(rawId),
        site: asSiteId("test"),
        match: { path: "/integ-ts-upd-v2/**", methods: ["GET"] },
        upstream: "http://127.0.0.1:4457/health",
        enabled: true,
      });
      expect(updated.match.path).toBe("/integ-ts-upd-v2/**");

      const fetched = await admin.getRoute(rawId);
      expect(fetched.match.path).toBe("/integ-ts-upd-v2/**");
    } finally {
      await admin.deleteRoute(rawId);
    }
  });

  // ── Policy CRUD + history + rollback ──────────────────────────────────────

  it("createPolicy → listPolicies → updatePolicy → getPolicyHistory → rollbackPolicy → deletePolicy round-trip", async () => {
    const policyId = uniqueId("integ-policy");
    const v1Text = "permit(principal, action, resource);";

    // Create (v1)
    const created = await admin.createPolicy({
      id: policyId,
      policy_text: v1Text,
      enabled: true,
    });
    expect(created.id).toBe(policyId);

    try {
      // List — must appear
      const policies = await admin.listPolicies();
      expect(policies.some((p) => p.id === policyId)).toBe(true);

      // Get by id
      const fetched = await admin.getPolicy(policyId);
      expect(fetched.policy_text).toBe(v1Text);

      // Update (v2)
      const v2Text = "forbid(principal, action, resource);";
      await admin.updatePolicy(policyId, {
        policy_text: v2Text,
        enabled: true,
      });

      // History — ≥2 versions
      const hist = await admin.getPolicyHistory(policyId);
      expect(hist.versions.length).toBeGreaterThanOrEqual(2);

      // Rollback to v1
      const v1Entry = hist.versions.find((v) => v.policy_text === v1Text);
      expect(v1Entry).toBeDefined();
      const rb = await admin.rollbackPolicy(policyId, v1Entry!.version_num);
      expect(rb.policy_id).toBe(policyId);

      // History after rollback — ≥3 entries
      const hist2 = await admin.getPolicyHistory(policyId);
      expect(hist2.versions.length).toBeGreaterThanOrEqual(3);

      // Delete
      const del = await admin.deletePolicy(policyId);
      expect(del.status).toBe("deleted");
    } catch (err) {
      // Attempt cleanup even on mid-test failure (best-effort; 404 is fine)
      try {
        await admin.deletePolicy(policyId);
      } catch {
        // ignore
      }
      throw err;
    }
  });

  // ── Approvals smoke ───────────────────────────────────────────────────────

  it("listApprovals returns without error (empty in a fresh fixture)", async () => {
    const approvals = await admin.listApprovals();
    expect(Array.isArray(approvals)).toBe(true);
  });

  // ── API Keys CRUD ─────────────────────────────────────────────────────────

  it("createApiKey → getApiKeys → revokeApiKey round-trip", async () => {
    const clientId = uniqueId("integ-key");

    // Create — plaintext secret returned exactly once
    const { apiKey, key } = await admin.createApiKey({
      clientId,
      scopes: ["read", "write"],
    });

    expect(key.length).toBeGreaterThan(8);
    expect(apiKey.clientId).toBe(clientId);

    try {
      // List — key must appear
      const keys = await admin.getApiKeys();
      expect(keys.some((k) => k.id === apiKey.id)).toBe(true);
    } finally {
      // Always clean up
      await admin.revokeApiKey(apiKey.id);
    }

    // Idempotent revoke — 404 is acceptable
    try {
      await admin.revokeApiKey(apiKey.id);
    } catch {
      // 404 is fine
    }
  });

  // ── Approval full-flow ────────────────────────────────────────────────────

  it(
    "approval full-flow: approve path — TOOL_CALL_START forwarded after decision",
    async () => {
      const policyId = uniqueId("integ-approval-policy");
      const policyText = [
        `@require_approval("human review required")`,
        `permit(principal, action, resource == Route::"integ_test_tool");`,
      ].join("\n");

      const created = await admin.createPolicy({
        id: policyId,
        policy_text: policyText,
        enabled: true,
      });
      expect(created.id).toBe(policyId);

      try {
        // Start streaming request to the proxy /stream-test route.
        // The mock-upstream will emit TOOL_CALL_START which the Cedar policy
        // intercepts → RequireApproval → gate buffers the event.
        const ac = new AbortController();
        const streamPromise = fetch(`${proxyUrl}/stream-test`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            accept: "application/x-ndjson",
            authorization: `Bearer ${testJWT()}`,
          },
          body: "{}",
          signal: ac.signal,
        });

        // Poll until the approval appears in the buffer.
        const approvalId = await pollForApproval(admin, 10_000);

        // Approve — the gate should release the buffered tool call.
        await admin.decideApproval(approvalId, "approve");

        // Read all ndjson lines from the now-unblocked stream.
        const resp = await streamPromise;
        expect(resp.ok).toBe(true);

        const body = await resp.text();
        ac.abort(); // safe to abort after body is consumed

        const eventTypes = body
          .split("\n")
          .filter(Boolean)
          .map((line) => {
            try {
              return (JSON.parse(line) as { type?: string }).type ?? "";
            } catch {
              return "";
            }
          });

        expect(eventTypes).toContain("TOOL_CALL_START");
      } finally {
        await admin.deletePolicy(policyId).catch(() => {
          // best-effort cleanup
        });
      }
    },
    { timeout: 25_000 },
  );

  it(
    "approval full-flow: deny path — stream closes without forwarding TOOL_CALL_START",
    async () => {
      const policyId = uniqueId("integ-approval-deny-policy");
      const policyText = [
        `@require_approval("human review required")`,
        `permit(principal, action, resource == Route::"integ_test_tool");`,
      ].join("\n");

      const created = await admin.createPolicy({
        id: policyId,
        policy_text: policyText,
        enabled: true,
      });
      expect(created.id).toBe(policyId);

      try {
        const streamPromise = fetch(`${proxyUrl}/stream-test`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            accept: "application/x-ndjson",
            authorization: `Bearer ${testJWT()}`,
          },
          body: "{}",
        });

        const approvalId = await pollForApproval(admin, 10_000);

        // Deny — the gate should discard the buffered call and close the stream.
        await admin.decideApproval(approvalId, "deny");

        const resp = await streamPromise;
        const body = await resp.text();

        const eventTypes = body
          .split("\n")
          .filter(Boolean)
          .map((line) => {
            try {
              return (JSON.parse(line) as { type?: string }).type ?? "";
            } catch {
              return "";
            }
          });

        // After a deny, TOOL_CALL_START must NOT be forwarded to the client.
        expect(eventTypes).not.toContain("TOOL_CALL_START");
      } finally {
        await admin.deletePolicy(policyId).catch(() => {
          // best-effort cleanup
        });
      }
    },
    { timeout: 25_000 },
  );
});
