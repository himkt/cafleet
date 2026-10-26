use std::time::{Duration, Instant};

use super::*;
use crate::broker::records::Placement;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, FleetDeletionHost, PaneLock, delete_fleet_with_host};

#[derive(Clone, Copy)]
enum Mutation {
    None,
    AfterShutdown,
    AfterClose,
}

struct Host {
    settings: Settings,
    deadline: Instant,
    run_id: i64,
    mutation: Mutation,
    calls: Vec<&'static str>,
    owners: Vec<(i64, i64)>,
    panes: Vec<Placement>,
}

fn settings(fixture: &Fixture) -> Settings {
    Settings {
        database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
        max_text_len: 200,
        multiplexer: Some("tmux".into()),
    }
}

impl Host {
    fn new(settings: Settings, run_id: i64, deadline: Deadline) -> Self {
        Self {
            settings,
            deadline: deadline.instant(),
            run_id,
            mutation: Mutation::None,
            calls: Vec::new(),
            owners: Vec::new(),
            panes: Vec::new(),
        }
    }

    fn assert_pane_owned(&self, placement: &Placement) {
        assert!(
            PaneLock::try_acquire(&self.settings.database_url, placement)
                .unwrap()
                .is_none()
        );
    }
}

impl FleetDeletionHost for Host {
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        deadline: Deadline,
    ) -> Result<ShutdownOutcome, CafleetError> {
        assert_eq!(deadline.instant(), self.deadline);
        let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        self.assert_pane_owned(&placement);
        self.calls.push("shutdown");
        self.owners.push((run.run_id, run.owner_epoch));
        self.run_id = run.run_id;
        conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T13:00:00Z',reachability='reachable',last_health_at='2026-10-26T13:00:00Z' WHERE run_id=?1", [run.run_id]).unwrap();
        if matches!(self.mutation, Mutation::AfterShutdown) {
            conn.execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [run.run_id],
            )
            .unwrap();
        }
        Ok(ShutdownOutcome {
            native_exit: Some(ChildExit::Code(0)),
            cleanup_state: CleanupState::Complete,
            diagnostics: Vec::new(),
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        })
    }

    fn close_pane(
        &mut self,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<(), CafleetError> {
        assert_eq!(deadline.instant(), self.deadline);
        self.assert_pane_owned(placement);
        self.calls.push("close");
        self.panes.push(placement.clone());
        if matches!(self.mutation, Mutation::AfterClose) {
            crate::db::connect(&self.settings.database_url)
                .unwrap()
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [self.run_id],
                )
                .unwrap();
        }
        Ok(())
    }
}

fn assert_registry_active(fixture: &Fixture) {
    assert!(
        crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_none()
    );
    assert!(
        crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
            .unwrap()
            .is_some()
    );
}

