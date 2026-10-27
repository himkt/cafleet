use super::{
    ControlIdentity, DrainedInput, InputControlOperation, InputControlReply, InputControlRequest,
    InputDrain, InputEligibility, ObservationController, ProgrammaticWrite, PtyIo,
    SubmissionResult, SubmissionStatus, Visibility, canonical_json, validate_operation_key,
};
use super::{
    InputFailure, ProcessIdentity, Result, RunLock, SubmissionStage, Transport, check_deadline,
    ensure, error, stable_run_identity, with_database_deadline,
};
use super::{
    MissingEvidence, PermissionInspectInput, PermissionInspection, PermissionResponseResult,
};
#[cfg(test)]
use super::{RealInputDeadlineClock, receive_input_expiration};
use crate::broker::members::db_err;
use crate::broker::records::{MemberKind, Placement};
use crate::broker::runtime::{self as storage, OwnerKind, RunRecord, RunStatus};
use crate::coding_agent::{
    AnswerMatch, AnswerPlan, AnswerStage, ProfileCatalog, SemanticFingerprint,
};
use crate::coding_agent::{ComposerOperation, coding_agent};
use crate::error::CafleetError;
use crate::input::{Deadline, inherited_database_deadline, with_database_budget};
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod permission_inspection_contracts {
    use super::contract_tests::ordinary_prompt::support::{
        Host, catalog_definition, with_catalog_host,
    };
    use super::*;
    use crate::broker::runtime::{Coverage, InteractionClass, PermissionDecision};
    use crate::coding_agent::{inspection_fixture_definition, inspection_fixture_form};
    use crate::runtime::runner::{
        MissingEvidence, PermissionInspectInput, PermissionInspection, RuntimePaths,
    };
    use rusqlite::OptionalExtension;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    struct Tokens {
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }
    impl RequestTokenSource for Tokens {
        fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail.load(Ordering::SeqCst) {
                return Err(error("owned inspection entropy failure"));
            }
            bytes.fill(u8::try_from(call).expect("bounded fixture token calls"));
            Ok(())
        }
    }

    pub(super) fn with_inspection(body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>)) {
        with_definition(
            inspection_fixture_definition(),
            b"unused-program~".to_vec(),
            body,
        );
    }
    pub(super) fn with_definition(
        mut definition: serde_json::Value,
        expected_program: Vec<u8>,
        body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
    ) {
        let composer = catalog_definition();
        definition["fixtures"]
            .as_array_mut()
            .unwrap()
            .push(composer["fixtures"][1].clone());
        definition["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(composer["profiles"][0]["rules"][0].clone());
        definition["profiles"][0]["capabilities"]["composer"] =
            composer["profiles"][0]["capabilities"]["composer"].clone();
        with_catalog_host(
            ProfileCatalog::from_definitions(&definition).unwrap(),
            expected_program,
            |h| {
                let calls = Arc::new(AtomicUsize::new(0));
                let fail = Arc::new(AtomicBool::new(false));
                h.coordinator.services.token_source = Box::new(Tokens {
                    calls: calls.clone(),
                    fail: fail.clone(),
                });
                body(h, calls, fail);
            },
        );
    }

    pub(super) fn form(h: &mut Host, operation: &str, focus: &str) {
        h.pty
            .child
            .bytes
            .extend(inspection_fixture_form(operation, focus));
        h.drain();
    }

    fn current_ids(h: &Host) -> Option<(i64, i64, i64, i64)> {
        h.fixture.conn.query_row(
            "SELECT e.event_id,e.interaction_id,e.message_id,i.episode_seq FROM member_runtime_events e JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 AND e.kind='interaction_opened' AND i.closed_at IS NULL ORDER BY e.event_id DESC LIMIT 1",
            [h.fixture.run.run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).optional().unwrap()
    }

    pub(super) fn stable_form(h: &mut Host) -> (i64, i64, i64, i64) {
        form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
        loop {
            h.drain();
            if h.controller.current_request().is_some()
                && let Some(ids) = current_ids(h)
            {
                return ids;
            }
            assert!(
                Instant::now() < h.until,
                "stable actual request within original fixture deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn request(h: &Host, ids: (i64, i64, i64, i64)) -> InputControlRequest {
        let mut request = h.request("unused-inspection-key");
        request.operation = InputControlOperation::PermissionInspect(PermissionInspectInput {
            event_id: ids.0,
            interaction_id: ids.1,
            director_member_id: h.fixture.run.director_member_id,
        });
        request
    }

    fn inspect_request(
        h: &mut Host,
        request: InputControlRequest,
        until: Instant,
    ) -> std::result::Result<PermissionInspection, InputFailure> {
        h.coordinator
            .inspect_received(
                &mut h.fixture.conn,
                &h.fixture.run,
                request,
                (&h.controller, &mut h.transport),
                until,
            )
            .map(|reply| {
                let InputControlReply::Inspection(value) = reply else {
                    panic!("actual inspection typed reply")
                };
                *value
            })
    }
    pub(super) fn inspect(h: &mut Host, ids: (i64, i64, i64, i64)) -> PermissionInspection {
        h.drain();
        inspect_request(h, request(h, ids), h.until).unwrap()
    }
    fn token(value: &PermissionInspection) -> String {
        let token = value
            .request_token
            .as_ref()
            .expect("current matched request token");
        assert_eq!(token.len(), 64);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        token.clone()
    }
    pub(super) fn counts(h: &Host) -> [i64; 3] {
        [
            "member_input_operations",
            "member_permission_responses",
            "member_permission_response_claims",
        ]
        .map(|table| {
            h.fixture
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
    }
    fn pending(h: &Host) -> (String, String, Vec<u8>, Instant, InputPhase, Option<i64>) {
        let p = h
            .coordinator
            .pending
            .as_ref()
            .expect("genuine Armed writer");
        (
            p.request.operation_key.clone(),
            p.request.payload_digest.clone(),
            p.bytes.to_vec(),
            p.deadline,
            p.phase,
            p.input_id,
        )
    }

    #[test]
    fn permission_inspection_matches_durable_current_request_and_preserves_the_armed_writer() {
        with_inspection(|h, calls, _| {
            let input_id = h.arm("prompt:inspection-pending");
            let ids = stable_form(h);
            let pending = pending(h);
            let row = storage::get_input(&h.fixture.conn, input_id).unwrap();
            let retained = h.coordinator.retained_checkpoint(input_id).unwrap();
            let before = counts(h);
            let hooks = h.hooks.points.lock().unwrap().len();
            let value = inspect(h, ids);
            assert_eq!(
                (
                    value.event_id,
                    value.interaction_id,
                    value.message_id,
                    value.episode_seq
                ),
                ids
            );
            let run = &h.fixture.run;
            assert_eq!(
                (
                    value.run_id,
                    value.owner_epoch,
                    value.fleet_id,
                    value.member_id,
                    value.director_member_id
                ),
                (
                    run.run_id,
                    run.owner_epoch,
                    run.fleet_id,
                    run.member_id,
                    run.director_member_id
                )
            );
            assert_eq!(
                value.classification,
                Some(InteractionClass::PermissionRequest)
            );
            assert_eq!(value.visibility, Visibility::Visible);
            assert_eq!(value.answer_capability, Coverage::Tested);
            assert_eq!(
                value.answer_profile_version.as_deref(),
                Some("synthetic-profile-1")
            );
            assert!(
                value.answer_rule_id.is_none(),
                "distinct Allow/Deny rules have no unique common rule"
            );
            assert_eq!(
                value.supported_decisions,
                vec![PermissionDecision::Allow, PermissionDecision::Deny]
            );
            assert!(value.capability_reason.is_none());
            assert_eq!(
                value.evidence,
                h.controller.current_evidence().unwrap().clone()
            );
            assert!(value.evidence["operation"].is_null());
            assert_eq!(
                value
                    .missing_evidence
                    .iter()
                    .map(MissingEvidence::as_str)
                    .collect::<Vec<_>>(),
                [
                    "operation",
                    "arguments",
                    "cwd",
                    "environment",
                    "targets",
                    "effects",
                    "choice_scope"
                ]
            );
            let first = token(&value);
            let no_drain = inspect_request(h, request(h, ids), h.until).unwrap();
            assert!(no_drain.request_token.is_none());
            assert!(no_drain.supported_decisions.is_empty());
            assert_eq!(no_drain.answer_capability, Coverage::Unknown);
            assert_eq!(token(&inspect(h, ids)), first);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(pending == self::pending(h));
            assert_eq!(storage::get_input(&h.fixture.conn, input_id).unwrap(), row);
            let after = h.coordinator.retained_checkpoint(input_id).unwrap();
            assert_eq!(
                (after.stage, after.written_bytes, after.output_seq),
                (retained.stage, retained.written_bytes, retained.output_seq)
            );
            assert_eq!(counts(h), before);
            assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
            assert!(h.pty.offers.is_empty());
            let (evidence,body,notified):(String,String,Option<String>) = h.fixture.conn.query_row("SELECT i.evidence_json,m.text,m.notified_at FROM member_interactions i JOIN messages m ON m.message_id=?1 WHERE i.interaction_id=?2",rusqlite::params![ids.2,ids.1],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            for stored in [evidence, body] {
                assert!(!stored.contains(&first));
                assert!(!stored.contains("PRIVATE_INSPECTION_OPERATION"));
            }
            let paths =
                RuntimePaths::for_run(&h.fixture.run.database_identity_json, h.fixture.run.run_id)
                    .unwrap();
            let outbox = std::fs::read(paths.outbox).unwrap();
            for secret in [first.as_bytes(), b"PRIVATE_INSPECTION_OPERATION".as_slice()] {
                assert!(!outbox.windows(secret.len()).any(|window| window == secret));
            }
            assert!(notified.is_none());
        });
    }

    #[test]
    fn permission_inspection_token_binds_focus_plan_set_and_intermediate_invalidations() {
        with_inspection(|h, calls, _| {
            let ids = stable_form(h);
            let original = inspect(h, ids);
            let mut previous = token(&original);
            let revision = h.controller.request_generation();
            h.pty.child.bytes.extend(b"\x1b[?25l\x1b[?25h");
            h.drain();
            let redraw = inspect(h, ids);
            assert!(redraw.output_seq > original.output_seq);
            assert_eq!(h.controller.request_generation(), revision);
            assert_eq!(token(&redraw), previous);
            h.transport
                .resize(
                    &mut h.pty,
                    super::super::TerminalSize { rows: 24, cols: 81 },
                )
                .unwrap();
            assert_eq!(token(&inspect(h, ids)), previous);
            h.coordinator.observe_manual(0).unwrap();
            assert_eq!(token(&inspect(h, ids)), previous);
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
            let rebuilt = token(&inspect(h, ids));
            assert_ne!(
                rebuilt, previous,
                "clear/rebuild traverses incompatible observed screens"
            );
            previous = rebuilt;
            h.human.bytes.push_back(b'!');
            h.drain();
            assert_eq!(h.pty.manual, b"!");
            let manual = token(&inspect(h, ids));
            assert_ne!(manual, previous);
            previous = manual;
            let revision = h.controller.request_generation();
            form(h, "PRIVATE_INSPECTION_OPERATION", "missing-focus");
            assert!(h.controller.current_request().is_none());
            assert!(h.controller.request_generation() > revision);
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
            let returned = token(&inspect(h, ids));
            assert_ne!(
                returned, previous,
                "A-invalid-A cannot resurrect a token without an intervening inspect"
            );
            previous = returned;
            assert_eq!(
                current_ids(h),
                Some(ids),
                "invalidation precedes durable obscurity debounce"
            );
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other");
            let focused = token(&inspect(h, ids));
            assert_ne!(focused, previous);
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt");
            let alternate = inspect(h, ids);
            assert_ne!(token(&alternate), focused);
            assert_eq!(
                alternate.supported_decisions,
                vec![PermissionDecision::Allow]
            );
            assert_eq!(
                alternate.answer_rule_id.as_deref(),
                Some("inspection-alternate-allow")
            );
            assert_eq!(
                current_ids(h),
                Some(ids),
                "plan/focus changes are independent of semantic episode identity"
            );
            form(h, "different semantic request", "synthetic-only");
            assert!(h.controller.current_request().is_none());
            let candidate = inspect_request(h, request(h, ids), h.until).unwrap();
            assert!(candidate.request_token.is_none());
            assert!(candidate.supported_decisions.is_empty());
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt");
            assert_ne!(token(&inspect(h, ids)), token(&alternate));
            assert_eq!(calls.load(Ordering::SeqCst), 7);
            assert!(h.pty.program.is_empty());
        });
    }

    #[test]
    fn permission_inspection_refuses_stale_associations_identity_and_expired_budget_without_effects()
     {
        with_inspection(|h, calls, _| {
            let ids = stable_form(h);
            let original = token(&inspect(h, ids));
            let before = counts(h);
            h.drain();
            let mut wrong = request(h, ids);
            wrong.identity.launch_nonce = "f".repeat(64);
            assert!(inspect_request(h, wrong, h.until).is_err());
            h.drain();
            assert!(inspect_request(h, request(h, ids), Instant::now()).is_err());
            h.fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",rusqlite::params![crate::time::format_utc(crate::time::now_utc()),ids.1]).unwrap();
            h.drain();
            assert!(inspect_request(h, request(h, ids), h.until).is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(counts(h), before);
            assert!(h.pty.offers.is_empty());
            assert!(!original.is_empty());
        });
    }

    #[test]
    fn permission_inspection_entropy_failure_and_manual_overflow_supply_no_fallback_authority() {
        with_inspection(|h, calls, fail| {
            let ids = stable_form(h);
            fail.store(true, Ordering::SeqCst);
            h.drain();
            let failure = match inspect_request(h, request(h, ids), h.until) {
                Err(e) => e,
                Ok(_) => panic!("entropy failure is explicit"),
            };
            assert!(
                failure
                    .source
                    .message()
                    .contains("owned inspection entropy failure")
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            fail.store(false, Ordering::SeqCst);
            let value = inspect(h, ids);
            assert_eq!(token(&value), "02".repeat(32));
            h.coordinator.manual_generation = u64::MAX;
            assert!(h.coordinator.observe_manual(1).is_err());
            assert!(h.coordinator.manual_accounting_failed);
            h.drain();
            let result = inspect_request(h, request(h, ids), h.until);
            assert!(result.is_err() || result.is_ok_and(|v| v.request_token.is_none()));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert_eq!(counts(h), [0, 0, 0]);
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn permission_inspection_owner_loss_and_shutdown_revoke_the_existing_token() {
        with_inspection(|h, calls, _| {
            let ids = stable_form(h);
            let original = token(&inspect(h, ids));
            let paths =
                RuntimePaths::for_run(&h.fixture.run.database_identity_json, h.fixture.run.run_id)
                    .unwrap();
            let backup = paths.owner_lock.with_extension("inspection-owned-backup");
            h.drain();
            std::fs::rename(&paths.owner_lock, &backup).unwrap();
            let lost = inspect_request(h, request(h, ids), h.until);
            std::fs::rename(&backup, &paths.owner_lock).unwrap();
            assert!(
                lost.is_err(),
                "actual lock inode no longer occupies its pinned path"
            );
            assert_ne!(token(&inspect(h, ids)), original);
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            h.coordinator.fence(h.until).unwrap();
            h.drain();
            let fenced = inspect_request(h, request(h, ids), h.until);
            assert!(fenced.is_err() || fenced.is_ok_and(|v| v.request_token.is_none()));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn permission_inspection_observation_gap_revokes_authority_before_recovery() {
        with_inspection(|h, _, _| {
            let ids = stable_form(h);
            let original = token(&inspect(h, ids));
            let generation = h.controller.request_generation();
            h.controller
                .observer_failed(
                    super::super::ObservationFailure::Parser,
                    &super::super::RealObservationClock,
                )
                .unwrap();
            assert!(h.controller.current_request().is_none());
            assert!(h.controller.request_generation() > generation);
            let unavailable = inspect_request(h, request(h, ids), h.until);
            assert!(
                unavailable.is_err()
                    || unavailable.is_ok_and(|value| value.request_token.is_none())
            );
            let later_ids = stable_form(h);
            assert_ne!(token(&inspect(h, later_ids)), original);
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn permission_inspection_missing_facts_use_validated_post_policy_evidence_only() {
        use super::super::{TerminalSize, TerminalState};
        use crate::coding_agent::{CodingAgent, Detector};
        let catalog = ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
        let detector =
            Detector::new(&catalog, CodingAgent::Codex, Some("synthetic-0.0.1")).unwrap();
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        terminal
            .feed(&inspection_fixture_form(
                "write public.txt",
                "synthetic-only",
            ))
            .unwrap();
        let raw = detector.observe(&terminal.snapshot()).unwrap().raw_evidence;
        let redacted = storage::evidence::EvidencePolicy::redacted(&serde_json::json!({"schema_version":1,"patterns":["OWNED_SECRET"],"sensitive_rules":[]}),&["synthetic-permission"]).unwrap();
        let missing = |value: &serde_json::Value| {
            inspection_missing_evidence(value)
                .unwrap()
                .iter()
                .map(MissingEvidence::as_str)
                .collect::<Vec<_>>()
        };
        let public = redacted.apply(&raw).unwrap();
        let unavailable = [
            "operation",
            "arguments",
            "cwd",
            "environment",
            "targets",
            "effects",
            "choice_scope",
        ];
        assert_eq!(missing(&public), unavailable[1..]);
        assert_eq!(
            missing(
                &storage::evidence::EvidencePolicy::metadata()
                    .apply(&raw)
                    .unwrap()
            ),
            unavailable
        );
        for operation in [
            serde_json::Value::Null,
            serde_json::json!(" "),
            serde_json::json!("OWNED_SECRET"),
            serde_json::json!("[redacted]"),
        ] {
            let mut changed = raw.clone();
            changed["operation"] = operation;
            assert_eq!(missing(&redacted.apply(&changed).unwrap()), unavailable);
        }
        let mut truncated = raw.clone();
        truncated["truncated"] = serde_json::json!(true);
        assert_eq!(missing(&redacted.apply(&truncated).unwrap()), unavailable);
        let mut corrupt = public;
        corrupt["evidence_digest"] = serde_json::json!("f".repeat(64));
        assert!(inspection_missing_evidence(&corrupt).is_err());
        corrupt.as_object_mut().unwrap().remove("operation");
        assert!(inspection_missing_evidence(&corrupt).is_err());
    }

    #[test]
    fn permission_inspection_current_authority_has_no_debug_or_serialized_representation() {
        trait AmbiguousIfImplemented<Marker> {
            fn check() {}
        }
        struct DebugImplemented;
        struct SerializeImplemented;
        impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
        impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
        impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
        let _ = <CurrentRequest as AmbiguousIfImplemented<_>>::check;
    }
}

#[cfg(test)]
mod permission_response_contracts {
    use super::contract_tests::ordinary_prompt::support::{Host, Pty};
    use super::permission_inspection_contracts::{
        counts, form, inspect, stable_form, with_definition, with_inspection,
    };
    use super::*;
    use crate::broker::runtime::{DecisionSource, DirectorMode, PermissionDecision};
    use crate::coding_agent::{CodingAgent, inspection_fixture_definition};
    use crate::runtime::runner::{
        ByteInput, PermissionInspection, PermissionRespondInput, PermissionResponseResult,
        ReceiptAction, ReceiptReply, TerminalSize,
    };
    use std::time::Duration;

    fn command(
        h: &Host,
        inspection: &PermissionInspection,
        key: &str,
        decision: PermissionDecision,
    ) -> PermissionRespondInput {
        PermissionRespondInput {
            event_id: inspection.event_id,
            interaction_id: inspection.interaction_id,
            director_member_id: h.fixture.run.director_member_id,
            expected_run_id: h.fixture.run.run_id,
            expected_owner_epoch: h.fixture.run.owner_epoch,
            request_token: inspection
                .request_token
                .as_ref()
                .expect("actual inspected token")
                .clone(),
            decision,
            decision_source: DecisionSource::DirectorAuto,
            director_mode: DirectorMode::Auto,
            mode_evidence_ref: "fixture:response-mode".into(),
            decision_evidence_ref: "fixture:response-decision".into(),
            operation_key: key.into(),
        }
    }
    fn copy(input: &PermissionRespondInput) -> PermissionRespondInput {
        serde_json::from_value(serde_json::to_value(input).unwrap()).unwrap()
    }
    fn handle(
        h: &mut Host,
        input: &PermissionRespondInput,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let mut request = h.request("unused-response-key");
        request.operation = InputControlOperation::PermissionRespond(copy(input));
        h.coordinator.handle_received(
            &mut h.fixture.conn,
            &h.fixture.run,
            request,
            &h.controller,
            h.until,
        )
    }
    fn response(reply: InputControlReply) -> PermissionResponseResult {
        let InputControlReply::PermissionResponse(value) = reply else {
            panic!("typed actual permission response")
        };
        value
    }
    fn arm_stage(h: &mut Host, before: usize) -> i64 {
        let offers = h.pty.offers.len();
        for _ in 0..16 {
            h.drain();
            assert!(
                h.advance().is_none(),
                "stage acquisition yields after its bounded hooks"
            );
            assert_eq!(
                h.pty.offers.len(),
                offers,
                "fresh guard acquisition precedes its syscall"
            );
            if h.hooks.count("before") == before {
                return h.hooks.input_id();
            }
        }
        panic!("real selected stage reaches Armed within its finite turn budget");
    }
    fn audit(
        h: &Host,
        id: i64,
        input: &PermissionRespondInput,
        rule: &str,
    ) -> storage::PermissionReceipt {
        let receipt = storage::get_permission_response(&h.fixture.conn, id).unwrap();
        assert_eq!(receipt.input.kind, storage::InputKind::PermissionResponse);
        assert_eq!(receipt.input.operation_key, input.operation_key);
        assert_eq!(receipt.input.message_id, None);
        let m = &receipt.response.metadata;
        assert_eq!(
            (
                m.event_id,
                m.interaction_id,
                m.expected_owner_epoch,
                m.director_member_id
            ),
            (
                input.event_id,
                input.interaction_id,
                input.expected_owner_epoch,
                input.director_member_id
            )
        );
        assert_eq!(
            (
                &m.request_token,
                m.decision,
                m.decision_source,
                m.director_mode
            ),
            (
                &input.request_token,
                input.decision,
                input.decision_source,
                input.director_mode
            )
        );
        assert_eq!(
            (&m.mode_evidence_ref, &m.decision_evidence_ref),
            (&input.mode_evidence_ref, &input.decision_evidence_ref)
        );
        assert_eq!(m.answer_profile_version, "synthetic-profile-1");
        assert_eq!(m.answer_rule_id, rule);
        assert_eq!(
            receipt
                .claim
                .as_ref()
                .map(|c| (c.interaction_id, c.input_id)),
            Some((input.interaction_id, id))
        );
        receipt
    }

    #[test]
    fn permission_response_fresh_admission_rejects_revoked_token_then_reserves_selected_deny_plan()
    {
        with_inspection(|h, _, _| {
            let ids = stable_form(h);
            let inspected = inspect(h, ids);
            let stale = command(h, &inspected, "response:stale", PermissionDecision::Allow);
            h.human.bytes.push_back(b'!');
            h.drain();
            let failure = match handle(h, &stale) {
                Err(failure) => failure,
                Ok(_) => panic!("revoked token refuses the fresh response"),
            };
            assert_eq!(
                (failure.run_id, failure.interaction_id, failure.input_id),
                (Some(h.fixture.run.run_id), Some(ids.1), None)
            );
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(stale.operation_key.as_str())
            );
            assert!(!failure.source.message().contains(&stale.request_token));
            assert_eq!(counts(h), [0, 0, 0]);
            assert!(h.pty.program.is_empty());
            assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
            let before_gap = inspect(h, ids);
            let before_gap = command(
                h,
                &before_gap,
                "response:before-gap",
                PermissionDecision::Allow,
            );
            h.fixture.conn.execute_batch("CREATE TRIGGER reject_response_refresh BEFORE UPDATE OF evidence_json ON member_interactions BEGIN SELECT RAISE(ABORT,'owned response refresh failure'); END").unwrap();
            let generation = h.controller.request_generation();
            h.pty.child.bytes.extend(b"\x1b[?25l");
            h.coordinator
                .progress_with_manual(&mut h.transport, |transport| {
                    transport.quiesce_for_input(
                        &mut h.human,
                        &mut h.pty,
                        &mut h.output,
                        &mut h
                            .controller
                            .observer(&h.writer, &super::super::RealObservationClock),
                        h.until,
                    )
                })
                .unwrap();
            loop {
                let status = h.writer.status();
                if status.degraded {
                    assert!(status.pending_evidence);
                    assert!(
                        status
                            .diagnostic
                            .as_deref()
                            .unwrap()
                            .contains("owned response refresh failure")
                    );
                    break;
                }
                assert!(
                    Instant::now() < h.until,
                    "actual refresh failure is observed"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            h.controller
                .poll(
                    &h.transport.snapshot(),
                    &h.writer,
                    &super::super::RealObservationClock,
                )
                .unwrap();
            assert!(h.controller.status().gap_count > 0);
            assert!(h.controller.current_request().is_none());
            assert!(h.controller.request_generation() > generation);
            let during_gap = h.controller.health(&h.writer);
            let ordinary_key = "prompt:recovered-writer";
            let ordinary = h.request(ordinary_key);
            let InputControlOperation::SubmitPrompt(ordinary_input) = &ordinary.operation else {
                panic!("existing ordinary fixture supplies a Prompt request")
            };
            let ordinary_literal = ordinary_input.literal.clone();
            let held = h.coordinator.handle_received(
                &mut h.fixture.conn,
                &h.fixture.run,
                ordinary,
                &h.controller,
                h.until,
            );
            let removed = h
                .fixture
                .conn
                .execute_batch("DROP TRIGGER reject_response_refresh");
            assert!(
                removed.is_ok(),
                "primary={:?}; trigger_cleanup={removed:?}",
                held.as_ref().err()
            );
            assert!(during_gap.degraded);
            assert!(matches!(during_gap.eligibility, InputEligibility::Hold(_)));
            let Some(InputControlReply::Submission(held)) = held.unwrap() else {
                panic!("actual writer gap returns the ordinary Held reply")
            };
            assert_eq!(
                (held.status, held.stage, held.input_id),
                (SubmissionStatus::Held, SubmissionStage::ZeroBytes, None)
            );
            assert!(h.coordinator.pending.is_none());
            assert!(
                storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
            while h.writer.status().degraded {
                assert!(
                    Instant::now() < h.until,
                    "original writer recovers in budget"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            loop {
                h.drain();
                if h.controller.current_request().is_some() {
                    break;
                }
                assert!(
                    Instant::now() < h.until,
                    "complete request is observed again"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            let recovered = h.controller.health(&h.writer);
            assert!(h.controller.status().gap_count > 0);
            assert!(recovered.gap_count > 0);
            assert!(!recovered.degraded);
            assert_eq!(recovered.pending_count, 0);
            assert!(!recovered.pending_evidence);
            assert!(recovered.diagnostic.is_none());
            let proof = h
                .transport
                .take_input_drain()
                .expect("actual recovered transport installs a current quiet proof");
            h.transport
                .validate_observation_drain(&proof, h.until)
                .unwrap();
            assert!(handle(h, &before_gap).is_err());
            assert_eq!(counts(h), [0, 0, 0]);
            assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
            let fresh = inspect(h, ids);
            let input = command(h, &fresh, "response:fresh-deny", PermissionDecision::Deny);
            assert_ne!(input.request_token, stale.request_token);
            assert_ne!(input.request_token, before_gap.request_token);
            assert!(
                handle(h, &input).unwrap().is_none(),
                "fresh supported response enters the actual pending writer"
            );
            assert_eq!(counts(h), [0, 0, 0]);
            let id = arm_stage(h, 1);
            assert_eq!(counts(h), [1, 1, 1]);
            assert_eq!(
                audit(h, id, &input, "inspection-deny-plan").input.state,
                storage::InputState::InProgress
            );
            h.drain();
            assert!(
                h.advance().is_none(),
                "first complete answer stage is not complete submission"
            );
            let checkpoint = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (checkpoint.stage, checkpoint.written_bytes),
                (SubmissionStage::Partial, 1)
            );
            arm_stage(h, 2);
            h.drain();
            let result = response(h.advance().expect("actual final response stage"));
            assert_eq!(
                (
                    result.submission.status,
                    result.submission.stage,
                    result.submission.input_id
                ),
                (
                    SubmissionStatus::Accepted,
                    SubmissionStage::Submitted,
                    Some(id)
                )
            );
            assert_eq!(
                (
                    result.interaction_id,
                    result.event_id,
                    result.decision,
                    result.decision_source
                ),
                (
                    input.interaction_id,
                    input.event_id,
                    input.decision,
                    input.decision_source
                )
            );
            assert_eq!(
                h.pty.offers,
                vec![b"!".to_vec(), b"n".to_vec(), b"\r".to_vec()]
            );
            assert_eq!(
                audit(h, id, &input, "inspection-deny-plan").input.state,
                storage::InputState::Accepted
            );
            let notified: Option<String> = h
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [ids.2],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(notified.is_none());
            let before_composer = h.transport.snapshot().screen_seq;
            h.pty
                .child
                .bytes
                .extend(b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end");
            loop {
                h.drain();
                let status = h.controller.status();
                let health = h.controller.health(&h.writer);
                if status.eligibility == InputEligibility::ComposerEmpty
                    && status.visibility == Visibility::NotVisible
                    && !status.retained_transition
                    && !status.retained_draft
                    && !status.retained_evidence
                    && status.diagnostic.is_none()
                    && !health.degraded
                    && health.pending_count == 0
                    && !health.pending_evidence
                {
                    break;
                }
                assert!(
                    Instant::now() < h.until,
                    "actual recovered writer observes complete ordinary composition"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(h.transport.snapshot().screen_seq > before_composer);
            assert!(h.controller.status().gap_count > 0);
            let current = h.controller.health(&h.writer);
            assert!(current.gap_count > 0);
            assert_eq!(current.eligibility, InputEligibility::ComposerEmpty);
            assert_eq!(current.visibility, Visibility::NotVisible);
            assert!(current.diagnostic.is_none());
            let proof = h
                .transport
                .take_input_drain()
                .expect("real recovered composer supplies a complete quiet proof");
            h.transport
                .validate_observation_drain(&proof, h.until)
                .unwrap();
            assert!(
                storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                    .unwrap()
                    .is_none()
            );
            assert!(h.coordinator.pending.is_none());
            assert_eq!(
                h.pty.offers,
                vec![b"!".to_vec(), b"n".to_vec(), b"\r".to_vec()]
            );
            let ordinary = h.request(ordinary_key);
            let resumed = h
                .coordinator
                .handle_received(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    ordinary,
                    &h.controller,
                    h.until,
                )
                .unwrap();
            let projection = match resumed.as_ref() {
                None => None,
                Some(InputControlReply::Submission(reply)) => Some((
                    reply.status,
                    reply.stage,
                    reply.input_id,
                    reply.reason.as_deref(),
                )),
                Some(_) => panic!("ordinary retry has its typed submission reply"),
            };
            assert!(
                projection.is_none(),
                "current complete composer accepts explicit retry after writer recovery: reply={projection:?}; status={:?}",
                h.controller.status()
            );
            for _ in 0..16 {
                h.drain();
                assert!(h.advance().is_none());
                assert_eq!(
                    h.pty.offers.len(),
                    3,
                    "fresh guard precedes ordinary native input"
                );
                if h.hooks.count("before") == 3 {
                    break;
                }
            }
            assert_eq!(h.hooks.count("reserved"), 2);
            assert_eq!(h.hooks.count("before"), 3);
            let reserved = storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                .unwrap()
                .unwrap();
            assert_eq!(reserved.kind, storage::InputKind::Prompt);
            assert_eq!(reserved.state, storage::InputState::InProgress);
            assert_ne!(reserved.input_id, id);
            h.drain();
            let Some(InputControlReply::Submission(submitted)) = h.advance() else {
                panic!("recovered ordinary writer returns its real completed submission")
            };
            assert_eq!(
                (submitted.status, submitted.stage, submitted.input_id),
                (
                    SubmissionStatus::Accepted,
                    SubmissionStage::Submitted,
                    Some(reserved.input_id)
                )
            );
            assert_eq!(
                storage::get_input(&h.fixture.conn, reserved.input_id)
                    .unwrap()
                    .state,
                storage::InputState::Accepted
            );
            let expected_bytes = [ordinary_literal.as_bytes(), b"~"].concat();
            assert_eq!(
                h.pty.offers,
                vec![
                    b"!".to_vec(),
                    b"n".to_vec(),
                    b"\r".to_vec(),
                    expected_bytes.clone()
                ]
            );
            let point = h
                .coordinator
                .retained_checkpoint(reserved.input_id)
                .unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (
                    SubmissionStage::Submitted,
                    u64::try_from(expected_bytes.len()).unwrap()
                )
            );
            assert!(h.controller.status().gap_count > 0);
        });
    }
    fn admitted(h: &mut Host, key: &str) -> ((i64, i64, i64, i64), PermissionRespondInput, i64) {
        let ids = stable_form(h);
        let current = inspect(h, ids);
        let input = command(h, &current, key, PermissionDecision::Allow);
        assert!(handle(h, &input).unwrap().is_none());
        let id = arm_stage(h, 1);
        audit(h, id, &input, "inspection-allow-plan");
        (ids, input, id)
    }
    fn first_stage(h: &mut Host, id: i64) {
        h.drain();
        assert!(h.advance().is_none());
        let point = h.coordinator.retained_checkpoint(id).unwrap();
        assert_eq!(
            (point.stage, point.written_bytes),
            (SubmissionStage::Partial, 1)
        );
        assert_eq!(
            storage::get_input(&h.fixture.conn, id).unwrap().state,
            storage::InputState::InProgress
        );
        assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
    }

    fn terminal_zero(h: &mut Host, key: &str) -> (PermissionRespondInput, i64) {
        let (_, input, id) = admitted(h, key);
        h.pty.short = Some(0);
        h.drain();
        let failure = advance(h)
            .err()
            .expect("actual zero-length native write terminates its attempt");
        assert!(
            failure
                .source
                .message()
                .contains("native input write returned zero bytes")
        );
        assert_eq!(failure.input_id, Some(id));
        let point = h.coordinator.retained_checkpoint(id).unwrap();
        assert_eq!(
            (point.stage, point.written_bytes),
            (SubmissionStage::ZeroBytes, 0)
        );
        assert_eq!(
            audit(h, id, &input, "inspection-allow-plan").input.state,
            storage::InputState::Ready
        );
        h.coordinator.finish_pending_attempt();
        h.pty.short = None;
        assert!(h.coordinator.pending.is_none());
        assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
        (input, id)
    }

    #[test]
    fn permission_ready_actual_zero_rearms_exact_binding_and_revoked_authority_preserves_ready() {
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:ready-rearm");
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert!(
                handle(h, &input).unwrap().is_none(),
                "exact current Ready request enters live revalidation"
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                original
            );
            assert_eq!(arm_stage(h, 2), id);
            let rearmed = audit(h, id, &input, "inspection-allow-plan");
            assert_eq!(rearmed.input.state, storage::InputState::InProgress);
            assert_eq!(rearmed.response, original.response);
            assert_eq!(rearmed.input.created_at, original.input.created_at);
            assert_eq!(rearmed.input.payload_digest, original.input.payload_digest);
            assert_eq!(counts(h), [1, 1, 1]);
            h.drain();
            assert!(h.advance().is_none());
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Partial, 1)
            );
            assert_eq!(arm_stage(h, 3), id);
            h.drain();
            let completed = response(h.advance().expect("actual final rearmed stage"));
            assert_eq!(
                (
                    completed.submission.status,
                    completed.submission.stage,
                    completed.submission.input_id
                ),
                (
                    SubmissionStatus::Accepted,
                    SubmissionStage::Submitted,
                    Some(id)
                )
            );
            let accepted = audit(h, id, &input, "inspection-allow-plan");
            assert_eq!(accepted.input.state, storage::InputState::Accepted);
            assert_eq!(accepted.response, original.response);
            assert_eq!(
                h.pty.offers,
                vec![b"y".to_vec(), b"y".to_vec(), b"\r".to_vec()]
            );
        });
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:ready-revoked");
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            h.human.bytes.push_back(b'!');
            h.drain();
            let failure = handle(h, &input)
                .err()
                .expect("actual manual generation revokes Ready authority");
            assert_eq!(
                (failure.run_id, failure.interaction_id, failure.input_id),
                (
                    Some(h.fixture.run.run_id),
                    Some(input.interaction_id),
                    Some(id)
                )
            );
            assert!(h.coordinator.pending.is_none());
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                original
            );
            assert_eq!(counts(h), [1, 1, 1]);
            assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"!".to_vec()]);
        });
    }

    fn advance(h: &mut Host) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        h.coordinator.advance(
            &mut h.fixture.conn,
            &h.fixture.run,
            &mut h.transport,
            &mut h.pty,
            &h.controller,
            h.until,
        )
    }

    fn receipt(
        h: &mut Host,
        action: ReceiptAction,
    ) -> std::result::Result<ReceiptReply, InputFailure> {
        let mut request = h.request("unused-permission-receipt-key");
        request.operation = InputControlOperation::Receipt(action);
        h.coordinator
            .handle_received(
                &mut h.fixture.conn,
                &h.fixture.run,
                request,
                &h.controller,
                h.until,
            )
            .map(|reply| {
                let InputControlReply::Receipt(reply) =
                    reply.expect("receipt completes without taking a pending client")
                else {
                    panic!("actual permission receipt has its typed reply")
                };
                reply
            })
    }

    fn refuses_zero_proof(h: &mut Host, row: &storage::PermissionReceipt) {
        let tx = h
            .fixture
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let guard = h
            .coordinator
            .acquire_receipt_guard(&tx, &h.fixture.run, row, h.until)
            .unwrap();
        assert!(
            guard
                .zero_byte_retirement(
                    &h.fixture.run.owner(),
                    row.input.input_id,
                    row.response.metadata.interaction_id
                )
                .is_err(),
            "actual retained uncertainty supplies exclusion but no zero-byte proof"
        );
        drop(guard);
        tx.rollback().unwrap();
        assert_eq!(
            storage::get_permission_response(&h.fixture.conn, row.input.input_id).unwrap(),
            *row
        );
    }

    fn release_ready_claim(
        h: &mut Host,
        row: &storage::PermissionReceipt,
    ) -> storage::PermissionReceipt {
        let tx = h
            .fixture
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let guard = h
            .coordinator
            .acquire_receipt_guard(&tx, &h.fixture.run, row, h.until)
            .unwrap();
        let proof = guard
            .zero_byte_retirement(
                &h.fixture.run.owner(),
                row.input.input_id,
                row.response.metadata.interaction_id,
            )
            .unwrap();
        tx.rollback().unwrap();
        storage::release_permission_claim(
            &mut h.fixture.conn,
            &h.fixture.run.owner(),
            row.input.input_id,
            &proof,
            h.until,
        )
        .unwrap()
    }

    #[test]
    fn permission_ready_rearm_restores_its_released_claim_and_refuses_a_competing_claim() {
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:ready-claim-restore");
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            let released = release_ready_claim(h, &original);
            assert_eq!(released.input, original.input);
            assert_eq!(released.response, original.response);
            assert!(released.claim.is_none());
            assert!(handle(h, &input).unwrap().is_none());
            assert_eq!(arm_stage(h, 2), id);
            let rearmed = audit(h, id, &input, "inspection-allow-plan");
            assert_eq!(rearmed.input.state, storage::InputState::InProgress);
            assert_eq!(rearmed.response, original.response);
            assert_eq!(rearmed.claim, original.claim);
            assert_eq!(counts(h), [1, 1, 1]);
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            h.coordinator.finish_pending_attempt();
        });
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:ready-foreign-claim");
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            release_ready_claim(h, &original);
            let request = storage::NewPermissionResponse {
                operation_key: "response:competing-episode-owner".into(),
                metadata: original.response.metadata.clone(),
            };
            let cap = h
                .coordinator
                .current_request
                .as_ref()
                .unwrap()
                .0
                .answer_matches
                .iter()
                .find(|matched| matched.plan().capability().decision() == PermissionDecision::Allow)
                .unwrap()
                .plan()
                .capability();
            let storage::PermissionReservation::Reserved(competing) =
                storage::reserve_permission_response(
                    &mut h.fixture.conn,
                    &h.fixture.run.owner(),
                    &request,
                    Some(cap),
                    h.until,
                )
                .unwrap()
            else {
                panic!("separate deliberate key acquires the released claim through actual storage")
            };
            assert_ne!(competing.input.input_id, id);
            let outcome = handle(h, &input);
            let refused = match outcome {
                Err(_) => true,
                Ok(None) => {
                    let mut refused = false;
                    for _ in 0..16 {
                        h.drain();
                        match advance(h) {
                            Err(_) => {
                                refused = true;
                                break;
                            }
                            Ok(None) => {}
                            Ok(Some(reply)) => {
                                let reply = response(reply);
                                assert!(matches!(
                                    reply.submission.status,
                                    SubmissionStatus::Held | SubmissionStatus::Rejected
                                ));
                                assert_eq!(
                                    (reply.submission.input_id, reply.submission.stage),
                                    (Some(id), SubmissionStage::ZeroBytes)
                                );
                                refused = true;
                                break;
                            }
                        }
                    }
                    refused
                }
                Ok(Some(reply)) => {
                    let reply = response(reply);
                    assert!(matches!(
                        reply.submission.status,
                        SubmissionStatus::Held | SubmissionStatus::Rejected
                    ));
                    assert_eq!(
                        (reply.submission.input_id, reply.submission.stage),
                        (Some(id), SubmissionStage::ZeroBytes)
                    );
                    true
                }
            };
            assert!(
                refused,
                "actual foreign-claim check refuses within finite turns"
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, competing.input.input_id)
                    .unwrap(),
                competing
            );
            let unchanged = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(unchanged.input, original.input);
            assert_eq!(unchanged.response, original.response);
            assert!(unchanged.claim.is_none());
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            h.coordinator.finish_pending_attempt();
        });
    }

    #[test]
    fn permission_receipt_possible_initial_io_keeps_claim_through_attributed_settlement_and_retirement()
     {
        with_inspection(|h, _, _| {
            let (_, input, id) = admitted(h, "response:possible-initial-io");
            h.drain();
            let mut guard = h
                .coordinator
                .acquire_write_guard(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    &mut h.transport,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .unwrap();
            assert!(
                guard
                    .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                    .is_err(),
                "an Armed writer lease is not a receipt-exclusion proof"
            );
            guard.begin_write(&h.transport, b"y", h.until).unwrap();
            guard.mark_io_possible();
            drop(guard);
            h.coordinator.finish_pending_attempt();
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Unknown, 0)
            );
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(original.input.state, storage::InputState::InProgress);
            refuses_zero_proof(h, &original);
            assert!(matches!(receipt(h, ReceiptAction::Reconcile {
                input_id: id, expected: storage::InputState::InProgress,
                disposition: storage::InputDisposition::NotSubmitted,
                evidence_ref: "fixture:attributed-possible-io".into(),
            }).unwrap(), ReceiptReply::Settled { input_id } if input_id == id));
            let settled = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(
                settled.input.disposition,
                Some(storage::InputDisposition::NotSubmitted)
            );
            assert_eq!(settled.response, original.response);
            assert_eq!(settled.claim, original.claim);
            refuses_zero_proof(h, &settled);
            assert!(
                matches!(receipt(h, ReceiptAction::Retire { input_id: id }).unwrap(), ReceiptReply::Retired { input_id } if input_id == id)
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                settled
            );
            refuses_zero_proof(h, &settled);
            let repeated = response(handle(h, &input).unwrap().unwrap());
            assert_eq!(
                (repeated.submission.status, repeated.submission.stage),
                (SubmissionStatus::Rejected, SubmissionStage::ZeroBytes)
            );
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn permission_receipt_genuine_zero_exclusion_settlement_and_claim_release_are_atomic() {
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:zero-settlement");
            let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            let run = storage::get_run(&h.fixture.conn, h.fixture.run.run_id).unwrap();
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            let mut other = copy(&input);
            other.operation_key = "response:other-pending".into();
            assert!(handle(h, &other).unwrap().is_none());
            let pending = h.coordinator.pending.as_ref().unwrap();
            let pending_before = (
                pending.request.operation_key.clone(),
                pending.phase,
                pending.deadline,
                pending.input_id,
            );
            let ReceiptReply::Present {
                input_id,
                submission,
            } = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: input.operation_key.clone(),
                },
            )
            .unwrap()
            else {
                panic!("durable Ready receipt remains present alongside another pending writer")
            };
            assert_eq!(input_id, id);
            assert_eq!(
                (submission.status, submission.stage, submission.output_seq),
                (
                    SubmissionStatus::Held,
                    SubmissionStage::ZeroBytes,
                    Some(point.output_seq)
                )
            );
            let reconcile = || ReceiptAction::Reconcile {
                input_id: id,
                expected: storage::InputState::Ready,
                disposition: storage::InputDisposition::NotSubmitted,
                evidence_ref: "fixture:live-zero-byte".into(),
            };
            assert!(receipt(h, reconcile()).is_err());
            let pending = h.coordinator.pending.as_ref().unwrap();
            assert!(
                (
                    pending.request.operation_key.clone(),
                    pending.phase,
                    pending.deadline,
                    pending.input_id
                ) == pending_before
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                original
            );
            h.coordinator.finish_pending_attempt();
            let tx = h
                .fixture
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let mut guard = h
                .coordinator
                .acquire_receipt_guard(&tx, &h.fixture.run, &original, h.until)
                .unwrap();
            assert!(
                guard.begin_write(&h.transport, b"y", h.until).is_err(),
                "receipt exclusion grants no selected-stage write permit"
            );
            drop(guard);
            tx.rollback().unwrap();
            {
                let tx = h
                    .fixture
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let guard = h
                    .coordinator
                    .acquire_receipt_guard(&tx, &h.fixture.run, &original, h.until)
                    .unwrap();
                let proof = guard
                    .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                    .unwrap();
                tx.rollback().unwrap();
                let request = storage::NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: original.response.metadata.clone(),
                };
                assert!(
                    storage::rearm_permission_response(
                        &mut h.fixture.conn,
                        &h.fixture.run.owner(),
                        &request,
                        &proof,
                        h.until
                    )
                    .is_err(),
                    "exclusion-only proof cannot grant current-plan rearm authority"
                );
            }
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                original
            );
            h.fixture
                .conn
                .execute_batch(&format!(
                    "CREATE TRIGGER reject_live_claim_delete
                 BEFORE DELETE ON member_permission_response_claims
                 WHEN OLD.input_id={id}
                 BEGIN
                   SELECT CASE WHEN EXISTS(
                     SELECT 1 FROM member_input_operations
                     WHERE input_id=OLD.input_id AND state='settled'
                       AND disposition='not_submitted'
                       AND evidence_ref='fixture:live-zero-byte'
                   ) THEN RAISE(ABORT,'owned live claim delete failure')
                     ELSE RAISE(ABORT,'owned claim delete missing prior settlement') END;
                 END"
                ))
                .unwrap();
            let failed = receipt(h, reconcile());
            let removed = h
                .fixture
                .conn
                .execute_batch("DROP TRIGGER reject_live_claim_delete");
            assert!(
                removed.is_ok(),
                "primary={:?}; trigger_cleanup={removed:?}",
                failed.as_ref().err()
            );
            let failed = failed
                .err()
                .expect("real claim deletion failure aborts settlement");
            assert!(
                failed
                    .source
                    .message()
                    .contains("owned live claim delete failure")
            );
            assert_eq!(failed.input_id, Some(id));
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                original
            );
            assert_eq!(storage::get_run(&h.fixture.conn, run.run_id).unwrap(), run);
            let unchanged = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (
                    unchanged.stage,
                    unchanged.written_bytes,
                    unchanged.output_seq
                ),
                (point.stage, point.written_bytes, point.output_seq)
            );
            assert!(
                matches!(receipt(h, reconcile()).unwrap(), ReceiptReply::Settled { input_id } if input_id == id)
            );
            let settled = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(settled.input.state, storage::InputState::Settled);
            assert_eq!(
                settled.input.disposition,
                Some(storage::InputDisposition::NotSubmitted)
            );
            assert_eq!(
                settled.input.evidence_ref.as_deref(),
                Some("fixture:live-zero-byte")
            );
            assert_eq!(settled.response, original.response);
            assert_eq!(settled.claim, None);
            assert!(
                matches!(receipt(h, ReceiptAction::Retire { input_id: id }).unwrap(), ReceiptReply::Retired { input_id } if input_id == id)
            );
            assert!(!h.coordinator.operations.contains_key(&id));
            let ReceiptReply::Present {
                input_id,
                submission,
            } = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: input.operation_key.clone(),
                },
            )
            .unwrap()
            else {
                panic!("retired cache retains its consumed durable operation")
            };
            assert_eq!(input_id, id);
            assert_eq!(
                (submission.status, submission.stage, submission.output_seq),
                (SubmissionStatus::Rejected, SubmissionStage::Unknown, None)
            );
            let repeated = response(handle(h, &input).unwrap().unwrap());
            assert_eq!(
                (
                    repeated.submission.status,
                    repeated.submission.input_id,
                    repeated.submission.stage
                ),
                (
                    SubmissionStatus::Rejected,
                    Some(id),
                    SubmissionStage::ZeroBytes
                )
            );
            let ids = stable_form(h);
            let current = inspect(h, ids);
            let fresh = command(
                h,
                &current,
                "response:deliberate-after-zero-settlement",
                PermissionDecision::Allow,
            );
            assert!(handle(h, &fresh).unwrap().is_none());
            for _ in 0..16 {
                h.drain();
                assert!(advance(h).unwrap().is_none());
                if h.hooks.count("reserved") == 2 {
                    break;
                }
            }
            assert_eq!(h.hooks.count("reserved"), 2);
            let next =
                storage::find_input(&h.fixture.conn, h.fixture.run.run_id, &fresh.operation_key)
                    .unwrap()
                    .expect("new deliberate key reserves after proven release");
            assert_ne!(next.input_id, id);
            let next_receipt =
                storage::get_permission_response(&h.fixture.conn, next.input_id).unwrap();
            assert_eq!(next_receipt.claim.as_ref().unwrap().input_id, next.input_id);
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                settled
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            h.coordinator.finish_pending_attempt();
        });
    }

    #[test]
    fn permission_receipt_cacheless_ready_proof_is_borrowed_and_not_an_attributed_disposition() {
        trait AmbiguousIfSend<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        struct SendType;
        impl<T: ?Sized + Send> AmbiguousIfSend<SendType> for T {}
        trait AmbiguousIfClone<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfClone<()> for T {}
        struct CloneType;
        impl<T: Clone> AmbiguousIfClone<CloneType> for T {}
        trait AmbiguousIfSerialize<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
        struct SerializeType;
        impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<SerializeType> for T {}
        let _ = <InputGuard<'static> as AmbiguousIfSend<_>>::check;
        let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfSend<_>>::check;
        let _ = <InputGuard<'static> as AmbiguousIfClone<_>>::check;
        let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfClone<_>>::check;
        let _ = <InputGuard<'static> as AmbiguousIfSerialize<_>>::check;
        let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfSerialize<_>>::check;
        struct NoTokens;
        impl RequestTokenSource for NoTokens {
            fn fill(&mut self, _: &mut [u8; 32]) -> Result<()> {
                panic!("receipt-only proof creates no request token")
            }
        }
        with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
            let (input, id) = terminal_zero(h, "response:cacheless-zero");
            let ready = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            h.coordinator.fence(h.until).unwrap();
            let mut empty = InputCoordinator::new(
                &mut h.fixture.conn,
                &h.fixture.run,
                h.coordinator.lock.clone(),
                InputServices {
                    catalog: ProfileCatalog::from_definitions(&inspection_fixture_definition())
                        .unwrap(),
                    token_source: Box::new(NoTokens),
                    hooks: h.hooks.clone(),
                },
                h.until,
            )
            .unwrap();
            assert!(empty.operations.is_empty());
            {
                let tx = h
                    .fixture
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let guard = empty
                    .acquire_receipt_guard(&tx, &h.fixture.run, &ready, h.until)
                    .unwrap();
                assert!(
                    guard
                        .zero_byte_retirement(&h.fixture.run.owner(), id + 1, input.interaction_id)
                        .is_err()
                );
                let _proof = guard
                    .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                    .unwrap();
                tx.rollback().unwrap();
            }
            storage::settle_input(
                &mut h.fixture.conn,
                &h.fixture.run.owner(),
                id,
                storage::InputState::Ready,
                storage::InputDisposition::NotSubmitted,
                "fixture:attributed-only",
            )
            .unwrap();
            let attributed = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert!(attributed.claim.is_some());
            let tx = h
                .fixture
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let guard = empty
                .acquire_receipt_guard(&tx, &h.fixture.run, &attributed, h.until)
                .unwrap();
            assert!(
                guard
                    .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                    .is_err(),
                "cache-less NotSubmitted label supplies no byte proof"
            );
            drop(guard);
            tx.rollback().unwrap();
            let mut request = h.request("unused-cacheless-retire");
            request.operation =
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
            assert!(
                matches!(empty.handle_received(&mut h.fixture.conn, &h.fixture.run, request, &h.controller, h.until).unwrap(), Some(InputControlReply::Receipt(ReceiptReply::Retired { input_id })) if input_id == id)
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                attributed
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
        });
    }
    struct StageIo<'a> {
        inner: &'a mut Pty,
        fault: Option<io::ErrorKind>,
    }
    impl ByteInput for StageIo<'_> {
        fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.inner.try_read(bytes)
        }
    }
    impl PtyIo for StageIo<'_> {
        fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.fault.take() {
                assert!(!bytes.is_empty());
                self.inner.offers.push(bytes.to_vec());
                return Err(kind.into());
            }
            self.inner.try_write(bytes)
        }
        fn resize(&mut self, size: TerminalSize) -> Result<()> {
            self.inner.resize(size)
        }
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }
    }
    fn pending_syscall(h: &mut Host, kind: io::ErrorKind) {
        h.drain();
        let mut io = StageIo {
            inner: &mut h.pty,
            fault: Some(kind),
        };
        let result = h.coordinator.advance(
            &mut h.fixture.conn,
            &h.fixture.run,
            &mut h.transport,
            &mut io,
            &h.controller,
            h.until,
        );
        assert!(
            io.fault.is_none(),
            "real selected-stage PtyIo call consumes this fault"
        );
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn permission_response_raced_retained_reservation_and_claim_supply_no_write_authority() {
        with_inspection(|h, _, _| {
            let ids = stable_form(h);
            let inspected = inspect(h, ids);
            let input = command(
                h,
                &inspected,
                "response:reservation-race",
                PermissionDecision::Allow,
            );
            assert!(handle(h, &input).unwrap().is_none());
            let plan = h
                .coordinator
                .services
                .catalog
                .answer_plan(
                    CodingAgent::Codex,
                    "synthetic-0.0.1",
                    "synthetic-permission",
                    input.decision,
                )
                .unwrap()
                .unwrap();
            let cap = plan.capability();
            let request = storage::NewPermissionResponse {
                operation_key: input.operation_key.clone(),
                metadata: storage::PermissionResponseMetadata {
                    event_id: input.event_id,
                    interaction_id: input.interaction_id,
                    expected_owner_epoch: input.expected_owner_epoch,
                    request_token: input.request_token.clone(),
                    decision: input.decision,
                    director_member_id: input.director_member_id,
                    director_mode: input.director_mode,
                    decision_source: input.decision_source,
                    mode_evidence_ref: input.mode_evidence_ref.clone(),
                    decision_evidence_ref: input.decision_evidence_ref.clone(),
                    answer_profile_version: cap.profile_version().into(),
                    answer_rule_id: cap.answer_rule_id().into(),
                },
            };
            let storage::PermissionReservation::Reserved(original) =
                storage::reserve_permission_response(
                    &mut h.fixture.conn,
                    &h.fixture.run.owner(),
                    &request,
                    Some(cap),
                    h.until,
                )
                .unwrap()
            else {
                panic!("independent storage wins actual fresh reservation")
            };
            h.drain();
            let result = response(h.advance().expect("race returns existing receipt"));
            assert_eq!(
                (
                    result.submission.input_id,
                    result.submission.status,
                    result.submission.stage
                ),
                (
                    Some(original.input.input_id),
                    SubmissionStatus::Uncertain,
                    SubmissionStage::Unknown
                )
            );
            assert_eq!(h.hooks.count("reserved"), 0);
            assert!(h.coordinator.operations.is_empty());
            assert!(h.coordinator.pending.is_none());
            assert!(h.pty.offers.is_empty());
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, original.input.input_id).unwrap(),
                original
            );
            let other = command(
                h,
                &inspected,
                "response:different-claimed-key",
                PermissionDecision::Deny,
            );
            let mut outcome = handle(h, &other);
            for _ in 0..16 {
                if !matches!(outcome, Ok(None)) {
                    break;
                }
                h.drain();
                outcome = advance(h);
            }
            match outcome {
                Err(failure) => assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(other.operation_key.as_str())
                ),
                Ok(Some(reply)) => assert_ne!(
                    response(reply).submission.status,
                    SubmissionStatus::Accepted
                ),
                Ok(None) => panic!("real competing claim completes refusal within finite turns"),
            }
            assert_eq!(counts(h), [1, 1, 1]);
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, original.input.input_id).unwrap(),
                original
            );
            assert!(h.pty.offers.is_empty());
            h.coordinator.finish_pending_attempt();
            let id = original.input.input_id;
            storage::record_input_state(
                &mut h.fixture.conn,
                &h.fixture.run.owner(),
                id,
                storage::InputState::InProgress,
                storage::InputState::Ready,
            )
            .unwrap();
            let ready = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(ready.response, original.response);
            assert_eq!(ready.claim, original.claim);
            assert!(h.coordinator.operations.is_empty());
            let expired = Instant::now() + Duration::from_millis(100);
            assert!(expired < h.until);
            let mut request = h.request("unused-cacheless-ready-expiry");
            request.operation = InputControlOperation::PermissionRespond(copy(&input));
            assert!(
                h.coordinator
                    .handle_received(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        request,
                        &h.controller,
                        expired,
                    )
                    .unwrap()
                    .is_none()
            );
            let pending = h.coordinator.pending.as_ref().unwrap();
            assert_eq!((pending.input_id, pending.deadline), (Some(id), expired));
            assert_eq!(pending.ready.as_ref().unwrap(), &ready);
            assert!(h.coordinator.operations.is_empty());
            assert!(h.pty.offers.is_empty());
            std::thread::sleep(expired.saturating_duration_since(Instant::now()));
            assert!(Instant::now() >= expired);
            let failure = advance(h)
                .err()
                .expect("original pending expiration returns its primary failure");
            assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
            assert_eq!(
                (failure.run_id, failure.interaction_id, failure.input_id),
                (
                    Some(h.fixture.run.run_id),
                    Some(input.interaction_id),
                    Some(id)
                )
            );
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(input.operation_key.as_str())
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                ready
            );
            assert!(h.coordinator.operations.is_empty());
            assert!(h.pty.offers.is_empty());
            h.coordinator.finish_pending_attempt();
            assert!(h.coordinator.pending.is_none());
            assert!(handle(h, &input).unwrap().is_none());
            assert_eq!(h.coordinator.pending.as_ref().unwrap().input_id, Some(id));
            h.coordinator.finish_pending_attempt();
            assert!(h.coordinator.pending.is_none());
            assert!(h.coordinator.operations.is_empty());
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                ready
            );
            assert!(h.pty.offers.is_empty());
            assert_eq!(h.hooks.count("reserved"), 0);
            assert!(handle(h, &input).unwrap().is_none());
            assert_eq!(arm_stage(h, 1), id);
            h.drain();
            assert!(h.advance().is_none());
            assert_eq!(arm_stage(h, 2), id);
            h.drain();
            let accepted = response(h.advance().expect("fresh explicit rearm remains usable"));
            assert_eq!(
                (accepted.submission.input_id, accepted.submission.status),
                (Some(id), SubmissionStatus::Accepted)
            );
            let completed = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(completed.response, ready.response);
            assert_eq!(completed.claim, ready.claim);
            assert_eq!(completed.input.state, storage::InputState::Accepted);
            assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"\r".to_vec()]);
        });
    }

    #[test]
    fn permission_response_later_pending_preserves_partial_and_retries_only_untouched_stage() {
        with_inspection(|h, _, _| {
            let (_, input, id) = admitted(h, "response:later-pending");
            first_stage(h, id);
            let original = h.coordinator.pending.as_ref().unwrap().deadline;
            for (index, kind) in [io::ErrorKind::WouldBlock, io::ErrorKind::Interrupted]
                .into_iter()
                .enumerate()
            {
                arm_stage(h, index + 2);
                pending_syscall(h, kind);
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                assert_eq!(h.coordinator.pending.as_ref().unwrap().deadline, original);
                assert_eq!(
                    audit(h, id, &input, "inspection-allow-plan").input.state,
                    storage::InputState::InProgress
                );
                let offers = h.pty.offers.len();
                assert!(advance(h).unwrap().is_none());
                assert_eq!(
                    h.pty.offers.len(),
                    offers,
                    "consumed guard requires a new real drain"
                );
            }
            arm_stage(h, 4);
            h.drain();
            let result = response(h.advance().expect("last untouched stage completes"));
            assert_eq!(
                (result.submission.status, result.submission.stage),
                (SubmissionStatus::Accepted, SubmissionStage::Submitted)
            );
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(point.written_bytes, 2);
            assert_eq!(
                h.pty.offers,
                vec![
                    b"y".to_vec(),
                    b"\r".to_vec(),
                    b"\r".to_vec(),
                    b"\r".to_vec()
                ]
            );
            let after = h
                .hooks
                .points
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.boundary == "after")
                .map(|p| p.count)
                .collect::<Vec<_>>();
            assert_eq!(after, vec![1, 1, 1, 2]);
            assert_eq!(
                audit(h, id, &input, "inspection-allow-plan").input.state,
                storage::InputState::Accepted
            );
        });
    }

    #[test]
    fn permission_response_changed_focus_after_positive_stage_stops_remaining_bytes() {
        with_inspection(|h, _, _| {
            let (_, input, id) = admitted(h, "response:changed-focus");
            first_stage(h, id);
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other");
            let outcome = advance(h);
            match outcome {
                Err(failure) => {
                    assert_eq!(failure.input_id, Some(id));
                    assert_eq!(failure.interaction_id, Some(input.interaction_id));
                }
                Ok(Some(reply)) => assert_eq!(
                    response(reply).submission.status,
                    SubmissionStatus::Uncertain
                ),
                Ok(None) => {
                    panic!("incompatible positive attempt terminates without another stage")
                }
            }
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Partial, 1)
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            assert_eq!(
                audit(h, id, &input, "inspection-allow-plan").input.state,
                storage::InputState::Uncertain
            );
        });
    }

    #[test]
    fn permission_response_positive_short_later_stage_keeps_cumulative_uncertainty_without_tail() {
        let mut definition = inspection_fixture_definition();
        definition["profiles"][0]["capabilities"]["answers"][0]["stages"][1]["bytes_hex"] =
            serde_json::json!("6f6b");
        with_definition(definition, b"ok".to_vec(), |h, _, _| {
            let (_, input, id) = admitted(h, "response:short-later-stage");
            first_stage(h, id);
            arm_stage(h, 2);
            h.pty.short = Some(1);
            h.drain();
            let result = response(h.advance().expect("positive short ends the answer plan"));
            assert_eq!(
                (
                    result.submission.status,
                    result.submission.stage,
                    result.submission.input_id
                ),
                (
                    SubmissionStatus::Uncertain,
                    SubmissionStage::Partial,
                    Some(id)
                )
            );
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Partial, 2)
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"ok".to_vec()]);
            assert_eq!(
                audit(h, id, &input, "inspection-allow-plan").input.state,
                storage::InputState::Uncertain
            );
            assert!(advance(h).unwrap().is_none());
            assert_eq!(h.pty.offers.len(), 2);
            let row = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            refuses_zero_proof(h, &row);
        });
    }

    #[test]
    fn permission_response_unrecorded_possible_later_io_and_abandonment_preserve_prior_evidence() {
        with_inspection(|h, _, _| {
            let (_, input, id) = admitted(h, "response:abandoned-later-io");
            first_stage(h, id);
            arm_stage(h, 2);
            h.drain();
            let row = audit(h, id, &input, "inspection-allow-plan");
            let mut guard = h
                .coordinator
                .acquire_write_guard(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    &mut h.transport,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .expect("genuine later-stage guard");
            guard.begin_write(&h.transport, b"\r", h.until).unwrap();
            guard.mark_io_possible();
            drop(guard);
            h.coordinator.finish_pending_attempt();
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Partial, 1)
            );
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                row
            );
            refuses_zero_proof(h, &row);
            assert!(h.coordinator.pending.is_none());
            assert!(advance(h).unwrap().is_none());
            let repeated = response(
                handle(h, &input)
                    .unwrap()
                    .expect("durable InProgress returns uncertainty"),
            );
            assert_eq!(
                (repeated.submission.input_id, repeated.submission.status),
                (Some(id), SubmissionStatus::Uncertain)
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
        });
    }

    #[test]
    fn permission_response_accepted_bookkeeping_failure_keeps_known_context_claim_and_actual_counts()
     {
        with_inspection(|h, _, _| {
            let (_, input, id) = admitted(h, "response:accepted-failure");
            first_stage(h, id);
            arm_stage(h, 2);
            h.fixture.conn.execute_batch("CREATE TRIGGER owned_response_accept_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.kind='permission_response' AND NEW.state='accepted' BEGIN SELECT RAISE(ABORT,'owned response accepted failure'); END;").unwrap();
            h.drain();
            let failure = match advance(h) {
                Err(failure) => failure,
                Ok(_) => panic!("real Accepted trigger failure remains an error"),
            };
            h.fixture
                .conn
                .execute_batch("DROP TRIGGER owned_response_accept_failure;")
                .unwrap();
            assert!(
                failure
                    .source
                    .message()
                    .contains("owned response accepted failure")
            );
            assert_eq!(
                (failure.run_id, failure.interaction_id, failure.input_id),
                (
                    Some(h.fixture.run.run_id),
                    Some(input.interaction_id),
                    Some(id)
                )
            );
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(input.operation_key.as_str())
            );
            assert!(!failure.source.message().contains(&input.request_token));
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Submitted, 2)
            );
            assert_eq!(
                audit(h, id, &input, "inspection-allow-plan").input.state,
                storage::InputState::InProgress
            );
            h.coordinator.finish_pending_attempt();
            assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"\r".to_vec()]);
            let repeated = response(
                handle(h, &input)
                    .unwrap()
                    .expect("failed commit is retained uncertainty"),
            );
            assert_eq!(repeated.submission.status, SubmissionStatus::Uncertain);
            assert_eq!(h.pty.offers.len(), 2);
        });
    }
}

