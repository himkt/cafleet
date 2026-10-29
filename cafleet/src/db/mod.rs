use rusqlite::{Connection, OptionalExtension};

use crate::error::CafleetError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

pub fn migration_chain() -> Vec<(u32, String)> {
    let mut chain: Vec<(u32, String)> = embedded::migrations::runner()
        .get_migrations()
        .iter()
        .map(|migration| {
            let version = u32::try_from(migration.version())
                .expect("embedded migration versions are positive");
            (version, migration.name().to_string())
        })
        .collect();
    chain.sort();
    chain
}

/// The file path a `sqlite:///<path>` database URL names.
pub fn database_path(database_url: &str) -> Result<&str, CafleetError> {
    database_url.strip_prefix("sqlite:///").ok_or_else(|| {
        CafleetError::App(format!(
            "database URL must use the sqlite scheme (sqlite:///<path>); got '{database_url}'"
        ))
    })
}

pub fn connect(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::default())
}

pub(crate) fn connect_read_only(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn connect_with_flags(
    database_url: &str,
    flags: rusqlite::OpenFlags,
) -> Result<Connection, CafleetError> {
    let path = database_path(database_url)?;
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| CafleetError::App(format!("failed to open database at '{path}': {e}")))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .map_err(|e| CafleetError::App(format!("failed to apply connection PRAGMAs: {e}")))?;
    Ok(conn)
}

/// Apply the embedded migration chain to head (idempotent) and return the
/// head version.
pub fn migrate_to_head(conn: &mut Connection) -> Result<u32, CafleetError> {
    embedded::migrations::runner()
        .set_grouped(true)
        .run(conn)
        .map_err(|e| CafleetError::App(format!("migration failed: {e}")))?;
    Ok(head_version())
}

/// The embedded chain's head version.
pub fn head_version() -> u32 {
    migration_chain()
        .last()
        .expect("the embedded migration chain has at least the baseline")
        .0
}

pub const APPLICATION_ID: i64 = 1128351288;

