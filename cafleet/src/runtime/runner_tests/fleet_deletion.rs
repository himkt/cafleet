use std::time::{Duration, Instant};

use super::*;
use crate::broker::records::Placement;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, DeletionHost, PaneLock, delete_fleet_with_host};
use crate::multiplexer::PostCloseLayout;

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
    failing_pane: Option<String>,
    restored: usize,
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
            failing_pane: None,
            restored: 0,
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

impl DeletionHost for Host {
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
    ) -> Result<PostCloseLayout, CafleetError> {
        assert_eq!(deadline.instant(), self.deadline);
        self.assert_pane_owned(placement);
        self.calls.push("close");
        self.panes.push(placement.clone());
        if self.failing_pane.is_some() && self.failing_pane == placement.mux_pane_id {
            return Err(CafleetError::App("pane vanished".into()));
        }
        if matches!(self.mutation, Mutation::AfterClose) {
            crate::db::connect(&self.settings.database_url)
                .unwrap()
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [self.run_id],
                )
                .unwrap();
        }
        Ok(PostCloseLayout::None)
    }

    fn restore_layout(&mut self, layout: PostCloseLayout) -> Result<(), CafleetError> {
        assert!(matches!(layout, PostCloseLayout::None));
        self.restored += 1;
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

fn fleet_deleted(fixture: &Fixture) -> bool {
    crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
        .unwrap()
        .unwrap()
        .deleted_at
        .is_some()
}

fn outcomes(deleted: &crate::broker::records::DeletedFleet) -> Vec<(i64, String)> {
    deleted
        .members
        .iter()
        .map(|member| (member.member_id, member.pane_status.clone()))
        .collect()
}

fn ordinary_placements_in_lock_order(
    fixture: &Fixture,
    settings: &Settings,
) -> Vec<(i64, Placement)> {
    let director = crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
        .unwrap()
        .unwrap()
        .director_member_id
        .unwrap();
    let mut placements = crate::broker::list_members(&fixture.conn, fixture.run.fleet_id)
        .unwrap()
        .into_iter()
        .filter(|activity| activity.member.member_id != director)
        .filter_map(|activity| Some((activity.member.member_id, activity.member.placement?)))
        .filter(|(_, placement)| placement.mux_pane_id.is_some())
        .collect::<Vec<_>>();
    placements.sort_by_key(|(_, placement)| {
        crate::input::lock_path(&settings.database_url, placement).unwrap()
    });
    placements
}

#[test]
fn fleet_deletion_ends_every_ordinary_member_and_keeps_the_director_pane() {
    let mut fixture = Fixture::new();
    let fleet = fixture.run.fleet_id;
    let unwrapped = common::register(&mut fixture.conn, fleet, "unwrapped", Some("%5"));
    let pending = common::register(&mut fixture.conn, fleet, "pending", None);
    let placementless =
        crate::broker::register_member(&mut fixture.conn, fleet, "bare", "assignment", None)
            .unwrap()
            .member_id;
    let settings = settings(&fixture);
    let deadline = Deadline::new();
    let mut host = Host::new(settings.clone(), fixture.run.run_id, deadline);
    let deleted =
        delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut host).unwrap();
    assert_eq!(deleted.fleet_id, fleet);
    assert_eq!(deleted.deregistered_count, 5);
    let mut expected = vec![
        (fixture.run.member_id, "%2 (killed)".to_owned()),
        (unwrapped, "%5 (killed)".to_owned()),
        (pending, "(pending — no pane)".to_owned()),
        (placementless, "(no placement)".to_owned()),
    ];
    expected.sort();
    assert_eq!(outcomes(&deleted), expected);
    let mut closed = host
        .panes
        .iter()
        .map(|placement| placement.mux_pane_id.clone().unwrap())
        .collect::<Vec<_>>();
    closed.sort();
    assert_eq!(closed, ["%2", "%5"]);
    assert_eq!(host.restored, 2);
    assert!(fleet_deleted(&fixture));
    assert_eq!(
        fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM members WHERE fleet_id=?1 AND status='active'",
                [fleet],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn fleet_deletion_stops_at_the_first_failed_member_and_a_rerun_resumes() {
    let mut fixture = Fixture::new();
    let fleet = fixture.run.fleet_id;
    for (name, pane) in [("first", "%5"), ("second", "%6"), ("third", "%7")] {
        common::register(&mut fixture.conn, fleet, name, Some(pane));
    }
    let settings = settings(&fixture);
    let order = ordinary_placements_in_lock_order(&fixture, &settings);
    assert_eq!(order.len(), 4);
    let failing = if order[1].0 == fixture.run.member_id {
        2
    } else {
        1
    };
    let (failed, failed_placement) = order[failing].clone();
    let failed_pane = failed_placement.mux_pane_id.clone().unwrap();
    let deadline = Deadline::new();
    let mut host = Host::new(settings.clone(), fixture.run.run_id, deadline);
    host.failing_pane = Some(failed_pane.clone());
    let error = delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut host)
        .unwrap_err();
    let ended = order[..failing]
        .iter()
        .map(|(member, _)| member.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        error.message(),
        format!(
            "fleet {fleet} was not deleted: member {failed}: kill_pane failed for pane {failed_pane}: pane vanished. Verify with 'cafleet doctor'. Members already deleted: {ended}."
        )
    );
    assert_eq!(host.panes.len(), failing + 1);
    assert_eq!(host.restored, failing);
    assert!(!fleet_deleted(&fixture));
    for (member, _) in &order[..failing] {
        assert!(
            crate::broker::get_member(&fixture.conn, *member, fleet)
                .unwrap()
                .is_none()
        );
    }
    for (member, placement) in &order[failing..] {
        let current = crate::broker::get_member(&fixture.conn, *member, fleet)
            .unwrap()
            .unwrap();
        assert_eq!(current.placement.as_ref(), Some(placement));
        assert!(
            PaneLock::try_acquire(&settings.database_url, placement)
                .unwrap()
                .is_some()
        );
    }

    let deadline = Deadline::new();
    let mut rerun = Host::new(settings.clone(), fixture.run.run_id, deadline);
    let deleted =
        delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut rerun).unwrap();
    let mut expected = order[failing..]
        .iter()
        .map(|(member, placement)| {
            (
                *member,
                format!("{} (killed)", placement.mux_pane_id.as_deref().unwrap()),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(outcomes(&deleted), expected);
    assert_eq!(deleted.deregistered_count, order.len() - failing + 1);
    assert_eq!(rerun.panes.len(), order.len() - failing);
    assert!(fleet_deleted(&fixture));
}
