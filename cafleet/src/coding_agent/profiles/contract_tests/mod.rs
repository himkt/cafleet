use super::*;
use crate::broker::runtime::{
    Coverage, InteractionClass, InteractionCloseReason, RuntimeEventKind, evidence::EvidencePolicy,
};
use crate::runtime::runner::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

mod catalog;
mod detector;
mod episodes;

const VERSION: &str = "synthetic-0.0.1";
const PROFILE: &str = "synthetic-profile-1";
const RULE: &str = "synthetic-permission";
const FIXTURE: &str = "synthetic-permission-1";
const NOW: &str = "2026-10-26T15:35:00.000000+00:00";

fn definitions() -> Value {
    json!({
        "schema_version":1,
        "fixtures":[{"id":FIXTURE,"kind":"synthetic","backend":"codex","native_version":VERSION,"rows":24,"cols":80,"source_ref":"synthetic-permission-screen.json","sha256":"689868668a9fa97d9efca75cda3f2cc0a87fde25884a5cb36aab3163b8abd773"}],
        "profiles":[{"backend":"codex","native_versions":[VERSION],"profile_version":PROFILE,
            "rules":[{"id":RULE,"priority":100,"observation":"permission_request",
                "region":{"screen":"either","start_pattern":"^Synthetic permission request$","end_pattern":"^Selection: $","max_rows":8,"cursor":"at_end"},
                "matchers":[
                    {"id":"operation","scope":"line","pattern":"^Operation: (?P<operation>.+)$"},
                    {"id":"allow","scope":"line","pattern":"^> (?P<label>Allow once)$"},
                    {"id":"deny","scope":"line","pattern":"^  (?P<label>Deny once)$"},
                    {"id":"form","scope":"line","pattern":"^Request form: synthetic-only$"}
                ],
                "extract":{"operation":{"matcher_id":"operation","capture":"operation"},"choices":[
                    {"id":"allow-once","label":{"matcher_id":"allow","capture":"label"},"scope":"single_request"},
                    {"id":"deny-once","label":{"matcher_id":"deny","capture":"label"},"scope":"single_request"}
                ],"native_request_id":null},"verified_noninteraction":false,"sensitive":false}],
            "capabilities":{"detection":{"rule_ids":[RULE],"fixture_ids":[FIXTURE]},"composer":null,"answers":[]}}]
    })
}

fn lines(operation: &str) -> Vec<String> {
    vec![
        "Synthetic permission request".into(),
        format!("Operation: {operation}"),
        "> Allow once".into(),
        "  Deny once".into(),
        "Request form: synthetic-only".into(),
        "Selection: ".into(),
    ]
}

fn terminal(lines: &[String], cols: u16, alternate: bool) -> TerminalState {
    assert!(lines.len() <= 24);
    let mut state = TerminalState::new(TerminalSize { rows: 24, cols }).unwrap();
    if alternate {
        state.feed(b"\x1b[?1049h").unwrap();
    }
    state.feed(b"\x1b[2J").unwrap();
    for (index, line) in lines.iter().enumerate() {
        state
            .feed(format!("\x1b[{};1H{line}", 25 - lines.len() + index).as_bytes())
            .unwrap();
    }
    state
}

fn observation(definition: &Value, screen: &ScreenSnapshot) -> ScreenObservation {
    let catalog = ProfileCatalog::from_definitions(definition).unwrap();
    Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
        .unwrap()
        .observe(screen)
        .unwrap()
}

fn permission(operation: &str) -> ScreenObservation {
    observation(
        &definitions(),
        &terminal(&lines(operation), 80, false).snapshot(),
    )
}

fn simple_definition(kind: &str, verified: bool) -> Value {
    let mut value = definitions();
    let rule = &mut value["profiles"][0]["rules"][0];
    rule["id"] = json!(format!("synthetic-{kind}"));
    rule["observation"] = json!(kind);
    rule["region"]["start_pattern"] = json!("^Synthetic state$");
    rule["region"]["end_pattern"] = json!("^State end$");
    rule["matchers"] = json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
    rule["extract"] = json!({"operation":null,"choices":[],"native_request_id":null});
    rule["verified_noninteraction"] = json!(verified);
    let declaration = json!({"rule_ids":[format!("synthetic-{kind}")],"fixture_ids":[FIXTURE]});
    value["profiles"][0]["capabilities"] = if kind.starts_with("composer_") {
        json!({"detection":null,"composer":declaration,"answers":[]})
    } else {
        json!({"detection":declaration,"composer":null,"answers":[]})
    };
    value
}

fn state_observation(kind: &str, verified: bool) -> ScreenObservation {
    observation(
        &simple_definition(kind, verified),
        &terminal(&["Synthetic state".into(), "State end".into()], 80, false).snapshot(),
    )
}

fn tracker() -> EpisodeTracker {
    EpisodeTracker::new(1, 1, EvidencePolicy::metadata()).unwrap()
}

fn open(tracker: &mut EpisodeTracker, value: &ScreenObservation, origin: Instant) -> EpisodeUpdate {
    let candidate = tracker.observe(value, origin, NOW).unwrap();
    assert!(candidate.transitions.is_empty());
    assert!(matches!(
        candidate.eligibility,
        InputEligibility::Hold(GateReason::Candidate)
    ));
    let update = tracker
        .observe(value, origin + Duration::from_millis(100), NOW)
        .unwrap();
    assert_eq!(update.transitions.len(), 1);
    assert_eq!(
        update.transitions[0].kind,
        RuntimeEventKind::InteractionOpened
    );
    update
}
