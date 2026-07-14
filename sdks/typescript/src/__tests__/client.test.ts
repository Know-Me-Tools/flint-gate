import { describe, expect, it, vi, beforeEach } from "vitest";
import { FlintGateClient, isRateLimited, isUnauthorized, isApprovalRequired, isNotFound } from "../client";
import { FlintGateApiError } from "../types";
import type { TokenProvider } from "../types";

// ---------------------------------------------------------------------------
// Error helper functions
// ---------------------------------------------------------------------------

describe("error helpers", () => {
  it.each([
    { fn: isRateLimited, status: 429, name: "isRateLimited" },
    { fn: isUnauthorized, status: 401, name: "isUnauthorized" },
    { fn: isApprovalRequired, status: 403, name: "isApprovalRequired" },
    { fn: isNotFound, status: 404, name: "isNotFound" },
  ])("$name returns true for FlintGateApiError with status $status", ({ fn, status }) => {
    const err = new FlintGateApiError("test", { status });
    expect(fn(err)).toBe(true);
  });

  it.each([
    { fn: isRateLimited, status: 401 },
    { fn: isUnauthorized, status: 429 },
    { fn: isApprovalRequired, status: 404 },
    { fn: isNotFound, status: 403 },
  ])("returns false for wrong status", ({ fn, status }) => {
    const err = new FlintGateApiError("test", { status });
    expect(fn(err)).toBe(false);
  });

  it("returns false for non-APIError objects", () => {
    expect(isRateLimited(new Error("oops"))).toBe(false);
    expect(isUnauthorized("string")).toBe(false);
    expect(isApprovalRequired(null)).toBe(false);
    expect(isNotFound(undefined)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Retry on 429
// ---------------------------------------------------------------------------

describe("retry on 429", () => {
  it("succeeds when server returns 429 twice then 200", async () => {
    let calls = 0;
    const mockFetch = vi.fn().mockImplementation(() => {
      calls++;
      if (calls <= 2) {
        return Promise.resolve(
          new Response(JSON.stringify({ error: "rate limited" }), {
            status: 429,
            headers: { "Content-Type": "application/json" },
          }),
        );
      }
      return Promise.resolve(
        new Response(JSON.stringify({ status: "ok" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      fetch: mockFetch,
      maxRetries: 3,
    });

    // Patch sleep to skip delays in tests
    const result = await client.request<{ status: string }>("/health");
    expect(result.status).toBe("ok");
    expect(calls).toBe(3);
  });

  it("throws after all retries are exhausted", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "rate limited" }), {
        status: 429,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      fetch: mockFetch,
      maxRetries: 2,
    });

    await expect(client.request("/health")).rejects.toSatisfy(isRateLimited);
    expect(mockFetch).toHaveBeenCalledTimes(3); // initial + 2 retries
  });

  it("does not retry on 401", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "unauthorized" }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      }),
    );

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      fetch: mockFetch,
      maxRetries: 3,
    });

    await expect(client.request("/health")).rejects.toSatisfy(isUnauthorized);
    expect(mockFetch).toHaveBeenCalledTimes(1); // no retries for non-429
  });
});

// ---------------------------------------------------------------------------
// TokenProvider
// ---------------------------------------------------------------------------

describe("TokenProvider", () => {
  it("static token string sets Authorization header", async () => {
    let capturedHeaders: Record<string, string> = {};
    const mockFetch = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      capturedHeaders = init.headers as Record<string, string>;
      return Promise.resolve(
        new Response(JSON.stringify({ status: "ok" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      token: "my-static-token",
      fetch: mockFetch,
    });

    await client.request("/health");
    expect(capturedHeaders["authorization"]).toBe("Bearer my-static-token");
  });

  it("async TokenProvider is called and sets Authorization header", async () => {
    let capturedHeaders: Record<string, string> = {};
    const mockFetch = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      capturedHeaders = init.headers as Record<string, string>;
      return Promise.resolve(
        new Response(JSON.stringify({ status: "ok" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    let providerCallCount = 0;
    const tokenProvider: TokenProvider = async () => {
      providerCallCount++;
      return "dynamic-token-" + providerCallCount;
    };

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      tokenProvider,
      fetch: mockFetch,
    });

    await client.request("/health");
    expect(capturedHeaders["authorization"]).toBe("Bearer dynamic-token-1");
    expect(providerCallCount).toBe(1);

    await client.request("/health");
    expect(capturedHeaders["authorization"]).toBe("Bearer dynamic-token-2");
    expect(providerCallCount).toBe(2);
  });

  it("tokenProvider takes precedence over token", async () => {
    let capturedHeaders: Record<string, string> = {};
    const mockFetch = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      capturedHeaders = init.headers as Record<string, string>;
      return Promise.resolve(
        new Response(JSON.stringify({}), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    const client = new FlintGateClient({
      baseUrl: "http://gate.test",
      token: "static-token",
      tokenProvider: async () => "provider-token",
      fetch: mockFetch,
    });

    await client.request("/health");
    expect(capturedHeaders["authorization"]).toBe("Bearer provider-token");
  });
});
