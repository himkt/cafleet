mod doctor;
pub(crate) mod fleet;
pub(crate) mod helpers;
pub(crate) mod member;
mod message;
mod runtime;
mod setup;

#[cfg(test)]
mod launch_policy_parser_tests {
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
        let dir = tempfile::Builder::new()
            .prefix(".p194-")
            .tempdir_in("/tmp")
            .unwrap();
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
            .expect_err("invalid launch must fail");
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
}
#[cfg(test)]
mod press_parser_tests {
    use super::*;

    #[test]
    fn step5_press_cli_preserves_keys_after_the_ordinary_option_separator() {
        assert!(CliArgs::try_parse_from(["cafleet", "member", "show", "42"]).is_ok());
        let result = CliArgs::try_parse_from([
            "cafleet",
            "member",
            "press",
            "--json",
            "42",
            "--",
            "-",
            "+",
            ";",
            "A",
            "Ctrl+A",
            "Shift+Tab",
        ]);
        match result
            .unwrap_or_else(|cause| panic!("press parse: {cause}"))
            .command
        {
            Command::Member(member::MemberCommand::Press {
                member_id,
                keys,
                json,
            }) => {
                assert_eq!(member_id, 42);
                assert!(json);
                assert_eq!(keys, ["-", "+", ";", "A", "Ctrl+A", "Shift+Tab"]);
            }
            _ => panic!("press has its dedicated command arm"),
        }
        for arguments in [
            vec!["cafleet", "member", "press", "0", "Enter"],
            vec!["cafleet", "member", "press", "42"],
        ] {
            match CliArgs::try_parse_from(arguments) {
                Ok(_) => panic!("press requires a positive member and keys"),
                Err(cause) => assert_eq!(cause.exit_code(), 2),
            }
        }
    }

    #[test]
    fn step5_press_cli_invalid_tail_is_usage_before_missing_database_validation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("absent.db");
        let settings = Settings {
            database_url: format!("sqlite:///{}", path.display()),
            max_text_len: 1000,
            multiplexer: Some("tmux".into()),
        };
        let command = CliArgs::try_parse_from([
            "cafleet", "member", "press", "42", "Enter", "Ctrl+1", "--json",
        ])
        .unwrap_or_else(|cause| panic!("raw key arguments reach typed validation: {cause}"))
        .command;
        let failure =
            dispatch(&settings, command).expect_err("the complete key vector is validated first");
        assert!(matches!(&failure, CafleetError::Usage(message)
            if message == "invalid press key at position 2: \"Ctrl+1\""));
        assert_eq!(failure.exit_code(), 2);
        assert!(!path.exists());
    }

    #[test]
    fn step5_press_presentation_is_canonical_acknowledgment_without_effect_counts() {
        let keys = crate::multiplexer::PressKeys::parse(&["cTrL+A".into(), ";".into()]).unwrap();
        let value = crate::runtime::KeyAcknowledgment {
            member_id: 42,
            pane_id: "%5".into(),
            keys: keys.canonical(),
        };
        assert_eq!(
            serde_json::to_string(&crate::presentation::member_press(&value)).unwrap(),
            "{\"member_id\":42,\"pane_id\":\"%5\",\"keys\":[\"Ctrl+a\",\";\"],\"status\":\"acknowledged\"}",
        );
        assert_eq!(
            crate::output::format_member_press(&value),
            "Acknowledged keys [\"Ctrl+a\",\";\"] for member 42 (%5).",
        );
    }
}
#[cfg(test)]
mod runtime_parser_tests {
    use clap::Parser;

    use super::CliArgs;

    mod permission_entry {
        use clap::Parser;

        use super::CliArgs;

