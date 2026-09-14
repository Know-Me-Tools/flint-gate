use super::{
    AuthorityCursor, AuthorityCursorStore, AuthorityError, AuthorityEvent, AuthorityEventKind,
    AuthorityHighWater, AuthoritySnapshot, AuthoritySource, AuthorityStamp, DeniedSession,
    DeniedSessionKey,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::time::Duration;

const ROLE_CHECK: &str = "
    SELECT pg_has_role(session_user, 'aso_authority_event_reader', 'MEMBER')
       AND NOT EXISTS (
         SELECT 1 FROM pg_roles r
         WHERE r.rolname IN (session_user, 'aso_authority_event_reader') AND (
           r.rolsuper OR r.rolbypassrls OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication
           OR (r.rolname = 'aso_authority_event_reader' AND r.rolcanlogin)
           OR has_schema_privilege(r.oid, 'aso', 'CREATE')
           OR has_database_privilege(r.oid, current_database(), 'CREATE')
           OR EXISTS (SELECT 1 FROM pg_namespace n WHERE n.nspname = 'aso'
                       AND pg_has_role(r.oid, n.nspowner, 'USAGE'))
           OR EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                       WHERE n.nspname = 'aso' AND (
                         pg_has_role(r.oid, c.relowner, 'USAGE') OR
                         (c.relkind IN ('r', 'p', 'v', 'm', 'f') AND (
                           has_table_privilege(r.oid, c.oid,
                             'INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER, MAINTAIN') OR
                           has_any_column_privilege(r.oid, c.oid, 'INSERT, UPDATE, REFERENCES')))))
           OR EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
                       WHERE n.nspname = 'aso' AND pg_has_role(r.oid, p.proowner, 'USAGE'))
         ))
       AND NOT EXISTS (
         SELECT 1 FROM pg_auth_members membership
         JOIN pg_roles granted ON granted.oid = membership.roleid
         WHERE (
           membership.member = (SELECT oid FROM pg_roles WHERE rolname = session_user)
           AND granted.rolname <> 'aso_authority_event_reader'
         ) OR membership.member = (
           SELECT oid FROM pg_roles WHERE rolname = 'aso_authority_event_reader'
         )
       )
       AND NOT EXISTS (
         SELECT 1
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE n.nspname = 'aso'
            AND (
              has_function_privilege(session_user, p.oid, 'EXECUTE')
              OR has_function_privilege('aso_authority_event_reader', p.oid, 'EXECUTE')
            )
       )
       AND NOT EXISTS (
         SELECT 1
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'aso'
            AND c.relkind = 'S'
            AND (
              has_sequence_privilege(session_user, c.oid, 'SELECT')
              OR has_sequence_privilege(session_user, c.oid, 'USAGE')
              OR has_sequence_privilege(session_user, c.oid, 'UPDATE')
              OR has_sequence_privilege('aso_authority_event_reader', c.oid, 'SELECT')
              OR has_sequence_privilege('aso_authority_event_reader', c.oid, 'USAGE')
              OR has_sequence_privilege('aso_authority_event_reader', c.oid, 'UPDATE')
            )
       )
       AND NOT EXISTS (
         SELECT 1
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'aso'
            AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
            AND c.relname NOT IN (
              'authority_deployment', 'authorization_revision',
              'session_denials', 'authority_outbox'
            )
            AND (
              has_table_privilege(session_user, c.oid, 'SELECT')
              OR has_table_privilege('aso_authority_event_reader', c.oid, 'SELECT')
              OR has_any_column_privilege(session_user, c.oid, 'SELECT')
              OR has_any_column_privilege('aso_authority_event_reader', c.oid, 'SELECT')
            )
       )";

#[derive(Clone)]
pub struct PostgresAuthoritySource {
    pool: PgPool,
}

impl PostgresAuthoritySource {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, AuthorityError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        let source = Self { pool };
        if !source.role_contract_satisfied().await? {
            return Err(AuthorityError::PrivilegeContract);
        }
        Ok(source)
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn role_contract_satisfied(&self) -> Result<bool, AuthorityError> {
        Ok(sqlx::query_scalar(ROLE_CHECK).fetch_one(&self.pool).await?)
    }

