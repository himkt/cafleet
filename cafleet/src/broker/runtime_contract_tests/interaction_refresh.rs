use super::*;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

struct RefreshFixture {
    fixture: Fixture,
    owner: RunOwner,
    event: RuntimeEventRecord,
    update: InteractionEvidenceUpdate,
}

impl RefreshFixture {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        fixture.conn.execute("UPDATE member_runs SET profile_version='fixture-v1',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        let update = InteractionEvidenceUpdate {
            run_id: owner.run_id,
            episode_seq: 1,
            output_seq: 121,
            evidence_json: evidence(121),
        };
        Self {
            fixture,
            owner,
            event,
            update,
        }
    }
    fn refresh(&mut self, barrier: i64) -> Result<InteractionRecord, crate::error::CafleetError> {
        storage::update_interaction_evidence(
            &mut self.fixture.conn,
            &self.owner,
            &self.update,
            barrier,
            Instant::now() + Duration::from_secs(1),
        )
    }
    fn interaction(&self) -> InteractionRecord {
        storage::get_event(&self.fixture.conn, self.event.event_id)
            .unwrap()
            .interaction
            .unwrap()
    }
}

fn evidence(sequence: u64) -> Value {
    let mut raw = raw_evidence(SECRET);
    raw["screen_seq"] = json!(sequence);
    evidence::EvidencePolicy::metadata().apply(&raw).unwrap()
}

fn state(conn: &Connection) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let names = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let escaped = name.replace('"', "\"\"");
            let values = rows(conn, &format!("\"{escaped}\""));
            (name, values)
        })
        .collect()
}

fn assert_unchanged_except_interaction(
    before: &BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>,
    after: &BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>,
) {
    assert_eq!(before.len(), after.len());
    for (table, old) in before {
        if table != "member_interactions" {
            assert_eq!(old, after.get(table).unwrap(), "{table}");
        }
    }
}

#[test]
fn runtime_refresh_replaces_only_latest_evidence_and_preserves_opening_bytes_and_sequences() {
    let mut fixture = RefreshFixture::new();
    let original = fixture.interaction();
    let before = state(&fixture.fixture.conn);
    let updated = fixture.refresh(1).unwrap();
    assert_eq!(updated.interaction_id, original.interaction_id);
    assert_eq!(updated.run_id, original.run_id);
    assert_eq!(updated.episode_seq, original.episode_seq);
    assert_eq!(updated.classification, original.classification);
    assert_eq!(updated.opened_at, original.opened_at);
    assert_eq!(updated.closed_at, None);
    assert_eq!(updated.close_reason, None);
    assert_eq!(updated.evidence_json, fixture.update.evidence_json);
    assert_eq!(
        updated.evidence_digest,
        fixture.update.evidence_json["evidence_digest"]
            .as_str()
            .unwrap()
    );
    assert_unchanged_except_interaction(&before, &state(&fixture.fixture.conn));
    assert_eq!(
        storage::get_event(&fixture.fixture.conn, fixture.event.event_id)
            .unwrap()
            .event,
        fixture.event
    );
    assert_no_secret(&fixture.fixture.conn, SECRET);
    assert_eq!(
        storage::get_event(&fixture.fixture.reopen(), fixture.event.event_id)
            .unwrap()
            .interaction
            .unwrap(),
        updated
    );
}

#[test]
fn runtime_refresh_equal_canonical_evidence_is_idempotent_without_rewriting_the_row() {
    let mut fixture = RefreshFixture::new();
    let first = fixture.refresh(1).unwrap();
    let before = state(&fixture.fixture.conn);
    let old = fixture.update.evidence_json.as_object().unwrap();
    let mut reordered = serde_json::Map::new();
    for (key, value) in old.iter().rev() {
        reordered.insert(key.clone(), value.clone());
    }
    fixture.update.evidence_json = Value::Object(reordered);
    assert_eq!(fixture.refresh(1).unwrap(), first);
    assert_eq!(state(&fixture.fixture.conn), before);
}

#[test]
fn runtime_refresh_regressing_or_same_sequence_conflicting_evidence_fails_atomically() {
    for sequence in [120, 121] {
        let mut fixture = RefreshFixture::new();
        fixture.refresh(1).unwrap();
        let mut raw = raw_evidence(SECRET);
        raw["screen_seq"] = json!(sequence);
        raw["truncated"] = json!(true);
        fixture.update.output_seq = sequence;
        fixture.update.evidence_json = evidence::EvidencePolicy::metadata().apply(&raw).unwrap();
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(1).is_err());
        assert_eq!(state(&fixture.fixture.conn), before);
    }
}

