mod common;

use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, Output};
use std::time::{Duration, Instant};

use common::{Cli, code, stderr, stdout, write_executable};
use sha2::{Digest, Sha256};

const INPUT_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
stage=other
case "$1" in
    capture-pane) stage=capture ;;
    send-keys)
        case "$4" in
            -l) stage=literal ;;
            Enter) stage=enter ;;
            Escape) stage=escape ;;
        esac
        ;;
esac
if [ "$stage" = "$CAFLEET_TEST_GATE_STAGE" ]; then
    printf '%s\n' "$stage" > "$CAFLEET_TEST_ENTERED"
    attempts=0
    while [ ! -e "$CAFLEET_TEST_RELEASE" ] && [ "$attempts" -lt 700 ]; do
        /bin/sleep 0.01
        attempts=$((attempts + 1))
    done
fi
case "$stage" in
    escape) delay="$CAFLEET_TEST_ESCAPE_DELAY" ;;
    literal) delay="$CAFLEET_TEST_LITERAL_DELAY" ;;
    enter) delay="$CAFLEET_TEST_ENTER_DELAY" ;;
    *) delay= ;;
esac
if [ -n "$delay" ]; then /bin/sleep "$delay"; fi
if [ "$stage" = "$CAFLEET_TEST_FAIL_STAGE" ]; then
    printf 'injected %s transport failure\n' "$stage" >&2
    exit 1
fi
case "$1" in
    display-message) printf 'main|@1|%%0\n' ;;
    capture-pane)
        while [ "$#" -gt 0 ]; do
            if [ "$1" = "-t" ]; then
                shift
                if [ ! -f "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$1" ]; then
                    echo "unexpected capture-pane" >&2
                    exit 1
                fi
                while IFS= read -r line || [ -n "$line" ]; do
                    printf '%s\n' "$line"
                done < "$CAFLEET_TEST_TMUX_CAPTURE_DIR/$1"
                exit 0
            fi
            shift
        done
        exit 2
        ;;
    list-panes) printf '%%0\n%%6\n%%7\n%%8\n' ;;
esac
"#;

struct InputFixture {
    cli: Cli,
    fleet: i64,
    director: i64,
    member: i64,
    pane: String,
}

impl InputFixture {
    fn new() -> Self {
        let cli = Cli::new();
        let (fleet, director) = cli.seeded_fleet();
        let member = cli.seed_member(fleet, "worker");
        let pane = format!("%{}", member + 4);
        write_executable(&cli.shim_dir.join("tmux"), INPUT_SHIM);
        Self {
            cli,
            fleet,
            director,
            member,
            pane,
        }
    }

    fn action(&self, action: &str) -> Vec<String> {
        if action == "preview" {
            return vec![
                "message".into(),
                "send".into(),
                "--from-member-id".into(),
                self.director.to_string(),
                "--to-member-id".into(),
                self.member.to_string(),
                "preview body".into(),
                "--json".into(),
            ];
        }
        let mut args = vec!["member".into(), action.into(), self.member.to_string()];
        args.push(
            if action == "exec" {
                "printf example"
            } else {
                "hello worker"
            }
            .into(),
        );
        args.push("--json".into());
        args
    }

    fn run(&self, action: &str) -> Output {
        let args = self.action(action);
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        bounded_output(self.cli.spawn(&refs))
    }

    fn lock_path(&self) -> PathBuf {
        placement_lock(&self.cli, "tmux", "main", &self.pane)
    }

    fn hold_lock(&self) -> File {
        hold_lock(&self.lock_path())
    }

    fn observe_lock(&self) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.lock_path())
            .unwrap()
    }

    fn gate(&mut self, action: &str, stage: &str) -> InputChild {
        let entered = self.cli.home.path().join("entered");
        let release = self.cli.home.path().join("release");
        self.cli.set_env("CAFLEET_TEST_GATE_STAGE", stage);
        self.cli
            .set_env("CAFLEET_TEST_ENTERED", entered.to_str().unwrap());
        self.cli
            .set_env("CAFLEET_TEST_RELEASE", release.to_str().unwrap());
        let args = self.action(action);
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        let mut owner = InputChild {
            child: Some(self.cli.spawn(&refs)),
            release,
        };
        self.cli.extra_env.retain(|(key, _)| {
            !matches!(
                key.as_str(),
                "CAFLEET_TEST_GATE_STAGE" | "CAFLEET_TEST_ENTERED" | "CAFLEET_TEST_RELEASE"
            )
        });
        let deadline = Instant::now() + Duration::from_millis(5800);
        while !entered.exists() {
            assert!(Instant::now() < deadline, "writer never reached {stage}");
            if owner.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = owner.child.take().unwrap().wait_with_output().unwrap();
                panic!("writer exited before {stage}: {}", stderr(&output));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        owner
    }

    fn send(&self, text: &str) -> Output {
        bounded_output(self.cli.spawn(&[
            "message",
            "send",
            "--from-member-id",
            &self.director.to_string(),
            "--to-member-id",
            &self.member.to_string(),
            text,
            "--json",
        ]))
    }

    fn assert_active(&self) {
        let conn = self.cli.sqlite();
        let active: String = conn
            .query_row(
                "SELECT status FROM members WHERE member_id=?1",
                [self.member],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, "active");
        let placement: Option<String> = conn
            .query_row(
                "SELECT mux_pane_id FROM member_placements WHERE member_id=?1",
                [self.member],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placement.as_deref(), Some(self.pane.as_str()));
        let fleet: Option<String> = conn
            .query_row(
                "SELECT deleted_at FROM fleets WHERE fleet_id=?1",
                [self.fleet],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fleet, None);
    }
}