    async fn begin_scoped(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, AuthorityError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '200ms'")
            .execute(&mut *tx)
            .await?;
        let allowed: bool = sqlx::query_scalar(ROLE_CHECK).fetch_one(&mut *tx).await?;
        if !allowed {
            return Err(AuthorityError::PrivilegeContract);
        }
        sqlx::query("SET LOCAL ROLE aso_authority_event_reader")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL search_path = pg_catalog, aso")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    async fn read_high_water(
        tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> Result<AuthorityHighWater, AuthorityError> {
        let row = sqlx::query(
            "SELECT deployment.deployment_id, revision.incarnation,
                    revision.revision,
                    COALESCE(MAX(outbox.sequence), 0)::bigint AS outbox_sequence
               FROM aso.authority_deployment deployment
               CROSS JOIN aso.authorization_revision revision
               LEFT JOIN aso.authority_outbox outbox
                 ON outbox.deployment_id = deployment.deployment_id
                AND outbox.authority_incarnation = revision.incarnation
              WHERE deployment.singleton = true AND revision.singleton = true
              GROUP BY deployment.deployment_id, revision.incarnation, revision.revision",
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok(AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: row.try_get("deployment_id")?,
                incarnation: row.try_get("incarnation")?,
                revision: row.try_get("revision")?,
            },
            outbox_sequence: row.try_get("outbox_sequence")?,
        })
    }
}

