use super::*;
use crate::coding_agent::CodingAgent;
use rusqlite::Connection;

fn registry_state(conn: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
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
            let mut query = conn
                .prepare(&format!("SELECT * FROM \"{escaped}\" ORDER BY 1"))
                .unwrap();
            let width = query.column_count();
            let rows = query
                .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn invalid_creation(backend: CodingAgent, extra: &[String]) -> CafleetError {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = crate::broker::tests::migrated_conn(&dir);
    let (fleet, _) = crate::broker::tests::create_fleet(&mut conn, "invalid-launch-fixture");
    let mut arguments = create_arguments()
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    arguments[4] = fleet.to_string();
    arguments.extend(["--coding-agent".into(), backend.name().into()]);
    arguments.extend_from_slice(extra);
    arguments.push("literal assignment".into());
    let Command::Member(command) = CliArgs::try_parse_from(arguments).unwrap().command else {
        panic!("member command expected")
    };
    let before = registry_state(&conn);
    let identity = crate::broker::runtime::database_identity(&conn).unwrap();
    let paths = crate::runtime::runner::RuntimePaths::for_run(&identity, 1).unwrap();
    let settings = Settings {
        database_url: format!("sqlite:///{}", conn.path().unwrap()),
        max_text_len: 200,
        multiplexer: Some("unavailable-test-multiplexer".into()),
    };
    let failure = member::run(&mut conn, &settings, command, crate::input::Deadline::new())
        .err()
        .expect("invalid launch must fail");
    assert!(
        matches!(failure, CafleetError::Usage(_)),
        "{}",
        failure.message()
    );
    assert_eq!(registry_state(&conn), before);
    assert!(!paths.run_dir.exists());
    failure
}

fn create_arguments() -> Vec<&'static str> {
    vec![
        "cafleet",
        "member",
        "create",
        "--fleet-id",
        "1",
        "--name",
        "worker",
        "--description",
        "policy fixture",
    ]
}

#[test]
fn runtime_policy_parser_accepts_the_backend_qualified_launch_selection() {
    for (backend, mode) in [
        ("claude", "default"),
        ("claude", "manual"),
        ("codex", "on-request"),
        ("opencode", "cafleet"),
    ] {
        let mut arguments = create_arguments();
        arguments.extend([
            "--coding-agent",
            backend,
            "--permission-mode",
            mode,
            "literal assignment",
        ]);
        assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
    }
}

#[test]
fn runtime_policy_parser_accepts_explicit_metadata_evidence() {
    let mut arguments = create_arguments();
    arguments.extend(["--event-evidence", "metadata", "literal assignment"]);
    assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
}

#[test]
fn runtime_policy_parser_accepts_redacted_evidence_with_its_absolute_file() {
    let mut arguments = create_arguments();
    arguments.extend([
        "--event-evidence",
        "redacted",
        "--event-redaction-file",
        "/tmp/synthetic-redaction.json",
        "literal assignment",
    ]);
    assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
}

#[test]
fn runtime_policy_parser_keeps_omitted_selection_and_model_effort_boundaries() {
    let mut arguments = create_arguments();
    arguments.extend([
        "--coding-agent",
        "codex",
        "--model",
        "fixture-model",
        "--effort",
        "high",
        "literal '$x' `text` assignment",
    ]);
    let Command::Member(member::MemberCommand::Create {
        coding_agent,
        model,
        effort,
        ..
    }) = CliArgs::try_parse_from(arguments).unwrap().command
    else {
        panic!("member create expected")
    };
    assert_eq!(coding_agent, Some(crate::coding_agent::CodingAgent::Codex));
    assert_eq!(model.as_deref(), Some("fixture-model"));
    assert_eq!(effort.as_deref(), Some("high"));
}

#[test]
fn runtime_policy_invalid_launch_selection_has_usage_diagnostics_before_creation_effects() {
    for &backend in CodingAgent::all() {
        let failure = invalid_creation(
            backend,
            &[
                "--permission-mode".into(),
                "unsupported-fixture-mode".into(),
            ],
        );
        assert!(failure.message().contains(backend.name()));
        for choice in backend.policy_choices(None) {
            assert!(failure.message().contains(&choice));
        }
    }
}

#[test]
fn runtime_policy_invalid_redaction_configuration_fails_before_creation_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("redaction.json");
    for bytes in [
        b"not json".to_vec(),
        vec![0xff],
        br#"{"schema_version":1,"patterns":[],"sensitive_rules":[]}"#.to_vec(),
        br#"{"schema_version":1,"patterns":["["],"sensitive_rules":[]}"#.to_vec(),
        br#"{"schema_version":1,"patterns":["secret"],"sensitive_rules":["missing-rule"]}"#
            .to_vec(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        invalid_creation(
            CodingAgent::Codex,
            &[
                "--event-evidence".into(),
                "redacted".into(),
                "--event-redaction-file".into(),
                path.to_str().unwrap().into(),
            ],
        );
    }
}

#[test]
fn runtime_policy_redaction_options_form_a_validated_launch_only_group() {
    for extra in [
        vec!["--event-evidence".into(), "redacted".into()],
        vec![
            "--event-evidence".into(),
            "metadata".into(),
            "--event-redaction-file".into(),
            "/tmp/unused-redaction.json".into(),
        ],
        vec![
            "--event-evidence".into(),
            "redacted".into(),
            "--event-redaction-file".into(),
            "relative-redaction.json".into(),
        ],
    ] {
        invalid_creation(CodingAgent::Codex, &extra);
    }
}