pub(crate) trait RequestTokenSource: Send {
    fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()>;
}

#[expect(
    dead_code,
    reason = "the reviewed input phases publish hook boundaries"
)]
pub(crate) enum InputBoundary {
    Reserved,
    BeforeWrite,
    AfterWrite,
    TerminalState,
    Settled,
    Retired,
}

pub(crate) struct InputCheckpoint {
    pub input_id: Option<i64>,
    pub stage: SubmissionStage,
    pub written_bytes: u64,
    pub output_seq: u64,
}

pub(crate) trait InputHooks: Send + Sync {
    fn checkpoint(
        &self,
        boundary: InputBoundary,
        state: &InputCheckpoint,
        deadline: Instant,
    ) -> Result<()>;
}

pub(crate) struct InertInputHooks;

impl InputHooks for InertInputHooks {
    fn checkpoint(&self, _: InputBoundary, _: &InputCheckpoint, _: Instant) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct InputServices {
    pub catalog: ProfileCatalog,
    pub token_source: Box<dyn RequestTokenSource>,
    pub hooks: Arc<dyn InputHooks>,
}

pub(crate) struct InputCoordinator {
    run: RunRecord,
    lock: Arc<RunLock>,
    services: InputServices,
    manual_generation: u64,
    manual_accounting_failed: bool,
    placement: Placement,
    operations: BTreeMap<i64, RetainedInput>,
    pending: Option<PendingInput>,
    current_request: Option<(CurrentRequest, String)>,
    input_fenced: bool,
}

#[derive(Clone)]
pub(crate) struct CurrentRequest {
    request_generation: u64,
    identity: ControlIdentity,
    interaction_id: i64,
    event_id: i64,
    message_id: i64,
    episode_seq: i64,
    native_version: String,
    detector_profile_version: String,
    request_rule_id: String,
    semantic: SemanticFingerprint,
    choices: Vec<String>,
    focus: String,
    native_correlation: Option<String>,
    manual_generation: u64,
    output_seq: u64,
    answer_matches: Vec<AnswerMatch>,
}
impl CurrentRequest {
    fn same_request(&self, other: &Self) -> bool {
        self.request_generation == other.request_generation
            && self.identity == other.identity
            && self.interaction_id == other.interaction_id
            && self.event_id == other.event_id
            && self.message_id == other.message_id
            && self.episode_seq == other.episode_seq
            && self.native_version == other.native_version
            && self.detector_profile_version == other.detector_profile_version
            && self.request_rule_id == other.request_rule_id
            && self.semantic == other.semantic
            && self.choices == other.choices
            && self.focus == other.focus
            && self.native_correlation == other.native_correlation
            && self.manual_generation == other.manual_generation
            && self.answer_matches.len() == other.answer_matches.len()
            && self
                .answer_matches
                .iter()
                .zip(&other.answer_matches)
                .all(|(a, b)| a.same_request(b))
    }
}

pub(super) fn inspection_association(
    conn: &Connection,
    run: &RunRecord,
    input: &PermissionInspectInput,
) -> Result<storage::RuntimeEventView> {
    ensure(
        input.event_id > 0
            && input.interaction_id > 0
            && input.director_member_id == run.director_member_id,
        "permission inspection identity mismatch",
    )?;
    let view = storage::get_event(conn, input.event_id)?;
    ensure(
        view.event.run_id == run.run_id
            && view.event.interaction_id == Some(input.interaction_id)
            && view.event.kind == storage::RuntimeEventKind::InteractionOpened
            && view.event.notification_state == storage::NotificationState::Inbox
            && view.event.message_id.is_some_and(|id| id > 0)
            && view.interaction.as_ref().is_some_and(|i| {
                i.run_id == run.run_id
                    && i.classification == storage::InteractionClass::PermissionRequest
                    && i.closed_at.is_none()
            }),
        "permission inspection current event/interaction/message mismatch",
    )?;
    let director = crate::broker::get_member(conn, run.director_member_id, run.fleet_id)?
        .ok_or_else(|| error("permission Director is absent"))?;
    ensure(
        director.kind == MemberKind::Director
            && crate::broker::active_member_fleet(conn, run.director_member_id)?
                == Some(run.fleet_id),
        "permission Director is inactive",
    )?;
    let live: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM fleets WHERE fleet_id=?1 AND director_member_id=?2 AND deleted_at IS NULL)", rusqlite::params![run.fleet_id, run.director_member_id], |r| r.get(0)).map_err(db_err)?;
    ensure(live, "permission fleet/Director changed")?;
    Ok(view)
}

