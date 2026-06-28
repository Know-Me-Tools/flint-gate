# Flutter SDK

The Flutter / Dart SDK is at `sdks/flutter`. It provides a typed REST client and an SSE stream client for consuming proxied model endpoints.

## Install

Add to `pubspec.yaml`:

```yaml
dependencies:
  flint_gate:
    git:
      url: https://github.com/know-me-tools/flint-gate
      path: sdks/flutter
```

Then:

```sh
dart pub get
```

Requires Dart SDK `>=3.4.0 <4.0.0`.

## Quickstart

```dart
import "package:flint_gate/flint_gate.dart";

final client = FlintGateClient(
  baseUrl: "https://gate.example.com",
  authHeaders: {"Authorization": "Bearer <proxy-api-key>"},
);

final routes = await client.getRoutes();
final health = await client.getHealth();
```

## Streaming an SSE endpoint

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

The SSE client parses `id:`, `event:`, and `data:` fields, reconnects with exponential backoff on dropped connections, and replays `Last-Event-ID` for stream resumption.

## Flutter widget example

```dart
import "package:flutter/material.dart";
import "package:flint_gate/flint_gate.dart";

class StreamView extends StatefulWidget {
  const StreamView({super.key, required this.baseUrl, required this.token});
  final String baseUrl;
  final String token;

  @override
  State<StreamView> createState() => _StreamViewState();
}

class _StreamViewState extends State<StreamView> {
  final List<String> _lines = [];
  SseClient? _sse;

  @override
  void initState() {
    super.initState();
    _sse = SseClient(
      url: "${widget.baseUrl}/v1/chat/stream",
      headers: {"Authorization": "Bearer ${widget.token}"},
    )..connect().listen((event) {
        setState(() => _lines.add(event.data));
      });
  }

  @override
  void dispose() {
    _sse?.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: _lines.map((l) => ListTile(title: Text(l))).toList(),
    );
  }
}
```

## REST API methods

| Method | Path | Dart return |
|--------|------|-------------|
| `getRoutes()` | `GET /routes` | `List<RouteConfig>` |
| `createRoute(...)` | `POST /routes` | `RouteConfig` |
| `deleteRoute(id)` | `DELETE /routes/{id}` | void |
| `getApiKeys()` | `GET /keys` | `List<ApiKey>` |
| `getHealth()` | `GET /health` | `HealthStatus` |

## Auth

`AuthInterceptor` builds the `Authorization` header from an in-memory token. Override `persist`/`load` to store tokens securely on device:

```dart
class SecureAuth extends AuthInterceptor {
  @override
  void persist(String token) {
    // write to flutter_secure_storage
  }
}
```

## Testing

```sh
dart pub get
dart test
```

Tests use `package:http/testing.dart` and do not require network access.
