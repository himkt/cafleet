use std::ops::RangeInclusive;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::runtime::{self as storage, *};
use super::tests as common;
use crate::output::format_json;

mod permission_responses;
mod versioned_events;

const NOW: &str = "2026-10-26T10:00:00.000000+00:00";
const SECRET: &str = "fixture-secret-194-unique";

struct Fixture {
    _dir: TempDir,
    conn: Connection,
    fleet: i64,
    director: i64,
    member: i64,
    run: RunRecord,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "runtime-contract");
        let member = common::register(&mut conn, fleet, "worker", Some("%2"));
        let spec = Self::spec(&conn, fleet, director, member);
        let run = storage::create_run(&mut conn, &spec).unwrap();
        Self {
            _dir: dir,
            conn,
            fleet,
            director,
            member,
            run,
        }
    }

    fn spec(conn: &Connection, fleet: i64, director: i64, member: i64) -> NewRun {
        NewRun {
            member_id: member,
            fleet_id: fleet,
            director_member_id: director,
            backend: "codex".into(),
            database_identity_json: storage::database_identity(conn).unwrap(),
            launch_nonce: "a".repeat(64),
            runtime_path: std::path::Path::new(conn.path().unwrap())
                .parent()
                .unwrap()
                .join(".cafleet-runs/fixture/run")
                .to_str()
                .unwrap()
                .into(),
            requested_mode: None,
            resolved_mode: "never".into(),
            mode_source: ModeSource::Default,
            permission_argv_json: json!([
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write"
            ]),
            event_evidence: EventEvidence::Metadata,
            native_version: Some("0.163.0".into()),
            profile_version: None,
            coverage: Coverage::Unknown,
            owner_pid: i64::from(std::process::id()),
            owner_start_identity: "fixture-start-identity".into(),
        }
    }

    fn activate(&mut self) -> RunOwner {
        self.conn.execute("UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2 WHERE run_id=?1",[self.run.run_id]).unwrap();
        self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
        self.run.owner()
    }

    fn recoverer(&mut self) -> RunOwner {
        self.conn.execute("UPDATE member_runs SET status='recovering',owner_kind='reconciler',owner_epoch=owner_epoch+1 WHERE run_id=?1",[self.run.run_id]).unwrap();
        self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
        self.run.owner()
    }

    fn reserve(&mut self, owner: &RunOwner) -> RangeInclusive<i64> {
        storage::reserve_event_sequences(&mut self.conn, owner).unwrap()
    }

    fn open(&mut self, owner: &RunOwner) -> RuntimeEventRecord {
        self.reserve(owner);
        let transition = opening(self.run.run_id, 1, 1, metadata());
        storage::persist_transition(&mut self.conn, owner, &transition, PersistenceMode::Live)
            .unwrap()
    }

    fn input(
        &mut self,
        owner: &RunOwner,
        key: &str,
        kind: InputKind,
        message_id: Option<i64>,
    ) -> InputOperation {
        storage::reserve_input(
            &mut self.conn,
            owner,
            &NewInput {
                operation_key: key.into(),
                kind,
                payload_digest: "d".repeat(64),
                message_id,
            },
        )
        .unwrap()
    }

    fn reopen(&self) -> Connection {
        crate::db::connect(&format!("sqlite:///{}", self.conn.path().unwrap())).unwrap()
    }
}

fn raw_evidence(text: &str) -> Value {
    json!({
        "backend":"codex","native_version":"0.163.0","profile_version":"fixture-v1",
        "rule_id":"native-tool-permission","screen_seq":120,
        "operation":format!("run {text}"),"choices":[format!("allow {text}"),"deny"],
        "native_request_id":format!("native-{text}"),"excerpt":format!("request {text}"),
        "semantic_fingerprint":format!("fingerprint-{text}"),
        "truncated":false,"sensitive":false,
    })
}

fn metadata() -> Value {
    evidence::EvidencePolicy::metadata()
        .apply(&raw_evidence(SECRET))
        .unwrap()
}

fn opening(run: i64, event_seq: i64, episode_seq: i64, evidence: Value) -> ObservedTransition {
    ObservedTransition {
        run_id: run,
        event_seq,
        episode_seq: Some(episode_seq),
        kind: RuntimeEventKind::InteractionOpened,
        classification: Some(InteractionClass::PermissionRequest),
        observed_at: NOW.into(),
        evidence_json: evidence,
        close_reason: None,
        cause_json: None,
    }
}

fn closure(run: i64, event_seq: i64, episode_seq: i64) -> ObservedTransition {
    ObservedTransition {
        kind: RuntimeEventKind::InteractionClosed,
        close_reason: Some(InteractionCloseReason::NotVisible),
        ..opening(run, event_seq, episode_seq, metadata())
    }
}

fn rows(conn: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
        .unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|index| row.get(index)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn durable_state(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "member_runs",
        "member_interactions",
        "member_runtime_events",
        "member_input_operations",
        "messages",
    ]
    .map(|table| rows(conn, table))
    .into()
}

fn assert_no_secret(conn: &Connection, secret: &str) {
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for table in tables {
        let table = table.replace('"', "\"\"");
        for row in rows(conn, &format!("\"{table}\"")) {
            for value in row {
                match value {
                    rusqlite::types::Value::Text(text) => {
                        assert!(!text.contains(secret), "secret in {table}: {text}")
                    }
                    rusqlite::types::Value::Blob(bytes) => assert!(
                        !bytes
                            .windows(secret.len())
                            .any(|part| part == secret.as_bytes()),
                        "secret in {table} blob"
                    ),
                    _ => {}
                }
            }
        }
    }
}

fn assert_key_order(value: &Value, expected: &[&str]) {
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        expected
    );
    let text = format_json(value);
    let mut previous = 0;
    for key in expected {
        let needle = format!("\"{key}\":");
        let position = text[previous..]
            .find(&needle)
            .unwrap_or_else(|| panic!("missing ordered {key} in {text}"))
            + previous;
        previous = position + needle.len();
    }
}

