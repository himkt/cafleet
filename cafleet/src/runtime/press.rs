use std::time::Duration;

use rusqlite::Connection;

use crate::broker::{
    self,
    records::{MemberKind, MemberRecord},
};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::multiplexer::{AnyMultiplexer, PressKeys};

pub(crate) struct KeyAcknowledgment {
    pub member_id: i64,
    pub pane_id: String,
    pub keys: Vec<String>,
}

fn effect_error(member_id: i64, cause: CafleetError, started: bool) -> CafleetError {
    let diagnostic = if started {
        format!(
            "key delivery to member {member_id} is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry."
        )
    } else {
        format!("keys were not dispatched to member {member_id}.")
    };
    CafleetError::WithDiagnostics {
        cause: Box::new(cause),
        diagnostics: vec![diagnostic],
    }
}

fn load_member(conn: &Connection, member_id: i64) -> Result<MemberRecord, CafleetError> {
    let fleet = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    broker::get_member(conn, member_id, fleet)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))
}

pub(crate) fn press_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &PressKeys,
    deadline: Deadline,
) -> Result<KeyAcknowledgment, CafleetError> {
    deadline
        .remaining()
        .map_err(|cause| effect_error(member_id, cause, false))?;
    let mux = super::resolve_mux(settings)
        .map_err(|cause| effect_error(member_id, CafleetError::App(cause.to_string()), false))?;
    press_member_with_mux(conn, settings, member_id, keys, deadline, &mux)
}