#[test]
fn runtime_refresh_a_later_committed_semantic_transition_rejects_the_old_barrier() {
    let mut fixture = RefreshFixture::new();
    let obscured = ObservedTransition {
        kind: RuntimeEventKind::InteractionObscured,
        ..opening(fixture.owner.run_id, 2, 1, metadata())
    };
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.owner,
        &obscured,
        PersistenceMode::Live,
    )
    .unwrap();
    let before = state(&fixture.fixture.conn);
    assert!(fixture.refresh(1).is_err());
    assert_eq!(state(&fixture.fixture.conn), before);
    let refreshed = fixture.refresh(2).unwrap();
    assert_eq!(refreshed.evidence_json["screen_seq"], 121);
    assert_unchanged_except_interaction(&before, &state(&fixture.fixture.conn));
}

#[test]
fn runtime_refresh_barrier_must_resolve_the_current_committed_maximum() {
    for barrier in [-1, 0, 2, 256] {
        let mut fixture = RefreshFixture::new();
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(barrier).is_err());
        assert_eq!(state(&fixture.fixture.conn), before);
    }
}

#[test]
fn runtime_refresh_closed_or_superseded_episode_preserves_its_closure_evidence() {
    for reason in [
        InteractionCloseReason::NotVisible,
        InteractionCloseReason::Superseded,
        InteractionCloseReason::ProcessExited,
    ] {
        let mut fixture = RefreshFixture::new();
        let closed = ObservedTransition {
            close_reason: Some(reason),
            ..closure(fixture.owner.run_id, 2, 1)
        };
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.owner,
            &closed,
            PersistenceMode::Live,
        )
        .unwrap();
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(2).is_err());
        assert_eq!(state(&fixture.fixture.conn), before);
    }
}

#[test]
fn runtime_refresh_targets_the_matching_open_run_and_episode_only() {
    for field in ["run", "episode", "absent"] {
        let mut fixture = RefreshFixture::new();
        match field {
            "run" => fixture.update.run_id += 1,
            "episode" => fixture.update.episode_seq = 0,
            _ => fixture.update.episode_seq = 2,
        }
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(1).is_err());
        assert_eq!(state(&fixture.fixture.conn), before);
    }
}

#[test]
fn runtime_refresh_semantic_barrier_and_episode_identity_are_scoped_to_the_owned_run() {
    let mut fixture = RefreshFixture::new();
    let other_member = common::register(
        &mut fixture.fixture.conn,
        fixture.fixture.fleet,
        "other-refresh-run",
        Some("%3"),
    );
    let mut spec = Fixture::spec(
        &fixture.fixture.conn,
        fixture.fixture.fleet,
        fixture.fixture.director,
        other_member,
    );
    spec.runtime_path.push_str("-other");
    spec.launch_nonce = "b".repeat(64);
    spec.profile_version = Some("fixture-v1".into());
    spec.coverage = Coverage::Tested;
    let other = storage::create_run(&mut fixture.fixture.conn, &spec).unwrap();
    fixture.fixture.conn.execute("UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2 WHERE run_id=?1",[other.run_id]).unwrap();
    let owner = storage::get_run(&fixture.fixture.conn, other.run_id)
        .unwrap()
        .owner();
    storage::reserve_event_sequences(&mut fixture.fixture.conn, &owner).unwrap();
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &owner,
        &opening(other.run_id, 9, 1, metadata()),
        PersistenceMode::Live,
    )
    .unwrap();
    assert_eq!(fixture.refresh(1).unwrap().run_id, fixture.owner.run_id);
    fixture.update.run_id = other.run_id;
    let before = state(&fixture.fixture.conn);
    assert!(fixture.refresh(9).is_err());
    assert_eq!(state(&fixture.fixture.conn), before);
}

#[test]
fn runtime_refresh_validates_evidence_sequence_digest_policy_backend_and_profile() {
    for field in [
        "shape", "sequence", "digest", "policy", "backend", "profile", "native", "raw",
    ] {
        let mut fixture = RefreshFixture::new();
        match field {
            "shape" => fixture.update.evidence_json = Value::Null,
            "sequence" => fixture.update.output_seq = 122,
            "digest" => fixture.update.evidence_json["evidence_digest"] = json!("0".repeat(64)),
            "raw" => fixture.update.evidence_json["operation"] = json!(SECRET),
            "policy" => {
                let mut raw = raw_evidence(SECRET);
                raw["screen_seq"] = json!(121);
                fixture.update.evidence_json = evidence::EvidencePolicy::redacted(
                    &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
                    &["native-tool-permission"],
                )
                .unwrap()
                .apply(&raw)
                .unwrap();
            }
            other => {
                let mut raw = raw_evidence(SECRET);
                raw["screen_seq"] = json!(121);
                let key = match other {
                    "backend" => "backend",
                    "profile" => "profile_version",
                    _ => "native_version",
                };
                raw[key] = json!("other");
                fixture.update.evidence_json =
                    evidence::EvidencePolicy::metadata().apply(&raw).unwrap();
            }
        }
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(1).is_err(), "{field}");
        assert_eq!(state(&fixture.fixture.conn), before, "{field}");
    }
}

