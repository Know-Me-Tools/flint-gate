import "package:http/http.dart" as http;

import "types.dart";

/// HTTP client for the Flint Gate auth proxy REST API.
///
/// Pass an [http.Client] to enable mocking in tests, or leave default
/// to use [http.Client] in production. Set [baseUrl] to the proxy origin,
/// e.g. `https://gate.example.com`.
class FlintGateClient {
  FlintGateClient({
    required this.baseUrl,
    Map<String, String> authHeaders = const {},
    http.Client? httpClient,
  })  : _authHeaders = Map<String, String>.from(authHeaders),
        _httpClient = httpClient ?? http.Client();

  /// Origin of the Flint Gate proxy (no trailing slash).
  final String baseUrl;

  final Map<String, String> _authHeaders;
  final http.Client _httpClient;
  bool _closed = false;

  /// Additional auth headers to merge with per-call headers.
  Map<String, String> get authHeaders => Map<String, String>.unmodifiable(_authHeaders);

  /// Replace the auth header set (e.g. after token refresh).
  void setAuthHeaders(Map<String, String> headers) {
    _authHeaders
      ..clear()
      ..addAll(headers);
  }

  Map<String, String> _mergedHeaders([Map<String, String>? extra]) {
    return <String, String>{
      "Accept": "application/json",
      ..._authHeaders,
      if (extra != null) ...extra,
    };
  }

  Uri _uri(String path, [Map<String, dynamic>? query]) {
    final base = Uri.parse(baseUrl);
    return base.replace(
      pathSegments: path.startsWith("/")
          ? path.substring(1).split("/").where((s) => s.isNotEmpty).toList()
          : [...base.pathSegments.where((s) => s.isNotEmpty), ...path.split("/")],
      queryParameters: query,
    );
  }

  /// GET /routes — list configured routes.
  Future<List<RouteConfig>> getRoutes() async {
    final res = await _httpClient.get(_uri("/routes"), headers: _mergedHeaders());
    _ensureOk(res);
    return RouteConfig.parseList(res.body);
  }

  /// POST /routes — create a new route.
  Future<RouteConfig> createRoute(RouteConfig route) async {
    final res = await _httpClient.post(
      _uri("/routes"),
      headers: _mergedHeaders({"Content-Type": "application/json"}),
      body: route.toJson(),
    );
    _ensureOk(res);
    return RouteConfig.parse(res.body);
  }

  /// DELETE /routes/{id} — remove a route by id.
  Future<void> deleteRoute(String id) async {
    final res = await _httpClient.delete(_uri("/routes/$id"), headers: _mergedHeaders());
    _ensureOk(res);
  }

  /// GET /keys — list API keys (masked).
  Future<List<ApiKey>> getApiKeys() async {
    final res = await _httpClient.get(_uri("/keys"), headers: _mergedHeaders());
    _ensureOk(res);
    return ApiKey.parseList(res.body);
  }

  /// GET /health — proxy health probe.
  Future<HealthStatus> getHealth() async {
    final res = await _httpClient.get(_uri("/health"), headers: _mergedHeaders());
    _ensureOk(res);
    return HealthStatus.parse(res.body);
  }

  void _ensureOk(http.Response res) {
    if (res.statusCode < 200 || res.statusCode >= 300) {
      throw FlintGateException(res.statusCode, res.body);
    }
  }

  /// Release the underlying HTTP client. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _httpClient.close();
  }
}

/// Raised for non-2xx responses.
class FlintGateException implements Exception {
  const FlintGateException(this.statusCode, this.body);
  final int statusCode;
  final String body;

  @override
  String toString() => "FlintGateException($statusCode): $body";
}