struct InputChild {
    child: Option<Child>,
    release: PathBuf,
}

impl InputChild {
    fn release(mut self) -> Output {
        std::fs::write(&self.release, b"release").unwrap();
        bounded_output(self.child.take().unwrap())
    }

    fn kill(&mut self) {
        let child = self.child.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        std::fs::write(&self.release, b"release").unwrap();
    }
}

impl Drop for InputChild {
    fn drop(&mut self) {
        std::fs::write(&self.release, b"release").unwrap();
        if let Some(mut child) = self.child.take() {
            if child.try_wait().unwrap().is_none() {
                child.kill().unwrap();
            }
            child.wait().unwrap();
        }
    }
}

fn bounded_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(9);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("CLI exceeded test cleanup bound: {}", stderr(&output));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn placement_lock(cli: &Cli, backend: &str, session: &str, pane: &str) -> PathBuf {
    let database = cli.db_path().canonicalize().unwrap();
    let mut directory = database.into_os_string();
    directory.push(".pane-locks");
    let digest = Sha256::digest(format!("{backend}\0{session}\0{pane}").as_bytes());
    let key: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    PathBuf::from(directory).join(format!("{key}.lock"))
}

fn hold_lock(path: &Path) -> File {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.try_lock().unwrap();
    file
}

fn json(output: &Output) -> serde_json::Value {
    assert_eq!(code(output), 0, "{}", stderr(output));
    serde_json::from_str(&stdout(output)).unwrap()
}

fn assert_no_input(calls: &[String]) {
    assert!(
        calls.iter().all(|line| !line.starts_with("send-keys")),
        "{calls:?}"
    );
}

fn assert_no_capture(calls: &[String]) {
    assert!(
        calls.iter().all(|line| !line.starts_with("capture-pane")),
        "{calls:?}"
    );
}

fn label(action: &str) -> &'static str {
    match action {
        "prompt" => "prompt",
        "exec" => "native command",
        other => panic!("no dispatch label for {other}"),
    }
}

fn assert_busy(output: &Output, member: i64, action: &str) {
    assert_eq!(code(output), 1, "{}", stdout(output));
    assert_eq!(stdout(output), "");
    assert!(
        stderr(output).contains(&format!(
            "member {member}'s pane input remained busy; {} was not dispatched.",
            label(action)
        )),
        "{}",
        stderr(output)
    );
}

#[test]
fn prompt_sends_escape_then_exact_literal_text_then_enter_and_preserves_json_order() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let output = fixture.cli.run(&[
        "member",
        "prompt",
        &fixture.member.to_string(),
        "  /help  ",
        "--json",
    ]);
    let payload = json(&output);
    assert_eq!(
        payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["member_id", "pane_id", "text"]
    );
    assert_eq!(
        payload,
        serde_json::json!({"member_id":fixture.member,"pane_id":fixture.pane,"text":"  /help  "})
    );
    let calls = fixture.cli.shim_calls();
    let writes: Vec<_> = calls
        .iter()
        .filter(|line| line.starts_with("send-keys"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        [
            format!("send-keys -t {} Escape", fixture.pane),
            format!("send-keys -t {} -l --   /help  ", fixture.pane),
            format!("send-keys -t {} Enter", fixture.pane),
        ],
        "the Director-authored payload gets no appended clauses"
    );
    assert_no_capture(&calls);
}

#[test]
fn prompt_text_output_retains_the_existing_success_contract() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let output = fixture.cli.run(&[
        "member",
        "prompt",
        &fixture.member.to_string(),
        "hello worker",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Sent prompt \"hello worker\" to member worker ({}).\n",
            fixture.pane
        )
    );
}

