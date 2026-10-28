use super::CodingAgent;
use super::catalog::{Capture, Profile, ProfileCatalog, Rule};
use crate::broker::runtime::InteractionClass;
use crate::coding_agent::AnswerPlan;
use crate::error::CafleetError;
use crate::runtime::runner::ScreenSnapshot;
use serde_json::{Value, json};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, CafleetError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservationKind {
    Interaction(InteractionClass),
    PermissionDenied,
    Working,
    ComposerEmpty,
    ComposerDraft,
    Unknown,
}

#[cfg(test)]
pub(super) mod inspection_tests {
    use super::*;
    use crate::broker::runtime::PermissionDecision;
    use crate::coding_agent::{inspection_fixture_definition, inspection_fixture_form};
    use crate::runtime::runner::{TerminalSize, TerminalState};

    #[test]
    fn permission_inspection_answer_match_uses_complete_current_rule_choice_and_focus() {
        let catalog = ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
        let detector =
            Detector::new(&catalog, CodingAgent::Codex, Some("synthetic-0.0.1")).unwrap();
        let plan = catalog
            .answer_plan(
                CodingAgent::Codex,
                "synthetic-0.0.1",
                "synthetic-permission",
                PermissionDecision::Allow,
            )
            .unwrap()
            .unwrap();
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        terminal
            .feed(&inspection_fixture_form(
                "private operation",
                "synthetic-only",
            ))
            .unwrap();
        let screen = terminal.snapshot();
        let observation = detector.observe(&screen).unwrap();
        let matched = detector.match_answer(&screen, &plan).unwrap().unwrap();
        assert_eq!(
            observation.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert!(Some(matched.semantic()) == observation.fingerprint.as_ref());
        assert_eq!(
            matched.choices(),
            &["Allow once".to_owned(), "Deny once".to_owned()]
        );
        assert_eq!(matched.output_seq(), screen.screen_seq);
        assert_eq!(matched.plan().choice_id(), "allow-once");
        let focus = matched.focus().to_owned();
        assert!(!focus.is_empty());
        terminal
            .feed(&inspection_fixture_form(
                "private operation",
                "synthetic-other",
            ))
            .unwrap();
        let other = detector
            .match_answer(&terminal.snapshot(), &plan)
            .unwrap()
            .unwrap();
        assert!(other.semantic() == matched.semantic());
        assert_ne!(other.focus(), focus);
        for form in ["missing-focus", "synthetic-alt"] {
            terminal
                .feed(&inspection_fixture_form("private operation", form))
                .unwrap();
            assert!(
                detector
                    .match_answer(&terminal.snapshot(), &plan)
                    .unwrap()
                    .is_none()
            );
        }
        terminal
            .feed(&inspection_fixture_form(
                "private operation",
                "synthetic-only",
            ))
            .unwrap();
        terminal.feed(b"\x1b[21;1H  Allow once").unwrap();
        assert!(
            detector
                .match_answer(&terminal.snapshot(), &plan)
                .unwrap()
                .is_none()
        );
    }

    const CLAUDE_VERSION: &str = "2.1.302";
    const NATIVE_SCREEN: &str = include_str!("native/claude-2.1.302-default-permission-1.json");
    const ALLOW_SCREEN: &str = include_str!("native/claude-2.1.302-allow-pre-stage-1.json");
    const DENY_SCREEN: &str = include_str!("native/claude-2.1.302-deny-pre-stage-1.json");
    const IDLE_SCREEN: &str = include_str!("native/claude-2.1.302-idle-composer-1.json");

    fn native_screen(change: impl FnOnce(&mut Vec<String>), alternate: bool) -> ScreenSnapshot {
        replay_native_fixture(NATIVE_SCREEN, change, alternate, None)
    }

    pub(in crate::coding_agent) fn replay_native_fixture(
        fixture: &str,
        change: impl FnOnce(&mut Vec<String>),
        alternate: bool,
        cursor: Option<(u16, u16)>,
    ) -> ScreenSnapshot {
        let screen: Value = serde_json::from_str(fixture).unwrap();
        let mut lines: Vec<String> = screen["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        change(&mut lines);
        let mut terminal = TerminalState::new(TerminalSize {
            rows: 36,
            cols: 120,
        })
        .unwrap();
        if alternate {
            terminal.feed(b"\x1b[?1049h").unwrap();
        }
        terminal.feed(b"\x1b[2J").unwrap();
        for (row, line) in lines.iter().enumerate() {
            if !line.is_empty() {
                terminal
                    .feed(format!("\x1b[{};1H{line}", row + 1).as_bytes())
                    .unwrap();
            }
        }
        let (row, col) = cursor.unwrap_or((
            u16::try_from(screen["cursor_row"].as_u64().unwrap()).unwrap(),
            u16::try_from(screen["cursor_col"].as_u64().unwrap()).unwrap(),
        ));
        terminal
            .feed(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes())
            .unwrap();
        terminal.snapshot()
    }

    fn allow_or_deny_plan(decision: PermissionDecision) -> AnswerPlan {
        ProfileCatalog::bundled()
            .unwrap()
            .answer_plan(
                CodingAgent::Claude,
                CLAUDE_VERSION,
                "claude-bash-permission",
                decision,
            )
            .unwrap()
            .expect("measured bundled answer plan")
    }

    fn claude_answer(screen: &ScreenSnapshot, plan: &AnswerPlan) -> Option<AnswerMatch> {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
            .unwrap()
            .match_answer(screen, plan)
            .unwrap()
    }

    #[test]
    fn bundled_claude_answer_plans_match_their_measured_pre_stage_screens_and_bind_focus() {
        for (decision, fixture, choice) in [
            (PermissionDecision::Allow, ALLOW_SCREEN, "allow-once"),
            (PermissionDecision::Deny, DENY_SCREEN, "deny-once"),
        ] {
            let plan = allow_or_deny_plan(decision);
            let screen = replay_native_fixture(fixture, |_| {}, true, None);
            let matched = claude_answer(&screen, &plan).expect("pre-stage form matches its plan");
            assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
            assert_eq!(matched.plan().choice_id(), choice);
            assert!(!matched.focus().is_empty());
            assert!(Some(matched.semantic()) == claude_observation(&screen).fingerprint.as_ref());
            let moved = replay_native_fixture(
                fixture,
                |lines| {
                    lines[23] = lines[23].replacen('❯', " ", 1);
                    lines[24] = lines[24].replacen("   2.", " ❯ 2.", 1);
                },
                true,
                Some((24, 1)),
            );
            let refocused =
                claude_answer(&moved, &plan).expect("a moved focus still identifies the form");
            assert!(refocused.semantic() == matched.semantic());
            assert_ne!(
                refocused.focus(),
                matched.focus(),
                "token binding sees the focus move"
            );
            let unfocused = replay_native_fixture(
                fixture,
                |lines| lines[23] = lines[23].replacen('❯', " ", 1),
                true,
                None,
            );
            assert!(
                claude_answer(&unfocused, &plan).is_none(),
                "{decision:?} without ❯"
            );
            let primary = replay_native_fixture(fixture, |_| {}, false, None);
            assert!(
                claude_answer(&primary, &plan).is_none(),
                "{decision:?} primary screen"
            );
        }
    }

    #[test]
    fn bundled_claude_idle_composer_is_a_verified_non_interaction_distinct_from_the_form() {
        let idle = claude_observation(&replay_native_fixture(IDLE_SCREEN, |_| {}, true, None));
        assert_eq!(idle.kind, ObservationKind::ComposerEmpty);
        assert_eq!(idle.match_evidence, MatchEvidence::Complete);
        assert!(idle.verified_noninteraction);
        assert_eq!(idle.rule_id, "claude-idle-composer");
        let region = idle
            .matched_region
            .expect("the idle rule locates its composer");
        assert!(region.top >= 12, "{region:?}");
        let form = claude_observation(&native_screen(|_| {}, true));
        assert_eq!(form.rule_id, "claude-bash-permission");
        assert_eq!(
            form.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert_eq!(form.match_evidence, MatchEvidence::Complete);
        assert!(!form.verified_noninteraction);
    }

    #[test]
    fn bundled_claude_idle_rule_rejects_drafts_primary_copies_and_a_cursor_off_the_prefix_end() {
        let draft = claude_observation(&replay_native_fixture(
            IDLE_SCREEN,
            |lines| lines[31] = "❯\u{a0}hello".into(),
            true,
            Some((31, 7)),
        ));
        assert_ne!(draft.kind, ObservationKind::ComposerEmpty, "typed draft");
        assert!(!draft.verified_noninteraction);
        let primary = claude_observation(&replay_native_fixture(IDLE_SCREEN, |_| {}, false, None));
        assert!(!primary.verified_noninteraction, "primary-screen copy");
        let away = claude_observation(&replay_native_fixture(
            IDLE_SCREEN,
            |_| {},
            true,
            Some((31, 0)),
        ));
        assert!(
            !away.verified_noninteraction,
            "cursor before the prefix end"
        );
        assert_ne!(away.kind, ObservationKind::ComposerEmpty);
    }

    fn claude_observation(screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
            .unwrap()
            .observe(screen)
            .unwrap()
    }

    #[test]
    fn bundled_claude_detector_replays_the_native_permission_fixture_from_unmasked_rows() {
        let screen = native_screen(|_| {}, true);
        assert_eq!((screen.cursor_row, screen.cursor_col), (23, 1));
        let observation = claude_observation(&screen);
        assert_eq!(
            observation.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert_eq!(observation.match_evidence, MatchEvidence::Complete);
        assert_eq!(
            observation.raw_evidence["operation"],
            "/private/tmp/cafleet-194/cells-002/permission-probe allow"
        );
        assert_eq!(observation.raw_evidence["choices"], json!(["Yes", "No"]));
        assert!(!observation.verified_noninteraction);
        assert!(observation.native_correlation.is_none());
        let region = observation
            .matched_region
            .expect("a permission match locates its live region");
        assert!(
            region.top >= 12,
            "detection region lies below the masked header: {region:?}"
        );
        assert!(region.top <= 14 && region.bottom >= 26, "{region:?}");
        let replay = claude_observation(&native_screen(|_| {}, true));
        assert!(observation.fingerprint.is_some());
        assert!(replay.fingerprint == observation.fingerprint);
        let other_version = ProfileCatalog::bundled().unwrap();
        let unprofiled = Detector::new(&other_version, CodingAgent::Claude, Some("2.1.301"))
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_eq!(unprofiled.kind, ObservationKind::Unknown);
    }

    #[test]
    fn bundled_claude_detector_rejects_copies_that_are_not_the_live_permission_form() {
        let permission = ObservationKind::Interaction(InteractionClass::PermissionRequest);
        let primary = claude_observation(&native_screen(|_| {}, false));
        assert_ne!(primary.kind, permission, "primary-screen transcript copy");
        let unfocused = claude_observation(&native_screen(
            |lines| lines[23] = lines[23].replace('❯', " "),
            true,
        ));
        assert_ne!(unfocused.kind, permission, "form without a focus marker");
        let broadened = claude_observation(&native_screen(
            |lines| {
                lines[23].clear();
                lines[26].clear();
            },
            true,
        ));
        assert_ne!(broadened.kind, permission, "only broadened choices remain");
        let headless = claude_observation(&native_screen(|lines| lines[14].clear(), true));
        assert_ne!(headless.kind, permission, "form without its heading");
    }

    const CODEX_VERSION: &str = "0.163.0";
    const CODEX_PERMISSION_SCREEN: &str =
        include_str!("native/codex-0.163.0-on-request-permission-1.json");
    const CODEX_ALLOW_SCREEN: &str = include_str!("native/codex-0.163.0-allow-pre-stage-1.json");
    const CODEX_DENY_SCREEN: &str = include_str!("native/codex-0.163.0-deny-pre-stage-1.json");
    const CODEX_IDLE_SCREEN: &str = include_str!("native/codex-0.163.0-idle-composer-1.json");
    const CODEX_CHOICES: [&str; 2] = ["Yes, proceed", "No, and tell Codex what to do differently"];

    fn codex_observation(screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Codex, Some(CODEX_VERSION))
            .unwrap()
            .observe(screen)
            .unwrap()
    }

    fn codex_plan(decision: PermissionDecision) -> AnswerPlan {
        ProfileCatalog::bundled()
            .unwrap()
            .answer_plan(
                CodingAgent::Codex,
                CODEX_VERSION,
                "codex-exec-approval",
                decision,
            )
            .unwrap()
            .expect("measured bundled Codex answer plan")
    }

    fn codex_answer(screen: &ScreenSnapshot, plan: &AnswerPlan) -> Option<AnswerMatch> {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Codex, Some(CODEX_VERSION))
            .unwrap()
            .match_answer(screen, plan)
            .unwrap()
    }

    fn codex_form(
        change: impl FnOnce(&mut Vec<String>),
        cursor: Option<(u16, u16)>,
    ) -> ScreenSnapshot {
        replay_native_fixture(CODEX_PERMISSION_SCREEN, change, true, cursor)
    }

    // Writing the full-width cursor row last leaves the terminal in its pending-wrap state, which an
    // absolute cursor move cannot reach.
    fn replay_with_pending_wrap(
        fixture: &str,
        change: impl FnOnce(&mut Vec<String>),
    ) -> ScreenSnapshot {
        let screen: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(
            screen["cursor_col"], 120,
            "fixture records a pending-wrap cursor"
        );
        let row = usize::try_from(screen["cursor_row"].as_u64().unwrap()).unwrap();
        let mut lines: Vec<String> = screen["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        change(&mut lines);
        assert_eq!(
            lines[row].chars().count(),
            120,
            "the cursor row fills the width"
        );
        let mut terminal = TerminalState::new(TerminalSize {
            rows: 36,
            cols: 120,
        })
        .unwrap();
        terminal.feed(b"\x1b[?1049h\x1b[2J").unwrap();
        for (index, line) in lines.iter().enumerate() {
            if index != row && !line.is_empty() {
                terminal
                    .feed(format!("\x1b[{};1H{line}", index + 1).as_bytes())
                    .unwrap();
            }
        }
        terminal
            .feed(format!("\x1b[{};1H{}", row + 1, lines[row]).as_bytes())
            .unwrap();
        terminal.snapshot()
    }

    fn continued_choice_two(lines: &mut Vec<String>) {
        let choice = lines[32]
            .strip_suffix(" (p)")
            .expect("choice 2 fits one row")
            .to_owned();
        lines.remove(12);
        lines[31] = choice;
        lines.insert(32, "     (p)".into());
    }

    #[test]
    fn bundled_codex_detector_replays_the_native_approval_form_from_unmasked_rows() {
        let screen = codex_form(|_| {}, None);
        let observation = codex_observation(&screen);
        assert_eq!(
            observation.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert_eq!(observation.rule_id, "codex-exec-approval");
        assert_eq!(observation.match_evidence, MatchEvidence::Complete);
        assert_eq!(
            observation.raw_evidence["operation"],
            "/tmp/cafleet-194/codex-cells-001/permission-probe allow"
        );
        assert_eq!(observation.raw_evidence["choices"], json!(CODEX_CHOICES));
        assert!(!observation.verified_noninteraction);
        assert!(observation.native_correlation.is_none());
        let region = observation
            .matched_region
            .expect("an approval match locates its live region");
        assert!(region.top >= 12, "{region:?}");
        assert!(region.top <= 22 && region.bottom >= 36, "{region:?}");
        let replay = codex_observation(&codex_form(|_| {}, None));
        assert!(observation.fingerprint.is_some());
        assert!(replay.fingerprint == observation.fingerprint);
        let catalog = ProfileCatalog::bundled().unwrap();
        let unprofiled = Detector::new(&catalog, CodingAgent::Codex, Some("0.162.0"))
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_eq!(unprofiled.kind, ObservationKind::Unknown);
    }

    #[test]
    fn bundled_codex_detection_counts_a_pending_wrap_cursor_on_the_full_width_closing_row_as_inside()
     {
        let screen = replay_with_pending_wrap(CODEX_PERMISSION_SCREEN, |_| {});
        assert_eq!(
            (screen.cursor_row, screen.cursor_col),
            (35, 120),
            "the live terminal reports the pending-wrap column"
        );
        let observation = codex_observation(&screen);
        assert_eq!(
            observation.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest),
            "a pending-wrap cursor on the closing row lies inside the form"
        );
        assert!(
            observation.fingerprint == codex_observation(&codex_form(|_| {}, None)).fingerprint
        );
        for fixture in [CODEX_ALLOW_SCREEN, CODEX_DENY_SCREEN] {
            let pre_stage = codex_observation(&replay_with_pending_wrap(fixture, |_| {}));
            assert_eq!(
                pre_stage.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
        }
    }

    #[test]
    fn bundled_codex_detector_rejects_copies_that_are_not_the_live_approval_form() {
        let permission = ObservationKind::Interaction(InteractionClass::PermissionRequest);
        let primary = codex_observation(&replay_native_fixture(
            CODEX_PERMISSION_SCREEN,
            |_| {},
            false,
            None,
        ));
        assert_ne!(primary.kind, permission, "primary-screen transcript copy");
        let unfocused = codex_observation(&codex_form(
            |lines| lines[31] = lines[31].replacen('›', " ", 1),
            None,
        ));
        assert_ne!(unfocused.kind, permission, "form without a focus marker");
        let broadened = codex_observation(&codex_form(
            |lines| {
                lines[31].clear();
                lines[33].clear();
            },
            None,
        ));
        assert_ne!(
            broadened.kind, permission,
            "only the prefix-approval choice remains"
        );
        let headless = codex_observation(&codex_form(|lines| lines[22].clear(), None));
        assert_ne!(headless.kind, permission, "form without its heading");
        let reasonless = codex_observation(&codex_form(|lines| lines[26].clear(), None));
        assert_ne!(reasonless.kind, permission, "form without its reason line");
        let outside = codex_observation(&codex_form(|_| {}, Some((12, 0))));
        assert_ne!(outside.kind, permission, "cursor outside the form");
    }

    #[test]
    fn bundled_codex_form_with_choice_two_on_one_row_or_continued_detects_the_same_request() {
        let single = codex_observation(&codex_form(|_| {}, None));
        let continued = codex_observation(&codex_form(continued_choice_two, None));
        for observation in [&single, &continued] {
            assert_eq!(
                observation.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_eq!(
                observation.raw_evidence["operation"],
                "/tmp/cafleet-194/codex-cells-001/permission-probe allow"
            );
            assert_eq!(observation.raw_evidence["choices"], json!(CODEX_CHOICES));
        }
        assert!(single.fingerprint.is_some());
        assert!(
            continued.fingerprint == single.fingerprint,
            "a continued (p) row keeps the request identity"
        );
        for (fixture, operation) in [
            (
                CODEX_ALLOW_SCREEN,
                "/tmp/cafleet-194/codex-ans-allow-001/permission-probe allow",
            ),
            (
                CODEX_DENY_SCREEN,
                "/tmp/cafleet-194/codex-ans-deny-001/permission-probe deny",
            ),
        ] {
            let pre_stage = codex_observation(&replay_native_fixture(fixture, |_| {}, true, None));
            assert_eq!(
                pre_stage.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_eq!(pre_stage.raw_evidence["operation"], operation);
            assert_eq!(pre_stage.raw_evidence["choices"], json!(CODEX_CHOICES));
        }
    }

    #[test]
    fn bundled_codex_answer_plans_match_their_measured_pre_stage_screens_and_bind_focus() {
        for (decision, fixture, choice) in [
            (PermissionDecision::Allow, CODEX_ALLOW_SCREEN, "allow-once"),
            (PermissionDecision::Deny, CODEX_DENY_SCREEN, "deny-once"),
        ] {
            let plan = codex_plan(decision);
            let screen = replay_native_fixture(fixture, |_| {}, true, None);
            let matched = codex_answer(&screen, &plan).expect("pre-stage form matches its plan");
            assert_eq!(
                matched.choices(),
                CODEX_CHOICES.map(str::to_owned).as_slice()
            );
            assert_eq!(matched.plan().choice_id(), choice);
            assert!(!matched.focus().is_empty());
            assert!(Some(matched.semantic()) == codex_observation(&screen).fingerprint.as_ref());
            let moved = replay_native_fixture(
                fixture,
                |lines| {
                    lines[30] = lines[30].replacen('›', " ", 1);
                    lines[31] = lines[31].replacen("  2.", "› 2.", 1);
                },
                true,
                None,
            );
            let refocused =
                codex_answer(&moved, &plan).expect("a moved focus still identifies the form");
            assert!(refocused.semantic() == matched.semantic());
            assert_ne!(
                refocused.focus(),
                matched.focus(),
                "token binding sees the focus move"
            );
            let unfocused = replay_native_fixture(
                fixture,
                |lines| lines[30] = lines[30].replacen('›', " ", 1),
                true,
                None,
            );
            assert!(
                codex_answer(&unfocused, &plan).is_none(),
                "{decision:?} without ›"
            );
            let primary = replay_native_fixture(fixture, |_| {}, false, None);
            assert!(
                codex_answer(&primary, &plan).is_none(),
                "{decision:?} primary screen"
            );
        }
    }

    #[test]
    fn bundled_codex_idle_composer_is_a_verified_non_interaction_distinct_from_the_form() {
        let idle = codex_observation(&replay_native_fixture(
            CODEX_IDLE_SCREEN,
            |_| {},
            true,
            None,
        ));
        assert_eq!(idle.kind, ObservationKind::ComposerEmpty);
        assert_eq!(idle.match_evidence, MatchEvidence::Complete);
        assert!(idle.verified_noninteraction);
        assert_eq!(idle.rule_id, "codex-idle-composer");
        let region = idle
            .matched_region
            .expect("the idle rule locates its composer");
        assert_eq!((region.top, region.bottom), (32, 33), "{region:?}");
        let form = codex_observation(&codex_form(|_| {}, None));
        assert_eq!(form.rule_id, "codex-exec-approval");
        assert!(!form.verified_noninteraction);
    }

    #[test]
    fn bundled_codex_idle_rule_rejects_drafts_primary_copies_and_a_cursor_off_the_prefix_end() {
        let draft = codex_observation(&replay_native_fixture(
            CODEX_IDLE_SCREEN,
            |lines| lines[32] = "› hello".into(),
            true,
            Some((32, 7)),
        ));
        assert_ne!(draft.kind, ObservationKind::ComposerEmpty, "typed draft");
        assert!(!draft.verified_noninteraction);
        let primary = codex_observation(&replay_native_fixture(
            CODEX_IDLE_SCREEN,
            |_| {},
            false,
            None,
        ));
        assert!(!primary.verified_noninteraction, "primary-screen copy");
        let away = codex_observation(&replay_native_fixture(
            CODEX_IDLE_SCREEN,
            |_| {},
            true,
            Some((32, 0)),
        ));
        assert!(
            !away.verified_noninteraction,
            "cursor before the prefix end"
        );
        assert_ne!(away.kind, ObservationKind::ComposerEmpty);
    }

    #[test]
    fn permission_inspection_answer_matches_have_no_debug_or_serialized_representation() {
        trait AmbiguousIfImplemented<Marker> {
            fn check() {}
        }
        struct DebugImplemented;
        struct SerializeImplemented;
        impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
        impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
        impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
        let _ = <AnswerMatch as AmbiguousIfImplemented<_>>::check;
    }
}
impl ObservationKind {
    pub(super) fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "permission_request" => Self::Interaction(InteractionClass::PermissionRequest),
            "question" => Self::Interaction(InteractionClass::Question),
            "trust" => Self::Interaction(InteractionClass::Trust),
            "update" => Self::Interaction(InteractionClass::Update),
            "unknown_interaction" => Self::Interaction(InteractionClass::UnknownInteraction),
            "permission_denied" => Self::PermissionDenied,
            "working" => Self::Working,
            "composer_empty" => Self::ComposerEmpty,
            "composer_draft" => Self::ComposerDraft,
            "unknown" => Self::Unknown,
            _ => return Err(CafleetError::Usage("invalid profile observation".into())),
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Interaction(class) => class.as_str(),
            Self::PermissionDenied => "permission_denied",
            Self::Working => "working",
            Self::ComposerEmpty => "composer_empty",
            Self::ComposerDraft => "composer_draft",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchEvidence {
    Complete,
    Conflicting,
    Absent,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SemanticFingerprint {
    parts: Vec<SemanticPart>,
}
type SemanticPart = (String, String, String, Option<String>, Vec<String>);
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScreenRegion {
    pub top: u16,
    pub left: u16,
    pub bottom: u16,
    pub right: u16,
}
#[derive(Clone)]
pub(crate) struct ScreenObservation {
    pub kind: ObservationKind,
    pub profile_version: Option<String>,
    pub rule_id: String,
    pub match_evidence: MatchEvidence,
    pub matched_region: Option<ScreenRegion>,
    pub raw_evidence: Value,
    pub fingerprint: Option<SemanticFingerprint>,
    pub verified_noninteraction: bool,
    pub native_correlation: Option<String>,
    pub output_seq: u64,
}
pub(crate) struct Detector {
    backend: CodingAgent,
    version: Option<String>,
    profile: Option<Profile>,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AnswerMatch {
    plan: AnswerPlan,
    semantic: SemanticFingerprint,
    choices: Vec<String>,
    focus: String,
    output_seq: u64,
}
impl AnswerMatch {
    pub(crate) fn plan(&self) -> &AnswerPlan {
        &self.plan
    }
    pub(crate) fn semantic(&self) -> &SemanticFingerprint {
        &self.semantic
    }
    pub(crate) fn choices(&self) -> &[String] {
        &self.choices
    }
    pub(crate) fn focus(&self) -> &str {
        &self.focus
    }
    pub(crate) fn output_seq(&self) -> u64 {
        self.output_seq
    }
    pub(crate) fn same_request(&self, other: &Self) -> bool {
        self.plan == other.plan
            && self.semantic == other.semantic
            && self.choices == other.choices
            && self.focus == other.focus
    }
}
struct LogicalLine {
    text: String,
    start: u16,
    end: u16,
    boundaries: Vec<(u16, u16)>,
}
fn logical_lines(screen: &ScreenSnapshot) -> Result<Vec<LogicalLine>> {
    let mut lines = Vec::new();
    let mut current: Option<LogicalLine> = None;
    for row in 0..screen.rows {
        let line = current.get_or_insert_with(|| LogicalLine {
            text: String::new(),
            start: row,
            end: row,
            boundaries: vec![(row, 0)],
        });
        line.end = row;
        let end = (0..screen.cols)
            .rev()
            .find(|&col| {
                screen
                    .screen
                    .cell(row, col)
                    .is_some_and(|c| c.has_contents() || c.is_wide_continuation())
            })
            .map_or(0, |col| col + 1);
        for col in 0..end {
            let cell = screen
                .screen
                .cell(row, col)
                .ok_or_else(|| CafleetError::App("screen cell geometry mismatch".into()))?;
            if cell.is_wide_continuation() {
                continue;
            }
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            let width = if cell.is_wide() { 2 } else { 1 };
            *line
                .boundaries
                .last_mut()
                .expect("line has initial boundary") = (row, col);
            line.text.push_str(text);
            for _ in 0..text.len() {
                line.boundaries.push((row, col));
            }
            *line.boundaries.last_mut().expect("cell end boundary") = (row, col + width);
        }
        if !screen.screen.row_wrapped(row) || row + 1 == screen.rows {
            lines.push(current.take().expect("current logical line"));
        }
    }
    Ok(lines)
}
struct Matched {
    rule: Rule,
    region: ScreenRegion,
    operation: Option<String>,
    choices: Vec<String>,
    native: Option<String>,
    excerpt: String,
    focuses: BTreeMap<String, String>,
}
fn captured(
    reference: &Capture,
    values: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<String> {
    values
        .get(&reference.matcher)?
        .get(&reference.name)
        .filter(|s| !s.is_empty())
        .cloned()
}
fn match_rule(rule: &Rule, screen: &ScreenSnapshot, lines: &[LogicalLine]) -> Option<Matched> {
    if (rule.region.screen == "primary" && screen.alternate_screen)
        || (rule.region.screen == "alternate" && !screen.alternate_screen)
    {
        return None;
    }
    let mut regions = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        if rule.region.start.find_iter(&line.text).count() != 1 {
            continue;
        }
        for (end, last) in lines.iter().enumerate().skip(start) {
            if last.end - line.start + 1 > rule.region.max_rows {
                break;
            }
            let mut matches = rule.region.end.find_iter(&last.text);
            let Some(found) = matches.next() else {
                continue;
            };
            if matches.next().is_some() {
                continue;
            }
            let region = ScreenRegion {
                top: line.start,
                left: 0,
                bottom: last.end + 1,
                right: screen.cols,
            };
            // A pending-wrap cursor (column == cols) stands on its row's last cell.
            let inside = screen.cursor_row >= region.top
                && screen.cursor_row < region.bottom
                && (screen.cursor_col < region.right
                    || (region.right == screen.cols && screen.cursor_col == screen.cols));
            let qualified = match rule.region.cursor.as_str() {
                "at_end" => {
                    last.boundaries.get(found.end())
                        == Some(&(screen.cursor_row, screen.cursor_col))
                }
                "inside" => inside,
                "outside" => !inside,
                "any" => true,
                _ => unreachable!("compiled cursor enum"),
            };
            if qualified {
                regions.push((start, end, region));
            }
        }
    }
    if regions.len() != 1 {
        return None;
    }
    let (start, end, region) = regions[0];
    let selected = &lines[start..=end];
    let region_text = selected
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = BTreeMap::new();
    let mut focuses = BTreeMap::new();
    for matcher in &rule.matchers {
        let texts = if matcher.scope == "region" {
            vec![region_text.as_str()]
        } else {
            selected.iter().map(|line| line.text.as_str()).collect()
        };
        let mut captured_values = Vec::new();
        for text in texts {
            for captures in matcher.regex.captures_iter(text) {
                let named = matcher
                    .regex
                    .capture_names()
                    .flatten()
                    .filter_map(|name| {
                        captures
                            .name(name)
                            .map(|v| (name.into(), v.as_str().into()))
                    })
                    .collect::<BTreeMap<String, String>>();
                let full = captures
                    .get(0)
                    .expect("regex match has full capture")
                    .as_str();
                captured_values.push((named, full.to_owned()));
            }
        }
        if captured_values.len() != 1 {
            return None;
        }
        let (named, full) = captured_values.pop().expect("single matcher capture");
        focuses.insert(
            matcher.id.clone(),
            serde_json::to_string(&(&matcher.id, &full, &named)).expect("focus strings serialize"),
        );
        values.insert(matcher.id.clone(), named);
    }
    let operation = match &rule.operation {
        Some(reference) => Some(captured(reference, &values)?),
        None => None,
    };
    let native = match &rule.native {
        Some(reference) => Some(captured(reference, &values)?),
        None => None,
    };
    let choices = rule
        .choices
        .iter()
        .map(|choice| captured(&choice.label, &values))
        .collect::<Option<Vec<_>>>()?;
    Some(Matched {
        rule: rule.clone(),
        region,
        operation,
        choices,
        native,
        excerpt: region_text,
        focuses,
    })
}
impl Detector {
    pub(crate) fn match_answer(
        &self,
        screen: &ScreenSnapshot,
        plan: &AnswerPlan,
    ) -> Result<Option<AnswerMatch>> {
        let Some(profile) = &self.profile else {
            return Ok(None);
        };
        let capability = plan.capability();
        if capability.backend() != self.backend
            || self.version.as_deref() != Some(capability.native_version())
            || profile
                .answer_plan(
                    capability.native_version(),
                    capability.request_rule_id(),
                    capability.decision(),
                )
                .as_ref()
                != Some(plan)
        {
            return Ok(None);
        }
        let observed = self.observe(screen)?;
        if observed.kind != ObservationKind::Interaction(InteractionClass::PermissionRequest)
            || observed.match_evidence != MatchEvidence::Complete
            || observed.rule_id != capability.request_rule_id()
        {
            return Ok(None);
        }
        let rule = profile
            .rules
            .iter()
            .find(|r| r.id == observed.rule_id)
            .expect("selected compiled request rule");
        if !rule
            .choices
            .iter()
            .any(|c| c.id == plan.choice_id() && c.scope == "single_request")
        {
            return Ok(None);
        }
        let Some(matched) = match_rule(rule, screen, &logical_lines(screen)?) else {
            return Ok(None);
        };
        let focus = matched
            .focuses
            .get(plan.focus_matcher_id())
            .expect("validated focus matcher")
            .clone();
        Ok(Some(AnswerMatch {
            plan: plan.clone(),
            semantic: observed
                .fingerprint
                .expect("complete permission fingerprint"),
            choices: matched.choices,
            focus,
            output_seq: screen.screen_seq,
        }))
    }
    pub(crate) fn new(
        catalog: &ProfileCatalog,
        backend: CodingAgent,
        version: Option<&str>,
    ) -> Result<Self> {
        Ok(Self {
            backend,
            version: version.map(str::to_owned),
            profile: catalog.matching(backend, version).cloned(),
        })
    }
    pub(crate) fn observe(&self, screen: &ScreenSnapshot) -> Result<ScreenObservation> {
        let profile_version = self.profile.as_ref().map(|p| p.version.clone());
        let mut observation = ScreenObservation {
            kind: ObservationKind::Unknown,
            profile_version: profile_version.clone(),
            rule_id: "unmatched-screen".into(),
            match_evidence: MatchEvidence::Absent,
            matched_region: None,
            raw_evidence: json!({"backend":self.backend.name(),"native_version":self.version,"profile_version":profile_version,"rule_id":"unmatched-screen","screen_seq":screen.screen_seq,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}),
            fingerprint: None,
            verified_noninteraction: false,
            native_correlation: None,
            output_seq: screen.screen_seq,
        };
        let Some(profile) = &self.profile else {
            return Ok(observation);
        };
        let lines = logical_lines(screen)?;
        let mut matched = profile
            .rules
            .iter()
            .filter(|rule| {
                [profile.detection.as_ref(), profile.composer.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|cap| !cap.fixtures.is_empty() && cap.rules.contains(&rule.id))
            })
            .filter_map(|rule| match_rule(rule, screen, &lines))
            .collect::<Vec<_>>();
        let Some(priority) = matched.iter().map(|m| m.rule.priority).max() else {
            return Ok(observation);
        };
        matched.retain(|m| m.rule.priority == priority);
        matched.sort_by(|a, b| a.rule.id.cmp(&b.rule.id));
        let mut parts = matched
            .iter()
            .map(|m| {
                (
                    self.backend.name().into(),
                    profile.version.clone(),
                    m.rule.kind.name().into(),
                    m.operation.clone(),
                    m.choices.clone(),
                )
            })
            .collect::<Vec<SemanticPart>>();
        parts.sort();
        parts.dedup();
        let conflict = parts.len() > 1;
        let best = &matched[0];
        observation.kind = if conflict {
            if matched
                .iter()
                .any(|m| matches!(m.rule.kind, ObservationKind::Interaction(_)))
            {
                ObservationKind::Interaction(InteractionClass::UnknownInteraction)
            } else {
                ObservationKind::Unknown
            }
        } else {
            best.rule.kind
        };
        observation.rule_id = best.rule.id.clone();
        observation.match_evidence = if conflict {
            MatchEvidence::Conflicting
        } else {
            MatchEvidence::Complete
        };
        observation.matched_region = Some(best.region);
        observation.verified_noninteraction = !conflict && best.rule.verified;
        observation.native_correlation = if conflict { None } else { best.native.clone() };
        observation.raw_evidence["rule_id"] = json!(best.rule.id);
        observation.raw_evidence["operation"] = if conflict {
            Value::Null
        } else {
            json!(best.operation)
        };
        observation.raw_evidence["choices"] = if conflict {
            Value::Null
        } else {
            json!(best.choices)
        };
        observation.raw_evidence["native_request_id"] = json!(observation.native_correlation);
        observation.raw_evidence["excerpt"] = json!(best.excerpt);
        observation.raw_evidence["sensitive"] = json!(matched.iter().any(|m| m.rule.sensitive));
        if matches!(
            observation.kind,
            ObservationKind::Interaction(_) | ObservationKind::PermissionDenied
        ) {
            observation.fingerprint = Some(SemanticFingerprint { parts });
        }
        Ok(observation)
    }
}
