use super::worker::{Hooks, Release, committed, start, wait_until};
use super::*;

fn opening(fixture: &WriterFixture) -> ObservedTransition {
    ObservedTransition {
        episode_seq: Some(1),
        kind: RuntimeEventKind::InteractionOpened,
        classification: Some(InteractionClass::PermissionRequest),
        cause_json: None,
        ..transition(fixture, 1)
    }
}
fn update(fixture: &WriterFixture, sequence: u64) -> InteractionEvidenceUpdate {
    InteractionEvidenceUpdate {
        run_id: fixture.fixture.run.run_id,
        episode_seq: 1,
        output_seq: sequence,
        evidence_json: transition(fixture, sequence.try_into().unwrap()).evidence_json,
    }
}
fn latest(fixture: &WriterFixture) -> Value {
    let text: String = fixture
        .fixture
        .conn
        .query_row(
            "SELECT evidence_json FROM member_interactions WHERE run_id=?1 AND episode_seq=1",
            [fixture.fixture.run.run_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&text).unwrap()
}
fn committed_open(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
    let writer = start(fixture, hooks);
    assert!(writer.try_submit(opening(fixture)).is_ok());
    wait_until(|| committed(fixture) == 1 && writer.status().pending_count == 0);
    writer
}

#[test]
fn runner_batch2_refresh_waits_for_its_opening_barrier_without_an_outbox_frame() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Persist);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    assert!(writer.try_submit(opening(&fixture)).is_ok());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    hooks.wait(WriterBoundary::Persist, 1);
    assert_eq!(committed(&fixture), 0);
    assert!(writer.status().pending_evidence);
    assert_eq!(
        independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
        vec![opening(&fixture)]
    );
    drop(release);
    wait_until(|| committed(&fixture) == 1 && !writer.status().pending_evidence);
    assert_eq!(latest(&fixture)["screen_seq"], 2);
    assert_eq!(writer.finish(deadline()).0, 0);
    assert_eq!(committed(&fixture), 1);
}

#[test]
fn runner_batch2_refresh_newer_revision_survives_an_inflight_successful_attempt() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    let mut writer = committed_open(&fixture, hooks.clone());
    hooks.pause(WriterBoundary::Persist);
    let release = Release(hooks.clone());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    hooks.wait(WriterBoundary::Persist, 2);
    assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
    assert!(writer.status().pending_evidence);
    drop(release);
    wait_until(|| !writer.status().pending_evidence);
    assert_eq!(latest(&fixture)["screen_seq"], 3);
    assert_eq!(committed(&fixture), 1);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_refresh_newer_revision_survives_an_inflight_failed_attempt() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    let mut writer = committed_open(&fixture, hooks.clone());
    hooks.pause(WriterBoundary::Persist);
    let release = Release(hooks.clone());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    hooks.wait(WriterBoundary::Persist, 2);
    assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
    hooks.fail_once(WriterBoundary::Persist);
    drop(release);
    wait_until(|| !writer.status().pending_evidence);
    assert_eq!(latest(&fixture)["screen_seq"], 3);
    assert_eq!(committed(&fixture), 1);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_refresh_rejects_regression_conflict_and_foreign_episode_without_replacing_newer_work()
 {
    for field in ["regression", "conflict", "episode"] {
        let fixture = WriterFixture::new();
        let hooks = Arc::new(Hooks::default());
        let mut writer = committed_open(&fixture, hooks.clone());
        hooks.pause(WriterBoundary::Persist);
        let release = Release(hooks.clone());
        assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
        hooks.wait(WriterBoundary::Persist, 2);
        let mut invalid = update(&fixture, if field == "regression" { 2 } else { 3 });
        if field == "episode" {
            invalid.episode_seq = 2;
        }
        if field == "conflict" {
            let mut raw = invalid.evidence_json.clone();
            raw["sensitive"] = serde_json::json!(false);
            raw["truncated"] = serde_json::json!(true);
            invalid.evidence_json = storage::evidence::EvidencePolicy::metadata()
                .apply(&raw)
                .unwrap();
        }
        let returned = writer
            .try_refresh(invalid, deadline())
            .expect_err("invalid refresh retains caller ownership");
        assert_eq!(
            returned.update.output_seq,
            if field == "regression" { 2 } else { 3 }
        );
        assert!(writer.status().degraded);
        drop(release);
        wait_until(|| !writer.status().pending_evidence);
        assert_eq!(latest(&fixture)["screen_seq"], 3);
        writer.finish(deadline());
    }
}

#[test]
fn runner_batch2_refresh_closure_follows_earlier_evidence_and_rejects_late_old_episode_updates() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    let mut writer = committed_open(&fixture, hooks.clone());
    hooks.pause(WriterBoundary::Persist);
    let release = Release(hooks.clone());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    hooks.wait(WriterBoundary::Persist, 2);
    let close = ObservedTransition {
        event_seq: 2,
        kind: RuntimeEventKind::InteractionClosed,
        close_reason: Some(InteractionCloseReason::NotVisible),
        evidence_json: transition(&fixture, 3).evidence_json,
        ..opening(&fixture)
    };
    assert!(writer.try_submit(close.clone()).is_ok());
    assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_err());
    drop(release);
    wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
    assert_eq!(latest(&fixture), close.evidence_json);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_refresh_newer_same_episode_revision_advances_its_semantic_barrier() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Receive);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    hooks.wait(WriterBoundary::Receive, 1);
    assert!(writer.try_submit(opening(&fixture)).is_ok());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    let obscured = ObservedTransition {
        event_seq: 2,
        kind: RuntimeEventKind::InteractionObscured,
        evidence_json: transition(&fixture, 3).evidence_json,
        ..opening(&fixture)
    };
    assert!(writer.try_submit(obscured).is_ok());
    assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_ok());
    drop(release);
    wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
    assert_eq!(latest(&fixture)["screen_seq"], 4);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_refresh_equal_identical_requests_coalesce_and_expiry_has_no_enqueue_effect() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    let mut writer = committed_open(&fixture, hooks.clone());
    hooks.pause(WriterBoundary::Persist);
    let release = Release(hooks.clone());
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    hooks.wait(WriterBoundary::Persist, 2);
    assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
    let begin = Instant::now();
    assert!(
        writer
            .try_refresh(update(&fixture, 3), begin - Duration::from_millis(1))
            .is_err()
    );
    assert!(begin.elapsed() < Duration::from_millis(250));
    drop(release);
    wait_until(|| !writer.status().pending_evidence);
    assert_eq!(latest(&fixture)["screen_seq"], 2);
    writer.finish(deadline());
}
