use super::*;
use crate::broker::runtime::ModeSource;

fn version(backend: CodingAgent) -> Option<&'static str> {
    match backend {
        CodingAgent::Claude => Some("2.1.302"),
        CodingAgent::Codex => Some("0.163.0"),
        CodingAgent::Opencode => None,
    }
}

fn permission_argv(backend: CodingAgent, selected: &str) -> Vec<String> {
    match backend {
        CodingAgent::Claude => vec!["--permission-mode".into(), selected.into()],
        CodingAgent::Codex => vec![
            "--ask-for-approval".into(),
            selected.into(),
            "--sandbox".into(),
            "workspace-write".into(),
        ],
        CodingAgent::Opencode => vec!["--agent".into(), "cafleet".into()],
    }
}

#[test]
fn runtime_policy_omission_preserves_the_complete_existing_native_argv() {
    for &backend in CodingAgent::all() {
        let selection = backend.resolve_policy(version(backend), None).unwrap();
        let expected_mode = match backend {
            CodingAgent::Claude => "dontAsk",
            CodingAgent::Codex => "never",
            CodingAgent::Opencode => "cafleet",
        };
        assert_eq!(selection.requested_mode, None);
        assert_eq!(selection.resolved_mode, expected_mode);
        assert_eq!(selection.mode_source, ModeSource::Default);
        assert_eq!(
            selection.permission_argv,
            permission_argv(backend, expected_mode)
        );
        assert_eq!(
            backend
                .build_spawn_argv_with_policy(
                    "literal '$x' `text`",
                    "fixture name",
                    Some("provider/model"),
                    None,
                    &selection
                )
                .unwrap(),
            backend.build_spawn_argv(
                "literal '$x' `text`",
                "fixture name",
                Some("provider/model"),
                None
            )
        );
    }
}

#[test]
fn runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode() {
    for &backend in CodingAgent::all() {
        let choices = backend.policy_choices(version(backend));
        assert!(!choices.is_empty());
        let mut unique = std::collections::BTreeSet::new();
        for mode in choices {
            assert!(unique.insert(mode.clone()), "duplicate policy {mode}");
            let selection = backend
                .resolve_policy(version(backend), Some(&mode))
                .unwrap();
            assert_eq!(selection.requested_mode.as_deref(), Some(mode.as_str()));
            assert_eq!(selection.resolved_mode, mode);
            assert_eq!(selection.mode_source, ModeSource::Explicit);
            assert_eq!(selection.permission_argv, permission_argv(backend, &mode));
            let mut expected: Vec<String> = match backend {
                CodingAgent::Claude => vec![
                    "claude".into(),
                    "--permission-mode".into(),
                    mode.clone(),
                    "--prompt-suggestions".into(),
                    "false".into(),
                    "--allowedTools".into(),
                    "Bash(cafleet message *)".into(),
                    "--name".into(),
                    "fixture name".into(),
                ],
                CodingAgent::Codex => {
                    let mut argv = vec!["codex".into()];
                    argv.extend(permission_argv(backend, &mode));
                    argv
                }
                CodingAgent::Opencode => vec![
                    "opencode".into(),
                    "mini".into(),
                    "--standalone".into(),
                    "--agent".into(),
                    "cafleet".into(),
                ],
            };
            expected.extend(["--model".into(), "provider/model".into()]);
            let effort = (backend != CodingAgent::Opencode).then_some("high");
            match backend {
                CodingAgent::Claude => expected.extend(["--effort".into(), "high".into()]),
                CodingAgent::Codex => expected.push("--config=model_reasoning_effort=high".into()),
                CodingAgent::Opencode => expected.push("--prompt".into()),
            }
            expected.push("literal '$x' `text` assignment".into());
            assert_eq!(
                backend
                    .build_spawn_argv_with_policy(
                        "literal '$x' `text` assignment",
                        "fixture name",
                        Some("provider/model"),
                        effort,
                        &selection
                    )
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn runtime_policy_claude_default_and_manual_preserve_their_requested_spelling() {
    let backend = CodingAgent::Claude;
    for mode in ["default", "manual"] {
        assert!(
            backend
                .policy_choices(Some("2.1.302"))
                .contains(&mode.into())
        );
        let selected = backend.resolve_policy(Some("2.1.302"), Some(mode)).unwrap();
        assert_eq!(selected.resolved_mode, mode);
        assert_eq!(selected.permission_argv, ["--permission-mode", mode]);
    }
}

#[test]
fn runtime_policy_invalid_explicit_selection_reports_backend_and_catalog_choices() {
    for &backend in CodingAgent::all() {
        for invalid in ["", " ", "unsupported-fixture-mode"] {
            let failure = backend
                .resolve_policy(version(backend), Some(invalid))
                .expect_err("invalid explicit policy must fail");
            assert!(matches!(failure, CafleetError::Usage(_)));
            let text = failure.message();
            assert!(text.contains(backend.name()), "{text}");
            for choice in backend.policy_choices(version(backend)) {
                assert!(text.contains(&choice), "{text}");
            }
        }
    }
}

#[test]
fn runtime_policy_fixed_preset_and_unknown_native_version_keep_launch_intent() {
    let backend = CodingAgent::Opencode;
    assert_eq!(backend.policy_choices(None), ["cafleet"]);
    assert_eq!(
        backend
            .resolve_policy(None, Some("cafleet"))
            .unwrap()
            .mode_source,
        ModeSource::Explicit
    );
    for &backend in CodingAgent::all() {
        let known = backend.resolve_policy(version(backend), None).unwrap();
        let unknown = backend.resolve_policy(None, None).unwrap();
        assert_eq!(unknown.resolved_mode, known.resolved_mode);
        assert_eq!(unknown.permission_argv, known.permission_argv);
    }
}

#[test]
fn runtime_policy_foreign_selection_is_rejected_before_native_argv_construction() {
    let selected = CodingAgent::Claude
        .resolve_policy(Some("2.1.302"), Some("default"))
        .unwrap();
    assert!(
        CodingAgent::Codex
            .build_spawn_argv_with_policy("assignment", "worker", None, None, &selected)
            .is_err()
    );
}