#[async_trait]
impl AuthoritySource for PostgresAuthoritySource {
    async fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError> {
        let mut tx = self.begin_scoped().await?;
        let high_water = Self::read_high_water(&mut tx).await?;
        let observed_at: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows = sqlx::query(
            "SELECT kratos_issuer, kratos_session_id, retain_until
               FROM aso.session_denials
              WHERE deployment_id = $1 AND retain_until > transaction_timestamp()
              ORDER BY kratos_issuer, kratos_session_id",
        )
        .bind(high_water.stamp.deployment_id)
        .fetch_all(&mut *tx)
        .await?;
        let denied_sessions = rows
            .into_iter()
            .map(|row| {
                Ok(DeniedSession {
                    key: DeniedSessionKey {
                        issuer: row.try_get("kratos_issuer")?,
                        session_id: row.try_get("kratos_session_id")?,
                    },
                    retain_until: row.try_get("retain_until")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        tx.commit().await?;
        Ok(AuthoritySnapshot {
            high_water,
            observed_at,
            denied_sessions,
        })
    }

    async fn high_water(&self) -> Result<AuthorityHighWater, AuthorityError> {
        let mut tx = self.begin_scoped().await?;
        let high_water = Self::read_high_water(&mut tx).await?;
        tx.commit().await?;
        Ok(high_water)
    }

    async fn events_after(
        &self,
        sequence: i64,
        through: i64,
        limit: i64,
    ) -> Result<Vec<AuthorityEvent>, AuthorityError> {
        let mut tx = self.begin_scoped().await?;
        let rows = sqlx::query(
            "SELECT outbox.sequence, outbox.event_id, outbox.event_type,
                    outbox.deployment_id, outbox.authority_incarnation,
                    outbox.authorization_revision, outbox.kratos_issuer,
                    outbox.kratos_session_id, outbox.occurred_at,
                    denial.retain_until
               FROM aso.authority_outbox outbox
               JOIN aso.authority_deployment deployment
                 ON deployment.singleton = true
                AND deployment.deployment_id = outbox.deployment_id
               JOIN aso.authorization_revision revision
                 ON revision.singleton = true
                AND revision.incarnation = outbox.authority_incarnation
               LEFT JOIN aso.session_denials denial
                 ON denial.deployment_id = outbox.deployment_id
                AND denial.kratos_issuer = outbox.kratos_issuer
                AND denial.kratos_session_id = outbox.kratos_session_id
              WHERE outbox.sequence > $1 AND outbox.sequence <= $2
              ORDER BY outbox.sequence
              LIMIT $3",
        )
        .bind(sequence)
        .bind(through)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let event_type: String = row.try_get("event_type")?;
            let kind = match event_type.as_str() {
                "membership_revision" => AuthorityEventKind::MembershipRevision,
                "session_denied" => AuthorityEventKind::SessionDenied {
                    key: DeniedSessionKey {
                        issuer: row
                            .try_get::<Option<String>, _>("kratos_issuer")?
                            .ok_or(AuthorityError::InvalidEvent("denial issuer is missing"))?,
                        session_id: row
                            .try_get::<Option<String>, _>("kratos_session_id")?
                            .ok_or(AuthorityError::InvalidEvent("denial session is missing"))?,
                    },
                    retain_until: row.try_get("retain_until")?,
                },
                _ => return Err(AuthorityError::InvalidEvent("unknown event type")),
            };
            events.push(AuthorityEvent {
                event_id: row.try_get("event_id")?,
                sequence: row.try_get("sequence")?,
                stamp: AuthorityStamp {
                    deployment_id: row.try_get("deployment_id")?,
                    incarnation: row.try_get("authority_incarnation")?,
                    revision: row.try_get("authorization_revision")?,
                },
                occurred_at: row.try_get("occurred_at")?,
                kind,
            });
        }
        tx.commit().await?;
        Ok(events)
    }
}

#[derive(Clone)]
pub struct PostgresAuthorityCursorStore {
    pool: PgPool,
}

impl PostgresAuthorityCursorStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn cursor_from_row(row: sqlx::postgres::PgRow) -> Result<AuthorityCursor, AuthorityError> {
        Ok(AuthorityCursor {
            source_key: row.try_get("source_key")?,
            high_water: AuthorityHighWater {
                stamp: AuthorityStamp {
                    deployment_id: row.try_get("deployment_id")?,
                    incarnation: row.try_get("authority_incarnation")?,
                    revision: row.try_get("authorization_revision")?,
                },
                outbox_sequence: row.try_get("outbox_sequence")?,
            },
        })
    }
}

#[async_trait]
impl AuthorityCursorStore for PostgresAuthorityCursorStore {
    async fn load(
        &self,
        source_key: &str,
        stamp: &AuthorityStamp,
    ) -> Result<Option<AuthorityCursor>, AuthorityError> {
        let row = sqlx::query(
            "SELECT source_key, deployment_id, authority_incarnation,
                    authorization_revision, outbox_sequence
               FROM aso_authority_cursors
              WHERE source_key = $1 AND deployment_id = $2
                AND authority_incarnation = $3",
        )
        .bind(source_key)
        .bind(stamp.deployment_id)
        .bind(stamp.incarnation)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::cursor_from_row).transpose()
    }

    async fn store_snapshot(
        &self,
        source_key: &str,
        high_water: &AuthorityHighWater,
    ) -> Result<AuthorityCursor, AuthorityError> {
        let row = sqlx::query(
            "INSERT INTO aso_authority_cursors (
                 source_key, deployment_id, authority_incarnation,
                 authorization_revision, outbox_sequence
             ) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (source_key, deployment_id, authority_incarnation) DO UPDATE SET
                 authorization_revision = EXCLUDED.authorization_revision,
                 outbox_sequence = EXCLUDED.outbox_sequence,
                 updated_at = clock_timestamp()
             WHERE aso_authority_cursors.outbox_sequence <= EXCLUDED.outbox_sequence
             RETURNING source_key, deployment_id, authority_incarnation,
                       authorization_revision, outbox_sequence",
        )
        .bind(source_key)
        .bind(high_water.stamp.deployment_id)
        .bind(high_water.stamp.incarnation)
        .bind(high_water.stamp.revision)
        .bind(high_water.outbox_sequence)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            Self::cursor_from_row(row)
        } else {
            self.load(source_key, &high_water.stamp)
                .await?
                .ok_or(AuthorityError::CursorUnavailable)
        }
    }

    async fn advance(
        &self,
        source_key: &str,
        high_water: &AuthorityHighWater,
    ) -> Result<AuthorityCursor, AuthorityError> {
        let row = sqlx::query(
            "UPDATE aso_authority_cursors
                SET authorization_revision = GREATEST(authorization_revision, $4),
                    outbox_sequence = GREATEST(outbox_sequence, $5),
                    updated_at = clock_timestamp()
              WHERE source_key = $1 AND deployment_id = $2
                AND authority_incarnation = $3
              RETURNING source_key, deployment_id, authority_incarnation,
                        authorization_revision, outbox_sequence",
        )
        .bind(source_key)
        .bind(high_water.stamp.deployment_id)
        .bind(high_water.stamp.incarnation)
        .bind(high_water.stamp.revision)
        .bind(high_water.outbox_sequence)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::cursor_from_row)
            .transpose()?
            .ok_or(AuthorityError::CursorUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{
            AuthorityConsumer, AuthorityFenceStore, AuthorityReadiness, SharedAuthorityFence,
        },
        db::Database,
    };
    use std::sync::{Arc, Mutex};

    struct FixtureFence(Mutex<SharedAuthorityFence>);

    impl Default for FixtureFence {
        fn default() -> Self {
            Self(Mutex::new(SharedAuthorityFence::Missing))
        }
    }

    #[async_trait]
    impl AuthorityFenceStore for FixtureFence {
        async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn bootstrap(
            &self,
            observed: &SharedAuthorityFence,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            let mut current = self.0.lock().unwrap();
            if *current == SharedAuthorityFence::Present(candidate.clone()) {
                return Ok(());
            }
            if *current != *observed {
                return Err(AuthorityError::SharedFenceMismatch);
            }
            *current = SharedAuthorityFence::Present(candidate.clone());
            Ok(())
        }

        async fn advance(
            &self,
            expected: &AuthorityHighWater,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            let mut current = self.0.lock().unwrap();
            if *current != SharedAuthorityFence::Present(expected.clone()) {
                return Err(AuthorityError::SharedFenceMismatch);
            }
            *current = SharedAuthorityFence::Present(candidate.clone());
            Ok(())
        }

        async fn matches(&self, candidate: &AuthorityHighWater) -> Result<bool, AuthorityError> {
            Ok(*self.0.lock().unwrap() == SharedAuthorityFence::Present(candidate.clone()))
        }
    }

    fn required_url(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
    }

    async fn reader_query(source: &PostgresAuthoritySource, sql: &str) -> bool {
        let mut tx = source.pool.begin().await.expect("reader transaction");
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .expect("read-only transaction");
        sqlx::query("SET LOCAL ROLE aso_authority_event_reader")
            .execute(&mut *tx)
            .await
            .expect("reader role");
        sqlx::query(sql).execute(&mut *tx).await.is_ok()
    }

    /// Run through the prior-auth disposable fixture; never point this at a
    /// persistent database.
    #[tokio::test]
    #[ignore = "requires disposable ASO and Gate PostgreSQL databases"]
    async fn authority_reader_and_cursor_lifecycle() {
        let source_url = required_url("RA06C02_ASO_READER_DATABASE_URL");
        let owner_url = required_url("RA06C02_ASO_OWNER_DATABASE_URL");
        let gate_url = required_url("RA06C02_GATE_DATABASE_URL");
        let source = Arc::new(
            PostgresAuthoritySource::connect(&source_url, 2)
                .await
                .expect("restricted authority source"),
        );
        assert!(reader_query(&source, "SELECT count(*) FROM aso.authority_outbox").await);
        assert!(!reader_query(&source, "SELECT count(*) FROM aso.users").await);
        assert!(
            !reader_query(
                &source,
                "UPDATE aso.authorization_revision SET revision=revision"
            )
            .await
        );
        assert!(
            !reader_query(
                &source,
                "SELECT last_value FROM aso.authority_outbox_sequence_seq"
            )
            .await
        );
        println!("authority_consumer_check: least_privilege_reader");

        let gate_db = Database::connect(&gate_url, 2)
            .await
            .expect("Gate cursor database");
        gate_db.migrate().await.expect("Gate cursor migration");
        let cursors = Arc::new(PostgresAuthorityCursorStore::new(gate_db.pool()));
        let readiness = AuthorityReadiness::new();
        let consumer = AuthorityConsumer::new(
            "aso-test",
            source.clone(),
            cursors.clone(),
            Arc::new(FixtureFence::default()),
            readiness.clone(),
        );
        consumer.bootstrap().await.expect("authority bootstrap");
        assert!(readiness.cache_ready());
        println!("authority_consumer_check: snapshot_ready");

        let owner = PgPoolOptions::new()
            .max_connections(1)
            .connect(&owner_url)
            .await
            .expect("ASO owner connection");
        let mut owner_tx = owner.begin().await.expect("owner transaction");
        sqlx::query("GRANT CREATE ON SCHEMA aso TO aso_gate_owner")
            .execute(&mut *owner_tx)
            .await
            .expect("temporary function creation grant");
        sqlx::query("SET LOCAL ROLE aso_gate_owner")
            .execute(&mut *owner_tx)
            .await
            .expect("function owner role");
        sqlx::query(
            "CREATE FUNCTION aso.ra06c_reader_privilege_probe() RETURNS integer
             LANGUAGE sql IMMUTABLE AS 'SELECT 1'",
        )
        .execute(&mut *owner_tx)
        .await
        .expect("different-owner function");
        sqlx::query("RESET ROLE")
            .execute(&mut *owner_tx)
            .await
            .expect("restore owner role");
        sqlx::query("REVOKE CREATE ON SCHEMA aso FROM aso_gate_owner")
            .execute(&mut *owner_tx)
            .await
            .expect("remove function creation grant");
        owner_tx.commit().await.expect("commit function fixture");
        PostgresAuthoritySource::connect(&source_url, 1)
            .await
            .expect("different-owner default privileges remain restricted");
        sqlx::query("GRANT EXECUTE ON FUNCTION aso.ra06c_reader_privilege_probe() TO PUBLIC")
            .execute(&owner)
            .await
            .expect("inject effective PUBLIC function privilege");
        assert!(matches!(
            PostgresAuthoritySource::connect(&source_url, 1).await,
            Err(AuthorityError::PrivilegeContract)
        ));
        sqlx::query("REVOKE EXECUTE ON FUNCTION aso.ra06c_reader_privilege_probe() FROM PUBLIC")
            .execute(&owner)
            .await
            .expect("remove effective PUBLIC function privilege");
        sqlx::query("DROP FUNCTION aso.ra06c_reader_privilege_probe()")
            .execute(&owner)
            .await
            .expect("drop function fixture");
        sqlx::query("CREATE SEQUENCE aso.ra06c_reader_privilege_probe_seq")
            .execute(&owner)
            .await
            .expect("later sequence fixture");
        sqlx::query("GRANT USAGE ON SEQUENCE aso.ra06c_reader_privilege_probe_seq TO PUBLIC")
            .execute(&owner)
            .await
            .expect("inject effective PUBLIC sequence privilege");
        assert!(matches!(
            PostgresAuthoritySource::connect(&source_url, 1).await,
            Err(AuthorityError::PrivilegeContract)
        ));
        sqlx::query("REVOKE USAGE ON SEQUENCE aso.ra06c_reader_privilege_probe_seq FROM PUBLIC")
            .execute(&owner)
            .await
            .expect("remove effective PUBLIC sequence privilege");
        sqlx::query(
            "GRANT SELECT ON SEQUENCE aso.ra06c_reader_privilege_probe_seq
             TO aso_authority_event_reader",
        )
        .execute(&owner)
        .await
        .expect("inject direct reader sequence privilege");
        assert!(matches!(
            PostgresAuthoritySource::connect(&source_url, 1).await,
            Err(AuthorityError::PrivilegeContract)
        ));
        sqlx::query(
            "REVOKE SELECT ON SEQUENCE aso.ra06c_reader_privilege_probe_seq
             FROM aso_authority_event_reader",
        )
        .execute(&owner)
        .await
        .expect("remove direct reader sequence privilege");
        sqlx::query("DROP SEQUENCE aso.ra06c_reader_privilege_probe_seq")
            .execute(&owner)
            .await
            .expect("drop sequence fixture");
        let drift_role = format!("ra06c_reader_drift_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE ROLE {drift_role} NOLOGIN"))
            .execute(&owner)
            .await
            .expect("create reader membership drift fixture");
        sqlx::query(&format!("GRANT {drift_role} TO aso_authority_event_reader"))
            .execute(&owner)
            .await
            .expect("inject reader membership drift");
        assert!(matches!(
            PostgresAuthoritySource::connect(&source_url, 1).await,
            Err(AuthorityError::PrivilegeContract)
        ));
        sqlx::query(&format!(
            "REVOKE {drift_role} FROM aso_authority_event_reader"
        ))
        .execute(&owner)
        .await
        .expect("remove reader membership drift");
        sqlx::query(&format!("DROP ROLE {drift_role}"))
            .execute(&owner)
            .await
            .expect("drop reader membership drift fixture");
        sqlx::query("GRANT SELECT ON aso.users TO aso_authority_event_reader")
            .execute(&owner)
            .await
            .expect("inject excess reader grant");
        assert!(matches!(
            PostgresAuthoritySource::connect(&source_url, 1).await,
            Err(AuthorityError::PrivilegeContract)
        ));
        sqlx::query("REVOKE SELECT ON aso.users FROM aso_authority_event_reader")
            .execute(&owner)
            .await
            .expect("remove excess reader grant");
        let deployment: uuid::Uuid = sqlx::query_scalar(
            "SELECT deployment_id FROM aso.authority_deployment WHERE singleton=true",
        )
        .fetch_one(&owner)
        .await
        .expect("deployment identity");
        sqlx::query(
            "INSERT INTO aso.session_denials (
                 deployment_id, kratos_issuer, kratos_session_id,
                 session_expires_at, retain_until, next_attempt_at
             ) SELECT
                 $1, 'https://kratos.test', 'session-after-snapshot',
                 expiration, expiration + interval '1 second', clock_timestamp()
               FROM (SELECT clock_timestamp() + interval '60 seconds' AS expiration) value",
        )
        .bind(deployment)
        .execute(&owner)
        .await
        .expect("committed denial after bootstrap");

        consumer.synchronize().await.expect("authority catch-up");
        assert!(readiness.cache_ready());
        assert!(readiness.is_denied("https://kratos.test", "session-after-snapshot", Utc::now()));
        let applied = readiness.snapshot().high_water.expect("applied high-water");
        let durable = cursors
            .load("aso-test", &applied.stamp)
            .await
            .expect("cursor lookup")
            .expect("durable cursor");
        assert_eq!(durable.high_water, applied);
        let replacement = AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: applied.stamp.deployment_id,
                incarnation: uuid::Uuid::from_u128(applied.stamp.incarnation.as_u128() ^ 1),
                revision: 1,
            },
            outbox_sequence: 0,
        };
        cursors
            .store_snapshot("aso-test", &replacement)
            .await
            .expect("replacement namespace cursor");
        cursors
            .advance("aso-test", &applied)
            .await
            .expect("old namespace remains isolated");
        assert!(cursors
            .load("aso-test", &replacement.stamp)
            .await
            .unwrap()
            .is_some());
        assert!(cursors
            .load("aso-test", &applied.stamp)
            .await
            .unwrap()
            .is_some());
        println!("authority_consumer_check: committed_event_replayed_and_cursor_durable");

        let restarted = AuthorityReadiness::new();
        assert!(!restarted.cache_ready());
        assert!(cursors
            .load("aso-test", &applied.stamp)
            .await
            .unwrap()
            .is_some());
        println!("authority_consumer_check: restart_requires_local_bootstrap");
    }
}