#[test]
fn new_run_records_launch_intent_and_unreached_phases_without_inventing_policy() {
    let fixture = Fixture::new();
    let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Starting);
    assert_eq!(run.owner_epoch, 1);
    assert_eq!(run.owner_kind, OwnerKind::Creator);
    assert_eq!(run.cleanup_state, CleanupState::Incomplete);
    assert_eq!(run.event_seq_reserved_through, 0);
    assert_eq!(run.observation_gap_count, 0);
    assert_eq!(run.mode_source, ModeSource::Default);
    assert_eq!(run.requested_mode, None);
    assert_eq!(run.resolved_mode, "never");
    assert_eq!(run.effective_policy_json, None);
    assert_eq!(run.child_pid, None);
    assert_eq!(run.exit_code, None);
    assert_eq!(run.exit_signal, None);
    assert_eq!(run.coverage, Coverage::Unknown);
}

#[test]
fn run_creation_validates_member_fleet_and_pinned_director_before_allocating() {
    for mismatch in ["member", "fleet", "director", "database"] {
        let mut fixture = Fixture::new();
        let (other_fleet, other_director) = common::create_fleet(&mut fixture.conn, "foreign");
        let member = common::register(&mut fixture.conn, fixture.fleet, "new-worker", None);
        let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
        match mismatch {
            "member" => spec.member_id = other_director,
            "fleet" => spec.fleet_id = other_fleet,
            "director" => spec.director_member_id = other_director,
            "database" => spec.database_identity_json["instance_id"] = json!("0".repeat(32)),
            _ => unreachable!(),
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::create_run(&mut fixture.conn, &spec).is_err(),
            "{mismatch}"
        );
        assert_eq!(durable_state(&fixture.conn), before, "{mismatch}");
    }
}

#[test]
fn explicit_mode_is_distinct_from_the_default_and_does_not_prove_effective_policy() {
    let mut fixture = Fixture::new();
    let member = common::register(&mut fixture.conn, fixture.fleet, "explicit", None);
    let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
    spec.requested_mode = Some("on-request".into());
    spec.resolved_mode = "on-request".into();
    spec.mode_source = ModeSource::Explicit;
    spec.permission_argv_json = json!([
        "--ask-for-approval",
        "on-request",
        "--sandbox",
        "workspace-write"
    ]);
    let run = storage::create_run(&mut fixture.conn, &spec).unwrap();
    assert_eq!(run.requested_mode.as_deref(), Some("on-request"));
    assert_eq!(run.mode_source, ModeSource::Explicit);
    assert_eq!(run.permission_argv_json, spec.permission_argv_json);
    assert_eq!(run.effective_policy_json, None);
}

#[test]
fn typed_runtime_readers_fail_on_corrupt_required_values() {
    for (field, value) in [
        ("status", "invented"),
        ("owner_kind", "model"),
        ("coverage", "guaranteed"),
        ("database_identity_json", "[]"),
        ("permission_argv_json", "{}"),
        ("owner_start_identity", ""),
    ] {
        let fixture = Fixture::new();
        fixture
            .conn
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        fixture
            .conn
            .execute(
                &format!("UPDATE member_runs SET {field}=?1 WHERE run_id=?2"),
                params![value, fixture.run.run_id],
            )
            .unwrap();
        let error = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap_err();
        assert!(error.message().contains(field), "{field}: {error:?}");
    }
}

#[test]
fn current_owner_reserves_disjoint_ranges_of_256_across_reconnections() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    assert_eq!(fixture.reserve(&owner), 1..=256);
    let mut other = fixture.reopen();
    assert_eq!(
        storage::reserve_event_sequences(&mut other, &owner).unwrap(),
        257..=512
    );
    assert_eq!(
        storage::get_run(&fixture.conn, owner.run_id)
            .unwrap()
            .event_seq_reserved_through,
        512
    );
    let transition = opening(owner.run_id, 257, 1, metadata());
    assert!(
        storage::persist_transition(&mut other, &owner, &transition, PersistenceMode::Live).is_ok()
    );
}

#[test]
fn stale_owner_kind_epoch_or_database_identity_fences_every_mutation() {
    for mismatch in ["epoch", "kind", "database"] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let mut stale = owner.clone();
        match mismatch {
            "epoch" => stale.owner_epoch += 1,
            "kind" => stale.owner_kind = OwnerKind::Reconciler,
            "database" => stale.database_identity_json["instance_id"] = json!("0".repeat(32)),
            _ => unreachable!(),
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::reserve_event_sequences(&mut fixture.conn, &stale).is_err(),
            "{mismatch}"
        );
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &stale,
                &opening(owner.run_id, 1, 1, metadata()),
                PersistenceMode::Live
            )
            .is_err(),
            "{mismatch}"
        );
        assert!(
            storage::reserve_input(
                &mut fixture.conn,
                &stale,
                &NewInput {
                    operation_key: "stale".into(),
                    kind: InputKind::Prompt,
                    payload_digest: "d".repeat(64),
                    message_id: None
                }
            )
            .is_err(),
            "{mismatch}"
        );
        assert_eq!(durable_state(&fixture.conn), before, "{mismatch}");
    }
}

#[test]
fn failed_sequence_reservation_rolls_back_the_high_water_mark() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.conn.execute_batch("CREATE TRIGGER fail_reservation BEFORE UPDATE OF event_seq_reserved_through ON member_runs BEGIN SELECT RAISE(ABORT,'injected reservation failure'); END").unwrap();
    assert!(storage::reserve_event_sequences(&mut fixture.conn, &owner).is_err());
    assert_eq!(
        storage::get_run(&fixture.conn, owner.run_id)
            .unwrap()
            .event_seq_reserved_through,
        0
    );
    fixture
        .conn
        .execute_batch("DROP TRIGGER fail_reservation")
        .unwrap();
    assert_eq!(fixture.reserve(&owner), 1..=256);
}

#[test]
fn opening_commits_one_complete_event_interaction_and_director_message() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let event = fixture.open(&owner);
    let interaction = event.interaction_id.unwrap();
    let message_id = event.message_id.unwrap();
    let reader = fixture.reopen();
    let message = super::get_message(&reader, message_id).unwrap();
    let payload: Value = serde_json::from_str(&message.text).unwrap();
    assert_eq!(
        (message.from_member_id, message.to_member_id),
        (fixture.member, fixture.director)
    );
    assert_eq!(message.status, super::records::MessageStatus::InputRequired);
    assert_eq!(payload, event.payload_json);
    for (key, id) in [
        ("event_id", event.event_id),
        ("run_id", owner.run_id),
        ("interaction_id", interaction),
        ("member_id", fixture.member),
        ("fleet_id", fixture.fleet),
        ("director_member_id", fixture.director),
    ] {
        assert_eq!(payload[key], id, "{key}");
    }
    assert_eq!(payload["schema_version"], 2);
    assert_eq!(payload["decision_outcome"], "unknown");
    assert_eq!(payload["native_request_id"], Value::Null);
    assert_eq!(payload["source"], "runner_screen");
    assert_eq!(
        super::poll_messages(&reader, fixture.director).unwrap(),
        [message]
    );
}

