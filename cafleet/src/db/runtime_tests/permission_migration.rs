use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::Value;

use super::*;

fn v4_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut conn =
        crate::db::connect(&format!("sqlite:///{}", dir.path().join("v4.db").display())).unwrap();
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

fn retained_rows(conn: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
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
            let rows = query_rows(conn, &format!("SELECT * FROM {table} ORDER BY 1"));
            (table, rows)
        })
        .collect()
}

fn assert_retained(conn: &Connection, before: &[(String, Vec<Vec<Value>>)]) {
    for (table, expected) in before {
        assert_eq!(
            query_rows(conn, &format!("SELECT * FROM {table} ORDER BY 1")),
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
            .prepare("SELECT name,type,\"notnull\",pk FROM pragma_table_info(?1) ORDER BY cid")
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
