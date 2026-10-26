use super::*;
use std::sync::Condvar;

fn index(boundary: WriterBoundary) -> usize {
    match boundary {
        WriterBoundary::Receive => 0,
        WriterBoundary::Append => 1,
        WriterBoundary::Sync => 2,
        WriterBoundary::Persist => 3,
        WriterBoundary::Compact => 4,
    }
}
#[derive(Debug, Default)]
struct HookState {
    paused: [bool; 5],
    failures: [u64; 5],
    calls: [u64; 5],
    first: [Option<u64>; 5],
    ordinal: u64,
}
#[derive(Default)]
pub(super) struct Hooks {
    state: Mutex<HookState>,
    changed: Condvar,
}
impl Hooks {
    pub(super) fn pause(&self, boundary: WriterBoundary) {
        self.state.lock().unwrap().paused[index(boundary)] = true;
    }
    fn release_for_cleanup(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                eprintln!("fixture hook cleanup recovered poisoned state");
                poisoned.into_inner()
            }
        };
        state.paused = [false; 5];
        drop(state);
        self.changed.notify_all();
    }
    pub(super) fn fail_once(&self, boundary: WriterBoundary) {
        self.state.lock().unwrap().failures[index(boundary)] += 1;
    }
    pub(super) fn wait(&self, boundary: WriterBoundary, count: u64) {
        let slot = index(boundary);
        let until = deadline();
        let mut state = self.state.lock().unwrap();
        while state.calls[slot] < count {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let calls = state.calls;
                drop(state);
                panic!("worker boundary {slot} was not reached; calls={calls:?}");
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }
    pub(super) fn calls(&self, boundary: WriterBoundary) -> u64 {
        self.state.lock().unwrap().calls[index(boundary)]
    }
}
impl WriterHooks for Hooks {
    fn before(&self, boundary: WriterBoundary, until: Instant) -> Result<()> {
        let slot = index(boundary);
        let mut state = self.state.lock().unwrap();
        state.ordinal = state.ordinal.checked_add(1).unwrap();
        state.calls[slot] = state.calls[slot].checked_add(1).unwrap();
        let ordinal = state.ordinal;
        state.first[slot].get_or_insert(ordinal);
        self.changed.notify_all();
        while state.paused[slot] {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(CafleetError::DeadlineExceeded);
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        if state.failures[slot] > 0 {
            state.failures[slot] -= 1;
            return Err(error("fixture writer boundary failure"));
        }
        Ok(())
    }
}
pub(super) struct Release(pub Arc<Hooks>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release_for_cleanup();
    }
}

pub(super) fn start(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
    EventWriter::start_with_hooks(
        format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
        fixture.fixture.run.clone(),
        fixture.lock.clone(),
        fixture.artifacts.paths.outbox.clone(),
        hooks,
    )
    .unwrap()
}
pub(super) fn wait_until(mut condition: impl FnMut() -> bool) {
    let until = deadline();
    while !condition() {
        assert!(
            Instant::now() < until,
            "bounded worker condition did not complete"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
pub(super) fn committed(fixture: &WriterFixture) -> i64 {
    fixture
        .fixture
        .conn
        .query_row(
            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
            [fixture.fixture.run.run_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn runner_batch2_hook_timeout_releases_the_mutex_before_unwinding_through_cleanup() {
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Receive);
    let observed = hooks.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _release = Release(observed.clone());
        observed.wait(WriterBoundary::Persist, 1);
    }));
    assert!(result.is_err());
    assert!(!hooks.state.is_poisoned());
    assert_eq!(hooks.state.lock().unwrap().paused, [false; 5]);
}

#[test]
fn runner_batch2_hook_cleanup_releases_known_state_after_a_worker_poison_without_panicking() {
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Receive);
    let observed = hooks.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _state = observed.state.lock().unwrap();
        panic!("owned fixture worker panic");
    }));
    assert!(result.is_err());
    let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(Release(hooks.clone()))
    }));
    assert!(cleanup.is_ok());
    let state = hooks
        .state
        .lock()
        .expect_err("ordinary access preserves poisoned-state invariant")
        .into_inner();
    assert_eq!(state.paused, [false; 5]);
}

#[test]
fn runner_batch2_worker_receive_stall_pins_64_channel_slots_and_returns_the_failed_original() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Receive);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    hooks.wait(WriterBoundary::Receive, 1);
    for sequence in 1..=64 {
        assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
    }
    let original = transition(&fixture, 65);
    let begin = Instant::now();
    let rejected = writer
        .try_submit(original.clone())
        .expect_err("channel pressure returns the original");
    assert!(begin.elapsed() < Duration::from_millis(250));
    assert_eq!(rejected.transition, original);
    let status = writer.status();
    assert_eq!(status.pending_count, 64);
    assert!(status.degraded);
    assert!(status.gap_count > 0);
    assert_eq!(committed(&fixture), 0);
    drop(release);
    wait_until(|| writer.status().pending_count == 0);
    assert_eq!(committed(&fixture), 64);
    assert_eq!(writer.finish(deadline()).0, 0);
}

