-- Hydra's DSN sets search_path=hydra, so its migrations need the schema to
-- exist; Postgres runs this once when the data volume is first initialized.
CREATE SCHEMA IF NOT EXISTS hydra;
