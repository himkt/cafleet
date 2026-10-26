use super::*;
use crate::runtime::runner::{
    InputCommandHost, InputControlReply, InputControlRequest, InputFailure, PermissionRespondInput,
    PermissionResponseResult, SubmissionStage, SubmissionStatus, respond_permission_with_host,
};

#[derive(Default)]
struct NoLive {
    calls: Vec<&'static str>,
}
impl NoLive {
    fn failure(&mut self, operation: &'static str, run: &RunRecord) -> InputFailure {
        self.calls.push(operation);
        InputFailure {
            source: CafleetError::App("history fixture reached a forbidden live operation".into()),
            run_id: Some(run.run_id),
            interaction_id: None,
            input_id: None,
            operation_key: None,
        }
    }
}
impl InputCommandHost for NoLive {
    fn lock(
        &mut self,
        _: &Settings,
        run: &RunRecord,
        _: &crate::broker::records::Placement,
        _: Deadline,
    ) -> std::result::Result<Option<PaneLock>, InputFailure> {
        Err(self.failure("lock", run))
    }
    fn health(
        &mut self,
        run: &RunRecord,
        _: Instant,
    ) -> std::result::Result<HealthSnapshot, InputFailure> {
        Err(self.failure("health", run))
    }
    fn exchange(
        &mut self,
        run: &RunRecord,
        _: &InputControlRequest,
        _: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        Err(self.failure("exchange", run))
    }
}
fn command(case: &Case) -> PermissionRespondInput {
    let metadata = &case.request.metadata;
    PermissionRespondInput {
        interaction_id: metadata.interaction_id,
        event_id: metadata.event_id,
        director_member_id: metadata.director_member_id,
        expected_run_id: case.owner.run_id,
        expected_owner_epoch: metadata.expected_owner_epoch,
        request_token: metadata.request_token.clone(),
        decision: metadata.decision,
        decision_source: metadata.decision_source,
        director_mode: metadata.director_mode,
        mode_evidence_ref: metadata.mode_evidence_ref.clone(),
        decision_evidence_ref: metadata.decision_evidence_ref.clone(),
        operation_key: case.request.operation_key.clone(),
    }
}
fn invoke(
    conn: &mut Connection,
    settings: &Settings,
    request: &PermissionRespondInput,
    deadline: Deadline,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    let mut host = NoLive::default();
    let result = with_database_budget(conn, deadline, |conn| {
        Ok(respond_permission_with_host(
            conn, settings, request, deadline, &mut host,
        ))
    })
    .unwrap();
    assert!(
        host.calls.is_empty(),
        "historical coordination reached live operations: {:?}",
        host.calls
    );
    result
}
fn attempt(
    case: &mut Case,
    request: &PermissionRespondInput,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    invoke(
        &mut case.fixture.conn,
        &case.settings,
        request,
        Deadline::until(Instant::now() + Duration::from_secs(1)),
    )
}
fn success(
    result: std::result::Result<PermissionResponseResult, InputFailure>,
) -> PermissionResponseResult {
    match result {
        Ok(result) => result,
        Err(error) => panic!("valid retained response returns history: {}", error.source),
    }
}
fn assert_projection(case: &Case, result: &PermissionResponseResult, state: Retained) {
    assert_eq!(result.submission.input_id, Some(case.input_id));
    assert_eq!(result.member_id, case.fixture.run.member_id);
    assert_eq!(result.run_id, case.owner.run_id);
    assert_eq!(
        result.owner_epoch,
        case.request.metadata.expected_owner_epoch
    );
    assert_eq!(result.interaction_id, case.request.metadata.interaction_id);
    assert_eq!(result.event_id, case.event.event_id);
    assert_eq!(result.decision, case.request.metadata.decision);
    assert_eq!(
        result.decision_source,
        case.request.metadata.decision_source
    );
    assert_eq!(result.submission.output_seq, None);
    match state {
        Retained::Accepted | Retained::Submitted => {
            assert!(matches!(
                result.submission.status,
                SubmissionStatus::Accepted
            ));
            assert!(matches!(
                result.submission.stage,
                SubmissionStage::Submitted
            ));
        }
        Retained::InProgress | Retained::Uncertain => {
            assert!(matches!(
                result.submission.status,
                SubmissionStatus::Uncertain
            ));
            assert!(matches!(result.submission.stage, SubmissionStage::Unknown));
        }
        Retained::NotSubmitted | Retained::Abandoned => {
            assert!(matches!(
                result.submission.status,
                SubmissionStatus::Rejected
            ));
            assert!(matches!(
                result.submission.stage,
                SubmissionStage::ZeroBytes
            ));
            assert!(
                result
                    .submission
                    .reason
                    .as_ref()
                    .is_some_and(|reason| reason.to_ascii_lowercase().contains("consum"))
            );
        }
    }
}
fn projection(state: Retained) {
    let mut case = Case::new(state);
    let before = Snapshot::take(&case.fixture.conn);
    let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
    let request = command(&case);
    let result = success(attempt(&mut case, &request));
    assert_projection(&case, &result, state);
    before.unchanged(&case.fixture.conn);
    assert_eq!(
        storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
        receipt
    );
}
#[test]
fn step5_a1_retained_accepted_preserves_original_binding_without_live_calls() {
    projection(Retained::Accepted);
}
#[test]
fn step5_a1_retained_inprogress_preserves_unknown_stage_and_claim_without_live_calls() {
    projection(Retained::InProgress);
}
#[test]
fn step5_a1_retained_uncertain_preserves_unknown_stage_and_claim_without_live_calls() {
    projection(Retained::Uncertain);
}
#[test]
fn step5_a1_settled_submitted_remains_accepted_without_live_calls() {
    projection(Retained::Submitted);
}
#[test]
fn step5_a1_consumed_not_submitted_rejects_this_retry_without_changing_claim() {
    projection(Retained::NotSubmitted);
}
#[test]
fn step5_a1_consumed_abandoned_uncertain_preserves_possible_transmission_claim() {
    projection(Retained::Abandoned);
}

