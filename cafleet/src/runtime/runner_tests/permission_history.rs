use super::*;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, PaneLock, with_database_budget};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod entry;

const NOW: &str = "2026-10-26T22:00:00.000000+00:00";

#[derive(Clone, Copy, Debug)]
pub(in crate::runtime) enum Retained {
    InProgress,
    Accepted,
    Uncertain,
    Submitted,
    NotSubmitted,
    Abandoned,
}
pub(in crate::runtime) struct Case {
    pub(in crate::runtime) fixture: Fixture,
    owner: RunOwner,
    event: RuntimeEventRecord,
    request: NewPermissionResponse,
    pub(in crate::runtime) input_id: i64,
    pub(in crate::runtime) settings: Settings,
    placement: crate::broker::records::Placement,
    _artifacts: RunArtifacts,
    _lock: Arc<RunLock>,
}
impl Case {
    pub(in crate::runtime) fn new(state: Retained) -> Self {
        Self::decision(
            state,
            PermissionDecision::Allow,
            DirectorMode::Auto,
            DecisionSource::DirectorAuto,
        )
    }
    fn decision(
        state: Retained,
        decision: PermissionDecision,
        mode: DirectorMode,
        source: DecisionSource,
    ) -> Self {
        Self::from_fixture(Fixture::new(), state, decision, mode, source)
    }
    fn from_fixture(
        mut fixture: Fixture,
        state: Retained,
        decision: PermissionDecision,
        mode: DirectorMode,
        source: DecisionSource,
    ) -> Self {
        let artifacts = fixture.artifacts();
        let lock = Arc::new(
            RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap(),
        );
        let owner = lock
            .transfer_to_runner(
                &mut fixture.conn,
                &fixture.run.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
        assert_eq!(fixture.conn.execute(
            "UPDATE member_runs SET status='running',coverage='tested',profile_version='fixture-detector-v1',started_at=?1 WHERE run_id=?2",
            rusqlite::params![NOW, fixture.run.run_id],
        ).unwrap(), 1);
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        storage::reserve_event_sequences(&mut fixture.conn, &owner).unwrap();
        let evidence = storage::evidence::EvidencePolicy::metadata().apply(&json!({
            "backend":"codex","native_version":fixture.run.native_version,"profile_version":"fixture-detector-v1",
            "rule_id":"fixture-permission","screen_seq":1,"operation":null,"choices":null,
            "native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false
        })).unwrap();
        let transition = ObservedTransition {
            run_id: owner.run_id,
            event_seq: 1,
            episode_seq: Some(1),
            kind: RuntimeEventKind::InteractionOpened,
            classification: Some(InteractionClass::PermissionRequest),
            observed_at: NOW.into(),
            evidence_json: evidence,
            close_reason: None,
            cause_json: None,
        };
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Live,
        )
        .unwrap();
        let answer_rule = match decision {
            PermissionDecision::Allow => "fixture-allow-once",
            PermissionDecision::Deny => "fixture-deny-once",
        };
        let capability = crate::coding_agent::AnswerCapability::fixture(
            crate::coding_agent::CodingAgent::Codex,
            fixture.run.native_version.as_deref().unwrap(),
            "fixture-answer-v1",
            "fixture-permission",
            answer_rule,
            decision,
            &["synthetic-history-storage".into()],
        )
        .unwrap();
        let request = NewPermissionResponse {
            operation_key: "fixture:retained-response".into(),
            metadata: PermissionResponseMetadata {
                event_id: event.event_id,
                interaction_id: event.interaction_id.unwrap(),
                expected_owner_epoch: owner.owner_epoch,
                request_token: "a".repeat(64),
                decision,
                director_member_id: fixture.run.director_member_id,
                director_mode: mode,
                decision_source: source,
                mode_evidence_ref: "fixture:host-mode".into(),
                decision_evidence_ref: "fixture:attributed-decision".into(),
                answer_profile_version: "fixture-answer-v1".into(),
                answer_rule_id: answer_rule.into(),
            },
        };
        let receipt = match storage::reserve_permission_response(
            &mut fixture.conn,
            &owner,
            &request,
            Some(&capability),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap()
        {
            PermissionReservation::Reserved(receipt) => receipt,
            PermissionReservation::Retained(_) => {
                panic!("a fresh history fixture reserves its own identity")
            }
        };
        let input_id = receipt.input.input_id;
        match state {
            Retained::InProgress => {}
            Retained::Accepted | Retained::Uncertain => {
                storage::record_input_state(
                    &mut fixture.conn,
                    &owner,
                    input_id,
                    InputState::InProgress,
                    if matches!(state, Retained::Accepted) {
                        InputState::Accepted
                    } else {
                        InputState::Uncertain
                    },
                )
                .unwrap();
            }
            Retained::Submitted | Retained::Abandoned => {
                storage::settle_input(
                    &mut fixture.conn,
                    &owner,
                    input_id,
                    InputState::InProgress,
                    if matches!(state, Retained::Submitted) {
                        InputDisposition::Submitted
                    } else {
                        InputDisposition::AbandonedUncertain
                    },
                    "fixture:recorded-disposition",
                )
                .unwrap();
            }
            Retained::NotSubmitted => {
                storage::record_input_state(
                    &mut fixture.conn,
                    &owner,
                    input_id,
                    InputState::InProgress,
                    InputState::Ready,
                )
                .unwrap();
                storage::settle_input(
                    &mut fixture.conn,
                    &owner,
                    input_id,
                    InputState::Ready,
                    InputDisposition::NotSubmitted,
                    "fixture:recorded-zero-byte-disposition",
                )
                .unwrap();
            }
        }
        let read = storage::get_permission_response(&fixture.conn, input_id).unwrap();
        assert!(read.claim.is_some());
        assert_eq!(read.response.metadata, request.metadata);
        let settings = Settings {
            database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
            max_text_len: 200,
            multiplexer: None,
        };
        let placement =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
        Self {
            fixture,
            owner,
            event,
            request,
            input_id,
            settings,
            placement,
            _artifacts: artifacts,
            _lock: lock,
        }
    }
}

#[derive(PartialEq)]
struct Table {
    name: String,
    values: Vec<Vec<rusqlite::types::Value>>,
}
struct Snapshot {
    tables: Vec<Table>,
    changes: u64,
}
impl Snapshot {
    fn take(conn: &Connection) -> Self {
        let names = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let tables = names
            .into_iter()
            .map(|name| {
                let quoted = name.replace('"', "\"\"");
                let mut statement = conn
                    .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY rowid"))
                    .unwrap();
                let columns = statement.column_count();
                let values = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                Table { name, values }
            })
            .collect();
        Self {
            tables,
            changes: conn.total_changes(),
        }
    }
    fn unchanged(&self, conn: &Connection) {
        let actual = Self::take(conn);
        assert_eq!(
            actual.changes, self.changes,
            "historical coordination performs no SQL mutation"
        );
        assert!(
            actual.tables == self.tables,
            "historical coordination preserves every table exactly"
        );
    }
}
