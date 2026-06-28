# Chat Completion Stream Example

Streams a chat completion through Flint Gate and prints each text delta as it arrives.

## Prerequisites

- Node.js 18+
- pnpm (or npm/yarn)
- Flint Gate running on `http://127.0.0.1:4456` with a route matching `/api/chat/completions`

## Setup

```bash
cd examples/chat-completion-stream
pnpm install
```

The example resolves `@know-me/flint-gate` through a local file dependency and a `tsconfig.json` path mapping that points directly at the SDK source, so no separate build step is required.

## Run

```bash
# Anonymous auth (route must allow anonymous)
pnpm dev

# Or with a bearer token / API key
FLINT_GATE_TOKEN="your-token" pnpm dev
```

## Expected output

```text
Flint Gate is an AI-native auth proxy and API gateway built for streaming LLM workloads.
[stream done]
usage: { promptTokens: 12, completionTokens: 18, totalTokens: 30 }

--- complete message ---
Flint Gate is an AI-native auth proxy and API gateway built for streaming LLM workloads.
```

The exact text depends on the upstream model. `text-delta` events are written incrementally, so short fragments may appear on the same terminal line.
