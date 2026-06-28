# Flint Gate Examples

Minimal, runnable examples for the Flint Gate AI auth proxy.

| Example | Language / Framework | What it shows |
|---|---|---|
| [`chat-completion-stream`](./chat-completion-stream/) | Node.js / TypeScript | Stream a chat completion through Flint Gate and print text deltas |
| [`nextjs-middleware`](./nextjs-middleware/) | Next.js | Validate Flint Gate authentication headers in Edge middleware |
| [`go-downstream`](./go-downstream/) | Go | Rehydrate identity headers in a downstream HTTP service |
| [`flutter-chat`](./flutter-chat/) | Flutter | Simple chat UI consuming a Flint Gate SSE stream |

## SDK references

- TypeScript SDK: [`../../sdks/typescript/`](../../sdks/typescript/)
- Go SDK: [`../../sdks/go/`](../../sdks/go/)
- Flutter SDK: [`../../sdks/flutter/`](../../sdks/flutter/)

## Common prerequisites

Most examples assume Flint Gate is running locally on `http://127.0.0.1:4456` with a route that matches the example's path. See the main [README](../../README.md) for how to start the proxy.
