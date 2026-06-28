# TypeScript SDK

The TypeScript SDK is published as `@know-me/flint-gate`. It runs in browsers, Node.js 18+, Deno, Bun, Cloudflare Workers, and Vercel Edge. It uses only `globalThis.fetch`, `AbortController`, `TextDecoder`, and `WebSocket`.

## Install

```bash
npm install @know-me/flint-gate
```

## Streaming an LLM response

```ts
import { FlintGateClient, streamSSE } from "@know-me/flint-gate";

const client = new FlintGateClient({
  baseUrl: "https://gate.example.com",
  auth: { type: "apiKey", key: process.env.FLINT_KEY! },
});

const res = await client.requestStream("/v1/chat/completions", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    model: "gpt-4o-mini",
    messages: [{ role: "user", content: "Summarize Flint Gate in one line." }],
    stream: true,
  }),
});

let text = "";
for await (const evt of streamSSE(res)) {
  switch (evt.type) {
    case "text-delta":
      text += evt.text;
      break;
    case "tool-call":
      console.log("tool:", evt.name, evt.args);
      break;
    case "done":
      console.log("tokens:", evt.usage?.totalTokens);
      break;
    case "error":
      console.error("stream error:", evt.message);
      break;
  }
}
```

## Cancel a stream

```ts
const ctrl = new AbortController();

const res = await client.requestStream("/v1/messages", {
  method: "POST",
  body: JSON.stringify(payload),
  signal: ctrl.signal,
});

for await (const evt of streamSSE(res, ctrl.signal)) {
  if (evt.type === "text-delta") write(evt.text);
}
```

## NDJSON

```ts
import { streamNDJSON } from "@know-me/flint-gate";

const res = await client.requestStream("/v1/embeddings", { method: "POST", body });
for await (const evt of streamNDJSON(res)) {
  // evt: StreamEvent
}
```

## WebSocket

Browsers use `globalThis.WebSocket` by default. In Node.js, install `ws` and pass the constructor:

```ts
import WebSocket from "ws";
import { streamWS } from "@know-me/flint-gate";

for await (const evt of streamWS("wss://gate.example.com/ws", {
  channel: "chat",
  auth: { type: "apiKey", key: process.env.FLINT_KEY! },
  WebSocketCtor: WebSocket as unknown as typeof WebSocket,
})) {
  if (evt.type === "text-delta") process.stdout.write(evt.text);
}
```

## Admin API

```ts
import { FlintGateClient, FlintGateAdmin } from "@know-me/flint-gate";

const client = new FlintGateClient({
  baseUrl: "https://gate.example.com",
  adminUrl: "http://gate-internal:4457",
});
const admin = new FlintGateAdmin(client);

const ready = await admin.getReady();
if (ready.status !== "ready") throw new Error("flint gate not ready");

await admin.createRoute({
  id: "chat-route",
  site: "my-app",
  match: { path: "/v1/chat/**", methods: ["POST"] },
  upstream: "http://backend:3000",
});

const routes = await admin.getRoutes();
await admin.deleteRoute("chat-route");

const created = await admin.createApiKey({
  clientId: "mobile-app",
  scopes: ["chat", "embed"],
});
console.log(created.apiKey.id, created.key.slice(0, 8) + "…");
```

## Next.js middleware

```ts
// middleware.ts
import { createFlintGateMiddleware } from "@know-me/flint-gate";

export const middleware = createFlintGateMiddleware({
  sharedSecret: process.env.FLINT_SECRET,
  requiredScopes: ["chat"],
});

export const config = { matcher: ["/api/:path*"] };
```

```ts
// app/api/route.ts
import { readFlintIdentity } from "@know-me/flint-gate";

export function GET(req: Request) {
  const identity = readFlintIdentity(req.headers);
  return Response.json({ user: identity.subject });
}
```

## Stream event types

| `type` | Payload | Meaning |
|--------|---------|---------|
| `text-delta` | `{ text, messageId?, index? }` | Incremental text fragment |
| `tool-call` | `{ id, name, args }` | Structured tool invocation |
| `done` | `{ usage?, requestId? }` | Terminal success marker |
| `error` | `{ message, code?, status? }` | Stream error |
