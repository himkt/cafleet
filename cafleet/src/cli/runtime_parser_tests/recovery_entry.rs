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
fn runner_recovery_entry_unwrapped_member_fails_explicitly_before_registry_or_artifact_mutation() {
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
    let args = CliArgs::try_parse_from(["cafleet", "member", "recover", &id, "--json"]).unwrap();
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
            let path = std::path::PathBuf::from(format!("{}{suffix}", conn.path().unwrap()));
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        });
        let id = member_id.to_string();
        let args =
            CliArgs::try_parse_from(["cafleet", "member", "recover", &id, "--json"]).unwrap();
        assert!(super::super::dispatch(&settings, args.command).is_err());
        for (path, bytes) in before {
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{mutation}: {path:?}");
        }
    }
}