#[test]
fn prompt_invalid_lines_fail_before_pane_io() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    for text in [
        "",
        " \u{2003}",
        "a\nb",
        "a\rb",
        "a\tb",
        "a\u{1b}b",
        "a\u{7f}b",
        "a\u{1}b",
    ] {
        let before = fixture.cli.shim_calls().len();
        let output = fixture.cli.run(&[
            "member",
            "prompt",
            &fixture.member.to_string(),
            text,
            "--json",
        ]);
        assert_eq!(code(&output), 2, "{text:?}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("text must be one non-empty line without control characters."),
            "{}",
            stderr(&output)
        );
        assert_eq!(fixture.cli.shim_calls().len(), before);
    }
}

#[test]
fn a_held_lock_makes_prompt_and_exec_wait_then_report_busy_without_capture_or_input() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let _lock = fixture.hold_lock();
    for action in ["prompt", "exec"] {
        let started = Instant::now();
        let output = fixture.run(action);
        let waited = started.elapsed();
        assert_busy(&output, fixture.member, action);
        assert!(
            waited >= Duration::from_millis(1400),
            "{action}: {waited:?}"
        );
        assert!(waited < Duration::from_secs(4), "{action}: {waited:?}");
    }
    assert_no_capture(&fixture.cli.shim_calls());
    assert_no_input(&fixture.cli.shim_calls());
}

#[test]
fn a_lock_released_during_the_wait_lets_prompt_and_exec_dispatch() {
    let _test_resource = common::process_test_resource_guard();
    for action in ["prompt", "exec"] {
        let fixture = InputFixture::new();
        let lock = fixture.hold_lock();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(lock);
        });
        let output = fixture.run(action);
        holder.join().unwrap();
        json(&output);
        assert_eq!(
            fixture
                .cli
                .shim_calls()
                .iter()
                .filter(|line| line.contains(" -l "))
                .count(),
            1,
            "{action}"
        );
    }
}

#[test]
fn duplicate_registrations_for_one_concrete_pane_share_the_lock() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let duplicate = fixture.cli.seed_member(fixture.fleet, "alias");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=?1, mux_window_id='@99' WHERE member_id=?2",
            rusqlite::params![fixture.pane, duplicate],
        )
        .unwrap();
    let _lock = fixture.hold_lock();
    let output = fixture.cli.run(&[
        "member",
        "prompt",
        &duplicate.to_string(),
        "hello alias",
        "--json",
    ]);
    assert_busy(&output, duplicate, "prompt");
    assert_no_input(&fixture.cli.shim_calls());
}

#[test]
fn the_session_and_pane_components_select_independent_input_locks() {
    let _test_resource = common::process_test_resource_guard();
    for separate_session in [false, true] {
        let fixture = InputFixture::new();
        let other = fixture.cli.seed_member(fixture.fleet, "other");
        let pane = if separate_session {
            fixture.pane.clone()
        } else {
            "%99".into()
        };
        let session = if separate_session {
            "other-session"
        } else {
            "main"
        };
        fixture
            .cli
            .sqlite()
            .execute(
                "UPDATE member_placements SET mux_session=?1,mux_pane_id=?2 WHERE member_id=?3",
                rusqlite::params![session, pane, other],
            )
            .unwrap();
        let _lock = fixture.hold_lock();
        let output = fixture.cli.run(&[
            "member",
            "prompt",
            &other.to_string(),
            "other input",
            "--json",
        ]);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert!(placement_lock(&fixture.cli, "tmux", session, &pane).exists());
    }
}

#[test]
fn canonical_database_aliases_share_the_input_lock_directory() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let alias = fixture.cli.home.path().join("database-alias.db");
    symlink(fixture.cli.db_path(), &alias).unwrap();
    let _lock = fixture.hold_lock();
    fixture.cli.set_env(
        "CAFLEET_DATABASE_URL",
        &format!("sqlite:///{}", alias.display()),
    );
    assert_busy(&fixture.run("prompt"), fixture.member, "prompt");
    assert!(
        !fixture
            .cli
            .home
            .path()
            .join("database-alias.db.pane-locks")
            .exists()
    );
}

#[test]
fn lockfiles_have_stable_identity_owner_only_permissions_and_unmodified_contents() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    json(&fixture.run("prompt"));
    let path = fixture.lock_path();
    let before = std::fs::metadata(&path).unwrap();
    assert_eq!(before.permissions().mode() & 0o777, 0o600);
    std::fs::write(&path, b"stable identity sentinel").unwrap();
    json(&fixture.run("exec"));
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(std::fs::read(&path).unwrap(), b"stable identity sentinel");
    let lock = fixture.observe_lock();
    lock.try_lock().unwrap();
    drop(lock);
    let output = fixture
        .cli
        .run(&["member", "delete", &fixture.member.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), before.ino());
}

