import { describe, expect, it, vi } from "vitest";
import { streamSSEWithReconnect } from "../stream";
import type { StreamEvent } from "../types";

function makeSSEStream(lines: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  const body = lines.join("\n");
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
}

function makeSSEResponse(lines: string[], status = 200): Response {
  return new Response(makeSSEStream(lines), {
    status,
    headers: { "content-type": "text/event-stream" },
  });
}

describe("streamSSEWithReconnect", () => {
  it("delivers events from a clean single connection", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      makeSSEResponse([
        'data: {"type":"text-delta","text":"hello"}',
        "",
        'data: {"type":"done"}',
        "",
      ]),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSEWithReconnect("http://gate.test/stream", {
      fetch: mockFetch,
    })) {
      events.push(evt);
    }

    expect(events).toHaveLength(2);
    expect(events[0]).toEqual({ type: "text-delta", text: "hello" });
    expect(events[1]).toEqual({ type: "done" });
    expect(mockFetch).toHaveBeenCalledTimes(1);
  });

  it("reconnects after a dropped connection and delivers subsequent events", async () => {
    let call = 0;
    const mockFetch = vi.fn().mockImplementation(() => {
      call++;
      if (call === 1) {
        // First connection: yields one event then closes (no done frame)
        return Promise.resolve(
          makeSSEResponse([
            'data: {"type":"text-delta","text":"first"}',
            "",
            // Connection drops here — no done frame
          ]),
        );
      }
      // Second connection: yields remaining events and done
      return Promise.resolve(
        makeSSEResponse([
          'data: {"type":"text-delta","text":"second"}',
          "",
          'data: {"type":"done"}',
          "",
        ]),
      );
    });

    const events: StreamEvent[] = [];
    for await (const evt of streamSSEWithReconnect("http://gate.test/stream", {
      fetch: mockFetch,
      maxReconnects: 3,
    })) {
      events.push(evt);
    }

    expect(mockFetch).toHaveBeenCalledTimes(2);
    const texts = events
      .filter((e): e is Extract<StreamEvent, { type: "text-delta" }> => e.type === "text-delta")
      .map((e) => e.text);
    expect(texts).toEqual(["first", "second"]);
    expect(events.at(-1)?.type).toBe("done");
  });

  it("sends Last-Event-ID header on reconnect when id: field was received", async () => {
    let call = 0;
    let secondCallHeaders: Record<string, string> = {};

    const mockFetch = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      call++;
      if (call === 1) {
        return Promise.resolve(
          makeSSEResponse([
            "id: event-42",
            'data: {"type":"text-delta","text":"first"}',
            "",
            // No done — triggers reconnect
          ]),
        );
      }
      secondCallHeaders = (init?.headers ?? {}) as Record<string, string>;
      return Promise.resolve(
        makeSSEResponse([
          'data: {"type":"done"}',
          "",
        ]),
      );
    });

    const events: StreamEvent[] = [];
    for await (const evt of streamSSEWithReconnect("http://gate.test/stream", {
      fetch: mockFetch,
      maxReconnects: 2,
    })) {
      events.push(evt);
    }

    expect(secondCallHeaders["Last-Event-ID"]).toBe("event-42");
  });

  it("throws immediately on 4xx without retrying", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      new Response("Unauthorized", { status: 401 }),
    );

    await expect(async () => {
      for await (const _ of streamSSEWithReconnect("http://gate.test/stream", {
        fetch: mockFetch,
        maxReconnects: 3,
      })) {
        // should not reach here
      }
    }).rejects.toMatchObject({ status: 401 });

    expect(mockFetch).toHaveBeenCalledTimes(1); // no retries
  });

  it("throws after exhausting reconnect attempts on 5xx", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      new Response("server error", { status: 503 }),
    );

    await expect(async () => {
      for await (const _ of streamSSEWithReconnect("http://gate.test/stream", {
        fetch: mockFetch,
        maxReconnects: 2,
      })) {
        // should not reach here
      }
    }).rejects.toMatchObject({ status: 503 });

    expect(mockFetch).toHaveBeenCalledTimes(3); // initial + 2 retries
  });

  it("stops when AbortSignal is fired", async () => {
    const encoder = new TextEncoder();
    const ctrl = new AbortController();

    const mockFetch = vi.fn().mockImplementation(() => {
      return Promise.resolve(
        new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              controller.enqueue(
                encoder.encode('data: {"type":"text-delta","text":"hi"}\n\n'),
              );
              // Stream stays open — will be aborted
            },
          }),
          { status: 200, headers: { "content-type": "text/event-stream" } },
        ),
      );
    });

    const events: StreamEvent[] = [];
    setTimeout(() => ctrl.abort(), 10);

    for await (const evt of streamSSEWithReconnect("http://gate.test/stream", {
      fetch: mockFetch,
      signal: ctrl.signal,
    })) {
      events.push(evt);
    }

    // Should have stopped — no infinite loop
    expect(events.length).toBeLessThan(10);
  });
});
