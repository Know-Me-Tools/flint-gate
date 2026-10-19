-- Channel authority is owned by Gate, not supplied as GrantRef facts by a caller.
CREATE TABLE governed_channel_grants (
    issuer TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    active BOOLEAN NOT NULL,
    specification JSONB NOT NULL,
    changed_by_issuer TEXT NOT NULL,
    changed_by_subject TEXT NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (issuer, grant_id)
);

CREATE TABLE governed_channel_effect_receipts (
    receipt_id UUID PRIMARY KEY,
    effect_id UUID NOT NULL,
    action_id UUID NOT NULL,
    stage TEXT NOT NULL CHECK (stage IN ('evaluate', 'release')),
    issuer TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    grant_revision BIGINT NOT NULL,
    action TEXT NOT NULL,
    occurrence_id TEXT NOT NULL,
    request_sha256 TEXT NOT NULL CHECK (char_length(request_sha256) = 64),
    policy_set_id TEXT NOT NULL,
    policy_revision TEXT NOT NULL,
    policy_digest TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('eligible', 'released', 'withheld')),
    reason TEXT NOT NULL,
    evaluated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_at TIMESTAMPTZ,
    UNIQUE (effect_id, stage),
    UNIQUE (issuer, action_id, action, stage)
);

CREATE INDEX governed_channel_receipts_occurrence
    ON governed_channel_effect_receipts (issuer, occurrence_id);