#[test]
fn lock_open_errors_preserve_member_and_path_context_without_typing() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let path = fixture.lock_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::create_dir(&path).unwrap();
    let output = fixture.run("prompt");
    assert_eq!(code(&output), 1);
    let error = stderr(&output);
    assert!(error.contains(&fixture.member.to_string()), "{error}");
    assert!(error.contains(path.to_str().unwrap()), "{error}");
    assert!(!error.contains("remained busy"), "{error}");
    assert_no_input(&fixture.cli.shim_calls());
}

#[test]
fn read_only_capture_works_while_an_input_lock_is_held() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    fixture.cli.set_pane_capture(&fixture.pane, "captured pane");
    let _lock = fixture.hold_lock();
    let output = fixture.cli.run(&[
        "member",
        "capture",
        &fixture.member.to_string(),
        "--ansi",
        "--json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_no_input(&fixture.cli.shim_calls());
}

#[test]
fn lock_is_held_through_escape_literal_and_enter_for_every_explicit_writer() {
    let _test_resource = common::process_test_resource_guard();
    for action in ["prompt", "exec"] {
        for stage in ["escape", "literal", "enter"] {
            let mut fixture = InputFixture::new();
            let owner = fixture.gate(action, stage);
            let observer = fixture.observe_lock();
            assert!(
                matches!(observer.try_lock(), Err(TryLockError::WouldBlock)),
                "{action}/{stage}"
            );
            if stage == "enter" {
                assert_busy(&fixture.run("prompt"), fixture.member, "prompt");
            }
            let output = owner.release();
            assert_eq!(code(&output), 0, "{action}/{stage}: {}", stderr(&output));
            observer.try_lock().unwrap();
        }
    }
}

#[test]
fn process_exit_releases_input_ownership_without_replacing_the_lockfile() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let mut owner = fixture.gate("prompt", "literal");
    let path = fixture.lock_path();
    let before = std::fs::metadata(&path).unwrap();
    let observer = fixture.observe_lock();
    assert!(matches!(observer.try_lock(), Err(TryLockError::WouldBlock)));
    owner.kill();
    observer.try_lock().unwrap();
    drop(observer);
    json(&fixture.run("prompt"));
    assert_eq!(std::fs::metadata(path).unwrap().ino(), before.ino());
}

#[test]
fn a_literal_timeout_retains_ownership_until_failure_and_releases_it_without_enter() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let mut owner = fixture.gate("prompt", "literal");
    let contender = fixture.observe_lock();
    assert!(matches!(
        contender.try_lock(),
        Err(TryLockError::WouldBlock)
    ));
    let output = bounded_output(owner.child.take().unwrap());
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("is uncertain:"),
        "{}",
        stderr(&output)
    );
    contender.try_lock().unwrap();
    let literals: Vec<_> = fixture
        .cli
        .shim_calls()
        .into_iter()
        .filter(|line| line.contains(" -l "))
        .collect();
    assert_eq!(literals.len(), 1, "{literals:?}");
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.ends_with(" Enter"))
    );
}

#[test]
fn preview_contention_preserves_the_committed_message_and_pending_stamp() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let _lock = fixture.hold_lock();
    let output = fixture.send("held assignment");
    let payload = json(&output);
    assert_eq!(payload["notification_sent"], false);
    let id = payload["message"]["message_id"].as_i64().unwrap();
    assert!(
        stderr(&output).contains(&format!(
            "message {id} was not notified to member {}: pane input remained busy; the next send to member {} or the next message wait retries it.",
            fixture.member, fixture.member
        )),
        "{}",
        stderr(&output)
    );
    let stamp: Option<String> = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stamp, None);
    assert_no_input(&fixture.cli.shim_calls());
    assert_no_capture(&fixture.cli.shim_calls());
}