#[test]
fn step5_a1_history_preserves_both_decisions_and_every_valid_attributed_source_mode() {
    for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
        for (mode, source) in [
            (DirectorMode::Auto, DecisionSource::DirectorAuto),
            (DirectorMode::Auto, DecisionSource::User),
            (DirectorMode::Manual, DecisionSource::User),
            (DirectorMode::Unknown, DecisionSource::User),
        ] {
            let mut case = Case::decision(Retained::Accepted, decision, mode, source);
            let before = Snapshot::take(&case.fixture.conn);
            let request = command(&case);
            let result = success(attempt(&mut case, &request));
            assert_projection(&case, &result, Retained::Accepted);
            before.unchanged(&case.fixture.conn);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum HistoryChange {
    Closed,
    MemberInactive,
    DirectorInactive,
    FleetDeleted,
    OwnerFenced,
}
fn changed_history(change: HistoryChange) {
    let mut case = Case::new(Retained::Accepted);
    let run = &case.fixture.run;
    match change {
        HistoryChange::Closed => {
            assert_eq!(case.fixture.conn.execute(
            "UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",
            rusqlite::params![NOW,case.request.metadata.interaction_id]).unwrap(),1);
        }
        HistoryChange::MemberInactive | HistoryChange::DirectorInactive => {
            assert_eq!(case.fixture.conn.execute(
            "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
            rusqlite::params![NOW,if matches!(change,HistoryChange::MemberInactive){run.member_id}else{run.director_member_id}]).unwrap(),1);
        }
        HistoryChange::FleetDeleted => {
            assert_eq!(
                case.fixture
                    .conn
                    .execute(
                        "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                        rusqlite::params![NOW, run.fleet_id]
                    )
                    .unwrap(),
                1
            );
        }
        HistoryChange::OwnerFenced => {
            assert_eq!(case.fixture.conn.execute(
            "UPDATE member_runs SET status='lost',owner_kind='reconciler',owner_epoch=owner_epoch+1,ended_at=?1 WHERE run_id=?2",
            rusqlite::params![NOW,run.run_id]).unwrap(),1);
        }
    }
    let request = command(&case);
    let before = Snapshot::take(&case.fixture.conn);
    let result = success(attempt(&mut case, &request));
    assert_projection(&case, &result, Retained::Accepted);
    before.unchanged(&case.fixture.conn);
    if matches!(change, HistoryChange::OwnerFenced) {
        assert_eq!(
            storage::get_run(&case.fixture.conn, case.owner.run_id)
                .unwrap()
                .owner_epoch,
            result.owner_epoch + 1
        );
    }
}
#[test]
fn step5_a1_history_survives_closed_interaction_without_request_reinspection() {
    changed_history(HistoryChange::Closed);
}
#[test]
fn step5_a1_history_survives_member_deactivation_without_active_preflight() {
    changed_history(HistoryChange::MemberInactive);
}
#[test]
fn step5_a1_history_survives_director_deactivation_without_active_preflight() {
    changed_history(HistoryChange::DirectorInactive);
}
#[test]
fn step5_a1_history_survives_deleted_fleet_without_live_preflight() {
    changed_history(HistoryChange::FleetDeleted);
}
#[test]
fn step5_a1_history_reports_audit_epoch_after_terminal_owner_fencing() {
    changed_history(HistoryChange::OwnerFenced);
}

#[test]
fn step5_a1_retained_result_precedes_the_actual_held_pane_lock() {
    let mut case = Case::new(Retained::Accepted);
    let guard = PaneLock::acquire_within(
        &case.settings.database_url,
        &case.placement,
        Deadline::until(Instant::now() + Duration::from_secs(1)),
    )
    .unwrap()
    .unwrap();
    let request = command(&case);
    let before = Snapshot::take(&case.fixture.conn);
    let result = success(attempt(&mut case, &request));
    assert_projection(&case, &result, Retained::Accepted);
    before.unchanged(&case.fixture.conn);
    assert!(
        PaneLock::acquire_within(
            &case.settings.database_url,
            &case.placement,
            Deadline::until(Instant::now() + Duration::from_millis(20))
        )
        .unwrap()
        .is_none()
    );
    drop(guard);
}
#[test]
fn step5_a1_reopened_connection_retries_settled_history_without_mutation() {
    let mut case = Case::new(Retained::Submitted);
    let request = command(&case);
    let before = Snapshot::take(&case.fixture.conn);
    success(attempt(&mut case, &request));
    before.unchanged(&case.fixture.conn);
    let mut reopened = crate::db::connect(&case.settings.database_url).unwrap();
    let second = Snapshot::take(&reopened);
    let result = success(invoke(
        &mut reopened,
        &case.settings,
        &request,
        Deadline::until(Instant::now() + Duration::from_secs(1)),
    ));
    assert_projection(&case, &result, Retained::Submitted);
    second.unchanged(&reopened);
}
#[test]
fn step5_a1_retained_lookup_reads_while_another_connection_holds_sqlite_write_ownership() {
    let mut case = Case::new(Retained::Accepted);
    let writer = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = Snapshot::take(&case.fixture.conn);
    let request = command(&case);
    let result = invoke(
        &mut case.fixture.conn,
        &case.settings,
        &request,
        Deadline::until(Instant::now() + Duration::from_millis(250)),
    );
    let held = !writer.is_autocommit();
    writer.execute_batch("ROLLBACK").unwrap();
    assert!(held);
    assert_projection(&case, &success(result), Retained::Accepted);
    before.unchanged(&case.fixture.conn);
}
#[test]
fn step5_a1_retained_lookup_precedes_actual_unresolved_capacity_refusal() {
    let mut case = Case::new(Retained::Accepted);
    assert_eq!(case.fixture.conn.execute(
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) SELECT ?1,'fixture:capacity:'||x,'prompt',?2,'in_progress',?3,?3 FROM n",
        rusqlite::params![case.owner.run_id,"b".repeat(64),NOW]).unwrap(),4095);
    assert_eq!(case.fixture.conn.query_row("SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND state IN ('in_progress','accepted','uncertain')",[case.owner.run_id],|row|row.get::<_,i64>(0)).unwrap(),4096);
    let before = Snapshot::take(&case.fixture.conn);
    let request = command(&case);
    let result = success(attempt(&mut case, &request));
    assert_projection(&case, &result, Retained::Accepted);
    before.unchanged(&case.fixture.conn);
}

#[test]
fn step5_a1_each_conflicting_caller_binding_fails_before_live_calls_with_exact_rows() {
    for field in [
        "interaction",
        "event",
        "director",
        "run",
        "epoch",
        "token",
        "decision",
        "source",
        "mode",
        "mode_ref",
        "decision_ref",
    ] {
        let mut case = Case::decision(
            Retained::Accepted,
            PermissionDecision::Allow,
            DirectorMode::Auto,
            DecisionSource::User,
        );
        let mut request = command(&case);
        match field {
            "interaction" => request.interaction_id += 1,
            "event" => request.event_id += 1,
            "director" => request.director_member_id = case.fixture.run.member_id,
            "run" => request.expected_run_id += 1,
            "epoch" => request.expected_owner_epoch += 1,
            "token" => request.request_token = "b".repeat(64),
            "decision" => request.decision = PermissionDecision::Deny,
            "source" => request.decision_source = DecisionSource::DirectorAuto,
            "mode" => request.director_mode = DirectorMode::Manual,
            "mode_ref" => request.mode_evidence_ref = "fixture:other-mode".into(),
            "decision_ref" => request.decision_evidence_ref = "fixture:other-decision".into(),
            _ => unreachable!(),
        }
        let before = Snapshot::take(&case.fixture.conn);
        assert!(attempt(&mut case, &request).is_err(), "{field}");
        before.unchanged(&case.fixture.conn);
    }
}
#[test]
fn step5_a1_retained_ordinary_input_kind_cannot_impersonate_a_permission_response() {
    let mut case = Case::new(Retained::Accepted);
    storage::reserve_input(
        &mut case.fixture.conn,
        &case.owner,
        &NewInput {
            operation_key: "fixture:ordinary-input".into(),
            kind: InputKind::Prompt,
            payload_digest: "c".repeat(64),
            message_id: None,
        },
    )
    .unwrap();
    let mut request = command(&case);
    request.operation_key = "fixture:ordinary-input".into();
    let before = Snapshot::take(&case.fixture.conn);
    assert!(attempt(&mut case, &request).is_err());
    before.unchanged(&case.fixture.conn);
}
#[test]
fn step5_a1_historical_message_association_is_checked_despite_matching_canonical_digest() {
    let mut case = Case::new(Retained::Accepted);
    assert_eq!(
        storage::permission_response_digest(case.owner.run_id, &case.request.metadata).unwrap(),
        storage::get_input(&case.fixture.conn, case.input_id)
            .unwrap()
            .payload_digest
    );
    assert_eq!(
        case.fixture
            .conn
            .execute(
                "UPDATE messages SET to_member_id=?1 WHERE message_id=?2",
                rusqlite::params![case.fixture.run.member_id, case.event.message_id.unwrap()]
            )
            .unwrap(),
        1
    );
    let request = command(&case);
    let source = stored_corruption(&case);
    let before = Snapshot::take(&case.fixture.conn);
    let result = attempt(&mut case, &request);
    known_corruption(&case, result, &source);
    before.unchanged(&case.fixture.conn);
}

fn stored_corruption(case: &Case) -> CafleetError {
    match storage::get_permission_response(&case.fixture.conn, case.input_id) {
        Err(error) => error,
        Ok(_) => panic!("the fixture exposes actual durable association corruption"),
    }
}

fn known_corruption(
    case: &Case,
    result: std::result::Result<PermissionResponseResult, InputFailure>,
    source: &CafleetError,
) {
    let failure = match result {
        Err(failure) => failure,
        Ok(_) => panic!("corrupt history rejects before live effects"),
    };
    assert_eq!(failure.run_id, Some(case.owner.run_id));
    assert_eq!(
        failure.interaction_id,
        Some(case.request.metadata.interaction_id)
    );
    assert_eq!(failure.input_id, Some(case.input_id));
    assert_eq!(
        failure.operation_key.as_deref(),
        Some(case.request.operation_key.as_str())
    );
    assert_eq!(
        std::mem::discriminant(&failure.source),
        std::mem::discriminant(source)
    );
    assert_eq!(failure.source.exit_code(), source.exit_code());
    assert_eq!(failure.source.message(), source.message());
}
#[test]
fn step5_a1_history_rejects_missing_audit_or_possibly_transmitted_claim_without_live_fallback() {
    for remove_audit in [false, true] {
        let mut case = Case::new(Retained::Accepted);
        assert_eq!(
            case.fixture
                .conn
                .execute(
                    "DELETE FROM member_permission_response_claims WHERE input_id=?1",
                    [case.input_id]
                )
                .unwrap(),
            1
        );
        if remove_audit {
            assert_eq!(
                case.fixture
                    .conn
                    .execute(
                        "DELETE FROM member_permission_responses WHERE input_id=?1",
                        [case.input_id]
                    )
                    .unwrap(),
                1
            );
        }
        let request = command(&case);
        let source = stored_corruption(&case);
        let before = Snapshot::take(&case.fixture.conn);
        let result = attempt(&mut case, &request);
        known_corruption(&case, result, &source);
        before.unchanged(&case.fixture.conn);
    }
}
#[test]
fn step5_a1_history_validates_actual_member_fleet_and_director_relationships() {
    for changed in ["member", "director", "fleet"] {
        let mut case = Case::new(Retained::Accepted);
        if changed == "fleet" {
            let alternate = crate::broker::register_member(
                &mut case.fixture.conn,
                case.fixture.run.fleet_id,
                "alternative",
                "fixture",
                None,
            )
            .unwrap()
            .member_id;
            assert_eq!(
                case.fixture
                    .conn
                    .execute(
                        "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
                        rusqlite::params![alternate, case.fixture.run.fleet_id]
                    )
                    .unwrap(),
                1
            );
        } else {
            let other = common::create_fleet(&mut case.fixture.conn, "other-history-fleet").0;
            let member = if changed == "member" {
                case.fixture.run.member_id
            } else {
                case.fixture.run.director_member_id
            };
            assert_eq!(
                case.fixture
                    .conn
                    .execute(
                        "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                        rusqlite::params![other, member]
                    )
                    .unwrap(),
                1
            );
        }
        let request = command(&case);
        let before = Snapshot::take(&case.fixture.conn);
        assert!(attempt(&mut case, &request).is_err(), "{changed}");
        before.unchanged(&case.fixture.conn);
    }
}
#[test]
fn step5_a1_copied_database_cannot_reuse_the_original_retained_identity() {
    let case = Case::new(Retained::Accepted);
    let copied = case.fixture._dir.path().join("copied-history.db");
    std::fs::copy(case.fixture.conn.path().unwrap(), &copied).unwrap();
    let settings = Settings {
        database_url: format!("sqlite:///{}", copied.display()),
        max_text_len: 200,
        multiplexer: None,
    };
    let mut conn = crate::db::connect(&settings.database_url).unwrap();
    let before = Snapshot::take(&conn);
    assert!(
        invoke(
            &mut conn,
            &settings,
            &command(&case),
            Deadline::until(Instant::now() + Duration::from_secs(1))
        )
        .is_err()
    );
    before.unchanged(&conn);
}
#[test]
fn step5_a1_semantic_source_mode_preflight_preserves_usage_category_before_history_or_live_effects()
{
    for mode in [DirectorMode::Manual, DirectorMode::Unknown] {
        let mut case = Case::new(Retained::Accepted);
        let mut request = command(&case);
        request.director_mode = mode;
        let before = Snapshot::take(&case.fixture.conn);
        let failure = match attempt(&mut case, &request) {
            Err(failure) => failure,
            Ok(_) => panic!("director-auto requires auto before retained projection"),
        };
        assert!(matches!(failure.source, CafleetError::Usage(_)));
        before.unchanged(&case.fixture.conn);
    }
}

fn causes() -> Vec<CafleetError> {
    vec![
        CafleetError::Usage("fixture usage".into()),
        CafleetError::App("fixture application".into()),
        CafleetError::Value("fixture value".into()),
        CafleetError::DatabaseBusy("fixture contention".into()),
        CafleetError::DeadlineExceeded,
        CafleetError::InvalidStoredValue {
            field: "fixture.field".into(),
            value: "invalid".into(),
        },
        CafleetError::WithDiagnostics {
            cause: Box::new(CafleetError::Usage("nested usage".into())),
            diagnostics: vec!["original diagnostic".into()],
        },
    ]
}
#[test]
fn step5_a1_input_failure_without_context_preserves_the_complete_source_category() {
    for cause in causes() {
        let failure = InputFailure {
            source: cause.clone(),
            run_id: None,
            interaction_id: None,
            input_id: None,
            operation_key: None,
        };
        let converted: CafleetError = failure.into();
        assert_eq!(
            std::mem::discriminant(&converted),
            std::mem::discriminant(&cause)
        );
        assert_eq!(converted.exit_code(), cause.exit_code());
        assert_eq!(converted.message(), cause.message());
    }
}
#[test]
fn step5_a1_input_failure_context_preserves_cause_and_escapes_ordered_key_metadata() {
    let key = "fixture:key\nwith\"quotes\"\\tail";
    for original in causes() {
        let failure = InputFailure {
            source: original.clone(),
            run_id: Some(29),
            interaction_id: Some(17),
            input_id: Some(31),
            operation_key: Some(key.into()),
        };
        let converted: CafleetError = failure.into();
        assert_eq!(converted.exit_code(), original.exit_code());
        let CafleetError::WithDiagnostics { cause, diagnostics } = converted else {
            panic!("known identities retain the source with context")
        };
        assert_eq!(
            std::mem::discriminant(cause.as_ref()),
            std::mem::discriminant(&original)
        );
        assert_eq!(cause.message(), original.message());
        assert_eq!(diagnostics.len(), 1);
        let context = &diagnostics[0];
        let positions = ["run_id", "interaction_id", "input_id", "operation_key"]
            .map(|name| context.find(name).expect("known context field"));
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(context.contains(&serde_json::to_string(key).unwrap()));
        assert!(!context.contains('\n'));
    }
}
#[test]
fn step5_a1_input_failure_context_omits_unknown_optional_identities() {
    let failure = InputFailure {
        source: CafleetError::DeadlineExceeded,
        run_id: None,
        interaction_id: Some(17),
        input_id: None,
        operation_key: None,
    };
    let converted: CafleetError = failure.into();
    let CafleetError::WithDiagnostics { cause, diagnostics } = converted else {
        panic!("one known identity is represented explicitly")
    };
    assert!(matches!(*cause, CafleetError::DeadlineExceeded));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("interaction_id"));
    for absent in ["run_id", "input_id", "operation_key"] {
        assert!(!diagnostics[0].contains(absent));
    }
}
