# flint_gate (Flutter / Dart SDK)

> **Coming Soon** — this SDK is a work in progress and is not yet published
> to pub.dev. The API surface shown below is a design draft; it may change
> before the initial release.
>
> For production use today, please use one of the available SDKs:
> - **Go**: `github.com/know-me-tools/flint-gate/sdks/go`
> - **TypeScript / Node.js**: `@know-me/flint-gate` (npm)

---

Flutter/Dart client SDK for the **Flint Gate** AI auth proxy. Provides a typed
REST client and a Server-Sent Events (SSE) client for streaming responses from
proxied model endpoints.

## Install (future)

Add to `pubspec.yaml`:

```yaml
dependencies:
  flint_gate: ^0.1.0
```

```sh
dart pub get
```

Requires Dart SDK `>=3.4.0 <4.0.0`.

## Quickstart (draft)

```dart
import "package:flint_gate/flint_gate.dart";

final client = FlintGateClient(
  baseUrl: "https://gate.example.com",
  authHeaders: {"Authorization": "Bearer <proxy-api-key>"},
);

final routes = await client.getRoutes();
final health = await client.getHealth();
```

### Streaming an SSE endpoint

```dart
final sse = SseClient(
  url: "https://gate.example.com/v1/chat/stream?model=...",
  headers: {"Authorization": "Bearer <proxy-api-key>"},
);

await for (final ev in sse.connect()) {
  print("[${ev.event ?? "message"}] ${ev.data}");
  if (ev.event == "done") break;
}
await sse.close();
```

## License

MIT
