use clap::Parser;

use super::CliArgs;

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