pub(super) fn inspection_missing_evidence(
    evidence: &serde_json::Value,
) -> Result<Vec<MissingEvidence>> {
    storage::evidence::validate(evidence)?;
    let mut missing = Vec::new();
    let known = evidence["truncated"] == false
        && evidence["operation"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty() && text != "[redacted]");
    if !known {
        missing.push(MissingEvidence::Operation);
    }
    missing.extend([
        MissingEvidence::Arguments,
        MissingEvidence::Cwd,
        MissingEvidence::Environment,
        MissingEvidence::Targets,
        MissingEvidence::Effects,
        MissingEvidence::ChoiceScope,
    ]);
    Ok(missing)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    AwaitReservation,
    AwaitBeforeWrite,
    Armed,
    Blocked,
}

struct PermissionSelection {
    request: storage::NewPermissionResponse,
    binding: CurrentRequest,
    plan: AnswerPlan,
}

enum RetainedSelection {
    Composer,
    Permission {
        selection: Arc<PermissionSelection>,
        next_stage: usize,
    },
}

#[derive(Clone, Copy)]
enum PendingReservation {
    Unreserved,
    HistoricalReady,
    Installed,
}

struct PendingInput {
    reservation: PendingReservation,
    ready: Option<storage::PermissionReceipt>,
    request: storage::NewInput,
    bytes: Arc<[u8]>,
    permission: Option<Arc<PermissionSelection>>,
    deadline: Instant,
    phase: InputPhase,
    input_id: Option<i64>,
}