#[test]
fn identical_observed_transition_is_idempotent_without_generated_ids_or_commit_time() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.reserve(&owner);
    let transition = opening(owner.run_id, 1, 1, metadata());
    let first = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &transition,
        PersistenceMode::Live,
    )
    .unwrap();
    let before = durable_state(&fixture.conn);
    let mut other = fixture.reopen();
    let again = storage::persist_transition(&mut other, &owner, &transition, PersistenceMode::Live)
        .unwrap();
    assert_eq!(
        (again.event_id, again.message_id, again.interaction_id),
        (first.event_id, first.message_id, first.interaction_id)
    );
    assert_eq!(again.payload_json, first.payload_json);
    assert_eq!(durable_state(&other), before);
}

#[test]
fn each_conflicting_original_transition_field_rejects_sequence_reuse() {
    for changed in [
        "time",
        "episode",
        "kind",
        "classification",
        "evidence",
        "closure",
        "cause",
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let original = opening(owner.run_id, 1, 1, metadata());
        storage::persist_transition(&mut fixture.conn, &owner, &original, PersistenceMode::Live)
            .unwrap();
        let mut conflict = original.clone();
        match changed {
            "time" => conflict.observed_at = "2026-10-26T10:00:01.000000+00:00".into(),
            "episode" => conflict.episode_seq = Some(2),
            "kind" => conflict.kind = RuntimeEventKind::InteractionObscured,
            "classification" => conflict.classification = Some(InteractionClass::Question),
            "evidence" => conflict.evidence_json["screen_seq"] = json!(121),
            "closure" => conflict.close_reason = Some(InteractionCloseReason::Superseded),
            "cause" => conflict.cause_json = Some(json!({"kind":"changed"})),
            _ => unreachable!(),
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &conflict,
                PersistenceMode::Live
            )
            .is_err(),
            "{changed}"
        );
        assert_eq!(durable_state(&fixture.conn), before, "{changed}");
    }
}

#[test]
fn failure_at_each_transaction_stage_rolls_back_opening_and_inbox() {
    for target in [
        "BEFORE INSERT ON member_interactions",
        "BEFORE INSERT ON member_runtime_events",
        "BEFORE INSERT ON messages",
        "commit",
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        if target == "commit" {
            use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
            fixture
                .conn
                .authorizer(Some(|context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Transaction {
                            // rusqlite 0.39 maps SQLite's COMMIT operation to Unknown.
                            operation: TransactionOperation::Unknown
                        }
                    ) {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }))
                .unwrap();
        } else {
            fixture.conn.execute_batch(&format!("CREATE TRIGGER injected_failure {target} BEGIN SELECT RAISE(ABORT,'injected event failure'); END")).unwrap();
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &opening(owner.run_id, 1, 1, metadata()),
                PersistenceMode::Live
            )
            .is_err(),
            "{target}"
        );
        assert_eq!(durable_state(&fixture.reopen()), before, "{target}");
    }
}

#[test]
fn closure_and_its_notification_roll_back_together() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.open(&owner);
    fixture.conn.execute_batch("CREATE TRIGGER fail_closure BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'closure inbox failure'); END").unwrap();
    let before = durable_state(&fixture.conn);
    assert!(
        storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &closure(owner.run_id, 2, 1),
            PersistenceMode::Live
        )
        .is_err()
    );
    assert_eq!(durable_state(&fixture.reopen()), before);
    fixture
        .conn
        .execute_batch("DROP TRIGGER fail_closure")
        .unwrap();
    let closed = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &closure(owner.run_id, 2, 1),
        PersistenceMode::Live,
    )
    .unwrap();
    let view = storage::get_event(&fixture.conn, closed.event_id).unwrap();
    assert_eq!(
        view.interaction.unwrap().close_reason,
        Some(InteractionCloseReason::NotVisible)
    );
    assert_eq!(closed.payload_json["decision_outcome"], "unknown");
}

#[test]
fn archived_recovery_selects_the_lifecycle_reason_without_fabricating_an_inbox() {
    for (damage, reason) in [
        ("member", "member_inactive"),
        ("director", "director_inactive"),
        ("fleet", "fleet_deleted"),
    ] {
        let mut fixture = Fixture::new();
        fixture.activate();
        let owner = fixture.recoverer();
        fixture.reserve(&owner);
        match damage {
            "member" => {
                super::deregister_member(&mut fixture.conn, fixture.member).unwrap();
            }
            "director" => {
                fixture.conn.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",params![NOW,fixture.director]).unwrap();
            }
            "fleet" => {
                fixture
                    .conn
                    .execute(
                        "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                        params![NOW, fixture.fleet],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let transition = opening(owner.run_id, 1, 1, metadata());
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live
            )
            .is_err()
        );
        assert_eq!(durable_state(&fixture.conn), before);
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Recovery,
        )
        .unwrap();
        let output = crate::presentation::runtime_event(
            &storage::get_event(&fixture.conn, event.event_id).unwrap(),
        );
        assert_eq!(output["notification_state"], "archived");
        assert_eq!(output["archive_reason"], reason);
        assert_eq!(event.message_id, None);
        assert!(rows(&fixture.conn, "messages").is_empty());
    }
}

#[test]
fn committed_inbox_retry_after_deregistration_retains_the_original_association() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let first = fixture.open(&owner);
    super::deregister_member(&mut fixture.conn, fixture.member).unwrap();
    let recovery = fixture.recoverer();
    let retry = storage::persist_transition(
        &mut fixture.conn,
        &recovery,
        &opening(owner.run_id, 1, 1, metadata()),
        PersistenceMode::Recovery,
    )
    .unwrap();
    assert_eq!(
        (retry.event_id, retry.message_id),
        (first.event_id, first.message_id)
    );
    assert_eq!(retry.notification_state, NotificationState::Inbox);
    assert_eq!(rows(&fixture.conn, "messages").len(), 1);
}