#[test]
fn a_real_prompt_owner_prevents_preview_input_from_joining_its_payload() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let owner = fixture.gate("prompt", "enter");
    let before = fixture.cli.shim_calls().len();
    let started = Instant::now();
    let sender = fixture.cli.spawn(&[
        "message",
        "send",
        "--from-member-id",
        &fixture.director.to_string(),
        "--to-member-id",
        &fixture.member.to_string(),
        "queued behind native input",
        "--json",
    ]);
    let writer = fixture.cli.sqlite();
    while writer
        .query_row("SELECT count(*) FROM messages", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
        == 0
    {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the send never committed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_no_input(&fixture.cli.shim_calls()[before..]);
    let output = owner.release();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload = json(&bounded_output(sender));
    assert_eq!(
        payload["notification_sent"], true,
        "the waiting send delivers after the prompt releases the lock"
    );
    let writes: Vec<_> = fixture.cli.shim_calls()[before..]
        .iter()
        .filter(|line| line.starts_with("send-keys"))
        .cloned()
        .collect();
    assert_eq!(
        writes
            .iter()
            .map(|line| line.split_whitespace().nth(3).unwrap())
            .collect::<Vec<_>>(),
        ["Escape", "-l", "Enter"],
        "only the preview's own submission follows the prompt's gated Enter: {writes:?}"
    );
    assert!(
        writes[1].contains("[cafleet msg "),
        "the preview line stays separate from the prompt payload: {writes:?}"
    );
}

#[test]
fn a_preview_owner_holds_the_same_guard_through_escape_literal_and_enter() {
    let _test_resource = common::process_test_resource_guard();
    for stage in ["escape", "literal", "enter"] {
        let mut fixture = InputFixture::new();
        let owner = fixture.gate("preview", stage);
        let observer = fixture.observe_lock();
        assert!(
            matches!(observer.try_lock(), Err(TryLockError::WouldBlock)),
            "{stage}"
        );
        if stage == "enter" {
            assert_busy(&fixture.run("exec"), fixture.member, "exec");
        }
        let notified: Option<String> = fixture
            .cli
            .sqlite()
            .query_row(
                "SELECT notified_at FROM messages WHERE to_member_id=?1",
                [fixture.member],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            notified, None,
            "notification is stamped after complete submission"
        );
        let output = owner.release();
        let payload = json(&output);
        assert_eq!(
            payload["notification_sent"],
            true,
            "{stage}: stderr={}; payload={payload}",
            stderr(&output)
        );
    }
}

#[test]
fn prompt_and_native_exec_escape_failures_preserve_the_cause_and_release_the_guard() {
    let _test_resource = common::process_test_resource_guard();
    for action in ["prompt", "exec"] {
        let mut fixture = InputFixture::new();
        fixture.cli.set_env("CAFLEET_TEST_FAIL_STAGE", "escape");
        let output = fixture.run(action);
        assert_eq!(code(&output), 1);
        assert_eq!(stdout(&output), "");
        let error = stderr(&output);
        assert!(
            error.contains(&format!(
                "{} was not dispatched to member {}:",
                label(action),
                fixture.member
            )),
            "{error}"
        );
        assert!(
            error.contains("injected escape transport failure"),
            "{error}"
        );
        assert!(
            fixture
                .cli
                .shim_calls()
                .iter()
                .all(|line| !line.contains(" -l ") && !line.ends_with(" Enter")),
            "{:?}",
            fixture.cli.shim_calls()
        );
        fixture.observe_lock().try_lock().unwrap();
        fixture.cli.extra_env.clear();
        json(&fixture.run(action));
    }
}

#[test]
fn prompt_and_exec_report_first_write_and_enter_failures_as_uncertain_once() {
    let _test_resource = common::process_test_resource_guard();
    for action in ["prompt", "exec"] {
        for stage in ["literal", "enter"] {
            let mut fixture = InputFixture::new();
            fixture.cli.set_env("CAFLEET_TEST_FAIL_STAGE", stage);
            let output = fixture.run(action);
            assert_eq!(code(&output), 1);
            assert_eq!(stdout(&output), "");
            let error = stderr(&output);
            assert!(
                error.contains(&format!(
                    "{} delivery to member {} is uncertain:",
                    label(action),
                    fixture.member
                )),
                "{error}"
            );
            assert!(
                error.contains(&format!("injected {stage} transport failure")),
                "{error}"
            );
            assert!(
                error.contains("Inspect the pane before retrying."),
                "{error}"
            );
            let calls = fixture.cli.shim_calls();
            assert_eq!(
                calls
                    .iter()
                    .filter(|line| line.ends_with(" Escape"))
                    .count(),
                1,
                "{calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|line| line.contains(" -l ")).count(),
                1,
                "{calls:?}"
            );
            assert_eq!(
                calls.iter().filter(|line| line.ends_with(" Enter")).count(),
                usize::from(stage == "enter"),
                "{calls:?}"
            );
            fixture.observe_lock().try_lock().unwrap();
        }
    }
}

#[test]
fn a_hung_escape_is_bounded_and_types_nothing() {
    let _test_resource = common::process_test_resource_guard();
    for action in ["prompt", "exec"] {
        let mut fixture = InputFixture::new();
        fixture.cli.set_env("CAFLEET_TEST_ESCAPE_DELAY", "8");
        let start = Instant::now();
        let output = fixture.run(action);
        assert_eq!(code(&output), 1, "{action}: {}", stdout(&output));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{action}: {:?}",
            start.elapsed()
        );
        assert_eq!(stdout(&output), "");
        assert!(
            stderr(&output).contains(&format!("{} was not dispatched", label(action))),
            "{}",
            stderr(&output)
        );
        assert!(
            fixture
                .cli
                .shim_calls()
                .iter()
                .all(|line| !line.contains(" -l ")),
            "{:?}",
            fixture.cli.shim_calls()
        );
    }
}

