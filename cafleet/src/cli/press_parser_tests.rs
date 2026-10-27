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
