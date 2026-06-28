# flint_gate

Flutter/Dart client SDK for the **Flint Gate** AI auth proxy. Provides a typed
REST client and a Server-Sent Events (SSE) client for streaming responses from
proxied model endpoints.

## Install

Add to `pubspec.yaml`:

```yaml
dependencies:
  flint_gate:
    path: ../sdks/flutter   # or git / hosted reference
```

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

The client parses the `id:`, `event:`, and `data:` fields per the SSE spec,
reconnects with exponential backoff on dropped connections, and replays
`Last-Event-ID` so the server can resume the stream.

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

| Method | Path | Dart |
| --- | --- | --- |
| GET | `/routes` | `getRoutes()` → `List<RouteConfig>` |
| POST | `/routes` | `createRoute(route)` → `RouteConfig` |
| DELETE | `/routes/{id}` | `deleteRoute(id)` |
| GET | `/keys` | `getApiKeys()` → `List<ApiKey>` |
| GET | `/health` | `getHealth()` → `HealthStatus` |

## Auth

`AuthInterceptor` builds the `Authorization` header from an in-memory token
and can be wired into your own `http.BaseClient` chain. Override
`persist`/`load` for secure storage on device:

```dart
class SecureAuth extends AuthInterceptor {
  @override
  void persist(String token) { /* write to flutter_secure_storage */ }
}
```

## Testing

```sh
dart pub get
dart test
```

The tests use `package:http/testing.dart` so no network access is required.

## License

MIT
