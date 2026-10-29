use super::observation_services::{count, health, private_write, synthetic, wait_until};
use super::*;

#[test]
fn runner_batch2_service_private_capture_publishes_manifest_last_and_keeps_native_work_running() {
    let mut fixture = ServiceFixture::configured(false, super::observation_services::capture);
    let started = fixture.start_running();
    let request = NativeCaptureArm {
        schema_version: 1,
        capture_id: "service-pair".into(),
        run_id: started.identity.run_id,
        owner_epoch: started.identity.owner_epoch,
        minimum_interval_ms: 100,
        timeout_ms: 2000,
    };
    let arm_at = crate::time::now_utc();
    private_write(
        &fixture.fixture._dir.path().join("capture-arm.json"),
        &serde_json::to_vec(&request).unwrap(),
    );
    let output = fixture.fixture._dir.path().join("capture-output");
    let manifest_path = output.join("service-pair.manifest.json");
    wait_until(|| manifest_path.exists());
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["publication_status"], "complete");
    let artifact = std::fs::read(output.join("service-pair.complete.json"))
        .expect("manifest publication follows complete synced artifact");
    assert_eq!(manifest["artifact_bytes"], artifact.len() as u64);
    assert_eq!(manifest["artifact_sha256"], sha256_hex(&artifact));
    let result: Value = serde_json::from_slice(&artifact).unwrap();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["samples"].as_array().unwrap().len(), 2);
    assert_eq!(result["source"]["producer"]["kind"], "synthetic");
    assert_eq!(
        result["source"]["producer"]["catalog_origin"],
        "bundled_unknown"
    );
    assert_eq!(
        result["source"]["runner"],
        serde_json::to_value(&started.runner).unwrap()
    );
    assert_eq!(
        result["source"]["launch_nonce"],
        started.identity.launch_nonce
    );
    let clock = serde_json::to_string(&result["source"]["clock"])
        .unwrap()
        .to_lowercase();
    assert!(result["source"]["clock"].is_object());
    for attribution in ["real", "instant", "utc", "origin"] {
        assert!(
            clock.contains(attribution),
            "real service clock requires {attribution}: {clock}"
        );
    }
    assert!(!clock.contains("deterministic"));
    let samples = result["samples"].as_array().unwrap();
    let first_end = samples[0]["monotonic_end_ns"].as_u64().unwrap();
    let second_start = samples[1]["monotonic_start_ns"].as_u64().unwrap();
    assert!(second_start.checked_sub(first_end).unwrap() >= 100_000_000);
    let collected_at = crate::time::now_utc();
    for sample in samples {
        assert!(
            sample["monotonic_end_ns"].as_u64().unwrap()
                >= sample["monotonic_start_ns"].as_u64().unwrap()
        );
        let observed =
            chrono::DateTime::parse_from_rfc3339(sample["observed_at"].as_str().unwrap()).unwrap();
        assert!(observed >= arm_at && observed <= collected_at);
    }
    assert_eq!(result["source"]["child"]["pid"], started.child.pid);
    assert_eq!(
        result["source"]["owner_epoch"],
        started.identity.owner_epoch
    );
    assert!(fixture.child.try_wait().unwrap().is_none());
    fixture.health();
    fixture.assert_manual_io();
    fixture.finish();
}

#[test]
fn runner_batch2_service_quiet_timer_persists_one_opening_without_changing_output_sequence() {
    let mut fixture = ServiceFixture::configured(false, synthetic);
    fixture.start_running();
    fixture.master.write_all(b"form-one\r").unwrap();
    fixture.read_until("Operation: write form-one.txt");
    let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
    wait_until(|| count(&fixture, "interaction_opened") == 1);
    let after = health(&mut fixture);
    assert_eq!(after["output_seq"], sequence);
    assert_eq!(after["observation"]["visibility"], "visible");
    assert_eq!(after["observation"]["eligibility"]["state"], "hold");
    let messages:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events e JOIN messages m ON m.message_id=e.message_id WHERE e.run_id=?1 AND e.kind='interaction_opened'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
    assert_eq!(messages, 1);
    fixture.assert_manual_io();
    fixture.finish();
}

