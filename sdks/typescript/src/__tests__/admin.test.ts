import { describe, expect, it, vi } from "vitest";
import { FlintGateClient } from "../client";
import { FlintGateAdmin } from "../admin";
import type {
  PolicyHistoryResponse,
  PolicyRow,
  RollbackResponse,
  UpsertPolicyResponse,
} from "../types";

/** Build a client whose adminRequest is replaced with a spy returning `payload`. */
function makeAdmin<T>(payload: T): {
  admin: FlintGateAdmin;
  spy: ReturnType<typeof vi.fn>;
} {
  const client = new FlintGateClient({
    baseUrl: "http://gate.test",
    adminUrl: "http://admin.test",
  });
  const spy = vi.fn().mockResolvedValue(payload);
  (client as unknown as Record<string, unknown>).adminRequest = spy;
  const admin = new FlintGateAdmin(client);
  return { admin, spy };
}

// ---------------------------------------------------------------------------
// listPolicies
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.listPolicies", () => {
  it("GETs /policies and returns the list", async () => {
    const rows: PolicyRow[] = [
      { id: "allow-all", policy_text: "permit(principal, action, resource);", enabled: true },
    ];
    const { admin, spy } = makeAdmin({ policies: rows });

    const result = await admin.listPolicies();

    expect(result).toEqual(rows);
    expect(spy).toHaveBeenCalledWith("/policies", { signal: undefined });
  });
});

// ---------------------------------------------------------------------------
// getPolicy
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.getPolicy", () => {
  it("GETs /policies/{id}", async () => {
    const row: PolicyRow = { id: "p1", policy_text: "forbid(principal, action, resource);", enabled: false };
    const { admin, spy } = makeAdmin(row);

    const result = await admin.getPolicy("p1");

    expect(result).toEqual(row);
    expect(spy).toHaveBeenCalledWith("/policies/p1", { signal: undefined });
  });

  it("URL-encodes policy ids that contain special characters", async () => {
    const { admin, spy } = makeAdmin({} as PolicyRow);
    await admin.getPolicy("my policy/v2");
    expect(spy).toHaveBeenCalledWith("/policies/my%20policy%2Fv2", expect.anything());
  });
});

// ---------------------------------------------------------------------------
// createPolicy
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.createPolicy", () => {
  it("POSTs to /policies with JSON body", async () => {
    const response: UpsertPolicyResponse = { status: "created", id: "new-pol", reloaded: true };
    const { admin, spy } = makeAdmin(response);

    const result = await admin.createPolicy({
      id: "new-pol",
      policy_text: "permit(principal, action, resource);",
      enabled: true,
    });

    expect(result).toEqual(response);
    expect(spy).toHaveBeenCalledWith(
      "/policies",
      expect.objectContaining({ method: "POST" }),
    );
    const body = JSON.parse(spy.mock.calls[0][1].body as string);
    expect(body.id).toBe("new-pol");
    expect(body.enabled).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// updatePolicy
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.updatePolicy", () => {
  it("PUTs to /policies/{id}", async () => {
    const response: UpsertPolicyResponse = { status: "updated", id: "p1" };
    const { admin, spy } = makeAdmin(response);

    await admin.updatePolicy("p1", {
      id: "p1",
      policy_text: "forbid(principal, action, resource);",
      enabled: false,
    });

    expect(spy).toHaveBeenCalledWith(
      "/policies/p1",
      expect.objectContaining({ method: "PUT" }),
    );
  });
});

// ---------------------------------------------------------------------------
// deletePolicy
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.deletePolicy", () => {
  it("DELETEs /policies/{id}", async () => {
    const { admin, spy } = makeAdmin({ status: "deleted", id: "p1" });

    const result = await admin.deletePolicy("p1");

    expect(result.status).toBe("deleted");
    expect(spy).toHaveBeenCalledWith("/policies/p1", expect.objectContaining({ method: "DELETE" }));
  });
});

// ---------------------------------------------------------------------------
// getPolicyHistory
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.getPolicyHistory", () => {
  it("GETs /policies/{id}/history with no options", async () => {
    const history: PolicyHistoryResponse = { policy_id: "p1", total_hint: 1, versions: [] };
    const { admin, spy } = makeAdmin(history);

    const result = await admin.getPolicyHistory("p1");

    expect(result).toEqual(history);
    expect(spy).toHaveBeenCalledWith("/policies/p1/history", { signal: undefined });
  });

  it("appends offset and limit as query string", async () => {
    const { admin, spy } = makeAdmin({ policy_id: "p1", total_hint: null, versions: [] } as PolicyHistoryResponse);

    await admin.getPolicyHistory("p1", { offset: 20, limit: 10 });

    const path: string = spy.mock.calls[0][0];
    expect(path).toContain("offset=20");
    expect(path).toContain("limit=10");
  });
});

// ---------------------------------------------------------------------------
// rollbackPolicy
// ---------------------------------------------------------------------------

describe("FlintGateAdmin.rollbackPolicy", () => {
  it("POSTs to /policies/{id}/rollback with version_num", async () => {
    const response: RollbackResponse = {
      status: "rolled_back",
      policy_id: "p1",
      from_version: 5,
      to_version: 3,
    };
    const { admin, spy } = makeAdmin(response);

    const result = await admin.rollbackPolicy("p1", 3);

    expect(result).toEqual(response);
    expect(spy).toHaveBeenCalledWith(
      "/policies/p1/rollback",
      expect.objectContaining({ method: "POST" }),
    );
    const body = JSON.parse(spy.mock.calls[0][1].body as string);
    expect(body.version_num).toBe(3);
  });
});