#[test]
fn ack_consumes_the_message_without_closing_the_observed_interaction() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let event = fixture.open(&owner);
    super::ack_message(&mut fixture.conn, event.message_id.unwrap()).unwrap();
    let view = storage::get_event(&fixture.reopen(), event.event_id).unwrap();
    assert_eq!(view.interaction.unwrap().closed_at, None);
    assert_eq!(view.event.payload_json["decision_outcome"], "unknown");
    assert!(
        super::poll_messages(&fixture.conn, fixture.director)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn nonexistent_episode_closure_and_corrupt_cross_run_links_fail_explicitly() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.reserve(&owner);
    assert!(
        storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &closure(owner.run_id, 1, 404),
            PersistenceMode::Live
        )
        .is_err()
    );
    let event = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &opening(owner.run_id, 1, 1, metadata()),
        PersistenceMode::Live,
    )
    .unwrap();
    let other_member = common::register(&mut fixture.conn, fixture.fleet, "other-worker", None);
    let spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, other_member);
    let other = storage::create_run(&mut fixture.conn, &spec).unwrap();
    fixture
        .conn
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON")
        .unwrap();
    fixture
        .conn
        .execute(
            "UPDATE member_runtime_events SET run_id=?1 WHERE event_id=?2",
            params![other.run_id, event.event_id],
        )
        .unwrap();
    assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
}

#[test]
fn refusal_without_native_correlation_and_run_exit_keep_nullable_episode_links() {
    for (kind, source, cause) in [
        (
            RuntimeEventKind::PermissionDeniedObserved,
            "runner_screen",
            None,
        ),
        (
            RuntimeEventKind::RunnerExited,
            "runner_lifecycle",
            Some(json!({"kind":"native_exit","exit_code":7,"exit_signal":null})),
        ),
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let transition = ObservedTransition {
            run_id: owner.run_id,
            event_seq: 1,
            episode_seq: None,
            kind,
            classification: None,
            observed_at: NOW.into(),
            evidence_json: metadata(),
            close_reason: None,
            cause_json: cause,
        };
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Live,
        )
        .unwrap();
        assert_eq!(event.interaction_id, None);
        assert_eq!(event.payload_json["interaction_id"], Value::Null);
        assert_eq!(event.payload_json["classification"], Value::Null);
        assert_eq!(event.payload_json["source"], source);
        assert_eq!(event.payload_json["decision_outcome"], "unknown");
    }
}

#[test]
fn event_read_rejects_a_tampered_message_body_or_public_identity() {
    for damage in [
        "UPDATE messages SET text='{}'",
        "UPDATE member_runtime_events SET payload_json='{\"schema_version\":1,\"run_id\":404}'",
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        fixture.conn.execute_batch(damage).unwrap();
        assert!(
            storage::get_event(&fixture.conn, event.event_id).is_err(),
            "{damage}"
        );
    }
}

#[test]
fn receipt_key_digest_and_settled_disposition_survive_reconnection() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let input = fixture.input(&owner, "prompt:one", InputKind::Prompt, None);
    assert_eq!(input.state, InputState::InProgress);
    let request = NewInput {
        operation_key: "prompt:one".into(),
        kind: InputKind::Prompt,
        payload_digest: "d".repeat(64),
        message_id: None,
    };
    let mut other = fixture.reopen();
    assert_eq!(
        storage::reserve_input(&mut other, &owner, &request)
            .unwrap()
            .input_id,
        input.input_id
    );
    let settled = storage::settle_input(
        &mut other,
        &owner,
        input.input_id,
        InputState::InProgress,
        InputDisposition::NotSubmitted,
        "audit:zero-byte-confirmation",
    )
    .unwrap();
    assert_eq!(settled.state, InputState::Settled);
    let retry = storage::reserve_input(&mut fixture.conn, &owner, &request).unwrap();
    assert_eq!(retry.input_id, input.input_id);
    assert_eq!(retry.disposition, Some(InputDisposition::NotSubmitted));
    assert_eq!(rows(&fixture.conn, "member_input_operations").len(), 1);
}

#[test]
fn operation_key_rejects_a_changed_digest_kind_or_message() {
    for changed in ["digest", "kind", "message"] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.input(&owner, "same-key", InputKind::Prompt, None);
        let message = super::send_message(
            &mut fixture.conn,
            fixture.director,
            fixture.member,
            "preview",
        )
        .unwrap();
        let mut request = NewInput {
            operation_key: "same-key".into(),
            kind: InputKind::Prompt,
            payload_digest: "d".repeat(64),
            message_id: None,
        };
        match changed {
            "digest" => request.payload_digest = "e".repeat(64),
            "kind" => request.kind = InputKind::Exec,
            "message" => {
                request.kind = InputKind::Preview;
                request.message_id = Some(message.message_id);
            }
            _ => unreachable!(),
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::reserve_input(&mut fixture.conn, &owner, &request).is_err(),
            "{changed}"
        );
        assert_eq!(durable_state(&fixture.conn), before);
    }
}

#[test]
fn preview_receipt_requires_a_message_addressed_to_its_run_member() {
    for destination in ["missing", "director", "foreign"] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let recipient = if destination == "director" {
            fixture.director
        } else {
            common::register(&mut fixture.conn, fixture.fleet, "other", None)
        };
        let message = super::send_message(
            &mut fixture.conn,
            fixture.director,
            recipient,
            "other-message",
        )
        .unwrap();
        let request = NewInput {
            operation_key: format!("preview:{}:{}", owner.run_id, message.message_id),
            kind: InputKind::Preview,
            payload_digest: "d".repeat(64),
            message_id: if destination == "missing" {
                None
            } else {
                Some(message.message_id)
            },
        };
        assert!(
            storage::reserve_input(&mut fixture.conn, &owner, &request).is_err(),
            "{destination}"
        );
        assert!(rows(&fixture.conn, "member_input_operations").is_empty());
    }
}

