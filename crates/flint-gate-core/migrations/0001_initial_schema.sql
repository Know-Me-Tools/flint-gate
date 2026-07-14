CREATE TABLE IF NOT EXISTS gate_routes (
    id          TEXT PRIMARY KEY,
    config      JSONB NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 0,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS gate_sites (
    id              TEXT PRIMARY KEY,
    domains         JSONB NOT NULL DEFAULT '[]',
    default_auth    TEXT,
    default_upstream TEXT,
    config          JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS api_keys (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash    TEXT NOT NULL UNIQUE,
    client_id   TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT 'service_role',
    principal_type TEXT NOT NULL DEFAULT 'Service',
    key_prefix  TEXT,
    scopes      JSONB NOT NULL DEFAULT '[]',
    active      BOOLEAN NOT NULL DEFAULT true,
    expires_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS usage_events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id  TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    route_id    TEXT NOT NULL,
    tokens      BIGINT NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    metadata    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS jwt_signing_keys (
    id          TEXT PRIMARY KEY,
    algorithm   TEXT NOT NULL,
    public_key  TEXT NOT NULL,
    private_key TEXT NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS authz_policies (
    id            TEXT PRIMARY KEY,
    policy_text   TEXT NOT NULL,
    schema_json   JSONB,
    entities_json JSONB,
    enabled       BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS authz_audit (
    id          UUID PRIMARY KEY,
    request_id  TEXT,
    principal   TEXT NOT NULL,
    action      TEXT NOT NULL,
    resource    TEXT NOT NULL,
    decision    TEXT NOT NULL,
    reason      TEXT,
    context     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS authz_audit_created_at_idx ON authz_audit (created_at DESC);
CREATE INDEX IF NOT EXISTS authz_audit_principal_idx ON authz_audit (principal);

CREATE TABLE IF NOT EXISTS oauth_clients (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    client_id      TEXT NOT NULL UNIQUE,
    secret_hash    TEXT NOT NULL,
    scopes         JSONB NOT NULL DEFAULT '[]',
    audience       TEXT,
    active         BOOLEAN NOT NULL DEFAULT true,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS agent_identities (
    id             TEXT PRIMARY KEY,
    kind           TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active',
    label          TEXT,
    rotated_at     TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS agent_identities_status_idx ON agent_identities (status);

CREATE TABLE IF NOT EXISTS cedar_policy_versions (
    id            SERIAL PRIMARY KEY,
    policy_id     TEXT NOT NULL REFERENCES authz_policies(id) ON DELETE CASCADE,
    version_num   INT  NOT NULL,
    policy_text   TEXT NOT NULL,
    schema_json   JSONB,
    entities_json JSONB,
    written_by    TEXT,
    written_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (policy_id, version_num)
);

CREATE INDEX IF NOT EXISTS cedar_policy_versions_policy_id_idx
    ON cedar_policy_versions (policy_id, version_num DESC);