fn while_locks_are_acquired(
    fixture: &mut Fixture,
    change: impl FnOnce(&mut Fixture, &Settings) -> Option<PaneLock>,
) -> (
    Result<crate::broker::records::DeletedFleet, CafleetError>,
    Host,
) {
    let configuration = settings(fixture);
    let mut placements = crate::broker::list_members(&fixture.conn, fixture.run.fleet_id)
        .unwrap()
        .into_iter()
        .filter_map(|activity| activity.member.placement)
        .filter(|placement| placement.mux_pane_id.is_some())
        .collect::<Vec<_>>();
    placements.sort_by_key(|placement| {
        crate::input::lock_path(&configuration.database_url, placement).unwrap()
    });
    assert!(placements.len() >= 2);
    let first = placements.first().unwrap().clone();
    let last = PaneLock::try_acquire(&configuration.database_url, placements.last().unwrap())
        .unwrap()
        .unwrap();
    let path = fixture.conn.path().unwrap().to_owned();
    let fleet_id = fixture.run.fleet_id;
    let run_id = fixture.run.run_id;
    let worker_settings = configuration.clone();
    let started = Instant::now();
    let deadline = Deadline::until(started + Duration::from_secs(1));
    std::thread::scope(|scope| {
        let deletion = scope.spawn(move || {
            let mut conn = Connection::open(path).unwrap();
            let mut host = Host::new(worker_settings.clone(), run_id, deadline);
            let result =
                delete_fleet_with_host(&mut conn, &worker_settings, fleet_id, deadline, &mut host);
            (result, host)
        });
        while PaneLock::try_acquire(&configuration.database_url, &first)
            .unwrap()
            .is_some()
        {
            assert!(
                started.elapsed() < Duration::from_millis(700),
                "deletion did not acquire its first physical lock"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let changed_lock = change(fixture, &configuration);
        drop(last);
        let result = deletion.join().unwrap();
        drop(changed_lock);
        assert!(started.elapsed() < Duration::from_millis(1750));
        result
    })
}

#[test]
fn runner_review_fleet_changed_placement_is_relocked_before_any_destructive_effect() {
    let mut fixture = Fixture::new();
    let (result, host) = while_locks_are_acquired(&mut fixture, |fixture, settings| {
        fixture
            .conn
            .execute(
                "UPDATE member_placements SET mux_pane_id='%changed' WHERE member_id=?1",
                [fixture.run.member_id],
            )
            .unwrap();
        let placement =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
        Some(
            PaneLock::try_acquire(&settings.database_url, &placement)
                .unwrap()
                .unwrap(),
        )
    });
    assert!(result.is_err());
    assert!(
        host.calls.is_empty(),
        "stale deletion effects: {:?}",
        host.calls
    );
    assert_registry_active(&fixture);
}

#[test]
fn runner_review_fleet_changed_owner_on_the_same_pane_uses_only_the_fresh_snapshot() {
    let mut fixture = Fixture::new();
    let old_epoch = fixture.run.owner_epoch;
    let (result, host) = while_locks_are_acquired(&mut fixture, |fixture, _| {
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        None
    });
    result.unwrap();
    assert_eq!(host.owners, [(fixture.run.run_id, old_epoch + 1)]);
    assert_eq!(host.calls, ["shutdown", "close"]);
}

#[test]
fn runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity() {
    let mut fixture = Fixture::new();
    let old_run = fixture.run.clone();
    let mut new_run_id = None;
    let (result, host) = while_locks_are_acquired(&mut fixture, |fixture, _| {
        fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", [old_run.run_id]).unwrap();
        let new = storage::create_run(
            &mut fixture.conn,
            &NewRun {
                member_id: old_run.member_id,
                fleet_id: old_run.fleet_id,
                director_member_id: old_run.director_member_id,
                backend: old_run.backend.clone(),
                database_identity_json: old_run.database_identity_json.clone(),
                launch_nonce: "b".repeat(64),
                runtime_path: fixture
                    ._dir
                    .path()
                    .join("replacement-run")
                    .to_str()
                    .unwrap()
                    .into(),
                requested_mode: old_run.requested_mode.clone(),
                resolved_mode: old_run.resolved_mode.clone(),
                mode_source: old_run.mode_source,
                permission_argv_json: old_run.permission_argv_json.clone(),
                event_evidence: old_run.event_evidence,
                native_version: old_run.native_version.clone(),
                profile_version: old_run.profile_version.clone(),
                coverage: old_run.coverage,
                owner_pid: old_run.owner_pid,
                owner_start_identity: old_run.owner_start_identity.clone(),
            },
        )
        .unwrap();
        new_run_id = Some(new.run_id);
        None
    });
    result.unwrap();
    assert_eq!(host.owners, [(new_run_id.unwrap(), 1)]);
    assert_ne!(host.owners[0].0, old_run.run_id);
    assert_eq!(host.calls, ["shutdown", "close"]);
}

#[test]
fn runner_review_fleet_owner_change_during_shutdown_stops_before_pane_close() {
    let mut fixture = Fixture::new();
    let settings = settings(&fixture);
    let deadline = Deadline::new();
    let mut host = Host::new(settings.clone(), fixture.run.run_id, deadline);
    host.mutation = Mutation::AfterShutdown;
    assert!(
        delete_fleet_with_host(
            &mut fixture.conn,
            &settings,
            fixture.run.fleet_id,
            deadline,
            &mut host
        )
        .is_err()
    );
    assert_eq!(host.calls, ["shutdown"]);
    assert_registry_active(&fixture);
}

#[test]
fn runner_review_fleet_final_identity_change_preserves_registry_without_repeating_effects() {
    let mut fixture = Fixture::new();
    let settings = settings(&fixture);
    let deadline = Deadline::new();
    let mut host = Host::new(settings.clone(), fixture.run.run_id, deadline);
    host.mutation = Mutation::AfterClose;
    assert!(
        delete_fleet_with_host(
            &mut fixture.conn,
            &settings,
            fixture.run.fleet_id,
            deadline,
            &mut host
        )
        .is_err()
    );
    assert_eq!(host.calls, ["shutdown", "close"]);
    assert_registry_active(&fixture);
}

#[test]
fn runner_review_fleet_mutable_shutdown_evidence_keeps_one_deadline_and_unwrapped_behavior() {
    for already_complete in [false, true] {
        let mut fixture = Fixture::new();
        if already_complete {
            fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", [fixture.run.run_id]).unwrap();
        }
        let settings = settings(&fixture);
        let deadline = Deadline::new();
        let mut host = Host::new(settings.clone(), fixture.run.run_id, deadline);
        let deleted = delete_fleet_with_host(
            &mut fixture.conn,
            &settings,
            fixture.run.fleet_id,
            deadline,
            &mut host,
        )
        .unwrap();
        assert_eq!(deleted.deregistered_count, 2);
        assert_eq!(
            host.calls,
            if already_complete {
                vec!["close"]
            } else {
                vec!["shutdown", "close"]
            }
        );
        assert_eq!(host.panes.len(), 1);
        assert_eq!(host.panes[0].mux_pane_id.as_deref(), Some("%2"));
        assert!(
            crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .deleted_at
                .is_some()
        );
    }
}