#[test]
fn accepted_preview_settlement_and_message_stamp_are_atomic() {
    for already_acked in [false, true] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let message = super::send_message(
            &mut fixture.conn,
            fixture.director,
            fixture.member,
            "held preview",
        )
        .unwrap();
        let input = fixture.input(
            &owner,
            &format!("preview:{}:{}", owner.run_id, message.message_id),
            InputKind::Preview,
            Some(message.message_id),
        );
        storage::record_input_state(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputState::Accepted,
        )
        .unwrap();
        if already_acked {
            super::ack_message(&mut fixture.conn, message.message_id).unwrap();
        }
        let settled = storage::settle_preview(&mut fixture.conn, &owner, input.input_id).unwrap();
        assert_eq!(settled.state, InputState::Settled);
        assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
        let stamp: Option<String> = fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stamp.is_some());
    }
}

#[test]
fn failed_preview_stamp_retains_the_durable_accepted_receipt_for_a_later_invocation() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let message = super::send_message(
        &mut fixture.conn,
        fixture.director,
        fixture.member,
        "preview",
    )
    .unwrap();
    let input = fixture.input(
        &owner,
        &format!("preview:{}:{}", owner.run_id, message.message_id),
        InputKind::Preview,
        Some(message.message_id),
    );
    storage::record_input_state(
        &mut fixture.conn,
        &owner,
        input.input_id,
        InputState::InProgress,
        InputState::Accepted,
    )
    .unwrap();
    fixture.conn.execute_batch("CREATE TRIGGER fail_stamp BEFORE UPDATE OF notified_at ON messages BEGIN SELECT RAISE(ABORT,'stamp failure'); END").unwrap();
    assert!(storage::settle_preview(&mut fixture.conn, &owner, input.input_id).is_err());
    let mut other = fixture.reopen();
    let durable = storage::find_input(&other, owner.run_id, &input.operation_key)
        .unwrap()
        .unwrap();
    assert_eq!(durable.state, InputState::Accepted);
    assert_eq!(
        other
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
    other.execute_batch("DROP TRIGGER fail_stamp").unwrap();
    storage::settle_preview(&mut other, &owner, input.input_id).unwrap();
    assert_eq!(rows(&other, "member_input_operations").len(), 1);
}

#[test]
fn settlement_cas_rejects_stale_state_and_preserves_recorded_evidence() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let input = fixture.input(&owner, "exec:lost-stdout", InputKind::Exec, None);
    storage::record_input_state(
        &mut fixture.conn,
        &owner,
        input.input_id,
        InputState::InProgress,
        InputState::Uncertain,
    )
    .unwrap();
    assert!(
        storage::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputDisposition::Submitted,
            "audit:stale"
        )
        .is_err()
    );
    assert!(
        storage::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::Uncertain,
            InputDisposition::Submitted,
            " "
        )
        .is_err()
    );
    let settled = storage::settle_input(
        &mut fixture.conn,
        &owner,
        input.input_id,
        InputState::Uncertain,
        InputDisposition::AbandonedUncertain,
        "audit:inspected-output",
    )
    .unwrap();
    assert_eq!(
        settled.disposition,
        Some(InputDisposition::AbandonedUncertain)
    );
    assert_eq!(
        settled.evidence_ref.as_deref(),
        Some("audit:inspected-output")
    );
    let read = storage::get_input(&fixture.reopen(), input.input_id).unwrap();
    assert_eq!(read.state, InputState::Settled);
    assert!(
        storage::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::Uncertain,
            InputDisposition::NotSubmitted,
            "audit:changed"
        )
        .is_err()
    );
    assert!(rows(&fixture.conn, "member_runtime_events").is_empty());
}

#[test]
fn settled_receipt_history_can_exceed_the_unresolved_cache_bound() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    for index in 0..4097 {
        let input = fixture.input(&owner, &format!("settled:{index}"), InputKind::Prompt, None);
        storage::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputDisposition::NotSubmitted,
            "audit:zero-byte",
        )
        .unwrap();
    }
    assert_eq!(rows(&fixture.conn, "member_input_operations").len(), 4097);
    assert!(
        storage::find_input(&fixture.reopen(), owner.run_id, "settled:0")
            .unwrap()
            .is_some()
    );
}

#[test]
fn metadata_removes_all_screen_fields_and_hashes_only_post_policy_evidence() {
    let policy = evidence::EvidencePolicy::metadata();
    let first = policy.apply(&raw_evidence(SECRET)).unwrap();
    let second = policy
        .apply(&raw_evidence("a different hidden native request"))
        .unwrap();
    for field in ["operation", "choices", "native_request_id", "excerpt"] {
        assert_eq!(first.get(field), Some(&Value::Null), "{field}");
    }
    assert_eq!(first["evidence_digest"], second["evidence_digest"]);
    assert_eq!(first["evidence_digest"].as_str().unwrap().len(), 64);
    assert!(!first.to_string().contains(SECRET));
    assert!(first.get("semantic_fingerprint").is_none());
    assert_eq!(first["evidence_policy"], "metadata");
}

#[test]
fn matching_redaction_replaces_each_normalized_field_in_full() {
    let policy = evidence::EvidencePolicy::redacted(
        &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
        &["native-tool-permission"],
    )
    .unwrap();
    let sanitized = policy.apply(&raw_evidence(SECRET)).unwrap();
    for field in ["operation", "native_request_id", "excerpt"] {
        assert_eq!(sanitized[field], "[redacted]", "{field}");
    }
    assert_eq!(sanitized["choices"], json!(["[redacted]", "deny"]));
    assert_eq!(sanitized["evidence_policy"], "redacted");
    assert!(!sanitized.to_string().contains(SECRET));
    assert!(sanitized.get("semantic_fingerprint").is_none());
}

#[test]
fn sensitive_forms_and_selected_rules_reduce_the_entire_form_to_metadata() {
    for by_profile in [false, true] {
        let sensitive_rules = if by_profile {
            json!([])
        } else {
            json!(["native-tool-permission"])
        };
        let policy = evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":["unmatched"],"sensitive_rules":sensitive_rules}),
            &["native-tool-permission"],
        )
        .unwrap();
        let mut raw = raw_evidence(SECRET);
        raw["sensitive"] = json!(by_profile);
        let sanitized = policy.apply(&raw).unwrap();
        for field in ["operation", "choices", "native_request_id", "excerpt"] {
            assert_eq!(sanitized[field], Value::Null, "{field}");
        }
        assert!(!sanitized.to_string().contains(SECRET));
    }
}

