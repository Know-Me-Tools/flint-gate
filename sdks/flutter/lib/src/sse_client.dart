import "dart:async";
import "dart:convert";

import "package:http/http.dart" as http;

import "types.dart";

/// A single Server-Sent Events message.
typedef SseEvent = SseEventData;

/// Connects to an SSE endpoint exposed by the Flint Gate proxy and emits
/// [SseEvent]s. Handles reconnects with exponential backoff and parses the
/// `data:`, `event:`, and `id:` fields per the SSE spec.
///
/// Use [connect] to obtain a [Stream<SseEvent>]; dispose via [close].
class SseClient {
  SseClient({
    required this.url,
    Map<String, String> headers = const {},
    http.Client? httpClient,
    this.reconnect = true,
    this.maxBackoff = const Duration(seconds: 30),
  })  : _headers = Map<String, String>.from(headers),
        _httpClient = httpClient ?? http.Client();

  /// Full SSE endpoint URL.
  final String url;
  final Map<String, String> _headers;
  final http.Client _httpClient;
  final bool reconnect;
  final Duration maxBackoff;

  final StreamController<SseEvent> _controller =
      StreamController<SseEvent>.broadcast();
  StreamSubscription<String>? _sub;
  bool _closed = false;
  String? _lastEventId;

  /// Stream of parsed SSE events. Completes when [close] is called or after
  /// a terminal error if [reconnect] is false.
  Stream<SseEvent> get events => _controller.stream;

  /// Begin listening to the SSE endpoint.
  Stream<SseEvent> connect() {
    _open();
    return _controller.stream;
  }

  Future<void> _open({Duration? initialDelay}) async {
    if (_closed) return;
    if (initialDelay != null) {
      await Future<void>.delayed(initialDelay);
      if (_closed) return;
    }

    final req = http.Request("GET", Uri.parse(url))
      ..headers.addAll({
        "Accept": "text/event-stream",
        "Cache-Control": "no-cache",
        "Connection": "keep-alive",
        ..._headers,
        if (_lastEventId != null) "Last-Event-ID": _lastEventId!,
      });

    final http.StreamedResponse response;
    try {
      response = await _httpClient.send(req);
    } catch (_) {
      _scheduleReconnect(const Duration(seconds: 1));
      return;
    }

    if (response.statusCode != 200) {
      await _drainAndFail(response);
      _scheduleReconnect(const Duration(seconds: 2));
      return;
    }

    final buffer = <String>[];
    _sub = response.stream
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen(
      (String line) => _onLine(line, buffer),
      onError: (Object _) => _scheduleReconnect(const Duration(seconds: 1)),
      onDone: () {
        _flush(buffer);
        _scheduleReconnect(const Duration(seconds: 1));
      },
      cancelOnError: true,
    );
  }

  void _onLine(String line, List<String> buffer) {
    if (line.isEmpty) {
      _flush(buffer);
      return;
    }
    buffer.add(line);
  }

  void _flush(List<String> buffer) {
    if (buffer.isEmpty) return;

    String? event;
    String? id;
    final data = <String>[];
    for (final raw in buffer) {
      if (raw.startsWith(":")) continue; // comment
      final colon = raw.indexOf(":");
      final field = colon < 0 ? raw : raw.substring(0, colon);
      var value = colon < 0 ? "" : raw.substring(colon + 1);
      if (value.startsWith(" ")) value = value.substring(1);
      switch (field) {
        case "event":
          event = value;
        case "data":
          data.add(value);
        case "id":
          id = value;
      }
    }
    buffer.clear();

    // Per spec: a line with only `data:` and empty payload = reset, not emit.
    if (data.isEmpty && event == null && id == null) return;

    if (id != null) _lastEventId = id;
    if (data.isEmpty) return; // nothing to deliver

    _controller.add(SseEvent(
      id: id,
      event: event,
      data: data.join("\n"),
    ));
  }

  void _scheduleReconnect(Duration delay) {
    if (_closed || !reconnect) {
      _controller.close();
      return;
    }
    final next = delay * 2;
    final bounded = next > maxBackoff ? maxBackoff : next;
    _open(initialDelay: bounded);
  }

  Future<void> _drainAndFail(http.StreamedResponse response) async {
    try {
      await response.stream.drain<void>();
    } catch (_) {
      // ignore
    }
  }

  /// Stop listening, cancel subscriptions, and close the stream.
  Future<void> close() async {
    _closed = true;
    await _sub?.cancel();
    _sub = null;
    await _controller.close();
    _httpClient.close();
  }
}