#[test]
fn startup_schema_wait_does_not_consume_the_member_submission_budget() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let blocker = fixture.cli.sqlite();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let child = fixture.cli.spawn(&[
        "member",
        "prompt",
        &fixture.member.to_string(),
        "--json",
        "--",
        "-R literal prompt",
    ]);
    std::thread::sleep(Duration::from_millis(3500));
    blocker.execute_batch("ROLLBACK").unwrap();
    let output = bounded_output(child);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["text"], "-R literal prompt");
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .any(|line| line.ends_with(" -l -- -R literal prompt"))
    );
}

#[test]
fn member_delete_contention_uses_one_bounded_deadline_without_kill_or_mutation() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let _lock = fixture.hold_lock();
    let before = fixture.cli.shim_calls().len();
    let start = Instant::now();
    let output = bounded_output(fixture.cli.spawn(&[
        "member",
        "delete",
        &fixture.member.to_string(),
        "--json",
    ]));
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains(&format!(
            "member {}'s pane input remained busy for 5 seconds; member was not deleted.",
            fixture.member
        )),
        "{}",
        stderr(&output)
    );
    assert!(start.elapsed() >= Duration::from_millis(4700));
    assert!(start.elapsed() < Duration::from_millis(6500));
    fixture.assert_active();
    assert!(
        fixture.cli.shim_calls()[before..]
            .iter()
            .all(|line| !line.starts_with("kill-pane"))
    );
}

#[test]
fn member_delete_retries_a_released_lock_and_only_then_kills() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let lock = fixture.hold_lock();
    let child = fixture
        .cli
        .spawn(&["member", "delete", &fixture.member.to_string(), "--json"]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("kill-pane"))
    );
    fixture.assert_active();
    drop(lock);
    let output = bounded_output(child);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        fixture
            .cli
            .shim_count(&format!("kill-pane -t {}", fixture.pane)),
        1
    );
}

#[test]
fn pending_placement_delete_needs_no_pane_lock() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let _lock = fixture.hold_lock();
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [fixture.member],
        )
        .unwrap();
    let start = Instant::now();
    let output = fixture
        .cli
        .run(&["member", "delete", &fixture.member.to_string(), "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("kill-pane"))
    );
}

#[test]
fn fleet_delete_contention_preserves_every_row_and_releases_other_guards() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let other = fixture.cli.seed_member(fixture.fleet, "other");
    let message = fixture
        .cli
        .seed_message(fixture.director, other, "preserve me");
    let _lock = fixture.hold_lock();
    let start = Instant::now();
    let output = bounded_output(fixture.cli.spawn(&[
        "fleet",
        "delete",
        &fixture.fleet.to_string(),
        "--json",
    ]));
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains(&format!(
            "fleet {} pane input remained busy for 5 seconds at member {}; fleet was not deleted.",
            fixture.fleet, fixture.member
        )),
        "{}",
        stderr(&output)
    );
    assert!(start.elapsed() >= Duration::from_millis(4700));
    assert!(start.elapsed() < Duration::from_millis(6500));
    fixture.assert_active();
    assert_eq!(
        fixture
            .cli
            .sqlite()
            .query_row(
                "SELECT text FROM messages WHERE message_id=?1",
                [message],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "preserve me"
    );
    let director_lock = hold_lock(&placement_lock(&fixture.cli, "tmux", "main", "%0"));
    let other_lock = hold_lock(&placement_lock(
        &fixture.cli,
        "tmux",
        "main",
        &format!("%{}", other + 4),
    ));
    drop((director_lock, other_lock));
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .all(|line| !line.starts_with("kill-pane"))
    );
}