#[test]
fn invalid_redaction_configuration_fails_instead_of_retaining_raw_evidence() {
    for config in [
        json!({"schema_version":2,"patterns":["secret"],"sensitive_rules":[]}),
        json!({"schema_version":1,"patterns":[],"sensitive_rules":[]}),
        json!({"schema_version":1,"patterns":["["],"sensitive_rules":[]}),
        json!({"schema_version":1,"patterns":["secret"],"sensitive_rules":["unknown-rule"]}),
        json!([]),
    ] {
        assert!(
            evidence::EvidencePolicy::redacted(&config, &["native-tool-permission"]).is_err(),
            "{config}"
        );
    }
}

#[test]
fn metadata_and_redacted_transitions_remain_secret_free_through_storage_and_updates() {
    for redacted in [false, true] {
        let mut fixture = Fixture::new();
        if redacted {
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
        }
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let policy = if redacted {
            evidence::EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
                &["native-tool-permission"],
            )
            .unwrap()
        } else {
            evidence::EvidencePolicy::metadata()
        };
        let transformed = policy.apply(&raw_evidence(SECRET)).unwrap();
        let transition = opening(owner.run_id, 1, 1, transformed.clone());
        assert!(!serde_json::to_string(&transition).unwrap().contains(SECRET));
        let first = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Live,
        )
        .unwrap();
        let update = ObservedTransition {
            event_seq: 2,
            kind: RuntimeEventKind::InteractionObscured,
            evidence_json: transformed,
            ..transition
        };
        storage::persist_transition(&mut fixture.conn, &owner, &update, PersistenceMode::Live)
            .unwrap();
        assert_no_secret(&fixture.conn, SECRET);
        let current = storage::get_event(&fixture.reopen(), first.event_id).unwrap();
        assert!(
            !current
                .interaction
                .unwrap()
                .evidence_json
                .to_string()
                .contains(SECRET)
        );
    }
}

#[test]
fn evidence_utf8_and_complete_payload_bounds_record_truncation_after_policy() {
    let policy = evidence::EvidencePolicy::redacted(
        &json!({"schema_version":1,"patterns":["unmatched"],"sensitive_rules":[]}),
        &["native-tool-permission"],
    )
    .unwrap();
    let mut raw = raw_evidence("ordinary");
    raw["excerpt"] = json!(format!("{}\u{1b}[2J\n", "界".repeat(8192)));
    let transformed = policy.apply(&raw).unwrap();
    assert!(transformed["excerpt"].as_str().unwrap().len() <= 8192);
    assert_eq!(transformed["truncated"], true);
    let mut fixture = Fixture::new();
    fixture
        .conn
        .execute(
            "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
            [fixture.run.run_id],
        )
        .unwrap();
    let owner = fixture.activate();
    fixture.reserve(&owner);
    let event = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &opening(owner.run_id, 1, 1, transformed),
        PersistenceMode::Live,
    )
    .unwrap();
    let message = super::get_message(&fixture.conn, event.message_id.unwrap()).unwrap();
    assert!(message.text.len() <= 16384);
    assert!(serde_json::from_str::<Value>(&message.text).is_ok());
    assert!(!message.text.contains('\u{1b}'));
}

#[test]
fn runtime_presentation_exposes_recorded_owner_snapshot_and_unresolved_inputs() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let event = fixture.open(&owner);
    let input = fixture.input(&owner, "exec:uncertain", InputKind::Exec, None);
    let member = super::get_member(&fixture.conn, fixture.member, fixture.fleet)
        .unwrap()
        .unwrap();
    let output = crate::presentation::member(&member);
    assert_key_order(
        &output,
        &[
            "member_id",
            "name",
            "description",
            "status",
            "registered_at",
            "kind",
            "placement",
            "runtime",
        ],
    );
    let runtime = &output["runtime"];
    assert_key_order(
        runtime,
        &[
            "run_id",
            "resolved_mode",
            "status",
            "reachability",
            "owner_epoch",
            "last_health_at",
            "coverage",
            "current_interaction_id",
            "unresolved_input_ids",
            "owner_kind",
            "owner_pid",
            "owner_start_identity",
            "runner_pid",
            "runner_start_identity",
            "child_pid",
            "child_start_identity",
        ],
    );
    assert_eq!(runtime["run_id"], owner.run_id);
    assert_eq!(
        runtime["current_interaction_id"],
        event.interaction_id.unwrap()
    );
    assert_eq!(runtime["unresolved_input_ids"], json!([input.input_id]));
    assert_eq!(runtime["owner_start_identity"], "fixture-start-identity");
    assert_eq!(runtime["child_pid"], Value::Null);
    assert_eq!(runtime["child_start_identity"], Value::Null);
    let unwrapped = super::get_member(&fixture.conn, fixture.director, fixture.fleet)
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::presentation::member(&unwrapped).get("runtime"),
        Some(&Value::Null)
    );
    let text = crate::output::format_member_detail(&member);
    assert_eq!(
        text,
        format!(
            "{} worker active\nRuntime: run={} mode=never status=running coverage=unknown",
            fixture.member, owner.run_id
        )
    );
}

#[test]
fn event_and_input_read_wrappers_follow_the_approved_order_and_compact_text() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let event = fixture.open(&owner);
    let view = storage::get_event(&fixture.conn, event.event_id).unwrap();
    let output = crate::presentation::runtime_event(&view);
    assert_key_order(
        &output,
        &[
            "event",
            "notification_state",
            "message_id",
            "archive_reason",
            "run",
            "interaction",
        ],
    );
    assert_key_order(
        &output["interaction"],
        &[
            "interaction_id",
            "run_id",
            "episode_seq",
            "classification",
            "evidence_digest",
            "opened_at",
            "evidence_json",
            "closed_at",
            "close_reason",
        ],
    );
    assert_eq!(
        crate::output::format_runtime_event(&view),
        format!(
            "Event: {} run={} seq=1 kind=interaction_opened notification=inbox message={}\nRuntime: run={} mode=never status=running coverage=unknown\nInteraction: {} classification=permission_request closed_at=- close_reason=-",
            event.event_id,
            owner.run_id,
            event.message_id.unwrap(),
            owner.run_id,
            event.interaction_id.unwrap()
        )
    );
    let input = fixture.input(&owner, "exec:lost-output", InputKind::Exec, None);
    let output = crate::presentation::input_operation(&input);
    assert_key_order(
        &output["input"],
        &[
            "input_id",
            "run_id",
            "operation_key",
            "kind",
            "payload_digest",
            "state",
            "created_at",
            "updated_at",
            "message_id",
            "disposition",
            "evidence_ref",
            "permission_response",
        ],
    );
    assert_eq!(output["input"]["permission_response"], Value::Null);
    assert_eq!(
        crate::output::format_input_operation(&input),
        format!(
            "Input: {} run={} kind=exec state=in_progress disposition=- message=-",
            input.input_id, owner.run_id
        )
    );
}