#[test]
fn runtime_refresh_validates_every_current_owner_identity_component() {
    for field in ["run", "database", "epoch", "kind", "pid", "start"] {
        let mut fixture = RefreshFixture::new();
        match field {
            "run" => fixture.owner.run_id += 1,
            "database" => fixture.owner.database_identity_json["instance_id"] = json!("foreign"),
            "epoch" => fixture.owner.owner_epoch += 1,
            "kind" => fixture.owner.owner_kind = OwnerKind::Reconciler,
            "pid" => fixture.owner.owner_pid += 1,
            _ => fixture.owner.owner_start_identity.push_str("-foreign"),
        }
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(1).is_err(), "{field}");
        assert_eq!(state(&fixture.fixture.conn), before, "{field}");
    }
}

#[test]
fn runtime_refresh_takeover_fences_the_previous_owner() {
    let mut fixture = RefreshFixture::new();
    fixture.fixture.recoverer();
    let before = state(&fixture.fixture.conn);
    assert!(fixture.refresh(1).is_err());
    assert_eq!(state(&fixture.fixture.conn), before);
}

#[test]
fn runtime_refresh_update_and_commit_failures_roll_back_for_exact_retry() {
    for commit in [false, true] {
        let mut fixture = RefreshFixture::new();
        if commit {
            fixture.fixture.conn.commit_hook(Some(|| true));
        } else {
            fixture.fixture.conn.execute_batch("CREATE TRIGGER block_refresh BEFORE UPDATE ON member_interactions BEGIN SELECT RAISE(ABORT,'fixture refresh update failed'); END").unwrap();
        }
        let before = state(&fixture.fixture.conn);
        assert!(fixture.refresh(1).is_err());
        assert_eq!(state(&fixture.fixture.reopen()), before);
        if commit {
            fixture.fixture.conn.commit_hook(None::<fn() -> bool>);
        } else {
            fixture
                .fixture
                .conn
                .execute_batch("DROP TRIGGER block_refresh")
                .unwrap();
        }
        assert_eq!(
            fixture.refresh(1).unwrap().evidence_json,
            fixture.update.evidence_json
        );
    }
}

#[test]
fn runtime_refresh_expired_deadline_has_zero_effect_and_no_fresh_budget() {
    let mut fixture = RefreshFixture::new();
    let before = state(&fixture.fixture.conn);
    let start = Instant::now();
    assert!(
        storage::update_interaction_evidence(
            &mut fixture.fixture.conn,
            &fixture.owner,
            &fixture.update,
            1,
            start - Duration::from_millis(1)
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(state(&fixture.fixture.conn), before);
}

#[test]
fn runtime_refresh_database_contention_obeys_the_absolute_deadline_and_restores_reuse() {
    let mut fixture = RefreshFixture::new();
    let blocking = fixture.fixture.reopen();
    blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
    fixture
        .fixture
        .conn
        .busy_timeout(Duration::from_secs(5))
        .unwrap();
    let before = state(&fixture.fixture.conn);
    let start = Instant::now();
    assert!(
        storage::update_interaction_evidence(
            &mut fixture.fixture.conn,
            &fixture.owner,
            &fixture.update,
            1,
            start + Duration::from_millis(80)
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_millis(750));
    assert_eq!(state(&fixture.fixture.conn), before);
    blocking.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5000
    );
    assert_eq!(
        fixture.refresh(1).unwrap().evidence_json,
        fixture.update.evidence_json
    );
}

#[test]
fn runtime_refresh_post_policy_redacted_and_sensitive_updates_remain_secret_free() {
    for sensitive in [false, true] {
        let mut fixture = RefreshFixture::new();
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                [fixture.owner.run_id],
            )
            .unwrap();
        let mut raw = raw_evidence(SECRET);
        raw["screen_seq"] = json!(121);
        raw["sensitive"] = json!(sensitive);
        fixture.update.evidence_json = evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
            &["native-tool-permission"],
        )
        .unwrap()
        .apply(&raw)
        .unwrap();
        let original_event = fixture.event.clone();
        let update = fixture.refresh(1).unwrap();
        assert_eq!(
            update.evidence_json["operation"],
            if sensitive {
                Value::Null
            } else {
                json!("[redacted]")
            }
        );
        assert_no_secret(&fixture.fixture.conn, SECRET);
        assert_eq!(
            storage::get_event(&fixture.fixture.conn, original_event.event_id)
                .unwrap()
                .event,
            original_event
        );
    }
}
