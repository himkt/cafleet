use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::Mutex;

#[test]
fn step5_a1_h1_released_not_submitted_claim_remains_valid_consumed_history() {
    let mut case = Case::new(Retained::NotSubmitted);
    assert_eq!(
        case.fixture
            .conn
            .execute(
                "DELETE FROM member_permission_response_claims WHERE input_id=?1",
                [case.input_id],
            )
            .unwrap(),
        1
    );
    let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
    assert!(receipt.claim.is_none());
    assert_eq!(receipt.input.state, InputState::Settled);
    assert_eq!(
        receipt.input.disposition,
        Some(InputDisposition::NotSubmitted)
    );
    assert_eq!(receipt.response.metadata, case.request.metadata);
    let request = command(&case);
    let before = Snapshot::take(&case.fixture.conn);
    let result = success(attempt(&mut case, &request));
    assert_projection(&case, &result, Retained::NotSubmitted);
    before.unchanged(&case.fixture.conn);
    assert_eq!(
        storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
        receipt
    );
}

fn related(case: &Case) -> Case {
    let mut conn = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let mut placement = common::placement(Some("%3"));
    placement.coding_agent = "codex".into();
    let member = crate::broker::register_member(
        &mut conn,
        case.fixture.run.fleet_id,
        "related-history-worker",
        "assignment",
        Some(&placement),
    )
    .unwrap()
    .member_id;
    let next_id = conn
        .query_row(
            "SELECT COALESCE(MAX(run_id),0)+1 FROM member_runs",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let process = ProcessIdentity::current().unwrap();
    let original = &case.fixture.run;
    let run = storage::create_run(
        &mut conn,
        &NewRun {
            member_id: member,
            fleet_id: original.fleet_id,
            director_member_id: original.director_member_id,
            backend: original.backend.clone(),
            database_identity_json: original.database_identity_json.clone(),
            launch_nonce: "b".repeat(64),
            runtime_path: RuntimePaths::for_run(&original.database_identity_json, next_id)
                .unwrap()
                .run_dir
                .to_str()
                .unwrap()
                .into(),
            requested_mode: original.requested_mode.clone(),
            resolved_mode: original.resolved_mode.clone(),
            mode_source: original.mode_source,
            permission_argv_json: original.permission_argv_json.clone(),
            event_evidence: original.event_evidence,
            native_version: original.native_version.clone(),
            profile_version: None,
            coverage: Coverage::Unknown,
            owner_pid: process.pid,
            owner_start_identity: process.start_identity,
        },
    )
    .unwrap();
    assert_eq!(run.run_id, next_id);
    Case::from_fixture(
        Fixture {
            _dir: tempfile::tempdir().unwrap(),
            conn,
            run,
        },
        Retained::Accepted,
        PermissionDecision::Allow,
        DirectorMode::Auto,
        DecisionSource::DirectorAuto,
    )
}

#[test]
fn step5_a1_h3_existing_foreign_run_rejects_association_before_live_boundary() {
    let mut original = Case::new(Retained::Accepted);
    let foreign = related(&original);
    assert_ne!(original.owner.run_id, foreign.owner.run_id);
    assert_eq!(
        original.owner.database_identity_json,
        foreign.owner.database_identity_json
    );
    assert_eq!(
        storage::get_run(&original.fixture.conn, foreign.owner.run_id).unwrap(),
        storage::get_run(&foreign.fixture.conn, foreign.owner.run_id).unwrap()
    );
    let mut request = command(&original);
    request.expected_run_id = foreign.owner.run_id;
    request.expected_owner_epoch = foreign.owner.owner_epoch;
    request.operation_key = "fixture:absent-on-foreign-run".into();
    assert_eq!(
        original
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
                rusqlite::params![foreign.owner.run_id, request.operation_key],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    let before = Snapshot::take(&original.fixture.conn);
    let failure = match attempt(&mut original, &request) {
        Err(failure) => failure,
        Ok(_) => panic!("an existing foreign run refuses the original event/interaction"),
    };
    assert!(matches!(&failure.source,CafleetError::App(message)
        if message=="permission run/event/interaction/Director association mismatch"));
    assert_eq!(failure.source.exit_code(), 1);
    assert_eq!(failure.run_id, Some(foreign.owner.run_id));
    assert_eq!(
        failure.interaction_id,
        Some(original.request.metadata.interaction_id)
    );
    assert_eq!(failure.input_id, None);
    assert_eq!(
        failure.operation_key.as_deref(),
        Some(request.operation_key.as_str())
    );
    before.unchanged(&original.fixture.conn);
}

#[test]
fn step5_a1_h3_second_run_matching_history_is_independently_valid() {
    let original = Case::new(Retained::Accepted);
    let mut foreign = related(&original);
    let receipt =
        storage::get_permission_response(&foreign.fixture.conn, foreign.input_id).unwrap();
    assert_eq!(receipt.input.run_id, foreign.owner.run_id);
    assert_ne!(
        receipt.response.metadata.interaction_id,
        original.request.metadata.interaction_id
    );
    let request = command(&foreign);
    let before = Snapshot::take(&foreign.fixture.conn);
    let result = success(attempt(&mut foreign, &request));
    assert_projection(&foreign, &result, Retained::Accepted);
    before.unchanged(&foreign.fixture.conn);
}

#[derive(Debug)]
enum ObservedSql {
    Transaction(TransactionOperation),
    Read {
        table: String,
        transaction_open: bool,
    },
}

#[test]
fn step5_a1_h2_supplied_deferred_snapshot_survives_committed_association_change() {
    let mut case = Case::new(Retained::Accepted);
    let mode: String = case
        .fixture
        .conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    let writer = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
    writer.busy_timeout(Duration::from_millis(200)).unwrap();
    let writer_mode: String = writer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(writer_mode.to_ascii_lowercase(), "wal");
    let before = Snapshot::take(&case.fixture.conn);
    let observations = Arc::new(Mutex::new(Vec::new()));
    let observed = observations.clone();
    let mutation = Arc::new(Mutex::new(None));
    let committed = mutation.clone();
    let mut writer = Some(writer);
    let message_id = case.event.message_id.unwrap();
    let wrong_recipient = case.fixture.run.member_id;
    let mut transaction_open = false;
    case.fixture
        .conn
        .authorizer(Some(move |context: AuthContext<'_>| {
            match context.action {
                AuthAction::Transaction { operation } => {
                    transaction_open = operation == TransactionOperation::Begin;
                    observed
                        .lock()
                        .unwrap()
                        .push(ObservedSql::Transaction(operation));
                }
                AuthAction::Read { table_name, .. } => {
                    observed.lock().unwrap().push(ObservedSql::Read {
                        table: table_name.into(),
                        transaction_open,
                    });
                    if table_name == "member_permission_responses"
                        && let Some(writer) = writer.take()
                    {
                        let result = (|| -> rusqlite::Result<Snapshot> {
                            let changed = writer.execute(
                                "UPDATE messages SET to_member_id=?1 WHERE message_id=?2",
                                rusqlite::params![wrong_recipient, message_id],
                            )?;
                            if changed != 1 || !writer.is_autocommit() {
                                return Err(rusqlite::Error::InvalidQuery);
                            }
                            Ok(Snapshot::take(&writer))
                        })();
                        *committed.lock().unwrap() = Some(result);
                    }
                }
                _ => {}
            }
            Authorization::Allow
        }))
        .unwrap();
    let request = command(&case);
    let result = attempt(&mut case, &request);
    case.fixture
        .conn
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    let observed = observations.lock().unwrap();
    let transactions = observed
        .iter()
        .filter_map(|operation| match operation {
            ObservedSql::Transaction(operation) => Some(*operation),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        transactions,
        // rusqlite represents SQLite's COMMIT authorization action as Unknown.
        vec![TransactionOperation::Begin, TransactionOperation::Unknown],
        "{observed:?}"
    );
    assert!(case.fixture.conn.is_autocommit());
    for table in [
        "member_runs",
        "member_input_operations",
        "member_permission_responses",
        "member_runtime_events",
        "member_interactions",
        "messages",
        "member_permission_response_claims",
    ] {
        assert!(observed.iter().any(|operation|matches!(operation,ObservedSql::Read {table:actual,transaction_open:true} if actual==table)),"supplied-connection preparation inside the observed transaction for {table}: {observed:?}");
    }
    assert!(
        observed.iter().all(|operation| !matches!(
            operation,
            ObservedSql::Read {
                transaction_open: false,
                ..
            }
        )),
        "every supplied-connection read is enclosed by the single transaction: {observed:?}"
    );
    eprintln!(
        "H2 supplied-connection transaction actions={transactions:?}, read authorizations={}",
        observed
            .iter()
            .filter(|operation| matches!(operation, ObservedSql::Read { .. }))
            .count()
    );
    let committed = mutation
        .lock()
        .unwrap()
        .take()
        .expect("the actual supplied-connection audit read triggers the owned WAL writer")
        .expect("the writer commits while the supplied transaction retains its read snapshot");
    let after = Snapshot::take(&case.fixture.conn);
    assert_eq!(
        after.changes, before.changes,
        "the coordinator makes zero SQL mutations"
    );
    assert!(
        after.tables == committed.tables,
        "all rows match only the intentional committed association change"
    );
    assert_eq!(
        case.fixture
            .conn
            .query_row(
                "SELECT to_member_id FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        wrong_recipient
    );
    assert!(
        storage::get_permission_response(&case.fixture.conn, case.input_id).is_err(),
        "the new snapshot exposes the committed corruption"
    );
    assert_projection(&case, &success(result), Retained::Accepted);
    eprintln!(
        "H2 WAL mutation committed before audit preparation; original history accepted, new snapshot rejects association; caller total_changes={}",
        after.changes
    );
}