#[test]
fn concurrent_sequence_reservations_commit_disjoint_ranges() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let url = format!("sqlite:///{}", fixture.conn.path().unwrap());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let url = url.clone();
            let owner = owner.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut conn = crate::db::connect(&url).unwrap();
                barrier.wait();
                storage::reserve_event_sequences(&mut conn, &owner).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let mut ranges = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| *range.start());
    assert_eq!(ranges, [1..=256, 257..=512]);
}

#[test]
fn transition_outside_reserved_sequences_or_with_invalid_payload_is_rejected_atomically() {
    for invalid in ["unreserved", "wrong-run", "missing-evidence", "array"] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let mut transition = opening(owner.run_id, 1, 1, metadata());
        match invalid {
            "unreserved" => transition.event_seq = 257,
            "wrong-run" => transition.run_id += 1,
            "missing-evidence" => transition.evidence_json = json!({}),
            "array" => transition.evidence_json = json!([]),
            _ => unreachable!(),
        }
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live
            )
            .is_err(),
            "{invalid}"
        );
        assert_eq!(durable_state(&fixture.conn), before);
    }
}

#[test]
fn failed_input_reservation_and_corrupt_receipt_reads_surface_the_original_state() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.conn.execute_batch("CREATE TRIGGER fail_input BEFORE INSERT ON member_input_operations BEGIN SELECT RAISE(ABORT,'injected input failure'); END").unwrap();
    assert!(
        storage::reserve_input(
            &mut fixture.conn,
            &owner,
            &NewInput {
                operation_key: "reserved".into(),
                kind: InputKind::Exec,
                payload_digest: "d".repeat(64),
                message_id: None
            }
        )
        .is_err()
    );
    assert!(rows(&fixture.reopen(), "member_input_operations").is_empty());
    fixture
        .conn
        .execute_batch("DROP TRIGGER fail_input")
        .unwrap();
    let input = fixture.input(&owner, "reserved", InputKind::Exec, None);
    fixture
        .conn
        .execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    fixture
        .conn
        .execute(
            "UPDATE member_input_operations SET state='corrupt' WHERE input_id=?1",
            [input.input_id],
        )
        .unwrap();
    let error = storage::get_input(&fixture.conn, input.input_id).unwrap_err();
    assert!(error.message().contains("state"));
    assert!(error.message().contains("corrupt"));
}

#[test]
fn missing_database_instance_identity_fails_at_the_identity_read_boundary() {
    let fixture = Fixture::new();
    fixture
        .conn
        .execute("DELETE FROM runtime_database_identity", [])
        .unwrap();
    assert!(storage::database_identity(&fixture.conn).is_err());
}

fn native_exit(run_id: i64, event_seq: i64) -> ObservedTransition {
    ObservedTransition {
        run_id,
        event_seq,
        episode_seq: None,
        kind: RuntimeEventKind::RunnerExited,
        classification: None,
        observed_at: NOW.into(),
        evidence_json: metadata(),
        close_reason: None,
        cause_json: Some(json!({"kind":"native_exit","exit_code":7,"exit_signal":null})),
    }
}

fn replace_public_payload(conn: &mut Connection, event: &RuntimeEventRecord, payload: Value) {
    let tx = conn.transaction().unwrap();
    tx.execute(
        "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
        params![payload.to_string(), event.event_id],
    )
    .unwrap();
    if let Some(message_id) = event.message_id {
        tx.execute(
            "UPDATE messages SET text=?1 WHERE message_id=?2",
            params![payload.to_string(), message_id],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn terminal_live_write_regression_keeps_committed_retries_and_rejects_new_observations() {
    for status in ["exited", "failed", "lost"] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let exit = native_exit(owner.run_id, 1);
        let first =
            storage::persist_transition(&mut fixture.conn, &owner, &exit, PersistenceMode::Live)
                .unwrap();
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .status,
            RunStatus::Exited
        );
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                params![status, owner.run_id],
            )
            .unwrap();
        let retry =
            storage::persist_transition(&mut fixture.conn, &owner, &exit, PersistenceMode::Live)
                .unwrap();
        assert_eq!(
            (retry.event_id, retry.message_id),
            (first.event_id, first.message_id)
        );
        let next = ObservedTransition {
            event_seq: 2,
            kind: RuntimeEventKind::PermissionDeniedObserved,
            cause_json: None,
            ..exit
        };
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(&mut fixture.conn, &owner, &next, PersistenceMode::Live)
                .is_err(),
            "{status}"
        );
        assert_eq!(durable_state(&fixture.reopen()), before, "{status}");
    }
}

#[test]
fn terminal_live_write_regression_preserves_explicitly_owned_historical_recovery() {
    let mut fixture = Fixture::new();
    fixture.activate();
    let owner = fixture.recoverer();
    fixture.reserve(&owner);
    fixture
        .conn
        .execute(
            "UPDATE member_runs SET recovery_origin_status='exited' WHERE run_id=?1",
            [owner.run_id],
        )
        .unwrap();
    super::deregister_member(&mut fixture.conn, fixture.member).unwrap();
    let opened = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &opening(owner.run_id, 1, 1, metadata()),
        PersistenceMode::Recovery,
    )
    .unwrap();
    let mut closed = closure(owner.run_id, 2, 1);
    closed.close_reason = Some(InteractionCloseReason::ProcessExited);
    storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &closed,
        PersistenceMode::Recovery,
    )
    .unwrap();
    let exited = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &native_exit(owner.run_id, 3),
        PersistenceMode::Recovery,
    )
    .unwrap();
    assert_eq!(opened.notification_state, NotificationState::Archived);
    assert_eq!(exited.notification_state, NotificationState::Archived);
    assert_eq!(exited.message_id, None);
    assert_eq!(
        storage::get_run(&fixture.conn, owner.run_id)
            .unwrap()
            .status,
        RunStatus::Exited
    );
    assert!(rows(&fixture.conn, "messages").is_empty());
}

