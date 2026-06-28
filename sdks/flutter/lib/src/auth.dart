import "package:http/http.dart" as http;

/// Holds auth tokens and produces headers for outbound requests.
///
/// Token storage is in-memory by default. Subclass and override [persist]
/// / [load] for secure storage (e.g. flutter_secure_storage).
class AuthInterceptor {
  AuthInterceptor({String scheme = "Bearer"})
      : _scheme = scheme,
        _token = "";

  final String _scheme;
  String _token;

  /// The current bearer token (empty string if unauthenticated).
  String get token => _token;

  /// Replace the token and (optionally) persist it.
  void setToken(String token) {
    _token = token;
    persist(token);
  }

  /// Clear the token.
  void clear() {
    _token = "";
    persist("");
  }

  /// Authorization headers ready to merge into a request.
  Map<String, String> headers() {
    if (_token.isEmpty) return const <String, String>{};
    return <String, String>{"Authorization": "$_scheme $_token"};
  }

  /// Apply auth headers to a [http.BaseRequest]. Returns the same request.
  T apply<T extends http.BaseRequest>(T request) {
    final h = headers();
    if (h.isNotEmpty) {
      request.headers.addAll(h);
    }
    return request;
  }

  /// Override to persist the token across sessions. No-op by default.
  void persist(String token) {}

  /// Override to load a previously persisted token. Returns "" by default.
  Future<String> load() async => "";
}