impl PendingInput {
    fn installed_input_id(&self) -> Option<i64> {
        match self.reservation {
            PendingReservation::Unreserved => {
                assert!(
                    self.input_id.is_none(),
                    "unreserved input has no installed ID"
                );
                assert!(
                    self.ready.is_none(),
                    "unreserved input has no Ready history"
                );
                None
            }
            PendingReservation::HistoricalReady => {
                let ready = self
                    .ready
                    .as_ref()
                    .expect("historical Ready snapshot retained");
                assert_eq!(self.input_id, Some(ready.input.input_id));
                assert_eq!(ready.input.state, storage::InputState::Ready);
                None
            }
            PendingReservation::Installed => Some(
                self.input_id
                    .expect("installed input reservation has an ID"),
            ),
        }
    }
}

struct RetainedInput {
    operation: storage::InputOperation,
    bytes: Arc<[u8]>,
    selection: RetainedSelection,
    stage: SubmissionStage,
    written_bytes: u64,
    output_seq: u64,
    attempt_available: bool,
    leased: bool,
}

fn selected_answer_stage(plan: &AnswerPlan, index: usize) -> Result<&AnswerStage> {
    plan.stages()
        .get(index)
        .ok_or_else(|| error("selected answer stage is absent"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardPermit {
    Fresh,
    Rejected,
    ConsumedPreIo,
    IoPossible,
    ResultRecorded,
}

struct ReceiptGuardBinding {
    owner: storage::RunOwner,
    receipt: storage::PermissionReceipt,
    zero_known: bool,
    rearm: Option<Arc<PermissionSelection>>,
}

pub(crate) struct InputGuard<'a> {
    retained: Option<&'a mut RetainedInput>,
    proof: Option<DrainedInput>,
    binding: Option<ReceiptGuardBinding>,
    deadline: Instant,
    permit: GuardPermit,
    exclusion: std::marker::PhantomData<(&'a mut InputCoordinator, std::rc::Rc<()>)>,
}

impl InputGuard<'_> {
    pub(crate) fn zero_byte_retirement(
        &self,
        owner: &storage::RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<storage::ZeroByteRetirement<'_>> {
        storage::ZeroByteRetirement::from_live_guard(self, owner, input_id, interaction_id)
    }

    pub(crate) fn retirement_binding(
        &self,
        owner: &storage::RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<(
        &storage::PermissionReceipt,
        Option<&crate::coding_agent::AnswerCapability>,
    )> {
        check_deadline(Instant::now(), self.deadline)?;
        let binding = self
            .binding
            .as_ref()
            .ok_or_else(|| error("native writer has no receipt-exclusion proof"))?;
        ensure(
            binding.owner == *owner
                && binding.receipt.input.input_id == input_id
                && binding.receipt.response.metadata.interaction_id == interaction_id
                && binding.zero_known,
            "matching actual zero-byte exclusion required",
        )?;
        Ok((
            &binding.receipt,
            binding
                .rearm
                .as_ref()
                .map(|selection| selection.plan.capability()),
        ))
    }

    pub(super) fn begin_write(
        &mut self,
        transport: &Transport,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<Instant> {
        ensure(
            self.binding.is_none(),
            "receipt exclusion grants no native-write permit",
        )?;
        let retained = self
            .retained
            .as_mut()
            .ok_or_else(|| error("native writer is absent"))?;
        ensure(
            self.permit == GuardPermit::Fresh,
            "input write permit is consumed",
        )?;
        self.permit = GuardPermit::Rejected;
        self.deadline = self.deadline.min(deadline);
        check_deadline(Instant::now(), self.deadline)?;
        ensure(
            bytes == retained.bytes.as_ref(),
            "input bytes do not match the selected stage",
        )?;
        ensure(
            retained.operation.state == storage::InputState::InProgress && retained.leased,
            "input write requires its retained reservation",
        )?;
        match &retained.selection {
            RetainedSelection::Composer => transport.validate_input_drain(
                self.proof
                    .as_ref()
                    .ok_or_else(|| error("native drain is absent"))?,
                self.deadline,
            )?,
            RetainedSelection::Permission {
                selection,
                next_stage,
            } => {
                let stage = selected_answer_stage(&selection.plan, *next_stage)?;
                ensure(stage.bytes() == bytes, "permission stage selection changed")?;
                transport.validate_observation_drain(
                    self.proof
                        .as_ref()
                        .ok_or_else(|| error("native drain is absent"))?,
                    self.deadline,
                )?;
            }
        }
        self.permit = GuardPermit::ConsumedPreIo;
        Ok(self.deadline)
    }

    pub(super) fn mark_io_possible(&mut self) {
        assert!(
            self.permit == GuardPermit::ConsumedPreIo,
            "possible I/O requires a validated consumed permit"
        );
        self.permit = GuardPermit::IoPossible;
    }

    pub(super) fn record_write_result(
        &mut self,
        result: &io::Result<usize>,
        offered: usize,
    ) -> Result<()> {
        let retained = self
            .retained
            .as_mut()
            .ok_or_else(|| error("native writer is absent"))?;
        ensure(
            self.permit == GuardPermit::IoPossible,
            "input result requires its possible-I/O attempt",
        )?;
        ensure(
            offered == retained.bytes.len(),
            "input offered count does not match its selected stage",
        )?;
        match result {
            Ok(count) => {
                ensure(*count <= offered, "native input count exceeds its offer")?;
                retained.written_bytes = retained
                    .written_bytes
                    .checked_add(
                        u64::try_from(*count)
                            .map_err(|_| error("native input count exceeds range"))?,
                    )
                    .ok_or_else(|| error("native input count exhausted"))?;
                if *count == offered {
                    retained.stage = match &mut retained.selection {
                        RetainedSelection::Composer => SubmissionStage::Submitted,
                        RetainedSelection::Permission {
                            selection,
                            next_stage,
                        } => {
                            let _ = selected_answer_stage(&selection.plan, *next_stage)?;
                            *next_stage = next_stage
                                .checked_add(1)
                                .ok_or_else(|| error("answer stage index exhausted"))?;
                            if *next_stage == selection.plan.stages().len() {
                                SubmissionStage::Submitted
                            } else {
                                retained.bytes = Arc::from(
                                    selected_answer_stage(&selection.plan, *next_stage)?.bytes(),
                                );
                                SubmissionStage::Partial
                            }
                        }
                    };
                } else if *count > 0 {
                    retained.stage = SubmissionStage::Partial;
                }
            }
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => {
                if retained.stage == SubmissionStage::ZeroBytes {
                    retained.stage = SubmissionStage::Unknown;
                }
            }
        }
        self.permit = GuardPermit::ResultRecorded;
        Ok(())
    }
}

impl Drop for InputGuard<'_> {
    fn drop(&mut self) {
        if let Some(retained) = self.retained.as_mut() {
            if self.permit == GuardPermit::IoPossible
                && retained.stage == SubmissionStage::ZeroBytes
            {
                retained.stage = SubmissionStage::Unknown;
            }
            retained.leased = false;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteValidation {
    Ready,
    Contended,
}

impl InputCoordinator {
    pub(super) fn inspect_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        live: (&ObservationController, &mut Transport),
        deadline: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let (observation, transport) = live;
        let interaction_id = match &request.operation {
            InputControlOperation::PermissionInspect(input) => Some(input.interaction_id),
            _ => None,
        };
        let result =
            (|| {
                let deadline = inherited_database_deadline(conn)
                    .map_or(deadline, |bound| deadline.min(bound.instant()));
                check_deadline(Instant::now(), deadline)?;
                ensure(
                    request.protocol_version == super::CONTROL_PROTOCOL_VERSION,
                    "input control protocol version mismatch",
                )?;
                request.identity.validate(&self.run)?;
                let InputControlOperation::PermissionInspect(input) = request.operation else {
                    return Err(error("permission inspection operation required"));
                };
                let view = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction().map_err(db_err)?;
                    self.validate_live(&tx, run)?;
                    let view = inspection_association(&tx, &self.run, &input)?;
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit().map_err(db_err)?;
                    Ok(view)
                })?;
                let interaction = view
                    .interaction
                    .expect("validated current permission interaction");
                let message_id = view.event.message_id.expect("validated inbox message");
                let evidence = observation
                    .current_evidence()
                    .ok_or_else(|| error("current inspection observation unavailable"))?
                    .clone();
                let missing_evidence = inspection_missing_evidence(&evidence)?;
                let fresh = match transport.take_input_drain() {
                    Some(proof) => {
                        transport.validate_observation_drain(&proof, deadline)?;
                        true
                    }
                    None => false,
                };
                let observed = observation.current_request();
                if let Some((current, _)) = &self.current_request
                    && (current.request_generation != observation.request_generation()
                        || current.manual_generation != self.manual_generation
                        || observed.is_none())
                {
                    self.current_request = None;
                }
                let mut supported_decisions = Vec::new();
                let mut answer_profile_version = None;
                let mut answer_rule_id = None;
                let mut request_token = None;
                let mut reason = Some("current executable permission request unavailable".into());
                if fresh && let Some(observed) = observed {
                    ensure(
                        observed.episode_seq == interaction.episode_seq
                            && observed.classification == interaction.classification,
                        "permission inspection durable episode changed",
                    )?;
                    let next = CurrentRequest {
                        request_generation: observed.request_generation,
                        identity: ControlIdentity::from_run(&self.run),
                        interaction_id: interaction.interaction_id,
                        event_id: view.event.event_id,
                        message_id,
                        episode_seq: observed.episode_seq,
                        native_version: observed.native_version.clone(),
                        detector_profile_version: observed.detector_profile_version.clone(),
                        request_rule_id: observed.request_rule_id.clone(),
                        semantic: observed.semantic.clone(),
                        choices: observed.choices.clone(),
                        focus: observed.focus.clone(),
                        native_correlation: observed.native_correlation.clone(),
                        manual_generation: self.manual_generation,
                        output_seq: observed.output_seq,
                        answer_matches: observed.answer_matches.clone(),
                    };
                    if !next.answer_matches.is_empty() {
                        if let Some((current, _)) = &mut self.current_request
                            && current.same_request(&next)
                        {
                            current.output_seq = next.output_seq;
                        } else {
                            self.current_request = None;
                            let mut bytes = [0; 32];
                            self.services.token_source.fill(&mut bytes)?;
                            let token = bytes.iter().map(|b| format!("{b:02x}")).collect();
                            self.current_request = Some((next, token));
                        }
                        let (current, token) = self
                            .current_request
                            .as_ref()
                            .expect("matched current request retained");
                        let first = current.answer_matches[0].plan().capability();
                        answer_profile_version = Some(first.profile_version().to_owned());
                        if current.answer_matches.iter().all(|m| {
                            m.plan().capability().answer_rule_id() == first.answer_rule_id()
                        }) {
                            answer_rule_id = Some(first.answer_rule_id().to_owned());
                        }
                        supported_decisions = current
                            .answer_matches
                            .iter()
                            .map(|m| m.plan().capability().decision())
                            .collect();
                        request_token = Some(token.clone());
                        reason = None;
                    }
                }
                check_deadline(Instant::now(), deadline)?;
                Ok(InputControlReply::Inspection(Box::new(
                    PermissionInspection {
                        event_id: view.event.event_id,
                        message_id,
                        member_id: self.run.member_id,
                        fleet_id: self.run.fleet_id,
                        director_member_id: self.run.director_member_id,
                        run_id: self.run.run_id,
                        owner_epoch: self.run.owner_epoch,
                        interaction_id: interaction.interaction_id,
                        episode_seq: interaction.episode_seq,
                        classification: observation.current_classification(),
                        visibility: observation.status().visibility,
                        backend: self.run.backend.clone(),
                        native_version: self.run.native_version.clone(),
                        profile_version: self.run.profile_version.clone(),
                        answer_profile_version,
                        answer_rule_id,
                        answer_capability: if request_token.is_some() {
                            storage::Coverage::Tested
                        } else {
                            storage::Coverage::Unknown
                        },
                        capability_reason: reason,
                        supported_decisions,
                        request_token,
                        output_seq: transport.snapshot().screen_seq,
                        evidence,
                        missing_evidence,
                    },
                )))
            })();
        result.map_err(|source| {
            self.current_request = None;
            InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id,
                input_id: None,
                operation_key: None,
            }
        })
    }

    pub(crate) fn fence(&mut self, deadline: Instant) -> Result<()> {
        self.current_request = None;
        self.input_fenced = true;
        check_deadline(Instant::now(), deadline)
    }
    fn validate_permission_binding(
        &self,
        selection: &PermissionSelection,
        observation: &ObservationController,
    ) -> Result<()> {
        let (current, token) = self
            .current_request
            .as_ref()
            .ok_or_else(|| error("current permission token unavailable"))?;
        let bound = &selection.binding;
        let observed = observation
            .current_request()
            .ok_or_else(|| error("current permission request unavailable"))?;
        let status = observation.status();
        ensure(
            token == &selection.request.metadata.request_token
                && current.same_request(bound)
                && !self.manual_accounting_failed
                && !self.input_fenced
                && self.manual_generation == bound.manual_generation
                && observation.request_generation() == bound.request_generation
                && observed.request_generation == bound.request_generation
                && observed.episode_seq == bound.episode_seq
                && observed.classification == storage::InteractionClass::PermissionRequest
                && observed.native_version == bound.native_version
                && observed.detector_profile_version == bound.detector_profile_version
                && observed.request_rule_id == bound.request_rule_id
                && observed.semantic == bound.semantic
                && observed.choices == bound.choices
                && observed.focus == bound.focus
                && observed.native_correlation == bound.native_correlation
                && observed.answer_matches.len() == bound.answer_matches.len()
                && observed
                    .answer_matches
                    .iter()
                    .zip(&bound.answer_matches)
                    .all(|(a, b)| a.same_request(b))
                && status.visibility == Visibility::Visible
                && status.eligibility == InputEligibility::Hold(super::GateReason::Interaction)
                && !status.retained_transition
                && !status.retained_draft
                && !status.retained_evidence
                && status.diagnostic.is_none(),
            "permission request token or current plan binding changed",
        )?;
        ensure(
            bound
                .answer_matches
                .iter()
                .any(|m| m.plan() == &selection.plan),
            "selected permission plan changed",
        )?;
        Ok(())
    }

    fn validate_permission_association(
        &self,
        conn: &Connection,
        selection: &PermissionSelection,
    ) -> Result<()> {
        let metadata = &selection.request.metadata;
        let view = inspection_association(
            conn,
            &self.run,
            &PermissionInspectInput {
                interaction_id: metadata.interaction_id,
                event_id: metadata.event_id,
                director_member_id: metadata.director_member_id,
            },
        )?;
        let interaction = view.interaction.expect("validated permission interaction");
        ensure(
            interaction.episode_seq == selection.binding.episode_seq
                && view.event.message_id == Some(selection.binding.message_id)
                && metadata.expected_owner_epoch == self.run.owner_epoch,
            "permission response current association changed",
        )
    }

    fn permission_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let InputControlOperation::PermissionRespond(input) = request.operation else {
            unreachable!("permission response route selects its semantic operation");
        };
        let mut known_id = None;
        let result = (|| {
            let deadline = inherited_database_deadline(conn)
                .map_or(deadline, |bound| deadline.min(bound.instant()));
            check_deadline(Instant::now(), deadline)?;
            super::input_client::validate_respond(&input)?;
            ensure(
                request.protocol_version == super::CONTROL_PROTOCOL_VERSION,
                "input protocol version mismatch",
            )?;
            let mut expected = ControlIdentity::from_run(&self.run);
            expected.owner_epoch = input.expected_owner_epoch;
            ensure(
                request.identity == expected
                    && input.expected_run_id == self.run.run_id
                    && run.run_id == self.run.run_id,
                "permission response fixed identity mismatch",
            )?;
            let history = with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction().map_err(db_err)?;
                ensure(
                    storage::database_identity(&tx)? == self.run.database_identity_json,
                    "input database identity changed",
                )?;
                let retained =
                    super::input_client::response_lookup(&tx, &self.run, &input, &mut known_id)?;
                if let Some(receipt) = &retained {
                    known_id = Some(receipt.input.input_id);
                }
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(db_err)?;
                Ok(retained)
            })?;
            if let Some(receipt) = &history
                && receipt.input.state != storage::InputState::Ready
            {
                return super::input_client::projection(&self.run, receipt.clone())
                    .map(|result| Some(InputControlReply::PermissionResponse(result)));
            }
            request.identity.validate(&self.run)?;
            let (binding, token) = self
                .current_request
                .as_ref()
                .ok_or_else(|| error("current permission token unavailable"))?;
            ensure(
                token == &input.request_token
                    && binding.interaction_id == input.interaction_id
                    && binding.event_id == input.event_id
                    && binding.identity == request.identity,
                "permission response token identity changed",
            )?;
            let plan = binding
                .answer_matches
                .iter()
                .find(|matched| matched.plan().capability().decision() == input.decision)
                .ok_or_else(|| error("selected permission decision is unsupported"))?
                .plan()
                .clone();
            let cap = plan.capability();
            let selection = Arc::new(PermissionSelection {
                request: storage::NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: storage::PermissionResponseMetadata {
                        event_id: input.event_id,
                        interaction_id: input.interaction_id,
                        expected_owner_epoch: input.expected_owner_epoch,
                        request_token: input.request_token.clone(),
                        decision: input.decision,
                        director_member_id: input.director_member_id,
                        director_mode: input.director_mode,
                        decision_source: input.decision_source,
                        mode_evidence_ref: input.mode_evidence_ref.clone(),
                        decision_evidence_ref: input.decision_evidence_ref.clone(),
                        answer_profile_version: cap.profile_version().into(),
                        answer_rule_id: cap.answer_rule_id().into(),
                    },
                },
                binding: binding.clone(),
                plan,
            });
            if let Some(receipt) = &history {
                ensure(
                    receipt.response.metadata == selection.request.metadata,
                    "ready permission metadata changed",
                )?;
            }
            self.validate_permission_binding(&selection, observation)?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction().map_err(db_err)?;
                self.validate_live(&tx, run)?;
                self.validate_permission_association(&tx, &selection)?;
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(db_err)
            })?;
            if self.pending.is_some() {
                return Ok(Some(InputControlReply::PermissionResponse(
                    self.permission_result(
                        &selection.request.metadata,
                        SubmissionResult {
                            status: SubmissionStatus::Held,
                            input_id: None,
                            stage: SubmissionStage::ZeroBytes,
                            output_seq: None,
                            reason: Some("another programmatic input is pending".into()),
                        },
                    ),
                )));
            }
            ensure(
                history
                    .as_ref()
                    .is_some_and(|r| self.operations.contains_key(&r.input.input_id))
                    || self.operations.len() < 4096,
                "input receipt cache is at capacity",
            )?;
            let bytes = Arc::from(selected_answer_stage(&selection.plan, 0)?.bytes());
            let digest =
                storage::permission_response_digest(self.run.run_id, &selection.request.metadata)?;
            self.pending = Some(PendingInput {
                reservation: if history.is_some() {
                    PendingReservation::HistoricalReady
                } else {
                    PendingReservation::Unreserved
                },
                request: storage::NewInput {
                    operation_key: input.operation_key.clone(),
                    kind: storage::InputKind::PermissionResponse,
                    payload_digest: digest,
                    message_id: None,
                },
                bytes,
                permission: Some(selection),
                ready: history,
                deadline,
                phase: InputPhase::AwaitReservation,
                input_id: known_id,
            });
            Ok(None)
        })();
        result.map_err(|source| InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: Some(input.interaction_id),
            input_id: known_id,
            operation_key: Some(input.operation_key),
        })
    }

    fn permission_result(
        &self,
        metadata: &storage::PermissionResponseMetadata,
        submission: SubmissionResult,
    ) -> PermissionResponseResult {
        PermissionResponseResult {
            submission,
            member_id: self.run.member_id,
            run_id: self.run.run_id,
            owner_epoch: metadata.expected_owner_epoch,
            interaction_id: metadata.interaction_id,
            event_id: metadata.event_id,
            decision: metadata.decision,
            decision_source: metadata.decision_source,
        }
    }

    fn failure(&self, source: CafleetError) -> InputFailure {
        InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: self.pending.as_ref().and_then(|pending| {
                pending
                    .permission
                    .as_ref()
                    .map(|selection| selection.request.metadata.interaction_id)
            }),
            input_id: self.pending.as_ref().and_then(|pending| pending.input_id),
            operation_key: self
                .pending
                .as_ref()
                .map(|pending| pending.request.operation_key.clone()),
        }
    }

    fn effective_deadline(&self, conn: &Connection, deadline: Instant) -> Result<Instant> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| error("input writer is absent"))?;
        let mut effective = deadline.min(pending.deadline);
        if let Some(inherited) = inherited_database_deadline(conn) {
            effective = effective.min(inherited.instant());
        }
        check_deadline(Instant::now(), effective)?;
        Ok(effective)
    }

    fn validate_live(&self, conn: &Connection, expected: &RunRecord) -> Result<()> {
        ensure(
            !self.manual_accounting_failed && !self.input_fenced,
            "manual accounting failure fenced input",
        )?;
        ensure(
            stable_run_identity(expected) == stable_run_identity(&self.run),
            "input coordinator run identity changed",
        )?;
        let current = storage::validate_owner(conn, &self.run.owner())?;
        ensure(
            stable_run_identity(&current) == stable_run_identity(&self.run)
                && current.status == RunStatus::Running
                && current.owner_kind == OwnerKind::Runner
                && current.native_version == self.run.native_version
                && current.profile_version == self.run.profile_version,
            "input coordinator current run changed",
        )?;
        ensure(
            crate::broker::active_member_fleet(conn, self.run.member_id)?
                == Some(self.run.fleet_id),
            "input member is inactive",
        )?;
        let member = crate::broker::get_member(conn, self.run.member_id, self.run.fleet_id)?
            .ok_or_else(|| error("input member is absent"))?;
        ensure(
            member.kind == MemberKind::Member && member.placement.as_ref() == Some(&self.placement),
            "input member placement changed",
        )?;
        self.lock.validate(conn, &current.owner())
    }

    fn validate_receipt_owner(&self, conn: &Connection, run: &RunRecord) -> Result<()> {
        ensure(
            stable_run_identity(run) == stable_run_identity(&self.run),
            "receipt caller run identity changed",
        )?;
        let current = storage::validate_owner(conn, &self.run.owner())?;
        ensure(
            stable_run_identity(&current) == stable_run_identity(&self.run),
            "receipt current run identity changed",
        )?;
        self.lock.validate(conn, &current.owner())
    }

    fn receipt_guard_snapshot(
        &self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        deadline: Instant,
    ) -> Result<ReceiptGuardBinding> {
        check_deadline(Instant::now(), deadline)?;
        self.validate_receipt_owner(conn, run)?;
        let receipt = storage::get_permission_response(conn, expected.input.input_id)?;
        ensure(receipt == *expected, "permission receipt snapshot changed")?;
        self.validate_receipt_cache(&receipt.input)?;
        let zero_known = if let Some(retained) = self.operations.get(&receipt.input.input_id) {
            ensure(
                !retained.leased && !retained.attempt_available,
                "receipt requires an unleased consumed attempt",
            )?;
            retained.stage == SubmissionStage::ZeroBytes && retained.written_bytes == 0
        } else {
            receipt.input.state == storage::InputState::Ready
        };
        Ok(ReceiptGuardBinding {
            owner: self.run.owner(),
            receipt,
            zero_known,
            rearm: None,
        })
    }

    fn acquire_receipt_guard<'a>(
        &'a mut self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        deadline: Instant,
    ) -> Result<InputGuard<'a>> {
        ensure(
            !conn.is_autocommit(),
            "receipt exclusion requires its ownership transaction",
        )?;
        ensure(
            self.pending.is_none(),
            "receipt exclusion requires no active writer",
        )?;
        let deadline = inherited_database_deadline(conn)
            .map_or(deadline, |bound| deadline.min(bound.instant()));
        let binding = self.receipt_guard_snapshot(conn, run, expected, deadline)?;
        Ok(InputGuard {
            retained: None,
            proof: None,
            binding: Some(binding),
            deadline,
            permit: GuardPermit::Rejected,
            exclusion: std::marker::PhantomData,
        })
    }

    fn acquire_rearm_guard<'a>(
        &'a mut self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        selection: &Arc<PermissionSelection>,
        live: (&mut Transport, &ObservationController),
        deadline: Instant,
    ) -> Result<Option<InputGuard<'a>>> {
        let (transport, observation) = live;
        self.validate_live(conn, run)?;
        self.validate_permission_binding(selection, observation)?;
        self.validate_permission_association(conn, selection)?;
        ensure(
            self.pending.as_ref().is_some_and(|p| {
                p.phase == InputPhase::AwaitReservation
                    && p.input_id == Some(expected.input.input_id)
            }) && expected.input.state == storage::InputState::Ready
                && expected.response.metadata == selection.request.metadata,
            "exact pending ready rearm required",
        )?;
        let mut binding = self.receipt_guard_snapshot(conn, run, expected, deadline)?;
        ensure(binding.zero_known, "ready rearm requires actual zero bytes")?;
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        transport.validate_observation_drain(&proof, deadline)?;
        let InputDrain::Quiescent {
            human_generation, ..
        } = proof.evidence()
        else {
            unreachable!("opaque drain is quiescent")
        };
        ensure(
            human_generation == self.manual_generation && !self.manual_accounting_failed,
            "permission manual generation changed",
        )?;
        binding.rearm = Some(selection.clone());
        Ok(Some(InputGuard {
            retained: None,
            proof: Some(proof),
            binding: Some(binding),
            deadline,
            permit: GuardPermit::Rejected,
            exclusion: std::marker::PhantomData,
        }))
    }

    fn validate_receipt_cache(&self, input: &storage::InputOperation) -> Result<()> {
        ensure(
            input.run_id == self.run.run_id,
            "receipt for this run required",
        )?;
        if let Some(retained) = self.operations.get(&input.input_id) {
            ensure(
                retained.operation.run_id == input.run_id
                    && retained.operation.operation_key == input.operation_key
                    && retained.operation.kind == input.kind
                    && retained.operation.payload_digest == input.payload_digest
                    && retained.operation.message_id == input.message_id
                    && retained.operation.permission_response == input.permission_response,
                "receipt cache identity changed",
            )?;
        }
        Ok(())
    }

    fn receipt_projection(&self, operation: &storage::InputOperation) -> Result<SubmissionResult> {
        self.validate_receipt_cache(operation)?;
        let mut result = self.result(operation);
        if self.operations.contains_key(&operation.input_id) {
            let actual = self.retained_checkpoint(operation.input_id)?;
            result.stage = actual.stage;
            result.output_seq = Some(actual.output_seq);
        } else if operation.state == storage::InputState::Settled
            && operation.disposition != Some(storage::InputDisposition::Submitted)
        {
            result.stage = SubmissionStage::Unknown;
        }
        Ok(result)
    }

    fn receipt_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        action: &super::ReceiptAction,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        use super::{ReceiptAction, ReceiptReply};
        if let ReceiptAction::Retire { input_id } = action {
            return self.retire_received(conn, run, *input_id, deadline);
        }
        let deadline = inherited_database_deadline(conn)
            .map_or(deadline, |inherited| deadline.min(inherited.instant()));
        check_deadline(Instant::now(), deadline)?;
        let mut input_id = None;
        let mut operation_key = None;
        let result = (|| match action {
            ReceiptAction::Query { operation_key: key } => {
                operation_key = Some(key.clone());
                validate_operation_key(key)?;
                with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction().map_err(db_err)?;
                    self.validate_receipt_owner(&tx, run)?;
                    let reply = if let Some(input) = storage::find_input(&tx, self.run.run_id, key)?
                    {
                        input_id = Some(input.input_id);
                        InputControlReply::Receipt(ReceiptReply::Present {
                            input_id: input.input_id,
                            submission: self.receipt_projection(&input)?,
                        })
                    } else {
                        ensure(
                            !self
                                .operations
                                .values()
                                .any(|retained| retained.operation.operation_key == *key),
                            "retained input has no durable reservation",
                        )?;
                        InputControlReply::Receipt(ReceiptReply::Missing)
                    };
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit().map_err(db_err)?;
                    Ok(reply)
                })
            }
            ReceiptAction::Reconcile {
                input_id: id,
                expected,
                disposition,
                evidence_ref,
            } => {
                input_id = Some(*id);
                ensure(*id > 0, "positive reconciliation input ID required")?;
                ensure(
                    !evidence_ref.trim().is_empty(),
                    "nonempty reconciliation evidence required",
                )?;
                let settled = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .map_err(db_err)?;
                    self.validate_receipt_owner(&tx, run)?;
                    let input = storage::get_input(&tx, *id)?;
                    operation_key = Some(input.operation_key.clone());
                    self.validate_receipt_cache(&input)?;
                    ensure(
                        self.pending.is_none(),
                        "reconciliation requires no active input writer",
                    )?;
                    if let Some(retained) = self.operations.get(id) {
                        ensure(
                            !retained.leased && !retained.attempt_available,
                            "reconciliation requires an unleased consumed attempt",
                        )?;
                    }
                    ensure(input.state == *expected, "input state changed")?;
                    let settled = if input.state == storage::InputState::Settled {
                        ensure(
                            input.disposition == Some(*disposition),
                            "settled input disposition changed",
                        )?;
                        input
                    } else {
                        if input.kind == storage::InputKind::PermissionResponse {
                            let owner = self.run.owner();
                            let receipt = storage::get_permission_response(&tx, *id)?;
                            let guard = self.acquire_receipt_guard(&tx, run, &receipt, deadline)?;
                            let proof = if *disposition == storage::InputDisposition::NotSubmitted
                                && guard
                                    .binding
                                    .as_ref()
                                    .expect("receipt guard has its binding")
                                    .zero_known
                            {
                                Some(guard.zero_byte_retirement(
                                    &owner,
                                    *id,
                                    receipt.response.metadata.interaction_id,
                                )?)
                            } else {
                                None
                            };
                            storage::settle_permission_response_in_transaction(
                                &tx,
                                &owner,
                                &input,
                                *disposition,
                                evidence_ref,
                                proof.as_ref(),
                                deadline,
                            )?
                            .input
                        } else {
                            storage::settle_input_in_transaction(
                                &tx,
                                &self.run.owner(),
                                *id,
                                *expected,
                                *disposition,
                                evidence_ref,
                            )?
                        }
                    };
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit().map_err(db_err)?;
                    Ok(settled)
                })?;
                if let Some(retained) = self.operations.get_mut(id) {
                    retained.operation = settled;
                }
                check_deadline(Instant::now(), deadline)?;
                Ok(InputControlReply::Receipt(ReceiptReply::Settled {
                    input_id: *id,
                }))
            }
            ReceiptAction::Retire { .. } => {
                unreachable!("retirement handled before receipt transaction")
            }
        })();
        result.map_err(|source| {
            CafleetError::from(InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id,
                operation_key,
            })
        })
    }

    fn retire_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        input_id: i64,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        let deadline = inherited_database_deadline(conn)
            .map_or(deadline, |inherited| deadline.min(inherited.instant()));
        check_deadline(Instant::now(), deadline)?;
        let mut operation_key = None;
        let result = (|| {
            ensure(input_id > 0, "positive retirement input ID required")?;
            ensure(
                stable_run_identity(run) == stable_run_identity(&self.run),
                "retirement caller run identity changed",
            )?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction().map_err(db_err)?;
                let current = storage::validate_owner(&tx, &self.run.owner())?;
                ensure(
                    stable_run_identity(&current) == stable_run_identity(&self.run),
                    "retirement current run identity changed",
                )?;
                self.lock.validate(&tx, &current.owner())?;
                let input = storage::get_input(&tx, input_id)?;
                operation_key = Some(input.operation_key.clone());
                self.validate_receipt_cache(&input)?;
                ensure(
                    input.run_id == self.run.run_id && input.state == storage::InputState::Settled,
                    "retirement requires this run's settled input",
                )?;
                ensure(
                    self.pending.is_none(),
                    "retirement requires no active input writer",
                )?;
                if let Some(retained) = self.operations.get(&input_id) {
                    ensure(
                        !retained.leased && !retained.attempt_available,
                        "retirement requires an unleased consumed attempt",
                    )?;
                    ensure(
                        retained.operation.run_id == input.run_id
                            && retained.operation.operation_key == input.operation_key
                            && retained.operation.kind == input.kind
                            && retained.operation.payload_digest == input.payload_digest
                            && retained.operation.message_id == input.message_id
                            && retained.operation.permission_response == input.permission_response,
                        "retirement cache identity changed",
                    )?;
                }
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(db_err)
            })?;
            check_deadline(Instant::now(), deadline)?;
            self.operations.remove(&input_id);
            Ok(InputControlReply::Receipt(super::ReceiptReply::Retired {
                input_id,
            }))
        })();
        result.map_err(|source| {
            CafleetError::from(InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id: Some(input_id),
                operation_key,
            })
        })
    }

    fn composer_eligible(observation: &ObservationController) -> bool {
        let status = observation.status();
        status.eligibility == InputEligibility::ComposerEmpty
            && status.visibility == Visibility::NotVisible
            && !status.retained_transition
            && !status.retained_draft
            && !status.retained_evidence
            && status.diagnostic.is_none()
    }

    fn result(&self, operation: &storage::InputOperation) -> SubmissionResult {
        use storage::{InputDisposition, InputState};
        let (status, mut stage, reason) = match (operation.state, operation.disposition) {
            (InputState::Accepted, _)
            | (InputState::Settled, Some(InputDisposition::Submitted)) => {
                (SubmissionStatus::Accepted, SubmissionStage::Submitted, None)
            }
            (InputState::InProgress | InputState::Uncertain, _) => {
                (SubmissionStatus::Uncertain, SubmissionStage::Unknown, None)
            }
            (InputState::Ready, _) => (
                SubmissionStatus::Held,
                SubmissionStage::ZeroBytes,
                Some("input requires live proven-zero revalidation".into()),
            ),
            (InputState::Settled, _) => (
                SubmissionStatus::Rejected,
                SubmissionStage::ZeroBytes,
                Some("input operation key is consumed".into()),
            ),
        };
        let retained = self.operations.get(&operation.input_id).map(|_| {
            self.retained_checkpoint(operation.input_id)
                .expect("present input cache supplies its checkpoint")
        });
        if matches!(
            operation.state,
            InputState::InProgress | InputState::Uncertain
        ) && let Some(retained) = retained.as_ref()
            && retained.stage != SubmissionStage::ZeroBytes
        {
            stage = retained.stage;
        }
        SubmissionResult {
            status,
            input_id: retained
                .as_ref()
                .map_or(Some(operation.input_id), |point| point.input_id),
            stage,
            output_seq: retained.map(|point| point.output_seq),
            reason,
        }
    }

    #[cfg(test)]
    pub(crate) fn handle(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let deadline = check_deadline(Instant::now(), deadline)
            .and_then(|()| {
                receive_input_expiration(
                    request.expires_at_mono_ns,
                    Instant::now(),
                    Some(deadline),
                    &mut RealInputDeadlineClock,
                )
            })
            .map_err(|source| InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id: None,
                operation_key: match &request.operation {
                    InputControlOperation::SubmitPrompt(input)
                    | InputControlOperation::SubmitExec(input) => Some(input.operation_key.clone()),
                    _ => None,
                },
            })?;
        self.handle_received(conn, run, request, observation, deadline)
    }

    pub(super) fn handle_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        if matches!(
            &request.operation,
            InputControlOperation::PermissionRespond(_)
        ) {
            return self.permission_received(conn, run, request, observation, deadline);
        }
        let mut known_input_id = None;
        let mut known_key = None;
        let result = (|| {
            check_deadline(Instant::now(), deadline)?;
            ensure(
                request.protocol_version == super::CONTROL_PROTOCOL_VERSION,
                "input protocol version mismatch",
            )?;
            let mut expected = ControlIdentity::from_run(&self.run);
            ensure(
                request.identity.owner_epoch > 0,
                "positive input owner epoch required",
            )?;
            expected.owner_epoch = request.identity.owner_epoch;
            ensure(
                request.identity == expected && run.run_id == self.run.run_id,
                "input request fixed identity mismatch",
            )?;
            if let InputControlOperation::Receipt(action) = &request.operation {
                match action {
                    super::ReceiptAction::Query { operation_key } => {
                        known_key = Some(operation_key.clone())
                    }
                    super::ReceiptAction::Retire { input_id }
                    | super::ReceiptAction::Reconcile { input_id, .. } => {
                        known_input_id = Some(*input_id)
                    }
                }
                request.identity.validate(&self.run)?;
                return self.receipt_received(conn, run, action, deadline).map(Some);
            }
            let (input, composer_operation) = match request.operation {
                InputControlOperation::SubmitPrompt(input) => (input, ComposerOperation::Prompt),
                InputControlOperation::SubmitExec(input) => (input, ComposerOperation::Exec),
                _ => return Err(error("composer submission operation required")),
            };
            known_key = Some(input.operation_key.clone());
            ensure(
                matches!(
                    (composer_operation, input.input_kind),
                    (
                        ComposerOperation::Prompt,
                        storage::InputKind::Prompt | storage::InputKind::Preview
                    ) | (ComposerOperation::Exec, storage::InputKind::Exec)
                ),
                "composer submission operation and kind mismatch",
            )?;
            if input.input_kind == storage::InputKind::Preview {
                let message_id = input
                    .message_id
                    .ok_or_else(|| error("preview message required"))?;
                ensure(
                    message_id > 0
                        && input.operation_key
                            == format!("preview:{}:{message_id}", self.run.run_id),
                    "preview operation key must bind its run and message",
                )?;
            } else {
                ensure(
                    input.message_id.is_none(),
                    "explicit input has no preview message",
                )?;
            }
            validate_operation_key(&input.operation_key)?;
            ensure(
                crate::input::valid_input(&input.literal),
                "composer submission requires one nonempty line without control characters",
            )?;
            let digest = Sha256::digest(canonical_json(&serde_json::json!({"run_id":self.run.run_id,"kind":input.input_kind.as_str(),"message_id":input.message_id,"literal":input.literal})).as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            let retained = with_database_deadline(conn, deadline, |conn| {
                ensure(
                    storage::database_identity(conn)? == self.run.database_identity_json,
                    "input database identity changed",
                )?;
                storage::find_input(conn, self.run.run_id, &input.operation_key)
            })?;
            if let Some(retained) = retained {
                known_input_id = Some(retained.input_id);
                ensure(
                    retained.kind == input.input_kind
                        && retained.payload_digest == digest
                        && retained.message_id == input.message_id,
                    "conflicting input operation key reuse",
                )?;
                return Ok(Some(InputControlReply::Submission(self.result(&retained))));
            }
            request.identity.validate(&self.run)?;
            if self.pending.is_some() || !Self::composer_eligible(observation) {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("ordinary composer input is held".into()),
                })));
            }
            ensure(
                self.operations.len() < 4096,
                "input receipt cache is at capacity",
            )?;
            with_database_deadline(conn, deadline, |conn| self.validate_live(conn, run))?;
            let Some(native) = self.run.native_version.as_deref() else {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("matching executable composer plan is unavailable".into()),
                })));
            };
            let backend =
                coding_agent(&self.run.backend).ok_or_else(|| error("unknown input backend"))?;
            let Some(plan) =
                self.services
                    .catalog
                    .composer_plan(backend, native, composer_operation)?
            else {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("matching executable composer plan is unavailable".into()),
                })));
            };
            ensure(
                plan.backend() == backend
                    && plan.native_version() == native
                    && Some(plan.profile_version()) == self.run.profile_version.as_deref()
                    && !plan.rule_ids().is_empty()
                    && !plan.fixture_ids().is_empty(),
                "composer plan provenance changed",
            )?;
            let bytes = Arc::from(plan.encode(&input.literal)?);
            self.pending = Some(PendingInput {
                reservation: PendingReservation::Unreserved,
                request: storage::NewInput {
                    operation_key: input.operation_key,
                    kind: input.input_kind,
                    payload_digest: digest,
                    message_id: input.message_id,
                },
                bytes,
                permission: None,
                ready: None,
                deadline,
                phase: InputPhase::AwaitReservation,
                input_id: None,
            });
            Ok(None)
        })();
        result.map_err(|source| InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: None,
            input_id: known_input_id,
            operation_key: known_key,
        })
    }

    fn retained_checkpoint(&self, input_id: i64) -> Result<InputCheckpoint> {
        let retained = self
            .operations
            .get(&input_id)
            .ok_or_else(|| error("retained input operation is absent"))?;
        Ok(InputCheckpoint {
            input_id: Some(input_id),
            stage: retained.stage,
            written_bytes: retained.written_bytes,
            output_seq: retained.output_seq,
        })
    }

    fn checkpoint(
        &mut self,
        input_id: i64,
        boundary: InputBoundary,
        deadline: Instant,
    ) -> Result<()> {
        let checkpoint = self.retained_checkpoint(input_id)?;
        let result = self
            .services
            .hooks
            .checkpoint(boundary, &checkpoint, deadline)
            .and_then(|()| check_deadline(Instant::now(), deadline));
        if result.is_err()
            && let Some(pending) = &mut self.pending
        {
            pending.phase = InputPhase::Blocked;
        }
        result
    }

    fn fresh_drain(
        &mut self,
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<Option<u64>> {
        if let Some(permission) = self.pending.as_ref().and_then(|p| p.permission.as_ref()) {
            self.validate_permission_binding(permission, observation)?;
        }
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        if let Some(permission) = self.pending.as_ref().and_then(|p| p.permission.as_ref()) {
            self.validate_permission_binding(permission, observation)?;
            transport.validate_observation_drain(&proof, deadline)?;
        } else {
            transport.validate_input_drain(&proof, deadline)?;
            if !Self::composer_eligible(observation) {
                return Ok(None);
            }
        }
        let InputDrain::Quiescent {
            human_generation,
            output_seq,
        } = proof.evidence()
        else {
            unreachable!("opaque drain is quiescent");
        };
        ensure(
            human_generation == self.manual_generation && !self.manual_accounting_failed,
            "input manual generation changed",
        )?;
        Ok(Some(output_seq))
    }

    fn validate_write_now(
        &self,
        conn: &mut Connection,
        run: &RunRecord,
        operation: &PendingInput,
        deadline: Instant,
    ) -> std::result::Result<WriteValidation, InputFailure> {
        let result = (|| {
            let effective = self.effective_deadline(conn, deadline)?;
            ensure(
                conn.is_autocommit(),
                "final input validation requires autocommit",
            )?;
            let result = with_database_budget(conn, Deadline::until(effective), |conn| {
                conn.busy_handler(Some(|_| false)).map_err(db_err)?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_err)?;
                self.validate_live(&tx, run)?;
                let id = operation
                    .input_id
                    .ok_or_else(|| error("input reservation is absent"))?;
                let retained = self
                    .operations
                    .get(&id)
                    .ok_or_else(|| error("retained input reservation is absent"))?;
                let current = storage::get_input(&tx, id)?;
                ensure(
                    current == retained.operation
                        && current.state == storage::InputState::InProgress
                        && current.run_id == self.run.run_id
                        && current.operation_key == operation.request.operation_key
                        && current.payload_digest == operation.request.payload_digest
                        && current.kind == operation.request.kind
                        && current.message_id == operation.request.message_id,
                    "input reservation changed before write",
                )?;
                if let Some(selection) = &operation.permission {
                    self.validate_permission_association(&tx, selection)?;
                    let receipt = storage::get_permission_response(&tx, id)?;
                    ensure(
                        receipt.input == current
                            && receipt.response.metadata == selection.request.metadata
                            && receipt.claim.as_ref().is_some_and(|claim| {
                                claim.input_id == id
                                    && claim.interaction_id == selection.binding.interaction_id
                            }),
                        "permission audit or claim changed before write",
                    )?;
                }
                check_deadline(Instant::now(), effective)?;
                tx.rollback().map_err(db_err)
            });
            let effective = inherited_database_deadline(conn)
                .map_or(effective, |bound| effective.min(bound.instant()));
            match result {
                Ok(()) => {
                    check_deadline(Instant::now(), effective)?;
                    Ok(WriteValidation::Ready)
                }
                Err(CafleetError::DatabaseBusy(_)) => {
                    check_deadline(Instant::now(), effective)?;
                    Ok(WriteValidation::Contended)
                }
                Err(cause) => Err(cause),
            }
        })();
        result.map_err(|source| self.failure(source))
    }

    fn acquire_write_guard<'a>(
        &'a mut self,
        conn: &mut Connection,
        run: &RunRecord,
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputGuard<'a>>, InputFailure> {
        let result = (|| {
            let Some(pending) = self.pending.as_ref() else {
                return Ok(None);
            };
            if pending.phase != InputPhase::Armed {
                return Ok(None);
            }
            let effective = self.effective_deadline(conn, deadline)?;
            let Some(proof) = transport.take_input_drain() else {
                return Ok(None);
            };
            if let Some(permission) = &pending.permission {
                self.validate_permission_binding(permission, observation)?;
                transport.validate_observation_drain(&proof, effective)?;
                let retained = self
                    .operations
                    .get(
                        &pending
                            .input_id
                            .ok_or_else(|| error("permission reservation absent"))?,
                    )
                    .ok_or_else(|| error("retained permission selection absent"))?;
                let RetainedSelection::Permission {
                    selection,
                    next_stage,
                } = &retained.selection
                else {
                    return Err(error("permission stage target changed"));
                };
                ensure(
                    Arc::ptr_eq(selection, permission),
                    "permission retained selection changed",
                )?;
                let stage = selected_answer_stage(&selection.plan, *next_stage)?;
                ensure(
                    stage.required_rule_id() == selection.binding.request_rule_id
                        && stage.required_choice_id() == selection.plan.choice_id(),
                    "permission stage rule or choice changed",
                )?;
            } else {
                transport.validate_input_drain(&proof, effective)?;
                if !Self::composer_eligible(observation) {
                    return Ok(None);
                }
            }
            let InputDrain::Quiescent {
                human_generation,
                output_seq,
            } = proof.evidence()
            else {
                unreachable!("opaque drain is quiescent");
            };
            ensure(
                human_generation == self.manual_generation && !self.manual_accounting_failed,
                "input manual generation changed",
            )?;
            let id = pending
                .input_id
                .ok_or_else(|| error("armed input has no reservation"))?;
            let retained = self
                .operations
                .get(&id)
                .ok_or_else(|| error("armed input reservation is absent"))?;
            ensure(
                retained.attempt_available && !retained.leased,
                "input attempt permit is consumed",
            )?;
            if self
                .validate_write_now(conn, run, pending, effective)
                .map_err(|failure| failure.source)?
                == WriteValidation::Contended
            {
                return Ok(None);
            }
            let effective = inherited_database_deadline(conn)
                .map_or(effective, |bound| effective.min(bound.instant()));
            check_deadline(Instant::now(), effective)?;
            Ok(Some((id, proof, effective, output_seq)))
        })()
        .map_err(|source| self.failure(source))?;
        let Some((id, proof, deadline, output_seq)) = result else {
            return Ok(None);
        };
        let retained = self
            .operations
            .get_mut(&id)
            .expect("validated input remains retained");
        retained.output_seq = output_seq;
        retained.attempt_available = false;
        retained.leased = true;
        Ok(Some(InputGuard {
            retained: Some(retained),
            proof: Some(proof),
            binding: None,
            deadline,
            permit: GuardPermit::Fresh,
            exclusion: std::marker::PhantomData,
        }))
    }

    pub(crate) fn advance(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        transport: &mut Transport,
        pty: &mut impl PtyIo,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let mut terminal_bookkeeping = false;
        let result = (|| {
            let Some(pending) = self.pending.as_ref() else {
                return Ok(None);
            };
            let effective = self.effective_deadline(conn, deadline)?;
            let phase = pending.phase;
            if phase == InputPhase::Blocked {
                return Ok(None);
            }
            if phase == InputPhase::AwaitReservation {
                let ready = self
                    .pending
                    .as_ref()
                    .and_then(|p| p.ready.clone().zip(p.permission.clone()));
                let (operation, output_seq) = if let Some((ready, selection)) = ready {
                    let owner = self.run.owner();
                    let Some(guard) = with_database_deadline(conn, effective, |conn| {
                        self.acquire_rearm_guard(
                            conn,
                            run,
                            &ready,
                            &selection,
                            (transport, observation),
                            effective,
                        )
                    })?
                    else {
                        return Ok(None);
                    };
                    let InputDrain::Quiescent { output_seq, .. } = guard
                        .proof
                        .as_ref()
                        .expect("rearm guard retains its drain")
                        .evidence()
                    else {
                        unreachable!("opaque drain is quiescent")
                    };
                    let proof = guard.zero_byte_retirement(
                        &owner,
                        ready.input.input_id,
                        ready.response.metadata.interaction_id,
                    )?;
                    let rearmed = storage::rearm_permission_response(
                        conn,
                        &owner,
                        &selection.request,
                        &proof,
                        effective,
                    )?;
                    (rearmed.input, output_seq)
                } else {
                    let Some(output_seq) = self.fresh_drain(transport, observation, effective)?
                    else {
                        return Ok(None);
                    };
                    let pending = self.pending.as_ref().expect("retained pending input");
                    let operation = if let Some(selection) = &pending.permission {
                        let reservation = with_database_deadline(conn, effective, |conn| {
                            self.validate_live(conn, run)?;
                            self.validate_permission_association(conn, selection)?;
                            storage::reserve_permission_response(
                                conn,
                                &self.run.owner(),
                                &selection.request,
                                Some(selection.plan.capability()),
                                effective,
                            )
                        })?;
                        match reservation {
                            storage::PermissionReservation::Reserved(receipt) => receipt.input,
                            storage::PermissionReservation::Retained(receipt) => {
                                let reply = InputControlReply::PermissionResponse(
                                    super::input_client::projection(&self.run, receipt)?,
                                );
                                self.pending = None;
                                return Ok(Some(reply));
                            }
                        }
                    } else {
                        let reservation = with_database_deadline(conn, effective, |conn| {
                            self.validate_live(conn, run)?;
                            storage::reserve_input_with_provenance(
                                conn,
                                &self.run.owner(),
                                &pending.request,
                            )
                        })?;
                        match reservation {
                            storage::InputReservation::Inserted(operation) => operation,
                            storage::InputReservation::Existing(operation) => {
                                let reply = InputControlReply::Submission(self.result(&operation));
                                self.pending = None;
                                return Ok(Some(reply));
                            }
                        }
                    };
                    (operation, output_seq)
                };
                ensure(
                    operation.state == storage::InputState::InProgress,
                    "ordinary input reservation is not writable",
                )?;
                let id = operation.input_id;
                let pending = self.pending.as_ref().expect("reserved pending input");
                self.operations.insert(
                    id,
                    RetainedInput {
                        operation,
                        bytes: pending.bytes.clone(),
                        selection: match &pending.permission {
                            Some(selection) => RetainedSelection::Permission {
                                selection: selection.clone(),
                                next_stage: 0,
                            },
                            None => RetainedSelection::Composer,
                        },
                        stage: SubmissionStage::ZeroBytes,
                        written_bytes: 0,
                        output_seq,
                        attempt_available: false,
                        leased: false,
                    },
                );
                let pending = self.pending.as_mut().expect("reserved pending input");
                pending.input_id = Some(id);
                pending.reservation = PendingReservation::Installed;
                pending.phase = InputPhase::AwaitBeforeWrite;
                self.checkpoint(id, InputBoundary::Reserved, effective)?;
                return Ok(None);
            }
            let id = self
                .pending
                .as_ref()
                .and_then(|pending| pending.input_id)
                .ok_or_else(|| error("input writer has no reservation"))?;
            if phase == InputPhase::AwaitBeforeWrite {
                let Some(output_seq) = self.fresh_drain(transport, observation, effective)? else {
                    return Ok(None);
                };
                let retained = self
                    .operations
                    .get_mut(&id)
                    .expect("reserved input remains retained");
                retained.output_seq = output_seq;
                retained.attempt_available = true;
                self.pending.as_mut().expect("pending input").phase = InputPhase::Armed;
                self.checkpoint(id, InputBoundary::BeforeWrite, effective)?;
                return Ok(None);
            }
            let bytes = self
                .pending
                .as_ref()
                .expect("armed pending input")
                .bytes
                .clone();
            let Some(mut guard) = self
                .acquire_write_guard(conn, run, transport, observation, effective)
                .map_err(|failure| failure.source)?
            else {
                return Ok(None);
            };
            let before_stage = guard
                .retained
                .as_ref()
                .expect("write guard retains its input")
                .stage;
            let before_count = guard
                .retained
                .as_ref()
                .expect("write guard retains its input")
                .written_bytes;
            let write = transport.write_programmatic(pty, &bytes, &mut guard, effective);
            let attempted = matches!(
                guard.permit,
                GuardPermit::IoPossible | GuardPermit::ResultRecorded
            );
            drop(guard);
            let after = if attempted {
                self.checkpoint(id, InputBoundary::AfterWrite, effective)
            } else {
                Ok(())
            };
            let write =
                combine_manual_progress(write, after).and_then(|outcome| {
                    let point = self.retained_checkpoint(id)?;
                    match &outcome {
                        ProgrammaticWrite::Complete { bytes: count } => ensure(
                            *count == bytes.len()
                                && matches!(
                                    point.stage,
                                    SubmissionStage::Partial | SubmissionStage::Submitted
                                )
                                && point.written_bytes
                                    == before_count
                                        .checked_add(u64::try_from(*count).map_err(|_| {
                                            error("native input count exceeds range")
                                        })?)
                                        .ok_or_else(|| error("native input count exhausted"))?,
                            "complete native result conflicts with retained evidence",
                        )?,
                        ProgrammaticWrite::Partial { bytes: count } => ensure(
                            *count > 0
                                && *count < bytes.len()
                                && point.stage == SubmissionStage::Partial
                                && point.written_bytes
                                    == before_count
                                        .checked_add(u64::try_from(*count).map_err(|_| {
                                            error("native input count exceeds range")
                                        })?)
                                        .ok_or_else(|| error("native input count exhausted"))?,
                            "partial native result conflicts with retained evidence",
                        )?,
                        ProgrammaticWrite::Pending => ensure(
                            point.stage == before_stage && point.written_bytes == before_count,
                            "pending native result conflicts with retained evidence",
                        )?,
                    }
                    Ok(outcome)
                });
            if matches!(&write, Ok(ProgrammaticWrite::Complete { .. }))
                && self
                    .operations
                    .get(&id)
                    .expect("actual stage remains retained")
                    .stage
                    == SubmissionStage::Partial
            {
                let next_bytes = self
                    .operations
                    .get(&id)
                    .expect("next stage retained")
                    .bytes
                    .clone();
                let pending = self.pending.as_mut().expect("pending multistage response");
                ensure(
                    pending.permission.is_some(),
                    "nonfinal stage requires a permission plan",
                )?;
                pending.bytes = next_bytes;
                pending.phase = InputPhase::AwaitBeforeWrite;
                return Ok(None);
            }
            if matches!(&write, Ok(ProgrammaticWrite::Pending)) {
                self.pending
                    .as_mut()
                    .expect("pending zero-byte attempt")
                    .phase = InputPhase::AwaitBeforeWrite;
                return Ok(None);
            }
            let retained = self
                .operations
                .get(&id)
                .expect("actual input outcome retained");
            let next = match retained.stage {
                SubmissionStage::Submitted => storage::InputState::Accepted,
                SubmissionStage::Partial | SubmissionStage::Unknown => {
                    storage::InputState::Uncertain
                }
                SubmissionStage::ZeroBytes => storage::InputState::Ready,
            };
            terminal_bookkeeping = true;
            let persisted = with_database_deadline(conn, effective, |conn| {
                self.validate_live(conn, run)?;
                storage::record_input_state(
                    conn,
                    &self.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    next,
                )
            });
            if let Ok(operation) = &persisted {
                self.operations
                    .get_mut(&id)
                    .expect("input retained")
                    .operation = operation.clone();
            }
            let completed = match (write, persisted) {
                (Ok(_), Ok(operation)) => Ok(operation),
                (Err(primary), Ok(_)) | (Ok(_), Err(primary)) => Err(primary),
                (Err(primary), Err(bookkeeping)) => Err(CafleetError::WithDiagnostics {
                    cause: Box::new(primary),
                    diagnostics: vec![bookkeeping.message().into_owned()],
                }),
            };
            self.pending.as_mut().expect("completed input").phase = InputPhase::Blocked;
            let operation = completed?;
            self.checkpoint(id, InputBoundary::TerminalState, effective)?;
            let pending = self.pending.as_ref().expect("completed input retained");
            let reply = match &pending.permission {
                Some(selection) => InputControlReply::PermissionResponse(
                    self.permission_result(&selection.request.metadata, self.result(&operation)),
                ),
                None => InputControlReply::Submission(self.result(&operation)),
            };
            self.pending = None;
            Ok(Some(reply))
        })();
        result.map_err(|source| {
            let source = if terminal_bookkeeping {
                source
            } else {
                self.retain_permission_failure(conn, run, deadline, source)
            };
            self.failure(source)
        })
    }

    fn retain_permission_failure(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        deadline: Instant,
        source: CafleetError,
    ) -> CafleetError {
        let Some(pending) = self.pending.as_mut() else {
            return source;
        };
        if pending.permission.is_none() {
            return source;
        }
        pending.phase = InputPhase::Blocked;
        let Some(id) = pending.installed_input_id() else {
            return source;
        };
        let retained = self
            .operations
            .get_mut(&id)
            .expect("pending permission receipt retained");
        assert!(!retained.leased, "failed attempt has released its guard");
        retained.attempt_available = false;
        if retained.operation.state != storage::InputState::InProgress {
            return source;
        }
        let next = if retained.stage == SubmissionStage::ZeroBytes {
            storage::InputState::Ready
        } else {
            storage::InputState::Uncertain
        };
        let Ok(effective) = self.effective_deadline(conn, deadline) else {
            return source;
        };
        let persisted = with_database_deadline(conn, effective, |conn| {
            self.validate_live(conn, run)?;
            storage::record_input_state(
                conn,
                &self.run.owner(),
                id,
                storage::InputState::InProgress,
                next,
            )
        });
        match persisted {
            Ok(operation) => {
                self.operations
                    .get_mut(&id)
                    .expect("failed receipt retained")
                    .operation = operation;
                source
            }
            Err(bookkeeping) => CafleetError::WithDiagnostics {
                cause: Box::new(source),
                diagnostics: vec![bookkeeping.message().into_owned()],
            },
        }
    }

    pub(super) fn finish_pending_attempt(&mut self) {
        if let Some(id) = self
            .pending
            .as_ref()
            .and_then(PendingInput::installed_input_id)
        {
            let retained = self
                .operations
                .get_mut(&id)
                .expect("pending input reservation remains retained");
            assert!(
                !retained.leased,
                "completed input has no active guard lease"
            );
            retained.attempt_available = false;
        }
        self.pending = None;
    }

    pub(crate) fn new(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        services: InputServices,
        deadline: Instant,
    ) -> Result<Self> {
        let run = with_database_deadline(conn, deadline, |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(db_err)?;
            let current = storage::validate_owner(&tx, &run.owner())?;
            ensure(
                stable_run_identity(&current) == stable_run_identity(run),
                "input coordinator run identity changed",
            )?;
            ensure(
                current.owner_kind == OwnerKind::Runner && current.status == RunStatus::Running,
                "input coordinator requires a running runner owner",
            )?;
            let process = ProcessIdentity::current()?;
            ensure(
                current.owner_pid == process.pid
                    && current.owner_start_identity == process.start_identity
                    && current.runner_pid == Some(process.pid)
                    && current.runner_start_identity.as_deref()
                        == Some(process.start_identity.as_str()),
                "input coordinator runner generation mismatch",
            )?;
            lock.validate(&tx, &current.owner())?;
            check_deadline(Instant::now(), deadline)?;
            tx.commit().map_err(db_err)?;
            Ok(current)
        })?;
        let placement = with_database_deadline(conn, deadline, |conn| {
            let member = crate::broker::get_member(conn, run.member_id, run.fleet_id)?
                .ok_or_else(|| error("input coordinator member is absent"))?;
            ensure(
                member.kind == MemberKind::Member,
                "input coordinator requires an ordinary member",
            )?;
            member
                .placement
                .ok_or_else(|| error("input coordinator placement is absent"))
        })?;
        Ok(Self {
            placement,
            operations: BTreeMap::new(),
            pending: None,
            run,
            lock,
            services,
            manual_generation: 0,
            manual_accounting_failed: false,
            current_request: None,
            input_fenced: false,
        })
    }

    pub(super) fn progress_with_manual<T>(
        &mut self,
        transport: &mut Transport,
        step: impl FnOnce(&mut Transport) -> Result<T>,
    ) -> std::result::Result<T, InputFailure> {
        let before = transport.human_generation();
        let progress = step(transport);
        let accounting = transport
            .human_generation()
            .checked_sub(before)
            .ok_or_else(|| error("manual input generation decreased"))
            .and_then(|count| {
                usize::try_from(count).map_err(|_| error("manual input delta exceeds count range"))
            })
            .and_then(|count| {
                if count == 0 {
                    Ok(())
                } else {
                    self.observe_manual(count)
                }
            });
        if accounting.is_err() {
            self.manual_accounting_failed = true;
        }
        combine_manual_progress(progress, accounting).map_err(|source| self.failure(source))
    }

    pub(crate) fn observe_manual(&mut self, count: usize) -> Result<()> {
        if count > 0 {
            self.current_request = None;
        }
        let next = u64::try_from(count)
            .map_err(|_| error("manual input count exceeds generation range"))
            .and_then(|count| {
                self.manual_generation
                    .checked_add(count)
                    .ok_or_else(|| error("manual input generation exhausted"))
            });
        match next {
            Ok(next) => {
                self.manual_generation = next;
                Ok(())
            }
            Err(cause) => {
                self.manual_accounting_failed = true;
                Err(cause)
            }
        }
    }
}

