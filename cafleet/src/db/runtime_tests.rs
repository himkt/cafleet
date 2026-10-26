use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;

use crate::broker::tests as common;

mod permission_migration;

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
            "run_id member_id fleet_id director_member_id backend database_identity_json launch_nonce runtime_path requested_mode resolved_mode mode_source permission_argv_json event_evidence coverage status created_at native_version profile_version effective_policy_json owner_epoch owner_kind owner_pid owner_start_identity reachability cleanup_state event_seq_reserved_through observation_gap_count runtime_dir_identity_json run_lock_identity_json runner_pid runner_start_identity child_pid child_start_identity started_at last_health_at ended_at exit_code exit_signal cleanup_diagnostics_json recovery_origin_status",
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
                    &format!("UPDATE member_interactions SET {assignment} WHERE interaction_id=?1"),
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
                    &format!("UPDATE member_input_operations SET {assignment} WHERE input_id=?1"),
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
