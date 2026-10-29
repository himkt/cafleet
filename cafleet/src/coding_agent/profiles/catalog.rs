use super::composer::{ComposerOperation, ComposerSubmission};
use super::detector::ObservationKind;
use super::{AnswerCapability, CodingAgent, PermissionDecision};
use crate::broker::runtime::{Coverage, ProfileMatch};
use crate::error::CafleetError;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, CafleetError>;
fn invalid(message: impl Into<String>) -> CafleetError {
    CafleetError::Usage(format!("invalid native profile: {}", message.into()))
}

#[cfg(test)]
pub(super) mod inspection_tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn inspection_fixture_definition() -> Value {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../runtime/runner_tests/fixtures/synthetic-profile.json"
        ))
        .unwrap();
        value["profiles"][0]["rules"][0]["matchers"][3]["pattern"] =
            json!("^Request form: (?P<focus>synthetic-only|synthetic-other)$");
        let mut alternate = value["profiles"][0]["rules"][0].clone();
        alternate["id"] = json!("inspection-alternate-request");
        alternate["matchers"][3]["pattern"] = json!("^Request form: (?P<focus>synthetic-alt)$");
        value["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(alternate);
        value["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
            json!(["synthetic-permission", "inspection-alternate-request"]);
        value["profiles"][0]["capabilities"]["answers"] = json!([
            answer(
                "allow",
                "synthetic-permission",
                "inspection-allow-plan",
                "allow-once",
                "79"
            ),
            answer(
                "deny",
                "synthetic-permission",
                "inspection-deny-plan",
                "deny-once",
                "6e"
            ),
            answer(
                "allow",
                "inspection-alternate-request",
                "inspection-alternate-allow",
                "allow-once",
                "61"
            )
        ]);
        value
    }

    fn answer(decision: &str, request: &str, id: &str, choice: &str, byte: &str) -> Value {
        json!({
            "decision":decision,"request_rule_id":request,"answer_rule_id":id,
            "choice_id":choice,"focus_matcher_id":"form","fixture_ids":["synthetic-permission-1"],
            "stages":[
                {"bytes_hex":byte,"required_rule_id":request,"required_choice_id":choice},
                {"bytes_hex":"0d","required_rule_id":request,"required_choice_id":choice}
            ]
        })
    }

    pub(crate) fn inspection_fixture_form(operation: &str, focus: &str) -> Vec<u8> {
        format!("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: {focus}\r\nSelection: ").into_bytes()
    }

    pub(crate) const GATED_RULE: &str = "synthetic-gated";
    pub(crate) const GATED_REASON_RULE: &str = "synthetic-reason";
    pub(crate) const GATED_REASON_FIXTURE: &str = "synthetic-reason-native-1";
    pub(crate) const FOCUS_SGR: &str = "\x1b[48;2;250;178;131m";

    pub(crate) fn gated_fixture_definition() -> Value {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../runtime/runner_tests/fixtures/synthetic-profile.json"
        ))
        .unwrap();
        let mut native = value["fixtures"][0].clone();
        native["id"] = json!(GATED_REASON_FIXTURE);
        native["kind"] = json!("native");
        value["fixtures"].as_array_mut().unwrap().push(native);
        value["profiles"][0]["rules"] = json!([
            {
                "id":GATED_RULE,"priority":100,"observation":"permission_request",
                "region":{"screen":"either","start_pattern":"^Synthetic gated request$","end_pattern":"^Selection: $","max_rows":8,"cursor":"any"},
                "matchers":[
                    {"id":"heading","scope":"line","pattern":"^Synthetic gated request$"},
                    {"id":"operation","scope":"line","pattern":"^Operation: (?P<operation>.+)$"},
                    {"id":"focus","scope":"line","pattern":"^  Allow once   (?:Always allow   )?Deny once$","focus_style":"background"},
                    {"id":"allow","scope":"line","pattern":"^  (?P<label>Allow once)   "},
                    {"id":"deny","scope":"line","pattern":"   (?P<label>Deny once)$"}
                ],
                "extract":{"operation":{"matcher_id":"operation","capture":"operation"},"choices":[
                    {"id":"allow-once","label":{"matcher_id":"allow","capture":"label"},"scope":"single_request"},
                    {"id":"deny-once","label":{"matcher_id":"deny","capture":"label"},"scope":"single_request"}
                ],"native_request_id":null},
                "verified_noninteraction":false,"sensitive":false
            },
            {
                "id":GATED_REASON_RULE,"priority":100,"observation":"permission_continuation",
                "region":{"screen":"either","start_pattern":"^Synthetic reject reason$","end_pattern":"^Confirm: enter$","max_rows":4,"cursor":"any"},
                "matchers":[
                    {"id":"heading","scope":"line","pattern":"^Synthetic reject reason$"},
                    {"id":"empty","scope":"line","pattern":"^Reason: \\(empty\\)$"}
                ],
                "extract":{"operation":null,"choices":[],"native_request_id":null},
                "verified_noninteraction":false,"sensitive":false,
                "continues_rule_id":GATED_RULE,"continues_choice_id":"deny-once",
                "fixture_ids":[GATED_REASON_FIXTURE]
            }
        ]);
        value["profiles"][0]["capabilities"]["detection"]["rule_ids"] = json!([GATED_RULE]);
        value["profiles"][0]["capabilities"]["answers"] = json!([
            {
                "decision":"allow","request_rule_id":GATED_RULE,"answer_rule_id":"synthetic-gated-allow",
                "choice_id":"allow-once","focus_matcher_id":"focus","fixture_ids":["synthetic-permission-1"],
                "stages":[
                    {"bytes_hex":"0d","required_rule_id":GATED_RULE,"required_choice_id":"allow-once","required_focus_choice_id":"allow-once"}
                ]
            },
            {
                "decision":"deny","request_rule_id":GATED_RULE,"answer_rule_id":"synthetic-gated-deny",
                "choice_id":"deny-once","focus_matcher_id":"focus","fixture_ids":["synthetic-permission-1"],
                "stages":[
                    {"bytes_hex":"68","required_rule_id":GATED_RULE,"required_choice_id":"deny-once","required_focus_choice_id":"allow-once"},
                    {"bytes_hex":"0d","required_rule_id":GATED_RULE,"required_choice_id":"deny-once","required_focus_choice_id":"deny-once"},
                    {"bytes_hex":"0d","required_rule_id":GATED_RULE,"required_choice_id":"deny-once","required_screen_rule_id":GATED_REASON_RULE}
                ]
            }
        ]);
        value
    }

    /// `focused` names the labels painted with `focus_sgr`; the rest of the choice row keeps
    /// `row_sgr`, which is the row background the detector compares against.
    pub(crate) fn gated_fixture_form_styled(
        operation: &str,
        always: bool,
        focused: &[&str],
        row_sgr: &str,
        focus_sgr: &str,
    ) -> Vec<u8> {
        let labels: &[&str] = if always {
            &["Allow once", "Always allow", "Deny once"]
        } else {
            &["Allow once", "Deny once"]
        };
        let mut row = format!("{row_sgr}  ");
        for (index, label) in labels.iter().enumerate() {
            if index > 0 {
                row.push_str("   ");
            }
            if focused.contains(label) {
                row.push_str(&format!("{focus_sgr}{label}\x1b[0m{row_sgr}"));
            } else {
                row.push_str(label);
            }
        }
        row.push_str("\x1b[0m");
        format!("\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: {operation}\r\n{row}\r\nSelection: ").into_bytes()
    }

    pub(crate) fn gated_fixture_form(operation: &str, focused: &str) -> Vec<u8> {
        let focused: &[&str] = match focused {
            "allow" => &["Allow once"],
            "deny" => &["Deny once"],
            "always" => &["Always allow"],
            "none" => &[],
            _ => panic!("known synthetic focus"),
        };
        gated_fixture_form_styled(operation, true, focused, "", FOCUS_SGR)
    }

    pub(crate) fn gated_reason_dialog(reason: Option<&str>) -> Vec<u8> {
        let reason = reason.unwrap_or("(empty)");
        format!(
            "\x1b[0m\x1b[2J\x1b[19;1HSynthetic reject reason\r\nReason: {reason}\r\nConfirm: enter"
        )
        .into_bytes()
    }

    #[test]
    fn permission_inspection_catalog_retains_exact_executable_plans_and_provenance() {
        let catalog = ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
        for (decision, choice, rule, bytes) in [
            (
                PermissionDecision::Allow,
                "allow-once",
                "inspection-allow-plan",
                b'y',
            ),
            (
                PermissionDecision::Deny,
                "deny-once",
                "inspection-deny-plan",
                b'n',
            ),
        ] {
            let plan = catalog
                .answer_plan(
                    CodingAgent::Codex,
                    "synthetic-0.0.1",
                    "synthetic-permission",
                    decision,
                )
                .unwrap()
                .unwrap();
            let capability = plan.capability();
            assert_eq!(capability.backend(), CodingAgent::Codex);
            assert_eq!(capability.native_version(), "synthetic-0.0.1");
            assert_eq!(capability.profile_version(), "synthetic-profile-1");
            assert_eq!(capability.request_rule_id(), "synthetic-permission");
            assert_eq!(capability.answer_rule_id(), rule);
            assert_eq!(capability.decision(), decision);
            assert_eq!(
                capability.fixture_ids(),
                &["synthetic-permission-1".to_owned()]
            );
            assert_eq!(plan.choice_id(), choice);
            assert_eq!(plan.focus_matcher_id(), "form");
            assert_eq!(plan.stages().len(), 2);
            assert_eq!(plan.stages()[0].bytes(), &[bytes]);
            assert_eq!(plan.stages()[1].bytes(), b"\r");
            for stage in plan.stages() {
                assert_eq!(stage.required_rule_id(), "synthetic-permission");
                assert_eq!(stage.required_choice_id(), choice);
            }
        }
        assert!(
            catalog
                .answer_plan(
                    CodingAgent::Claude,
                    "synthetic-0.0.1",
                    "synthetic-permission",
                    PermissionDecision::Allow
                )
                .unwrap()
                .is_none()
        );
        for (version, rule, decision) in [
            ("synthetic-0.0.1", "unknown", PermissionDecision::Allow),
            (
                "synthetic-0.0.1",
                "inspection-alternate-request",
                PermissionDecision::Deny,
            ),
        ] {
            assert!(
                catalog
                    .answer_plan(CodingAgent::Codex, version, rule, decision)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
                    "synthetic-0.0.1",
                    "synthetic-permission",
                    PermissionDecision::Allow
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn permission_inspection_catalog_bounds_each_stage_and_complete_plan() {
        let mut value = inspection_fixture_definition();
        let mut stage = value["profiles"][0]["capabilities"]["answers"][0]["stages"][0].clone();
        stage["bytes_hex"] = json!("79".repeat(256));
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] =
            json!(vec![stage.clone(); 16]);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        let plan = catalog
            .answer_plan(
                CodingAgent::Codex,
                "synthetic-0.0.1",
                "synthetic-permission",
                PermissionDecision::Allow,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            plan.stages().iter().map(|s| s.bytes().len()).sum::<usize>(),
            4096
        );
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] = json!(vec![stage; 17]);
        assert!(ProfileCatalog::from_definitions(&value).is_err());
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] = json!([{
            "bytes_hex":"79".repeat(257),"required_rule_id":"synthetic-permission","required_choice_id":"allow-once"
        }]);
        assert!(ProfileCatalog::from_definitions(&value).is_err());
    }

    #[test]
    fn permission_inspection_executable_plans_have_no_debug_or_serialized_representation() {
        trait AmbiguousIfImplemented<Marker> {
            fn check() {}
        }
        struct DebugImplemented;
        struct SerializeImplemented;
        impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
        impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
        impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
        let _ = <crate::coding_agent::AnswerPlan as AmbiguousIfImplemented<_>>::check;
        let _ = <crate::coding_agent::AnswerStage as AmbiguousIfImplemented<_>>::check;
    }
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(message)) }
}
pub(super) fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    let map = value
        .as_object()
        .ok_or_else(|| invalid("object required"))?;
    require(
        map.len() == keys.len() && keys.iter().all(|key| map.contains_key(*key)),
        "exact required object fields expected",
    )?;
    Ok(map)
}
fn text(value: &Value) -> Result<String> {
    let text = value.as_str().ok_or_else(|| invalid("text required"))?;
    require(!text.trim().is_empty(), "nonblank text required")?;
    Ok(text.into())
}
fn array(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().ok_or_else(|| invalid("array required"))
}
fn texts(value: &Value) -> Result<Vec<String>> {
    let values = array(value)?.iter().map(text).collect::<Result<Vec<_>>>()?;
    require(
        values.iter().collect::<BTreeSet<_>>().len() == values.len(),
        "duplicate string identity",
    )?;
    Ok(values)
}
fn boolean(value: &Value) -> Result<bool> {
    value.as_bool().ok_or_else(|| invalid("boolean required"))
}
fn backend(value: &Value) -> Result<CodingAgent> {
    crate::coding_agent::coding_agent(&text(value)?).ok_or_else(|| invalid("unknown backend"))
}
fn positive(value: &Value) -> Result<u16> {
    value
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("positive viewport bound required"))
}
fn one_of(value: &Value, choices: &[&str]) -> Result<String> {
    let value = text(value)?;
    require(choices.contains(&value.as_str()), "invalid profile enum")?;
    Ok(value)
}
fn expression(value: &Value) -> Result<Regex> {
    Regex::new(&text(value)?).map_err(|cause| invalid(format!("regex: {cause}")))
}
fn unique(ids: &mut BTreeSet<String>, id: &str) -> Result<()> {
    require(ids.insert(id.into()), "duplicate profile identity")
}
fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Fixture {
    pub id: String,
    pub kind: String,
    pub backend: CodingAgent,
    pub version: String,
}
#[derive(Clone)]
pub(super) struct Capture {
    pub matcher: String,
    pub name: String,
    pub row_margin: Option<String>,
}
#[derive(Clone)]
pub(super) struct Choice {
    pub id: String,
    pub label: Capture,
    pub scope: String,
}
#[derive(Clone)]
pub(super) struct Matcher {
    pub id: String,
    pub scope: String,
    pub regex: Regex,
    pub background_focus: bool,
}
#[derive(Clone)]
pub(super) struct Continuation {
    pub rule_id: String,
    pub choice_id: String,
    pub fixtures: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Region {
    pub screen: String,
    pub start: Regex,
    pub end: Regex,
    pub max_rows: u16,
    pub cursor: String,
}
#[derive(Clone)]
pub(super) struct Frame {
    pub require: Vec<Regex>,
    pub reject: Vec<Regex>,
}
#[derive(Clone)]
pub(super) struct Rule {
    pub id: String,
    pub priority: i64,
    pub kind: ObservationKind,
    pub region: Region,
    pub matchers: Vec<Matcher>,
    pub operation: Option<Capture>,
    pub choices: Vec<Choice>,
    pub native: Option<Capture>,
    pub verified: bool,
    pub sensitive: bool,
    pub continuation: Option<Continuation>,
    pub frame: Option<Frame>,
}
#[derive(Clone)]
pub(super) struct Capability {
    pub rules: Vec<String>,
    pub fixtures: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Answer {
    pub decision: PermissionDecision,
    pub request: String,
    pub id: String,
    pub fixtures: Vec<String>,
    pub choice_id: String,
    pub focus_matcher_id: String,
    pub stages: Vec<AnswerStage>,
    pub provenance: Vec<Fixture>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AnswerStage {
    bytes: Vec<u8>,
    required_rule_id: String,
    required_choice_id: String,
    gate: Option<StageGate>,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum StageGate {
    Focus(String),
    Screen(String),
}
impl AnswerStage {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn required_rule_id(&self) -> &str {
        &self.required_rule_id
    }
    pub(crate) fn required_choice_id(&self) -> &str {
        &self.required_choice_id
    }
    pub(crate) fn gate(&self) -> Option<&StageGate> {
        self.gate.as_ref()
    }
    pub(crate) fn required_focus_choice_id(&self) -> Option<&str> {
        match &self.gate {
            Some(StageGate::Focus(choice)) => Some(choice),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn required_screen_rule_id(&self) -> Option<&str> {
        match &self.gate {
            Some(StageGate::Screen(rule)) => Some(rule),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AnswerPlan {
    capability: AnswerCapability,
    choice_id: String,
    focus_matcher_id: String,
    stages: Vec<AnswerStage>,
    provenance: Vec<Fixture>,
}
impl AnswerPlan {
    pub(crate) fn capability(&self) -> &AnswerCapability {
        &self.capability
    }
    pub(crate) fn choice_id(&self) -> &str {
        &self.choice_id
    }
    pub(crate) fn focus_matcher_id(&self) -> &str {
        &self.focus_matcher_id
    }
    pub(crate) fn stages(&self) -> &[AnswerStage] {
        &self.stages
    }
}
#[derive(Clone)]
pub(super) struct Profile {
    pub backend: CodingAgent,
    pub versions: Vec<String>,
    pub version: String,
    pub rules: Vec<Rule>,
    pub detection: Option<Capability>,
    pub composer: Option<Capability>,
    pub annotations: Option<Capability>,
    pub submissions: [Option<ComposerSubmission>; 2],
    pub answers: Vec<Answer>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum VersionComponent {
    Number(u64),
    Text(String),
}
#[derive(Debug, Clone)]
struct DottedVersion(Vec<VersionComponent>);
impl PartialEq for DottedVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for DottedVersion {}
impl DottedVersion {
    fn parse(version: &str) -> Self {
        Self(
            version
                .split('.')
                .map(|part| {
                    part.parse().map_or_else(
                        |_| VersionComponent::Text(part.into()),
                        VersionComponent::Number,
                    )
                })
                .collect(),
        )
    }
}
impl Ord for DottedVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let missing = VersionComponent::Number(0);
        (0..self.0.len().max(other.0.len()))
            .map(|index| {
                let left = self.0.get(index).unwrap_or(&missing);
                let right = other.0.get(index).unwrap_or(&missing);
                left.cmp(right)
            })
            .find(|ordering| ordering.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for DottedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Profile {
    fn newest_version(&self) -> DottedVersion {
        self.versions
            .iter()
            .map(|version| DottedVersion::parse(version))
            .max()
            .expect("catalog validation requires every profile to declare a native version")
    }
    pub(super) fn answer_plan(
        &self,
        version: &str,
        request: &str,
        decision: PermissionDecision,
    ) -> Option<AnswerPlan> {
        let answer = self
            .answers
            .iter()
            .find(|a| a.request == request && a.decision == decision)?;
        Some(AnswerPlan {
            capability: AnswerCapability {
                backend: self.backend,
                native_version: version.into(),
                profile_version: self.version.clone(),
                request_rule_id: request.into(),
                answer_rule_id: answer.id.clone(),
                decision,
                fixture_ids: answer.fixtures.clone(),
            },
            choice_id: answer.choice_id.clone(),
            focus_matcher_id: answer.focus_matcher_id.clone(),
            stages: answer.stages.clone(),
            provenance: answer.provenance.clone(),
        })
    }
}
#[derive(Clone)]
pub struct ProfileCatalog {
    profiles: Vec<Profile>,
    #[cfg(test)]
    capture_origin: &'static str,
}
pub struct NativeCapabilities {
    pub backend: CodingAgent,
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub profile_match: Option<ProfileMatch>,
    pub detection: Coverage,
    pub composer: Coverage,
    pub prompt_submission: Coverage,
    pub native_exec_submission: Coverage,
    pub answer_decisions: Vec<PermissionDecision>,
    pub capability_reason: Option<String>,
}
fn capture(value: &Value, matchers: &[Matcher]) -> Result<Capture> {
    let map = object(value, &["matcher_id", "capture"])?;
    let matcher = text(&map["matcher_id"])?;
    let name = text(&map["capture"])?;
    let rule = matchers
        .iter()
        .find(|item| item.id == matcher)
        .ok_or_else(|| invalid("unknown capture matcher"))?;
    require(
        rule.regex
            .capture_names()
            .flatten()
            .any(|candidate| candidate == name),
        "unknown named capture",
    )?;
    Ok(Capture {
        matcher,
        name,
        row_margin: None,
    })
}
fn optional_capture(value: &Value, matchers: &[Matcher]) -> Result<Option<Capture>> {
    if value.is_null() {
        Ok(None)
    } else {
        capture(value, matchers).map(Some)
    }
}
fn operation_capture(value: &Value, matchers: &[Matcher]) -> Result<Option<Capture>> {
    let Some(row_margin) = value.get("row_margin") else {
        return optional_capture(value, matchers);
    };
    let row_margin = row_margin
        .as_str()
        .filter(|margin| !margin.is_empty() && !margin.contains('\n'))
        .ok_or_else(|| invalid("nonempty single-row margin required"))?
        .to_owned();
    let mut plain = object(value, &["matcher_id", "capture", "row_margin"])?.clone();
    plain.remove("row_margin");
    Ok(Some(Capture {
        row_margin: Some(row_margin),
        ..capture(&Value::Object(plain), matchers)?
    }))
}
const RULE_FIELDS: [&str; 8] = [
    "id",
    "priority",
    "observation",
    "region",
    "matchers",
    "extract",
    "verified_noninteraction",
    "sensitive",
];
const CONTINUATION_FIELDS: [&str; 3] = ["continues_rule_id", "continues_choice_id", "fixture_ids"];
fn compile_rule(value: &Value) -> Result<Rule> {
    let kind = ObservationKind::parse(&text(
        value
            .get("observation")
            .ok_or_else(|| invalid("rule observation required"))?,
    )?)?;
    let continues = kind == ObservationKind::PermissionContinuation;
    let annotation = kind.is_annotation();
    let fields = if continues {
        RULE_FIELDS
            .iter()
            .chain(&CONTINUATION_FIELDS)
            .copied()
            .collect()
    } else if annotation {
        RULE_FIELDS.iter().chain(&["frame"]).copied().collect()
    } else {
        RULE_FIELDS.to_vec()
    };
    let m = object(value, &fields)?;
    let id = text(&m["id"])?;
    let priority = m["priority"]
        .as_i64()
        .ok_or_else(|| invalid("integer priority required"))?;
    let verified = boolean(&m["verified_noninteraction"])?;
    require(
        !verified
            || !matches!(
                kind,
                ObservationKind::Interaction(_)
                    | ObservationKind::Unknown
                    | ObservationKind::PermissionContinuation
            ),
        "interaction/unknown/continuation cannot verify noninteraction",
    )?;
    let continuation = if continues {
        Some(Continuation {
            rule_id: text(&m["continues_rule_id"])?,
            choice_id: text(&m["continues_choice_id"])?,
            fixtures: texts(&m["fixture_ids"])?,
        })
    } else {
        None
    };
    let r = object(
        &m["region"],
        &[
            "screen",
            "start_pattern",
            "end_pattern",
            "max_rows",
            "cursor",
        ],
    )?;
    let region = Region {
        screen: one_of(&r["screen"], &["primary", "alternate", "either"])?,
        start: expression(&r["start_pattern"])?,
        end: expression(&r["end_pattern"])?,
        max_rows: positive(&r["max_rows"])?,
        cursor: one_of(&r["cursor"], &["at_end", "inside", "outside", "any"])?,
    };
    let mut ids = BTreeSet::new();
    let mut matchers = Vec::new();
    for v in array(&m["matchers"])? {
        let background_focus = v.get("focus_style").is_some();
        let m = if background_focus {
            object(v, &["id", "scope", "pattern", "focus_style"])?
        } else {
            object(v, &["id", "scope", "pattern"])?
        };
        if background_focus {
            one_of(&m["focus_style"], &["background"])?;
        }
        let id = text(&m["id"])?;
        unique(&mut ids, &id)?;
        matchers.push(Matcher {
            id,
            scope: one_of(&m["scope"], &["line", "region"])?,
            regex: expression(&m["pattern"])?,
            background_focus,
        });
    }
    require(!matchers.is_empty(), "required matchers cannot be empty")?;
    let ex = object(
        &m["extract"],
        &["operation", "choices", "native_request_id"],
    )?;
    let mut choices = Vec::new();
    let mut ids = BTreeSet::new();
    for v in array(&ex["choices"])? {
        let c = object(v, &["id", "label", "scope"])?;
        let id = text(&c["id"])?;
        unique(&mut ids, &id)?;
        choices.push(Choice {
            id,
            label: capture(&c["label"], &matchers)?,
            scope: one_of(&c["scope"], &["single_request", "session", "persistent"])?,
        });
    }
    require(
        !continues || (ex["operation"].is_null() && choices.is_empty()),
        "continuation declares no operation or choices",
    )?;
    let operation = operation_capture(&ex["operation"], &matchers)?;
    let native = optional_capture(&ex["native_request_id"], &matchers)?;
    let frame = if annotation {
        require(
            choices.is_empty() && native.is_none() && !verified,
            "annotation rules declare no choices, native request id or verified noninteraction",
        )?;
        require(
            operation.is_some() == (kind == ObservationKind::AutomaticRefusal),
            "only automatic refusal annotations capture an operation",
        )?;
        Some(compile_frame(&m["frame"])?)
    } else {
        None
    };
    Ok(Rule {
        id,
        priority,
        kind,
        region,
        operation,
        native,
        choices,
        matchers,
        verified,
        sensitive: boolean(&m["sensitive"])?,
        continuation,
        frame,
    })
}
fn compile_frame(value: &Value) -> Result<Frame> {
    let m = object(value, &["require_patterns", "reject_patterns"])?;
    let patterns = |value: &Value| {
        array(value)?
            .iter()
            .map(expression)
            .collect::<Result<Vec<_>>>()
    };
    let require_patterns = patterns(&m["require_patterns"])?;
    require(
        !require_patterns.is_empty(),
        "annotation frame requires a live-screen pattern",
    )?;
    Ok(Frame {
        require: require_patterns,
        reject: patterns(&m["reject_patterns"])?,
    })
}
const STAGE_FIELDS: [&str; 3] = ["bytes_hex", "required_rule_id", "required_choice_id"];
fn stage_fields(value: &Value) -> Result<(&Map<String, Value>, Option<StageGate>)> {
    let focus = value.get("required_focus_choice_id").is_some();
    let screen = value.get("required_screen_rule_id").is_some();
    require(!(focus && screen), "a stage declares at most one gate")?;
    let gate_field = if focus {
        Some("required_focus_choice_id")
    } else if screen {
        Some("required_screen_rule_id")
    } else {
        None
    };
    let fields = STAGE_FIELDS
        .iter()
        .copied()
        .chain(gate_field)
        .collect::<Vec<_>>();
    let map = object(value, &fields)?;
    let gate = match gate_field {
        Some(field) => {
            let id = text(&map[field])?;
            Some(if focus {
                StageGate::Focus(id)
            } else {
                StageGate::Screen(id)
            })
        }
        None => None,
    };
    Ok((map, gate))
}
fn validate_gates(
    stages: &[AnswerStage],
    request: &Rule,
    rules: &[Rule],
    choice_id: &str,
    focus_matcher: &str,
) -> Result<()> {
    if stages.iter().all(|stage| stage.gate.is_none()) {
        return Ok(());
    }
    require(
        stages.iter().all(|stage| stage.gate.is_some()),
        "a gated plan gates every stage",
    )?;
    let continuation = |id: &str| {
        rules
            .iter()
            .find(|rule| rule.id == id)
            .and_then(|rule| rule.continuation.as_ref())
            .ok_or_else(|| invalid("screen gate names no continuation rule"))
    };
    let mut focus_gated = false;
    for (index, stage) in stages.iter().enumerate() {
        match stage.gate.as_ref().expect("every stage is gated") {
            StageGate::Focus(choice) => {
                focus_gated = true;
                require(
                    request.choices.iter().any(|c| &c.id == choice),
                    "focus gate names no choice of the request rule",
                )?;
            }
            StageGate::Screen(rule) => {
                let continuation = continuation(rule)?;
                require(
                    continuation.rule_id == request.id,
                    "screen gate continues another request rule",
                )?;
                let previous = index
                    .checked_sub(1)
                    .and_then(|previous| stages[previous].required_focus_choice_id());
                require(
                    previous == Some(continuation.choice_id.as_str()),
                    "screen gate follows the stage selecting its continued choice",
                )?;
            }
        }
    }
    let final_choice = match stages.last().and_then(AnswerStage::gate) {
        Some(StageGate::Focus(choice)) => choice.as_str(),
        Some(StageGate::Screen(rule)) => continuation(rule)?.choice_id.as_str(),
        None => unreachable!("validated gated final stage"),
    };
    require(
        final_choice == choice_id,
        "a gated plan ends on its own choice",
    )?;
    if focus_gated {
        let matcher = request
            .matchers
            .iter()
            .find(|matcher| matcher.id == focus_matcher)
            .expect("validated focus matcher");
        require(
            matcher.background_focus,
            "a focus-gated plan needs a focus_style matcher",
        )?;
        for choice in &request.choices {
            let label = request
                .matchers
                .iter()
                .find(|matcher| matcher.id == choice.label.matcher)
                .expect("validated choice label matcher");
            require(
                label.scope == "line",
                "focus-gated choice labels are line-scoped",
            )?;
        }
    }
    Ok(())
}
fn validate_continuation(
    rule: &Rule,
    rules: &[Rule],
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<()> {
    let Some(continuation) = &rule.continuation else {
        return Ok(());
    };
    let continued = rules
        .iter()
        .find(|candidate| candidate.id == continuation.rule_id)
        .ok_or_else(|| invalid("continuation names an unknown rule"))?;
    require(
        continued.kind
            == ObservationKind::Interaction(
                crate::broker::runtime::InteractionClass::PermissionRequest,
            ),
        "continuation continues a permission request rule",
    )?;
    require(
        continued
            .choices
            .iter()
            .any(|choice| choice.id == continuation.choice_id),
        "continuation names an unknown continued choice",
    )?;
    provenance(&continuation.fixtures, backend, versions, fixtures)?;
    require(
        continuation.fixtures.iter().all(|id| {
            fixtures
                .iter()
                .any(|fixture| &fixture.id == id && fixture.kind == "native")
        }),
        "continuation cites native fixtures",
    )
}
fn provenance(
    ids: &[String],
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<()> {
    require(!ids.is_empty(), "capability provenance required")?;
    let mut covered = BTreeSet::new();
    for id in ids {
        let f = fixtures
            .iter()
            .find(|f| &f.id == id)
            .ok_or_else(|| invalid("unknown fixture"))?;
        require(
            f.backend == backend && versions.contains(&f.version),
            "fixture backend/version mismatch",
        )?;
        covered.insert(f.version.as_str());
    }
    require(
        versions.iter().all(|v| covered.contains(v.as_str())),
        "every exact version needs compatible provenance",
    )
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Detection,
    Composer,
    Annotations,
}
impl Group {
    fn of(kind: ObservationKind) -> Self {
        if kind.is_annotation() {
            Self::Annotations
        } else if matches!(
            kind,
            ObservationKind::ComposerEmpty | ObservationKind::ComposerDraft
        ) {
            Self::Composer
        } else {
            Self::Detection
        }
    }
}
fn capability(
    value: &Value,
    group: Group,
    rules: &[Rule],
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<Option<Capability>> {
    if value.is_null() {
        return Ok(None);
    }
    let fields: &[&str] = if group == Group::Composer && value.get("submission").is_some() {
        &["rule_ids", "fixture_ids", "submission"]
    } else {
        &["rule_ids", "fixture_ids"]
    };
    let m = object(value, fields)?;
    let ids = texts(&m["rule_ids"])?;
    require(!ids.is_empty(), "capability rules required")?;
    for id in &ids {
        let rule = rules
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| invalid("unknown capability rule"))?;
        require(
            Group::of(rule.kind) == group,
            "capability rule component mismatch",
        )?;
    }
    let fixture_ids = texts(&m["fixture_ids"])?;
    provenance(&fixture_ids, backend, versions, fixtures)?;
    Ok(Some(Capability {
        rules: ids,
        fixtures: fixture_ids,
    }))
}

fn decoded_hex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .expect("validated lowercase hexadecimal byte")
        })
        .collect()
}

fn composer_submission(
    value: &Value,
    operation: ComposerOperation,
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<Option<ComposerSubmission>> {
    if value.is_null() {
        return Ok(None);
    }
    let m = object(value, &["prefix_hex", "submit_hex", "fixture_ids"])?;
    let prefix = m["prefix_hex"]
        .as_str()
        .ok_or_else(|| invalid("composer prefix string required"))?;
    require(
        prefix == operation.prefix_hex(),
        "composer prefix must match its operation",
    )?;
    let submit = m["submit_hex"]
        .as_str()
        .ok_or_else(|| invalid("composer submit string required"))?;
    require(
        (2..=64).contains(&submit.len()) && submit.len().is_multiple_of(2) && lower_hex(submit),
        "composer submit requires 1–32 lowercase hexadecimal bytes",
    )?;
    let ids = texts(&m["fixture_ids"])?;
    provenance(&ids, backend, versions, fixtures)?;
    let fixtures = ids
        .iter()
        .map(|id| {
            fixtures
                .iter()
                .find(|fixture| &fixture.id == id)
                .expect("validated composer fixture reference")
                .clone()
        })
        .collect();
    Ok(Some(ComposerSubmission {
        prefix: decoded_hex(prefix),
        submit: decoded_hex(submit),
        fixtures,
    }))
}

fn composer_submissions(
    value: &Value,
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<[Option<ComposerSubmission>; 2]> {
    let Some(submission) = value.get("submission") else {
        return Ok([None, None]);
    };
    let m = object(submission, &["prompt", "exec"])?;
    Ok([
        composer_submission(
            &m["prompt"],
            ComposerOperation::Prompt,
            backend,
            versions,
            fixtures,
        )?,
        composer_submission(
            &m["exec"],
            ComposerOperation::Exec,
            backend,
            versions,
            fixtures,
        )?,
    ])
}

const BUNDLED_DEFINITIONS: &str = include_str!("bundled.json");
const BUNDLED_FIXTURES: &[(&str, &[u8])] = &[
    (
        "native/claude-2.1.302-default-permission-1.json",
        include_bytes!("native/claude-2.1.302-default-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-allow-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-allow-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-deny-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-deny-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-idle-composer-1.json",
        include_bytes!("native/claude-2.1.302-idle-composer-1.json"),
    ),
    (
        "native/claude-2.1.302-wrapped-permission-1.json",
        include_bytes!("native/claude-2.1.302-wrapped-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-wrapped-allow-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-wrapped-allow-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-wrapped-deny-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-wrapped-deny-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-boxed-permission-1.json",
        include_bytes!("native/claude-2.1.302-boxed-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-boxed-single-row-permission-1.json",
        include_bytes!("native/claude-2.1.302-boxed-single-row-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-approved-done-1.json",
        include_bytes!("native/claude-2.1.302-auto-approved-done-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-approved-running-1.json",
        include_bytes!("native/claude-2.1.302-auto-approved-running-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-ask-allow-continuation-1.json",
        include_bytes!("native/claude-2.1.302-auto-ask-allow-continuation-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-ask-deny-continuation-1.json",
        include_bytes!("native/claude-2.1.302-auto-ask-deny-continuation-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-ask-hiding-refusal-notice-1.json",
        include_bytes!("native/claude-2.1.302-auto-ask-hiding-refusal-notice-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1.json",
        include_bytes!("native/claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-ask-permission-1.json",
        include_bytes!("native/claude-2.1.302-auto-ask-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-fresh-first-launch-notice-1.json",
        include_bytes!("native/claude-2.1.302-auto-fresh-first-launch-notice-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-fresh-onboarding-theme-1.json",
        include_bytes!("native/claude-2.1.302-auto-fresh-onboarding-theme-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-idle-composer-1.json",
        include_bytes!("native/claude-2.1.302-auto-idle-composer-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-model-switch-question-1.json",
        include_bytes!("native/claude-2.1.302-auto-model-switch-question-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-quote-false-positive-1.json",
        include_bytes!("native/claude-2.1.302-auto-quote-false-positive-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-refusal-notice-expired-1.json",
        include_bytes!("native/claude-2.1.302-auto-refusal-notice-expired-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-refusal-notice-idle-1.json",
        include_bytes!("native/claude-2.1.302-auto-refusal-notice-idle-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-refusal-notice-working-1.json",
        include_bytes!("native/claude-2.1.302-auto-refusal-notice-working-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-transcript-viewer-1.json",
        include_bytes!("native/claude-2.1.302-auto-transcript-viewer-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-two-refusals-notice-1.json",
        include_bytes!("native/claude-2.1.302-auto-two-refusals-notice-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-unavailable-manual-footer-1.json",
        include_bytes!("native/claude-2.1.302-auto-unavailable-manual-footer-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-unavailable-notice-1.json",
        include_bytes!("native/claude-2.1.302-auto-unavailable-notice-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-quote-false-positive-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-quote-false-positive-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-refusal-above-form-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-refusal-above-form-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-refusal-hidden-by-form-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-refusal-hidden-by-form-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-refusal-restored-idle-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-refusal-restored-idle-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-refusal-rewrapped-form-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-refusal-rewrapped-form-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-refusal-rewrapped-idle-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-refusal-rewrapped-idle-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-transcript-viewer-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-transcript-viewer-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-verbose-two-refusals-1.json",
        include_bytes!("native/claude-2.1.302-auto-verbose-two-refusals-1.json"),
    ),
    (
        "native/claude-2.1.302-trust-dialog-1.json",
        include_bytes!("native/claude-2.1.302-trust-dialog-1.json"),
    ),
    (
        "native/claude-2.1.302-access-allow-continuation-1.json",
        include_bytes!("native/claude-2.1.302-access-allow-continuation-1.json"),
    ),
    (
        "native/claude-2.1.302-access-allow-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-access-allow-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-access-allow-running-1.json",
        include_bytes!("native/claude-2.1.302-access-allow-running-1.json"),
    ),
    (
        "native/claude-2.1.302-access-deny-continuation-1.json",
        include_bytes!("native/claude-2.1.302-access-deny-continuation-1.json"),
    ),
    (
        "native/claude-2.1.302-access-deny-pre-stage-1.json",
        include_bytes!("native/claude-2.1.302-access-deny-pre-stage-1.json"),
    ),
    (
        "native/claude-2.1.302-access-permission-1.json",
        include_bytes!("native/claude-2.1.302-access-permission-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-env-setup-cancelled-idle-1.json",
        include_bytes!("native/claude-2.1.302-auto-env-setup-cancelled-idle-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-env-setup-offer-1.json",
        include_bytes!("native/claude-2.1.302-auto-env-setup-offer-1.json"),
    ),
    (
        "native/claude-2.1.302-auto-env-setup-wizard-1.json",
        include_bytes!("native/claude-2.1.302-auto-env-setup-wizard-1.json"),
    ),
    (
        "native/claude-2.1.302-default-manual-idle-1.json",
        include_bytes!("native/claude-2.1.302-default-manual-idle-1.json"),
    ),
    (
        "native/codex-0.163.0-on-request-permission-1.json",
        include_bytes!("native/codex-0.163.0-on-request-permission-1.json"),
    ),
    (
        "native/codex-0.163.0-allow-pre-stage-1.json",
        include_bytes!("native/codex-0.163.0-allow-pre-stage-1.json"),
    ),
    (
        "native/codex-0.163.0-deny-pre-stage-1.json",
        include_bytes!("native/codex-0.163.0-deny-pre-stage-1.json"),
    ),
    (
        "native/codex-0.163.0-idle-composer-1.json",
        include_bytes!("native/codex-0.163.0-idle-composer-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-approved-escalated-1.json",
        include_bytes!("native/codex-0.163.0-auto-approved-escalated-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-denial-limit-interrupted-1.json",
        include_bytes!("native/codex-0.163.0-auto-denial-limit-interrupted-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-denial-limit-warning-1.json",
        include_bytes!("native/codex-0.163.0-auto-denial-limit-warning-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-expanded-detail-1.json",
        include_bytes!("native/codex-0.163.0-auto-expanded-detail-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-fresh-idle-1.json",
        include_bytes!("native/codex-0.163.0-auto-fresh-idle-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-idle-composer-1.json",
        include_bytes!("native/codex-0.163.0-auto-idle-composer-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-quote-false-positive-1.json",
        include_bytes!("native/codex-0.163.0-auto-quote-false-positive-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-refusal-after-overlay-1.json",
        include_bytes!("native/codex-0.163.0-auto-refusal-after-overlay-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-refusal-idle-1.json",
        include_bytes!("native/codex-0.163.0-auto-refusal-idle-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-refusal-warning-overlay-1.json",
        include_bytes!("native/codex-0.163.0-auto-refusal-warning-overlay-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-refusal-working-1.json",
        include_bytes!("native/codex-0.163.0-auto-refusal-working-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-reviewing-1.json",
        include_bytes!("native/codex-0.163.0-auto-reviewing-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-sandbox-failure-1.json",
        include_bytes!("native/codex-0.163.0-auto-sandbox-failure-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-second-launch-idle-1.json",
        include_bytes!("native/codex-0.163.0-auto-second-launch-idle-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-status-indicator-1.json",
        include_bytes!("native/codex-0.163.0-auto-status-indicator-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-two-refusals-1.json",
        include_bytes!("native/codex-0.163.0-auto-two-refusals-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-two-refusals-resized-1.json",
        include_bytes!("native/codex-0.163.0-auto-two-refusals-resized-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-two-refusals-restored-1.json",
        include_bytes!("native/codex-0.163.0-auto-two-refusals-restored-1.json"),
    ),
    (
        "native/codex-0.163.0-auto-warning-overlay-1.json",
        include_bytes!("native/codex-0.163.0-auto-warning-overlay-1.json"),
    ),
    (
        "native/opencode-2.1.4-ask-permission-1.json",
        include_bytes!("native/opencode-2.1.4-ask-permission-1.json"),
    ),
    (
        "native/opencode-2.1.4-allow-pre-stage-1.json",
        include_bytes!("native/opencode-2.1.4-allow-pre-stage-1.json"),
    ),
    (
        "native/opencode-2.1.4-deny-pre-stage-1.json",
        include_bytes!("native/opencode-2.1.4-deny-pre-stage-1.json"),
    ),
    (
        "native/opencode-2.1.4-deny-pre-stage-2.json",
        include_bytes!("native/opencode-2.1.4-deny-pre-stage-2.json"),
    ),
    (
        "native/opencode-2.1.4-deny-pre-stage-3.json",
        include_bytes!("native/opencode-2.1.4-deny-pre-stage-3.json"),
    ),
    (
        "native/opencode-2.1.4-focus-always-1.json",
        include_bytes!("native/opencode-2.1.4-focus-always-1.json"),
    ),
    (
        "native/opencode-2.1.4-idle-composer-1.json",
        include_bytes!("native/opencode-2.1.4-idle-composer-1.json"),
    ),
];

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn verify_native_screen(entry: &Value, embedded: &[(&str, &[u8])]) -> Result<()> {
    let id = text(&entry["id"])?;
    require(
        entry["kind"] == "native",
        "bundled fixtures require native provenance",
    )?;
    let source = text(&entry["source_ref"])?;
    let bytes = embedded
        .iter()
        .find(|(reference, _)| *reference == source)
        .map(|(_, bytes)| *bytes)
        .ok_or_else(|| invalid("bundled fixture bytes are not embedded"))?;
    require(
        sha256_hex(bytes) == text(&entry["sha256"])?,
        "bundled fixture digest mismatch",
    )?;
    let screen: Value = serde_json::from_slice(bytes)
        .map_err(|cause| invalid(format!("bundled fixture: {cause}")))?;
    let backgrounds = screen.get("cell_backgrounds").is_some();
    let fields = SCREEN_FIELDS
        .iter()
        .copied()
        .chain(backgrounds.then_some("cell_backgrounds"))
        .collect::<Vec<_>>();
    let m = object(&screen, &fields)?;
    require(
        m["schema_version"].as_u64() == Some(1)
            && text(&m["fixture_id"])? == id
            && m["provenance"] == "native"
            && m["backend"] == entry["backend"]
            && m["native_version"] == entry["native_version"]
            && m["rows"] == entry["rows"]
            && m["cols"] == entry["cols"],
        "bundled fixture identity differs from its catalog entry",
    )?;
    let rows = positive(&m["rows"])?;
    let cols = positive(&m["cols"])?;
    boolean(&m["alternate_screen"])?;
    m["screen_seq"]
        .as_u64()
        .ok_or_else(|| invalid("fixture screen sequence required"))?;
    require(
        m["cursor_row"]
            .as_u64()
            .is_some_and(|row| row < u64::from(rows))
            && m["cursor_col"]
                .as_u64()
                .is_some_and(|col| col <= u64::from(cols)),
        "fixture cursor outside its geometry",
    )?;
    let lines = array(&m["lines"])?;
    require(
        lines.len() == usize::from(rows) && lines.iter().all(Value::is_string),
        "fixture lines must cover every row",
    )?;
    let mut masked = BTreeSet::new();
    for mask in array(&m["masked_rows"])? {
        let mask = object(mask, &["row", "reason"])?;
        let row = mask["row"]
            .as_u64()
            .and_then(|row| usize::try_from(row).ok())
            .filter(|row| *row < lines.len())
            .ok_or_else(|| invalid("masked row outside fixture geometry"))?;
        require(masked.insert(row), "duplicate masked row")?;
        require(
            mask["reason"] == "reviewed_privacy",
            "masked rows require reviewed privacy",
        )?;
        require(
            lines[row].as_str().is_some_and(str::is_empty),
            "masked row must be empty",
        )?;
    }
    if backgrounds {
        verify_cell_backgrounds(&m["cell_backgrounds"], rows, cols, &masked)?;
    }
    Ok(())
}
const SCREEN_FIELDS: [&str; 13] = [
    "schema_version",
    "fixture_id",
    "provenance",
    "backend",
    "native_version",
    "rows",
    "cols",
    "alternate_screen",
    "screen_seq",
    "cursor_row",
    "cursor_col",
    "lines",
    "masked_rows",
];
fn verify_cell_backgrounds(
    value: &Value,
    rows: u16,
    cols: u16,
    masked: &BTreeSet<usize>,
) -> Result<()> {
    let index = |value: &Value, bound: u64| {
        value
            .as_u64()
            .filter(|n| *n <= bound)
            .ok_or_else(|| invalid("background run coordinate outside its geometry"))
    };
    let mut spans = BTreeMap::<u64, Vec<(u64, u64)>>::new();
    for run in array(value)? {
        let run = object(run, &["row", "start_col", "end_col", "bg"])?;
        let row = index(&run["row"], u64::from(rows) - 1)?;
        let start = index(&run["start_col"], u64::from(cols))?;
        let end = index(&run["end_col"], u64::from(cols))?;
        require(start < end, "background run requires start below end")?;
        require(
            !masked.contains(&usize::try_from(row).expect("bounded row")),
            "background run inside a masked row",
        )?;
        let bg = &run["bg"];
        match bg.get("kind").and_then(Value::as_str) {
            Some("indexed") => {
                let m = object(bg, &["kind", "index"])?;
                index(&m["index"], 255)?;
            }
            Some("rgb") => {
                let m = object(bg, &["kind", "r", "g", "b"])?;
                for channel in ["r", "g", "b"] {
                    index(&m[channel], 255)?;
                }
            }
            _ => return Err(invalid("background run requires a non-default colour")),
        }
        spans.entry(row).or_default().push((start, end));
    }
    for runs in spans.values_mut() {
        runs.sort_unstable();
        require(
            runs.windows(2).all(|pair| pair[0].1 <= pair[1].0),
            "background runs overlap",
        )?;
    }
    Ok(())
}

impl ProfileCatalog {
    pub fn bundled() -> Result<Self> {
        let definitions: Value = serde_json::from_str(BUNDLED_DEFINITIONS)
            .map_err(|cause| invalid(format!("bundled definitions: {cause}")))?;
        Self::from_bundled_definitions(&definitions, BUNDLED_FIXTURES)
    }
    pub(crate) fn from_bundled_definitions(
        definitions: &Value,
        embedded: &[(&str, &[u8])],
    ) -> Result<Self> {
        let catalog = Self::from_definitions(definitions)?;
        for entry in array(&definitions["fixtures"])? {
            verify_native_screen(entry, embedded)?;
        }
        require(
            catalog.profiles.iter().all(|profile| {
                profile
                    .submissions
                    .iter()
                    .flatten()
                    .all(ComposerSubmission::native_provenance)
                    && profile
                        .answers
                        .iter()
                        .all(|a| a.provenance.iter().all(|f| f.kind == "native"))
            }),
            "bundled executable composer plans require native provenance",
        )?;
        #[cfg(test)]
        let catalog = Self {
            capture_origin: "bundled_unknown",
            ..catalog
        };
        Ok(catalog)
    }
    #[cfg(test)]
    pub(crate) fn capture_origin(&self) -> &'static str {
        self.capture_origin
    }
    pub fn from_definitions(value: &Value) -> Result<Self> {
        let root = object(value, &["schema_version", "fixtures", "profiles"])?;
        require(
            root["schema_version"].as_u64() == Some(1),
            "unsupported profile schema",
        )?;
        let mut fixtures = Vec::new();
        let mut ids = BTreeSet::new();
        for v in array(&root["fixtures"])? {
            let m = object(
                v,
                &[
                    "id",
                    "kind",
                    "backend",
                    "native_version",
                    "rows",
                    "cols",
                    "source_ref",
                    "sha256",
                ],
            )?;
            let id = text(&m["id"])?;
            unique(&mut ids, &id)?;
            let kind = one_of(&m["kind"], &["synthetic", "native"])?;
            positive(&m["rows"])?;
            positive(&m["cols"])?;
            text(&m["source_ref"])?;
            let hash = text(&m["sha256"])?;
            require(
                hash.len() == 64 && lower_hex(&hash),
                "lowercase source digest required",
            )?;
            fixtures.push(Fixture {
                id,
                kind,
                backend: backend(&m["backend"])?,
                version: text(&m["native_version"])?,
            });
        }
        let mut profiles = Vec::new();
        let mut selections = BTreeSet::new();
        for v in array(&root["profiles"])? {
            let m = object(
                v,
                &[
                    "backend",
                    "native_versions",
                    "profile_version",
                    "rules",
                    "capabilities",
                ],
            )?;
            let backend = backend(&m["backend"])?;
            let versions = texts(&m["native_versions"])?;
            require(!versions.is_empty(), "exact native versions required")?;
            for version in &versions {
                unique(&mut selections, &format!("{}:{version}", backend.name()))?;
            }
            let version = text(&m["profile_version"])?;
            let rules = array(&m["rules"])?
                .iter()
                .map(compile_rule)
                .collect::<Result<Vec<_>>>()?;
            let mut rule_ids = BTreeSet::new();
            for rule in &rules {
                unique(&mut rule_ids, &rule.id)?;
            }
            for rule in &rules {
                validate_continuation(rule, &rules, backend, &versions, &fixtures)?;
            }
            let c = object(
                &m["capabilities"],
                &["detection", "composer", "annotations", "answers"],
            )?;
            let detection = capability(
                &c["detection"],
                Group::Detection,
                &rules,
                backend,
                &versions,
                &fixtures,
            )?;
            let composer = capability(
                &c["composer"],
                Group::Composer,
                &rules,
                backend,
                &versions,
                &fixtures,
            )?;
            let annotations = capability(
                &c["annotations"],
                Group::Annotations,
                &rules,
                backend,
                &versions,
                &fixtures,
            )?;
            require(
                rules
                    .iter()
                    .filter(|rule| rule.kind.is_annotation())
                    .all(|rule| {
                        annotations
                            .as_ref()
                            .is_some_and(|group| group.rules.contains(&rule.id))
                    }),
                "every annotation rule belongs to the annotations group",
            )?;
            let submissions = composer_submissions(&c["composer"], backend, &versions, &fixtures)?;
            let mut answers = Vec::new();
            let mut decisions = BTreeSet::new();
            for v in array(&c["answers"])? {
                let a = object(
                    v,
                    &[
                        "decision",
                        "request_rule_id",
                        "answer_rule_id",
                        "choice_id",
                        "focus_matcher_id",
                        "stages",
                        "fixture_ids",
                    ],
                )?;
                let decision = PermissionDecision::try_from(text(&a["decision"])?.as_str())?;
                let request = text(&a["request_rule_id"])?;
                let id = text(&a["answer_rule_id"])?;
                unique(&mut rule_ids, &id)?;
                unique(&mut decisions, &format!("{request}:{}", decision.as_str()))?;
                let rule = rules
                    .iter()
                    .find(|r| r.id == request)
                    .ok_or_else(|| invalid("unknown answer request rule"))?;
                require(
                    matches!(
                        rule.kind,
                        ObservationKind::Interaction(
                            crate::broker::runtime::InteractionClass::PermissionRequest
                        )
                    ),
                    "answer requires permission form",
                )?;
                let choice_id = text(&a["choice_id"])?;
                let choice = rule
                    .choices
                    .iter()
                    .find(|c| c.id == choice_id)
                    .ok_or_else(|| invalid("unknown answer choice"))?;
                require(
                    choice.scope == "single_request",
                    "answer requires single-request scope",
                )?;
                let focus = text(&a["focus_matcher_id"])?;
                require(
                    rule.matchers.iter().any(|m| m.id == focus),
                    "unknown answer focus matcher",
                )?;
                let stages = array(&a["stages"])?;
                require(
                    (1..=16).contains(&stages.len()),
                    "one to sixteen measured answer stages required",
                )?;
                let mut compiled_stages = Vec::new();
                for v in stages {
                    let (s, gate) = stage_fields(v)?;
                    let bytes = text(&s["bytes_hex"])?;
                    require(
                        (2..=512).contains(&bytes.len())
                            && bytes.len().is_multiple_of(2)
                            && lower_hex(&bytes),
                        "one to 256 lowercase hexadecimal stage bytes required",
                    )?;
                    let bytes = decoded_hex(&bytes);
                    require(!bytes.contains(&0x1b), "answer stages never contain Escape")?;
                    compiled_stages.push(AnswerStage {
                        bytes,
                        required_rule_id: request.clone(),
                        required_choice_id: choice_id.clone(),
                        gate,
                    });
                    require(
                        text(&s["required_rule_id"])? == request
                            && text(&s["required_choice_id"])? == choice_id,
                        "answer stage request/decision mismatch",
                    )?;
                }
                validate_gates(&compiled_stages, rule, &rules, &choice_id, &focus)?;
                let fixture_ids = texts(&a["fixture_ids"])?;
                provenance(&fixture_ids, backend, &versions, &fixtures)?;
                answers.push(Answer {
                    decision,
                    request,
                    id,
                    provenance: fixture_ids
                        .iter()
                        .map(|id| {
                            fixtures
                                .iter()
                                .find(|f| &f.id == id)
                                .expect("validated answer fixture reference")
                                .clone()
                        })
                        .collect(),
                    fixtures: fixture_ids,
                    choice_id,
                    focus_matcher_id: focus,
                    stages: compiled_stages,
                });
            }
            profiles.push(Profile {
                backend,
                versions,
                version,
                rules,
                detection,
                composer,
                annotations,
                submissions,
                answers,
            });
        }
        Ok(Self {
            profiles,
            #[cfg(test)]
            capture_origin: "synthetic",
        })
    }
    pub(super) fn matching(&self, backend: CodingAgent, version: Option<&str>) -> Option<&Profile> {
        self.selection(backend, version).map(|(profile, _)| profile)
    }
    fn selection(
        &self,
        backend: CodingAgent,
        version: Option<&str>,
    ) -> Option<(&Profile, ProfileMatch)> {
        let version = version?;
        let candidates = || self.profiles.iter().filter(|p| p.backend == backend);
        if let Some(exact) = candidates().find(|p| p.versions.iter().any(|s| s == version)) {
            return Some((exact, ProfileMatch::Exact));
        }
        candidates()
            .max_by(|a, b| a.newest_version().cmp(&b.newest_version()))
            .map(|newest| (newest, ProfileMatch::Fallback))
    }
    pub fn rule_ids(&self) -> Vec<String> {
        self.profiles
            .iter()
            .flat_map(|p| p.rules.iter().map(|r| r.id.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn capabilities(&self, backend: CodingAgent, version: Option<&str>) -> NativeCapabilities {
        let selection = self.selection(backend, version);
        let profile = selection.map(|(profile, _)| profile);
        NativeCapabilities {
            backend,
            native_version: version.map(str::to_owned),
            profile_version: profile.map(|p| p.version.clone()),
            profile_match: selection.map(|(_, profile_match)| profile_match),
            detection: if profile.is_some_and(|p| p.detection.is_some()) {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            composer: if profile.is_some_and(|p| p.composer.is_some()) {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            prompt_submission: if profile
                .is_some_and(|p| p.submissions[ComposerOperation::Prompt.index()].is_some())
            {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            native_exec_submission: if profile
                .is_some_and(|p| p.submissions[ComposerOperation::Exec.index()].is_some())
            {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            answer_decisions: profile
                .map(|p| {
                    let mut decisions = Vec::new();
                    for answer in &p.answers {
                        if !decisions.contains(&answer.decision) {
                            decisions.push(answer.decision);
                        }
                    }
                    decisions
                })
                .unwrap_or_default(),
            capability_reason: if profile.is_none() {
                Some("native profile unavailable".into())
            } else {
                None
            },
        }
    }
    pub fn answer_capability(
        &self,
        backend: CodingAgent,
        version: &str,
        request_rule: &str,
        decision: PermissionDecision,
    ) -> Result<Option<AnswerCapability>> {
        let Some(profile) = self.matching(backend, Some(version)) else {
            return Ok(None);
        };
        Ok(profile
            .answers
            .iter()
            .find(|a| a.request == request_rule && a.decision == decision)
            .map(|answer| AnswerCapability {
                backend,
                native_version: version.into(),
                profile_version: profile.version.clone(),
                request_rule_id: request_rule.into(),
                answer_rule_id: answer.id.clone(),
                decision,
                fixture_ids: answer.fixtures.clone(),
            }))
    }

    pub(crate) fn answer_plan(
        &self,
        backend: CodingAgent,
        version: &str,
        request: &str,
        decision: PermissionDecision,
    ) -> Result<Option<AnswerPlan>> {
        Ok(self
            .matching(backend, Some(version))
            .and_then(|p| p.answer_plan(version, request, decision)))
    }
}
