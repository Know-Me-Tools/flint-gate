export interface RouteMatch {
  path: string;
  methods?: string[];
  host?: string;
}

export interface StreamConfig {
  enabled?: boolean;
  protocol?: string;
  ai?: {
    ag_ui?: { enabled?: boolean };
    a2ui?: { enabled?: boolean };
    backpressure?: { enabled?: boolean };
  };
}

export interface HooksConfig {
  pre_request: PreRequestHook[];
  post_response: PostResponseHook[];
}

export interface PreRequestHook {
  type: 'claims_enhancement' | 'body_transform' | 'max_token_budget' | 'authorize' | 'guardrail';
  config?: Record<string, unknown>;
}

export interface PostResponseHook {
  type: 'stream_meter';
  config?: Record<string, unknown>;
}

export interface RouteConfig {
  id: string;
  site: string;
  match: RouteMatch;
  upstream?: string;
  auth?: string;
  hooks?: HooksConfig;
  stream?: StreamConfig;
  priority?: number;
  enabled?: boolean;
}

export interface DbRoute {
  id: string;
  config: RouteConfig;
  priority: number;
  enabled: boolean;
}

export interface RouteListResponse {
  routes: DbRoute[];
  source: string;
  note?: string;
}

export interface PolicyRow {
  id: string;
  policy_text: string;
  schema_json?: Record<string, unknown> | null;
  entities_json?: Record<string, unknown> | null;
  enabled: boolean;
}

export interface PolicyListResponse {
  policies: PolicyRow[];
}

export interface ApiKey {
  id: string;
  client_id: string;
  scopes: string[];
  expires_at?: string | null;
}

export interface ApiKeyListResponse {
  api_keys: ApiKey[];
}

export interface ApiKeyCreateRequest {
  client_id: string;
  scopes: string[];
  expires_at?: string | null;
}

export interface ApiKeyCreatedResponse extends ApiKey {
  key: string;
  note: string;
}

export type AuthProviderConfig =
  | { type: 'kratos'; base_url: string; forward_cookies?: boolean; session_cookie?: string }
  | { type: 'jwt'; jwks_url: string; issuer?: string; audience?: string; leeway_seconds?: number }
  | { type: 'mcp'; jwks_url: string; issuer?: string; audience?: string; resource: string; authorization_servers?: string[]; required_scopes?: string[]; leeway_seconds?: number }
  | { type: 'api_key'; header?: string; store?: string }
  | { type: 'anonymous'; default_subject?: string };

export interface SiteConfig {
  id: string;
  domains?: string[];
  default_auth?: string;
  default_upstream?: string;
}

export interface GateConfig {
  auth_providers: Record<string, AuthProviderConfig>;
  sites: SiteConfig[];
  routes?: RouteConfig[];
  jwt?: {
    signing_algorithm?: string;
    issuer?: string;
    default_ttl_seconds?: number;
  };
}

export interface ConfigResponse extends GateConfig {}

export interface HealthResponse {
  status: string;
  service?: string;
}

export interface ReadyResponse {
  status: string;
  db?: string;
}
