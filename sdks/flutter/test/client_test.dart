import "dart:async";
import "dart:convert";

import "package:flint_gate/flint_gate.dart";
import "package:http/http.dart" as http;
import "package:http/testing.dart";
import "package:test/test.dart";

/// Minimal streaming mock: returns a fixed [http.StreamedResponse] for any
/// request, ignoring method/url.
http.Client _streamingClient(List<int> bytes) {
  return _FixedStreamClient(bytes);
}

class _FixedStreamClient extends http.BaseClient {
  _FixedStreamClient(this._bytes);
  final List<int> _bytes;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    return http.StreamedResponse(
      Stream<List<int>>.fromIterable([_bytes]),
      200,
    );
  }

  @override
  void close() {}
}
void main() {
  group("FlintGateClient", () {
    test("getRoutes parses route list", () async {
      final mock = MockClient((request) async {
        expect(request.url.path, "/routes");
        expect(request.headers["Authorization"], "Bearer t");
        return http.Response(
          jsonEncode([
            {
              "id": "r1",
              "path": "/v1/*",
              "upstream": "https://api.example.com",
              "description": "primary",
              "enabled": true,
              "methods": ["POST"],
            }
          ]),
          200,
          headers: {"content-type": "application/json"},
        );
      });

      final client = FlintGateClient(
        baseUrl: "https://gate.test",
        authHeaders: {"Authorization": "Bearer t"},
        httpClient: mock,
      );
      addTearDown(client.close);

      final routes = await client.getRoutes();
      expect(routes, hasLength(1));
      expect(routes.single.id, "r1");
      expect(routes.single.methods, ["POST"]);
    });

    test("deleteRoute issues DELETE and asserts 2xx", () async {
      final mock = MockClient((request) async {
        expect(request.method, "DELETE");
        expect(request.url.path, "/routes/r1");
        return http.Response("", 204);
      });
      final client = FlintGateClient(
        baseUrl: "https://gate.test",
        httpClient: mock,
      );
      addTearDown(client.close);
      await expectLater(client.deleteRoute("r1"), completes);
    });

    test("getHealth parses status", () async {
      final mock = MockClient((_) async => http.Response(
            jsonEncode({"healthy": true, "version": "1.2.3", "uptimeSeconds": 90}),
            200,
          ));
      final client = FlintGateClient(
        baseUrl: "https://gate.test",
        httpClient: mock,
      );
      addTearDown(client.close);
      final h = await client.getHealth();
      expect(h.healthy, isTrue);
      expect(h.version, "1.2.3");
      expect(h.uptime.inSeconds, 90);
    });

    test("non-2xx throws FlintGateException", () async {
      final mock = MockClient((_) async => http.Response("nope", 503));
      final client = FlintGateClient(
        baseUrl: "https://gate.test",
        httpClient: mock,
      );
      addTearDown(client.close);
      await expectLater(
        client.getRoutes(),
        throwsA(isA<FlintGateException>()),
      );
    });
  });

  group("SseClient", () {
    test("parses data/event/id fields", () async {
      final payload = [
        "id: 7",
        "event: update",
        "data: {\"x\":1}",
        "",
        "data: line1",
        "data: line2",
        "",
      ].join("\n");
      final mock = _streamingClient(utf8.encode(payload));

      final sse = SseClient(
        url: "https://gate.test/stream",
        httpClient: mock,
        reconnect: false,
      );
      final events = await sse.connect().take(2).toList();
      await sse.close();

      expect(events.first.id, "7");
      expect(events.first.event, "update");
      expect(events.first.data, '{"x":1}');
      expect(events.last.data, "line1\nline2");
    });
  });

  group("AuthInterceptor", () {
    test("headers empty when token unset", () {
      final a = AuthInterceptor();
      expect(a.headers(), isEmpty);
    });
    test("headers set after setToken", () {
      final a = AuthInterceptor();
      a.setToken("abc");
      expect(a.headers(), {"Authorization": "Bearer abc"});
    });
  });
}
