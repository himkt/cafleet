use super::*;

fn incomplete<T>(result: std::result::Result<T, InputFailure>) -> InputFailure {
    let failure = match result {
        Err(failure) => failure,
        Ok(_) => panic!("the staged live path reports its incomplete implementation"),
    };
    assert!(matches!(&failure.source, CafleetError::App(message)
        if message == "live permission coordination is not implemented"));
    assert_eq!(failure.source.exit_code(), 1);
    failure
}

fn response_context(case: &Case, failure: &InputFailure, key: &str, input_id: Option<i64>) {
    assert_eq!(failure.run_id, Some(case.owner.run_id));
    assert_eq!(
        failure.interaction_id,
        Some(case.request.metadata.interaction_id)
    );
    assert_eq!(failure.input_id, input_id);
    assert_eq!(failure.operation_key.as_deref(), Some(key));
}

fn ready() -> Case {
    let mut case = Case::new(Retained::InProgress);
    storage::record_input_state(
        &mut case.fixture.conn,
        &case.owner,
        case.input_id,
        InputState::InProgress,
        InputState::Ready,
    )
    .unwrap();
    let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
    assert_eq!(receipt.input.state, InputState::Ready);
    assert_eq!(receipt.response.metadata, case.request.metadata);
    assert!(receipt.claim.is_some());
    case
}

#[test]
fn step5_checkpoint_absent_response_key_has_zero_host_calls_and_reservations() {
    let mut case = Case::new(Retained::Accepted);
    let mut request = command(&case);
    request.operation_key = "fixture:fresh-staged-response".into();
    assert_eq!(
        case.fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
                rusqlite::params![case.owner.run_id, request.operation_key],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    let before = Snapshot::take(&case.fixture.conn);
    let failure = incomplete(attempt(&mut case, &request));
    response_context(&case, &failure, &request.operation_key, None);
    before.unchanged(&case.fixture.conn);
}

#[test]
fn step5_checkpoint_matching_ready_preserves_exact_old_metadata_and_claim() {
    let mut case = ready();
    let request = command(&case);
    let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
    let before = Snapshot::take(&case.fixture.conn);
    let failure = incomplete(attempt(&mut case, &request));
    response_context(&case, &failure, &request.operation_key, Some(case.input_id));
    before.unchanged(&case.fixture.conn);
    assert_eq!(
        storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
        receipt
    );
}

#[test]
fn step5_checkpoint_ready_conflict_precedes_incomplete_boundary_and_preserves_binding() {
    for field in ["epoch", "token", "decision", "mode_ref", "decision_ref"] {
        let mut case = ready();
        let mut request = command(&case);
        match field {
            "epoch" => request.expected_owner_epoch += 1,
            "token" => request.request_token = "b".repeat(64),
            "decision" => request.decision = PermissionDecision::Deny,
            "mode_ref" => request.mode_evidence_ref = "fixture:other-mode".into(),
            "decision_ref" => request.decision_evidence_ref = "fixture:other-decision".into(),
            _ => unreachable!(),
        }
        let before = Snapshot::take(&case.fixture.conn);
        let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
        let failure = match attempt(&mut case, &request) {
            Err(failure) => failure,
            Ok(_) => panic!("conflicting Ready {field} rejects the attempt"),
        };
        assert!(
            matches!(failure.source, CafleetError::InvalidStoredValue { .. }),
            "Ready {field} preserves the metadata-conflict category: {}",
            failure.source
        );
        response_context(&case, &failure, &request.operation_key, Some(case.input_id));
        before.unchanged(&case.fixture.conn);
        assert_eq!(
            storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
            receipt
        );
    }
}
