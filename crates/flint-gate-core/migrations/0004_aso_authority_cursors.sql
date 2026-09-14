-- Durable shared watermark for the ASO authority stream. Every Gate process
-- still performs its own snapshot and catch-up before its local cache becomes
-- ready; this row cannot attest to process-local or Redis freshness.
CREATE TABLE IF NOT EXISTS aso_authority_cursors (
    source_key             TEXT        NOT NULL CHECK (btrim(source_key) <> ''),
    deployment_id          UUID        NOT NULL,
    authority_incarnation  UUID        NOT NULL,
    authorization_revision BIGINT      NOT NULL CHECK (authorization_revision > 0),
    outbox_sequence        BIGINT      NOT NULL CHECK (outbox_sequence >= 0),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (source_key, deployment_id, authority_incarnation)
);
