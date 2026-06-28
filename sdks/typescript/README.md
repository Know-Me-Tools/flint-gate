# @know-me/flint-gate

TypeScript client SDK for [Flint Gate](https://github.com/know-me-tools/flint-gate), the streaming-first AI auth proxy. Consumes SSE, WebSocket, and NDJSON streams from proxied LLM traffic and calls the admin API.

Edge-runtime safe. Uses only `globalThis.fetch`, `AbortController`, `TextDecoder`, and `WebSocket`. No Node.js built-ins. Runs in browsers, Cloudflare Workers, Vercel Edge, Deno, Bun, and Node.js 18+.

## Install

```bash
npm install @know-me/flint-gate
# or
pnpm add @know-me/flint-gate
# or
yarn add @know-me/flint-gate
```

## Quickstart: stream an LLM response

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
console.log(text);
```

## Cancel a stream

```ts
const ctrl = new AbortController();
document.getElementById("stop").addEventListener("click", () => ctrl.abort());

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

The browser WebSocket API is used by default. In Node.js, install `ws` and pass the constructor.

```ts
import WebSocket from "ws"; // Node.js only
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

The admin port (`:4457` by default) must be network-isolated from the public internet.

```ts
import { FlintGateClient, FlintGateAdmin } from "@know-me/flint-gate";

const client = new FlintGateClient({
  baseUrl: "https://gate.example.com",         // public — not used here
  adminUrl: "http://gate-internal:4457",       // admin plane
});
const admin = new FlintGateAdmin(client);

// Health / readiness
const ready = await admin.getReady();
if (ready.status !== "ready") throw new Error("flint gate not ready");

// Routes
await admin.createRoute({
  id: "chat-route",
  site: "my-app",
  match: { path: "/v1/chat/**", methods: ["POST"] },
  upstream: "http://backend:3000",
  auth: { type: "api_key", id: "default" },
  hooks: [
    { type: "inject_headers", headers: { "x-upstream-call": "flint" } },
  ],
});

const routes = await admin.getRoutes();
await admin.deleteRoute("chat-route");

// API keys
const created = await admin.createApiKey({
  clientId: "mobile-app",
  scopes: ["chat", "embed"],
  description: "prod mobile key",
});
// Store created.key once — the server keeps only the SHA-256 hash.
console.log(created.apiKey.id, created.key.slice(0, 8) + "…");
```

## Next.js middleware

Flint Gate verifies auth on the edge and injects headers your backend can trust. Configure `inject_headers` on the route to set `x-flint-authenticated` (and optionally `x-flint-identity`).

```ts
// middleware.ts
import { createFlintGateMiddleware } from "@know-me/flint-gate";

export const middleware = createFlintGateMiddleware({
  sharedSecret: process.env.FLINT_SECRET, // verifies the injected header value
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

## Express adapter

```ts
import express from "express";
import { expressFlintGateAdapter } from "@know-me/flint-gate";

const app = express();
app.use("/api", expressFlintGateAdapter({
  sharedSecret: process.env.FLINT_SECRET,
  requiredScopes: ["chat"],
}));

app.post("/api/chat", (req, res) => {
  const identity = (req as any).flintIdentity;
  res.json({ user: identity?.subject });
});
```

## Stream event types

`streamSSE` and `streamNDJSON` yield a discriminated union. Pattern-match on `evt.type`:

| `type`         | Payload                                            | Meaning                                  |
| -------------- | -------------------------------------------------- | ---------------------------------------- |
| `text-delta`   | `{ text, messageId?, index? }`                     | Incremental text fragment.               |
| `tool-call`    | `{ id, name, args }`                               | Structured tool invocation.              |
| `done`         | `{ usage?, requestId? }`                           | Terminal success marker (emitted once).  |
| `error`        | `{ message, code?, status? }`                      | Stream error (also throws for terminal). |

Both producers accept AG-UI-style events (`TEXT_MESSAGE_CONTENT`, `TOOL_CALL`, `DONE`, `ERROR`) and normalize them to the union above.

## Errors

| Class                  | Thrown when                                          |
| ---------------------- | ---------------------------------------------------- |
| `FlintGateError`       | Base class for all SDK errors.                       |
| `FlintGateApiError`    | Admin/data-plane request returned non-2xx.           |
| `StreamProtocolError`  | Stream surfaced an `error` frame.                    |
| `StreamClosedError`    | Stream ended without a `done` frame.                 |

## Runtime requirements

- `globalThis.fetch` (Node 18+, browsers, Deno, Bun, Cloudflare Workers, Vercel Edge).
- Optional: `globalThis.WebSocket` for `streamWS` (browsers + Node with `ws`).
- Optional: `TextDecoderStream` — falls back to `TextDecoder` if absent.

## License

MIT © KnowMe, LLC