fn combine_manual_progress<T>(progress: Result<T>, accounting: Result<()>) -> Result<T> {
    match (progress, accounting) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(cause), Ok(())) | (Ok(_), Err(cause)) => Err(cause),
        (Err(primary), Err(accounting)) => Err(CafleetError::WithDiagnostics {
            cause: Box::new(primary),
            diagnostics: vec![accounting.message().into_owned()],
        }),
    }
}

#[cfg(test)]
mod retirement_contracts {
    use super::contract_tests::ordinary_prompt::support::{Host, with_host, with_host_until};
    use super::*;
    use crate::runtime::runner::{ReceiptAction, ReceiptReply, SubmitInput};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn completed_coordinator_settlement_and_retirement_exceeds_cache_capacity_without_replay() {
        let until = Instant::now() + Duration::from_secs(120);
        with_host_until(until, |h| {
            let run = h.fixture.run.clone();
            let first_key = "prompt:capacity:0";
            let first_request = h.request(first_key);
            let expected_program = "ordinary fixture λ~".as_bytes();
            let evidence = "owned completed coordinator input";
            let mut first_settled = None;
            for index in 0..4097 {
                assert!(
                    Instant::now() < until,
                    "original batch cap at cycle {index}"
                );
                assert_eq!(h.until, until);
                assert!(h.coordinator.operations.is_empty());
                assert!(h.coordinator.pending.is_none());
                let request = h.request(&format!("prompt:capacity:{index}"));
                assert_eq!(request.expires_at_mono_ns, first_request.expires_at_mono_ns);
                assert!(
                    h.coordinator
                        .handle(&mut h.fixture.conn, &run, request, &h.controller, until)
                        .unwrap()
                        .is_none()
                );
                let reply = (0..8)
                    .find_map(|_| {
                        h.drain();
                        h.advance()
                    })
                    .expect("actual completion within finite drained turns");
                let InputControlReply::Submission(accepted) = reply else {
                    panic!("actual Prompt submission reply")
                };
                assert_eq!(accepted.status, SubmissionStatus::Accepted);
                assert_eq!(accepted.stage, SubmissionStage::Submitted);
                let id = accepted.input_id.expect("actual completed input ID");
                assert_eq!(h.coordinator.operations.len(), 1);
                assert!(h.coordinator.pending.is_none());
                let retained = cached(h, id);
                assert_eq!(retained.0.state, storage::InputState::Accepted);
                assert_eq!(retained.1, SubmissionStage::Submitted);
                assert_eq!(retained.2, u64::try_from(expected_program.len()).unwrap());
                assert!(!retained.4 && !retained.5);
                assert_eq!(h.pty.offers.len(), index + 1);
                assert_eq!(h.pty.program.len(), index + 1);
                assert_eq!(h.pty.program.last().unwrap(), expected_program);
                assert!(h.pty.manual.is_empty());
                for boundary in ["reserved", "before", "after", "terminal"] {
                    assert_eq!(h.hooks.count(boundary), 1, "cycle {index}: {boundary}");
                }
                let settled_reply = receipt(
                    h,
                    ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::Accepted,
                        disposition: storage::InputDisposition::Submitted,
                        evidence_ref: evidence.into(),
                    },
                )
                .unwrap();
                assert!(
                    matches!(settled_reply, InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
                );
                let settled = storage::get_input(&h.fixture.conn, id).unwrap();
                assert_eq!(settled.run_id, run.run_id);
                assert_eq!(settled.operation_key, format!("prompt:capacity:{index}"));
                assert_eq!(settled.kind, storage::InputKind::Prompt);
                assert_eq!(settled.state, storage::InputState::Settled);
                assert_eq!(
                    settled.disposition,
                    Some(storage::InputDisposition::Submitted)
                );
                assert_eq!(settled.evidence_ref.as_deref(), Some(evidence));
                assert_eq!(cached(h, id).0, settled);
                assert_retired(
                    receipt(h, ReceiptAction::Retire { input_id: id }).unwrap(),
                    id,
                );
                assert!(h.coordinator.operations.is_empty());
                assert!(h.coordinator.pending.is_none());
                assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
                assert_eq!(h.pty.offers.len(), index + 1);
                if index == 0 {
                    first_settled = Some(settled);
                }
                h.hooks.points.lock().unwrap().clear();
            }
            let (completed, unresolved): (i64, i64) = h.fixture.conn.query_row(
                "SELECT COUNT(*),COUNT(CASE WHEN state IN ('in_progress','accepted','uncertain') THEN 1 END) FROM member_input_operations WHERE run_id=?1",
                [run.run_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!((completed, unresolved), (4097, 0));
            let first = first_settled.expect("first real completed operation retained as history");
            let changes = h.fixture.conn.total_changes();
            let reply = h
                .coordinator
                .handle(
                    &mut h.fixture.conn,
                    &run,
                    first_request,
                    &h.controller,
                    until,
                )
                .unwrap()
                .expect("settled key returns history immediately");
            let InputControlReply::Submission(replayed) = reply else {
                panic!("historical Prompt submission reply")
            };
            assert_eq!(replayed.status, SubmissionStatus::Accepted);
            assert_eq!(replayed.stage, SubmissionStage::Submitted);
            assert_eq!(replayed.input_id, Some(first.input_id));
            assert!(h.coordinator.operations.is_empty());
            assert!(h.coordinator.pending.is_none());
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(
                storage::get_input(&h.fixture.conn, first.input_id).unwrap(),
                first
            );
            assert_eq!(h.pty.offers.len(), 4097);
            assert_eq!(h.pty.program.len(), 4097);
            assert!(h.pty.manual.is_empty());
            assert!(h.hooks.points.lock().unwrap().is_empty());
        });
    }

    fn assert_retired(reply: InputControlReply, expected: i64) {
        assert!(
            matches!(reply, InputControlReply::Receipt(ReceiptReply::Retired { input_id }) if input_id == expected)
        );
    }

    fn receipt(
        h: &mut Host,
        action: ReceiptAction,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let mut request = h.request("unused-receipt-key");
        request.operation = InputControlOperation::Receipt(action);
        h.coordinator
            .handle(
                &mut h.fixture.conn,
                &h.fixture.run,
                request,
                &h.controller,
                h.until,
            )
            .map(|reply| reply.expect("receipt actions reply immediately"))
    }

    fn pending(h: &Host) -> (String, String, Vec<u8>, Instant, InputPhase, Option<i64>) {
        let pending = h
            .coordinator
            .pending
            .as_ref()
            .expect("actual pending writer");
        (
            pending.request.operation_key.clone(),
            pending.request.payload_digest.clone(),
            pending.bytes.to_vec(),
            pending.deadline,
            pending.phase,
            pending.input_id,
        )
    }

    #[test]
    fn live_receipt_query_preserves_the_real_zero_byte_checkpoint_and_pending_writer() {
        with_host(|h| {
            let key = "prompt:live-receipt-query";
            let id = h.arm(key);
            let point = cached(h, id);
            assert!(matches!(point.1, SubmissionStage::ZeroBytes));
            assert!(point.4 && !point.5);
            h.pty
                .child
                .bytes
                .extend(b"\x1b[2J\x1b[HLater unclassified output");
            h.drain();
            assert!(h.transport.snapshot().screen_seq > point.3);
            let writer = pending(h);
            let changes = h.fixture.conn.total_changes();
            let hooks = h.hooks.points.lock().unwrap().len();
            let reply = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .expect("Query projects actual retained facts even while a writer is Armed");
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = reply
            else {
                panic!("actual retained input is Present")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.input_id, Some(id));
            assert_eq!(submission.status, SubmissionStatus::Uncertain);
            assert_eq!(submission.stage, SubmissionStage::ZeroBytes);
            assert_eq!(submission.output_seq, Some(point.3));
            assert!(matches!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: "prompt:absent-receipt".into()
                    }
                )
                .unwrap(),
                InputControlReply::Receipt(ReceiptReply::Missing)
            ));
            let mut stale = h.request("unused-receipt-key");
            stale.operation = InputControlOperation::Receipt(ReceiptAction::Query {
                operation_key: key.into(),
            });
            stale.identity.owner_epoch += 1;
            assert!(
                h.coordinator
                    .handle(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        stale,
                        &h.controller,
                        h.until
                    )
                    .is_err()
            );
            assert!(
                receipt(
                    h,
                    ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::InProgress,
                        disposition: storage::InputDisposition::NotSubmitted,
                        evidence_ref: "fixture:active-writer-is-not-zero-proof".into(),
                    }
                )
                .is_err(),
                "reconciliation preserves the actual Armed writer and its available permit"
            );
            assert!(
                pending(h) == writer,
                "Query preserves exact request, bytes, phase and original deadline"
            );
            assert_eq!(cached(h, id), point);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), point.0);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
            assert!(h.pty.offers.is_empty());
            h.fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2",
                    rusqlite::params!["f".repeat(64), id],
                )
                .unwrap();
            assert!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: key.into()
                    }
                )
                .is_err(),
                "cached immutable identity must match its actual durable reservation"
            );
            h.fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2",
                    rusqlite::params![point.0.payload_digest, id],
                )
                .unwrap();
            assert_eq!(
                h.fixture
                    .conn
                    .execute(
                        "DELETE FROM member_input_operations WHERE input_id=?1",
                        [id]
                    )
                    .unwrap(),
                1
            );
            let changes = h.fixture.conn.total_changes();
            assert!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: key.into()
                    }
                )
                .is_err(),
                "a retained attempt without its actual reservation is an invariant error"
            );
            assert!(pending(h) == writer);
            assert_eq!(cached(h, id), point);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn live_receipt_reconciliation_uses_cas_rolls_back_and_keeps_real_stage_until_retirement() {
        with_host(|h| {
            let key = "prompt:live-receipt-partial";
            let id = h.arm(key);
            h.pty.short = Some(3);
            h.drain();
            let InputControlReply::Submission(partial) = h.advance().expect("actual partial write")
            else {
                panic!("actual partial result")
            };
            assert_eq!(partial.status, SubmissionStatus::Uncertain);
            assert_eq!(partial.stage, SubmissionStage::Partial);
            let original = storage::get_input(&h.fixture.conn, id).unwrap();
            let point = cached(h, id);
            assert_eq!(point.2, 3);
            let offers = h.pty.offers.clone();
            let action = |expected| ReceiptAction::Reconcile {
                input_id: id,
                expected,
                disposition: storage::InputDisposition::AbandonedUncertain,
                evidence_ref: "fixture:observed-partial".into(),
            };
            let Err(stale) = receipt(h, action(storage::InputState::Accepted)) else {
                panic!("actual state CAS rejects stale expected state")
            };
            assert!(
                stale.source.to_string().contains("state"),
                "{}",
                stale.source
            );
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), original);
            assert_eq!(cached(h, id), point);
            h.fixture.conn.execute_batch("CREATE TRIGGER receipt_settlement_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.state='settled' BEGIN SELECT RAISE(ABORT,'owned receipt settlement failure'); END").unwrap();
            let failed = receipt(h, action(storage::InputState::Uncertain));
            let removed = h
                .fixture
                .conn
                .execute_batch("DROP TRIGGER receipt_settlement_failure");
            assert!(
                failed.as_ref().is_err_and(|failure| failure
                    .source
                    .to_string()
                    .contains("owned receipt settlement failure")),
                "settlement_error={:?}; trigger_cleanup={removed:?}",
                failed.as_ref().err()
            );
            removed.unwrap();
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), original);
            assert_eq!(cached(h, id), point);
            assert!(
                matches!(receipt(h, action(storage::InputState::Uncertain)).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
            );
            let settled = storage::get_input(&h.fixture.conn, id).unwrap();
            assert_eq!(settled.state, storage::InputState::Settled);
            assert_eq!(
                settled.disposition,
                Some(storage::InputDisposition::AbandonedUncertain)
            );
            assert_eq!(
                settled.evidence_ref.as_deref(),
                Some("fixture:observed-partial")
            );
            let changes = h.fixture.conn.total_changes();
            assert!(
                matches!(receipt(h, ReceiptAction::Reconcile { input_id: id, expected: storage::InputState::Settled, disposition: storage::InputDisposition::AbandonedUncertain, evidence_ref: "fixture:preserve-first-evidence".into() }).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
            );
            assert!(
                receipt(
                    h,
                    ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::Settled,
                        disposition: storage::InputDisposition::Submitted,
                        evidence_ref: "fixture:conflicting-wire-disposition".into()
                    }
                )
                .is_err()
            );
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            let retained = cached(h, id);
            assert_eq!(
                (retained.1, retained.2, retained.3),
                (point.1, point.2, point.3)
            );
            let hooks = h.hooks.points.lock().unwrap().len();
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .unwrap()
            else {
                panic!("settled cache retains its actual prior facts")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.status, SubmissionStatus::Rejected);
            assert_eq!(submission.stage, SubmissionStage::Partial);
            assert_eq!(submission.output_seq, Some(point.3));
            assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
            assert_retired(
                receipt(h, ReceiptAction::Retire { input_id: id }).unwrap(),
                id,
            );
            assert!(!h.coordinator.operations.contains_key(&id));
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .unwrap()
            else {
                panic!("retired cache preserves durable Present history")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.input_id, Some(id));
            assert_eq!(submission.status, SubmissionStatus::Rejected);
            assert_eq!(submission.stage, SubmissionStage::Unknown);
            assert_eq!(submission.output_seq, None);
            let repeat = h.request(key);
            let InputControlReply::Submission(repeat) = h
                .coordinator
                .handle(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    repeat,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .unwrap()
            else {
                panic!("consumed key returns its submission retry projection")
            };
            assert_eq!(repeat.status, SubmissionStatus::Rejected);
            assert_eq!(repeat.stage, SubmissionStage::ZeroBytes);
            assert!(h.coordinator.pending.is_none());
            assert_eq!(h.pty.offers, offers);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            assert_eq!(h.hooks.count("reserved"), 1);
            assert_eq!(h.hooks.count("before"), 1);
            assert_eq!(h.hooks.count("after"), 1);
        });
    }

    fn cached(
        h: &Host,
        id: i64,
    ) -> (
        storage::InputOperation,
        SubmissionStage,
        u64,
        u64,
        bool,
        bool,
    ) {
        let retained = h
            .coordinator
            .operations
            .get(&id)
            .expect("actual retained reservation");
        (
            retained.operation.clone(),
            retained.stage,
            retained.written_bytes,
            retained.output_seq,
            retained.attempt_available,
            retained.leased,
        )
    }

    #[test]
    fn retirement_requires_settlement_and_no_active_writer_then_removes_only_its_cache() {
        with_host(|h| {
            let run = h.fixture.run.clone();
            let message = crate::broker::send_message(
                &mut h.fixture.conn,
                run.director_member_id,
                run.member_id,
                "retirement preview",
            )
            .unwrap();
            let key = format!("preview:{}:{}", run.run_id, message.message_id);
            let mut request = h.request(&key);
            request.operation = InputControlOperation::SubmitPrompt(SubmitInput {
                operation_key: key.clone(),
                input_kind: storage::InputKind::Preview,
                message_id: Some(message.message_id),
                literal: "ordinary fixture λ".into(),
            });
            assert!(
                h.coordinator
                    .handle(&mut h.fixture.conn, &run, request, &h.controller, h.until)
                    .unwrap()
                    .is_none()
            );
            let accepted = (0..8)
                .find_map(|_| {
                    h.drain();
                    h.advance()
                })
                .expect("actual Preview completes within finite drained turns");
            let InputControlReply::Submission(accepted) = accepted else {
                panic!("actual Preview submission reply")
            };
            assert!(matches!(accepted.status, SubmissionStatus::Accepted));
            let id = accepted.input_id.expect("durable Preview ID");
            let before = cached(h, id);
            assert!(matches!(before.1, SubmissionStage::Submitted));
            assert!(!before.4 && !before.5);
            assert!(h.coordinator.pending.is_none());
            let offers = h.pty.offers.clone();
            assert_eq!(
                h.pty.program,
                vec!["ordinary fixture λ~".as_bytes().to_vec()]
            );

            let Err(uncommitted) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, h.until)
            else {
                panic!("Accepted Preview requires committed settlement before retirement")
            };
            assert!(uncommitted.to_string().contains(&id.to_string()));
            assert_eq!(cached(h, id), before);
            assert_eq!(
                storage::get_input(&h.fixture.conn, id).unwrap().state,
                storage::InputState::Accepted
            );
            let settled = storage::settle_preview(&mut h.fixture.conn, &run.owner(), id).unwrap();
            let stamp: Option<String> = h
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(stamp.is_some());

            let Err(expired) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, Instant::now())
            else {
                panic!("retirement requires a live original deadline")
            };
            assert!(matches!(expired, CafleetError::DeadlineExceeded));
            let mut foreign_owner = run.clone();
            foreign_owner.owner_epoch += 1;
            assert!(
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &foreign_owner, id, h.until)
                    .is_err()
            );
            let mut foreign_request = h.request("unused-retirement-key");
            foreign_request.operation =
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
            foreign_request.identity.member_id += 1;
            assert!(
                h.coordinator
                    .handle(
                        &mut h.fixture.conn,
                        &run,
                        foreign_request,
                        &h.controller,
                        h.until
                    )
                    .is_err()
            );
            assert_eq!(cached(h, id), before);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);

            let unrelated_key = "prompt:retirement-active-writer";
            let unrelated = h.request(unrelated_key);
            assert!(
                h.coordinator
                    .handle(&mut h.fixture.conn, &run, unrelated, &h.controller, h.until)
                    .unwrap()
                    .is_none()
            );
            for _ in 0..8 {
                h.drain();
                assert!(h.advance().is_none());
                if h.coordinator.pending.as_ref().unwrap().phase == InputPhase::Armed {
                    break;
                }
            }
            let pending = h.coordinator.pending.as_ref().unwrap();
            assert!(pending.phase == InputPhase::Armed);
            let unrelated_id = pending.input_id.expect("actual second reservation");
            let original_deadline = pending.deadline;
            let original_bytes = pending.bytes.clone();
            let unrelated_before = cached(h, unrelated_id);
            assert!(unrelated_before.4 && !unrelated_before.5);
            let Err(failure) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, h.until)
            else {
                panic!("retirement excludes the actual Armed writer")
            };
            assert!(failure.to_string().contains(&id.to_string()));
            let pending = h
                .coordinator
                .pending
                .as_ref()
                .expect("refusal preserves unrelated writer");
            assert_eq!(pending.request.operation_key, unrelated_key);
            assert_eq!(pending.input_id, Some(unrelated_id));
            assert_eq!(pending.deadline, original_deadline);
            assert_eq!(pending.bytes, original_bytes);
            assert!(pending.phase == InputPhase::Armed);
            assert_eq!(cached(h, unrelated_id), unrelated_before);
            assert_eq!(cached(h, id), before);
            assert_eq!(h.pty.offers, offers);

            h.coordinator.finish_pending_attempt();
            let other_cache = cached(h, unrelated_id);
            let other_row = storage::get_input(&h.fixture.conn, unrelated_id).unwrap();
            assert_eq!(other_row.state, storage::InputState::InProgress);
            h.pty
                .child
                .bytes
                .extend(b"\x1b[2J\x1b[HUnclassified screen");
            h.drain();
            assert!(!matches!(
                h.controller.health(&h.writer).eligibility,
                InputEligibility::ComposerEmpty
            ));
            let changes = h.fixture.conn.total_changes();
            let mut retirement = h.request("unused-retirement-key");
            retirement.operation =
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
            let reply = h
                .coordinator
                .handle(
                    &mut h.fixture.conn,
                    &run,
                    retirement,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .expect("typed retirement dispatch");
            assert_retired(reply, id);
            assert!(!h.coordinator.operations.contains_key(&id));
            assert_eq!(cached(h, unrelated_id), other_cache);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            assert_eq!(
                storage::get_input(&h.fixture.conn, unrelated_id).unwrap(),
                other_row
            );
            let after_stamp: Option<String> = h
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(after_stamp, stamp);
            assert_eq!(h.pty.offers, offers);

            let reads = Arc::new(AtomicUsize::new(0));
            let observed = reads.clone();
            h.fixture
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "member_input_operations",
                            ..
                        }
                    ) {
                        observed.fetch_add(1, Ordering::SeqCst);
                    }
                    Authorization::Allow
                }))
                .unwrap();
            let again = h
                .coordinator
                .retire_received(&mut h.fixture.conn, &run, id, h.until);
            let removed = h
                .fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            assert!(
                again.is_ok(),
                "repeat_error={:?}; authorizer_cleanup={removed:?}",
                again.as_ref().err()
            );
            removed.unwrap();
            assert_retired(again.unwrap(), id);
            assert!(
                reads.load(Ordering::SeqCst) > 0,
                "cache absence still checks the durable row on the supplied connection"
            );
            assert!(
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, i64::MAX, h.until)
                    .is_err()
            );
            assert_eq!(cached(h, unrelated_id), other_cache);
            assert_eq!(h.pty.offers, offers);
        });
    }

    #[test]
    fn live_receipt_retirement_preserves_settled_ordinary_history_without_a_cached_attempt() {
        with_host(|h| {
            let run = h.fixture.run.clone();
            let prompt = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: "prompt:retirement-history".into(),
                    kind: storage::InputKind::Prompt,
                    payload_digest: "a".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            storage::record_input_state(
                &mut h.fixture.conn,
                &run.owner(),
                prompt.input_id,
                storage::InputState::InProgress,
                storage::InputState::Accepted,
            )
            .unwrap();
            let prompt = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                prompt.input_id,
                storage::InputState::Accepted,
                storage::InputDisposition::Submitted,
                "owned historical Prompt",
            )
            .unwrap();
            let message = crate::broker::send_message(
                &mut h.fixture.conn,
                run.director_member_id,
                run.member_id,
                "abandoned preview",
            )
            .unwrap();
            let preview = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: format!("preview:{}:{}", run.run_id, message.message_id),
                    kind: storage::InputKind::Preview,
                    payload_digest: "b".repeat(64),
                    message_id: Some(message.message_id),
                },
            )
            .unwrap();
            storage::record_input_state(
                &mut h.fixture.conn,
                &run.owner(),
                preview.input_id,
                storage::InputState::InProgress,
                storage::InputState::Uncertain,
            )
            .unwrap();
            let preview = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                preview.input_id,
                storage::InputState::Uncertain,
                storage::InputDisposition::AbandonedUncertain,
                "owned abandoned Preview",
            )
            .unwrap();
            let exec = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: "exec:retirement-history".into(),
                    kind: storage::InputKind::Exec,
                    payload_digest: "c".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            let exec = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                exec.input_id,
                storage::InputState::InProgress,
                storage::InputDisposition::NotSubmitted,
                "owned no-write Exec history",
            )
            .unwrap();
            assert!(h.coordinator.operations.is_empty());
            let changes = h.fixture.conn.total_changes();
            for row in [prompt, preview, exec] {
                assert_retired(
                    h.coordinator
                        .retire_received(&mut h.fixture.conn, &run, row.input_id, h.until)
                        .unwrap(),
                    row.input_id,
                );
                assert_eq!(
                    storage::get_input(&h.fixture.conn, row.input_id).unwrap(),
                    row
                );
                assert_eq!(h.fixture.conn.total_changes(), changes);
            }
            assert!(h.coordinator.operations.is_empty());
            assert!(h.pty.offers.is_empty());
        });
    }
}
