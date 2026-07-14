-- Migration: pending_approvals table for durable cross-replica approval store.
-- Used by PostgresApprovalStore; MemoryApprovalStore does not require this table.

CREATE TABLE IF NOT EXISTS pending_approvals (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_sub     TEXT        NOT NULL,
    tool_name     TEXT        NOT NULL,
    reason        TEXT        NOT NULL DEFAULT '',
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL,
    decision      TEXT        CHECK (decision IN ('approved', 'rejected')),
    decided_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_pending_approvals_expires
    ON pending_approvals (expires_at)
    WHERE decision IS NULL;
