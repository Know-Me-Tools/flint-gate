-- A single ordered revision serializes every configuration mutation with its
-- commit. LISTEN/NOTIFY consumers advance the shared Redis configuration epoch
-- for every greater revision they observe. Sequential notifications advance
-- once each; restart reconciliation may safely coalesce revisions that this
-- replica never published, while duplicate or reordered delivery is idempotent.
CREATE TABLE IF NOT EXISTS config_revision (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    revision  BIGINT  NOT NULL CHECK (revision >= 0)
);

INSERT INTO config_revision (singleton, revision)
VALUES (TRUE, 0)
ON CONFLICT (singleton) DO NOTHING;