#[test]
fn fleet_delete_shares_one_deadline_across_multiple_contended_panes() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let mut paths = [
        (
            placement_lock(&fixture.cli, "tmux", "main", "%0"),
            fixture.director,
        ),
        (fixture.lock_path(), fixture.member),
    ];
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    let first = hold_lock(&paths[0].0);
    let _second = hold_lock(&paths[1].0);
    let start = Instant::now();
    let child = fixture
        .cli
        .spawn(&["fleet", "delete", &fixture.fleet.to_string(), "--json"]);
    std::thread::sleep(Duration::from_secs(3));
    drop(first);
    let output = bounded_output(child);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("at member {};", paths[1].1)),
        "{}",
        stderr(&output)
    );
    assert!(start.elapsed() >= Duration::from_millis(4700));
    assert!(
        start.elapsed() < Duration::from_millis(6500),
        "{:?}",
        start.elapsed()
    );
    fixture.assert_active();
}

#[test]
fn fleet_delete_revalidates_a_changed_placement_set_before_deregistering() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let mut paths = [
        placement_lock(&fixture.cli, "tmux", "main", "%0"),
        fixture.lock_path(),
    ];
    paths.sort();
    assert!(!paths[0].exists());
    let lock = hold_lock(&paths[1]);
    let start = Instant::now();
    let mut owner = InputChild {
        child: Some(
            fixture
                .cli
                .spawn(&["fleet", "delete", &fixture.fleet.to_string(), "--json"]),
        ),
        release: fixture.cli.home.path().join("fleet-delete-release"),
    };
    while !paths[0].exists() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "fleet acquisition never reached its first sorted lock"
        );
        assert!(
            owner.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "fleet deletion finished before acquiring input locks"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let added = fixture
        .cli
        .seed_member(fixture.fleet, "added-during-acquisition");
    let added_path = placement_lock(&fixture.cli, "tmux", "main", &format!("%{}", added + 4));
    let _added_lock = hold_lock(&added_path);
    drop(lock);
    let output = owner.release();
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!("at member {added};")),
        "{}",
        stderr(&output)
    );
    assert!(start.elapsed() < Duration::from_millis(6500));
    fixture.assert_active();
    let active: String = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT status FROM members WHERE member_id=?1",
            [added],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, "active");
}

#[test]
fn the_backend_component_selects_an_independent_input_lock() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let _other_backend = hold_lock(&placement_lock(
        &fixture.cli,
        "herdr",
        "main",
        &fixture.pane,
    ));
    json(&fixture.run("prompt"));
    assert!(fixture.lock_path().exists());
}

#[test]
fn prompt_and_exec_dispatch_regardless_of_the_recorded_coding_agent() {
    let _test_resource = common::process_test_resource_guard();
    for agent in ["claude", "codex", "opencode"] {
        let fixture = InputFixture::new();
        fixture
            .cli
            .sqlite()
            .execute(
                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                rusqlite::params![agent, fixture.member],
            )
            .unwrap();
        json(&fixture.run("prompt"));
        json(&fixture.run("exec"));
        let calls = fixture.cli.shim_calls();
        assert_no_capture(&calls);
        assert_eq!(
            calls
                .iter()
                .filter(|line| line.ends_with(" Escape"))
                .count(),
            2,
            "{agent}: {calls:?}"
        );
    }
}

#[test]
fn preview_ownership_extends_through_a_blocked_notification_stamp() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let mut owner = fixture.gate("preview", "enter");
    let writer = fixture.cli.sqlite();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    std::fs::write(&owner.release, b"release").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(owner.child.as_mut().unwrap().try_wait().unwrap().is_none());
    let observer = fixture.observe_lock();
    assert!(matches!(observer.try_lock(), Err(TryLockError::WouldBlock)));
    drop(writer);
    let payload = json(&owner.release());
    assert_eq!(payload["notification_sent"], true);
    observer.try_lock().unwrap();
}

