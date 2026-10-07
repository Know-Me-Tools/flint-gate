-- Fixed claims a client_credentials token carries in addition to client_id,
-- scope and aud (for example `{"role": "service_role"}` so a downstream
-- verifier that keys on a `role` claim can authorize a service client).
-- Reserved claims (iss, sub, aud, exp, iat, nbf, jti, flint_kind, client_id,
-- scope) are never taken from this column; the gateway ignores them at mint.
ALTER TABLE oauth_clients
    ADD COLUMN IF NOT EXISTS claims JSONB NOT NULL DEFAULT '{}'::jsonb;