fn metadata_rows(
    conn: &Connection,
    sql: &str,
    name: &str,
) -> Result<Vec<Vec<rusqlite::types::Value>>, CafleetError> {
    let mut statement = conn.prepare(sql).map_err(crate::broker::members::db_err)?;
    let rows = statement
        .query_map([name], |row| {
            (0..row.as_ref().column_count())
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(crate::broker::members::db_err)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(crate::broker::members::db_err)
}

const COLUMNS: &str =
    "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid";
const FOREIGN_KEYS: &str = "SELECT \"table\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?1) ORDER BY \"from\", \"to\"";
const INDEX_COLUMNS: &str = "SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno";
type Metadata = Vec<Vec<rusqlite::types::Value>>;

struct SchemaIndex {
    name: String,
    unique: i64,
    partial: i64,
    columns: Metadata,
}

struct SchemaTable {
    name: String,
    columns: Metadata,
    foreign_keys: Metadata,
    indexes: Vec<SchemaIndex>,
}

impl SchemaTable {
    fn matches(&self, conn: &Connection) -> Result<bool, CafleetError> {
        if metadata_rows(conn, COLUMNS, &self.name)? != self.columns
            || metadata_rows(conn, FOREIGN_KEYS, &self.name)? != self.foreign_keys
        {
            return Ok(false);
        }
        for index in &self.indexes {
            let definition: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT \"unique\", partial FROM pragma_index_list(?1) WHERE name=?2",
                    rusqlite::params![self.name, index.name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(crate::broker::members::db_err)?;
            if definition != Some((index.unique, index.partial))
                || metadata_rows(conn, INDEX_COLUMNS, &index.name)? != index.columns
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

struct SchemaShape(Vec<SchemaTable>);

impl SchemaShape {
    fn matches(&self, conn: &Connection) -> Result<bool, CafleetError> {
        for table in &self.0 {
            if !table.matches(conn)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn read_schema(conn: &Connection) -> Result<SchemaShape, CafleetError> {
    let mut tables = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).map_err(crate::broker::members::db_err)?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(crate::broker::members::db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::broker::members::db_err)?;
    let mut schema = Vec::new();
    for name in names {
        let mut indexes = conn
            .prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1)")
            .map_err(crate::broker::members::db_err)?;
        let indexes = indexes
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(crate::broker::members::db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::broker::members::db_err)?;
        let indexes = indexes
            .into_iter()
            .map(|(name, unique, partial)| {
                Ok(SchemaIndex {
                    columns: metadata_rows(conn, INDEX_COLUMNS, &name)?,
                    name,
                    unique,
                    partial,
                })
            })
            .collect::<Result<Vec<_>, CafleetError>>()?;
        schema.push(SchemaTable {
            columns: metadata_rows(conn, COLUMNS, &name)?,
            foreign_keys: metadata_rows(conn, FOREIGN_KEYS, &name)?,
            name,
            indexes,
        });
    }
    Ok(SchemaShape(schema))
}

fn reference_schema(version: u32) -> Result<SchemaShape, CafleetError> {
    let mut expected = Connection::open_in_memory().map_err(crate::broker::members::db_err)?;
    embedded::migrations::runner()
        .set_grouped(true)
        .set_target(refinery::Target::Version(
            i32::try_from(version).expect("supported version fits i32"),
        ))
        .run(&mut expected)
        .map_err(|error| CafleetError::App(format!("reference migration failed: {error}")))?;
    read_schema(&expected)
}

pub(crate) fn schema_matches_version(
    conn: &Connection,
    version: u32,
) -> Result<bool, CafleetError> {
    if version == 0 {
        return Ok(false);
    }
    let head = head_version();
    let reference = reference_schema(version.min(head))?;
    let snapshot = if conn.is_autocommit() {
        Some(
            conn.unchecked_transaction()
                .map_err(crate::broker::members::db_err)?,
        )
    } else {
        None
    };
    let matches = if version > head {
        reference
            .0
            .iter()
            .find(|table| table.name == "refinery_schema_history")
            .expect("refinery migrations create their ledger")
            .matches(conn)?
    } else {
        reference.matches(conn)?
    };
    if let Some(snapshot) = snapshot {
        snapshot.commit().map_err(crate::broker::members::db_err)?;
    }
    Ok(matches)
}

#[cfg(test)]
mod runtime_tests {
    use rusqlite::{Connection, params};
    use serde_json::json;
    use tempfile::TempDir;

    use crate::broker::tests as common;

    mod permission_migration {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use rusqlite::types::Value;

        use super::*;

        fn v4_fixture() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let mut conn =
                crate::db::connect(&format!("sqlite:///{}", dir.path().join("v4.db").display()))
                    .unwrap();
            crate::db::embedded::migrations::runner()
                .set_grouped(true)
                .set_target(refinery::Target::Version(4))
                .run(&mut conn)
                .unwrap();
            let (fleet, director) = common::create_fleet(&mut conn, "permission-upgrade");
            let member = common::register(&mut conn, fleet, "retained", Some("%2"));
            Fixture {
                _dir: dir,
                conn,
                fleet,
                director,
                member,
            }
        }

        fn query_rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
            let mut statement = conn.prepare(sql).unwrap();
            let width = statement.column_count();
            statement
                .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }

        fn retained_rows(conn: &Connection) -> Vec<(String, String, Vec<Vec<Value>>)> {
            let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name != 'refinery_schema_history' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
            tables
                .into_iter()
                .map(|table| {
                    let columns = conn
                        .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
                        .unwrap()
                        .query_map([&table], |row| row.get::<_, String>(0))
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap()
                        .join(",");
                    let rows =
                        query_rows(conn, &format!("SELECT {columns} FROM {table} ORDER BY 1"));
                    (table, columns, rows)
                })
                .collect()
        }

        fn assert_retained(conn: &Connection, before: &[(String, String, Vec<Vec<Value>>)]) {
            for (table, columns, expected) in before {
                assert_eq!(
                    query_rows(conn, &format!("SELECT {columns} FROM {table} ORDER BY 1")),
                    *expected,
                    "{table}"
                );
            }
        }

        fn seed_retained(fixture: &mut Fixture) -> i64 {
            let run = fixture.run("running").unwrap();
            let interaction = fixture.interaction(run, 1, "permission_request").unwrap();
            let event = fixture.archived_event(run, 1).unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET interaction_id=?1 WHERE event_id=?2",
                    params![interaction, event],
                )
                .unwrap();
            for (key, kind, state, disposition) in [
                ("retained:ready", "prompt", "ready", None),
                ("retained:accepted", "exec", "accepted", None),
                ("retained:uncertain", "prompt", "uncertain", None),
                ("retained:settled", "exec", "settled", Some("submitted")),
            ] {
                fixture.conn.execute(
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref) VALUES(?1,?2,?3,?4,?5,?6,?6,?7,'fixture:evidence')",
            params![run,key,kind,"d".repeat(64),state,NOW,disposition],
        ).unwrap();
            }
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.director,
                fixture.member,
                "retained message bytes 世界",
            )
            .unwrap();
            fixture.conn.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id) VALUES(?1,?2,'preview',?3,'in_progress',?4,?4,?5)",params![run,format!("preview:{run}:{}",message.message_id),"d".repeat(64),NOW,message.message_id]).unwrap();
            let deleted = fixture.input(run, "retained:deleted-high-water").unwrap();
            fixture
                .conn
                .execute(
                    "DELETE FROM member_input_operations WHERE input_id=?1",
                    [deleted],
                )
                .unwrap();
            run
        }

        fn require_v5(conn: &Connection) {
            assert!(
                crate::db::head_version() >= 5,
                "the contiguous permission migration is required"
            );
            for table in [
                "member_permission_responses",
                "member_permission_response_claims",
            ] {
                let exists: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        [table],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(exists, 1, "{table}");
            }
        }

        #[test]
        fn runtime_v5_permission_migration_is_the_contiguous_successor_to_v4() {
            let fixture = Fixture::new();
            require_v5(&fixture.conn);
            let chain = crate::db::migration_chain();
            assert!(chain.iter().any(|(version, _)| *version == 5));
            assert_eq!(
                chain
                    .iter()
                    .map(|(version, _)| *version)
                    .collect::<Vec<_>>(),
                (1..=crate::db::head_version()).collect::<Vec<_>>()
            );
            assert_eq!(
                fixture
                    .conn
                    .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                crate::db::APPLICATION_ID
            );
        }

        #[test]
        fn runtime_v5_upgrade_preserves_linked_rows_and_input_high_water() {
            let mut fixture = v4_fixture();
            let run = seed_retained(&mut fixture);
            let before = retained_rows(&fixture.conn);
            let high_water: i64 = fixture
                .conn
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='member_input_operations'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_retained(&fixture.conn, &before);
            assert_eq!(fixture.input(run, "after-upgrade").unwrap(), high_water + 1);
            require_v5(&fixture.conn);
        }

        #[test]
        fn runtime_v5_repeated_migration_and_reconnect_preserve_identity_and_rows() {
            let mut fixture = v4_fixture();
            seed_retained(&mut fixture);
            let instance: String = fixture
                .conn
                .query_row(
                    "SELECT instance_id FROM runtime_database_identity",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            let before = retained_rows(&fixture.conn);
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            let reopened =
                crate::db::connect(&format!("sqlite:///{}", fixture.conn.path().unwrap())).unwrap();
            assert_retained(&reopened, &before);
            assert_eq!(
                reopened
                    .query_row(
                        "SELECT instance_id FROM runtime_database_identity",
                        [],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                instance
            );
            require_v5(&reopened);
        }

        #[test]
        fn runtime_v5_audit_and_claim_columns_match_the_shipping_contract() {
            let fixture = Fixture::new();
            require_v5(&fixture.conn);
            for (table, expected) in [
                (
                    "member_permission_responses",
                    "input_id:INTEGER event_id:INTEGER interaction_id:INTEGER expected_owner_epoch:INTEGER request_token:TEXT decision:TEXT director_member_id:INTEGER director_mode:TEXT decision_source:TEXT mode_evidence_ref:TEXT decision_evidence_ref:TEXT answer_profile_version:TEXT answer_rule_id:TEXT decided_at:TEXT",
                ),
                (
                    "member_permission_response_claims",
                    "interaction_id:INTEGER input_id:INTEGER",
                ),
            ] {
                let columns = fixture
                    .conn
                    .prepare(
                        "SELECT name,type,\"notnull\",pk FROM pragma_table_info(?1) ORDER BY cid",
                    )
                    .unwrap()
                    .query_map([table], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                assert_eq!(
                    columns
                        .iter()
                        .map(|(name, kind, _, _)| format!("{name}:{kind}"))
                        .collect::<Vec<_>>(),
                    expected.split_whitespace().collect::<Vec<_>>(),
                    "{table}"
                );
                for (_, _, required, primary) in columns {
                    assert!(
                        required == 1 || primary == 1,
                        "{table}: every audit/claim field is required"
                    );
                }
                let foreign_keys = query_rows(
                    &fixture.conn,
                    &format!(
                        "SELECT \"table\",\"from\",\"to\",on_delete FROM pragma_foreign_key_list('{table}') ORDER BY id,seq"
                    ),
                );
                assert!(!foreign_keys.is_empty());
                assert!(
                    foreign_keys
                        .iter()
                        .all(|row| row[3] == Value::Text("RESTRICT".into()))
                );
            }
        }

        #[test]
        fn runtime_v5_input_kind_expansion_preserves_existing_sql_constraints() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            for kind in ["prompt", "exec", "permission_response"] {
                fixture.conn.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,?2,?2,?3,'in_progress',?4,?4)",params![run,kind,"d".repeat(64),NOW]).unwrap();
            }
            assert!(fixture.conn.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,'preview:no-message','preview',?2,'in_progress',?3,?3)",params![run,"d".repeat(64),NOW]).is_err());
            let original = query_rows(
                &fixture.conn,
                "SELECT * FROM member_input_operations ORDER BY input_id",
            );
            for mutation in [
                "UPDATE member_input_operations SET kind='invalid'",
                "UPDATE member_input_operations SET state='settled'",
                "UPDATE member_input_operations SET disposition='submitted'",
                "UPDATE member_input_operations SET run_id=99999",
                "UPDATE member_input_operations SET payload_digest='invalid'",
                "UPDATE member_input_operations SET operation_key=' '",
            ] {
                assert!(fixture.conn.execute(mutation, []).is_err(), "{mutation}");
                assert_eq!(
                    query_rows(
                        &fixture.conn,
                        "SELECT * FROM member_input_operations ORDER BY input_id"
                    ),
                    original
                );
            }
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
        }

        #[test]
        fn runtime_v5_guard_requires_each_new_table_and_required_column() {
            for damage in [
                "DROP TABLE member_permission_response_claims",
                "DROP TABLE member_permission_responses",
                "ALTER TABLE member_permission_responses DROP COLUMN answer_rule_id",
            ] {
                let fixture = Fixture::new();
                require_v5(&fixture.conn);
                fixture.conn.execute_batch(damage).unwrap();
                assert!(
                    crate::diagnosis::classify_schema(&fixture.conn, crate::db::head_version())
                        .require_current()
                        .is_err(),
                    "{damage}"
                );
            }
        }

        #[test]
        fn runtime_v5_failed_migration_retains_v4_schema_ledger_and_rows_for_retry() {
            let mut fixture = v4_fixture();
            seed_retained(&mut fixture);
            let before = retained_rows(&fixture.conn);
            let schema = query_rows(
                &fixture.conn,
                "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name",
            );
            let ledger = query_rows(
                &fixture.conn,
                "SELECT * FROM refinery_schema_history ORDER BY version",
            );
            fixture
                .conn
                .authorizer(Some(|context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::CreateTable {
                            table_name: "member_permission_response_claims"
                        }
                    ) {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }))
                .unwrap();
            let result = crate::db::migrate_to_head(&mut fixture.conn);
            fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert!(
                result.is_err(),
                "the denied V5 schema operation must fail migration"
            );
            assert!(fixture.conn.is_autocommit());
            assert_retained(&fixture.conn, &before);
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name"
                ),
                schema
            );
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT * FROM refinery_schema_history ORDER BY version"
                ),
                ledger
            );
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_retained(&fixture.conn, &before);
            require_v5(&fixture.conn);
        }

        #[test]
        fn runtime_v5_ahead_schema_refusal_preserves_database_bytes() {
            let fixture = Fixture::new();
            require_v5(&fixture.conn);
            let head = crate::db::head_version();
            fixture.conn.execute("INSERT INTO refinery_schema_history(version,name,applied_on,checksum) VALUES(?1,'future','2026-10-26T00:00:00Z','1')",[head + 1]).unwrap();
            let path = fixture.conn.path().unwrap();
            let before = std::fs::read(path).unwrap();
            assert!(
                crate::diagnosis::classify_schema(&fixture.conn, head)
                    .require_current()
                    .is_err()
            );
            assert_eq!(std::fs::read(path).unwrap(), before);
        }

        fn insert_audit(fixture: &Fixture, input: i64, event: i64, interaction: i64) {
            fixture.conn.execute("INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) VALUES(?1,?2,?3,1,?4,'allow',?5,'auto','director-auto','fixture:mode','fixture:decision','answer-fixture-v1','single-request-choice',?6)",params![input,event,interaction,"a".repeat(64),fixture.director,NOW]).unwrap();
        }

        fn raw_response() -> (Fixture, i64, i64, i64, i64) {
            let fixture = Fixture::new();
            require_v5(&fixture.conn);
            let run = fixture.run("running").unwrap();
            let interaction = fixture.interaction(run, 1, "permission_request").unwrap();
            let event = fixture.archived_event(run, 1).unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET interaction_id=?1 WHERE event_id=?2",
                    params![interaction, event],
                )
                .unwrap();
            let input = fixture.input(run, "sql-control").unwrap();
            fixture
        .conn
        .execute(
            "UPDATE member_input_operations SET kind='permission_response' WHERE input_id=?1",
            [input],
        )
        .unwrap();
            insert_audit(&fixture, input, event, interaction);
            fixture
        .conn
        .execute(
            "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES(?1,?2)",
            params![interaction, input],
        )
        .unwrap();
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
            (fixture, run, event, interaction, input)
        }

        fn assert_constraint_rollback(fixture: &Fixture, sql: &str, extended: Option<i32>) {
            let before = retained_rows(&fixture.conn);
            let error = fixture.conn.execute(sql, []).expect_err(sql);
            match error {
                rusqlite::Error::SqliteFailure(code, _) => {
                    assert_eq!(code.code, rusqlite::ErrorCode::ConstraintViolation, "{sql}");
                    if let Some(expected) = extended {
                        assert_eq!(code.extended_code, expected, "{sql}");
                    }
                }
                other => panic!("expected SQL constraint failure for {sql}: {other}"),
            }
            assert_retained(&fixture.conn, &before);
        }

        fn closed_interaction(fixture: &Fixture, run: i64) -> i64 {
            fixture.conn.execute("INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason) VALUES(?1,2,'permission_request',?2,?3,'{}',?3,'not_visible')",params![run,"c".repeat(64),NOW]).unwrap();
            fixture.conn.last_insert_rowid()
        }

        #[test]
        fn runtime_v5_raw_audit_checks_reject_invalid_values_and_retain_valid_controls() {
            let (fixture, _, _, _, _) = raw_response();
            for change in [
                "expected_owner_epoch=0",
                "expected_owner_epoch=-1",
                "request_token='short'",
                "request_token=replace(hex(zeroblob(32)),'0','A')",
                "request_token=replace(hex(zeroblob(32)),'0','g')",
                "decision='invalid'",
                "director_mode='invalid'",
                "decision_source='invalid'",
                "director_mode='manual'",
                "director_mode='unknown'",
                "mode_evidence_ref=''",
                "decision_evidence_ref=''",
                "mode_evidence_ref=replace(hex(zeroblob(1025)),'0','x')",
                "decision_evidence_ref=replace(hex(zeroblob(1025)),'0','x')",
                "answer_profile_version=''",
                "answer_profile_version=' '",
                "answer_rule_id=''",
                "answer_rule_id=' '",
                "decided_at=''",
                "decided_at=' '",
            ] {
                assert_constraint_rollback(
                    &fixture,
                    &format!("UPDATE member_permission_responses SET {change}"),
                    Some(rusqlite::ffi::SQLITE_CONSTRAINT_CHECK),
                );
            }
            for mode in ["auto", "manual", "unknown"] {
                fixture.conn.execute("UPDATE member_permission_responses SET decision_source='user',director_mode=?1,decision='deny'",[mode]).unwrap();
            }
            let boundary = format!("{}ab", "界".repeat(682));
            fixture
        .conn
        .execute(
            "UPDATE member_permission_responses SET mode_evidence_ref=?1,decision_evidence_ref=?1",
            [&boundary],
        )
        .unwrap();
            assert_eq!(
        fixture
            .conn
            .query_row(
                "SELECT length(CAST(mode_evidence_ref AS BLOB)) FROM member_permission_responses",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2048
    );
            let before = retained_rows(&fixture.conn);
            assert!(
                fixture
                    .conn
                    .execute(
                        "UPDATE member_permission_responses SET mode_evidence_ref=?1",
                        [format!("{boundary}x")]
                    )
                    .is_err()
            );
            assert_retained(&fixture.conn, &before);
        }

        #[test]
        fn runtime_v5_raw_audit_and_claim_required_columns_reject_null() {
            let (fixture, _, _, _, _) = raw_response();
            for field in [
                "event_id",
                "interaction_id",
                "expected_owner_epoch",
                "request_token",
                "decision",
                "director_member_id",
                "director_mode",
                "decision_source",
                "mode_evidence_ref",
                "decision_evidence_ref",
                "answer_profile_version",
                "answer_rule_id",
                "decided_at",
            ] {
                assert_constraint_rollback(
                    &fixture,
                    &format!("UPDATE member_permission_responses SET {field}=NULL"),
                    Some(rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL),
                );
            }
            assert_constraint_rollback(
                &fixture,
                "UPDATE member_permission_response_claims SET input_id=NULL",
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL),
            );
        }

        #[test]
        fn runtime_v5_composite_claim_foreign_key_rejects_a_valid_but_unassociated_pair() {
            let (fixture, run, event, interaction, _) = raw_response();
            let other = closed_interaction(&fixture, run);
            let input = fixture.input(run, "unclaimed-audit").unwrap();
            fixture
        .conn
        .execute(
            "UPDATE member_input_operations SET kind='permission_response' WHERE input_id=?1",
            [input],
        )
        .unwrap();
            insert_audit(&fixture, input, event, interaction);
            assert_constraint_rollback(
                &fixture,
                &format!(
                    "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES({other},{input})"
                ),
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
            );
        }

        #[test]
        fn runtime_v5_claim_interaction_primary_key_excludes_a_second_valid_audit_pair() {
            let (fixture, run, event, interaction, _) = raw_response();
            let input = fixture.input(run, "second-same-episode-audit").unwrap();
            fixture
        .conn
        .execute(
            "UPDATE member_input_operations SET kind='permission_response' WHERE input_id=?1",
            [input],
        )
        .unwrap();
            insert_audit(&fixture, input, event, interaction);
            assert_constraint_rollback(
                &fixture,
                &format!(
                    "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES({interaction},{input})"
                ),
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY),
            );
        }

        #[test]
        fn runtime_v5_claim_input_uniqueness_is_independent_of_its_composite_foreign_key() {
            let (fixture, run, _, _, input) = raw_response();
            let other = closed_interaction(&fixture, run);
            fixture
                .conn
                .execute_batch("PRAGMA foreign_keys=OFF")
                .unwrap();
            assert_constraint_rollback(
                &fixture,
                &format!(
                    "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES({other},{input})"
                ),
                Some(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE),
            );
            fixture
                .conn
                .execute_batch("PRAGMA foreign_keys=ON")
                .unwrap();
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
        }

        #[test]
        fn runtime_v5_foreign_key_targets_and_composite_group_match_the_shipping_contract() {
            let fixture = Fixture::new();
            require_v5(&fixture.conn);
            for (table, mut expected) in [
                (
                    "member_permission_responses",
                    vec![
                        ("member_input_operations", "input_id", "input_id"),
                        ("member_runtime_events", "event_id", "event_id"),
                        ("member_interactions", "interaction_id", "interaction_id"),
                        ("members", "director_member_id", "member_id"),
                    ],
                ),
                (
                    "member_permission_response_claims",
                    vec![
                        ("member_interactions", "interaction_id", "interaction_id"),
                        ("member_input_operations", "input_id", "input_id"),
                        (
                            "member_permission_responses",
                            "interaction_id",
                            "interaction_id",
                        ),
                        ("member_permission_responses", "input_id", "input_id"),
                    ],
                ),
            ] {
                let mut actual = fixture
            .conn
            .prepare("SELECT \"table\",\"from\",\"to\",on_delete FROM pragma_foreign_key_list(?1)")
            .unwrap()
            .query_map([table], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
                actual.sort();
                expected.sort();
                assert_eq!(
                    actual,
                    expected
                        .into_iter()
                        .map(|(target, from, to)| (
                            target.to_string(),
                            from.to_string(),
                            to.to_string(),
                            "RESTRICT".to_string()
                        ))
                        .collect::<Vec<_>>(),
                    "{table}"
                );
            }
            let composite = query_rows(
                &fixture.conn,
                "SELECT id,seq,\"from\",\"to\" FROM pragma_foreign_key_list('member_permission_response_claims') WHERE \"table\"='member_permission_responses' ORDER BY seq",
            );
            assert_eq!(composite.len(), 2);
            assert_eq!(composite[0][0], composite[1][0]);
            assert_eq!(composite[0][1], Value::Integer(0));
            assert_eq!(composite[1][1], Value::Integer(1));
        }
    }

    mod automatic_review_migration {
        use rusqlite::OptionalExtension;
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use rusqlite::types::Value;

        use super::*;

        const REBUILT: [&str; 3] = [
            "member_runtime_events",
            "member_permission_responses",
            "member_permission_response_claims",
        ];

        const V6_EVENT_KINDS: [&str; 10] = [
            "coverage_unknown",
            "interaction_opened",
            "interaction_obscured",
            "interaction_closed",
            "permission_denied_observed",
            "automatic_refusal_observed",
            "automatic_review_unavailable",
            "runner_degraded",
            "runner_lost",
            "runner_exited",
        ];

        const WIDENED_KIND_CHECK: &str = "CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','runner_degraded','runner_lost','runner_exited'))";

        fn fixture_at(version: i32) -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = crate::db::connect(&format!(
                "sqlite:///{}",
                dir.path().join(format!("v{version}.db")).display()
            ))
            .unwrap();
            crate::db::embedded::migrations::runner()
                .set_grouped(true)
                .set_target(refinery::Target::Version(version))
                .run(&mut conn)
                .unwrap();
            let (fleet, director) = common::create_fleet(&mut conn, "automatic-review-upgrade");
            let member = common::register(&mut conn, fleet, "retained", Some("%2"));
            Fixture {
                _dir: dir,
                conn,
                fleet,
                director,
                member,
            }
        }

        fn query_rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
            let mut statement = conn.prepare(sql).unwrap();
            let width = statement.column_count();
            statement
                .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }

        fn column_names(conn: &Connection, table: &str) -> Vec<String> {
            conn.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
                .unwrap()
                .query_map([table], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }

        type Snapshot = Vec<(String, Vec<String>, Vec<Vec<Value>>)>;

        fn snapshot(conn: &Connection) -> Snapshot {
            let tables = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name != 'refinery_schema_history' ORDER BY name")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            tables
                .into_iter()
                .map(|table| {
                    let columns = column_names(conn, &table);
                    let rows = query_rows(
                        conn,
                        &format!("SELECT {} FROM {table} ORDER BY 1, 2", columns.join(",")),
                    );
                    (table, columns, rows)
                })
                .collect()
        }

        fn assert_preserved(conn: &Connection, before: &Snapshot) {
            for (table, columns, expected) in before {
                assert_eq!(
                    query_rows(
                        conn,
                        &format!("SELECT {} FROM {table} ORDER BY 1, 2", columns.join(","))
                    ),
                    *expected,
                    "{table}"
                );
            }
        }

        fn insert_event(
            fixture: &Fixture,
            run: i64,
            seq: i64,
            kind: &str,
        ) -> rusqlite::Result<i64> {
            fixture.conn.execute(
                "INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,committed_at,
                 payload_json,notification_state,archive_reason)
                 VALUES(?1,?2,?3,?4,?4,'{\"schema_version\":2}','archived','member_inactive')",
                params![run, seq, kind, NOW],
            )?;
            Ok(fixture.conn.last_insert_rowid())
        }

        fn set_profile(fixture: &Fixture, run: i64, profile: &str) {
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET native_version='2.1.302',profile_version=?1 WHERE run_id=?2",
                    params![profile, run],
                )
                .unwrap();
        }

        struct Seeded {
            profiled_runs: Vec<i64>,
            unprofiled_runs: Vec<i64>,
            event_high_water: i64,
            live_run: i64,
        }

        fn seed_v5(fixture: &mut Fixture) -> Seeded {
            let profiled_exited = fixture.run("exited").unwrap();
            set_profile(fixture, profiled_exited, "claude-2.1.302-default-3");
            let unprofiled_exited = fixture.run("exited").unwrap();
            let live_run = fixture.run("running").unwrap();
            set_profile(fixture, live_run, "codex-0.163.0-on-request-1");

            let interaction = fixture
                .interaction(live_run, 1, "permission_request")
                .unwrap();
            let opened = fixture.archived_event(live_run, 1).unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET interaction_id=?1,kind='interaction_opened' WHERE event_id=?2",
                    params![interaction, opened],
                )
                .unwrap();
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.member,
                fixture.director,
                "runtime event inbox 世界",
            )
            .unwrap();
            fixture.conn.execute(
                "INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,committed_at,
                 payload_json,notification_state,message_id)
                 VALUES(?1,2,'permission_denied_observed',?2,?2,'{\"schema_version\":2}','inbox',?3)",
                params![live_run, NOW, message.message_id],
            ).unwrap();
            insert_event(fixture, unprofiled_exited, 1, "runner_exited").unwrap();
            let discarded = insert_event(fixture, live_run, 3, "coverage_unknown").unwrap();
            fixture
                .conn
                .execute(
                    "DELETE FROM member_runtime_events WHERE event_id=?1",
                    [discarded],
                )
                .unwrap();

            let input = fixture.input(live_run, "permission:allow").unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET kind='permission_response' WHERE input_id=?1",
                    [input],
                )
                .unwrap();
            fixture.conn.execute(
                "INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) VALUES(?1,?2,?3,1,?4,'allow',?5,'auto','director-auto','fixture:mode','fixture:decision','codex-0.163.0-on-request-1','single-request-choice',?6)",
                params![input, opened, interaction, "a".repeat(64), fixture.director, NOW],
            ).unwrap();
            fixture
                .conn
                .execute(
                    "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES(?1,?2)",
                    params![interaction, input],
                )
                .unwrap();
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
            for table in REBUILT {
                assert!(super::count(&fixture.conn, table) > 0, "{table}");
            }
            Seeded {
                profiled_runs: vec![profiled_exited, live_run],
                unprofiled_runs: vec![unprofiled_exited],
                event_high_water: discarded,
                live_run,
            }
        }

        fn event_sequence(conn: &Connection) -> Option<i64> {
            conn.query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='member_runtime_events'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
        }

        fn table_shape(conn: &Connection, table: &str) -> Vec<Vec<Vec<Value>>> {
            let mut shape = vec![
                query_rows(
                    conn,
                    &format!(
                        "SELECT name,type,\"notnull\",dflt_value,pk FROM pragma_table_info('{table}') ORDER BY cid"
                    ),
                ),
                query_rows(
                    conn,
                    &format!(
                        "SELECT id,seq,\"table\",\"from\",\"to\",on_update,on_delete,\"match\" FROM pragma_foreign_key_list('{table}') ORDER BY id,seq"
                    ),
                ),
                query_rows(
                    conn,
                    &format!(
                        "SELECT name,\"unique\",origin,partial FROM pragma_index_list('{table}') ORDER BY name"
                    ),
                ),
            ];
            let indexes = conn
                .prepare("SELECT name FROM pragma_index_list(?1) ORDER BY name")
                .unwrap()
                .query_map([table], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for index in indexes {
                shape.push(query_rows(
                    conn,
                    &format!("SELECT seqno,name FROM pragma_index_info('{index}') ORDER BY seqno"),
                ));
            }
            shape
        }

        fn table_ddl(conn: &Connection, table: &str) -> String {
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap()
        }

        fn assert_check_violation(
            fixture: &Fixture,
            result: rusqlite::Result<usize>,
            context: &str,
        ) {
            match result {
                Err(rusqlite::Error::SqliteFailure(code, _)) => {
                    assert_eq!(
                        code.code,
                        rusqlite::ErrorCode::ConstraintViolation,
                        "{context}"
                    );
                    assert_eq!(
                        code.extended_code,
                        rusqlite::ffi::SQLITE_CONSTRAINT_CHECK,
                        "{context}"
                    );
                }
                other => panic!("expected a CHECK violation for {context}: {other:?}"),
            }
            assert!(fixture.conn.is_autocommit(), "{context}");
        }

        #[test]
        fn runtime_v6_automatic_review_migration_is_the_contiguous_successor_to_v5() {
            let chain = crate::db::migration_chain();
            assert!(crate::db::head_version() >= 6);
            assert!(
                chain.contains(&(6, "automatic_review_events".to_string())),
                "{chain:?}"
            );
            assert_eq!(
                chain
                    .iter()
                    .map(|(version, _)| *version)
                    .collect::<Vec<_>>(),
                (1..=crate::db::head_version()).collect::<Vec<_>>()
            );
            let fixture = Fixture::new();
            crate::diagnosis::classify_schema(&fixture.conn, crate::db::head_version())
                .require_current()
                .unwrap();
        }

        #[test]
        fn runtime_v6_upgrade_preserves_event_response_and_claim_rows_with_identical_values() {
            let mut fixture = fixture_at(5);
            seed_v5(&mut fixture);
            let before = snapshot(&fixture.conn);
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_preserved(&fixture.conn, &before);
            for table in REBUILT {
                let (_, columns, _) = before.iter().find(|(name, _, _)| name == table).unwrap();
                assert_eq!(&column_names(&fixture.conn, table), columns, "{table}");
            }
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
            assert_eq!(
                query_rows(&fixture.conn, "PRAGMA integrity_check"),
                vec![vec![Value::Text("ok".into())]]
            );
        }

        #[test]
        fn runtime_v6_upgrade_preserves_the_event_sequence_so_ids_are_never_reused() {
            let mut fixture = fixture_at(5);
            let seeded = seed_v5(&mut fixture);
            let sequence_before = query_rows(
                &fixture.conn,
                "SELECT name,seq FROM sqlite_sequence ORDER BY name",
            );
            assert_eq!(event_sequence(&fixture.conn), Some(seeded.event_high_water));
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT name,seq FROM sqlite_sequence ORDER BY name"
                ),
                sequence_before
            );
            assert!(
                query_rows(
                    &fixture.conn,
                    "SELECT name FROM sqlite_sequence WHERE name LIKE '%\\_v6' ESCAPE '\\'"
                )
                .is_empty()
            );
            let next =
                insert_event(&fixture, seeded.live_run, 4, "automatic_refusal_observed").unwrap();
            assert_eq!(next, seeded.event_high_water + 1);
        }

        #[test]
        fn runtime_v6_upgrade_of_a_database_without_events_keeps_the_sequence_absent() {
            let mut fixture = fixture_at(5);
            assert_eq!(event_sequence(&fixture.conn), None);
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_eq!(event_sequence(&fixture.conn), None);
            let run = fixture.run("running").unwrap();
            assert_eq!(
                insert_event(&fixture, run, 1, "coverage_unknown").unwrap(),
                1
            );
        }

        #[test]
        fn runtime_v6_rebuild_keeps_every_column_foreign_key_and_index_of_the_rebuilt_tables() {
            let v5 = fixture_at(5);
            let head = Fixture::new();
            for table in REBUILT {
                assert_eq!(
                    table_shape(&head.conn, table),
                    table_shape(&v5.conn, table),
                    "{table}"
                );
            }
            for row in query_rows(
                &head.conn,
                "SELECT type,name,tbl_name,COALESCE(sql,'') FROM sqlite_master",
            ) {
                for value in row {
                    if let Value::Text(text) = value {
                        assert!(!text.contains("_v6"), "{text}");
                    }
                }
            }
        }

        #[test]
        fn runtime_v6_event_kind_check_is_widened_to_the_specified_domain() {
            let fixture = Fixture::new();
            let ddl = table_ddl(&fixture.conn, "member_runtime_events");
            assert!(ddl.contains(WIDENED_KIND_CHECK), "{ddl}");
            let run = fixture.run("running").unwrap();
            for (seq, kind) in (1..).zip(V6_EVENT_KINDS) {
                insert_event(&fixture, run, seq, kind)
                    .unwrap_or_else(|error| panic!("{kind}: {error}"));
            }
            for kind in [
                "automatic_review_available",
                "automatic_refusal",
                "approved",
                "",
            ] {
                assert_check_violation(
                    &fixture,
                    insert_event(&fixture, run, 99, kind).map(|_| 1),
                    kind,
                );
            }
        }

        #[test]
        fn runtime_v6_upgraded_database_accepts_both_new_kinds_and_rejects_an_unknown_kind() {
            let mut fixture = fixture_at(5);
            let seeded = seed_v5(&mut fixture);
            assert!(
                insert_event(&fixture, seeded.live_run, 10, "automatic_refusal_observed").is_err(),
                "V5 must still reject the new kinds"
            );
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            insert_event(&fixture, seeded.live_run, 10, "automatic_refusal_observed").unwrap();
            insert_event(
                &fixture,
                seeded.live_run,
                11,
                "automatic_review_unavailable",
            )
            .unwrap();
            let before = snapshot(&fixture.conn);
            assert_check_violation(
                &fixture,
                insert_event(&fixture, seeded.live_run, 12, "automatic_refusal_rejected")
                    .map(|_| 1),
                "unknown kind",
            );
            assert_preserved(&fixture.conn, &before);
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
        }

        #[test]
        fn runtime_v6_response_foreign_key_targets_the_rebuilt_event_table() {
            let mut fixture = fixture_at(5);
            let seeded = seed_v5(&mut fixture);
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            let referenced: i64 = fixture
                .conn
                .query_row(
                    "SELECT event_id FROM member_permission_responses",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                fixture
                    .conn
                    .execute(
                        "DELETE FROM member_runtime_events WHERE event_id=?1",
                        [referenced]
                    )
                    .is_err(),
                "the response foreign key restricts deleting its event"
            );
            let interaction: i64 = fixture
                .conn
                .query_row(
                    "SELECT interaction_id FROM member_permission_response_claims",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                fixture
                    .conn
                    .execute(
                        "DELETE FROM member_permission_responses WHERE interaction_id=?1",
                        [interaction]
                    )
                    .is_err(),
                "the claim foreign key restricts deleting its response"
            );
            let input = fixture.input(seeded.live_run, "orphan-response").unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET kind='permission_response' WHERE input_id=?1",
                    [input],
                )
                .unwrap();
            assert!(
                fixture.conn.execute(
                    "INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) VALUES(?1,99999,?2,1,?3,'deny',?4,'auto','director-auto','fixture:mode','fixture:decision','p','r',?5)",
                    params![input, interaction, "a".repeat(64), fixture.director, NOW],
                ).is_err(),
                "a response naming a missing event is rejected"
            );
            assert!(query_rows(&fixture.conn, "PRAGMA foreign_key_check").is_empty());
        }

        #[test]
        fn runtime_v6_profile_match_column_is_nullable_text_appended_to_member_runs() {
            let fixture = Fixture::new();
            let columns = query_rows(
                &fixture.conn,
                "SELECT name,type,\"notnull\",dflt_value,pk FROM pragma_table_info('member_runs') ORDER BY cid",
            );
            assert_eq!(
                columns.last().unwrap(),
                &vec![
                    Value::Text("profile_match".into()),
                    Value::Text("TEXT".into()),
                    Value::Integer(0),
                    Value::Null,
                    Value::Integer(0),
                ]
            );
        }

        #[test]
        fn runtime_v6_backfills_profile_match_exact_for_every_profiled_v5_run() {
            let mut fixture = fixture_at(5);
            let seeded = seed_v5(&mut fixture);
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            let profile_match = |run: i64| -> Option<String> {
                fixture
                    .conn
                    .query_row(
                        "SELECT profile_match FROM member_runs WHERE run_id=?1",
                        [run],
                        |row| row.get(0),
                    )
                    .unwrap()
            };
            for run in &seeded.profiled_runs {
                assert_eq!(profile_match(*run).as_deref(), Some("exact"), "run {run}");
            }
            for run in &seeded.unprofiled_runs {
                assert_eq!(profile_match(*run), None, "run {run}");
            }
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT COUNT(*) FROM member_runs WHERE (profile_version IS NULL) != (profile_match IS NULL)"
                ),
                vec![vec![Value::Integer(0)]]
            );
        }

        #[test]
        fn runtime_v6_profile_match_must_agree_with_profile_version() {
            let fixture = Fixture::new();
            let unprofiled = fixture.run("exited").unwrap();
            let profiled = fixture.run("running").unwrap();
            assert_check_violation(
                &fixture,
                fixture.conn.execute(
                    "UPDATE member_runs SET profile_version='claude-2.1.302-default-3' WHERE run_id=?1",
                    [profiled],
                ),
                "profile_version without profile_match",
            );
            for value in ["exact", "fallback"] {
                assert_check_violation(
                    &fixture,
                    fixture.conn.execute(
                        "UPDATE member_runs SET profile_match=?1 WHERE run_id=?2",
                        params![value, unprofiled],
                    ),
                    &format!("profile_match={value} without profile_version"),
                );
            }
            for value in ["exact", "fallback"] {
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET native_version='2.1.306',profile_version='claude-2.1.302-default-3',profile_match=?1 WHERE run_id=?2",
                        params![value, profiled],
                    )
                    .unwrap_or_else(|error| panic!("{value}: {error}"));
            }
            for change in [
                "profile_match=NULL",
                "profile_match='bogus'",
                "profile_match='EXACT'",
                "profile_match=''",
                "profile_version=NULL",
            ] {
                assert_check_violation(
                    &fixture,
                    fixture.conn.execute(
                        &format!("UPDATE member_runs SET {change} WHERE run_id=?1"),
                        [profiled],
                    ),
                    change,
                );
            }
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET profile_version=NULL,profile_match=NULL WHERE run_id=?1",
                    [profiled],
                )
                .unwrap();
        }

        #[test]
        fn runtime_v6_runtime_event_kind_values_match_the_sql_domain() {
            use crate::broker::runtime::RuntimeEventKind;
            let mut values = RuntimeEventKind::VALUES.to_vec();
            let mut expected = V6_EVENT_KINDS.to_vec();
            values.sort_unstable();
            expected.sort_unstable();
            assert_eq!(values, expected);
            for kind in ["automatic_refusal_observed", "automatic_review_unavailable"] {
                assert_eq!(RuntimeEventKind::try_from(kind).unwrap().as_str(), kind);
                assert_eq!(
                    serde_json::to_value(RuntimeEventKind::try_from(kind).unwrap()).unwrap(),
                    serde_json::Value::String(kind.into())
                );
            }
            assert!(RuntimeEventKind::try_from("automatic_review_available").is_err());
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            for (seq, kind) in (1..).zip(RuntimeEventKind::VALUES) {
                let event = insert_event(&fixture, run, seq, kind).unwrap();
                let stored: RuntimeEventKind = fixture
                    .conn
                    .query_row(
                        "SELECT kind FROM member_runtime_events WHERE event_id=?1",
                        [event],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(stored.as_str(), *kind);
            }
        }

        #[test]
        fn runtime_v6_head_guard_refuses_a_v5_shape_recorded_as_v6() {
            let fixture = fixture_at(5);
            fixture
                .conn
                .execute(
                    "INSERT INTO refinery_schema_history(version,name,applied_on,checksum) VALUES(6,'automatic_review_events','2026-10-29T00:00:00Z','1')",
                    [],
                )
                .unwrap();
            assert!(!crate::db::schema_matches_version(&fixture.conn, 6).unwrap());
        }

        #[test]
        fn runtime_v6_failed_migration_retains_v5_schema_ledger_and_rows_for_retry() {
            let mut fixture = fixture_at(5);
            seed_v5(&mut fixture);
            let before = snapshot(&fixture.conn);
            let schema = query_rows(
                &fixture.conn,
                "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name",
            );
            let ledger = query_rows(
                &fixture.conn,
                "SELECT * FROM refinery_schema_history ORDER BY version",
            );
            let sequence = query_rows(
                &fixture.conn,
                "SELECT name,seq FROM sqlite_sequence ORDER BY name",
            );
            fixture
                .conn
                .authorizer(Some(|context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::AlterTable {
                            table_name: "member_runs",
                            ..
                        }
                    ) {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }))
                .unwrap();
            let result = crate::db::migrate_to_head(&mut fixture.conn);
            fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert!(
                result.is_err(),
                "the denied V6 profile_match column must fail migration"
            );
            assert!(fixture.conn.is_autocommit());
            assert_preserved(&fixture.conn, &before);
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name"
                ),
                schema
            );
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT * FROM refinery_schema_history ORDER BY version"
                ),
                ledger
            );
            assert_eq!(
                query_rows(
                    &fixture.conn,
                    "SELECT name,seq FROM sqlite_sequence ORDER BY name"
                ),
                sequence
            );
            crate::db::migrate_to_head(&mut fixture.conn).unwrap();
            assert_preserved(&fixture.conn, &before);
            crate::diagnosis::classify_schema(&fixture.conn, crate::db::head_version())
                .require_current()
                .unwrap();
        }
    }

    const NOW: &str = "2026-10-26T10:00:00.000000+00:00";

    struct Fixture {
        _dir: TempDir,
        conn: Connection,
        fleet: i64,
        director: i64,
        member: i64,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = common::migrated_conn(&dir);
            let (fleet, director) = common::create_fleet(&mut conn, "runtime-schema");
            let member = common::register(&mut conn, fleet, "worker", Some("%2"));
            Self {
                _dir: dir,
                conn,
                fleet,
                director,
                member,
            }
        }

        fn run(&self, status: &str) -> rusqlite::Result<i64> {
            self.conn.execute(
            "INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,
             database_identity_json,launch_nonce,runtime_path,resolved_mode,mode_source,
             permission_argv_json,event_evidence,coverage,status,created_at,
             owner_epoch,owner_kind,owner_pid,owner_start_identity,reachability,
             cleanup_state,event_seq_reserved_through,observation_gap_count)
             VALUES(?1,?2,?3,'codex',?4,?5,'/isolated/runtime','never','default',
             '[\"--ask-for-approval\",\"never\",\"--sandbox\",\"workspace-write\"]',
             'metadata','unknown',?6,?7,1,'creator',1234,'fixture-owner','reachable',
             'incomplete',0,0)",
            params![self.member,self.fleet,self.director,
                json!({"canonical_path": self.conn.path().unwrap(), "instance_id": "a".repeat(32), "device":1,"inode":2}).to_string(),
                "b".repeat(64),status,NOW],
        )?;
            Ok(self.conn.last_insert_rowid())
        }

        fn interaction(&self, run: i64, seq: i64, class: &str) -> rusqlite::Result<i64> {
            self.conn.execute(
            "INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json)
             VALUES(?1,?2,?3,?4,?5,'{}')",
            params![run,seq,class,"c".repeat(64),NOW],
        )?;
            Ok(self.conn.last_insert_rowid())
        }

        fn archived_event(&self, run: i64, seq: i64) -> rusqlite::Result<i64> {
            self.conn.execute(
                "INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,committed_at,
             payload_json,notification_state,archive_reason)
             VALUES(?1,?2,'coverage_unknown',?3,?3,'{\"schema_version\":1}',
             'archived','member_inactive')",
                params![run, seq, NOW],
            )?;
            Ok(self.conn.last_insert_rowid())
        }

        fn input(&self, run: i64, key: &str) -> rusqlite::Result<i64> {
            self.conn.execute(
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at)
             VALUES(?1,?2,'prompt',?3,'in_progress',?4,?4)",
            params![run,key,"d".repeat(64),NOW],
        )?;
            Ok(self.conn.last_insert_rowid())
        }
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn runtime_migration_extends_the_existing_family_after_v3() {
        let fixture = Fixture::new();
        assert!(super::head_version() >= 4);
        assert_eq!(
            fixture
                .conn
                .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            super::APPLICATION_ID
        );
        for table in [
            "runtime_database_identity",
            "member_runs",
            "member_interactions",
            "member_runtime_events",
            "member_input_operations",
        ] {
            assert_eq!(
                fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                        [table],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                1,
                "{table}"
            );
        }
    }

    #[test]
    fn database_instance_identity_is_singleton_stable_and_distinct() {
        let mut first = Fixture::new();
        let second = Fixture::new();
        let identity = |conn: &Connection| {
            conn.query_row(
                "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        let original = identity(&first.conn);
        assert_eq!(original.len(), 32);
        assert!(original.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(original, identity(&second.conn));
        super::migrate_to_head(&mut first.conn).unwrap();
        let url = format!("sqlite:///{}", first.conn.path().unwrap());
        assert_eq!(identity(&super::connect(&url).unwrap()), original);
        assert_eq!(count(&first.conn, "runtime_database_identity"), 1);
        assert!(
            first
                .conn
                .execute(
                    "INSERT INTO runtime_database_identity(singleton_id,instance_id) VALUES(2,?1)",
                    ["e".repeat(32)]
                )
                .is_err()
        );
    }

    #[test]
    fn runtime_schema_exposes_the_complete_record_shapes() {
        let fixture = Fixture::new();
        for (table, expected) in [
            (
                "member_runs",
                "run_id member_id fleet_id director_member_id backend database_identity_json launch_nonce runtime_path requested_mode resolved_mode mode_source permission_argv_json event_evidence coverage status created_at native_version profile_version effective_policy_json owner_epoch owner_kind owner_pid owner_start_identity reachability cleanup_state event_seq_reserved_through observation_gap_count runtime_dir_identity_json run_lock_identity_json runner_pid runner_start_identity child_pid child_start_identity started_at last_health_at ended_at exit_code exit_signal cleanup_diagnostics_json recovery_origin_status profile_match",
            ),
            (
                "member_interactions",
                "interaction_id run_id episode_seq classification evidence_digest opened_at closed_at close_reason evidence_json",
            ),
            (
                "member_runtime_events",
                "event_id run_id event_seq interaction_id kind observed_at committed_at payload_json notification_state message_id archive_reason",
            ),
            (
                "member_input_operations",
                "input_id run_id operation_key kind payload_digest state created_at updated_at message_id disposition evidence_ref",
            ),
        ] {
            let actual = fixture
                .conn
                .prepare("SELECT name FROM pragma_table_info(?1)")
                .unwrap()
                .query_map([table], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for field in expected.split_whitespace() {
                assert!(actual.iter().any(|name| name == field), "{table}.{field}");
            }
        }
    }

    #[test]
    fn current_schema_guard_requires_each_runtime_table() {
        for table in [
            "runtime_database_identity",
            "member_runs",
            "member_interactions",
            "member_runtime_events",
            "member_input_operations",
        ] {
            let fixture = Fixture::new();
            fixture
                .conn
                .execute_batch("PRAGMA foreign_keys=OFF")
                .unwrap();
            fixture
                .conn
                .execute_batch(&format!("DROP TABLE {table}"))
                .unwrap();
            assert!(
                !super::schema_matches_version(&fixture.conn, super::head_version()).unwrap(),
                "{table}"
            );
        }
    }

    #[test]
    fn every_nonterminal_status_enforces_one_live_run_per_member() {
        for status in [
            "starting",
            "prepared",
            "running",
            "degraded",
            "stopping",
            "recovering",
        ] {
            let fixture = Fixture::new();
            let run = fixture.run(status).unwrap();
            assert!(fixture.run("starting").is_err(), "{status}");
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='exited' WHERE run_id=?1",
                    [run],
                )
                .unwrap();
            assert!(fixture.run("starting").unwrap() > run);
        }
    }

    #[test]
    fn terminal_run_history_allows_a_new_run_without_losing_the_old_one() {
        for status in ["exited", "failed", "lost"] {
            let fixture = Fixture::new();
            let old = fixture.run(status).unwrap();
            assert!(fixture.run("starting").unwrap() > old);
            assert_eq!(count(&fixture.conn, "member_runs"), 2);
        }
    }

    #[test]
    fn run_sql_domains_and_json_shapes_reject_corrupt_values() {
        let fixture = Fixture::new();
        let run = fixture.run("starting").unwrap();
        for (field, value) in [
            ("status", "bogus"),
            ("mode_source", "inherited"),
            ("owner_kind", "member"),
            ("reachability", "dead"),
            ("cleanup_state", "unknown"),
            ("coverage", "universal"),
            ("event_evidence", "raw"),
            ("backend", "bogus"),
            ("database_identity_json", "[]"),
            ("effective_policy_json", "[]"),
            ("permission_argv_json", "{}"),
            ("cleanup_diagnostics_json", "{}"),
            ("runtime_dir_identity_json", "[]"),
            ("run_lock_identity_json", "[]"),
        ] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_runs SET {field}=?1 WHERE run_id=?2"),
                        params![value, run]
                    )
                    .is_err(),
                "{field}={value}"
            );
        }
        for field in ["owner_epoch", "owner_pid"] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_runs SET {field}=0 WHERE run_id=?1"),
                        [run]
                    )
                    .is_err(),
                "{field}"
            );
        }
        for field in ["event_seq_reserved_through", "observation_gap_count"] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_runs SET {field}=-1 WHERE run_id=?1"),
                        [run]
                    )
                    .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn interaction_identity_and_the_single_open_episode_are_enforced() {
        let fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        let first = fixture.interaction(run, 1, "permission_request").unwrap();
        assert!(fixture.interaction(run, 2, "question").is_err());
        fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",params![NOW,first]).unwrap();
        assert!(fixture.interaction(run, 1, "question").is_err());
        assert!(fixture.interaction(run, 2, "question").unwrap() > first);
    }

    #[test]
    fn interaction_classes_preserve_distinct_native_forms() {
        for class in [
            "permission_request",
            "question",
            "trust",
            "update",
            "unknown_interaction",
        ] {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            fixture.interaction(run, 1, class).unwrap();
        }
        for class in [
            "permission_denied",
            "working",
            "composer_empty",
            "unknown",
            "bogus",
        ] {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            assert!(fixture.interaction(run, 1, class).is_err(), "{class}");
        }
    }

    #[test]
    fn interaction_closure_fields_are_paired_and_reasons_are_typed() {
        let fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        let interaction = fixture.interaction(run, 1, "permission_request").unwrap();
        for assignment in [
            "closed_at='2026-10-26T10:00:00Z'",
            "close_reason='not_visible'",
            "closed_at='2026-10-26T10:00:00Z',close_reason='approved'",
            "evidence_json='[]'",
            "episode_seq=0",
        ] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!(
                            "UPDATE member_interactions SET {assignment} WHERE interaction_id=?1"
                        ),
                        [interaction]
                    )
                    .is_err(),
                "{assignment}"
            );
        }
        for reason in [
            "not_visible",
            "superseded",
            "process_exited",
            "monitor_lost",
        ] {
            fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason=?2 WHERE interaction_id=?3",params![NOW,reason,interaction]).unwrap();
        }
    }

    #[test]
    fn runtime_event_sequences_are_positive_and_unique_within_a_run() {
        let fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        fixture.archived_event(run, 1).unwrap();
        assert!(fixture.archived_event(run, 1).is_err());
        for seq in [0, -1] {
            assert!(fixture.archived_event(run, seq).is_err());
        }
        let old = fixture.run("exited").unwrap();
        fixture.archived_event(old, 1).unwrap();
    }

    #[test]
    fn notification_state_requires_exactly_its_message_or_archive_reason() {
        let mut fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        let event = fixture.archived_event(run, 1).unwrap();
        let message = crate::broker::send_message(
            &mut fixture.conn,
            fixture.member,
            fixture.director,
            "runtime",
        )
        .unwrap();
        for assignment in [
            "notification_state='inbox'",
            "archive_reason=NULL",
            "archive_reason='notified'",
            "notification_state='unknown'",
            "kind='approved'",
            "payload_json='[]'",
            "payload_json='broken'",
        ] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_runtime_events SET {assignment} WHERE event_id=?1"),
                        [event]
                    )
                    .is_err(),
                "{assignment}"
            );
        }
        fixture.conn.execute("UPDATE member_runtime_events SET notification_state='inbox',archive_reason=NULL,message_id=?1 WHERE event_id=?2",params![message.message_id,event]).unwrap();
        for assignment in ["message_id=NULL", "archive_reason='member_inactive'"] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_runtime_events SET {assignment} WHERE event_id=?1"),
                        [event]
                    )
                    .is_err(),
                "{assignment}"
            );
        }
    }

    #[test]
    fn event_kinds_and_archive_reasons_accept_the_specified_domains() {
        let fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        let event = fixture.archived_event(run, 1).unwrap();
        for kind in [
            "coverage_unknown",
            "interaction_opened",
            "interaction_obscured",
            "interaction_closed",
            "permission_denied_observed",
            "automatic_refusal_observed",
            "automatic_review_unavailable",
            "runner_degraded",
            "runner_lost",
            "runner_exited",
        ] {
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET kind=?1 WHERE event_id=?2",
                    params![kind, event],
                )
                .unwrap();
        }
        for reason in ["member_inactive", "fleet_deleted", "director_inactive"] {
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET archive_reason=?1 WHERE event_id=?2",
                    params![reason, event],
                )
                .unwrap();
        }
    }

    #[test]
    fn durable_input_keys_are_run_scoped_and_settlements_require_a_disposition() {
        let fixture = Fixture::new();
        let run = fixture.run("running").unwrap();
        let input = fixture.input(run, "prompt:one").unwrap();
        assert!(fixture.input(run, "prompt:one").is_err());
        let old = fixture.run("exited").unwrap();
        fixture.input(old, "prompt:one").unwrap();
        for assignment in [
            "state='settled'",
            "kind='preview'",
            "kind='approval'",
            "state='running'",
            "disposition='approved'",
        ] {
            assert!(
                fixture
                    .conn
                    .execute(
                        &format!(
                            "UPDATE member_input_operations SET {assignment} WHERE input_id=?1"
                        ),
                        [input]
                    )
                    .is_err(),
                "{assignment}"
            );
        }
        for disposition in ["submitted", "not_submitted", "abandoned_uncertain"] {
            fixture.conn.execute("UPDATE member_input_operations SET state='settled',disposition=?1 WHERE input_id=?2",params![disposition,input]).unwrap();
        }
    }

    #[test]
    fn runtime_foreign_keys_retain_history_and_reject_orphans() {
        let fixture = Fixture::new();
        assert!(fixture.interaction(404, 1, "question").is_err());
        assert!(fixture.archived_event(404, 1).is_err());
        assert!(fixture.input(404, "orphan").is_err());
        let run = fixture.run("exited").unwrap();
        fixture.interaction(run, 1, "question").unwrap();
        fixture.archived_event(run, 1).unwrap();
        fixture.input(run, "retained").unwrap();
        assert!(
            fixture
                .conn
                .execute("DELETE FROM member_runs WHERE run_id=?1", [run])
                .is_err()
        );
        assert!(
            fixture
                .conn
                .execute("DELETE FROM members WHERE member_id=?1", [fixture.member])
                .is_err()
        );
        assert_eq!(count(&fixture.conn, "member_runs"), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn require_current(conn: &Connection) -> Result<(), CafleetError> {
        crate::diagnosis::classify_schema(conn, head_version()).require_current()
    }

    fn at_version(version: u32) -> Connection {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        embedded::migrations::runner()
            .set_grouped(true)
            .set_target(refinery::Target::Version(i32::try_from(version).unwrap()))
            .run(&mut conn)
            .unwrap();
        conn
    }

    fn rows(conn: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
        let mut statement = conn.prepare(sql).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn application_rows(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "SELECT * FROM fleets ORDER BY fleet_id",
            "SELECT * FROM members ORDER BY member_id",
            "SELECT * FROM member_placements ORDER BY member_id",
            "SELECT message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at FROM messages ORDER BY message_id",
            "SELECT * FROM asset_installs ORDER BY coding_agent, path",
            "SELECT * FROM sqlite_sequence ORDER BY name",
        ].map(|sql| rows(conn, sql)).into()
    }

    #[test]
    fn supported_upgrades_and_repeated_migration_preserve_all_application_rows() {
        for (version, _) in migration_chain() {
            let mut conn = at_version(version);
            conn.execute_batch(
                "INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'preserved', 'created');
                 INSERT INTO members(member_id, fleet_id, name, description, status, registered_at, deregistered_at) VALUES
                    (1, 1, 'Director', 'director assignment', 'active', 'registered', NULL),
                    (2, 1, 'worker', 'worker assignment', 'active', 'registered', NULL),
                    (3, 1, 'departed', 'finished assignment', 'deregistered', 'registered', 'departed');
                 INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) VALUES
                    (1, 'main', '@1', '%0', 'tmux', 'claude', 'placed'),
                    (2, 'main', '@1', NULL, 'tmux', 'codex', 'placed');
                 UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;
                 INSERT INTO messages(message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at) VALUES
                    (7, 1, 2, 'created-7', 'input_required', 'status-7', 'pending unicode 世界', NULL),
                    (8, 1, 2, 'created-8', 'input_required', 'status-8', 'notified', 'notified-8'),
                    (9, 2, 1, 'created-9', 'completed', 'status-9', 'completed', NULL);
                 INSERT INTO asset_installs VALUES('claude', '/preserved/config', '0.29.0', 'installed');"
            ).unwrap();
            if version == 2 {
                conn.execute(
                    "UPDATE messages SET preview_started_at='claimed' WHERE message_id=7",
                    [],
                )
                .unwrap();
            }
            assert!(schema_matches_version(&conn, version).unwrap());
            let state = crate::diagnosis::classify_schema(&conn, head_version());
            if version < head_version() {
                assert!(
                    matches!(state, crate::diagnosis::SchemaState::Behind { recorded, .. } if recorded == version)
                );
                assert!(state.require_current().is_err());
            } else {
                state.require_current().unwrap();
            }
            let before = application_rows(&conn);
            let ledger = rows(
                &conn,
                "SELECT * FROM refinery_schema_history ORDER BY version",
            );
            for _ in 0..2 {
                assert_eq!(migrate_to_head(&mut conn).unwrap(), head_version());
                assert_eq!(application_rows(&conn), before, "migration from {version}");
                let after = rows(
                    &conn,
                    "SELECT * FROM refinery_schema_history ORDER BY version",
                );
                assert_eq!(&after[..ledger.len()], ledger);
                require_current(&conn).unwrap();
            }
        }
    }

    #[test]
    fn selected_reference_rechecks_ddl_and_accepts_auxiliary_objects() {
        let conn = at_version(head_version());
        conn.execute_batch(
            "CREATE TABLE operator_data(value TEXT);
             CREATE INDEX operator_messages ON messages(text);
             CREATE TRIGGER operator_message_insert AFTER INSERT ON messages BEGIN
                INSERT INTO operator_data VALUES(NEW.text);
             END;",
        )
        .unwrap();
        assert!(schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN text TO broken")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN broken TO text")
            .unwrap();
        assert!(schema_matches_version(&conn, head_version()).unwrap());
        conn.execute_batch("CREATE TEMP TABLE messages(wrong TEXT)")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version()).unwrap());
    }

    #[test]
    fn required_schema_components_and_identity_are_refused() {
        for damage in [
            "DROP INDEX idx_messages_to_member_status_ts",
            "DROP INDEX idx_messages_to_member_status_ts; CREATE INDEX idx_messages_to_member_status_ts ON messages(from_member_id, status_timestamp)",
            "DROP TABLE asset_installs",
            "ALTER TABLE members RENAME COLUMN status TO wrong",
            "ALTER TABLE refinery_schema_history RENAME COLUMN checksum TO wrong",
            "PRAGMA application_id=1",
        ] {
            let conn = at_version(head_version());
            conn.execute_batch(damage).unwrap();
            assert!(require_current(&conn).is_err(), "{damage}");
        }
    }

    #[test]
    fn structural_reference_checks_required_keys_types_defaults_and_foreign_keys() {
        for (table, original, replacement) in [
            (
                "asset_installs",
                "PRIMARY KEY (coding_agent, path)",
                "PRIMARY KEY (coding_agent)",
            ),
            ("members", "name TEXT NOT NULL", "name TEXT"),
            ("messages", "text TEXT NOT NULL", "text INTEGER NOT NULL"),
            ("member_placements", "DEFAULT 'tmux'", "DEFAULT 'herdr'"),
            ("messages", "ON DELETE RESTRICT", "ON DELETE CASCADE"),
        ] {
            let conn = at_version(head_version());
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(ddl.contains(original));
            let version: i64 = conn
                .query_row("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=ON").unwrap();
            conn.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE name=?2",
                rusqlite::params![ddl.replace(original, replacement), table],
            )
            .unwrap();
            conn.pragma_update(None, "schema_version", version + 1)
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=OFF").unwrap();
            assert!(
                !schema_matches_version(&conn, head_version()).unwrap(),
                "{table}: {original}"
            );
        }
    }

    #[test]
    fn ahead_versions_validate_the_ledger_without_requiring_the_head_application_shape() {
        let conn = at_version(head_version());
        conn.execute_batch("ALTER TABLE messages ADD COLUMN future_value TEXT")
            .unwrap();
        conn.execute("UPDATE refinery_schema_history SET version=?1 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)", [head_version() + 1]).unwrap();
        assert!(schema_matches_version(&conn, head_version() + 1).unwrap());
        assert!(matches!(
            crate::diagnosis::classify_schema(&conn, head_version()),
            crate::diagnosis::SchemaState::Ahead { .. }
        ));
        conn.execute_batch("ALTER TABLE refinery_schema_history RENAME COLUMN checksum TO wrong")
            .unwrap();
        assert!(!schema_matches_version(&conn, head_version() + 1).unwrap());
        assert!(!schema_matches_version(&conn, 0).unwrap());
    }

    #[test]
    fn connection_pragmas_and_foreign_keys_protect_real_writes() {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        assert_eq!(
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            5000
        );
        migrate_to_head(&mut conn).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            APPLICATION_ID
        );
        assert!(conn.execute("INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(404, 'orphan', '', 'active', '')", []).is_err());
        conn.execute_batch("INSERT INTO fleets(fleet_id, created_at) VALUES(1, 'created'); INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES(1, 1, 'valid', '', 'active', 'registered');").unwrap();
        for (from, to) in [(404, 1), (1, 404)] {
            assert!(conn.execute("INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '', 'input_required', '', '')", rusqlite::params![from, to]).is_err());
        }
        for url in ["postgresql:///registry", "mysql://localhost/registry"] {
            assert!(connect(url).is_err());
        }
    }

    #[test]
    fn failed_baseline_initialization_rolls_back_identity_tables_and_ledger() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA max_page_count=1").unwrap();
        assert!(migrate_to_head(&mut conn).is_err());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            rows(
                &conn,
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
            .is_empty()
        );
    }

    #[test]
    fn migration_chain_is_contiguous_from_one_with_a_single_baseline() {
        let chain = migration_chain();
        assert_eq!(
            chain
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            (1..=u32::try_from(chain.len()).unwrap()).collect::<Vec<_>>()
        );
        assert_eq!(
            chain.iter().filter(|(_, name)| name == "baseline").count(),
            1
        );
        assert_eq!(chain.first(), Some(&(1, "baseline".into())));
        assert!(chain.iter().all(|(_, name)| !name.is_empty()));
    }
}
