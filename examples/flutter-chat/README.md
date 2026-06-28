# Flutter Chat Example

A minimal Flutter app that connects to a Flint Gate SSE stream and appends deltas to a chat UI.

## Setup

```bash
cd examples/flutter-chat
flutter pub get
```

## Run

```bash
# Web
flutter run -d chrome --dart-define=FLINT_GATE_URL=http://127.0.0.1:4456

# macOS
flutter run -d macos --dart-define=FLINT_GATE_URL=http://127.0.0.1:4456

# With authentication
flutter run -d chrome \
  --dart-define=FLINT_GATE_URL=http://127.0.0.1:4456 \
  --dart-define=FLINT_GATE_TOKEN=your-token
```

## How it works

- `SseClient` opens a GET to `/api/chat/stream` through Flint Gate.
- Each `SseEvent.data` payload is appended to the latest bot message bubble.
- The connection is closed and reopened for each new user message.

## Expected behavior

1. Type a message and press send.
2. The app streams response fragments from the upstream model.
3. `[stream done]` appears when the SSE connection closes cleanly.

Make sure your Flint Gate route exposes `/api/chat/stream` and sets the correct CORS headers for Flutter web.
