use super::support::*;
use super::worker::{Hooks, Release, committed, start, wait_until};
use super::*;
use crate::coding_agent::ProfileCatalog;
use std::os::unix::process::ExitStatusExt;

fn assert_retained_pressure<T>(attempt: Result<T>, status: ObservationStatus) {
    if let Err(cause) = attempt {
        assert!(
            status.retained_transition || status.retained_draft || status.gap_count > 0,
            "observer error requires retained work or a recorded gap: {cause}"
        );
        assert!(
            matches!(status.eligibility, InputEligibility::Hold(_)),
            "observer error holds eligibility: {cause}"
        );
    }
}

#[test]
fn runner_batch2_controller_accepts_only_the_exact_fresh_initial_committed_grant() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let catalog = catalog();
    assert!(
        ObservationController::new(
            &fixture.fixture.run,
            &catalog,
            storage::evidence::EvidencePolicy::metadata(),
            1..=256
        )
        .is_ok()
    );
    for range in [0..=255, 1..=255, 2..=257, 257..=512] {
        assert!(
            ObservationController::new(
                &fixture.fixture.run,
                &catalog,
                storage::evidence::EvidencePolicy::metadata(),
                range
            )
            .is_err()
        );
    }
    for field in [
        "epoch",
        "reservation",
        "status",
        "child",
        "start",
        "end",
        "recovery",
    ] {
        let mut run = fixture.fixture.run.clone();
        match field {
            "epoch" => run.owner_epoch = 0,
            "reservation" => run.event_seq_reserved_through = 512,
            "status" => run.status = RunStatus::Running,
            "child" => run.child_pid = Some(1),
            "start" => run.started_at = Some(NOW.into()),
            "end" => run.ended_at = Some(NOW.into()),
            _ => run.recovery_origin_status = Some(RunStatus::Running),
        }
        assert!(
            ObservationController::new(
                &run,
                &catalog,
                storage::evidence::EvidencePolicy::metadata(),
                1..=256
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn runner_batch2_controller_quiet_poll_opens_one_episode_with_the_unchanged_parser_sequence() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let hooks = Arc::new(Hooks::default());
    let mut writer = start(&fixture, hooks);
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    controller
        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
        .unwrap();
    terminal
        .feed_observed(
            &form("write disposable.txt"),
            &mut controller.observer(&writer, &clock),
        )
        .unwrap();
    let sequence = terminal.snapshot().screen_seq;
    assert!(matches!(
        controller.status().eligibility,
        InputEligibility::Hold(GateReason::Candidate)
    ));
    clock.set(99);
    controller
        .poll(&terminal.snapshot(), &writer, &clock)
        .unwrap();
    assert_eq!(committed(&fixture), 0);
    clock.set(100);
    controller
        .poll(&terminal.snapshot(), &writer, &clock)
        .unwrap();
    wait_until(|| committed(&fixture) == 1);
    let view = storage::get_event(&fixture.fixture.conn, 1).unwrap();
    let interaction = view
        .interaction
        .as_ref()
        .expect("opening links its persisted episode");
    assert_eq!(interaction.run_id, fixture.fixture.run.run_id);
    assert_eq!(interaction.episode_seq, 1);
    let event = &view.event;
    assert_eq!(event.kind, RuntimeEventKind::InteractionOpened);
    assert_eq!(event.interaction_id, Some(interaction.interaction_id));
    assert_eq!(event.payload_json["evidence"]["screen_seq"], sequence);
    assert_eq!(terminal.snapshot().screen_seq, sequence);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_controller_actual_adapter_observes_transient_form_before_clear_without_opening() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let hooks = Arc::new(Hooks::default());
    let mut writer = start(&fixture, hooks);
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    controller
        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
        .unwrap();
    let mut bytes = form("write disposable.txt");
    bytes.extend_from_slice(b"\x1b[2J\x1b[Hworking");
    terminal
        .feed_observed(&bytes, &mut controller.observer(&writer, &clock))
        .unwrap();
    clock.set(100);
    let status = controller
        .poll(&terminal.snapshot(), &writer, &clock)
        .unwrap();
    assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
    assert_eq!(committed(&fixture), 0);
    assert!(!status.retained_transition);
    assert!(!status.retained_draft);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_controller_typed_failures_keep_metadata_only_diagnostics_and_hold_input() {
    for (reason, code) in [
        (ObservationFailure::Parser, "parser_unavailable"),
        (ObservationFailure::Detector, "detector_unavailable"),
        (ObservationFailure::Sequence, "sequence_unavailable"),
        (ObservationFailure::Sink, "sink_unavailable"),
    ] {
        let mut fixture = WriterFixture::new();
        configure(&mut fixture);
        let mut controller = ObservationController::new(
            &fixture.fixture.run,
            &catalog(),
            storage::evidence::EvidencePolicy::metadata(),
            1..=256,
        )
        .unwrap();
        controller.observer_failed(reason, &Clock::new()).unwrap();
        let status = controller.status();
        assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
        assert_eq!(status.diagnostic.as_deref(), Some(code));
        assert!(status.gap_count > 0);
    }
}

#[test]
fn runner_batch2_controller_started_validates_current_identity_and_is_single_use() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let hooks = Arc::new(Hooks::default());
    let mut writer = start(&fixture, hooks);
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let screen = TerminalState::new(TerminalSize { rows: 24, cols: 80 })
        .unwrap()
        .snapshot();
    assert!(
        controller
            .started(&fixture.fixture.run, &screen, &writer, &clock)
            .is_err()
    );
    let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    let mut stale = fixture.fixture.run.clone();
    stale.owner_epoch += 1;
    assert!(
        controller
            .started(&stale, &screen, &writer, &clock)
            .is_err()
    );
    controller
        .started(&fixture.fixture.run, &screen, &writer, &clock)
        .unwrap();
    assert!(
        controller
            .started(&fixture.fixture.run, &screen, &writer, &clock)
            .is_err()
    );
    writer.finish(deadline());
}

#[test]
fn runner_batch2_controller_exit_closes_the_open_episode_before_recording_the_real_native_signal() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let hooks = Arc::new(Hooks::default());
    let mut writer = start(&fixture, hooks);
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let mut child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    controller
        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
        .unwrap();
    terminal
        .feed_observed(
            &form("write disposable.txt"),
            &mut controller.observer(&writer, &clock),
        )
        .unwrap();
    clock.set(100);
    controller
        .poll(&terminal.snapshot(), &writer, &clock)
        .unwrap();
    wait_until(|| committed(&fixture) == 1);
    assert!(child.identity.matches().unwrap());
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(child.identity.pid).unwrap()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let until = deadline();
    let exit = loop {
        if let Some(exit) = child.child.try_wait().unwrap() {
            break exit;
        }
        assert!(Instant::now() < until, "owned child signal exit");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(exit.signal(), Some(15));
    clock.set(200);
    controller
        .child_exited(ChildExit::Signal(15), &terminal.snapshot(), &writer, &clock)
        .unwrap();
    wait_until(|| committed(&fixture) == 3);
    let kinds: Vec<String> = fixture
        .fixture
        .conn
        .prepare("SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq")
        .unwrap()
        .query_map([fixture.fixture.run.run_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        kinds,
        ["interaction_opened", "interaction_closed", "runner_exited"]
    );
    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.exit_signal, Some(15));
    assert_eq!(run.exit_code, None);
    writer.finish(deadline());
}

#[test]
fn runner_batch2_controller_refills_shared_lifecycle_sequences_only_from_committed_worker_grants() {
    let until = Instant::now() + Duration::from_secs(15);
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let mut writer = start(&fixture, Arc::new(Hooks::default()));
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    controller
        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
        .unwrap();
    let mut millis = 0u64;
    let mut original_sequences = Vec::new();
    for index in 0..130 {
        assert!(Instant::now() < until, "bounded controller refill fixture");
        clock.set(millis);
        terminal
            .feed_observed(
                &form(&format!("write range-{index}.txt")),
                &mut controller.observer(&writer, &clock),
            )
            .unwrap();
        original_sequences.push(terminal.snapshot().screen_seq);
        millis = millis.checked_add(100).unwrap();
        clock.set(millis);
        let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
        assert_retained_pressure(attempt, controller.status());
        wait_until(|| {
            assert!(Instant::now() < until, "original refill fixture deadline");
            millis = millis.checked_add(1).unwrap();
            clock.set(millis);
            let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
            assert_retained_pressure(attempt, controller.status());
            let openings:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
            openings == i64::from(index + 1)
                && !controller.status().retained_transition
                && !controller.status().retained_draft
        });
        millis = millis.checked_add(100).unwrap();
    }
    wait_until(|| {
        millis = millis.checked_add(1).unwrap();
        clock.set(millis);
        let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
        assert_retained_pressure(attempt, controller.status());
        writer.status().pending_count == 0
            && !controller.status().retained_transition
            && !controller.status().retained_draft
            && controller.status().diagnostic.is_none()
    });
    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert!(run.event_seq_reserved_through >= 512);
    assert_eq!(run.event_seq_reserved_through % 256, 0);
    let ids: Vec<i64> = fixture
        .fixture
        .conn
        .prepare("SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq")
        .unwrap()
        .query_map([run.run_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        ids.iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        ids.len()
    );
    let mut last = 0;
    let mut interactions = Vec::new();
    let mut gap = false;
    for id in ids {
        let view = storage::get_event(&fixture.fixture.conn, id).unwrap();
        let event = &view.event;
        assert!(event.event_seq > last && event.event_seq <= run.event_seq_reserved_through);
        last = event.event_seq;
        match event.kind {
            RuntimeEventKind::InteractionOpened => {
                let sequence = view.interaction.as_ref().unwrap().episode_seq;
                assert_eq!(
                    event.payload_json["evidence"]["screen_seq"],
                    original_sequences[usize::try_from(sequence - 1).unwrap()]
                );
                interactions.push((event.kind, sequence));
            }
            RuntimeEventKind::InteractionClosed => {
                assert_eq!(event.payload_json["close_reason"], "superseded");
                interactions.push((event.kind, view.interaction.as_ref().unwrap().episode_seq));
            }
            RuntimeEventKind::RunnerDegraded => {
                let cause = &event.payload_json["cause"];
                assert!(cause["gap_count"].as_u64().is_some());
                assert!(cause["gap_started_at"].is_string());
                let kind = cause["kind"].as_str().unwrap();
                if kind == "storage_recovered" {
                    assert!(gap);
                    assert!(cause["gap_ended_at"].is_string());
                    gap = false;
                } else {
                    assert!(matches!(kind, "storage_pressure" | "observer_error"));
                    assert!(!gap);
                    gap = true;
                }
            }
            other => panic!("unexpected refill event: {other:?}"),
        }
    }
    let mut expected = Vec::new();
    for episode in 1..=130 {
        if episode > 1 {
            expected.push((RuntimeEventKind::InteractionClosed, episode - 1));
        }
        expected.push((RuntimeEventKind::InteractionOpened, episode));
    }
    assert_eq!(interactions, expected);
    assert!(!gap);
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1",
                [run.run_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        130
    );
    writer.finish(deadline());
}

#[test]
fn runner_batch2_controller_retains_both_superseded_originals_when_the_first_submit_hits_pressure()
{
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let mut definitions = definitions();
    let mut refusal = definitions["profiles"][0]["rules"][0].clone();
    refusal["id"] = serde_json::json!("synthetic-refusal");
    refusal["observation"] = serde_json::json!("permission_denied");
    refusal["verified_noninteraction"] = serde_json::json!(true);
    refusal["region"]["start_pattern"] = serde_json::json!("^Synthetic refusal result$");
    definitions["profiles"][0]["rules"]
        .as_array_mut()
        .unwrap()
        .push(refusal);
    definitions["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
        serde_json::json!(["synthetic-permission", "synthetic-refusal"]);
    let catalog = ProfileCatalog::from_definitions(&definitions).unwrap();
    let hooks = Arc::new(Hooks::default());
    hooks.pause(WriterBoundary::Receive);
    let mut writer = start(&fixture, hooks.clone());
    let release = Release(hooks.clone());
    hooks.wait(WriterBoundary::Receive, 1);
    let clock = Clock::new();
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog,
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
    running(&mut fixture, &child.identity);
    let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
    controller
        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
        .unwrap();
    for index in 0..32 {
        clock.set(index * 200);
        terminal
            .feed_observed(
                &form(&format!("write fixture-{index:02}.txt")),
                &mut controller.observer(&writer, &clock),
            )
            .unwrap();
        clock.set(index * 200 + 100);
        controller
            .poll(&terminal.snapshot(), &writer, &clock)
            .unwrap();
    }
    assert_eq!(writer.status().pending_count, 63);
    let refusal = String::from_utf8(form("write fixture-31.txt"))
        .unwrap()
        .replace("Synthetic permission request", "Synthetic refusal result");
    clock.set(6400);
    terminal
        .feed_observed(
            refusal.as_bytes(),
            &mut controller.observer(&writer, &clock),
        )
        .unwrap();
    clock.set(6500);
    controller
        .poll(&terminal.snapshot(), &writer, &clock)
        .unwrap();
    assert_eq!(writer.status().pending_count, 64);
    clock.set(6550);
    terminal
        .feed_observed(
            &form("write fixture-31.txt"),
            &mut controller.observer(&writer, &clock),
        )
        .unwrap();
    clock.set(6600);
    terminal
        .feed_observed(
            &form("write final-original.txt"),
            &mut controller.observer(&writer, &clock),
        )
        .unwrap();
    clock.set(6700);
    let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
    assert_retained_pressure(attempt, controller.status());
    let original_sequence = terminal.snapshot().screen_seq;
    assert!(controller.status().retained_transition);
    assert!(matches!(
        controller.status().eligibility,
        InputEligibility::Hold(_)
    ));
    let attempt = terminal.feed_observed(
        b"\x1b[2J\x1b[Hlater output during the gap",
        &mut controller.observer(&writer, &clock),
    );
    assert_retained_pressure(attempt, controller.status());
    drop(release);
    wait_until(|| {
        let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
        assert_retained_pressure(attempt, controller.status());
        committed(&fixture) >= 66
    });
    let event = fixture
        .fixture
        .conn
        .query_row(
            "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=66",
            [fixture.fixture.run.run_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let original = storage::get_event(&fixture.fixture.conn, event)
        .unwrap()
        .event;
    assert_eq!(original.kind, RuntimeEventKind::InteractionOpened);
    assert_eq!(
        original.payload_json["evidence"]["screen_seq"],
        original_sequence
    );
    writer.finish(deadline());
}
