use super::*;
use crate::broker::runtime::{self as storage, *};
use crate::runtime::OwnedChild;
use crate::runtime::runner::{ProcessIdentity, RunArtifacts, RunLock, RuntimePaths};
use std::process::{Command, Stdio};

#[test]
fn runner_recovery_member_deletion_terminal_no_work_finishes_under_the_original_guard() {
    run_case("terminal");
}

#[test]
fn runner_recovery_member_deletion_foreign_epoch_after_close_preserves_registration() {
    run_case("epoch");
}

#[test]
fn runner_recovery_member_deletion_foreign_placement_after_close_preserves_registration() {
    run_case("placement");
}

fn run_case(scenario: &str) {
    let dir = tempfile::Builder::new()
        .prefix(".p194-member-delete-")
        .tempdir_in("/tmp")
        .unwrap();
    let log = dir.path().join("child.log");
    let file = std::fs::File::create(&log).unwrap();
    let started = Instant::now();
    let cleanup = started + Duration::from_secs(6);
    let mut child = OwnedChild::new(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cli::member::tests::recovery_deletion::runner_recovery_member_deletion_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CAFLEET_TEST_MEMBER_RECOVERY_CASE", scenario)
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap(),
    );
    let status = child.poll_until(started + Duration::from_secs(5)).unwrap();
    if status.is_none() {
        child.terminate_until(cleanup);
    }
    let diagnostics = std::fs::read_to_string(&log).unwrap();
    assert!(
        status.is_some_and(|status| status.success()),
        "{scenario}: {status:?}\n{diagnostics}"
    );
}

#[test]
#[ignore = "owned isolated member-deletion helper"]
fn runner_recovery_member_deletion_child() {
    let scenario = std::env::var("CAFLEET_TEST_MEMBER_RECOVERY_CASE").unwrap();
    assert!(matches!(
        scenario.as_str(),
        "terminal" | "epoch" | "placement"
    ));
    let _dir = tempfile::Builder::new()
        .prefix(".m194-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut conn = common::migrated_conn(&_dir);
    let settings = Settings {
        database_url: format!("sqlite:///{}", conn.path().unwrap()),
        max_text_len: 200,
        multiplexer: Some("tmux".into()),
    };
    let (fleet, director) = common::create_fleet(&mut conn, "member-recovery");
    let worker = common::register(&mut conn, fleet, "worker", Some("%9"));
    broker::update_placement_pane_id(&mut conn, worker, "%9").unwrap();
    let identity = storage::database_identity(&conn).unwrap();
    let process = ProcessIdentity::current().unwrap();
    let run = storage::create_run(
        &mut conn,
        &NewRun {
            member_id: worker,
            fleet_id: fleet,
            director_member_id: director,
            backend: "claude".into(),
            database_identity_json: identity.clone(),
            launch_nonce: "a".repeat(64),
            runtime_path: RuntimePaths::for_run(&identity, 1)
                .unwrap()
                .run_dir
                .to_str()
                .unwrap()
                .into(),
            requested_mode: None,
            resolved_mode: "default".into(),
            mode_source: ModeSource::Default,
            permission_argv_json: json!(["--permission-mode", "default"]),
            event_evidence: EventEvidence::Metadata,
            native_version: None,
            profile_version: None,
            coverage: Coverage::Unknown,
            owner_pid: process.pid,
            owner_start_identity: process.start_identity.clone(),
        },
    )
    .unwrap();
    let _artifacts = RunArtifacts::create(&mut conn, run.run_id).unwrap();
    let run = storage::get_run(&conn, run.run_id).unwrap();
    let runtime_lock = RunLock::acquire(&conn, &run.owner()).unwrap().unwrap();
    let owner = runtime_lock
        .transfer_to_runner(&mut conn, &run.owner(), process)
        .unwrap();
    conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T20:00:00Z' WHERE run_id=?1",[run.run_id]).unwrap();
    drop(runtime_lock);
    let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
    let mut locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
    let placement = locked.snapshot.placement.clone().unwrap();
    let database_url = settings.database_url.clone();
    let run_id = run.run_id;
    let expected_epoch = owner.owner_epoch;
    let mutation = scenario.clone();
    let runner = Rc::new(CloseRunner {
        calls: Cell::new(0),
        failure: None,
        check: Box::new(move || {
            assert!(
                PaneLock::try_acquire(&database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            let check = crate::db::connect(&database_url).unwrap();
            let current = storage::get_run(&check, run_id).unwrap();
            assert_eq!(
                current.owner_epoch, expected_epoch,
                "a terminal no-work observation advances no owner"
            );
            assert_eq!(
                check
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                        [run_id],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            match mutation.as_str() {
                "terminal" => {}
                "epoch" => {
                    check
                        .execute(
                            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                            [run_id],
                        )
                        .unwrap();
                }
                "placement" => {
                    check.execute("UPDATE member_placements SET mux_pane_id='%foreign' WHERE member_id=?1",[worker]).unwrap();
                }
                _ => unreachable!(),
            }
        }),
    });
    let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()))
        .bounded(deadline);
    let result = close_member(&mut conn, &settings, worker, &mut locked, &mux, deadline);
    assert_eq!(runner.calls.get(), 1);
    if scenario == "terminal" {
        assert_eq!(result.unwrap(), "%9 (killed)");
        assert!(broker::get_member(&conn, worker, fleet).unwrap().is_none());
    } else {
        let error = match result {
            Ok(_) => {
                panic!("foreign {scenario} change preserves registration after the fake pane close")
            }
            Err(error) => error,
        };
        assert!(error.message().contains("closed"), "{error}");
        assert!(broker::get_member(&conn, worker, fleet).unwrap().is_some());
    }
    assert!(
        broker::get_member(&conn, director, fleet)
            .unwrap()
            .is_some()
    );
}