pub(crate) fn press_member_with_mux(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &PressKeys,
    deadline: Deadline,
    mux: &AnyMultiplexer,
) -> Result<KeyAcknowledgment, CafleetError> {
    let mut started = false;
    let result = (|| {
        let member = with_database_budget(conn, deadline, |conn| load_member(conn, member_id))?;
        if member.kind == MemberKind::Director {
            return Err(CafleetError::App(
                "cannot press keys in the Director's own pane".into(),
            ));
        }
        let placement = member.placement.as_ref().ok_or_else(|| CafleetError::App(format!("member {member_id} has no placement row; it was not spawned via `cafleet member create`.")))?;
        let pane_id = placement.mux_pane_id.as_ref().ok_or_else(|| {
            CafleetError::App(format!(
                "member {member_id} has no pane yet (pending placement) — nothing to press."
            ))
        })?;
        if mux.name() != placement.backend {
            return Err(CafleetError::App(
                "press backend does not match the recorded placement".into(),
            ));
        }
        let _span = crate::signals::DeferTermination::begin()?;
        let locked = crate::input::lock_member_with_reservation(
            conn,
            settings,
            &member,
            deadline,
            Duration::from_secs(1),
        )?
        .ok_or_else(|| {
            CafleetError::App(format!("member {member_id}'s pane input remained busy"))
        })?;
        if locked.member.fleet_id != member.fleet_id
            || locked.member.kind != member.kind
            || locked.member.placement != member.placement
        {
            return Err(CafleetError::App(format!(
                "member {member_id} placement changed before key dispatch"
            )));
        }
        mux.validate_key_target(placement, deadline)
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        deadline.remaining()?;
        let current = with_database_budget(conn, deadline, |conn| load_member(conn, member_id))?;
        if current.fleet_id != member.fleet_id
            || current.kind != member.kind
            || current.placement != member.placement
        {
            return Err(CafleetError::App(format!(
                "member {member_id} placement changed before key dispatch"
            )));
        }
        locked.pane_lock(&settings.database_url)?;
        deadline.remaining()?;
        mux.send_keys(pane_id, keys, deadline).map_err(|cause| {
            started = cause.submission_started();
            CafleetError::App(cause.to_string())
        })?;
        Ok(KeyAcknowledgment {
            member_id,
            pane_id: pane_id.clone(),
            keys: keys.canonical(),
        })
    })();
    result.map_err(|cause| effect_error(member_id, cause, started))
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::broker::records::Placement;
    use crate::config::Settings;
    use crate::error::CafleetError;
    use crate::input::{Deadline, PaneLock};
    use crate::multiplexer::{
        AnyMultiplexer, CommandRunner, PressKeys, RunError, TimedRunError, TmuxMultiplexer,
    };
    use crate::runtime::runner::test_support::Fixture;
    use nix::sys::signal::{SigSet, Signal};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, Debug)]
    enum Mode {
        Success,
        ChangePlacement,
        ExpireMetadata,
        NotStarted,
        StartedFailure,
    }

    struct Probe {
        database_url: String,
        placement: Placement,
        member_id: i64,
        until: Instant,
        mode: Mode,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Probe {
        fn writes(&self) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|args| args[1] == "send-keys")
                .count()
        }
    }

    impl CommandRunner for Probe {
        fn binary_exists(&self, name: &str) -> bool {
            assert_eq!(name, "tmux");
            true
        }
        fn run(&self, _: &[String], _: Option<u64>) -> Result<String, RunError> {
            panic!("press probes and write retain absolute deadlines")
        }
        fn run_for(&self, _: &[String], _: Duration) -> Result<String, RunError> {
            panic!("press does not renew a relative phase budget")
        }
        fn sleep(&self, _: f64) {
            panic!("explicit keys have no prompt settling phase")
        }
        fn run_until(&self, args: &[String], until: Instant) -> Result<String, TimedRunError> {
            assert_eq!(until, self.until);
            assert!(
                Instant::now() < until,
                "no new process is offered after expiry"
            );
            assert!(
                PaneLock::try_acquire(&self.database_url, &self.placement)
                    .unwrap()
                    .is_none(),
                "the original physical pane lock remains held through every process call"
            );
            let mask = SigSet::thread_get_mask().unwrap();
            for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
                assert!(
                    mask.contains(signal),
                    "termination is deferred while press owns input"
                );
            }
            self.calls.borrow_mut().push(args.to_vec());
            assert_eq!(args[0], "tmux");
            match args[1].as_str() {
                "display-message" => match args.last().unwrap().as_str() {
                    "#{session_name}|#{window_id}|#{pane_id}" => Ok("main|@1|%0\n".into()),
                    "#{pane_id}|#{window_id}|#{session_name}" => {
                        assert!(args.iter().any(|value| value == "%2"));
                        match self.mode {
                            Mode::ChangePlacement => {
                                let conn = crate::db::connect(&self.database_url).unwrap();
                                assert_eq!(conn.execute(
                                    "UPDATE member_placements SET coding_agent='claude' WHERE member_id=?1",
                                    [self.member_id],
                                ).unwrap(), 1);
                            }
                            Mode::ExpireMetadata => std::thread::sleep(
                                until.saturating_duration_since(Instant::now())
                                    + Duration::from_millis(1),
                            ),
                            _ => {}
                        }
                        Ok("%2|@1|main\n".into())
                    }
                    other => panic!("unexpected metadata query: {other}"),
                },
                "send-keys" => {
                    assert_eq!(args, &["tmux", "send-keys", "-t", "%2", "Tab"]);
                    assert_eq!(self.writes(), 1, "a started failure is never retried");
                    match self.mode {
                        Mode::Success => Ok(String::new()),
                        Mode::NotStarted => Err(TimedRunError {
                            source: RunError::NotStarted("owned prelaunch refusal".into()),
                            cleanup_diagnostics: vec![],
                        }),
                        Mode::StartedFailure => Err(TimedRunError {
                            source: RunError::Failed {
                                stderr: "can't find pane: owned started failure".into(),
                            },
                            cleanup_diagnostics: vec!["owned cleanup diagnostic".into()],
                        }),
                        Mode::ChangePlacement | Mode::ExpireMetadata => {
                            panic!("post-metadata refusal precedes writing")
                        }
                    }
                }
                other => panic!("unexpected press process: {other}"),
            }
        }
    }

    fn settings(fixture: &Fixture) -> Settings {
        Settings::for_database(
            std::path::Path::new(fixture.conn.path().unwrap()),
            Some("tmux"),
        )
    }

    fn setup(
        fixture: &Fixture,
        mode: Mode,
        until: Instant,
    ) -> (Settings, Rc<Probe>, AnyMultiplexer) {
        let settings = settings(fixture);
        let member =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap();
        assert!(member.runtime.is_some());
        let probe = Rc::new(Probe {
            database_url: settings.database_url.clone(),
            placement: member.placement.unwrap(),
            member_id: member.member_id,
            until,
            mode,
            calls: RefCell::default(),
        });
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            probe.clone(),
            HashMap::from([
                ("TMUX".into(), "owned-fixture".into()),
                ("TMUX_PANE".into(), "%0".into()),
            ]),
        ));
        (settings, probe, mux)
    }

    fn keys() -> PressKeys {
        PressKeys::parse(&["tAb".into()]).unwrap()
    }

    fn failure(result: Result<KeyAcknowledgment, CafleetError>) -> CafleetError {
        match result {
            Ok(_) => panic!("this press attempt is refused"),
            Err(cause) => cause,
        }
    }

    fn primary(cause: &CafleetError) -> &CafleetError {
        match cause {
            CafleetError::WithDiagnostics { cause, .. } => primary(cause),
            other => other,
        }
    }

    fn assert_zero_effect(cause: &CafleetError, member: i64) {
        assert_eq!(cause.exit_code(), 1);
        assert!(
            cause
                .message()
                .ends_with(&format!("keys were not dispatched to member {member}.")),
            "{cause}"
        );
    }

    fn acquire_released(database_url: &str, placement: &Placement) -> bool {
        PaneLock::acquire_released(database_url, placement)
            .unwrap()
            .is_some()
    }

    fn post_refusal_lock_diagnostic(probe: &Probe) -> String {
        use std::os::unix::fs::MetadataExt;

        let path = crate::input::lock_path(&probe.database_url, &probe.placement);
        let metadata = path.as_ref().map(|path| {
            std::fs::symlink_metadata(path).map(|metadata| {
                format!(
                    "mode={:o}; dev={}; inode={}; symlink={}",
                    metadata.mode(),
                    metadata.dev(),
                    metadata.ino(),
                    metadata.file_type().is_symlink(),
                )
            })
        });
        #[cfg(target_os = "macos")]
        let open_processes = path.as_ref().map(|path| {
            libproc::processes::pids_by_path(path, false, false).map(|mut pids| {
                pids.sort_unstable();
                let total = pids.len();
                let sampled = pids
                    .into_iter()
                    .take(16)
                    .map(|pid| {
                        let info = libproc::libproc::proc_pid::pidinfo::<
                            libproc::libproc::bsd_info::BSDInfo,
                        >(i32::try_from(pid).unwrap(), 0)
                        .map(|info| (info.pbi_ppid, info.pbi_start_tvsec, info.pbi_start_tvusec));
                        (pid, info)
                    })
                    .collect::<Vec<_>>();
                (total, sampled)
            })
        });
        #[cfg(not(target_os = "macos"))]
        let open_processes = "path-filtered process observation is available on macOS";
        format!(
            "case={:?}; parent_pid={}; placement={:?}; lock_path={path:?}; named={metadata:?}; path_open_processes_total_and_pid_ppid_start={open_processes:?}; snapshot=after_refusal; open_file_observation_is_not_lock_ownership_proof",
            probe.mode,
            std::process::id(),
            probe.placement,
        )
    }

    #[test]
    fn step5_press_entry_uses_the_outer_wrapped_placement_and_held_lock_under_one_deadline() {
        let mut fixture = Fixture::new();
        let until = Instant::now() + Duration::from_secs(2);
        let (settings, probe, mux) = setup(&fixture, Mode::Success, until);
        let before = crate::broker::runtime::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let value = press_member_with_mux(
            &mut fixture.conn,
            &settings,
            fixture.run.member_id,
            &keys(),
            Deadline::until(until),
            &mux,
        )
        .unwrap();
        assert_eq!(value.member_id, fixture.run.member_id);
        assert_eq!(value.pane_id, "%2");
        assert_eq!(value.keys, ["Tab"]);
        assert_eq!(probe.calls.borrow().len(), 3);
        assert_eq!(probe.writes(), 1);
        assert_eq!(
            crate::broker::runtime::get_run(&fixture.conn, fixture.run.run_id).unwrap(),
            before
        );
        assert_eq!(
            fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert!(acquire_released(&settings.database_url, &probe.placement));
    }

    #[test]
    fn step5_press_entry_refuses_director_backend_mismatch_and_a_busy_shared_lock_before_probes() {
        let mut fixture = Fixture::new();
        let until = Instant::now() + Duration::from_secs(2);
        let (settings, probe, mux) = setup(&fixture, Mode::Success, until);
        let director = failure(press_member_with_mux(
            &mut fixture.conn,
            &settings,
            fixture.run.director_member_id,
            &keys(),
            Deadline::until(until),
            &mux,
        ));
        assert_eq!(
            primary(&director).message(),
            "cannot press keys in the Director's own pane"
        );
        assert_zero_effect(&director, fixture.run.director_member_id);
        assert!(probe.calls.borrow().is_empty());

        fixture
            .conn
            .execute(
                "UPDATE member_placements SET backend='herdr' WHERE member_id=?1",
                [fixture.run.member_id],
            )
            .unwrap();
        let mismatch = failure(press_member_with_mux(
            &mut fixture.conn,
            &settings,
            fixture.run.member_id,
            &keys(),
            Deadline::until(until),
            &mux,
        ));
        assert_eq!(
            primary(&mismatch).message(),
            "press backend does not match the recorded placement"
        );
        assert_zero_effect(&mismatch, fixture.run.member_id);
        assert!(probe.calls.borrow().is_empty());
        fixture
            .conn
            .execute(
                "UPDATE member_placements SET backend='tmux' WHERE member_id=?1",
                [fixture.run.member_id],
            )
            .unwrap();

        let held = PaneLock::try_acquire(&settings.database_url, &probe.placement)
            .unwrap()
            .unwrap();
        let busy_until = Instant::now() + Duration::from_millis(1100);
        let busy = failure(press_member_with_mux(
            &mut fixture.conn,
            &settings,
            fixture.run.member_id,
            &keys(),
            Deadline::until(busy_until),
            &mux,
        ));
        assert_eq!(
            primary(&busy).message(),
            format!(
                "member {}'s pane input remained busy",
                fixture.run.member_id
            )
        );
        assert_zero_effect(&busy, fixture.run.member_id);
        assert!(probe.calls.borrow().is_empty());
        drop(held);
        assert!(acquire_released(&settings.database_url, &probe.placement));
    }

    #[test]
    fn step5_press_entry_rechecks_full_placement_and_original_expiry_after_actual_metadata() {
        for mode in [Mode::ChangePlacement, Mode::ExpireMetadata] {
            let mut fixture = Fixture::new();
            let until = Instant::now() + Duration::from_secs(2);
            let (settings, probe, mux) = setup(&fixture, mode, until);
            let cause = failure(press_member_with_mux(
                &mut fixture.conn,
                &settings,
                fixture.run.member_id,
                &keys(),
                Deadline::until(until),
                &mux,
            ));
            assert_eq!(probe.calls.borrow().len(), 2);
            assert_eq!(probe.writes(), 0);
            assert_zero_effect(&cause, fixture.run.member_id);
            match mode {
                Mode::ChangePlacement => assert_eq!(
                    primary(&cause).message(),
                    format!(
                        "member {} placement changed before key dispatch",
                        fixture.run.member_id
                    )
                ),
                Mode::ExpireMetadata => {
                    assert!(matches!(primary(&cause), CafleetError::DeadlineExceeded))
                }
                _ => unreachable!(),
            }
            assert!(
                acquire_released(&settings.database_url, &probe.placement),
                "post-refusal pane lock is released: {}",
                post_refusal_lock_diagnostic(&probe),
            );
        }
    }

    #[test]
    fn step5_press_entry_keeps_prelaunch_and_started_effects_distinct_with_cleanup_before_effect() {
        for mode in [Mode::NotStarted, Mode::StartedFailure] {
            let mut fixture = Fixture::new();
            let until = Instant::now() + Duration::from_secs(2);
            let (settings, probe, mux) = setup(&fixture, mode, until);
            let cause = failure(press_member_with_mux(
                &mut fixture.conn,
                &settings,
                fixture.run.member_id,
                &keys(),
                Deadline::until(until),
                &mux,
            ));
            assert_eq!(probe.calls.borrow().len(), 3);
            assert_eq!(probe.writes(), 1);
            assert_eq!(cause.exit_code(), 1);
            match mode {
                Mode::NotStarted => {
                    assert!(
                        primary(&cause)
                            .message()
                            .contains("owned prelaunch refusal")
                    );
                    assert_zero_effect(&cause, fixture.run.member_id);
                }
                Mode::StartedFailure => {
                    let text = cause.message();
                    let primary_position = text
                        .find("owned started failure")
                        .expect("primary backend failure");
                    let cleanup_position = text
                        .find("owned cleanup diagnostic")
                        .expect("cleanup retained");
                    let effect = format!(
                        "key delivery to member {} is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.",
                        fixture.run.member_id
                    );
                    let effect_position = text
                        .find(&effect)
                        .expect("started effects stay uncertain even for pane-gone stderr");
                    assert!(
                        primary_position < cleanup_position && cleanup_position < effect_position
                    );
                    assert!(text.ends_with(&effect));
                    assert_eq!(text.matches(&effect).count(), 1);
                }
                _ => unreachable!(),
            }
            assert!(acquire_released(&settings.database_url, &probe.placement));
        }
    }
}
