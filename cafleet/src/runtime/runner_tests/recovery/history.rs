use super::*;

#[test]
fn runner_recovery_archives_each_original_and_synthesized_event_by_current_lifecycle() {
    for (lifecycle, reason) in [
        ("member", "member_inactive"),
        ("director", "director_inactive"),
        ("fleet", "fleet_deleted"),
    ] {
        let mut case = Case::new();
        case.seed(&[
            opening(&case.fixture.run, 1, 1),
            obscured(&case.fixture.run, 2, 1),
        ]);
        match lifecycle {
            "member" => {
                crate::broker::deregister_member(
                    &mut case.fixture.conn,
                    case.fixture.run.member_id,
                )
                .unwrap();
            }
            "director" => {
                case.fixture.conn.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",rusqlite::params![NOW,case.fixture.run.director_member_id]).unwrap();
            }
            "fleet" => {
                case.fixture
                    .conn
                    .execute(
                        "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                        rusqlite::params![NOW, case.fixture.run.fleet_id],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let report = case.recover(&mut new_host()).unwrap();
        assert_eq!(counts(&report), (2, 0, 4), "{lifecycle}");
        let events = events(&case);
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|row| row.event.notification_state
            == NotificationState::Archived
            && row.event.message_id.is_none()
            && row.event.archive_reason.unwrap().as_str() == reason));
        assert_eq!(
            case.fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE from_member_id=?1 AND to_member_id=?2",
                    rusqlite::params![
                        case.fixture.run.member_id,
                        case.fixture.run.director_member_id
                    ],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn runner_recovery_lifecycle_change_between_actual_commits_selects_each_disposition_atomically() {
    let mut case = Case::new();
    case.seed(&[
        opening(&case.fixture.run, 1, 1),
        obscured(&case.fixture.run, 2, 1),
    ]);
    let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
    let member = case.fixture.run.member_id;
    let mut changed = false;
    let mut host = new_host();
    host.action = Some(Box::new(move |boundary, _, _| {
        if boundary == "after_commit" && !changed {
            mutation.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",rusqlite::params![NOW,member]).unwrap();
            changed = true;
        }
        Ok(())
    }));
    let report = case.recover(&mut host).unwrap();
    assert_eq!(counts(&report), (2, 1, 3));
    let rows = events(&case);
    assert_eq!(rows[0].event.notification_state, NotificationState::Inbox);
    assert!(rows[0].event.message_id.is_some());
    assert!(rows[1..].iter().all(|row| row.event.notification_state
        == NotificationState::Archived
        && row.event.message_id.is_none()
        && row.event.archive_reason == Some(ArchiveReason::MemberInactive)));
}

#[test]
fn runner_recovery_preserves_committed_v1_bytes_ids_and_disposition_while_new_records_use_v2() {
    for archived in [false, true] {
        let mut case = Case::new();
        let original = opening(&case.fixture.run, 1, 1);
        let event = storage::persist_transition(
            &mut case.fixture.conn,
            &case.fixture.run.owner(),
            &original,
            PersistenceMode::Live,
        )
        .unwrap();
        let mut legacy = event.payload_json.clone();
        legacy["schema_version"] = json!(1);
        assert!(
            legacy
                .as_object_mut()
                .unwrap()
                .remove("director_action")
                .is_some()
        );
        let payload = legacy.to_string();
        case.fixture
            .conn
            .execute(
                "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
                rusqlite::params![payload, event.event_id],
            )
            .unwrap();
        case.fixture
            .conn
            .execute(
                "UPDATE messages SET text=?1 WHERE message_id=?2",
                rusqlite::params![payload, event.message_id.unwrap()],
            )
            .unwrap();
        let before = storage::get_event(&case.fixture.conn, event.event_id)
            .unwrap()
            .event;
        let message =
            crate::broker::get_message(&case.fixture.conn, before.message_id.unwrap()).unwrap();
        let second = obscured(&case.fixture.run, 2, 1);
        case.seed(&[original, second]);
        if archived {
            crate::broker::deregister_member(&mut case.fixture.conn, case.fixture.run.member_id)
                .unwrap();
        }
        let report = case.recover(&mut new_host()).unwrap();
        assert_eq!(
            counts(&report),
            if archived { (1, 0, 3) } else { (1, 3, 0) }
        );
        let rows = events(&case);
        assert_eq!(rows[0].event, before);
        assert_eq!(
            crate::broker::get_message(&case.fixture.conn, before.message_id.unwrap())
                .unwrap()
                .text,
            message.text
        );
        assert!(
            rows[1..]
                .iter()
                .all(|row| row.event.payload_json["schema_version"] == 2)
        );
    }
}

#[test]
fn runner_recovery_preserves_latest_evidence_receipts_response_audits_and_episode_claims() {
    use crate::coding_agent::{AnswerCapability, CodingAgent};
    let mut case = Case::new();
    case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET native_version='0.162.0',coverage='tested',profile_version='fixture-v1',profile_match='exact' WHERE run_id=?1",
            [case.fixture.run.run_id],
        )
        .unwrap();
    case.refresh();
    let owner = case.fixture.run.owner();
    let original = opening(&case.fixture.run, 1, 1);
    let event = storage::persist_transition(
        &mut case.fixture.conn,
        &owner,
        &original,
        PersistenceMode::Live,
    )
    .unwrap();
    let latest = evidence(&case.fixture.run, 77);
    storage::update_interaction_evidence(
        &mut case.fixture.conn,
        &owner,
        &InteractionEvidenceUpdate {
            run_id: owner.run_id,
            episode_seq: 1,
            output_seq: 77,
            evidence_json: latest.clone(),
        },
        1,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    let input = storage::reserve_input(
        &mut case.fixture.conn,
        &owner,
        &NewInput {
            operation_key: "prompt:retained-uncertainty".into(),
            kind: InputKind::Prompt,
            payload_digest: "d".repeat(64),
            message_id: None,
        },
    )
    .unwrap();
    assert_eq!(input.state, InputState::InProgress);
    storage::record_input_state(
        &mut case.fixture.conn,
        &owner,
        input.input_id,
        InputState::InProgress,
        InputState::Uncertain,
    )
    .unwrap();
    let capability = AnswerCapability::fixture(
        CodingAgent::Codex,
        case.fixture
            .run
            .native_version
            .as_deref()
            .expect("fixture run records its native version"),
        "answer-fixture-v1",
        "recovery-fixture",
        "single-request-choice",
        PermissionDecision::Allow,
        &["synthetic-storage-fixture".into()],
    )
    .unwrap();
    let request = NewPermissionResponse {
        operation_key: "permission:retained-claim".into(),
        metadata: PermissionResponseMetadata {
            event_id: event.event_id,
            interaction_id: event.interaction_id.unwrap(),
            expected_owner_epoch: owner.owner_epoch,
            request_token: "a".repeat(64),
            decision: PermissionDecision::Allow,
            director_member_id: case.fixture.run.director_member_id,
            director_mode: DirectorMode::Auto,
            decision_source: DecisionSource::DirectorAuto,
            mode_evidence_ref: "fixture:mode".into(),
            decision_evidence_ref: "fixture:decision".into(),
            answer_profile_version: "answer-fixture-v1".into(),
            answer_rule_id: "single-request-choice".into(),
        },
    };
    assert!(matches!(
        storage::reserve_permission_response(
            &mut case.fixture.conn,
            &owner,
            &request,
            Some(&capability),
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap(),
        PermissionReservation::Reserved(_)
    ));
    let snapshot = || {
        rows(&case.fixture.conn)
            .into_iter()
            .filter(|table| {
                table.name == "member_input_operations"
                    || table.name.starts_with("member_permission_")
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    assert!(before.iter().all(|table| !table.rows.is_empty()));
    case.recover(&mut new_host()).unwrap();
    let after = rows(&case.fixture.conn)
        .into_iter()
        .filter(|table| {
            table.name == "member_input_operations" || table.name.starts_with("member_permission_")
        })
        .collect::<Vec<_>>();
    assert_eq!(after, before);
    let interaction = events(&case)[0].interaction.clone().unwrap();
    assert_eq!(interaction.evidence_json, latest);
    assert_eq!(
        interaction.close_reason,
        Some(InteractionCloseReason::MonitorLost)
    );
    assert_eq!(
        storage::get_input(&case.fixture.conn, input.input_id)
            .unwrap()
            .state,
        InputState::Uncertain
    );
    assert_eq!(events(&case)[0].event.payload_json, event.payload_json);
}