        fn inspect() -> Vec<String> {
            [
                "cafleet",
                "permission",
                "inspect",
                "17",
                "--event-id",
                "19",
                "--from-member-id",
                "23",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        }
        fn respond() -> Vec<String> {
            [
                "cafleet",
                "permission",
                "respond",
                "17",
                "--event-id",
                "19",
                "--from-member-id",
                "23",
                "--expected-run-id",
                "29",
                "--expected-owner-epoch",
                "3",
                "--request-token",
                &"a".repeat(64),
                "--decision",
                "allow",
                "--decision-source",
                "director-auto",
                "--director-mode",
                "auto",
                "--mode-evidence-ref",
                "fixture:mode",
                "--decision-evidence-ref",
                "fixture:decision",
                "--operation-key",
                "fixture:response-1",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        }
        fn set(args: &mut [String], field: &str, value: &str) {
            let index = if field == "interaction" {
                3
            } else {
                args.iter()
                    .position(|arg| arg == field)
                    .expect("the fixture contains the selected flag")
                    + 1
            };
            args[index] = value.into();
        }
        fn accepted(args: &[String]) {
            if let Err(error) = CliArgs::try_parse_from(args) {
                panic!("valid permission command parses: {error}");
            }
        }
        fn rejected(args: &[String]) {
            let error = match CliArgs::try_parse_from(args) {
                Ok(_) => panic!("invalid permission command produces a usage error"),
                Err(error) => error,
            };
            assert_eq!(error.exit_code(), 2);
            assert_ne!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
        }

        #[test]
        fn step5_a0_permission_inspect_accepts_positive_id_boundaries_and_json_selection() {
            accepted(&inspect());
            for field in ["interaction", "--event-id", "--from-member-id"] {
                for value in ["1", "9223372036854775807"] {
                    for json in [false, true] {
                        let mut args = inspect();
                        set(&mut args, field, value);
                        if json {
                            args.push("--json".into());
                        }
                        accepted(&args);
                    }
                }
            }
        }
        #[test]
        fn step5_a0_permission_respond_accepts_positive_id_epoch_boundaries_and_json_selection() {
            accepted(&respond());
            for field in [
                "interaction",
                "--event-id",
                "--from-member-id",
                "--expected-run-id",
                "--expected-owner-epoch",
            ] {
                for value in ["1", "9223372036854775807"] {
                    for json in [false, true] {
                        let mut args = respond();
                        set(&mut args, field, value);
                        if json {
                            args.push("--json".into());
                        }
                        accepted(&args);
                    }
                }
            }
        }
        #[test]
        fn step5_a0_permission_respond_accepts_each_semantic_decision_and_valid_source_mode_pair() {
            accepted(&respond());
            for decision in ["allow", "deny"] {
                for (source, mode) in [
                    ("director-auto", "auto"),
                    ("user", "auto"),
                    ("user", "manual"),
                    ("user", "unknown"),
                ] {
                    let mut args = respond();
                    set(&mut args, "--decision", decision);
                    set(&mut args, "--decision-source", source);
                    set(&mut args, "--director-mode", mode);
                    accepted(&args);
                }
            }
        }
        #[test]
        fn step5_a0_permission_inspect_rejects_invalid_assigned_ids() {
            accepted(&inspect());
            for field in ["interaction", "--event-id", "--from-member-id"] {
                for value in [
                    "0",
                    "-1",
                    "unknown",
                    "1.0",
                    "9223372036854775808",
                    "18446744073709551616",
                ] {
                    let mut args = inspect();
                    set(&mut args, field, value);
                    rejected(&args);
                }
            }
        }
        #[test]
        fn step5_a0_permission_respond_rejects_invalid_assigned_ids_and_epochs() {
            accepted(&respond());
            for field in [
                "interaction",
                "--event-id",
                "--from-member-id",
                "--expected-run-id",
                "--expected-owner-epoch",
            ] {
                for value in [
                    "0",
                    "-1",
                    "unknown",
                    "1.0",
                    "9223372036854775808",
                    "18446744073709551616",
                ] {
                    let mut args = respond();
                    set(&mut args, field, value);
                    rejected(&args);
                }
            }
        }
        fn required(mut args: Vec<String>, flags: &[&str]) {
            accepted(&args);
            let mut missing_id = args.clone();
            missing_id.remove(3);
            rejected(&missing_id);
            for flag in flags {
                let index = args
                    .iter()
                    .position(|arg| arg == flag)
                    .expect("required fixture flag");
                let mut missing = args.clone();
                missing.drain(index..index + 2);
                rejected(&missing);
            }
            args.push("--json".into());
            accepted(&args);
        }
        #[test]
        fn step5_a0_permission_inspect_requires_its_interaction_event_and_director() {
            required(inspect(), &["--event-id", "--from-member-id"]);
        }
        #[test]
        fn step5_a0_permission_respond_requires_each_snapshot_decision_and_provenance_field() {
            required(
                respond(),
                &[
                    "--event-id",
                    "--from-member-id",
                    "--expected-run-id",
                    "--expected-owner-epoch",
                    "--request-token",
                    "--decision",
                    "--decision-source",
                    "--director-mode",
                    "--mode-evidence-ref",
                    "--decision-evidence-ref",
                    "--operation-key",
                ],
            );
        }
        #[test]
        fn step5_a0_permission_response_accepts_exact_lowercase_hex_tokens() {
            accepted(&respond());
            for token in ["0".repeat(64), "f".repeat(64), "0123456789abcdef".repeat(4)] {
                let mut args = respond();
                set(&mut args, "--request-token", &token);
                accepted(&args);
            }
        }
        #[test]
        fn step5_a0_permission_response_rejects_uppercase_nonhex_and_wrong_length_tokens() {
            accepted(&respond());
            for token in [
                String::new(),
                "a".repeat(63),
                "a".repeat(65),
                "A".repeat(64),
                format!("{}A", "a".repeat(63)),
                "g".repeat(64),
                " ".repeat(64),
                format!("{} ", "a".repeat(63)),
                "é".repeat(32),
                "０".repeat(64),
            ] {
                let mut args = respond();
                set(&mut args, "--request-token", &token);
                rejected(&args);
            }
        }
        #[test]
        fn step5_a0_permission_response_references_accept_nonblank_utf8_at_exact_byte_boundaries() {
            accepted(&respond());
            for field in ["--mode-evidence-ref", "--decision-evidence-ref"] {
                for value in [
                    "x".into(),
                    "x".repeat(2048),
                    "é".repeat(1024),
                    format!("{}é", "界".repeat(682)),
                    format!("{}x", " ".repeat(2047)),
                ] {
                    assert!(value.len() <= 2048);
                    let mut args = respond();
                    set(&mut args, field, &value);
                    accepted(&args);
                }
            }
        }
        #[test]
        fn step5_a0_permission_response_references_reject_blank_oversize_and_every_control() {
            accepted(&respond());
            for field in ["--mode-evidence-ref", "--decision-evidence-ref"] {
                for value in [
                    String::new(),
                    " ".into(),
                    "\t".into(),
                    "x".repeat(2049),
                    format!("{}x", "é".repeat(1024)),
                ] {
                    let mut args = respond();
                    set(&mut args, field, &value);
                    rejected(&args);
                }
                for control in (0u8..=31).chain(std::iter::once(127)) {
                    let value = format!("fixture:{}:reference", char::from(control));
                    let mut args = respond();
                    set(&mut args, field, &value);
                    rejected(&args);
                }
            }
        }
        #[test]
        fn step5_a0_permission_response_operation_key_accepts_literal_nonblank_values_and_rejects_blank()
         {
            accepted(&respond());
            for key in ["fixture:key", "fixture:literal key 世界"] {
                let mut args = respond();
                set(&mut args, "--operation-key", key);
                accepted(&args);
            }
            for key in ["", " ", "\t\n"] {
                let mut args = respond();
                set(&mut args, "--operation-key", key);
                rejected(&args);
            }
        }
        #[test]
        fn step5_a0_permission_response_rejects_invalid_decision_source_and_mode_enums() {
            accepted(&respond());
            for (field, values) in [
                ("--decision", ["", "approved", "yes", "Allow"]),
                ("--decision-source", ["", "director_auto", "member", "User"]),
                ("--director-mode", ["", "AUTO", "never", "on-request"]),
            ] {
                for value in values {
                    let mut args = respond();
                    set(&mut args, field, value);
                    rejected(&args);
                }
            }
        }
    }
    mod recovery_entry {
        use clap::{CommandFactory, Parser, error::ErrorKind};
        use rusqlite::Connection;
        use tempfile::TempDir;

        use super::CliArgs;
        use crate::broker::tests as common;
        use crate::config::Settings;

        fn recognized_usage_error(args: &[&str]) {
            let error = super::usage_error(args);
            assert_eq!(error.exit_code(), 2, "{args:?}: {error}");
            assert!(
                !matches!(
                    error.kind(),
                    ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument
                ),
                "the recovery parser must validate these arguments: {args:?}: {error}"
            );
        }

        fn exceptional_args() -> Vec<&'static str> {
            vec![
                "member",
                "recover",
                "7",
                "--owner-terminated",
                "--expected-run-id",
                "11",
                "--expected-owner-epoch",
                "3",
                "--evidence-ref",
                "audit:owner-termination",
            ]
        }

        fn argument_id(command: &clap::Command, long: &str) -> String {
            command
                .get_arguments()
                .find(|argument| argument.get_long() == Some(long))
                .unwrap_or_else(|| panic!("recovery defines --{long}"))
                .get_id()
                .to_string()
        }

        #[test]
        fn runner_recovery_entry_ordinary_parser_accepts_positive_member_ids_and_json_selection() {
            for member_id in ["1", "481", "9223372036854775807"] {
                for json in [false, true] {
                    let mut args = vec!["cafleet", "member", "recover", member_id];
                    if json {
                        args.push("--json");
                    }
                    assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                }
            }
        }

        #[test]
        fn runner_recovery_entry_member_ids_require_positive_assigned_integers() {
            recognized_usage_error(&["member", "recover", "--json"]);
            for member_id in ["", "0", "-1", "unknown", "1.0", "9223372036854775808"] {
                recognized_usage_error(&["member", "recover", "--json", "--", member_id]);
            }
        }

        #[test]
        fn runner_recovery_entry_full_attestation_preserves_snapshot_reference_and_json_values() {
            for (member_id, run_id, epoch) in [
                ("7", "11", "3"),
                (
                    "9223372036854775807",
                    "9223372036854775807",
                    "9223372036854775807",
                ),
            ] {
                for reference in [
                    "audit:owner-termination",
                    "  audit:owner 世界 '$x' `literal`  ",
                ] {
                    for json in [false, true] {
                        let mut args = vec![
                            "cafleet",
                            "member",
                            "recover",
                            member_id,
                            "--owner-terminated",
                            "--expected-run-id",
                            run_id,
                            "--expected-owner-epoch",
                            epoch,
                            "--evidence-ref",
                            reference,
                        ];
                        if json {
                            args.push("--json");
                        }
                        assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                        let command = CliArgs::command();
                        let recovery = command
                            .find_subcommand("member")
                            .unwrap()
                            .find_subcommand("recover")
                            .unwrap();
                        let member_key = recovery
                            .get_arguments()
                            .find(|argument| argument.get_index() == Some(1))
                            .expect("recover has its member positional argument")
                            .get_id()
                            .to_string();
                        let run_key = argument_id(recovery, "expected-run-id");
                        let epoch_key = argument_id(recovery, "expected-owner-epoch");
                        let reference_key = argument_id(recovery, "evidence-ref");
                        let terminated_key = argument_id(recovery, "owner-terminated");
                        let json_key = argument_id(recovery, "json");
                        let matches = command.try_get_matches_from(args).unwrap();
                        let recovery = matches
                            .subcommand_matches("member")
                            .unwrap()
                            .subcommand_matches("recover")
                            .unwrap();
                        assert_eq!(
                            recovery.get_one::<i64>(&member_key),
                            Some(&member_id.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<i64>(&run_key),
                            Some(&run_id.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<i64>(&epoch_key),
                            Some(&epoch.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<String>(&reference_key).unwrap(),
                            reference
                        );
                        assert!(recovery.get_flag(&terminated_key));
                        assert_eq!(recovery.get_flag(&json_key), json);
                    }
                }
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_options_require_the_complete_group() {
            let options: [&[&str]; 4] = [
                &["--owner-terminated"],
                &["--expected-run-id", "11"],
                &["--expected-owner-epoch", "3"],
                &["--evidence-ref", "audit:owner-termination"],
            ];
            for selected in 1_u8..15 {
                let mut args = vec!["member", "recover", "7", "--json"];
                for (index, option) in options.iter().enumerate() {
                    if selected & (1 << index) != 0 {
                        args.extend_from_slice(option);
                    }
                }
                recognized_usage_error(&args);
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_snapshot_ids_require_positive_integers() {
            for option in ["--expected-run-id", "--expected-owner-epoch"] {
                for invalid in ["", "0", "-1", "unknown", "1.0", "9223372036854775808"] {
                    let mut args = exceptional_args();
                    let index = args.iter().position(|value| *value == option).unwrap();
                    args.drain(index..index + 2);
                    let literal = format!("{option}={invalid}");
                    args.push(&literal);
                    recognized_usage_error(&args);
                }
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_rejects_blank_references_and_missing_option_values() {
            for reference in ["", " ", "\t\n"] {
                let mut args = exceptional_args();
                *args.last_mut().unwrap() = reference;
                recognized_usage_error(&args);
            }
            for option in [
                "--expected-run-id",
                "--expected-owner-epoch",
                "--evidence-ref",
            ] {
                let mut args = exceptional_args();
                let index = args.iter().position(|value| *value == option).unwrap();
                args.drain(index..index + 2);
                args.push(option);
                recognized_usage_error(&args);
            }
        }

        fn unwrapped_registry() -> (TempDir, Settings, Connection, i64) {
            let dir = tempfile::Builder::new()
                .prefix(".p194-entry-")
                .tempdir_in("/tmp")
                .unwrap();
            let mut conn = common::migrated_conn(&dir);
            let (fleet, _) = common::create_fleet(&mut conn, "recovery-entry");
            let member_id = crate::broker::register_member(
                &mut conn,
                fleet,
                "unwrapped",
                "recovery lookup fixture",
                None,
            )
            .unwrap()
            .member_id;
            let settings = Settings {
                database_url: format!("sqlite:///{}", conn.path().unwrap()),
                max_text_len: 200,
                multiplexer: None,
            };
            (dir, settings, conn, member_id)
        }

        #[test]
        fn runner_recovery_entry_unwrapped_member_fails_explicitly_before_registry_or_artifact_mutation()
         {
            let (dir, settings, conn, member_id) = unwrapped_registry();
            let path = std::path::PathBuf::from(conn.path().unwrap());
            drop(conn);
            let before = std::fs::read(&path).unwrap();
            let mut names = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            names.sort();
            let id = member_id.to_string();
            let args =
                CliArgs::try_parse_from(["cafleet", "member", "recover", &id, "--json"]).unwrap();
            let result = super::super::dispatch(&settings, args.command);
            let error = match result {
                Ok(()) => panic!("an unwrapped member has no recorded run to recover"),
                Err(error) => error,
            };
            let message = error.message();
            assert!(message.contains("run"), "{message}");
            assert!(message.contains(&id), "{message}");
            assert_eq!(std::fs::read(&path).unwrap(), before);
            let mut after_names = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            after_names.sort();
            assert_eq!(after_names, names);
        }

        #[test]
        fn runner_recovery_entry_refused_registry_preserves_wal_database_and_sidecar_bytes() {
            for mutation in [
                "PRAGMA application_id=42",
                "ALTER TABLE member_runtime_events RENAME COLUMN notification_state TO broken",
                "ALTER TABLE runtime_database_identity RENAME COLUMN instance_id TO broken",
            ] {
                let (_dir, settings, conn, member_id) = unwrapped_registry();
                conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
                conn.execute_batch(mutation).unwrap();
                conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                    .unwrap();
                let before = ["", "-wal", "-shm"].map(|suffix| {
                    let path =
                        std::path::PathBuf::from(format!("{}{suffix}", conn.path().unwrap()));
                    let bytes = std::fs::read(&path).unwrap();
                    (path, bytes)
                });
                let id = member_id.to_string();
                let args = CliArgs::try_parse_from(["cafleet", "member", "recover", &id, "--json"])
                    .unwrap();
                assert!(super::super::dispatch(&settings, args.command).is_err());
                for (path, bytes) in before {
                    assert_eq!(std::fs::read(&path).unwrap(), bytes, "{mutation}: {path:?}");
                }
            }
        }
    }

    fn usage_error(args: &[&str]) -> clap::Error {
        match CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied())) {
            Ok(_) => panic!("expected usage failure for {args:?}"),
            Err(error) => error,
        }
    }

    #[test]
    fn runtime_read_commands_accept_positive_assigned_ids_and_json_selection() {
        for command in ["event", "input"] {
            for id in ["1", "481", "9223372036854775807"] {
                for json in [false, true] {
                    let mut args = vec!["cafleet", command, "show", id];
                    if json {
                        args.push("--json");
                    }
                    assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                }
            }
        }
    }

    #[test]
    fn runtime_read_ids_reject_zero_negative_malformed_and_overflowing_values() {
        for command in ["event", "input"] {
            for id in ["0", "-1", "unknown", "1.0", "9223372036854775808"] {
                assert_eq!(usage_error(&[command, "show", id, "--json"]).exit_code(), 2);
            }
            assert_eq!(usage_error(&[command, "show"]).exit_code(), 2);
        }
    }

    #[test]
    fn input_reconcile_accepts_exact_cli_dispositions_with_evidence() {
        for disposition in ["submitted", "not-submitted", "abandoned-uncertain"] {
            assert!(
                CliArgs::try_parse_from([
                    "cafleet",
                    "input",
                    "reconcile",
                    "17",
                    "--disposition",
                    disposition,
                    "--evidence-ref",
                    "audit:inspected-receipt",
                    "--json",
                ])
                .is_ok(),
                "{disposition}"
            );
        }
    }

    #[test]
    fn input_reconcile_requires_valid_disposition_and_nonempty_evidence() {
        for args in [
            vec!["input", "reconcile", "17"],
            vec!["input", "reconcile", "17", "--disposition", "submitted"],
            vec![
                "input",
                "reconcile",
                "17",
                "--evidence-ref",
                "audit:missing-choice",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "approved",
                "--evidence-ref",
                "audit:x",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "not_submitted",
                "--evidence-ref",
                "audit:x",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "",
            ],
            vec![
                "input",
                "reconcile",
                "17",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "   ",
            ],
            vec![
                "input",
                "reconcile",
                "0",
                "--disposition",
                "submitted",
                "--evidence-ref",
                "audit:x",
            ],
        ] {
            assert_eq!(usage_error(&args).exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn runtime_read_dispatch_preserves_refused_database_bytes() {
        for damage in [
            "DROP TABLE member_input_operations",
            "ALTER TABLE member_runtime_events RENAME COLUMN notification_state TO broken",
            "ALTER TABLE runtime_database_identity RENAME COLUMN instance_id TO broken",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("refused.db");
            let settings = crate::config::Settings::from_lookup(|name| {
                (name == "CAFLEET_DATABASE_URL").then(|| format!("sqlite:///{}", path.display()))
            })
            .unwrap();
            let mut conn = crate::db::connect(&settings.database_url).unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            conn.execute_batch(damage).unwrap();
            drop(conn);
            let before = std::fs::read(&path).unwrap();
            for command in ["event", "input"] {
                let args =
                    CliArgs::try_parse_from(["cafleet", command, "show", "1", "--json"]).unwrap();
                assert!(
                    super::dispatch(&settings, args.command).is_err(),
                    "{damage}"
                );
                assert_eq!(std::fs::read(&path).unwrap(), before, "{command}: {damage}");
            }
        }
    }

    #[test]
    fn member_runner_internal_command_accepts_an_absolute_descriptor_and_stays_hidden() {
        use clap::CommandFactory;

        CliArgs::try_parse_from([
            "cafleet",
            "__member-run",
            "--descriptor",
            "/private/tmp/allocated-run/descriptor.json",
        ])
        .unwrap();
        let help = CliArgs::command().render_long_help().to_string();
        assert!(!help.contains("__member-run"));
    }

    fn flattened(text: impl ToString) -> String {
        text.to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn subcommand_help(path: &[&str]) -> String {
        use clap::CommandFactory;
        let mut command = CliArgs::command();
        let mut selected = &mut command;
        for name in path {
            selected = selected
                .find_subcommand_mut(name)
                .unwrap_or_else(|| panic!("registered subcommand {name}"));
        }
        flattened(selected.render_long_help())
    }

    fn possible_values(path: &[&str], argument: &str) -> Vec<String> {
        use clap::CommandFactory;
        let command = CliArgs::command();
        let mut selected = &command;
        for name in path {
            selected = selected.find_subcommand(name).unwrap();
        }
        selected
            .get_arguments()
            .find(|candidate| candidate.get_id() == argument)
            .unwrap_or_else(|| panic!("{path:?} declares {argument}"))
            .get_possible_values()
            .iter()
            .map(|value| value.get_name().to_owned())
            .collect()
    }

    #[test]
    fn help_describes_every_argument_of_the_runtime_read_input_press_and_recover_commands() {
        use clap::CommandFactory;
        let command = CliArgs::command();
        for path in [
            &["event", "show"][..],
            &["input", "show"],
            &["input", "reconcile"],
            &["member", "press"],
            &["member", "recover"],
        ] {
            let mut selected = &command;
            for name in path {
                selected = selected
                    .find_subcommand(name)
                    .unwrap_or_else(|| panic!("registered subcommand {name}"));
            }
            let mut described = 0;
            for argument in selected.get_arguments() {
                let id = argument.get_id().as_str();
                if ["help", "version"].contains(&id) {
                    continue;
                }
                let help = argument
                    .get_help()
                    .map(|help| help.to_string())
                    .unwrap_or_default();
                assert!(
                    !help.trim().is_empty(),
                    "{path:?} argument {id} has a help description"
                );
                described += 1;
            }
            assert!(described > 0, "{path:?} declares its arguments");
        }
    }

    #[test]
    fn member_exec_help_describes_wrapped_and_unwrapped_input_and_orders_member_before_command() {
        use clap::CommandFactory;
        let help = subcommand_help(&["member", "exec"]);
        let lower = help.to_lowercase();
        for term in ["unwrapped", "escape"] {
            assert!(
                lower.contains(term),
                "member exec help mentions {term}: {help}"
            );
        }
        assert!(
            lower
                .split("unwrapped")
                .next()
                .is_some_and(|before| before.contains("wrapped")),
            "member exec help describes wrapped members before the unwrapped exception: {help}"
        );
        let mut command = CliArgs::command();
        let usage = flattened(
            command
                .find_subcommand_mut("member")
                .unwrap()
                .find_subcommand_mut("exec")
                .unwrap()
                .render_usage(),
        );
        let member = usage.find("MEMBER_ID").expect("usage names MEMBER_ID");
        let body = usage.find("COMMAND").expect("usage names COMMAND");
        assert!(member < body, "MEMBER_ID precedes COMMAND: {usage}");
        match CliArgs::try_parse_from(["cafleet", "member", "exec", "17", "printf ok"])
            .unwrap()
            .command
        {
            crate::cli::Command::Member(crate::cli::member::MemberCommand::Exec {
                member_id,
                ..
            }) => {
                assert_eq!(member_id, 17)
            }
            _ => panic!("member exec parses MEMBER_ID then COMMAND"),
        }
    }

    #[test]
    fn help_describes_command_groups_and_derives_choices_from_their_registries() {
        use crate::coding_agent::CodingAgent;
        use clap::CommandFactory;
        let catalog = CodingAgent::all()
            .iter()
            .map(|agent| {
                let default = agent.resolve_policy(None, None).unwrap().resolved_mode;
                format!(
                    "{}: {} (default {default})",
                    agent.name(),
                    agent.policy_choices(None).join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let create = subcommand_help(&["member", "create"]);
        assert!(create.contains(&catalog), "{catalog} in {create}");
        let command = CliArgs::command();
        for group in ["event", "input", "permission"] {
            let about = command
                .find_subcommand(group)
                .unwrap_or_else(|| panic!("registered group {group}"))
                .get_about()
                .map(|about| about.to_string())
                .unwrap_or_default();
            assert!(
                !about.trim().is_empty(),
                "{group} has a top-level description"
            );
        }
        for (argument, values) in [
            ("decision", vec!["allow", "deny"]),
            ("decision_source", vec!["director-auto", "user"]),
            ("director_mode", vec!["auto", "manual", "unknown"]),
        ] {
            assert_eq!(
                possible_values(&["permission", "respond"], argument),
                values,
                "{argument}"
            );
            let respond = subcommand_help(&["permission", "respond"]);
            assert!(
                respond.contains(&format!("[possible values: {}]", values.join(", "))),
                "{argument} help lists {values:?}: {respond}"
            );
        }
        assert_eq!(
            possible_values(&["input", "reconcile"], "disposition"),
            ["submitted", "not-submitted", "abandoned-uncertain"]
        );
        assert_eq!(
            possible_values(&["member", "create"], "event_evidence"),
            ["metadata", "redacted"]
        );
        let invalid = usage_error(&[
            "permission",
            "respond",
            "1",
            "--event-id",
            "1",
            "--from-member-id",
            "1",
            "--expected-run-id",
            "1",
            "--expected-owner-epoch",
            "1",
            "--request-token",
            &"0".repeat(64),
            "--decision",
            "maybe",
            "--decision-source",
            "user",
            "--director-mode",
            "manual",
            "--mode-evidence-ref",
            "mode",
            "--decision-evidence-ref",
            "decision",
            "--operation-key",
            "key",
        ]);
        assert_eq!(invalid.exit_code(), 2);
        assert!(flattened(invalid.render()).contains("allow, deny"));
    }

    #[test]
    fn member_runner_internal_command_requires_its_descriptor_argument() {
        assert_eq!(usage_error(&["__member-run"]).exit_code(), 2);
        assert_eq!(
            usage_error(&["__member-run", "--descriptor"]).exit_code(),
            2
        );
    }

    #[test]
    fn member_runner_dispatch_rejects_an_unallocated_descriptor_before_runtime_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let descriptor = dir.path().join("unallocated.json");
        std::fs::write(&descriptor, b"{\"schema_version\":1}").unwrap();
        let settings = crate::config::Settings::from_lookup(|name| {
            (name == "CAFLEET_DATABASE_URL").then(|| format!("sqlite:///{}", path.display()))
        })
        .unwrap();
        let mut conn = crate::db::connect(&settings.database_url).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        let args = CliArgs::try_parse_from([
            "cafleet",
            "__member-run",
            "--descriptor",
            descriptor.to_str().unwrap(),
        ])
        .unwrap();
        assert!(super::dispatch(&settings, args.command).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

use clap::{Parser, Subcommand};

use crate::config::Settings;
use crate::error::CafleetError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "cafleet",
    version,
    about = "CAFleet — CLI for the message broker and member registry."
)]
struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "__member-run", hide = true)]
    MemberRun {
        #[arg(long, value_name = "ABSOLUTE_PATH")]
        descriptor: std::path::PathBuf,
    },
    /// Migrate storage and install embedded coding-agent skills/presets.
    Setup(setup::SetupArgs),
    /// Report independent environment, database and backend checks.
    Doctor(doctor::DoctorArgs),
    /// Fleet lifecycle.
    #[command(subcommand)]
    Fleet(fleet::FleetCommand),
    /// Member lifecycle and pane interaction.
    #[command(subcommand)]
    Member(member::MemberCommand),
    /// Durable point-to-point messages.
    #[command(subcommand)]
    Message(message::MessageCommand),
    /// Read a durable runtime event and current state.
    #[command(subcommand)]
    Event(runtime::EventCommand),
    /// Read / settle a durable input receipt.
    #[command(subcommand)]
    Input(runtime::InputCommand),
    /// Inspect / answer the current native permission request.
    #[command(subcommand)]
    Permission(runtime::PermissionCommand),
}

pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    dispatch(&settings, args.command)
}

fn invocation_connection(database_url: &str) -> Result<rusqlite::Connection, CafleetError> {
    invocation_connection_with_deadline(database_url, crate::input::Deadline::new())
}

fn invocation_connection_with_deadline(
    database_url: &str,
    deadline: crate::input::Deadline,
) -> Result<rusqlite::Connection, CafleetError> {
    let path = crate::db::database_path(database_url)?;
    if !std::path::Path::new(path).try_exists().map_err(|cause| {
        CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
    })? {
        return Err(CafleetError::App(
            "no cafleet database; run 'cafleet setup'".into(),
        ));
    }
    let validate = |conn: &mut rusqlite::Connection| {
        crate::input::with_database_budget(conn, deadline, |conn| {
            crate::diagnosis::classify_selected_schema(
                conn,
                crate::db::head_version(),
                database_url,
            )
            .require_current()
        })
    };
    let mut preflight = crate::db::connect_read_only(database_url)?;
    validate(&mut preflight)?;
    drop(preflight);
    let mut conn = crate::db::connect(database_url)?;
    validate(&mut conn)?;
    Ok(conn)
}

fn dispatch(settings: &Settings, command: Command) -> Result<(), CafleetError> {
    match command {
        Command::Setup(cmd) => setup::run(settings, cmd),
        Command::Doctor(cmd) => doctor::run(settings, cmd),
        Command::MemberRun { descriptor } => {
            let code = crate::runtime::runner::run(settings, &descriptor)?;
            std::process::exit(code)
        }
        Command::Member(member::MemberCommand::Press {
            member_id,
            keys,
            json,
        }) => {
            let deadline = crate::input::Deadline::inherited_or_new();
            let keys = crate::multiplexer::PressKeys::parse(&keys)?;
            let mut conn = invocation_connection_with_deadline(&settings.database_url, deadline)
                .map_err(|cause| CafleetError::WithDiagnostics {
                    cause: Box::new(cause),
                    diagnostics: vec![format!("keys were not dispatched to member {member_id}.")],
                })?;
            member::press(&mut conn, settings, member_id, &keys, json, deadline)
        }
        Command::Permission(command) => {
            let deadline = crate::input::Deadline::new();
            runtime::permission_preflight(&command)
                .map_err(|source| runtime::permission_error(&command, source))?;
            let mut conn = invocation_connection_with_deadline(&settings.database_url, deadline)
                .map_err(|source| runtime::permission_error(&command, source))?;
            runtime::permission(&mut conn, settings, command, deadline)
        }
        mut command => {
            let mut conn = invocation_connection(&settings.database_url)?;
            if let Command::Member(command) = &mut command {
                member::prepare_input(command)?;
            }
            let deadline = crate::input::Deadline::new();
            match command {
                Command::Fleet(cmd) => fleet::run(&mut conn, settings, cmd, deadline),
                Command::Member(cmd) => member::run(&mut conn, settings, cmd, deadline),
                Command::Message(cmd) => message::run(&mut conn, settings, cmd),
                Command::Event(cmd) => runtime::event(&conn, cmd),
                Command::Input(cmd) => runtime::input(&mut conn, settings, cmd, deadline),
                Command::Setup(_)
                | Command::Doctor(_)
                | Command::MemberRun { .. }
                | Command::Permission(_) => {
                    unreachable!("handled above")
                }
            }
        }
    }
}

#[cfg(test)]
mod parser_contracts {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn ordinary_commands_preserve_refused_wal_database_and_sidecar_bytes() {
        for mutation in [
            "PRAGMA application_id=42",
            "UPDATE refinery_schema_history SET version=99 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)",
            "ALTER TABLE messages RENAME COLUMN text TO wrong",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("registry.db");
            let settings = Settings {
                database_url: format!("sqlite:///{}", path.display()),
                max_text_len: 200,
                multiplexer: None,
            };
            let mut conn = crate::db::connect(&settings.database_url).unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                .unwrap();
            let before = ["", "-wal", "-shm"].map(|suffix| {
                let file = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
                let bytes = std::fs::read(&file).unwrap();
                (file, bytes)
            });
            for command in [
                Command::Fleet(fleet::FleetCommand::List { json: true }),
                Command::Member(member::MemberCommand::Show {
                    member_id: 1,
                    json: true,
                }),
                Command::Message(message::MessageCommand::Poll {
                    member_id: 1,
                    json: true,
                }),
            ] {
                assert!(dispatch(&settings, command).is_err(), "{mutation}");
                for (file, bytes) in &before {
                    assert_eq!(&std::fs::read(file).unwrap(), bytes, "{}", file.display());
                }
            }
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }

    #[test]
    fn ordinary_commands_require_storage_without_creating_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.db");
        assert!(invocation_connection(&format!("sqlite:///{}", path.display())).is_err());
        assert!(!path.exists());
    }

    fn parse(args: &[&str]) -> Command {
        CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied()))
            .unwrap_or_else(|error| panic!("{args:?}: {error}"))
            .command
    }

    fn assert_error(args: &[&str], kind: ErrorKind, input: &str) {
        let error =
            match CliArgs::try_parse_from(std::iter::once("cafleet").chain(args.iter().copied())) {
                Err(error) => error,
                Ok(_) => panic!("{args:?} must reject {input}"),
            };
        assert_eq!(error.kind(), kind, "{args:?}: {error}");
        assert!(error.to_string().contains(input), "{args:?}: {error}");
    }

    #[test]
    fn command_options_are_scoped_to_their_subcommands() {
        let cases: &[(&[&str], &str)] = &[
            (&["--json", "fleet", "list"], "--json"),
            (&["setup", "--fleet-id", "1"], "--fleet-id"),
            (
                &[
                    "fleet",
                    "create",
                    "--fleet-id",
                    "1",
                    "--name",
                    "x",
                    "--coding-agent",
                    "claude",
                ],
                "--fleet-id",
            ),
            (&["fleet", "list", "--fleet-id", "1"], "--fleet-id"),
            (&["fleet", "show", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "list", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "show", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["message", "poll", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["message", "ack", "1", "--fleet-id", "1"], "--fleet-id"),
            (&["member", "show", "--member-id", "1"], "--member-id"),
            (&["message", "poll", "--member-id", "1"], "--member-id"),
            (&["message", "ack", "--message-id", "1"], "--message-id"),
            (&["message", "show", "--message-id", "1"], "--message-id"),
            (
                &[
                    "fleet",
                    "create",
                    "--name",
                    "x",
                    "--coding-agent",
                    "claude",
                    "--full",
                ],
                "--full",
            ),
            (&["fleet", "show", "1", "--full"], "--full"),
            (&["member", "show", "1", "--full"], "--full"),
            (&["member", "list", "1", "--full"], "--full"),
            (&["message", "show", "1", "--full"], "--full"),
            (&["message", "poll", "1", "--full"], "--full"),
            (
                &[
                    "message",
                    "send",
                    "--from-member-id",
                    "1",
                    "--to-member-id",
                    "2",
                    "hi",
                    "--quiet",
                ],
                "--quiet",
            ),
            (&["message", "ack", "1", "--quiet"], "--quiet"),
            (&["member", "capture", "1", "--no-ansi"], "--no-ansi"),
            (
                &[
                    "message",
                    "send",
                    "--from-member-id",
                    "1",
                    "--to-member-id",
                    "2",
                    "--text",
                    "hi",
                ],
                "--text",
            ),
            (
                &[
                    "member",
                    "create",
                    "--fleet-id",
                    "1",
                    "--name",
                    "w",
                    "--description",
                    "d",
                    "--text",
                    "prompt",
                ],
                "--text",
            ),
        ];
        for (args, input) in cases {
            assert_error(args, ErrorKind::UnknownArgument, input);
        }
        assert!(matches!(
            parse(&["fleet", "list", "--json"]),
            Command::Fleet(fleet::FleetCommand::List { json: true })
        ));
        assert!(matches!(
            parse(&["fleet", "show", "41", "--json"]),
            Command::Fleet(fleet::FleetCommand::Show {
                fleet_id: 41,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["member", "show", "42", "--json"]),
            Command::Member(member::MemberCommand::Show {
                member_id: 42,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["message", "poll", "43", "--json"]),
            Command::Message(message::MessageCommand::Poll {
                member_id: 43,
                json: true
            })
        ));
        assert!(matches!(
            parse(&["message", "ack", "44", "--json"]),
            Command::Message(message::MessageCommand::Ack {
                message_id: 44,
                json: true
            })
        ));
    }

    #[test]
    fn fleet_creation_requires_named_options_and_integer_subjects() {
        for (args, kind, input) in [
            (
                vec!["fleet", "create", "--coding-agent", "claude"],
                ErrorKind::MissingRequiredArgument,
                "--name",
            ),
            (
                vec!["fleet", "create", "--name", "alpha"],
                ErrorKind::MissingRequiredArgument,
                "--coding-agent",
            ),
            (
                vec![
                    "fleet",
                    "create",
                    "--name",
                    "alpha",
                    "--coding-agent",
                    "python",
                ],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                vec!["fleet", "show"],
                ErrorKind::MissingRequiredArgument,
                "FLEET_ID",
            ),
            (
                vec!["fleet", "show", "abc"],
                ErrorKind::ValueValidation,
                "abc",
            ),
        ] {
            assert_error(&args, kind, input);
        }
        let Command::Fleet(fleet::FleetCommand::Create {
            name,
            coding_agent,
            json,
        }) = parse(&[
            "fleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "codex",
            "--json",
        ])
        else {
            panic!("expected fleet creation");
        };
        assert_eq!(name, "alpha");
        assert_eq!(coding_agent, crate::coding_agent::CodingAgent::Codex);
        assert!(json);
    }

    #[test]
    fn member_and_message_body_sources_are_exclusive() {
        let member = [
            "member",
            "create",
            "--fleet-id",
            "17",
            "--name",
            "worker",
            "--description",
            "work",
        ];
        let send = [
            "message",
            "send",
            "--from-member-id",
            "17",
            "--to-member-id",
            "18",
        ];
        let exec = ["member", "exec", "17"];
        for base in [member.as_slice(), send.as_slice(), exec.as_slice()] {
            assert_error(base, ErrorKind::MissingRequiredArgument, "--file");
            let both: Vec<_> = base
                .iter()
                .copied()
                .chain(["payload", "--file", "body.txt"])
                .collect();
            assert_error(&both, ErrorKind::ArgumentConflict, "--file");
            for source in [
                vec!["payload"],
                vec!["--file", "body.txt"],
                vec!["--file", "-"],
            ] {
                let args: Vec<_> = base.iter().copied().chain(source).collect();
                match (base[0], parse(&args)) {
                    ("member", Command::Member(member::MemberCommand::Create { fleet_id, .. })) => {
                        assert_eq!(fleet_id, 17)
                    }
                    ("member", Command::Member(member::MemberCommand::Exec { member_id, .. })) => {
                        assert_eq!(member_id, 17)
                    }
                    (
                        "message",
                        Command::Message(message::MessageCommand::Send {
                            from_member_id,
                            to_member_id,
                            ..
                        }),
                    ) => assert_eq!((from_member_id, to_member_id), (17, 18)),
                    _ => panic!("{args:?} must preserve its command variant"),
                }
            }
        }
    }

    #[test]
    fn receive_leases_validate_integer_bounds_and_optional_listening() {
        assert!(matches!(
            parse(&["message", "wait", "17"]),
            Command::Message(message::MessageCommand::Wait {
                member_id: 17,
                timeout: 300,
                ..
            })
        ));
        assert!(matches!(
            parse(&["message", "listen", "17"]),
            Command::Message(message::MessageCommand::Listen {
                member_id: 17,
                timeout: None
            })
        ));
        for command in ["wait", "listen"] {
            for seconds in ["1", "1800"] {
                let parsed = parse(&["message", command, "17", "--timeout", seconds]);
                match parsed {
                    Command::Message(message::MessageCommand::Wait { timeout, .. }) => {
                        assert_eq!(timeout, seconds.parse::<u64>().unwrap());
                    }
                    Command::Message(message::MessageCommand::Listen { timeout, .. }) => {
                        assert_eq!(timeout, Some(seconds.parse::<u64>().unwrap()));
                    }
                    _ => panic!("expected finite receive"),
                }
            }
            for seconds in ["0", "1801", "fraction", "1.5"] {
                assert_error(
                    &["message", command, "17", "--timeout", seconds],
                    ErrorKind::ValueValidation,
                    seconds,
                );
            }
        }
        assert_error(
            &["message", "listen", "17", "--json"],
            ErrorKind::UnknownArgument,
            "--json",
        );
    }

    #[test]
    fn setup_rejects_unknown_options_positionals_and_agent_values() {
        let cases: &[(&[&str], ErrorKind, &str)] = &[
            (&["setup", "extra"], ErrorKind::UnknownArgument, "extra"),
            (&["setup", "claude"], ErrorKind::UnknownArgument, "claude"),
            (
                &["setup", "--skip", "claude"],
                ErrorKind::UnknownArgument,
                "--skip",
            ),
            (
                &["setup", "--coding-agent", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "extra"],
                ErrorKind::InvalidValue,
                "extra",
            ),
        ];
        for (args, kind, input) in cases {
            assert_error(args, *kind, input);
        }
    }
}
