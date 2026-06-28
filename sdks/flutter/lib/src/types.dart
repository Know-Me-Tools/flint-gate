import "dart:convert";

/// One parsed SSE message.
class SseEventData {
  const SseEventData({this.id, this.event, required this.data});

  /// The `id:` field, if present. Used as `Last-Event-ID` on reconnect.
  final String? id;

  /// The `event:` field, if present. Use this to dispatch event types.
  final String? event;

  /// Joined payload (multiple `data:` lines are joined with `\n`).
  final String data;

  /// Decode [data] as JSON.
  dynamic decodeJson() => jsonDecode(data);

  @override
  String toString() => "SseEvent(event: $event, id: $id, data: $data)";
}

/// A proxy route definition.
class RouteConfig {
  const RouteConfig({
    required this.id,
    required this.path,
    required this.upstream,
    this.description = "",
    this.enabled = true,
    this.methods = const ["GET", "POST", "PUT", "DELETE", "PATCH"],
  });

  factory RouteConfig.fromJson(Map<String, dynamic> json) {
    return RouteConfig(
      id: json["id"] as String,
      path: json["path"] as String,
      upstream: json["upstream"] as String,
      description: (json["description"] as String?) ?? "",
      enabled: (json["enabled"] as bool?) ?? true,
      methods: ((json["methods"] as List?) ?? const [])
          .map((e) => e as String)
          .toList(growable: false),
    );
  }

  final String id;
  final String path;
  final String upstream;
  final String description;
  final bool enabled;
  final List<String> methods;

  Map<String, dynamic> toJson() => {
        "id": id,
        "path": path,
        "upstream": upstream,
        "description": description,
        "enabled": enabled,
        "methods": methods,
      };

  String encode() => jsonEncode(toJson());

  static RouteConfig parse(String body) =>
      RouteConfig.fromJson(jsonDecode(body) as Map<String, dynamic>);

  static List<RouteConfig> parseList(String body) {
    final decoded = jsonDecode(body);
    if (decoded is! List) {
      throw const FormatException("Expected a JSON array of routes");
    }
    return decoded
        .map((e) => RouteConfig.fromJson(e as Map<String, dynamic>))
        .toList(growable: false);
  }

  @override
  String toString() =>
      "RouteConfig(id: $id, path: $path, upstream: $upstream, enabled: $enabled)";
}

/// An API key record (masked).
class ApiKey {
  const ApiKey({
    required this.id,
    required this.name,
    required this.masked,
    this.createdAt,
    this.lastUsedAt,
  });

  factory ApiKey.fromJson(Map<String, dynamic> json) {
    return ApiKey(
      id: json["id"] as String,
      name: json["name"] as String,
      masked: json["masked"] as String,
      createdAt: json["createdAt"] as String?,
      lastUsedAt: json["lastUsedAt"] as String?,
    );
  }

  final String id;
  final String name;
  final String masked;
  final String? createdAt;
  final String? lastUsedAt;

  static List<ApiKey> parseList(String body) {
    final decoded = jsonDecode(body);
    if (decoded is! List) {
      throw const FormatException("Expected a JSON array of keys");
    }
    return decoded
        .map((e) => ApiKey.fromJson(e as Map<String, dynamic>))
        .toList(growable: false);
  }

  @override
  String toString() => "ApiKey(id: $id, name: $name, masked: $masked)";
}

/// Health probe result.
class HealthStatus {
  const HealthStatus({
    required this.healthy,
    required this.version,
    this.uptime = Duration.zero,
  });

  factory HealthStatus.fromJson(Map<String, dynamic> json) {
    return HealthStatus(
      healthy: (json["healthy"] as bool?) ?? false,
      version: (json["version"] as String?) ?? "unknown",
      uptime: Duration(
        seconds: ((json["uptimeSeconds"] as num?) ?? 0).toInt(),
      ),
    );
  }

  final bool healthy;
  final String version;
  final Duration uptime;

  static HealthStatus parse(String body) =>
      HealthStatus.fromJson(jsonDecode(body) as Map<String, dynamic>);

  @override
  String toString() =>
      "HealthStatus(healthy: $healthy, version: $version, uptime: $uptime)";
}