#[test]
fn runner_batch2_worker_sync_precedes_sqlite_and_pending_delivery_is_not_an_inbox_commit() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Persist);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    let value = transition(&fixture, 1);
    assert!(writer.try_submit(value.clone()).is_ok());
    hooks.wait(WriterBoundary::Persist, 1);
    assert_eq!(committed(&fixture), 0);
    assert_eq!(
        independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
        vec![value]
    );
    let first = hooks.state.lock().unwrap().first;
    assert!(first[1].unwrap() < first[2].unwrap());
    assert!(first[2].unwrap() < first[3].unwrap());
    assert_eq!(writer.status().pending_count, 1);
    drop(release);
    wait_until(|| committed(&fixture) == 1);
    assert_eq!(writer.finish(deadline()), (0, 0));
}

#[test]
fn runner_batch2_worker_ambiguous_sync_failure_rescans_and_reuses_the_complete_original() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.fail_once(WriterBoundary::Sync);
    hooks.pause(WriterBoundary::Persist);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    let value = transition(&fixture, 1);
    assert!(writer.try_submit(value.clone()).is_ok());
    hooks.wait(WriterBoundary::Persist, 1);
    assert_eq!(
        independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
        vec![value]
    );
    assert_eq!(hooks.calls(WriterBoundary::Append), 1);
    assert!(hooks.calls(WriterBoundary::Sync) >= 2);
    drop(release);
    wait_until(|| committed(&fixture) == 1);
    assert_eq!(writer.finish(deadline()).0, 0);
}

#[test]
fn runner_batch2_worker_append_and_database_failures_retain_the_same_transition_for_retry() {
    for boundary in [WriterBoundary::Append, WriterBoundary::Persist] {
        let fixture = WriterFixture::new();
        let hooks = Arc::new(Hooks::default());
        hooks.fail_once(boundary);
        let mut writer = start(&fixture, hooks.clone());
        let _release = Release(hooks);
        let original = transition(&fixture, 1);
        assert!(writer.try_submit(original.clone()).is_ok());
        wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
        assert_eq!(
            storage::get_event(&fixture.fixture.conn, 1)
                .unwrap()
                .event
                .payload_json["evidence"],
            original.evidence_json
        );
        assert_eq!(committed(&fixture), 1);
        assert_eq!(writer.finish(deadline()).0, 0);
    }
}

#[test]
fn runner_batch2_worker_pending_capacity_counts_channel_and_worker_queue_without_growth() {
    let fixture = WriterFixture::new();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Persist);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
    hooks.wait(WriterBoundary::Persist, 1);
    let mut accepted = 1;
    for sequence in 2..=256 {
        match writer.try_submit(transition(&fixture, sequence)) {
            Ok(()) => accepted += 1,
            Err(retained) => {
                assert_eq!(retained.transition.event_seq, sequence);
                break;
            }
        }
    }
    assert!((64..=128).contains(&accepted));
    assert_eq!(writer.status().pending_count, accepted);
    assert!(writer.status().degraded);
    drop(release);
    wait_until(|| writer.status().pending_count == 0);
    assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
    assert_eq!(writer.finish(deadline()).0, 0);
}

#[test]
fn runner_batch2_worker_refill_is_nonblocking_coalesced_and_published_only_after_commit() {
    let fixture = WriterFixture::new();
    let blocking = crate::db::connect(&format!(
        "sqlite:///{}",
        fixture.fixture.conn.path().unwrap()
    ))
    .unwrap();
    blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
    let hooks = Arc::new(Hooks::default());
    let mut writer = start(&fixture, hooks);
    let begin = Instant::now();
    for _ in 0..10 {
        writer.request_sequence_range().unwrap();
    }
    assert!(begin.elapsed() < Duration::from_millis(250));
    assert!(writer.take_sequence_range().is_none());
    assert_eq!(
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
            .unwrap()
            .event_seq_reserved_through,
        256
    );
    blocking.execute_batch("ROLLBACK").unwrap();
    let mut range = None;
    wait_until(|| {
        range = writer.take_sequence_range();
        range.is_some()
    });
    assert_eq!(range.unwrap(), 257..=512);
    assert!(writer.take_sequence_range().is_none());
    assert_eq!(
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
            .unwrap()
            .event_seq_reserved_through,
        512
    );
    writer.finish(deadline());
}

#[test]
fn runner_batch2_worker_shutdown_retains_synced_uncommitted_frames_and_the_original_budget() {
    let fixture = WriterFixture::new();
    let blocking = crate::db::connect(&format!(
        "sqlite:///{}",
        fixture.fixture.conn.path().unwrap()
    ))
    .unwrap();
    blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Persist);
    let mut writer = start(&fixture, hooks.clone());
    let _release = Release(hooks.clone());
    let value = transition(&fixture, 1);
    assert!(writer.try_submit(value.clone()).is_ok());
    hooks.wait(WriterBoundary::Persist, 1);
    let begin = Instant::now();
    writer.set_deadline(begin + Duration::from_millis(80));
    writer.begin_shutdown(begin + Duration::from_secs(3));
    let pending = writer.finish(begin + Duration::from_secs(3));
    assert!(begin.elapsed() < Duration::from_millis(750));
    assert_eq!(pending.0, 1);
    assert_eq!(committed(&fixture), 0);
    assert_eq!(
        independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
        vec![value]
    );
    blocking.execute_batch("ROLLBACK").unwrap();
}