#[test]
fn preview_prewrite_database_revalidation_obeys_the_remaining_attempt_budget() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let lock = fixture.hold_lock();
    let start = Instant::now();
    let args = fixture.action("preview");
    let refs: Vec<_> = args.iter().map(String::as_str).collect();
    let child = fixture.cli.spawn(&refs);
    let writer = fixture.cli.sqlite();
    while writer
        .query_row("SELECT count(*) FROM messages", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
        == 0
    {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the send never committed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    let journal: String = writer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    let committed: (i64, String, String, Option<String>) = writer
        .query_row(
            "SELECT message_id,text,status_state,notified_at FROM messages",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(committed.1, "preview body");
    assert_eq!(committed.2, "input_required");
    assert_eq!(committed.3, None);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    drop(lock);
    let output = bounded_output(child);
    let elapsed = start.elapsed();
    let payload = json(&output);
    assert_eq!(payload["message"]["message_id"], committed.0);
    assert_eq!(payload["message"]["text"], committed.1);
    assert_eq!(payload["notification_sent"], false);
    let error = stderr(&output);
    assert!(
        error.contains(&format!("Message {} was persisted.", committed.0)),
        "{error}"
    );
    assert!(error.contains("database is locked"), "{error}");
    assert!(
        error.contains(&format!(
            "message {} was not notified to member {}:",
            committed.0, fixture.member
        )),
        "{error}"
    );
    let retained: (String, String, Option<String>) = writer
        .query_row(
            "SELECT text,status_state,notified_at FROM messages WHERE message_id=?1",
            [committed.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(retained, (committed.1, committed.2, None));
    drop(writer);
    let calls = fixture.cli.shim_calls();
    assert_no_capture(&calls);
    assert_no_input(&calls);
    fixture.observe_lock().try_lock().unwrap();
    assert!(elapsed < Duration::from_millis(6500), "{elapsed:?}");
}

#[test]
fn notification_database_wait_obeys_the_remaining_attempt_budget() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    let start = Instant::now();
    let owner = fixture.gate("preview", "enter");
    let writer = fixture.cli.sqlite();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let output = owner.release();
    assert!(
        start.elapsed() < Duration::from_millis(6500),
        "{:?}",
        start.elapsed()
    );
    let payload = json(&output);
    assert_eq!(payload["notification_sent"], false);
    assert!(
        stderr(&output).contains("was not notified to member"),
        "{}",
        stderr(&output)
    );
    let stamp: Option<String> = writer
        .query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [payload["message"]["message_id"].as_i64().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stamp, None);
    drop(writer);
}

#[test]
fn herdr_uses_the_same_input_guard_and_submits_escape_first_without_reading() {
    let _test_resource = common::process_test_resource_guard();
    let mut fixture = InputFixture::new();
    fixture.cli.set_env("CAFLEET_MULTIPLEXER", "herdr");
    fixture.cli.set_env("HERDR_ENV", "test-herdr");
    fixture.cli.sqlite().execute(
        "UPDATE member_placements SET backend='herdr',mux_session='herdr-session',mux_pane_id='w1:p2' WHERE member_id=?1",
        [fixture.member]).unwrap();
    write_executable(
        &fixture.cli.shim_dir.join("herdr"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$CAFLEET_TEST_TMUX_LOG"
if [ "$1" = "pane" ] && [ "$2" = "send-text" ]; then
    printf 'literal-argc %s\nliteral-text %s\n' "$#" "$4" >> "$CAFLEET_TEST_TMUX_LOG"
fi
if [ "$1" = "pane" ] && [ "$2" = "read" ]; then
    echo "unexpected pane read" >&2
    exit 1
fi
"#,
    );
    let path = placement_lock(&fixture.cli, "herdr", "herdr-session", "w1:p2");
    let lock = hold_lock(&path);
    assert_busy(&fixture.run("prompt"), fixture.member, "prompt");
    assert!(fixture.cli.shim_calls().is_empty());
    drop(lock);
    let payload = json(&fixture.run("prompt"));
    assert_eq!(payload["pane_id"], "w1:p2");
    let calls = fixture.cli.shim_calls();
    assert_eq!(
        calls
            .iter()
            .filter(|line| line.starts_with("literal-"))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["literal-argc 4", "literal-text hello worker"]
    );
    assert!(
        calls.iter().all(|line| !line.starts_with("pane read")),
        "{calls:?}"
    );
    let writes: Vec<_> = calls
        .iter()
        .filter(|line| line.starts_with("pane send-"))
        .cloned()
        .collect();
    assert_eq!(
        writes,
        [
            "pane send-keys w1:p2 escape",
            "pane send-text w1:p2 hello worker",
            "pane send-keys w1:p2 enter",
        ]
    );
}

#[test]
fn fleet_delete_acquires_sorted_keys_before_later_placements() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = InputFixture::new();
    let mut paths = [
        placement_lock(&fixture.cli, "tmux", "main", "%0"),
        fixture.lock_path(),
    ];
    paths.sort();
    let first = hold_lock(&paths[0]);
    let identity = std::fs::metadata(&paths[0]).unwrap().ino();
    assert!(!paths[1].exists());
    let mut owner = InputChild {
        child: Some(
            fixture
                .cli
                .spawn(&["fleet", "delete", &fixture.fleet.to_string(), "--json"]),
        ),
        release: fixture.cli.home.path().join("sorted-delete-release"),
    };
    std::thread::sleep(Duration::from_secs(2));
    assert!(owner.child.as_mut().unwrap().try_wait().unwrap().is_none());
    assert!(
        !paths[1].exists(),
        "later key was opened while the first sorted key remained unavailable"
    );
    fixture.assert_active();
    drop(first);
    let output = owner.release();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(paths[1].exists());
    assert_eq!(std::fs::metadata(&paths[0]).unwrap().ino(), identity);
}