#[test]
fn runner_batch2_service_transient_form_and_clear_in_one_native_write_produce_no_episode() {
    let mut fixture = ServiceFixture::configured(false, synthetic);
    fixture.start_running();
    fixture.master.write_all(b"transient\r").unwrap();
    fixture.read_until("working");
    let until = Instant::now() + Duration::from_millis(180);
    while Instant::now() < until {
        let status = health(&mut fixture);
        assert_eq!(status["observation"]["eligibility"]["state"], "hold");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(count(&fixture, "interaction_opened"), 0);
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                [fixture.fixture.run.run_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fixture.assert_manual_io();
    fixture.finish();
}

#[test]
fn runner_batch2_service_sqlite_and_writer_failures_keep_health_manual_display_and_resize_live() {
    for boundary in ["fail-sync", "block-persist"] {
        let mut fixture = ServiceFixture::configured(false, synthetic);
        fixture.start_running();
        let hook = fixture.fixture._dir.path().join(boundary);
        private_write(&hook, b"owned test gate");
        fixture.master.write_all(b"form-pressure\r").unwrap();
        fixture.read_until("Operation: write form-pressure.txt");
        wait_until(|| {
            health(&mut fixture)["observation"]["degraded"] == true
                || fixture.fixture._dir.path().join("persist-entered").exists()
        });
        fixture
            .fixture
            .conn
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        let begin = Instant::now();
        let status = health(&mut fixture);
        assert!(begin.elapsed() < Duration::from_millis(500));
        assert_eq!(status["observation"]["eligibility"]["state"], "hold");
        assert!(status["observation"]["pending_count"].as_u64().unwrap() <= 130);
        fixture.assert_manual_io();
        let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
        rustix::termios::tcsetwinsize(
            &fixture.master,
            rustix::termios::Winsize {
                ws_row: 25,
                ws_col: 90,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        assert!(fixture.process.as_ref().unwrap().matches().unwrap());
        kill(
            Pid::from_raw(i32::try_from(fixture.child.id()).unwrap()),
            Signal::SIGWINCH,
        )
        .unwrap();
        wait_until(|| health(&mut fixture)["output_seq"].as_u64().unwrap() > sequence);
        fixture.fixture.conn.execute_batch("ROLLBACK").unwrap();
        std::fs::remove_file(hook).unwrap();
        wait_until(|| count(&fixture, "interaction_opened") == 1);
        fixture.finish();
    }
}

#[test]
fn runner_batch2_service_shutdown_uses_the_active_controller_to_close_before_truthful_exit() {
    let mut fixture = ServiceFixture::configured(false, synthetic);
    fixture.start_running();
    fixture.master.write_all(b"form-shutdown\r").unwrap();
    fixture.read_until("Operation: write form-shutdown.txt");
    wait_until(|| count(&fixture, "interaction_opened") == 1);
    fixture.finish();
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
    let reason: String = fixture
        .fixture
        .conn
        .query_row(
            "SELECT close_reason FROM member_interactions WHERE run_id=?1",
            [fixture.fixture.run.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason, "process_exited");
}

#[test]
fn runner_batch2_service_existing_reservation_is_refused_before_native_spawn_or_new_grant() {
    let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
        synthetic(fixture, spec);
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_seq_reserved_through=256 WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    });
    let after = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert!(after.child_pid.is_none());
    assert_eq!(after.event_seq_reserved_through, 256);
    assert_eq!(
        fixture
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                [after.run_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(count(&fixture, "interaction_opened"), 0);
    assert!(!fixture.fixture._dir.path().join("native-started").exists());
}

#[test]
fn runner_batch2_service_each_durable_history_table_is_checked_before_initial_reservation() {
    for interaction in [false, true] {
        let mut seeded = None;
        let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
            synthetic(fixture, spec);
            let run = &fixture.run;
            let evidence=storage::evidence::EvidencePolicy::metadata().apply(&json!({"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"rule_id":"runner-lifecycle","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
            if interaction {
                seeded = Some(evidence.to_string());
                fixture.conn.execute("INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json) VALUES(?1,1,'permission_request',?2,'2026-10-26T17:00:00Z',?3)",rusqlite::params![run.run_id,evidence["evidence_digest"].as_str().unwrap(),evidence.to_string()]).unwrap();
            } else {
                let payload = json!({"schema_version":1,"event_id":1,"event_seq":1,"run_id":run.run_id,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,"kind":"coverage_unknown","interaction_id":null,"classification":null,"observed_at":"2026-10-26T17:00:00Z","source":"runner_lifecycle","native_request_id":null,"decision_outcome":"unknown","evidence":evidence,"cause":{"kind":"coverage_unknown"}});
                seeded = Some(payload.to_string());
                fixture.conn.execute("INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,committed_at,payload_json,notification_state,archive_reason) VALUES(?1,1,'coverage_unknown','2026-10-26T17:00:00Z','2026-10-26T17:00:00Z',?2,'archived','member_inactive')",rusqlite::params![run.run_id,payload.to_string()]).unwrap();
            }
        });
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        assert!(run.child_pid.is_none());
        assert_eq!(run.event_seq_reserved_through, 0);
        assert!(!fixture.fixture._dir.path().join("native-started").exists());
        assert_eq!(count(&fixture, "coverage_unknown"), i64::from(!interaction));
        let interactions: i64 = fixture
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1",
                [run.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(interactions, i64::from(interaction));
        let query = if interaction {
            "SELECT evidence_json FROM member_interactions WHERE run_id=?1"
        } else {
            "SELECT payload_json FROM member_runtime_events WHERE run_id=?1"
        };
        let retained: String = fixture
            .fixture
            .conn
            .query_row(query, [run.run_id], |row| row.get(0))
            .unwrap();
        assert_eq!(Some(retained), seeded);
    }
}

#[test]
fn runner_batch2_service_started_health_reports_observation_without_composer_authority() {
    let mut fixture = ServiceFixture::new(false);
    fixture.start_running();
    let mut client = fixture.connect("observation health");
    let reply = client
        .exchange(&fixture.request("health", fixture.prepared.as_ref().unwrap()))
        .unwrap();
    fixture.assert_manual_io();
    fixture.finish();
    assert!(reply.get("error").is_none(), "{reply}");
    let status = &reply["result"]["observation"];
    assert!(
        status.is_object(),
        "started health requires bounded observation status: {reply}"
    );
    assert_eq!(status["eligibility"]["state"], "hold");
    assert!(status["pending_count"].as_u64().unwrap() <= 130);
    assert!(status.get("operation").is_none());
    assert!(status.get("semantic_fingerprint").is_none());
}

#[test]
fn runner_batch2_service_unknown_coverage_emits_once_while_manual_work_continues() {
    let mut fixture = ServiceFixture::new(false);
    fixture.start_running();
    for _ in 0..3 {
        fixture.health();
        fixture.assert_manual_io();
    }
    fixture.finish();
    let count:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='coverage_unknown'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
    assert_eq!(count, 1);
    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.resolved_mode, "never");
    assert!(run.effective_policy_json.is_none());
}

#[test]
fn runner_batch2_service_fallback_profile_emits_no_coverage_unknown_while_no_profile_still_does() {
    for (profile, expected) in [(Some("fallback"), 0), (None, 1)] {
        let mut fixture = ServiceFixture::configured(false, |fixture, _| {
            if profile.is_some() {
                fixture.conn.execute("UPDATE member_runs SET native_version='0.164.0',profile_version='synthetic-profile-1',profile_match=?1,coverage='tested' WHERE run_id=?2",rusqlite::params![profile,fixture.run.run_id]).unwrap();
            } else {
                fixture.conn.execute("UPDATE member_runs SET native_version='0.164.0',profile_version=NULL,profile_match=NULL,coverage='unknown' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
            }
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        });
        fixture.start_running();
        for _ in 0..3 {
            fixture.health();
            fixture.assert_manual_io();
        }
        fixture.finish();
        let count:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='coverage_unknown'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
        assert_eq!(count, expected, "{profile:?}");
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        assert_eq!(
            run.profile_match.map(|value| value.as_str()),
            profile,
            "{profile:?}"
        );
    }
}