#[test]
fn event_shape_regression_rejects_lifecycle_transitions_linked_to_an_episode() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    fixture.open(&owner);
    let transition = ObservedTransition {
        episode_seq: Some(1),
        classification: Some(InteractionClass::PermissionRequest),
        ..native_exit(owner.run_id, 2)
    };
    let before = durable_state(&fixture.conn);
    assert!(
        storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Live
        )
        .is_err()
    );
    assert_eq!(durable_state(&fixture.reopen()), before);
}

#[test]
fn event_shape_regression_requires_a_typed_lifecycle_cause_before_commit() {
    for kind in [
        RuntimeEventKind::CoverageUnknown,
        RuntimeEventKind::RunnerDegraded,
        RuntimeEventKind::RunnerLost,
        RuntimeEventKind::RunnerExited,
    ] {
        for cause in [
            None,
            Some(json!({})),
            Some(json!({"kind":"invented-cause"})),
            Some(json!([])),
        ] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let transition = ObservedTransition {
                kind,
                cause_json: cause,
                ..native_exit(owner.run_id, 1)
            };
            let before = durable_state(&fixture.conn);
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live
                )
                .is_err(),
                "{kind:?}: {:?}",
                transition.cause_json
            );
            assert_eq!(durable_state(&fixture.reopen()), before);
        }
    }
}

#[test]
fn event_shape_regression_rejects_lifecycle_causes_on_screen_transitions() {
    for kind in [
        RuntimeEventKind::InteractionOpened,
        RuntimeEventKind::InteractionObscured,
        RuntimeEventKind::InteractionClosed,
        RuntimeEventKind::PermissionDeniedObserved,
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let mut transition = if matches!(
            kind,
            RuntimeEventKind::InteractionObscured | RuntimeEventKind::InteractionClosed
        ) {
            fixture.open(&owner);
            closure(owner.run_id, 2, 1)
        } else {
            fixture.reserve(&owner);
            opening(owner.run_id, 1, 1, metadata())
        };
        transition.kind = kind;
        transition.close_reason = if kind == RuntimeEventKind::InteractionClosed {
            Some(InteractionCloseReason::NotVisible)
        } else {
            None
        };
        if kind == RuntimeEventKind::PermissionDeniedObserved {
            transition.episode_seq = None;
            transition.classification = None;
        }
        transition.cause_json =
            Some(json!({"kind":"native_exit","exit_code":7,"exit_signal":null}));
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live
            )
            .is_err(),
            "{kind:?}"
        );
        assert_eq!(durable_state(&fixture.reopen()), before);
    }
}

#[test]
fn event_shape_regression_rejects_a_persisted_lifecycle_episode_even_with_a_matching_message() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let opened = fixture.open(&owner);
    storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &closure(owner.run_id, 2, 1),
        PersistenceMode::Live,
    )
    .unwrap();
    let event = storage::persist_transition(
        &mut fixture.conn,
        &owner,
        &native_exit(owner.run_id, 3),
        PersistenceMode::Live,
    )
    .unwrap();
    let mut payload = event.payload_json.clone();
    payload["interaction_id"] = json!(opened.interaction_id.unwrap());
    payload["classification"] = json!("permission_request");
    fixture
        .conn
        .execute(
            "UPDATE member_runtime_events SET interaction_id=?1 WHERE event_id=?2",
            params![opened.interaction_id, event.event_id],
        )
        .unwrap();
    replace_public_payload(&mut fixture.conn, &event, payload);
    assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
}

#[test]
fn event_shape_regression_rejects_missing_or_malformed_persisted_exit_causes() {
    for cause in [
        None,
        Some(json!({})),
        Some(json!({"kind":"invented-cause","exit_code":7})),
        Some(json!({"kind":"native_exit","exit_code":"7","exit_signal":null})),
    ] {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &native_exit(owner.run_id, 1),
            PersistenceMode::Live,
        )
        .unwrap();
        let mut payload = event.payload_json.clone();
        match cause {
            Some(cause) => {
                payload["cause"] = cause;
            }
            None => {
                payload.as_object_mut().unwrap().remove("cause");
            }
        }
        replace_public_payload(&mut fixture.conn, &event, payload);
        assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
    }
}

#[test]
fn event_shape_regression_rejects_a_persisted_screen_cause_even_with_a_matching_message() {
    let mut fixture = Fixture::new();
    let owner = fixture.activate();
    let event = fixture.open(&owner);
    let mut payload = event.payload_json.clone();
    payload["cause"] = json!({"kind":"native_exit","exit_code":7,"exit_signal":null});
    replace_public_payload(&mut fixture.conn, &event, payload);
    assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
}

#[test]
fn event_shape_regression_binds_public_native_request_id_to_post_policy_evidence() {
    for redacted in [false, true] {
        let mut fixture = Fixture::new();
        if redacted {
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
        }
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let evidence = if redacted {
            evidence::EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":["unmatched"],"sensitive_rules":[]}),
                &["native-tool-permission"],
            )
            .unwrap()
            .apply(&raw_evidence("ordinary-request"))
            .unwrap()
        } else {
            metadata()
        };
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &opening(owner.run_id, 1, 1, evidence),
            PersistenceMode::Live,
        )
        .unwrap();
        assert_eq!(
            event.payload_json["native_request_id"],
            event.payload_json["evidence"]["native_request_id"]
        );
        storage::get_event(&fixture.conn, event.event_id).unwrap();
        let mut payload = event.payload_json.clone();
        payload["native_request_id"] = json!("unrelated-public-request");
        replace_public_payload(&mut fixture.conn, &event, payload);
        assert!(
            storage::get_event(&fixture.conn, event.event_id).is_err(),
            "redacted={redacted}"
        );
    }
}
