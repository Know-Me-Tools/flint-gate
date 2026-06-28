import { describe, expect, it } from "vitest";
import { streamSSE } from "../stream";
import type { StreamEvent } from "../types";

/** Build a Response from a raw SSE string. */
function sseResponse(body: string): Response {
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { "content-type": "text/event-stream" },
  });
}

describe("streamSSE", () => {
  it("parses text-delta frames split across multiple data lines (SSE joins with \\n)", async () => {
    // Per SSE spec, multiple `data:` lines are joined with "\n" between values.
    // Split at a top-level field boundary so the assembled payload stays valid JSON.
    const res = sseResponse(
      [
        "event: text-delta",
        'data: {"type":"text-delta",',
        'data: "text":"Hello"}',
        "",
        "event: done",
        'data: {"type":"done","usage":{"totalTokens":7}}',
        "",
      ].join("\n"),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSE(res)) events.push(evt);

    expect(events).toHaveLength(2);
    expect(events[0]).toEqual({ type: "text-delta", text: "Hello" });
    expect(events[1]).toEqual({ type: "done", usage: { totalTokens: 7 } });
  });

  it("emits each data: line as part of one frame's payload", async () => {
    // When a producer intentionally sends a payload containing a newline
    // (escaped properly as JSON), the assembled data preserves it.
    const res = sseResponse(
      [
        'data: {"type":"text-delta","text":"line1\\nline2"}',
        "",
        'data: {"type":"done"}',
        "",
      ].join("\n"),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSE(res)) events.push(evt);

    expect(events[0]).toEqual({ type: "text-delta", text: "line1\nline2" });
  });

  it("normalizes AG-UI TEXT_MESSAGE_CONTENT events to text-delta", async () => {
    const res = sseResponse(
      [
        "data: {\"type\":\"TEXT_MESSAGE_CONTENT\",\"text\":\"hi\"}",
        "",
        "data: [DONE]",
        "",
      ].join("\n"),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSE(res)) events.push(evt);

    expect(events[0]).toEqual({ type: "text-delta", text: "hi" });
    expect(events.at(-1)?.type).toBe("done");
  });

  it("parses tool-call frames with arguments", async () => {
    const res = sseResponse(
      [
        'data: {"type":"tool-call","id":"call_1","name":"search","args":{"q":"flint gate"}}',
        "",
        "data: {\"type\":\"done\"}",
        "",
      ].join("\n"),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSE(res)) events.push(evt);

    expect(events[0]).toEqual({
      type: "tool-call",
      id: "call_1",
      name: "search",
      args: { q: "flint gate" },
    });
  });

  it("terminates on an error frame and throws StreamProtocolError", async () => {
    const res = sseResponse(
      [
        'data: {"type":"error","message":"rate limited","code":"RATE_LIMITED","status":429}',
        "",
      ].join("\n"),
    );

    const got: StreamEvent[] = [];
    await expect(async () => {
      for await (const evt of streamSSE(res)) got.push(evt);
    }).rejects.toMatchObject({
      name: "StreamProtocolError",
      code: "RATE_LIMITED",
      status: 429,
    });
    expect(got).toHaveLength(0);
  });

  it("ignores SSE comments and heartbeat lines", async () => {
    const res = sseResponse(
      [
        ": this is a heartbeat",
        "",
        "data: {\"type\":\"text-delta\",\"text\":\"ok\"}",
        "",
        "data: {\"type\":\"done\"}",
        "",
      ].join("\n"),
    );

    const events: StreamEvent[] = [];
    for await (const evt of streamSSE(res)) events.push(evt);

    expect(events.map((e) => e.type)).toEqual(["text-delta", "done"]);
  });

  it("respects an aborted signal and stops early", async () => {
    const encoder = new TextEncoder();
    let pulled = 0;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulled += 1;
        if (pulled > 5) {
          controller.close();
          return;
        }
        controller.enqueue(
          encoder.encode(
            `data: {"type":"text-delta","text":"chunk${pulled}"}\n\n`,
          ),
        );
      },
    });

    const ctrl = new AbortController();
    const events: StreamEvent[] = [];
    setTimeout(() => ctrl.abort(), 0);
    for await (const evt of streamSSE(stream, ctrl.signal)) {
      events.push(evt);
      if (events.length >= 1) ctrl.abort();
    }
    // We should have stopped before draining the entire producer.
    expect(events.length).toBeLessThanOrEqual(2);
  });
});
