CREATE TABLE IF NOT EXISTS governed_effect_approvals (
    issuer              TEXT        NOT NULL,
    challenge_id        UUID        NOT NULL,
    effect_id           UUID        NOT NULL,
    invocation_id       UUID        NOT NULL,
    request_json        JSONB       NOT NULL,
    request_sha256      TEXT        NOT NULL,
    authority_binding   JSONB       NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    decision            TEXT        CHECK (decision IN ('approve', 'deny')),
    decided_at          TIMESTAMPTZ,
    decided_by_issuer   TEXT,
    decided_by_subject  TEXT,
    PRIMARY KEY (issuer, challenge_id),
    CHECK (char_length(request_sha256) = 64),
    CHECK (
        (decision IS NULL AND decided_at IS NULL AND decided_by_issuer IS NULL AND decided_by_subject IS NULL)
        OR
        (decision IS NOT NULL AND decided_at IS NOT NULL AND decided_by_issuer IS NOT NULL AND decided_by_subject IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_governed_effect_approvals_pending
    ON governed_effect_approvals (expires_at)
    WHERE decision IS NULL;
