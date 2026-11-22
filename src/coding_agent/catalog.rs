use crate::broker::runtime::Coverage;
use crate::broker::runtime::InteractionClass;
use crate::broker::runtime::PermissionDecision;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::detector::ObservationKind;
use crate::error::CafleetError;
use crate::error::Result;
use regex::Regex;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error as _;
use serde::de::IgnoredAny;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::LazyLock;

#[cfg(test)]
use crate::coding_agent::detector::Detector;
#[cfg(test)]
use std::collections::BTreeMap;

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
    request_rule_id: String,
    answer_rule_id: String,
    key: AnswerKey,
}
impl AnswerCapability {
    pub(crate) fn backend(&self) -> CodingAgent {
        self.backend
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
}

fn invalid(message: impl Into<String>) -> CafleetError {
    CafleetError::Usage(format!("invalid native profile: {}", message.into()))
}

fn require(ok: bool, message: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(message)) }
}
fn unique(ids: &mut BTreeSet<String>, id: &str) -> Result<()> {
    require(ids.insert(id.into()), "duplicate profile identity")
}
/// The bytes of `text` when it is lowercase hexadecimal of at most `max` bytes.
fn lower_hex_bytes(text: &str, max: usize) -> Option<Vec<u8>> {
    let lower = text
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    (lower && text.len().is_multiple_of(2) && text.len() / 2 <= max).then(|| {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex digits"))
            .collect()
    })
}

type Field<T, E> = std::result::Result<T, E>;

/// An optional key that, when present, holds a value rather than `null`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Field<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn nonblank<'de, D: Deserializer<'de>>(d: D) -> Field<String, D::Error> {
    NonBlank::deserialize(d).map(|text| text.0)
}
fn pattern<'de, D: Deserializer<'de>>(d: D) -> Field<Regex, D::Error> {
    Regex::new(&nonblank(d)?).map_err(|cause| D::Error::custom(format!("regex: {cause}")))
}
fn patterns<'de, D: Deserializer<'de>>(d: D) -> Field<Vec<Regex>, D::Error> {
    Vec::<NonBlank>::deserialize(d)?
        .into_iter()
        .map(|text| {
            Regex::new(&text.0).map_err(|cause| D::Error::custom(format!("regex: {cause}")))
        })
        .collect()
}
fn positive_bound<'de, D: Deserializer<'de>>(d: D) -> Field<u16, D::Error> {
    u64::deserialize(d)
        .map(u16::try_from)?
        .ok()
        .filter(|bound| *bound > 0)
        .ok_or_else(|| D::Error::custom("positive viewport bound required"))
}
fn glyph<'de, D: Deserializer<'de>>(d: D) -> Field<Option<char>, D::Error> {
    let glyph = String::deserialize(d)?;
    let mut chars = glyph.chars();
    match (chars.next(), chars.next()) {
        (Some(separator), None) if !separator.is_whitespace() => Ok(Some(separator)),
        _ => Err(D::Error::custom(
            "panel separator requires one non-whitespace glyph",
        )),
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct NonBlank(String);
impl TryFrom<String> for NonBlank {
    type Error = &'static str;
    fn try_from(value: String) -> Field<Self, Self::Error> {
        if value.trim().is_empty() {
            Err("nonblank text required")
        } else {
            Ok(Self(value))
        }
    }
}

/// Nonblank, pairwise distinct strings.
#[derive(Deserialize)]
#[serde(try_from = "Vec<NonBlank>")]
struct Ids(Vec<String>);
impl TryFrom<Vec<NonBlank>> for Ids {
    type Error = &'static str;
    fn try_from(values: Vec<NonBlank>) -> Field<Self, Self::Error> {
        let values = values.into_iter().map(|value| value.0).collect::<Vec<_>>();
        if values.iter().collect::<BTreeSet<_>>().len() == values.len() {
            Ok(Self(values))
        } else {
            Err("duplicate string identity")
        }
    }
}

/// The fixture registry and every `fixture_ids` value are provenance that the bundled-catalog
/// build guard checks; the production loader accepts their contents unread.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    schema_version: u64,
    #[serde(rename = "fixtures")]
    _fixtures: IgnoredAny,
    profiles: Vec<RawProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    backend: CodingAgent,
    rules: Vec<RawRule>,
    capabilities: RawCapabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: NonBlank,
    priority: i64,
    observation: ObservationKind,
    region: Region,
    matchers: Vec<Matcher>,
    extract: RawExtract,
    verified_noninteraction: bool,
    #[serde(default, deserialize_with = "present")]
    frame: Option<Frame>,
    #[serde(default, deserialize_with = "present")]
    continues_rule_id: Option<NonBlank>,
    #[serde(default, deserialize_with = "present")]
    continues_choice_id: Option<NonBlank>,
    #[serde(default, deserialize_with = "present")]
    fixture_ids: Option<IgnoredAny>,
    #[serde(default, deserialize_with = "present")]
    suggestion: Option<RawSuggestion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExtract {
    #[serde(deserialize_with = "Option::deserialize")]
    operation: Option<RawOperation>,
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOperation {
    matcher_id: NonBlank,
    capture: NonBlank,
    #[serde(default, deserialize_with = "present")]
    row_margin: Option<String>,
    #[serde(default, deserialize_with = "present")]
    margin_rows: Option<MarginRows>,
    #[serde(default, deserialize_with = "present")]
    row_trailer: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilities {
    #[serde(deserialize_with = "Option::deserialize")]
    detection: Option<RawCapability>,
    #[serde(deserialize_with = "Option::deserialize")]
    composer: Option<RawCapability>,
    answers: Vec<RawAnswer>,
}

/// One capability group; only the composer group may declare `submission`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapability {
    rule_ids: Ids,
    #[serde(rename = "fixture_ids")]
    _fixture_ids: IgnoredAny,
    #[serde(default, deserialize_with = "present")]
    submission: Option<RawSubmissions>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubmissions {
    prompt: RawSubmission,
    exec: RawSubmission,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubmission {
    prefix_hex: String,
    submit_hex: String,
    literal_framing: LiteralFraming,
    #[serde(deserialize_with = "Option::deserialize")]
    plain_literal_limit: Option<PlainLiteralLimit>,
    #[serde(rename = "fixture_ids")]
    _fixture_ids: IgnoredAny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LiteralFraming {
    Plain,
    BracketedPaste,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LiteralUnit {
    Bytes,
    Scalars,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainLiteralLimit {
    unit: LiteralUnit,
    max: std::num::NonZeroU64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnswer {
    #[serde(default, deserialize_with = "present")]
    decision: Option<NonBlank>,
    request_rule_id: NonBlank,
    answer_rule_id: NonBlank,
    choice_id: NonBlank,
    focus_matcher_id: NonBlank,
    stages: Vec<RawStage>,
    #[serde(rename = "fixture_ids")]
    _fixture_ids: IgnoredAny,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStage {
    bytes_hex: NonBlank,
    required_rule_id: NonBlank,
    required_choice_id: NonBlank,
    #[serde(default, deserialize_with = "present")]
    required_focus_choice_id: Option<NonBlank>,
    #[serde(default, deserialize_with = "present")]
    required_screen_rule_id: Option<NonBlank>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Capture {
    #[serde(rename = "matcher_id", deserialize_with = "nonblank")]
    pub(crate) matcher: String,
    #[serde(rename = "capture", deserialize_with = "nonblank")]
    pub(crate) name: String,
    #[serde(skip)]
    pub(crate) row_margin: Option<String>,
    #[serde(skip)]
    pub(crate) margin_rows: MarginRows,
    #[serde(skip)]
    pub(crate) row_trailer: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MarginRows {
    #[default]
    All,
    Continuation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChoiceScope {
    SingleRequest,
    Persistent,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Choice {
    #[serde(deserialize_with = "nonblank")]
    pub(crate) id: String,
    pub(crate) label: Capture,
    pub(crate) scope: ChoiceScope,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MatcherScope {
    Line,
    Region,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Matcher {
    #[serde(deserialize_with = "nonblank")]
    pub(crate) id: String,
    pub(crate) scope: MatcherScope,
    #[serde(rename = "pattern", deserialize_with = "pattern")]
    pub(crate) regex: Regex,
    #[serde(default, deserialize_with = "present")]
    pub(crate) focus_style: Option<FocusStyle>,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FocusStyle {
    Background,
    Marker,
}
#[derive(Clone)]
pub(super) struct Continuation {
    pub(crate) rule_id: String,
    pub(crate) choice_id: String,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScreenSelector {
    Primary,
    Alternate,
    Either,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CursorRule {
    AtEnd,
    /// On the region's first row, right after the region's `start_pattern` match.
    AfterPrompt,
    Inside,
    Any,
}
/// The cell style a native CLI draws an inline suggestion in; such cells are not input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionStyle {
    Faint,
    Foreground(vt100::Color),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSuggestion {
    style: RawSuggestionStyle,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSuggestionStyle {
    #[serde(default, deserialize_with = "present")]
    faint: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    foreground: Option<RawColour>,
}
/// The fixture `cell_backgrounds` colour form.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawColour {
    Indexed { index: u8 },
    Rgb { r: u8, g: u8, b: u8 },
}
fn suggestion_style(raw: RawSuggestion) -> Result<SuggestionStyle> {
    match (raw.style.faint, raw.style.foreground) {
        (Some(true), None) => Ok(SuggestionStyle::Faint),
        (None, Some(RawColour::Indexed { index })) => {
            Ok(SuggestionStyle::Foreground(vt100::Color::Idx(index)))
        }
        (None, Some(RawColour::Rgb { r, g, b })) => {
            Ok(SuggestionStyle::Foreground(vt100::Color::Rgb(r, g, b)))
        }
        _ => Err(invalid(
            "a suggestion style is exactly {\"faint\": true} or one indexed or rgb foreground",
        )),
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    pub(crate) screen: ScreenSelector,
    #[serde(rename = "start_pattern", deserialize_with = "pattern")]
    pub(crate) start: Regex,
    #[serde(rename = "end_pattern", deserialize_with = "pattern")]
    pub(crate) end: Regex,
    #[serde(deserialize_with = "positive_bound")]
    pub(crate) max_rows: u16,
    pub(crate) cursor: CursorRule,
    #[serde(default, deserialize_with = "glyph")]
    pub(crate) panel_separator: Option<char>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Frame {
    #[serde(rename = "require_patterns", deserialize_with = "patterns")]
    pub(crate) require: Vec<Regex>,
    #[serde(rename = "reject_patterns", deserialize_with = "patterns")]
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
    pub(crate) verified: bool,
    pub(crate) continuation: Option<Continuation>,
    pub(crate) frame: Option<Frame>,
    pub(crate) suggestion: Option<SuggestionStyle>,
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
    pub(crate) choice_id: String,
    pub(crate) focus_matcher_id: String,
    pub(crate) stages: Vec<AnswerStage>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AnswerStage {
    bytes: Vec<u8>,
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
    pub(crate) rules: Vec<Rule>,
    pub(crate) detection: Option<Capability>,
    pub(crate) composer: Option<Capability>,
    pub(crate) submissions: [Option<ComposerSubmission>; 2],
    pub(crate) answers: Vec<Answer>,
}
impl Profile {
    pub(super) fn answer_plan(&self, request: &str, key: &AnswerKey) -> Option<AnswerPlan> {
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
                request_rule_id: request.into(),
                answer_rule_id: answer.id.clone(),
                key: key.clone(),
            },
            choice_id: answer.choice_id.clone(),
            choice_scope,
            focus_matcher_id: answer.focus_matcher_id.clone(),
            stages: answer.stages.clone(),
        })
    }
    /// Every plan of the request rule: decisions in `allow`, `deny` order, then choices in the
    /// rule's choice order.
    pub(super) fn answer_plans(&self, request: &str) -> Vec<AnswerPlan> {
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
            .filter_map(|key| self.answer_plan(request, &key))
            .collect()
    }
}
#[derive(Clone)]
pub(crate) struct ProfileCatalog {
    profiles: Vec<Profile>,
    #[cfg(test)]
    capture_origin: &'static str,
}
fn validate_capture(capture: &Capture, matchers: &[Matcher]) -> Result<()> {
    let rule = matchers
        .iter()
        .find(|item| item.id == capture.matcher)
        .ok_or_else(|| invalid("unknown capture matcher"))?;
    require(
        rule.regex
            .capture_names()
            .flatten()
            .any(|candidate| candidate == capture.name),
        "unknown named capture",
    )
}
fn operation_capture(raw: RawOperation, matchers: &[Matcher]) -> Result<Capture> {
    let single_row = |text: &String| !text.is_empty() && !text.contains('\n');
    require(
        raw.row_margin.as_ref().is_none_or(single_row),
        "nonempty single-row margin required",
    )?;
    require(
        raw.margin_rows.is_none() || raw.row_margin.is_some(),
        "margin_rows requires row_margin",
    )?;
    require(
        raw.row_trailer
            .as_ref()
            .is_none_or(|trailer| single_row(trailer) && trailer.trim() == trailer),
        "trimmed nonempty single-row trailer required",
    )?;
    let capture = Capture {
        matcher: raw.matcher_id.0,
        name: raw.capture.0,
        row_margin: raw.row_margin,
        margin_rows: raw.margin_rows.unwrap_or_default(),
        row_trailer: raw.row_trailer,
    };
    validate_capture(&capture, matchers)?;
    Ok(capture)
}
fn compile_rule(raw: RawRule) -> Result<Rule> {
    let kind = raw.observation;
    let continues = kind == ObservationKind::PermissionContinuation;
    let continuation = match (
        continues,
        raw.continues_rule_id,
        raw.continues_choice_id,
        raw.fixture_ids,
    ) {
        (true, Some(rule), Some(choice), Some(_)) => Some(Continuation {
            rule_id: rule.0,
            choice_id: choice.0,
        }),
        (false, None, None, None) => None,
        _ => {
            return Err(invalid(
                "exactly the permission continuation rules declare continues_rule_id, continues_choice_id and fixture_ids",
            ));
        }
    };
    let verified = raw.verified_noninteraction;
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
    let matchers = raw.matchers;
    let mut ids = BTreeSet::new();
    for matcher in &matchers {
        unique(&mut ids, &matcher.id)?;
    }
    require(!matchers.is_empty(), "required matchers cannot be empty")?;
    let choices = raw.extract.choices;
    let mut ids = BTreeSet::new();
    for choice in &choices {
        unique(&mut ids, &choice.id)?;
        validate_capture(&choice.label, &matchers)?;
    }
    require(
        !continues || (raw.extract.operation.is_none() && choices.is_empty()),
        "continuation declares no operation or choices",
    )?;
    let operation = raw
        .extract
        .operation
        .map(|operation| operation_capture(operation, &matchers))
        .transpose()?;
    match (&raw.frame, matches!(kind, ObservationKind::Interaction(_))) {
        (Some(frame), true) => require(
            !frame.reject.is_empty(),
            "interaction frame requires a reject pattern",
        )?,
        (Some(_), false) => return Err(invalid("only interaction rules declare a frame")),
        (None, _) => {}
    }
    let suggestion = raw.suggestion.map(suggestion_style).transpose()?;
    require(
        suggestion.is_none() || kind == ObservationKind::ComposerEmpty,
        "only composer_empty rules declare a suggestion style",
    )?;
    Ok(Rule {
        id: raw.id.0,
        priority: raw.priority,
        kind,
        region: raw.region,
        operation,
        choices,
        matchers,
        verified,
        continuation,
        frame: raw.frame,
        suggestion,
    })
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
fn validate_continuation(rule: &Rule, rules: &[Rule]) -> Result<()> {
    let Some(continuation) = &rule.continuation else {
        return Ok(());
    };
    let continued = rules
        .iter()
        .find(|candidate| candidate.id == continuation.rule_id)
        .ok_or_else(|| invalid("continuation names an unknown rule"))?;
    require(
        continued.kind == ObservationKind::Interaction(InteractionClass::PermissionRequest),
        "continuation continues a permission request rule",
    )?;
    require(
        continued
            .choices
            .iter()
            .any(|choice| choice.id == continuation.choice_id),
        "continuation names an unknown continued choice",
    )
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Detection,
    Composer,
}
impl Group {
    fn of(kind: ObservationKind) -> Self {
        if kind == ObservationKind::ComposerEmpty {
            Self::Composer
        } else {
            Self::Detection
        }
    }
}
fn capability(raw: &RawCapability, group: Group, rules: &[Rule]) -> Result<Capability> {
    require(
        group == Group::Composer || raw.submission.is_none(),
        "only the composer capability declares a submission",
    )?;
    require(
        group != Group::Composer || raw.submission.is_some(),
        "the composer capability declares a submission",
    )?;
    let ids = raw.rule_ids.0.clone();
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
    Ok(Capability { rules: ids })
}

fn composer_submission(
    raw: RawSubmission,
    operation: ComposerOperation,
) -> Result<ComposerSubmission> {
    require(
        lower_hex_bytes(&raw.prefix_hex, usize::MAX).as_deref() == Some(operation.prefix()),
        "composer prefix must match its operation",
    )?;
    let submit = lower_hex_bytes(&raw.submit_hex, 32)
        .filter(|submit| !submit.is_empty())
        .ok_or_else(|| invalid("composer submit requires 1–32 lowercase hexadecimal bytes"))?;
    Ok(ComposerSubmission {
        submit,
        literal_framing: raw.literal_framing,
        plain_literal_limit: raw.plain_literal_limit,
    })
}

fn compile_answer(
    raw: RawAnswer,
    rules: &[Rule],
    identities: &mut BTreeSet<String>,
    keys: &mut BTreeSet<String>,
) -> Result<Answer> {
    let (request, id, choice_id) = (raw.request_rule_id.0, raw.answer_rule_id.0, raw.choice_id.0);
    unique(identities, &id)?;
    let rule = rules
        .iter()
        .find(|r| r.id == request)
        .ok_or_else(|| invalid("unknown answer request rule"))?;
    let decided = raw.decision.is_some();
    let key = match raw.decision {
        Some(decision) => {
            let decision = PermissionDecision::try_from(decision.0.as_str())?;
            require(
                rule.kind == ObservationKind::Interaction(InteractionClass::PermissionRequest),
                "answer requires permission form",
            )?;
            AnswerKey::Decision(decision)
        }
        None => {
            require(
                rule.kind == ObservationKind::Interaction(InteractionClass::Question),
                "choice answer requires question form",
            )?;
            require(
                valid_choice_id(&choice_id),
                "choice id requires lowercase hyphenated words of at most 64 bytes",
            )?;
            AnswerKey::Choice(choice_id.clone())
        }
    };
    unique(
        keys,
        &match &key {
            AnswerKey::Decision(decision) => format!("{request}:decision:{}", decision.as_str()),
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
    let focus = raw.focus_matcher_id.0;
    require(
        rule.matchers.iter().any(|m| m.id == focus),
        "unknown answer focus matcher",
    )?;
    require(
        (1..=16).contains(&raw.stages.len()),
        "one to sixteen measured answer stages required",
    )?;
    let stages = raw
        .stages
        .into_iter()
        .map(|stage| {
            let bytes = lower_hex_bytes(&stage.bytes_hex.0, 256)
                .filter(|bytes| !bytes.is_empty())
                .ok_or_else(|| invalid("one to 256 lowercase hexadecimal stage bytes required"))?;
            require(!bytes.contains(&0x1b), "answer stages never contain Escape")?;
            require(
                stage.required_rule_id.0 == request && stage.required_choice_id.0 == choice_id,
                "answer stage request/decision mismatch",
            )?;
            let gate = match (
                stage.required_focus_choice_id,
                stage.required_screen_rule_id,
            ) {
                (Some(_), Some(_)) => return Err(invalid("a stage declares at most one gate")),
                (Some(choice), None) => Some(StageGate::Focus(choice.0)),
                (None, Some(rule)) => Some(StageGate::Screen(rule.0)),
                (None, None) => None,
            };
            Ok(AnswerStage { bytes, gate })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_gates(&stages, rule, rules, &choice_id, &focus)?;
    Ok(Answer {
        key,
        request,
        id,
        choice_id,
        focus_matcher_id: focus,
        stages,
    })
}

fn compile_profile(raw: RawProfile) -> Result<Profile> {
    let backend = raw.backend;
    let rules = raw
        .rules
        .into_iter()
        .map(compile_rule)
        .collect::<Result<Vec<_>>>()?;
    let mut identities = BTreeSet::new();
    for rule in &rules {
        unique(&mut identities, &rule.id)?;
    }
    for rule in &rules {
        validate_continuation(rule, &rules)?;
    }
    let capabilities = raw.capabilities;
    let group = |raw: &Option<RawCapability>, group| {
        raw.as_ref()
            .map(|raw| capability(raw, group, &rules))
            .transpose()
    };
    let detection = group(&capabilities.detection, Group::Detection)?;
    let composer = group(&capabilities.composer, Group::Composer)?;
    let submissions = match capabilities
        .composer
        .and_then(|composer| composer.submission)
    {
        Some(submission) => [
            Some(composer_submission(
                submission.prompt,
                ComposerOperation::Prompt,
            )?),
            Some(composer_submission(
                submission.exec,
                ComposerOperation::Exec,
            )?),
        ],
        None => [None, None],
    };
    let mut keys = BTreeSet::new();
    let answers = capabilities
        .answers
        .into_iter()
        .map(|answer| compile_answer(answer, &rules, &mut identities, &mut keys))
        .collect::<Result<Vec<_>>>()?;
    Ok(Profile {
        backend,
        rules,
        detection,
        composer,
        submissions,
        answers,
    })
}

const BUNDLED_DEFINITIONS: &str = include_str!("../../assets/profiles/bundled.json");

static BUNDLED: LazyLock<ProfileCatalog> = LazyLock::new(|| {
    let catalog = ProfileCatalog::from_definitions(
        &serde_json::from_str(BUNDLED_DEFINITIONS).expect("bundled.json is valid JSON"),
    )
    .expect("the bundled catalog passes its build guard");
    #[cfg(test)]
    let catalog = ProfileCatalog {
        capture_origin: "bundled_unknown",
        ..catalog
    };
    catalog
});

impl ProfileCatalog {
    pub(crate) fn bundled() -> &'static ProfileCatalog {
        &BUNDLED
    }
    #[cfg(test)]
    pub(crate) fn capture_origin(&self) -> &'static str {
        self.capture_origin
    }
    pub(crate) fn from_definitions(value: &Value) -> Result<Self> {
        let raw = RawCatalog::deserialize(value).map_err(|cause| invalid(cause.to_string()))?;
        require(raw.schema_version == 1, "unsupported profile schema")?;
        for (index, profile) in raw.profiles.iter().enumerate() {
            require(
                raw.profiles[..index]
                    .iter()
                    .all(|earlier| earlier.backend != profile.backend),
                "more than one profile for one backend",
            )?;
        }
        let profiles = raw
            .profiles
            .into_iter()
            .map(compile_profile)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            profiles,
            #[cfg(test)]
            capture_origin: "synthetic",
        })
    }
    pub(super) fn profile(&self, backend: CodingAgent) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.backend == backend)
    }
    pub(crate) fn detection_coverage(&self, backend: CodingAgent) -> Coverage {
        if self
            .profile(backend)
            .is_some_and(|profile| profile.detection.is_some())
        {
            Coverage::Tested
        } else {
            Coverage::Unknown
        }
    }
    pub(crate) fn answer_plans(&self, backend: CodingAgent, request: &str) -> Vec<AnswerPlan> {
        self.profile(backend)
            .map(|profile| profile.answer_plans(request))
            .unwrap_or_default()
    }
}

#[cfg(test)]
impl ProfileCatalog {
    /// This catalog without `operation`'s submission plan for `backend`, for missing-plan
    /// fallback tests.
    pub(crate) fn without_submission(
        mut self,
        backend: CodingAgent,
        operation: ComposerOperation,
    ) -> Self {
        for profile in self.profiles.iter_mut().filter(|p| p.backend == backend) {
            profile.submissions[operation.index()] = None;
        }
        self
    }
    pub(crate) fn answer_plan(
        &self,
        backend: CodingAgent,
        request: &str,
        key: &AnswerKey,
    ) -> Option<AnswerPlan> {
        self.profile(backend)?.answer_plan(request, key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
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

    /// The bytes that precede an operation's literal in the native composer.
    pub(crate) fn prefix(self) -> &'static [u8] {
        match self {
            Self::Prompt => b"",
            Self::Exec => b"! ",
        }
    }
}

#[derive(Clone)]
pub(super) struct ComposerSubmission {
    pub(crate) submit: Vec<u8>,
    literal_framing: LiteralFraming,
    plain_literal_limit: Option<PlainLiteralLimit>,
}

pub(crate) struct ComposerPlan {
    operation: ComposerOperation,
    submit: Vec<u8>,
    literal_framing: LiteralFraming,
    plain_literal_limit: Option<PlainLiteralLimit>,
}

impl ComposerPlan {
    pub(crate) fn literal_framing(&self) -> LiteralFraming {
        self.literal_framing
    }

    pub(crate) fn plain_literal_allowed(&self, literal: &str) -> bool {
        self.plain_literal_limit.is_none_or(|limit| {
            let size = match limit.unit {
                LiteralUnit::Bytes => self.operation.prefix().len().checked_add(literal.len()),
                LiteralUnit::Scalars => self
                    .operation
                    .prefix()
                    .len()
                    .checked_add(literal.chars().count()),
            };
            size.is_some_and(|size| u64::try_from(size).is_ok_and(|size| size <= limit.max.get()))
        })
    }

    pub(crate) fn plain_limit_refusal(&self, literal: &str, backend: &str) -> String {
        let limit = self
            .plain_literal_limit
            .expect("a refused plain literal has its limit");
        let (size, unit) = match limit.unit {
            LiteralUnit::Bytes => (literal.len(), "bytes"),
            LiteralUnit::Scalars => (literal.chars().count(), "characters"),
        };
        let size = size
            .checked_add(self.operation.prefix().len())
            .expect("a bounded input size fits");
        let reason = match self.literal_framing {
            LiteralFraming::Plain => {
                format!("the {backend} profile has no bracketed-paste delivery")
            }
            LiteralFraming::BracketedPaste => {
                "the native CLI has not enabled bracketed paste".into()
            }
        };
        format!(
            "the input is {size} {unit}, above the {backend} plain-input limit of {} {unit}, and {reason}",
            limit.max
        )
    }

    fn encode(&self, literal: &str) -> std::result::Result<Vec<u8>, CafleetError> {
        let prefix = self.operation.prefix();
        let size = prefix
            .len()
            .checked_add(literal.len())
            .and_then(|size| size.checked_add(self.submit.len()))
            .filter(|size| *size <= 65536)
            .ok_or_else(|| {
                CafleetError::App("native composer payload exceeds 65536 bytes".into())
            })?;
        let mut bytes = Vec::with_capacity(size);
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(&self.submit);
        Ok(bytes)
    }

    pub(crate) fn stages(
        &self,
        literal: &str,
        framing: LiteralFraming,
    ) -> std::result::Result<[Vec<u8>; 2], CafleetError> {
        let mut encoded = self.encode(literal)?;
        if framing == LiteralFraming::BracketedPaste {
            if self.literal_framing() != LiteralFraming::BracketedPaste {
                return Err(CafleetError::App(
                    "native composer plan requires plain literal framing".into(),
                ));
            }
            if encoded
                .len()
                .checked_add(12)
                .is_none_or(|size| size > 65536)
            {
                return Err(CafleetError::App(
                    "native composer payload exceeds 65536 bytes".into(),
                ));
            }
            let submit_offset = encoded.len() - self.submit.len();
            encoded.splice(submit_offset..submit_offset, b"\x1b[201~".iter().copied());
            let prefix_offset = self.operation.prefix().len();
            encoded.splice(prefix_offset..prefix_offset, b"\x1b[200~".iter().copied());
        }
        let submit = encoded.split_off(encoded.len() - self.submit.len());
        Ok([encoded, submit])
    }
}

impl ProfileCatalog {
    pub(crate) fn verified_composer(
        &self,
        backend: CodingAgent,
        operation: ComposerOperation,
    ) -> bool {
        self.profile(backend).is_some_and(|profile| {
            profile.composer.is_some() && profile.submissions[operation.index()].is_some()
        })
    }

    pub(crate) fn composer_plan(
        &self,
        backend: CodingAgent,
        operation: ComposerOperation,
    ) -> Option<ComposerPlan> {
        let profile = self.profile(backend)?;
        let submission = profile.submissions[operation.index()].as_ref()?;
        Some(ComposerPlan {
            operation,
            submit: submission.submit.clone(),
            literal_framing: submission.literal_framing,
            plain_literal_limit: submission.plain_literal_limit,
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::coding_agent::NativeVersion;
    use crate::coding_agent::catalog::*;
    use serde_json::Map;
    use serde_json::json;

    fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
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

    const CLAUDE_FIXTURES: &[u8] = include_bytes!("../../assets/profiles/native/claude.json");
    const CODEX_FIXTURES: &[u8] = include_bytes!("../../assets/profiles/native/codex.json");
    const OPENCODE_FIXTURES: &[u8] = include_bytes!("../../assets/profiles/native/opencode.json");
    const HERMES_FIXTURES: &[u8] = include_bytes!("../../assets/profiles/native/hermes.json");
    const PI_FIXTURES: &[u8] = include_bytes!("../../assets/profiles/native/pi.json");
    pub(crate) const BUNDLED_FIXTURES: &[(&str, &[u8])] = &[
        ("native/claude.json", CLAUDE_FIXTURES),
        ("native/codex.json", CODEX_FIXTURES),
        ("native/opencode.json", OPENCODE_FIXTURES),
        ("native/hermes.json", HERMES_FIXTURES),
        ("native/pi.json", PI_FIXTURES),
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

    const SYNTAX_LETTER_ESCAPES: &[char] = &['t', 'n', 'r', 'f', 'v', 'x', 'p', 'A', 'z'];

    /// Each escape of `pattern` outside the pattern syntax, spelled as written.
    fn escapes_outside_syntax(pattern: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = pattern;
        while let Some(at) = rest.find('\\') {
            let Some(letter) = rest[at + 1..].chars().next() else {
                found.push("\\".to_owned());
                break;
            };
            let after = &rest[at + 1 + letter.len_utf8()..];
            if !(letter.is_ascii_punctuation() || SYNTAX_LETTER_ESCAPES.contains(&letter)) {
                let braced = after
                    .starts_with('{')
                    .then(|| after.find('}'))
                    .flatten()
                    .map_or(0, |close| close + 1);
                found.push(format!("\\{letter}{}", &after[..braced]));
            }
            rest = after;
        }
        found
    }

    fn profile_strings<'a>(value: &'a Value, pointer: String, found: &mut Vec<(String, &'a str)>) {
        match value {
            Value::String(text) => found.push((pointer, text)),
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    profile_strings(item, format!("{pointer}/{index}"), found);
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    profile_strings(item, format!("{pointer}/{key}"), found);
                }
            }
            _ => {}
        }
    }

    /// Whether the catalog decoder compiles the profile string at `pointer` as a pattern.
    fn decodes_as_pattern(profile: &Value, pointer: &str) -> bool {
        let mut probe = profile.clone();
        *probe
            .pointer_mut(pointer)
            .expect("the pointer names a profile string") = json!("(");
        RawProfile::deserialize(&probe).is_err_and(|cause| cause.to_string().contains("regex: "))
    }

    fn pattern_location(profile: &Value, pointer: &str) -> String {
        let mut segments = pointer.trim_start_matches('/').split('/');
        match (segments.next(), segments.next()) {
            (Some("rules"), Some(index)) => {
                let index = index.parse::<usize>().expect("a rule index");
                format!(
                    "rule {}",
                    profile["rules"][index]["id"].as_str().expect("a rule id")
                )
            }
            (Some(field), _) => field.to_owned(),
            (None, _) => unreachable!("split yields at least one segment"),
        }
    }

    /// One ``<backend> <location>: `<construct>` `` line per Perl class, boundary
    /// or other escape outside the pattern syntax in a profile string the decoder compiles.
    pub(crate) fn implicit_pattern_classes(definitions: &Value) -> Vec<String> {
        let mut found = Vec::new();
        for profile in array(&definitions["profiles"]).expect("catalog profiles") {
            RawProfile::deserialize(profile).expect("the profile decodes before probing");
            let name = text(&profile["backend"]).expect("profile backend");
            let mut strings = Vec::new();
            profile_strings(profile, String::new(), &mut strings);
            for (pointer, pattern) in strings {
                let constructs = escapes_outside_syntax(pattern);
                if constructs.is_empty() || !decodes_as_pattern(profile, &pointer) {
                    continue;
                }
                let location = pattern_location(profile, &pointer);
                found.extend(
                    constructs
                        .into_iter()
                        .map(|construct| format!("{name} {location}: `{construct}`")),
                );
            }
        }
        found
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
        let styles = screen.get("cell_styles").is_some();
        let fields = SCREEN_FIELDS
            .iter()
            .copied()
            .chain(backgrounds.then_some("cell_backgrounds"))
            .chain(styles.then_some("cell_styles"))
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
        if styles {
            verify_cell_styles(&m["cell_styles"], rows, cols, &masked)?;
        }
        Ok(())
    }

    fn verify_colour(value: &Value) -> Result<()> {
        let channel = |value: &Value| {
            value
                .as_u64()
                .filter(|n| *n <= 255)
                .ok_or_else(|| invalid("fixture colour channel outside 0..=255"))
        };
        match value.get("kind").and_then(Value::as_str) {
            Some("indexed") => {
                channel(&object(value, &["kind", "index"])?["index"])?;
            }
            Some("rgb") => {
                let m = object(value, &["kind", "r", "g", "b"])?;
                for name in ["r", "g", "b"] {
                    channel(&m[name])?;
                }
            }
            _ => return Err(invalid("fixture colour must be indexed or rgb")),
        }
        Ok(())
    }

    fn verify_cell_styles(
        value: &Value,
        rows: u16,
        cols: u16,
        masked: &BTreeSet<usize>,
    ) -> Result<()> {
        let index = |value: &Value, bound: u64| {
            value
                .as_u64()
                .filter(|n| *n <= bound)
                .ok_or_else(|| invalid("style run coordinate outside its geometry"))
        };
        let mut spans = BTreeMap::<u64, Vec<(u64, u64)>>::new();
        for run in array(value)? {
            let run = object(run, &["row", "start_col", "end_col", "faint", "foreground"])?;
            let row = index(&run["row"], u64::from(rows) - 1)?;
            let start = index(&run["start_col"], u64::from(cols))?;
            let end = index(&run["end_col"], u64::from(cols))?;
            require(start < end, "style run requires start below end")?;
            require(
                !masked.contains(&usize::try_from(row).expect("bounded row")),
                "style run inside a masked row",
            )?;
            let faint = boolean(&run["faint"])?;
            let foreground = &run["foreground"];
            if !foreground.is_null() {
                verify_colour(foreground)?;
            }
            require(
                faint || !foreground.is_null(),
                "style run requires a non-default attribute",
            )?;
            spans.entry(row).or_default().push((start, end));
        }
        for runs in spans.values_mut() {
            runs.sort_unstable();
            require(
                runs.windows(2).all(|pair| pair[0].1 <= pair[1].0),
                "style runs overlap",
            )?;
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

    #[derive(Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum FixtureKind {
        Synthetic,
        Native,
    }

    #[derive(Deserialize)]
    struct ProvenanceCatalog {
        fixtures: Vec<ProvenanceFixture>,
        profiles: Vec<ProvenanceProfile>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ProvenanceFixture {
        id: NonBlank,
        kind: FixtureKind,
        backend: CodingAgent,
        #[serde(rename = "native_version")]
        _native_version: NativeVersion,
        #[serde(rename = "rows", deserialize_with = "positive_bound")]
        _rows: u16,
        #[serde(rename = "cols", deserialize_with = "positive_bound")]
        _cols: u16,
        #[serde(rename = "source_ref")]
        _source_ref: NonBlank,
        sha256: NonBlank,
        #[serde(default, deserialize_with = "present")]
        composer_evidence: Option<Vec<ComposerEvidence>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum EvidenceMultiplexer {
        Tmux,
        Herdr,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum ComposerEvidenceKind {
        Framing,
        PlainLimit,
        PostSubmit,
        UnsubmittedLiteral,
    }

    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum ComposerEvidence {
        Framing {
            operation: ComposerOperation,
            multiplexer: EvidenceMultiplexer,
            literal_framing: LiteralFraming,
        },
        PlainLimit {
            operation: ComposerOperation,
            multiplexer: EvidenceMultiplexer,
            #[serde(deserialize_with = "Option::deserialize")]
            plain_literal_limit: Option<PlainLiteralLimit>,
        },
        PostSubmit {
            operation: ComposerOperation,
            multiplexer: EvidenceMultiplexer,
        },
        UnsubmittedLiteral {
            operation: ComposerOperation,
            multiplexer: EvidenceMultiplexer,
        },
    }

    impl ComposerEvidence {
        fn identity(&self) -> (ComposerOperation, EvidenceMultiplexer, ComposerEvidenceKind) {
            match self {
                Self::Framing {
                    operation,
                    multiplexer,
                    ..
                } => (*operation, *multiplexer, ComposerEvidenceKind::Framing),
                Self::PlainLimit {
                    operation,
                    multiplexer,
                    ..
                } => (*operation, *multiplexer, ComposerEvidenceKind::PlainLimit),
                Self::PostSubmit {
                    operation,
                    multiplexer,
                } => (*operation, *multiplexer, ComposerEvidenceKind::PostSubmit),
                Self::UnsubmittedLiteral {
                    operation,
                    multiplexer,
                } => (
                    *operation,
                    *multiplexer,
                    ComposerEvidenceKind::UnsubmittedLiteral,
                ),
            }
        }

        fn matches_plan(&self, plan: &ProvenanceSubmission) -> bool {
            match self {
                Self::Framing {
                    literal_framing, ..
                } => *literal_framing == plan.literal_framing,
                Self::PlainLimit {
                    plain_literal_limit,
                    ..
                } => *plain_literal_limit == plan.plain_literal_limit,
                Self::PostSubmit { .. } | Self::UnsubmittedLiteral { .. } => true,
            }
        }
    }

    #[derive(Deserialize)]
    struct ProvenanceProfile {
        backend: CodingAgent,
        rules: Vec<ProvenanceRule>,
        capabilities: ProvenanceCapabilities,
    }

    #[derive(Deserialize)]
    struct ProvenanceRule {
        id: NonBlank,
        observation: ObservationKind,
        fixture_ids: Option<Ids>,
    }

    #[derive(Deserialize)]
    struct ProvenanceCapabilities {
        detection: Option<ProvenanceCapability>,
        composer: Option<ProvenanceCapability>,
        answers: Vec<Cited>,
    }

    #[derive(Deserialize)]
    struct ProvenanceCapability {
        rule_ids: Ids,
        fixture_ids: Ids,
        submission: Option<ProvenanceSubmissions>,
    }

    #[derive(Deserialize)]
    struct ProvenanceSubmissions {
        prompt: Option<ProvenanceSubmission>,
        exec: Option<ProvenanceSubmission>,
    }

    #[derive(Deserialize)]
    struct ProvenanceSubmission {
        fixture_ids: Ids,
        literal_framing: LiteralFraming,
        #[serde(deserialize_with = "Option::deserialize")]
        plain_literal_limit: Option<PlainLiteralLimit>,
    }

    impl ProvenanceSubmission {
        fn validate_evidence(
            &self,
            operation: ComposerOperation,
            fixtures: &[ProvenanceFixture],
        ) -> Result<()> {
            let mut coverage = BTreeSet::new();
            for id in &self.fixture_ids.0 {
                let fixture = fixtures
                    .iter()
                    .find(|fixture| &fixture.id.0 == id)
                    .ok_or_else(|| invalid("unknown fixture"))?;
                for record in fixture.composer_evidence.iter().flatten() {
                    let (observed_operation, multiplexer, kind) = record.identity();
                    if observed_operation != operation {
                        continue;
                    }
                    require(
                        record.matches_plan(self),
                        &format!(
                            "composer evidence {id} for {operation:?}/{multiplexer:?}/{kind:?} must match its selected plan"
                        ),
                    )?;
                    coverage.insert((multiplexer, kind));
                }
            }
            for multiplexer in [EvidenceMultiplexer::Tmux, EvidenceMultiplexer::Herdr] {
                for kind in [
                    ComposerEvidenceKind::Framing,
                    ComposerEvidenceKind::PlainLimit,
                    ComposerEvidenceKind::PostSubmit,
                    ComposerEvidenceKind::UnsubmittedLiteral,
                ] {
                    require(
                        coverage.contains(&(multiplexer, kind)),
                        &format!(
                            "composer {operation:?}/{multiplexer:?}/{kind:?} evidence required"
                        ),
                    )?;
                }
            }
            Ok(())
        }
    }

    #[derive(Deserialize)]
    struct Cited {
        fixture_ids: Ids,
    }

    pub(crate) fn validate_provenance(definitions: &Value) -> Result<()> {
        let raw = ProvenanceCatalog::deserialize(definitions)
            .map_err(|cause| invalid(cause.to_string()))?;
        let mut ids = BTreeSet::new();
        for fixture in &raw.fixtures {
            unique(&mut ids, &fixture.id.0)?;
            require(
                lower_hex_bytes(&fixture.sha256.0, 32).is_some_and(|hash| hash.len() == 32),
                "lowercase source digest required",
            )?;
            if let Some(evidence) = &fixture.composer_evidence {
                require(
                    !evidence.is_empty(),
                    "present composer evidence must be nonempty",
                )?;
                let mut claims = BTreeSet::new();
                for record in evidence {
                    require(
                        claims.insert(record.identity()),
                        "duplicate operation/multiplexer/evidence kind",
                    )?;
                }
            }
        }
        for profile in &raw.profiles {
            let cite = |cited: &Ids| -> Result<()> {
                require(!cited.0.is_empty(), "capability provenance required")?;
                for id in &cited.0 {
                    let fixture = raw
                        .fixtures
                        .iter()
                        .find(|fixture| &fixture.id.0 == id)
                        .ok_or_else(|| invalid("unknown fixture"))?;
                    require(
                        fixture.backend == profile.backend,
                        "fixture backend differs from the profile backend",
                    )?;
                }
                Ok(())
            };
            for cited in profile
                .rules
                .iter()
                .filter_map(|rule| rule.fixture_ids.as_ref())
            {
                cite(cited)?;
                require(
                    cited.0.iter().all(|id| {
                        raw.fixtures.iter().any(|fixture| {
                            &fixture.id.0 == id && fixture.kind == FixtureKind::Native
                        })
                    }),
                    "continuation cites native fixtures",
                )?;
            }
            let capabilities = &profile.capabilities;
            if let Some(composer) = &capabilities.composer
                && composer.submission.is_some()
            {
                require(
                    profile.rules.iter().any(|rule| {
                        rule.observation == ObservationKind::ComposerEmpty
                            && composer.rule_ids.0.contains(&rule.id.0)
                    }),
                    "executable composer plans require empty-composer observation",
                )?;
            }
            for capability in [&capabilities.detection, &capabilities.composer]
                .into_iter()
                .flatten()
            {
                cite(&capability.fixture_ids)?;
                if let Some(submission) = &capability.submission {
                    for (operation, plan) in [
                        (ComposerOperation::Prompt, submission.prompt.as_ref()),
                        (ComposerOperation::Exec, submission.exec.as_ref()),
                    ] {
                        if let Some(plan) = plan {
                            cite(&plan.fixture_ids)?;
                            plan.validate_evidence(operation, &raw.fixtures)?;
                        }
                    }
                }
            }
            for answer in &capabilities.answers {
                cite(&answer.fixture_ids)?;
            }
        }
        Ok(())
    }

    pub(crate) fn composer_evidence_records(
        operation: &str,
        multiplexer: &str,
        framing: &Value,
        limit: &Value,
    ) -> [Value; 4] {
        [
            json!({"operation":operation,"multiplexer":multiplexer,"kind":"framing","literal_framing":framing}),
            json!({"operation":operation,"multiplexer":multiplexer,"kind":"plain_limit","plain_literal_limit":limit}),
            json!({"operation":operation,"multiplexer":multiplexer,"kind":"post_submit"}),
            json!({"operation":operation,"multiplexer":multiplexer,"kind":"unsubmitted_literal"}),
        ]
    }

    pub(crate) fn with_synthetic_composer_evidence(definitions: &Value) -> Result<Value> {
        let mut decorated = definitions.clone();
        let fixtures = decorated["fixtures"]
            .as_array_mut()
            .ok_or_else(|| invalid("fixture registry array required"))?;
        let mut claims = std::collections::BTreeMap::<String, Vec<Value>>::new();
        for profile in definitions["profiles"].as_array().expect("parsed profiles") {
            let submission = &profile["capabilities"]["composer"]["submission"];
            for operation in ["prompt", "exec"] {
                let plan = &submission[operation];
                if plan.is_null() {
                    continue;
                }
                let cited: Ids = serde_json::from_value(plan["fixture_ids"].clone())
                    .map_err(|cause| invalid(cause.to_string()))?;
                for id in cited.0 {
                    let fixture = fixtures
                        .iter()
                        .find(|fixture| fixture["id"] == id)
                        .ok_or_else(|| invalid("unknown fixture"))?;
                    if fixture["kind"] != "synthetic" || fixture.get("composer_evidence").is_some()
                    {
                        continue;
                    }
                    for multiplexer in ["tmux", "herdr"] {
                        for record in composer_evidence_records(
                            operation,
                            multiplexer,
                            plan.get("literal_framing").expect("parsed literal framing"),
                            plan.get("plain_literal_limit").expect("parsed plain limit"),
                        ) {
                            let records = claims.entry(id.clone()).or_default();
                            if let Some(existing) = records.iter().find(|existing| {
                                existing["operation"] == record["operation"]
                                    && existing["multiplexer"] == record["multiplexer"]
                                    && existing["kind"] == record["kind"]
                            }) {
                                require(existing == &record, "synthetic evidence conflicts")?;
                            } else {
                                records.push(record);
                            }
                        }
                    }
                }
            }
        }
        for (id, records) in claims {
            let fixture = fixtures
                .iter_mut()
                .find(|fixture| fixture["id"] == id)
                .expect("a cited synthetic fixture remains in the registry");
            fixture["composer_evidence"] = json!(records);
        }
        Ok(decorated)
    }

    /// A test catalog: production loading followed by the provenance build guard.
    pub(crate) fn load(definitions: &Value) -> Result<ProfileCatalog> {
        let catalog = ProfileCatalog::from_definitions(definitions)?;
        validate_provenance(&with_synthetic_composer_evidence(definitions)?)?;
        Ok(catalog)
    }

    impl ProfileCatalog {
        pub(crate) fn from_bundled_definitions(
            definitions: &Value,
            embedded: &[(&str, &[u8])],
        ) -> Result<Self> {
            let catalog = load(definitions)?;
            verify_bundled_fixtures(array(&definitions["fixtures"])?, embedded)?;
            let catalog = Self {
                capture_origin: "bundled_unknown",
                ..catalog
            };
            Ok(catalog)
        }
    }

    pub(crate) fn fixture_at(id: &str, version: &str) -> Value {
        let mut fixture = synthetic_definitions()["fixtures"][0].clone();
        fixture["id"] = json!(id);
        fixture["native_version"] = json!(version);
        fixture
    }

    /// A catalog holding one `backend` profile per `(backend, label)` entry, each citing one
    /// fixture for detection, composer observation, both composer operations and an allow answer;
    /// the label names that fixture and answer rule.
    pub(crate) fn single_profiles(profiles: &[(CodingAgent, &str)]) -> Value {
        let base = synthetic_definitions();
        let composer = simple_definition("composer_empty", true)["profiles"][0]["rules"][0].clone();
        let mut value = base.clone();
        value["fixtures"] = json!([]);
        value["profiles"] = json!([]);
        for (backend, name) in profiles {
            let id = format!("{FIXTURE}-{}-{name}", backend.name());
            let mut fixture = fixture_at(&id, VERSION);
            fixture["backend"] = json!(backend.name());
            value["fixtures"].as_array_mut().unwrap().push(fixture);
            let mut profile = base["profiles"][0].clone();
            profile["backend"] = json!(backend.name());
            profile["rules"]
                .as_array_mut()
                .unwrap()
                .push(composer.clone());
            let capabilities = &mut profile["capabilities"];
            capabilities["detection"]["fixture_ids"] = json!([id]);
            capabilities["composer"] = json!({
                "rule_ids":["synthetic-composer_empty"],"fixture_ids":[id],
                "submission":{
                    "prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":[id]},
                    "exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":[id]}
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

    pub(crate) fn bundled_file(id: &str) -> (String, Value) {
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

    pub(crate) fn bundled_definitions() -> Value {
        serde_json::from_str(BUNDLED_DEFINITIONS).unwrap()
    }

    pub(crate) fn synthetic_definitions() -> Value {
        json!({
            "schema_version":1,
            "fixtures":[{"id":"synthetic-permission-1","kind":"synthetic","backend":"codex","native_version":"0.0.1-synthetic","rows":24,"cols":80,"source_ref":"synthetic-permission-screen.json","sha256":"689868668a9fa97d9efca75cda3f2cc0a87fde25884a5cb36aab3163b8abd773"}],
            "profiles":[{"backend":"codex",
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
                    ]},"verified_noninteraction":false}],
                "capabilities":{"detection":{"rule_ids":["synthetic-permission"],"fixture_ids":["synthetic-permission-1"]},"composer":null,"answers":[]}}]
        })
    }

    pub(crate) fn synthetic_catalog() -> ProfileCatalog {
        load(&synthetic_definitions()).unwrap()
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
                ]},
                "verified_noninteraction":false
            },
            {
                "id":GATED_REASON_RULE,"priority":100,"observation":"permission_continuation",
                "region":{"screen":"either","start_pattern":"^Synthetic reject reason$","end_pattern":"^Confirm: enter$","max_rows":4,"cursor":"any"},
                "matchers":[
                    {"id":"heading","scope":"line","pattern":"^Synthetic reject reason$"},
                    {"id":"empty","scope":"line","pattern":"^Reason: \\(empty\\)$"}
                ],
                "extract":{"operation":null,"choices":[]},
                "verified_noninteraction":false,"continues_rule_id":GATED_RULE,"continues_choice_id":"deny-once",
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
                {"id":"later","label":{"matcher_id":"later","capture":"label"},"scope":"persistent"}
            ]},
            "verified_noninteraction":false
        }));
        rules.push(json!({
            "id":QUESTION_NOTICE_RULE,"priority":100,"observation":"question",
            "region":{"screen":"either","start_pattern":"^Synthetic notice$","end_pattern":"^Synthetic notice$","max_rows":1,"cursor":"any"},
            "matchers":[{"id":"notice","scope":"line","pattern":"^Synthetic notice$"}],
            "extract":{"operation":null,"choices":[]},
            "verified_noninteraction":false
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
        rule["extract"] = json!({"operation":null,"choices":[]});
        rule["verified_noninteraction"] = json!(verified);
        let declaration = json!({"rule_ids":[format!("synthetic-{kind}")],"fixture_ids":[FIXTURE]});
        value["profiles"][0]["capabilities"] = if kind.starts_with("composer_") {
            let mut composer = declaration;
            composer["submission"] = json!({
                "prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":[FIXTURE]},
                "exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":[FIXTURE]}
            });
            json!({"detection":null,"composer":composer,"answers":[]})
        } else {
            json!({"detection":declaration,"composer":null,"answers":[]})
        };
        value
    }

    use crate::broker::runtime::Coverage;

    use serde_json::Value;

    pub(crate) const EMPTY: &str = "synthetic-composer_empty";
    pub(crate) const PROMPT_FIXTURE: &str = "synthetic-prompt-plan";
    pub(crate) const EXEC_FIXTURE: &str = "synthetic-exec-plan";

    /// A composer catalog whose submission declares both operations; an absent `prompt` or
    /// `exec` takes a placeholder submit key, and [`composer_catalog`] removes its plan.
    pub(crate) fn definition(prompt: Option<&str>, exec: Option<&str>) -> Value {
        let mut value = simple_definition("composer_empty", true);
        for id in [PROMPT_FIXTURE, EXEC_FIXTURE] {
            let mut fixture = value["fixtures"][0].clone();
            fixture["id"] = json!(id);
            value["fixtures"].as_array_mut().unwrap().push(fixture);
        }
        let prompt = json!({"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":prompt.unwrap_or("0d"),"fixture_ids":[PROMPT_FIXTURE]});
        let exec = json!({"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":exec.unwrap_or("0d"),"fixture_ids":[EXEC_FIXTURE]});
        value["profiles"][0]["capabilities"]["composer"] = json!({
            "rule_ids":[EMPTY],"fixture_ids":[FIXTURE],
            "submission":{"prompt":prompt,"exec":exec}
        });
        value
    }

    /// The [`definition`] catalog with the plan of each absent operation removed.
    pub(crate) fn composer_catalog(prompt: Option<&str>, exec: Option<&str>) -> ProfileCatalog {
        let mut catalog = load(&definition(prompt, exec)).unwrap();
        if prompt.is_none() {
            catalog = catalog.without_submission(CodingAgent::Codex, ComposerOperation::Prompt);
        }
        if exec.is_none() {
            catalog = catalog.without_submission(CodingAgent::Codex, ComposerOperation::Exec);
        }
        catalog
    }

    pub(crate) fn operation_name(operation: ComposerOperation) -> &'static str {
        match operation {
            ComposerOperation::Prompt => "prompt",
            ComposerOperation::Exec => "exec",
        }
    }

    pub(crate) fn plan(catalog: &ProfileCatalog, operation: ComposerOperation) -> ComposerPlan {
        catalog
            .composer_plan(CodingAgent::Codex, operation)
            .expect("valid selected operation has an executable plan")
    }

    /// The coverage the backend's profile declares per capability.
    pub(crate) struct Capabilities {
        pub(crate) detection: Coverage,
        pub(crate) composer: Coverage,
        pub(crate) prompt_submission: Coverage,
        pub(crate) native_exec_submission: Coverage,
        pub(crate) answer_decisions: Vec<PermissionDecision>,
    }

    impl ProfileCatalog {
        pub(crate) fn capabilities(&self, backend: CodingAgent) -> Capabilities {
            let profile = self.profile(backend);
            let tested = |declared: bool| {
                if declared {
                    Coverage::Tested
                } else {
                    Coverage::Unknown
                }
            };
            let submission = |operation: ComposerOperation| {
                tested(profile.is_some_and(|p| p.submissions[operation.index()].is_some()))
            };
            let mut answer_decisions = Vec::new();
            for answer in profile.iter().flat_map(|p| &p.answers) {
                if let Some(decision) = answer.key.decision()
                    && !answer_decisions.contains(&decision)
                {
                    answer_decisions.push(decision);
                }
            }
            Capabilities {
                detection: self.detection_coverage(backend),
                composer: tested(profile.is_some_and(|p| p.composer.is_some())),
                prompt_submission: submission(ComposerOperation::Prompt),
                native_exec_submission: submission(ComposerOperation::Exec),
                answer_decisions,
            }
        }
    }

    pub(crate) fn capabilities(
        catalog: &ProfileCatalog,
        backend: CodingAgent,
        observation: Coverage,
        operations: (Coverage, Coverage),
    ) {
        let actual = catalog.capabilities(backend);
        assert_eq!(actual.composer, observation);
        assert_eq!(actual.prompt_submission, operations.0);
        assert_eq!(actual.native_exec_submission, operations.1);
    }

    pub(crate) fn reject(
        good: &Value,
        bad: &Value,
        operation: ComposerOperation,
        label: &str,
    ) -> CafleetError {
        let catalog = crate::coding_agent::catalog::test_support::load(good).unwrap();
        let _ = plan(&catalog, operation)
            .encode("same-operation positive control")
            .unwrap();
        match crate::coding_agent::catalog::test_support::load(bad) {
            Err(error) => error,
            Ok(_) => panic!("invalid executable definition rejected: {label}"),
        }
    }

    pub(crate) fn expected(operation: ComposerOperation, literal: &str, submit: &[u8]) -> Vec<u8> {
        let mut bytes = operation.prefix().to_vec();
        bytes.extend_from_slice(literal.as_bytes());
        bytes.extend_from_slice(submit);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime::Coverage;
    use crate::broker::runtime::InteractionClass;
    use crate::coding_agent::catalog::test_support::synthetic_definitions as definitions;
    use crate::coding_agent::catalog::test_support::*;

    use crate::coding_agent::catalog::*;
    use crate::coding_agent::detector::test_support::*;
    use crate::runtime::*;
    use serde_json::Value;
    use serde_json::json;
    use std::collections::BTreeSet;

    mod verified_composer {
        use crate::coding_agent::catalog::tests::*;

        #[test]
        fn verified_composer_requires_an_executable_plan_for_the_selected_operation() {
            for (prompt, exec) in [(true, true), (true, false), (false, true), (false, false)] {
                let catalog = composer_catalog(prompt.then_some("0d"), exec.then_some("0d"));
                assert_eq!(
                    catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Prompt),
                    prompt
                );
                assert_eq!(
                    catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Exec),
                    exec
                );
            }
        }

        #[test]
        fn verified_composer_requires_a_selected_composer_declaration() {
            let catalog = load(&definitions()).unwrap();
            assert!(
                catalog
                    .composer_plan(CodingAgent::Codex, ComposerOperation::Prompt)
                    .is_none()
            );
            assert!(!catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Prompt));
            assert!(!catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Exec));
        }

        #[test]
        fn verified_composer_uses_the_recorded_backends_profile() {
            let catalog = composer_catalog(Some("0d"), Some("0d"));
            assert!(catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Prompt));
            for &backend in CodingAgent::all() {
                if backend != CodingAgent::Codex {
                    assert!(
                        !catalog.verified_composer(backend, ComposerOperation::Prompt),
                        "{backend:?}"
                    );
                }
            }
            let absent = load(&simple_definition("permission_request", false)).unwrap();
            assert!(!absent.verified_composer(CodingAgent::Codex, ComposerOperation::Prompt));
        }

        #[test]
        fn verified_composer_follows_the_backends_single_profile_per_operation() {
            let definition = single_profiles(&[
                (CodingAgent::Codex, "codex"),
                (CodingAgent::Claude, "claude"),
            ]);
            let mut catalog = load(&definition).unwrap();
            let claude = catalog
                .profiles
                .iter_mut()
                .find(|profile| profile.backend == CodingAgent::Claude)
                .expect("the synthetic claude profile exists");
            claude.submissions[ComposerOperation::Exec.index()] = None;
            assert!(catalog.verified_composer(CodingAgent::Codex, ComposerOperation::Exec));
            assert!(!catalog.verified_composer(CodingAgent::Claude, ComposerOperation::Exec));
            assert!(catalog.verified_composer(CodingAgent::Claude, ComposerOperation::Prompt));
        }
    }

    mod composer_reliability {
        use crate::coding_agent::catalog::tests::*;

        fn framed_definition(framing: &str, limit: Value) -> Value {
            let mut value = definition(Some("0d"), Some("0d"));
            for operation in ["prompt", "exec"] {
                let plan =
                    &mut value["profiles"][0]["capabilities"]["composer"]["submission"][operation];
                plan["literal_framing"] = json!(framing);
                plan["plain_literal_limit"] = limit.clone();
            }
            value
        }

        #[test]
        fn byte_plain_limit_diagnostics_include_the_prefix_and_measured_maximum_for_both_refusals()
        {
            for (operation, literal, size, maximum) in [
                (
                    ComposerOperation::Prompt,
                    format!("{}é", "あ".repeat(341)),
                    1025,
                    1024,
                ),
                (ComposerOperation::Exec, "あ".repeat(333), 1001, 1000),
            ] {
                for (framing, cause) in [
                    ("plain", "the codex profile has no bracketed-paste delivery"),
                    (
                        "bracketed_paste",
                        "the native CLI has not enabled bracketed paste",
                    ),
                ] {
                    let catalog = load(&framed_definition(
                        framing,
                        json!({"unit":"bytes","max":maximum}),
                    ))
                    .unwrap();
                    let plan = plan(&catalog, operation);
                    assert!(!plan.plain_literal_allowed(&literal));
                    assert_eq!(
                        plan.plain_limit_refusal(&literal, "codex"),
                        format!(
                            "the input is {size} bytes, above the codex plain-input limit of {maximum} bytes, and {cause}"
                        ),
                    );
                }
            }
        }

        #[test]
        fn scalar_plain_limit_diagnostics_include_the_prefix_and_use_characters_for_both_refusals()
        {
            for (operation, literal) in [
                (ComposerOperation::Prompt, "あ".repeat(1025)),
                (ComposerOperation::Exec, "あ".repeat(1023)),
            ] {
                for (framing, cause) in [
                    (
                        "plain",
                        "the hermes profile has no bracketed-paste delivery",
                    ),
                    (
                        "bracketed_paste",
                        "the native CLI has not enabled bracketed paste",
                    ),
                ] {
                    let catalog = load(&framed_definition(
                        framing,
                        json!({"unit":"scalars","max":1024}),
                    ))
                    .unwrap();
                    let plan = plan(&catalog, operation);
                    assert!(!plan.plain_literal_allowed(&literal));
                    assert_eq!(
                        plan.plain_limit_refusal(&literal, "hermes"),
                        format!(
                            "the input is 1025 characters, above the hermes plain-input limit of 1024 characters, and {cause}"
                        ),
                    );
                }
            }
        }

        #[test]
        fn composer_plans_require_explicit_framing_and_plain_limit_declarations() {
            for operation in ["prompt", "exec"] {
                for key in ["literal_framing", "plain_literal_limit"] {
                    let mut value = framed_definition("plain", Value::Null);
                    value["profiles"][0]["capabilities"]["composer"]["submission"][operation]
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                    assert!(load(&value).is_err(), "{operation}: {key}");
                }
            }
        }

        #[test]
        fn composer_plans_accept_plain_or_bracketed_framing_and_positive_byte_or_scalar_limits() {
            for framing in ["plain", "bracketed_paste"] {
                for limit in [
                    Value::Null,
                    json!({"unit":"bytes","max":1}),
                    json!({"unit":"scalars","max":1024}),
                ] {
                    assert!(
                        load(&framed_definition(framing, limit.clone())).is_ok(),
                        "{framing}: {limit}"
                    );
                }
            }
        }

        #[test]
        fn composer_plans_reject_unknown_framing_and_malformed_plain_limits() {
            for framing in [
                Value::Null,
                json!(true),
                json!(1),
                json!(""),
                json!("paste"),
            ] {
                let mut value = framed_definition("plain", Value::Null);
                value["profiles"][0]["capabilities"]["composer"]["submission"]["prompt"]["literal_framing"] =
                    framing.clone();
                assert!(load(&value).is_err(), "{framing}");
            }
            for limit in [
                json!({}),
                json!({"unit":"bytes"}),
                json!({"max":1}),
                json!({"unit":"cells","max":1}),
                json!({"unit":"bytes","max":0}),
                json!({"unit":"scalars","max":-1}),
                json!({"unit":"bytes","max":1.5}),
                json!({"unit":"bytes","max":"1"}),
                json!({"unit":"bytes","max":true}),
                json!({"unit":"bytes","max":1,"extra":true}),
                json!(false),
                json!([]),
            ] {
                assert!(
                    load(&framed_definition("plain", limit.clone())).is_err(),
                    "{limit}"
                );
            }
        }

        #[test]
        fn bracketed_literal_stages_preserve_the_plain_operation_prefix_and_utf8_literal() {
            let catalog = load(&framed_definition(
                "bracketed_paste",
                json!({"unit":"bytes","max":1024}),
            ))
            .unwrap();
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let plan = plan(&catalog, operation);
                let [literal, submit] = plan
                    .stages("literal あ", LiteralFraming::BracketedPaste)
                    .unwrap();
                let mut expected = operation.prefix().to_vec();
                expected.extend_from_slice(b"\x1b[200~");
                expected.extend_from_slice("literal あ".as_bytes());
                expected.extend_from_slice(b"\x1b[201~");
                assert_eq!(literal, expected);
                assert_eq!(submit, b"\r");
            }
        }

        #[test]
        fn plain_literal_stages_omit_framing_and_keep_the_selected_submit_key() {
            let catalog = load(&framed_definition("bracketed_paste", Value::Null)).unwrap();
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let plan = plan(&catalog, operation);
                let [literal, submit] = plan.stages("literal あ", LiteralFraming::Plain).unwrap();
                let mut expected = operation.prefix().to_vec();
                expected.extend_from_slice("literal あ".as_bytes());
                assert_eq!(literal, expected);
                assert_eq!(submit, b"\r");
            }
        }

        #[test]
        fn bracketed_payload_limit_counts_both_markers_prefix_and_submit_bytes() {
            let catalog = load(&framed_definition("bracketed_paste", Value::Null)).unwrap();
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                let plan = plan(&catalog, operation);
                let maximum = 65536 - operation.prefix().len() - 12 - 1;
                let stages = plan
                    .stages(&"a".repeat(maximum), LiteralFraming::BracketedPaste)
                    .unwrap();
                assert_eq!(stages.iter().map(Vec::len).sum::<usize>(), 65536);
                assert!(
                    plan.stages(&"a".repeat(maximum + 1), LiteralFraming::BracketedPaste)
                        .is_err()
                );
            }
        }

        #[test]
        fn bundled_guard_requires_an_empty_composer_rule_for_executable_plans() {
            let mut value = bundled_definitions();
            let profile = value["profiles"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|profile| !profile["capabilities"]["composer"].is_null())
                .expect("a bundled executable composer profile");
            let ids = profile["capabilities"]["composer"]["rule_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            for rule in profile["rules"].as_array_mut().unwrap() {
                if ids
                    .iter()
                    .any(|id| rule["id"].as_str() == Some(id.as_str()))
                {
                    rule["observation"] = json!("composer_visible");
                }
            }
            assert!(validate_provenance(&value).is_err());
        }

        #[test]
        fn bundled_guard_requires_plan_fixture_citations_for_measured_delivery() {
            let mut value = bundled_definitions();
            let profile = value["profiles"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|profile| !profile["capabilities"]["composer"].is_null())
                .expect("a bundled executable composer profile");
            profile["capabilities"]["composer"]["submission"]["prompt"]["fixture_ids"] = json!([]);
            assert!(validate_provenance(&value).is_err());
        }
    }

    mod composer_evidence {
        use crate::coding_agent::catalog::tests::*;

        fn measured() -> Value {
            let mut value = definition(Some("0d"), Some("0d"));
            let unrelated = value["fixtures"][0]["id"].clone();
            let template = value["fixtures"][0].clone();
            let mut sequence = 0;
            for operation in ["prompt", "exec"] {
                let framing = if operation == "prompt" {
                    json!("bracketed_paste")
                } else {
                    json!("plain")
                };
                let limit = if operation == "prompt" {
                    json!({"unit":"bytes","max":1024})
                } else {
                    json!({"unit":"scalars","max":1024})
                };
                let plan =
                    &mut value["profiles"][0]["capabilities"]["composer"]["submission"][operation];
                plan["literal_framing"] = framing.clone();
                plan["plain_literal_limit"] = limit.clone();
                let mut ids = vec![unrelated.clone()];
                for multiplexer in ["tmux", "herdr"] {
                    for record in
                        composer_evidence_records(operation, multiplexer, &framing, &limit)
                    {
                        sequence += 1;
                        let id = format!("witness-{sequence}");
                        let mut fixture = template.clone();
                        fixture["id"] = json!(id);
                        fixture["kind"] = json!("native");
                        fixture["composer_evidence"] = json!([record]);
                        value["fixtures"].as_array_mut().unwrap().push(fixture);
                        ids.push(json!(id));
                    }
                }
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["fixture_ids"] =
                    json!(ids);
            }
            value
        }

        fn witness_mut<'a>(
            value: &'a mut Value,
            operation: &str,
            multiplexer: &str,
            kind: &str,
        ) -> &'a mut Value {
            value["fixtures"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|fixture| {
                    fixture.get("composer_evidence").is_some_and(|records| {
                        records.as_array().unwrap().iter().any(|record| {
                            record["operation"] == operation
                                && record["multiplexer"] == multiplexer
                                && record["kind"] == kind
                        })
                    })
                })
                .expect("the explicit operation/multiplexer/category witness exists")
        }

        fn missing_category(kind: &str) {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            for operation in ["prompt", "exec"] {
                for multiplexer in ["tmux", "herdr"] {
                    let mut value = baseline.clone();
                    let id = witness_mut(&mut value, operation, multiplexer, kind)["id"].clone();
                    let unrelated = value["fixtures"][0]["id"].clone();
                    let ids = value["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation]["fixture_ids"]
                        .as_array_mut()
                        .unwrap();
                    ids.retain(|cited| cited != &id);
                    assert!(ids.contains(&unrelated));
                    assert!(ids.len() > 1);
                    assert!(
                        validate_provenance(&value).is_err(),
                        "{operation}/{multiplexer}/{kind}"
                    );
                }
            }
        }

        #[test]
        fn complete_operation_and_multiplexer_evidence_accepts_unrelated_additional_citations() {
            validate_provenance(&measured()).unwrap();
        }

        #[test]
        fn every_witness_category_is_required_for_each_operation_on_each_multiplexer() {
            for kind in [
                "framing",
                "plain_limit",
                "post_submit",
                "unsubmitted_literal",
            ] {
                missing_category(kind);
            }
        }

        #[test]
        fn unrelated_valid_native_citations_cannot_establish_an_executable_plan() {
            let mut value = definition(Some("0d"), Some("0d"));
            for fixture in value["fixtures"].as_array_mut().unwrap() {
                fixture["kind"] = json!("native");
            }
            let unrelated = &mut value["fixtures"][0];
            let id = unrelated["id"].clone();
            for operation in ["prompt", "exec"] {
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["fixture_ids"] =
                    json!([id]);
            }
            ProfileCatalog::from_definitions(&value).unwrap();
            assert!(validate_provenance(&value).is_err());
        }

        #[test]
        fn another_operations_claim_cannot_replace_the_selected_operations_witness() {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            for multiplexer in ["tmux", "herdr"] {
                for kind in [
                    "framing",
                    "plain_limit",
                    "post_submit",
                    "unsubmitted_literal",
                ] {
                    let mut value = baseline.clone();
                    witness_mut(&mut value, "prompt", multiplexer, kind)["composer_evidence"][0]
                        ["operation"] = json!("exec");
                    assert!(validate_provenance(&value).is_err(), "{multiplexer}/{kind}");
                }
            }
        }

        #[test]
        fn another_multiplexers_claim_cannot_replace_the_selected_multiplexers_witness() {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            for operation in ["prompt", "exec"] {
                for kind in [
                    "framing",
                    "plain_limit",
                    "post_submit",
                    "unsubmitted_literal",
                ] {
                    let mut value = baseline.clone();
                    witness_mut(&mut value, operation, "tmux", kind)["composer_evidence"][0]["multiplexer"] =
                        json!("herdr");
                    assert!(validate_provenance(&value).is_err(), "{operation}/{kind}");
                }
            }
        }

        #[test]
        fn framing_evidence_matches_each_selected_plan_value() {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            for operation in ["prompt", "exec"] {
                for multiplexer in ["tmux", "herdr"] {
                    let mut value = baseline.clone();
                    witness_mut(&mut value, operation, multiplexer, "framing")["composer_evidence"]
                        [0]["literal_framing"] = if operation == "prompt" {
                        json!("plain")
                    } else {
                        json!("bracketed_paste")
                    };
                    assert!(validate_provenance(&value).is_err());
                }
            }
        }

        #[test]
        fn plain_limit_evidence_matches_the_unit_bound_and_null_declaration() {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            for operation in ["prompt", "exec"] {
                for multiplexer in ["tmux", "herdr"] {
                    let (unit, other_unit) = if operation == "prompt" {
                        ("bytes", "scalars")
                    } else {
                        ("scalars", "bytes")
                    };
                    for limit in [
                        json!({"unit":unit,"max":512}),
                        json!({"unit":other_unit,"max":1024}),
                        Value::Null,
                    ] {
                        let mut value = baseline.clone();
                        witness_mut(&mut value, operation, multiplexer, "plain_limit")["composer_evidence"]
                            [0]["plain_literal_limit"] = limit;
                        assert!(validate_provenance(&value).is_err());
                    }
                }
            }
        }

        #[test]
        fn additional_selected_operation_claims_must_match_despite_complete_valid_coverage() {
            let mut baseline = measured();
            baseline["fixtures"][0]["kind"] = json!("native");
            validate_provenance(&baseline).unwrap();
            for operation in ["prompt", "exec"] {
                let plan =
                    &baseline["profiles"][0]["capabilities"]["composer"]["submission"][operation];
                let framing = plan["literal_framing"].as_str().unwrap();
                let unit = plan["plain_literal_limit"]["unit"].as_str().unwrap();
                let other_framing = if framing == "plain" {
                    "bracketed_paste"
                } else {
                    "plain"
                };
                let other_unit = if unit == "bytes" { "scalars" } else { "bytes" };
                for multiplexer in ["tmux", "herdr"] {
                    let matching = composer_evidence_records(
                        operation,
                        multiplexer,
                        &plan["literal_framing"],
                        &plan["plain_literal_limit"],
                    );
                    let mut valid_extra = baseline.clone();
                    valid_extra["fixtures"][0]["composer_evidence"] = json!(matching);
                    validate_provenance(&valid_extra).unwrap();
                    let mut wrong_framing = matching[0].clone();
                    wrong_framing["literal_framing"] = json!(other_framing);
                    let mut mismatched = vec![wrong_framing];
                    for limit in [
                        json!({"unit":unit,"max":512}),
                        json!({"unit":other_unit,"max":1024}),
                        Value::Null,
                    ] {
                        let mut record = matching[1].clone();
                        record["plain_literal_limit"] = limit;
                        mismatched.push(record);
                    }
                    for record in mismatched {
                        let mut value = baseline.clone();
                        value["fixtures"][0]["composer_evidence"] = json!([record]);
                        assert!(
                            validate_provenance(&value).is_err(),
                            "the additional selected claim contradicts its plan: {record}"
                        );
                    }
                }
            }
        }

        fn unlimited() -> Value {
            let mut value = measured();
            for operation in ["prompt", "exec"] {
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["plain_literal_limit"] =
                    Value::Null;
                for multiplexer in ["tmux", "herdr"] {
                    witness_mut(&mut value, operation, multiplexer, "plain_limit")["composer_evidence"]
                        [0]["plain_literal_limit"] = Value::Null;
                }
            }
            value
        }

        #[test]
        fn explicitly_measured_null_plain_limits_are_valid() {
            validate_provenance(&unlimited()).unwrap();
        }

        #[test]
        fn one_fixture_can_carry_distinct_operation_multiplexer_and_category_claims() {
            let mut value = measured();
            let records = value["fixtures"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|fixture| fixture.get("composer_evidence"))
                .flat_map(|records| records.as_array().unwrap().iter().cloned())
                .collect::<Vec<_>>();
            let fixture = &mut value["fixtures"][0];
            fixture["kind"] = json!("native");
            fixture["composer_evidence"] = json!(records);
            let id = fixture["id"].clone();
            for operation in ["prompt", "exec"] {
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["fixture_ids"] =
                    json!([id]);
            }
            validate_provenance(&value).unwrap();
        }

        #[test]
        fn missing_plain_limit_key_is_rejected_even_when_the_plan_is_unlimited() {
            let mut value = unlimited();
            validate_provenance(&value).unwrap();
            value["fixtures"][0]["composer_evidence"] =
                json!([{"operation":"prompt","multiplexer":"tmux","kind":"plain_limit"}]);
            assert!(validate_provenance(&value).is_err());
        }

        #[test]
        fn evidence_records_have_strict_required_fields_and_value_domains() {
            let baseline = measured();
            validate_provenance(&baseline).unwrap();
            let framing = json!({"operation":"prompt","multiplexer":"tmux","kind":"framing","literal_framing":"bracketed_paste"});
            let mut malformed = Vec::new();
            for field in ["operation", "multiplexer", "kind", "literal_framing"] {
                let mut record = framing.clone();
                record.as_object_mut().unwrap().remove(field).unwrap();
                malformed.push(record);
            }
            for (field, value) in [
                ("operation", json!("preview")),
                ("multiplexer", json!("screen")),
                ("kind", json!("idle")),
                ("literal_framing", json!("automatic")),
                ("unexpected", json!(true)),
            ] {
                let mut record = framing.clone();
                record[field] = value;
                malformed.push(record);
            }
            for limit in [
                json!({"unit":"bytes","max":0}),
                json!({"unit":"characters","max":1}),
                json!({"unit":"bytes","max":1,"extra":true}),
            ] {
                malformed.push(json!({"operation":"prompt","multiplexer":"tmux","kind":"plain_limit","plain_literal_limit":limit}));
            }
            for kind in ["post_submit", "unsubmitted_literal"] {
                malformed.push(json!({"operation":"prompt","multiplexer":"tmux","kind":kind,"literal_framing":"plain"}));
            }
            for record in malformed {
                let mut value = baseline.clone();
                value["fixtures"][0]["composer_evidence"] = json!([record]);
                assert!(validate_provenance(&value).is_err(), "{record}");
            }
        }

        #[test]
        fn duplicate_operation_multiplexer_category_records_on_one_fixture_are_rejected() {
            let mut value = measured();
            validate_provenance(&value).unwrap();
            let fixture = witness_mut(&mut value, "prompt", "tmux", "post_submit");
            let duplicate = fixture["composer_evidence"][0].clone();
            fixture["composer_evidence"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
            assert!(validate_provenance(&value).is_err());
        }

        #[test]
        fn a_present_empty_evidence_array_is_rejected_without_affecting_other_valid_citations() {
            let mut value = measured();
            validate_provenance(&value).unwrap();
            value["fixtures"][0]["composer_evidence"] = json!([]);
            assert!(validate_provenance(&value).is_err());
        }

        #[test]
        fn synthetic_test_helper_populates_absent_evidence_for_dynamic_plan_values() {
            let mut value = definition(Some("0d"), Some("0d"));
            for operation in ["prompt", "exec"] {
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["literal_framing"] =
                    json!("bracketed_paste");
                value["profiles"][0]["capabilities"]["composer"]["submission"][operation]["plain_literal_limit"] =
                    json!({"unit":"bytes","max":3});
            }
            validate_provenance(&with_synthetic_composer_evidence(&value).unwrap()).unwrap();
        }

        #[test]
        fn synthetic_test_helper_preserves_explicit_and_native_evidence_gaps() {
            let mut baseline = measured();
            baseline["fixtures"][0]["kind"] = json!("native");
            validate_provenance(&baseline).unwrap();
            for explicit in [false, true] {
                let mut value = baseline.clone();
                let fixture = witness_mut(&mut value, "prompt", "tmux", "post_submit");
                if explicit {
                    fixture["kind"] = json!("synthetic");
                    fixture["composer_evidence"] =
                        json!([{"operation":"exec","multiplexer":"tmux","kind":"post_submit"}]);
                } else {
                    fixture
                        .as_object_mut()
                        .unwrap()
                        .remove("composer_evidence")
                        .unwrap();
                }
                assert!(load(&value).is_err());
            }
        }
    }

    mod panel_separator {
        use crate::coding_agent::catalog::tests::*;

        fn with_separator(pointer: &str, separator: Value) -> Value {
            let mut value = crate::coding_agent::catalog::test_support::synthetic_definitions();
            value.pointer_mut(pointer).unwrap()["region"]["panel_separator"] = separator;
            value
        }

        #[test]
        fn a_region_loads_with_one_non_whitespace_panel_separator_glyph() {
            let value = with_separator("/profiles/0/rules/0", json!("│"));
            crate::coding_agent::catalog::test_support::load(&value).unwrap();
            assert!(
                crate::coding_agent::catalog::test_support::load(&question_fixture_definition())
                    .is_ok(),
                "panel_separator stays optional"
            );
        }

        #[test]
        fn a_panel_separator_that_is_not_one_non_whitespace_glyph_is_refused() {
            for separator in [json!(1), json!(""), json!(" "), json!("││")] {
                let value = with_separator("/profiles/0/rules/0", separator.clone());
                assert!(
                    crate::coding_agent::catalog::test_support::load(&value).is_err(),
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
            assert!(
                crate::coding_agent::catalog::test_support::load(&question_fixture_definition())
                    .is_ok()
            );
            match crate::coding_agent::catalog::test_support::load(value) {
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
            catalog.answer_plan(CodingAgent::Codex, request, &key)
        }
        fn choice(id: &str) -> AnswerKey {
            AnswerKey::Choice(id.into())
        }

        #[test]
        fn a_choice_answer_resolves_by_its_question_rule_and_choice_key() {
            let catalog =
                crate::coding_agent::catalog::test_support::load(&question_fixture_definition())
                    .unwrap();
            let decline = plan(&catalog, QUESTION_RULE, choice("decline")).expect("decline plan");
            let capability = decline.capability();
            assert_eq!(capability.key(), &choice("decline"));
            assert_eq!(capability.backend(), CodingAgent::Codex);
            assert_eq!(capability.request_rule_id(), QUESTION_RULE);
            assert_eq!(capability.answer_rule_id(), QUESTION_DECLINE_PLAN);
            assert_eq!(decline.choice_id(), "decline");
            assert_eq!(decline.focus_matcher_id(), "focus");
            let stages = decline
                .stages()
                .iter()
                .map(|stage| {
                    (
                        stage.bytes().to_vec(),
                        stage.required_focus_choice_id().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                stages,
                [
                    (vec![0x0e], Some("accept".to_owned())),
                    (vec![0x0d], Some("decline".to_owned())),
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
            for scope in ["single_request", "persistent"] {
                let value = changed(|value| {
                    value.pointer_mut(QUESTION).unwrap()["extract"]["choices"][2]["scope"] =
                        json!(scope);
                });
                let catalog = crate::coding_agent::catalog::test_support::load(&value)
                    .unwrap_or_else(|cause| panic!("{scope}: {}", cause.message()));
                assert!(
                    plan(&catalog, QUESTION_RULE, choice("later")).is_some(),
                    "{scope}"
                );
            }
        }

        #[test]
        fn an_answer_outside_its_form_scope_or_choice_set_is_refused() {
            type RefusalCase = (&'static str, fn(&mut Value), &'static str);
            let cases: [RefusalCase; 5] = [
                (
                    "a choice answer on a permission rule",
                    |value| {
                        answers(value).push(choice_answer(
                            RULE,
                            "synthetic-choice-on-permission",
                            "deny-once",
                            json!([{"bytes_hex":"6e","required_rule_id":RULE,"required_choice_id":"deny-once"}]),
                        ));
                    },
                    "invalid native profile: choice answer requires question form",
                ),
                (
                    "a decision answer on a question rule",
                    |value| {
                        let mut entry = answers(value)[1].clone();
                        entry["decision"] = json!("allow");
                        entry["answer_rule_id"] = json!("synthetic-decision-on-question");
                        entry["stages"] = json!([question_stage("33", "decline", None)]);
                        answers(value).push(entry);
                    },
                    "invalid native profile: answer requires permission form",
                ),
                (
                    "a decision answer on a persistent choice",
                    |value| {
                        value["profiles"][0]["rules"][0]["extract"]["choices"][0]["scope"] =
                            json!("persistent");
                    },
                    "invalid native profile: answer requires single-request scope",
                ),
                (
                    "a choice answer naming no rule choice",
                    |value| {
                        answers(value)[2]["choice_id"] = json!("never");
                        answers(value)[2]["stages"] = json!([question_stage("33", "never", None)]);
                    },
                    "invalid native profile: unknown answer choice",
                ),
                (
                    "a duplicate question rule and choice pair",
                    |value| {
                        let mut duplicate = answers(value)[2].clone();
                        duplicate["answer_rule_id"] = json!("synthetic-question-later-again");
                        answers(value).push(duplicate);
                    },
                    "invalid native profile: ",
                ),
            ];
            for (name, change, message) in cases {
                let refused = refusal(&changed(change));
                assert!(refused.starts_with(message), "{name}: {refused}");
            }
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
            for id in ["a", &"a".repeat(64)] {
                let catalog = crate::coding_agent::catalog::test_support::load(&renamed(id))
                    .unwrap_or_else(|cause| panic!("{id}: {}", cause.message()));
                assert!(plan(&catalog, QUESTION_RULE, choice(id)).is_some(), "{id}");
            }
            for id in ["Later", "-later", "ask--later", &"a".repeat(65)] {
                assert!(
                    crate::coding_agent::catalog::test_support::load(&renamed(id)).is_err(),
                    "{id} is outside the choice-id shape"
                );
            }
        }

        #[test]
        fn marker_focus_loads_and_gates_a_choice_plan() {
            let catalog =
                crate::coding_agent::catalog::test_support::load(&question_fixture_definition())
                    .unwrap();
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
            assert!(crate::coding_agent::catalog::test_support::load(&background).is_ok());
        }

        #[test]
        fn marker_focus_gates_a_decision_plan_as_background_focus_does() {
            let mut marker = gated_fixture_definition();
            marker["profiles"][0]["rules"][0]["matchers"][2]["focus_style"] = json!("marker");
            let catalog = crate::coding_agent::catalog::test_support::load(&marker).unwrap();
            let deny = catalog
                .answer_plan(
                    CodingAgent::Codex,
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
                crate::coding_agent::catalog::test_support::load(&value)
                    .unwrap_or_else(|cause| panic!("{require}: {}", cause.message()));
            }
            let permission = changed(|value| {
                value["profiles"][0]["rules"][0]["frame"] =
                    json!({"require_patterns":[],"reject_patterns":["^Live composer$"]});
            });
            assert!(crate::coding_agent::catalog::test_support::load(&permission).is_ok());
        }

        #[test]
        fn an_interaction_frame_requires_a_reject_pattern() {
            let value = changed(|value| {
                value.pointer_mut(QUESTION).unwrap()["frame"] =
                    json!({"require_patterns":[],"reject_patterns":[]});
            });
            assert_eq!(
                refusal(&value),
                "invalid native profile: interaction frame requires a reject pattern"
            );
        }
    }

    mod bundled_backend_files {
        use crate::coding_agent::catalog::ProfileCatalog;
        use crate::coding_agent::catalog::test_support::BUNDLED_FIXTURES;
        use crate::coding_agent::catalog::test_support::bundled_definitions;
        use crate::coding_agent::catalog::test_support::implicit_pattern_classes;
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
                crate::coding_agent::catalog::test_support::load(&self.definitions).unwrap_or_else(
                    |error| panic!("native test declarations are valid: {}", error.message()),
                );
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

        #[test]
        fn an_uncited_screen_in_an_unreferenced_file_is_rejected_before_referenced_files() {
            let fixture = NativeFixtureCatalog::new();
            let bytes = serde_json::to_vec(&fixture.file).unwrap();
            let extra = serde_json::to_vec(&json!({
                "schema_version":1,"backend":"claude","provenance":"native",
                "screens":{"unlisted-screen":screen("uncited file screen")}
            }))
            .unwrap();
            for extra_first in [true, false] {
                let mut embedded = vec![
                    (SOURCE, bytes.as_slice()),
                    ("native/claude.json", extra.as_slice()),
                ];
                if extra_first {
                    embedded.reverse();
                }
                assert_eq!(
                    usage_failure(fixture.load_embedded(&embedded)),
                    UNCITED,
                    "extra file first: {extra_first}"
                );
            }
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

        impl NativeFixtureCatalog {
            fn masked() -> Self {
                let mut fixture = Self::new();
                fixture.change_primary(|s| push_mask(s, 5, "reviewed_privacy"));
                fixture
            }

            fn change_primary(&mut self, change: impl FnOnce(&mut Value)) {
                let screen = &mut self.file["screens"][PRIMARY];
                change(screen);
                self.definitions["fixtures"][0]["sha256"] = json!(screen_digest(screen));
            }
        }

        fn failure_message(result: Result<ProfileCatalog, CafleetError>) -> Option<String> {
            result.err().map(|error| error.message().into_owned())
        }

        #[test]
        fn a_screen_outside_the_reviewed_shape_is_rejected() {
            for base in [NativeFixtureCatalog::new, NativeFixtureCatalog::masked] {
                assert_eq!(failure_message(base().load()), None);
                for (case, change) in SCREEN_CHANGES {
                    let mut fixture = base();
                    fixture.change_primary(change);
                    assert!(fixture.load().is_err(), "{case} must be rejected");
                }
                let mut pending_wrap = base();
                pending_wrap.change_primary(|s| s["cursor_col"] = s["cols"].clone());
                assert_eq!(
                    failure_message(pending_wrap.load()),
                    None,
                    "cursor_col equal to cols records the pending-wrap position"
                );
            }
            let mut flipped = NativeFixtureCatalog::new();
            let screen = &mut flipped.file["screens"][PRIMARY];
            screen["alternate_screen"] = json!(!screen["alternate_screen"].as_bool().unwrap());
            let failure = usage_failure(flipped.load());
            assert!(
                failure.starts_with(&format!(
                    "invalid native profile: bundled fixture digest mismatch for {PRIMARY}: "
                )),
                "{failure}"
            );
            let mut synthetic = NativeFixtureCatalog::new();
            synthetic.definitions["fixtures"][0]["kind"] = json!("synthetic");
            assert_eq!(
                usage_failure(synthetic.load()),
                "invalid native profile: bundled fixtures require native provenance"
            );
        }

        #[test]
        fn cell_backgrounds_outside_the_reviewed_shape_are_rejected() {
            let run = |row: u64, start: u64, end: u64, bg: &Value| json!({"row":row,"start_col":start,"end_col":end,"bg":bg});
            let rgb = json!({"kind":"rgb","r":1,"g":2,"b":3});
            let indexed = json!({"kind":"indexed","index":255});
            let load = |runs: Value| {
                let mut fixture = NativeFixtureCatalog::masked();
                fixture.change_primary(|s| s["cell_backgrounds"] = runs);
                fixture.load()
            };
            assert_eq!(
                failure_message(load(json!([
                    run(0, 0, 4, &rgb),
                    run(0, 4, 80, &indexed),
                    run(23, 0, 80, &rgb)
                ]))),
                None,
                "adjacent full-geometry runs are reviewed"
            );
            for (case, runs) in [
                ("not an array", json!({"row":0})),
                (
                    "run missing a field",
                    json!([{"row":0,"start_col":0,"bg":rgb}]),
                ),
                ("row below the geometry", json!([run(24, 0, 4, &rgb)])),
                ("start past the width", json!([run(0, 81, 82, &rgb)])),
                ("end past the width", json!([run(0, 0, 81, &rgb)])),
                ("empty run", json!([run(0, 4, 4, &rgb)])),
                ("masked row", json!([run(5, 0, 4, &rgb)])),
                (
                    "default colour",
                    json!([run(0, 0, 4, &json!({"kind":"default"}))]),
                ),
                (
                    "indexed colour out of range",
                    json!([run(0, 0, 4, &json!({"kind":"indexed","index":256}))]),
                ),
                (
                    "rgb channel out of range",
                    json!([run(0, 0, 4, &json!({"kind":"rgb","r":256,"g":0,"b":0}))]),
                ),
                (
                    "overlapping runs",
                    json!([run(0, 0, 6, &rgb), run(0, 5, 9, &rgb)]),
                ),
            ] {
                assert!(load(runs).is_err(), "{case} must be rejected");
            }
        }

        fn style_run(row: u64, start: u64, end: u64, faint: bool, foreground: &Value) -> Value {
            json!({"row":row,"start_col":start,"end_col":end,"faint":faint,"foreground":foreground})
        }

        #[test]
        fn cell_styles_outside_the_reviewed_shape_are_rejected() {
            let rgb = json!({"kind":"rgb","r":1,"g":2,"b":3});
            let indexed = json!({"kind":"indexed","index":8});
            let load = |runs: Value| {
                let mut fixture = NativeFixtureCatalog::masked();
                fixture.change_primary(|s| s["cell_styles"] = runs);
                fixture.load()
            };
            assert_eq!(
                failure_message(load(json!([
                    style_run(0, 0, 4, true, &Value::Null),
                    style_run(0, 4, 80, false, &indexed),
                    style_run(23, 0, 80, true, &rgb)
                ]))),
                None,
                "adjacent full-geometry faint, foreground and combined runs are reviewed"
            );
            let mut layered = NativeFixtureCatalog::masked();
            layered.change_primary(|s| {
                s["cell_backgrounds"] = json!([{"row":0,"start_col":0,"end_col":4,"bg":rgb}]);
                s["cell_styles"] = json!([style_run(0, 0, 4, true, &indexed)]);
            });
            assert_eq!(
                failure_message(layered.load()),
                None,
                "style runs may cover background runs"
            );
            for (case, runs) in [
                ("not an array", json!({"row":0})),
                (
                    "run missing a field",
                    json!([{"row":0,"start_col":0,"end_col":4,"faint":true}]),
                ),
                (
                    "run with an extra field",
                    json!([{"row":0,"start_col":0,"end_col":4,"faint":true,"foreground":null,"bold":true}]),
                ),
                (
                    "row below the geometry",
                    json!([style_run(24, 0, 4, true, &Value::Null)]),
                ),
                (
                    "start past the width",
                    json!([style_run(0, 81, 82, true, &Value::Null)]),
                ),
                (
                    "end past the width",
                    json!([style_run(0, 0, 81, true, &Value::Null)]),
                ),
                ("empty run", json!([style_run(0, 4, 4, true, &Value::Null)])),
                (
                    "masked row",
                    json!([style_run(5, 0, 4, true, &Value::Null)]),
                ),
                (
                    "default style",
                    json!([style_run(0, 0, 4, false, &Value::Null)]),
                ),
                (
                    "non-boolean faint",
                    json!([{"row":0,"start_col":0,"end_col":4,"faint":1,"foreground":null}]),
                ),
                (
                    "default foreground colour",
                    json!([style_run(0, 0, 4, false, &json!({"kind":"default"}))]),
                ),
                (
                    "indexed foreground out of range",
                    json!([style_run(
                        0,
                        0,
                        4,
                        false,
                        &json!({"kind":"indexed","index":256})
                    )]),
                ),
                (
                    "rgb foreground channel out of range",
                    json!([style_run(
                        0,
                        0,
                        4,
                        false,
                        &json!({"kind":"rgb","r":0,"g":256,"b":0})
                    )]),
                ),
                (
                    "overlapping runs",
                    json!([
                        style_run(0, 0, 6, true, &Value::Null),
                        style_run(0, 5, 9, false, &indexed)
                    ]),
                ),
            ] {
                assert!(load(runs).is_err(), "{case} must be rejected");
            }
        }

        #[test]
        fn fixture_replay_applies_each_style_run_and_keeps_its_background() {
            let mut fixture = screen("faint grey both plain");
            fixture["cell_backgrounds"] =
                json!([{"row":0,"start_col":11,"end_col":15,"bg":{"kind":"indexed","index":4}}]);
            fixture["cell_styles"] = json!([
                style_run(0, 0, 5, true, &Value::Null),
                style_run(0, 6, 10, false, &json!({"kind":"indexed","index":8})),
                style_run(0, 11, 15, true, &json!({"kind":"rgb","r":10,"g":20,"b":30})),
            ]);
            let replayed = crate::coding_agent::detector::test_support::replay_native_fixture(
                &crate::digest::canonical_json(&fixture),
                |_| {},
                true,
                None,
            );
            let cell = |col: u16| replayed.screen.cell(0, col).unwrap().clone();
            assert_eq!(
                replayed.screen.contents_between(0, 0, 0, 21),
                "faint grey both plain"
            );
            for col in 0..5 {
                assert!(cell(col).dim(), "column {col} is faint");
                assert_eq!(cell(col).fgcolor(), vt100::Color::Default);
            }
            for col in 6..10 {
                assert!(!cell(col).dim(), "column {col}");
                assert_eq!(cell(col).fgcolor(), vt100::Color::Idx(8));
            }
            for col in 11..15 {
                assert!(cell(col).dim(), "column {col}");
                assert_eq!(cell(col).fgcolor(), vt100::Color::Rgb(10, 20, 30));
                assert_eq!(cell(col).bgcolor(), vt100::Color::Idx(4));
            }
            for col in [5, 10, 15, 16, 20] {
                assert!(!cell(col).dim(), "column {col} keeps the default style");
                assert_eq!(cell(col).fgcolor(), vt100::Color::Default);
                assert_eq!(cell(col).bgcolor(), vt100::Color::Default);
            }
        }

        #[test]
        fn the_bundled_catalog_passes_its_fixture_build_guard() {
            assert_eq!(
                failure_message(ProfileCatalog::from_bundled_definitions(
                    &bundled_definitions(),
                    BUNDLED_FIXTURES
                )),
                None
            );
        }

        #[test]
        fn the_bundled_catalog_spells_every_pattern_class_explicitly() {
            assert_eq!(
                implicit_pattern_classes(&bundled_definitions()),
                Vec::<String>::new()
            );
        }

        /// The first bundled rule whose `suffix` holds a pattern string, as
        /// `(profile index, rule index)`.
        fn rule_with(definitions: &Value, suffix: &str) -> (usize, usize) {
            definitions["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .find_map(|(profile, value)| {
                    value["rules"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .position(|rule| rule.pointer(suffix).is_some_and(Value::is_string))
                        .map(|rule| (profile, rule))
                })
                .unwrap_or_else(|| panic!("a bundled rule carries {suffix}"))
        }

        fn with_pattern(suffix: &str, pattern: &str) -> (Value, String) {
            let mut definitions = bundled_definitions();
            let (profile, rule) = rule_with(&definitions, suffix);
            let value = &mut definitions["profiles"][profile];
            *value["rules"][rule].pointer_mut(suffix).unwrap() = json!(pattern);
            let location = format!(
                "{} rule {}",
                value["backend"].as_str().unwrap(),
                value["rules"][rule]["id"].as_str().unwrap()
            );
            (definitions, location)
        }

        #[test]
        fn the_explicit_class_guard_line_reads_backend_location_and_construct() {
            let mut value = definitions();
            value["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
                json!(r"^Operation:\s(?P<operation>.+)$");
            assert_eq!(
                implicit_pattern_classes(&value),
                vec!["codex rule synthetic-permission: `\\s`".to_owned()]
            );
        }

        const PATTERN_FIELDS: &[&str] = &[
            "/matchers/0/pattern",
            "/region/start_pattern",
            "/region/end_pattern",
            "/frame/require_patterns/0",
            "/frame/reject_patterns/0",
        ];

        #[test]
        fn the_explicit_class_guard_names_the_profile_rule_and_construct_in_every_pattern_field() {
            const CONSTRUCTS: &[&str] = &[
                r"\s", r"\S", r"\d", r"\D", r"\w", r"\W", r"\b", r"\B", r"\u{a0}",
            ];
            for field in PATTERN_FIELDS {
                for construct in CONSTRUCTS {
                    let (definitions, location) = with_pattern(field, &format!("x{construct}y"));
                    assert_eq!(
                        implicit_pattern_classes(&definitions),
                        vec![format!("{location}: `{construct}`")],
                        "{field} {construct}"
                    );
                }
            }
        }

        #[test]
        fn the_explicit_class_guard_reports_every_construct_of_a_pattern_in_order() {
            let (definitions, location) = with_pattern("/matchers/0/pattern", r"^\s+\d[\w,]$");
            assert_eq!(
                implicit_pattern_classes(&definitions),
                vec![
                    format!("{location}: `\\s`"),
                    format!("{location}: `\\d`"),
                    format!("{location}: `\\w`"),
                ]
            );
        }

        #[test]
        fn the_explicit_class_guard_admits_every_escape_of_the_pattern_syntax() {
            const ADMITTED: &[(&str, &str)] = &[
                ("ascii punctuation", r"\.\?\(\)\[\]\{\}\$\^\*\+\|\\\-\#\&\~"),
                ("control escapes", r"\t\n\r\f\v"),
                ("code points", r"\x41\x{a0}\x{3000}"),
                ("general category", r"\p{L}\p{Nd}"),
                ("text anchors", r"\Ax\z"),
                (
                    "explicit white space class",
                    r"[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]",
                ),
                ("explicit word boundary", r"settings(?:$|[^0-9A-Za-z_])"),
            ];
            for field in PATTERN_FIELDS {
                for (name, pattern) in ADMITTED {
                    let (definitions, _) = with_pattern(field, pattern);
                    assert_eq!(
                        implicit_pattern_classes(&definitions),
                        Vec::<String>::new(),
                        "{field} {name}"
                    );
                }
            }
        }

        #[test]
        fn the_explicit_class_guard_ignores_strings_the_decoder_does_not_compile_as_patterns() {
            let (mut definitions, _) = with_pattern("/matchers/0/pattern", "x");
            let (profile, rule) = rule_with(&definitions, "/matchers/0/pattern");
            definitions["profiles"][profile]["rules"][rule]["id"] = json!(r"rule-\s-\d");
            assert_eq!(implicit_pattern_classes(&definitions), Vec::<String>::new());
        }

        fn load_with_file(
            definitions: &Value,
            source: &str,
            file: &Value,
        ) -> Result<ProfileCatalog, CafleetError> {
            let bytes = serde_json::to_vec(file).unwrap();
            let embedded = BUNDLED_FIXTURES
                .iter()
                .map(|&(cited, original)| {
                    (
                        cited,
                        if cited == source {
                            bytes.as_slice()
                        } else {
                            original
                        },
                    )
                })
                .collect::<Vec<_>>();
            ProfileCatalog::from_bundled_definitions(definitions, &embedded)
        }

        #[test]
        fn each_embedded_backend_file_is_bound_to_its_catalog_identity_screens_and_bytes() {
            let definitions = bundled_definitions();
            for &(source, bytes) in BUNDLED_FIXTURES {
                let file: Value = serde_json::from_slice(bytes).unwrap();
                let other = if file["backend"] == "codex" {
                    "claude"
                } else {
                    "codex"
                };
                for (field, value) in [
                    ("schema_version", json!(2)),
                    ("provenance", json!("synthetic")),
                    ("backend", json!(other)),
                ] {
                    let mut changed = file.clone();
                    changed[field] = value;
                    assert_eq!(
                        usage_failure(load_with_file(&definitions, source, &changed)),
                        "invalid native profile: bundled fixture file identity differs from its catalog entry",
                        "{source}: {field}"
                    );
                }
                let mut missing = file.clone();
                let screens = missing["screens"].as_object_mut().unwrap();
                let first = screens.keys().next().unwrap().clone();
                screens.remove(&first);
                assert_eq!(
                    usage_failure(load_with_file(&definitions, source, &missing)),
                    "invalid native profile: bundled fixture screen is missing",
                    "{source}"
                );
                let absent = BUNDLED_FIXTURES
                    .iter()
                    .copied()
                    .filter(|&(cited, _)| cited != source)
                    .collect::<Vec<_>>();
                assert_eq!(
                    usage_failure(ProfileCatalog::from_bundled_definitions(
                        &definitions,
                        &absent
                    )),
                    "invalid native profile: bundled fixture bytes are not embedded",
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn permission_inspection_catalog_retains_exact_executable_plans_and_provenance() {
        let catalog =
            crate::coding_agent::catalog::test_support::load(&inspection_fixture_definition())
                .unwrap();
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
                    "synthetic-permission",
                    &AnswerKey::Decision(decision),
                )
                .unwrap();
            let capability = plan.capability();
            assert_eq!(capability.backend(), CodingAgent::Codex);
            assert_eq!(capability.request_rule_id(), "synthetic-permission");
            assert_eq!(capability.answer_rule_id(), rule);
            assert_eq!(capability.key(), &AnswerKey::Decision(decision));
            assert_eq!(plan.choice_id(), choice);
            assert_eq!(plan.focus_matcher_id(), "form");
            assert_eq!(plan.stages().len(), 2);
            assert_eq!(plan.stages()[0].bytes(), &[bytes]);
            assert_eq!(plan.stages()[1].bytes(), b"\r");
            for stage in plan.stages() {
                assert!(stage.gate().is_none());
            }
        }
        assert!(
            catalog
                .answer_plan(
                    CodingAgent::Claude,
                    "synthetic-permission",
                    &AnswerKey::Decision(PermissionDecision::Allow)
                )
                .is_none()
        );
        for (rule, decision) in [
            ("unknown", PermissionDecision::Allow),
            ("inspection-alternate-request", PermissionDecision::Deny),
        ] {
            assert!(
                catalog
                    .answer_plan(CodingAgent::Codex, rule, &AnswerKey::Decision(decision))
                    .is_none(),
                "{rule} {decision:?}"
            );
        }
    }

    #[test]
    fn permission_inspection_catalog_bounds_each_stage_and_complete_plan() {
        let mut value = inspection_fixture_definition();
        let mut stage = value["profiles"][0]["capabilities"]["answers"][0]["stages"][0].clone();
        stage["bytes_hex"] = json!("79".repeat(256));
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] =
            json!(vec![stage.clone(); 16]);
        let catalog = crate::coding_agent::catalog::test_support::load(&value).unwrap();
        let plan = catalog
            .answer_plan(
                CodingAgent::Codex,
                "synthetic-permission",
                &AnswerKey::Decision(PermissionDecision::Allow),
            )
            .unwrap();
        assert_eq!(
            plan.stages().iter().map(|s| s.bytes().len()).sum::<usize>(),
            4096
        );
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] = json!(vec![stage; 17]);
        assert!(crate::coding_agent::catalog::test_support::load(&value).is_err());
        value["profiles"][0]["capabilities"]["answers"][0]["stages"] = json!([{
            "bytes_hex":"79".repeat(257),"required_rule_id":"synthetic-permission","required_choice_id":"allow-once"
        }]);
        assert!(crate::coding_agent::catalog::test_support::load(&value).is_err());
    }

    #[test]
    fn permission_inspection_executable_plans_have_no_debug_or_serialized_representation() {
        assert_not_impl!(crate::coding_agent::AnswerPlan: std::fmt::Debug, serde::Serialize);
        assert_not_impl!(crate::coding_agent::AnswerStage: std::fmt::Debug, serde::Serialize);
    }
    fn invalid(change: impl FnOnce(&mut Value)) {
        assert!(crate::coding_agent::catalog::test_support::load(&definitions()).is_ok());
        let mut value = definitions();
        change(&mut value);
        assert!(
            crate::coding_agent::catalog::test_support::load(&value).is_err(),
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
    fn runtime_profile_capabilities_are_independent_and_backend_selected() {
        let catalog = crate::coding_agent::catalog::test_support::load(&definitions()).unwrap();
        let supported = catalog.capabilities(CodingAgent::Codex);
        assert_eq!(supported.detection, Coverage::Tested);
        assert_eq!(supported.composer, Coverage::Unknown);
        assert!(supported.answer_decisions.is_empty());
        for &backend in CodingAgent::all()
            .iter()
            .filter(|&&backend| backend != CodingAgent::Codex)
        {
            assert_eq!(catalog.capabilities(backend).detection, Coverage::Unknown);
        }
    }

    #[test]
    fn runtime_profile_answer_decisions_have_separate_request_and_backend_capabilities() {
        for (name, decision, other) in [
            ("allow", PermissionDecision::Allow, PermissionDecision::Deny),
            ("deny", PermissionDecision::Deny, PermissionDecision::Allow),
        ] {
            let value = with_answer(name);
            let catalog = crate::coding_agent::catalog::test_support::load(&value).unwrap();
            assert_eq!(
                catalog.capabilities(CodingAgent::Codex).answer_decisions,
                vec![decision]
            );
            assert_eq!(
                catalog
                    .answer_plan(CodingAgent::Codex, RULE, &AnswerKey::Decision(decision))
                    .unwrap()
                    .capability()
                    .answer_rule_id(),
                format!("synthetic-{name}-plan")
            );
            assert!(
                catalog
                    .answer_plan(CodingAgent::Codex, RULE, &AnswerKey::Decision(other))
                    .is_none()
            );
            assert!(
                catalog
                    .answer_plan(CodingAgent::Claude, RULE, &AnswerKey::Decision(decision))
                    .is_none()
            );
            assert!(
                catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        "another-form",
                        &AnswerKey::Decision(decision)
                    )
                    .is_none()
            );
        }
    }

    #[test]
    fn runtime_profile_root_schema_and_unknown_fields_fail_initialization() {
        for value in [
            Value::Null,
            json!([]),
            json!({}),
            json!({"schema_version":2,"fixtures":[],"profiles":[]}),
        ] {
            assert!(crate::coding_agent::catalog::test_support::load(&value).is_err());
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
            ("/fixtures/0/native_version", json!(" ")),
            ("/fixtures/0/native_version", json!([VERSION])),
            ("/fixtures/0/native_version", Value::Null),
            ("/profiles/0/rules/0/priority", json!(1.5)),
            ("/profiles/0/rules/0/observation", json!("approved")),
            (
                "/profiles/0/rules/0/verified_noninteraction",
                json!("false"),
            ),
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
    fn runtime_profile_duplicate_identity_and_a_second_backend_profile_fail() {
        invalid(|value| {
            let copy = value["fixtures"][0].clone();
            value["fixtures"].as_array_mut().unwrap().push(copy);
        });
        invalid(|value| {
            let copy = value["profiles"][0].clone();
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
        assert!(crate::coding_agent::catalog::test_support::load(&margined).is_ok());
        for margin in [json!(1), json!(""), json!("\n")] {
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
                crate::coding_agent::catalog::test_support::load(&value).is_ok(),
                "{margin_rows}"
            );
        }
        invalid(|value| {
            let operation = &mut value["profiles"][0]["rules"][0]["extract"]["operation"];
            operation["row_margin"] = json!(" ");
            operation["margin_rows"] = json!("first");
        });
        invalid(|value| {
            value["profiles"][0]["rules"][0]["extract"]["operation"]["margin_rows"] = json!("all");
        });
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
                crate::coding_agent::catalog::test_support::load(&value).is_ok(),
                "with margin: {with_margin}"
            );
        }
        for trailer in [json!(1), json!(""), json!(" █"), json!("█\n█")] {
            invalid(|value| {
                value["profiles"][0]["rules"][0]["extract"]["operation"]["row_trailer"] = trailer;
            });
        }
    }

    #[test]
    fn runtime_profile_capability_provenance_references_are_checked() {
        for (path, replacement) in [
            ("/fixtures/0/backend", json!("claude")),
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
        assert!(crate::coding_agent::catalog::test_support::load(&valid).is_ok());
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
                json!("persistent"),
            ),
            ("/profiles/0/rules/0/observation", json!("trust")),
        ] {
            let mut value = valid.clone();
            *value.pointer_mut(path).unwrap() = replacement;
            assert!(
                crate::coding_agent::catalog::test_support::load(&value).is_err(),
                "invalid answer relationship {path}"
            );
        }
    }

    #[test]
    fn runtime_profile_answer_objects_have_exact_fields_and_unique_plan_identities() {
        let valid = with_answer("allow");
        assert!(crate::coding_agent::catalog::test_support::load(&valid).is_ok());
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
            assert!(crate::coding_agent::catalog::test_support::load(&value).is_err());
        }
        let mut duplicate = valid.clone();
        duplicate["profiles"][0]["capabilities"]["answers"]
            .as_array_mut()
            .unwrap()
            .push(valid["profiles"][0]["capabilities"]["answers"][0].clone());
        assert!(crate::coding_agent::catalog::test_support::load(&duplicate).is_err());
    }

    mod gated_catalog {
        use crate::coding_agent::catalog::tests::*;

        const STAGES: &str = "/profiles/0/capabilities/answers/1/stages";
        const ALLOW_STAGES: &str = "/profiles/0/capabilities/answers/0/stages";
        const REASON: &str = "/profiles/0/rules/1";

        fn rejected(case: &str, change: impl FnOnce(&mut Value)) {
            assert!(
                crate::coding_agent::catalog::test_support::load(&gated_fixture_definition())
                    .is_ok()
            );
            let mut value = gated_fixture_definition();
            change(&mut value);
            assert!(
                crate::coding_agent::catalog::test_support::load(&value).is_err(),
                "{case} must fail catalog loading"
            );
        }

        fn stage(bytes: &str, choice: &str, gate: (&str, &str)) -> Value {
            let mut stage = json!({"bytes_hex":bytes,"required_rule_id":GATED_RULE,"required_choice_id":choice});
            stage[gate.0] = json!(gate.1);
            stage
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            crate::coding_agent::catalog::test_support::load(&gated_fixture_definition())
                .unwrap()
                .answer_plan(
                    CodingAgent::Codex,
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
                        stage.required_focus_choice_id().map(str::to_owned),
                        stage.required_screen_rule_id().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                gates,
                [
                    (b"h".to_vec(), Some("allow-once".into()), None),
                    (b"\r".to_vec(), Some("deny-once".into()), None),
                    (b"\r".to_vec(), None, Some(GATED_REASON_RULE.into())),
                ]
            );
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
                crate::coding_agent::catalog::test_support::load(&ends_on_deny_focus).is_ok(),
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
            rejected("focus_style bold", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][2]["focus_style"] = json!("bold");
            });
            rejected("region-scoped choice label", |definition| {
                definition["profiles"][0]["rules"][0]["matchers"][3]["scope"] = json!("region");
            });
            let mut textual = definitions();
            textual["profiles"][0]["rules"][0]["matchers"][1]["focus_style"] = json!("background");
            assert!(
                crate::coding_agent::catalog::test_support::load(&textual).is_ok(),
                "an ungated profile may declare a background focus matcher"
            );
        }

        #[test]
        fn every_backend_rejects_an_escape_byte_in_any_answer_stage() {
            rejected("gated stage 1b", |definition| {
                *definition
                    .pointer_mut(&format!("{STAGES}/0/bytes_hex"))
                    .unwrap() = json!("1b");
            });
            let mut value = definitions();
            value["profiles"][0]["capabilities"]["answers"] = json!([{
                "decision":"deny","request_rule_id":RULE,"answer_rule_id":"synthetic-escape",
                "choice_id":"deny-once","focus_matcher_id":"deny","fixture_ids":[FIXTURE],
                "stages":[{"bytes_hex":"6e","required_rule_id":RULE,"required_choice_id":"deny-once"}]
            }]);
            assert!(crate::coding_agent::catalog::test_support::load(&value).is_ok());
            value["profiles"][0]["capabilities"]["answers"][0]["stages"][0]["bytes_hex"] =
                json!("0d1b");
            assert!(
                crate::coding_agent::catalog::test_support::load(&value).is_err(),
                "ungated stage 0d1b"
            );
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
                    let catalog = crate::coding_agent::catalog::test_support::load(&good).unwrap();
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
            for (operation, invalid) in [
                (ComposerOperation::Prompt, json!("2120")),
                (ComposerOperation::Exec, json!("")),
                (ComposerOperation::Exec, json!(17)),
            ] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["submission"]
                    [operation_name(operation)]["prefix_hex"] = invalid;
                reject(&good, &bad, operation, "operation prefix");
            }
        }

        #[test]
        fn composer_submit_hex_requires_nonempty_even_lowercase_hex_and_string_type() {
            let good = definition(Some("7e"), Some("6b6c"));
            for invalid in [json!(""), json!("0"), json!("ABCDEF"), Value::Null] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["submission"]["prompt"]["submit_hex"] =
                    invalid;
                reject(&good, &bad, ComposerOperation::Prompt, "submit hex");
            }
        }

        #[test]
        fn composer_optional_submission_and_operation_objects_have_exact_complete_fields() {
            let good = definition(Some("7e"), Some("6b6c"));
            let submission = "/profiles/0/capabilities/composer/submission";
            let prompt = format!("{submission}/prompt");
            let rejected = |label: &str, change: &dyn Fn(&mut Value)| {
                let mut bad = good.clone();
                change(&mut bad);
                reject(&good, &bad, ComposerOperation::Prompt, label);
            };
            rejected("missing composer field", &|bad| {
                bad["profiles"][0]["capabilities"]["composer"]
                    .as_object_mut()
                    .unwrap()
                    .remove("rule_ids");
            });
            rejected("unknown composer field", &|bad| {
                bad["profiles"][0]["capabilities"]["composer"]["unknown"] = json!(true);
            });
            rejected("null submission", &|bad| {
                *bad.pointer_mut(submission).unwrap() = Value::Null;
            });
            rejected("array submission", &|bad| {
                *bad.pointer_mut(submission).unwrap() = json!([]);
            });
            rejected("missing operation", &|bad| {
                bad.pointer_mut(submission)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("exec");
            });
            rejected("unknown submission field", &|bad| {
                bad.pointer_mut(submission).unwrap()["unknown"] = json!(true);
            });
            rejected("missing plan field", &|bad| {
                bad.pointer_mut(&prompt)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("submit_hex");
            });
            rejected("unknown plan field", &|bad| {
                bad.pointer_mut(&prompt).unwrap()["unknown"] = json!(true);
            });
            rejected("plan object", &|bad| {
                *bad.pointer_mut(&prompt).unwrap() = json!(17);
            });
        }

        #[test]
        fn composer_each_operation_requires_its_own_valid_fixture_reference_set() {
            let good = definition(Some("7e"), Some("6b6c"));
            for invalid in [
                json!([]),
                json!([" "]),
                json!(["unknown-fixture"]),
                json!([PROMPT_FIXTURE, PROMPT_FIXTURE]),
                Value::Null,
            ] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["submission"]["prompt"]["fixture_ids"] =
                    invalid;
                reject(
                    &good,
                    &bad,
                    ComposerOperation::Prompt,
                    "operation fixture set",
                );
            }
            {
                let mut bad = good.clone();
                let mut foreign = bad["fixtures"][0].clone();
                foreign["id"] = json!("foreign-operation-fixture");
                foreign["backend"] = json!("claude");
                bad["fixtures"].as_array_mut().unwrap().push(foreign);
                bad["profiles"][0]["capabilities"]["composer"]["submission"]["prompt"]["fixture_ids"] =
                    json!(["foreign-operation-fixture"]);
                reject(
                    &good,
                    &bad,
                    ComposerOperation::Prompt,
                    "foreign operation provenance",
                );
            }
            let mut bad = good.clone();
            bad["profiles"][0]["capabilities"]["composer"]["submission"]["exec"]["fixture_ids"] =
                json!([]);
            reject(&good, &bad, ComposerOperation::Exec, "exec fixture set");
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
            let catalog = crate::coding_agent::catalog::test_support::load(&good).unwrap();
            assert_eq!(
                catalog.detection_coverage(CodingAgent::Codex),
                Coverage::Tested
            );
            for invalid in [
                json!([]),
                json!(["unknown-rule"]),
                json!([RULE]),
                json!([EMPTY, EMPTY]),
            ] {
                let mut bad = good.clone();
                bad["profiles"][0]["capabilities"]["composer"]["rule_ids"] = invalid;
                reject(
                    &good,
                    &bad,
                    ComposerOperation::Prompt,
                    "composer rule component",
                );
            }
        }
    }
    mod bundled_claude_auto {
        use crate::coding_agent::catalog::tests::*;

        const CLAUDE_VERSION: &str = "2.1.302";
        const ASK_RULE: &str = "claude-auto-ask-permission";
        const PERMISSION_RULE: &str = "claude-bash-permission";
        const TIP_RULE: &str = "claude-bash-permission-auto-tip";
        const STARTUP_RULE: &str = "claude-auto-startup";
        const WIZARD_RULE: &str = "claude-auto-env-setup-wizard";
        const READ_RULE: &str = "claude-read-outside-permission";
        const IDLE_RULE: &str = "claude-idle-composer";
        const UNMATCHED: &str = "unmatched-screen";
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

        type Expected = (&'static str, ObservationKind, &'static str);

        const COMPOSER: ObservationKind = ObservationKind::ComposerEmpty;
        const FORM: ObservationKind =
            ObservationKind::Interaction(InteractionClass::PermissionRequest);
        const QUESTION: ObservationKind = ObservationKind::Interaction(InteractionClass::Question);
        const UNKNOWN: ObservationKind = ObservationKind::Unknown;

        const CLASSIFICATION: [Expected; 46] = [
            (READ_ALLOW_CONTINUATION, COMPOSER, IDLE_RULE),
            (READ_DENY_CONTINUATION, COMPOSER, IDLE_RULE),
            (READ_FORM, FORM, READ_RULE),
            (READ_LONG, FORM, READ_RULE),
            (READ_ALLOW_PRE_STAGE, FORM, READ_RULE),
            (READ_DENY_PRE_STAGE, FORM, READ_RULE),
            (EDIT_READ_FORM, FORM, READ_RULE),
            (
                "claude-2.1.302-access-allow-continuation-1",
                COMPOSER,
                IDLE_RULE,
            ),
            ("claude-2.1.308-auto-tip-padded-question-1", FORM, TIP_RULE),
            (TIP_ALLOW_PRE_STAGE, FORM, TIP_RULE),
            (TIP_ALLOW_RUNNING, COMPOSER, IDLE_RULE),
            (
                "claude-2.1.302-access-deny-continuation-1",
                COMPOSER,
                IDLE_RULE,
            ),
            (TIP_DENY_PRE_STAGE, FORM, TIP_RULE),
            (TIP_FORM, FORM, TIP_RULE),
            ("claude-2.1.302-auto-approved-done-1", COMPOSER, IDLE_RULE),
            (RUNNING, COMPOSER, IDLE_RULE),
            (
                "claude-2.1.302-auto-ask-allow-continuation-1",
                COMPOSER,
                IDLE_RULE,
            ),
            (
                "claude-2.1.302-auto-ask-deny-continuation-1",
                COMPOSER,
                IDLE_RULE,
            ),
            (FORM_HIDING_NOTICE, FORM, ASK_RULE),
            (
                "claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1",
                FORM,
                ASK_RULE,
            ),
            (ASK_FORM, FORM, ASK_RULE),
            (STARTUP_CANCELLED, COMPOSER, IDLE_RULE),
            (STARTUP_OFFER, QUESTION, STARTUP_RULE),
            (STARTUP_WIZARD, QUESTION, WIZARD_RULE),
            (
                "claude-2.1.302-auto-fresh-first-launch-notice-1",
                COMPOSER,
                IDLE_RULE,
            ),
            (
                "claude-2.1.302-auto-fresh-onboarding-theme-1",
                UNKNOWN,
                UNMATCHED,
            ),
            (IDLE, COMPOSER, IDLE_RULE),
            (
                "claude-2.1.302-auto-model-switch-question-1",
                QUESTION,
                "claude-ask-user-question",
            ),
            (QUOTE, COMPOSER, IDLE_RULE),
            (NOTICE_EXPIRED, COMPOSER, IDLE_RULE),
            (NOTICE_IDLE, COMPOSER, IDLE_RULE),
            (NOTICE_WORKING, COMPOSER, IDLE_RULE),
            (VIEWER, UNKNOWN, UNMATCHED),
            (
                "claude-2.1.302-auto-two-refusals-notice-1",
                COMPOSER,
                IDLE_RULE,
            ),
            (MANUAL_FOOTER, COMPOSER, IDLE_RULE),
            (UNAVAILABLE_NOTICE, COMPOSER, IDLE_RULE),
            (VERBOSE_QUOTE, COMPOSER, IDLE_RULE),
            (
                "claude-2.1.302-auto-verbose-refusal-above-form-1",
                FORM,
                ASK_RULE,
            ),
            (HIDDEN_BY_FORM, FORM, ASK_RULE),
            (RESTORED, COMPOSER, IDLE_RULE),
            (REWRAPPED_FORM, FORM, ASK_RULE),
            (REWRAPPED, COMPOSER, IDLE_RULE),
            (VERBOSE_VIEWER, UNKNOWN, UNMATCHED),
            (TWO_REFUSALS, COMPOSER, IDLE_RULE),
            ("claude-2.1.302-default-manual-idle-1", COMPOSER, IDLE_RULE),
            ("claude-2.1.302-trust-dialog-1", UNKNOWN, UNMATCHED),
        ];

        fn screen(id: &str) -> ScreenSnapshot {
            replay_as_captured(id, |_| {}, None)
        }

        fn detector(catalog: &ProfileCatalog) -> Detector {
            Detector::new(catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
        }

        fn observe(id: &str) -> ScreenObservation {
            detector(ProfileCatalog::bundled())
                .observe(&screen(id))
                .unwrap()
        }

        fn shown(operation: &Value) -> Option<String> {
            operation.as_str().map(|text| {
                text.split('\n')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        }

        #[test]
        fn bundled_claude_auto_fixtures_produce_their_primary_observation() {
            for (id, kind, rule) in CLASSIFICATION {
                let observed = observe(id);
                assert_eq!(observed.kind, kind, "{id}");
                assert_eq!(observed.rule_id, rule, "{id}");
                assert_eq!(
                    observed.raw_evidence["native_version"], CLAUDE_VERSION,
                    "{id}"
                );
            }
        }

        #[test]
        fn bundled_claude_auto_ask_form_answers_allow_with_1_and_deny_with_2_ungated() {
            let catalog = ProfileCatalog::bundled();
            let form = observe(ASK_FORM);
            assert_eq!(form.match_evidence, MatchEvidence::Complete);
            assert!(!form.verified_noninteraction);
            assert_eq!(form.raw_evidence["operation"], ASK_ALLOW);
            assert_eq!(form.raw_evidence["choices"], json!(["Yes", "No"]));
            let default_form = screen("claude-2.1.302-default-permission-1");
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        ASK_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("ask-rule answer plan");
                assert!(
                    plan.stages().iter().all(|stage| stage.gate().is_none()),
                    "{decision:?} is ungated"
                );
                let matched = detector(catalog)
                    .match_answer(&screen(ASK_FORM), &plan)
                    .unwrap()
                    .expect("the ask form matches its plan");
                assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
                assert!(Some(matched.semantic()) == form.fingerprint.as_ref());
                assert!(
                    detector(catalog)
                        .match_answer(&default_form, &plan)
                        .unwrap()
                        .is_none(),
                    "{decision:?} ask plan on the predecessor form"
                );
                let predecessor = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        PERMISSION_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .unwrap();
                assert!(
                    detector(catalog)
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
        }

        #[test]
        fn bundled_claude_auto_tip_form_answers_allow_with_1_and_deny_with_4_ungated() {
            let catalog = ProfileCatalog::bundled();
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
            for (decision, pre_stage) in [
                (PermissionDecision::Allow, TIP_ALLOW_PRE_STAGE),
                (PermissionDecision::Deny, TIP_DENY_PRE_STAGE),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        TIP_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("tip-rule answer plan");
                assert!(
                    plan.stages().iter().all(|stage| stage.gate().is_none()),
                    "{decision:?} is ungated"
                );
                let matched = detector(catalog)
                    .match_answer(&screen(pre_stage), &plan)
                    .unwrap()
                    .expect("the pre-stage form matches its plan");
                assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
                assert!(Some(matched.semantic()) == observe(pre_stage).fingerprint.as_ref());
                for (other, rule) in other_forms {
                    assert!(
                        detector(catalog)
                            .match_answer(&screen(other), &plan)
                            .unwrap()
                            .is_none(),
                        "{decision:?} tip plan on {other}"
                    );
                    let foreign = catalog
                        .answer_plan(CodingAgent::Claude, rule, &AnswerKey::Decision(decision))
                        .unwrap();
                    assert!(
                        detector(catalog)
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
            let catalog = ProfileCatalog::bundled();
            for (id, operation, rows) in cases {
                let form = detector(catalog).observe(&screen(id)).unwrap();
                assert_eq!(form.kind, FORM, "{id}");
                assert_eq!(form.rule_id, READ_RULE, "{id}");
                assert_eq!(form.match_evidence, MatchEvidence::Complete, "{id}");
                assert!(!form.verified_noninteraction, "{id}");
                assert!(form.fingerprint.is_some(), "{id}");
                assert_eq!(form.raw_evidence["operation"], operation, "{id}");
                assert_eq!(form.raw_evidence["operation_rows"], rows, "{id}");
                assert_eq!(form.raw_evidence["choices"], json!(READ_CHOICES), "{id}");
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
            let catalog = ProfileCatalog::bundled();
            let other_forms = [
                ("claude-2.1.302-default-permission-1", PERMISSION_RULE),
                (ASK_FORM, ASK_RULE),
                (TIP_FORM, TIP_RULE),
            ];
            for (decision, pre_stage) in [
                (PermissionDecision::Allow, READ_ALLOW_PRE_STAGE),
                (PermissionDecision::Deny, READ_DENY_PRE_STAGE),
            ] {
                let plan = catalog
                    .answer_plan(
                        CodingAgent::Claude,
                        READ_RULE,
                        &AnswerKey::Decision(decision),
                    )
                    .expect("read-outside answer plan");
                assert!(
                    plan.stages().iter().all(|stage| stage.gate().is_none()),
                    "{decision:?} is ungated"
                );
                let matched = detector(catalog)
                    .match_answer(&screen(pre_stage), &plan)
                    .unwrap()
                    .expect("the pre-stage dialog matches its plan");
                assert_eq!(matched.choices(), READ_CHOICES.map(String::from).as_slice());
                assert!(Some(matched.semantic()) == observe(pre_stage).fingerprint.as_ref());
                for (other, rule) in other_forms {
                    assert!(
                        detector(catalog)
                            .match_answer(&screen(other), &plan)
                            .unwrap()
                            .is_none(),
                        "{decision:?} read-outside plan on {other}"
                    );
                    let foreign = catalog
                        .answer_plan(CodingAgent::Claude, rule, &AnswerKey::Decision(decision))
                        .unwrap();
                    assert!(
                        detector(catalog)
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
            let catalog = ProfileCatalog::bundled();
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
                            .answer_plan(CodingAgent::Claude, rule, &AnswerKey::Decision(decision))
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
                            STARTUP_RULE,
                            &AnswerKey::Choice(choice.into())
                        )
                        .is_some(),
                    "{choice}"
                );
            }
            assert!(
                catalog
                    .answer_plans(CodingAgent::Claude, WIZARD_RULE)
                    .is_empty(),
                "the wizard has no answer plan"
            );
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
        const TIP: &str = "claude-bash-permission-auto-tip";
        const ASK: &str = "claude-auto-ask-permission";
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

        const CLASSIFICATION: [Expected; 43] = [
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
                "claude-2.1.312-read-file-batched-permission-1",
                FORM,
                READ,
                Some("Read(/tmp/cafleet-194/claude-read-file-batched-cells-001-outside/probe.txt)"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-read-file-batched-allow-post-stage-1",
                FORM,
                READ,
                Some(
                    "Read(/tmp/cafleet-194/claude-read-file-batched-cells-001-outside/batched-probe.txt)",
                ),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-read-file-batched-deny-pre-stage-1",
                FORM,
                READ,
                Some("Read(/tmp/cafleet-194/claude-read-file-batched-deny-001-outside/probe.txt)"),
                &FORM_CHOICES,
            ),
            (
                "claude-2.1.312-bash-batched-permission-1",
                FORM,
                BASH,
                Some("/tmp/cafleet-194/claude-bash-batched-cells-001/permission-probe allow"),
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
            replay_as_captured(id, |_| {}, None)
        }
        fn detector() -> Detector {
            Detector::new(
                ProfileCatalog::bundled(),
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
                let primary = observe_screen(&replay_document(&document, false));
                let alternate = observe_screen(&replay_document(&document, true));
                assert_primary_matches_alternate(id, &primary, &alternate);
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
                let beside = observe_screen(&replay_document(&document, true));
                let full =
                    observe_screen(&replay_document(&full_width(&document, separator), true));
                assert_primary_matches_alternate(id, &beside, &full);
                assert_eq!(beside.rule_id, BASH, "{id}");
            }
        }

        const HEADINGS: [&str; 4] = [" Read file", " Edit file", " Create file", " Bash command"];
        const BATCHED: [&str; 3] = [
            "claude-2.1.312-read-file-batched-permission-1",
            "claude-2.1.312-read-file-batched-allow-post-stage-1",
            "claude-2.1.312-read-file-batched-deny-pre-stage-1",
        ];
        const HEADED_FORMS: [(&str, &str); 7] = [
            ("claude-2.1.312-read-file-permission-1", READ),
            ("claude-2.1.312-edit-file-permission-1", EDIT),
            ("claude-2.1.312-create-file-permission-1", CREATE),
            ("claude-2.1.312-bash-no-tip-permission-1", NO_TIP),
            ("claude-2.1.312-primary-default-permission-1", BASH),
            ("claude-2.1.302-access-permission-1", TIP),
            ("claude-2.1.302-auto-ask-permission-1", ASK),
        ];

        fn fixture_version(id: &str) -> &str {
            id.split('-').nth(1).unwrap()
        }
        fn observe_changed(id: &str, change: impl FnOnce(&mut Vec<String>)) -> ScreenObservation {
            let screen = replay_as_captured(id, change, None);
            Detector::new(
                ProfileCatalog::bundled(),
                CodingAgent::Claude,
                Some(fixture_version(id)),
            )
            .observe(&screen)
            .unwrap()
        }
        fn heading_row(lines: &[String], width: usize) -> usize {
            let rows: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    let left: String = line.chars().take(width).collect();
                    HEADINGS.contains(&left.trim_end())
                })
                .map(|(row, _)| row)
                .collect();
            assert_eq!(rows.len(), 1, "{rows:?}");
            rows[0]
        }
        fn with_counter(lines: &mut [String], width: usize, counter: &str) {
            let row = heading_row(lines, width);
            let left: String = lines[row].chars().take(width).collect();
            let rest: String = lines[row].chars().skip(width).collect();
            lines[row] = format!(
                "{:<pad$}{counter}{rest}",
                left.trim_end(),
                pad = width - counter.chars().count()
            );
        }
        fn without_counter(lines: &mut [String]) -> String {
            let counter =
                Regex::new(r"^( (?:Read file|Edit file|Create file|Bash command)) +(\d+ of \d+)$")
                    .unwrap();
            let rows: Vec<(usize, String, String)> = lines
                .iter()
                .enumerate()
                .filter_map(|(row, line)| {
                    counter
                        .captures(line.trim_end())
                        .map(|found| (row, found[1].to_owned(), found[2].to_owned()))
                })
                .collect();
            assert_eq!(rows.len(), 1, "{rows:?}");
            let (row, heading, shown) = rows.into_iter().next().unwrap();
            lines[row] = heading;
            shown
        }

        #[test]
        fn batched_read_fixtures_match_their_counter_free_single_form() {
            for (id, shown) in BATCHED.into_iter().zip(["1 of 2", "2 of 2", "1 of 2"]) {
                let batched = observe(id);
                let mut stripped = None;
                let single = observe_changed(id, |lines| stripped = Some(without_counter(lines)));
                assert_eq!(stripped.as_deref(), Some(shown), "{id}");
                assert_primary_matches_alternate(id, &batched, &single);
                assert_eq!(batched.rule_id, READ, "{id}");
            }
        }

        #[test]
        fn a_heading_counter_changes_no_identity_on_every_heading_anchored_form() {
            let mut rules = BTreeSet::new();
            for (id, rule) in HEADED_FORMS {
                let width = document(id)["cols"].as_u64().unwrap() as usize;
                let single = observe_changed(id, |_| {});
                assert_eq!(single.rule_id, rule, "{id}");
                let counted = observe_changed(id, |lines| with_counter(lines, width, "10 of 12"));
                assert_primary_matches_alternate(id, &counted, &single);
                rules.insert(rule);
            }
            assert_eq!(
                rules,
                BTreeSet::from([READ, EDIT, CREATE, NO_TIP, BASH, TIP, ASK])
            );
        }

        #[test]
        fn a_heading_counter_of_another_shape_leaves_the_form_unmatched() {
            let id = "claude-2.1.312-read-file-permission-1";
            for counter in ["1 of", "of 2", "1/2", "1 of 2 queued", "one of two"] {
                let observed = observe_changed(id, |lines| with_counter(lines, 120, counter));
                assert_ne!(observed.rule_id, READ, "{counter}");
                assert!(
                    !matches!(observed.kind, ObservationKind::Interaction(_)),
                    "{counter}: {:?}",
                    observed.kind
                );
            }
        }

        #[test]
        fn batched_read_forms_resolve_the_single_request_digits() {
            for (decision, fixtures) in [
                (PermissionDecision::Allow, &BATCHED[..2]),
                (PermissionDecision::Deny, &BATCHED[2..]),
            ] {
                let plan = plan(READ, AnswerKey::Decision(decision)).unwrap();
                for id in fixtures {
                    assert!(
                        detector()
                            .match_answer(&screen(id), &plan)
                            .unwrap()
                            .is_some(),
                        "{id} {decision:?}"
                    );
                }
            }
        }

        fn plan(rule: &str, key: AnswerKey) -> Option<AnswerPlan> {
            ProfileCatalog::bundled().answer_plan(CodingAgent::Claude, rule, &key)
        }
        #[test]
        fn dialog_a_decline_resolves_to_two_marker_gated_stages() {
            let decline =
                plan(DIALOG_A, AnswerKey::Choice("decline".into())).expect("Dialog A decline plan");
            assert!(
                decline
                    .stages()
                    .iter()
                    .all(|stage| stage.required_focus_choice_id().is_some()),
                "every decline stage is marker-gated"
            );
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
            for choice in ["not-now", "dont-show-again"] {
                let plan = plan(OFFER, AnswerKey::Choice(choice.into())).expect("offer plan");
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
            let catalog = ProfileCatalog::bundled();
            assert!(plan(DIALOG_A, AnswerKey::Choice("accept".into())).is_none());
            assert!(plan(OFFER, AnswerKey::Choice("yes".into())).is_none());
            for rule in [WIZARD, NOTICE] {
                assert!(
                    catalog.answer_plans(CodingAgent::Claude, rule).is_empty(),
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
        }

        #[test]
        fn file_and_no_tip_decisions_resolve_to_their_single_request_digits() {
            for (rule, pre_stages) in [
                (
                    READ,
                    [
                        "claude-2.1.312-read-file-allow-pre-stage-1",
                        "claude-2.1.312-read-file-deny-pre-stage-1",
                    ],
                ),
                (
                    EDIT,
                    [
                        "claude-2.1.312-edit-file-allow-pre-stage-1",
                        "claude-2.1.312-edit-file-deny-pre-stage-1",
                    ],
                ),
                (
                    CREATE,
                    [
                        "claude-2.1.312-create-file-allow-pre-stage-1",
                        "claude-2.1.312-create-file-deny-pre-stage-1",
                    ],
                ),
                (
                    NO_TIP,
                    [
                        "claude-2.1.312-bash-no-tip-allow-pre-stage-1",
                        "claude-2.1.312-bash-no-tip-deny-pre-stage-1",
                    ],
                ),
            ] {
                for (decision, pre_stage) in [PermissionDecision::Allow, PermissionDecision::Deny]
                    .into_iter()
                    .zip(pre_stages)
                {
                    let plan = plan(rule, AnswerKey::Decision(decision))
                        .unwrap_or_else(|| panic!("{rule} {decision:?} plan"));
                    assert!(
                        plan.stages().iter().all(|stage| stage.gate().is_none()),
                        "{rule} {decision:?}"
                    );
                    assert!(
                        detector()
                            .match_answer(&screen(pre_stage), &plan)
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
    }

    mod bundled_claude_ask_user_question {
        use crate::coding_agent::catalog::tests::*;

        const ASK_VERSION: &str = "2.1.322";
        const FORM_RULE: &str = "claude-ask-user-question";
        const REVIEW_RULE: &str = "claude-ask-user-question-review";

        pub(super) const FORMS: [&str; 19] = [
            "claude-2.1.322-ask-single-tmux-120-1",
            "claude-2.1.322-ask-tabs-tmux-120-1",
            "claude-2.1.322-ask-tabs-tmux-120-2",
            "claude-2.1.322-ask-multi-tmux-120-1",
            "claude-2.1.322-ask-multi-tmux-120-2",
            "claude-2.1.322-ask-multi-tmux-120-3",
            "claude-2.1.322-ask-review-tmux-120-1",
            "claude-2.1.322-ask-review-narrow-tmux-060-1",
            "claude-2.1.322-ask-typing-tmux-120-1",
            "claude-2.1.322-ask-typing-tmux-120-2",
            "claude-2.1.322-ask-narrow-tmux-060-1",
            "claude-2.1.322-ask-pending-tmux-120-1",
            "claude-2.1.322-ask-single-herdr-359-1",
            "claude-2.1.322-ask-tabs-herdr-359-1",
            "claude-2.1.322-ask-tabs-herdr-359-2",
            "claude-2.1.322-ask-multi-herdr-359-1",
            "claude-2.1.322-ask-multi-herdr-359-2",
            "claude-2.1.322-ask-multi-herdr-359-3",
            "claude-2.1.322-ask-review-herdr-359-1",
        ];
        pub(super) const REVIEWS: [&str; 4] = [
            "claude-2.1.322-ask-review-tmux-120-2",
            "claude-2.1.322-ask-review-narrow-tmux-060-2",
            "claude-2.1.322-ask-review-narrow-tmux-040-2",
            "claude-2.1.322-ask-review-herdr-359-2",
        ];
        pub(super) const BELOW_W_MIN: [&str; 2] = [
            "claude-2.1.322-ask-narrow-tmux-040-1",
            "claude-2.1.322-ask-review-narrow-tmux-040-1",
        ];
        pub(super) const QUOTED: &str = "claude-2.1.322-ask-quoted-tmux-120-1";
        pub(super) const CLOSED: [&str; 2] = [
            "claude-2.1.322-ask-single-closed-tmux-120-1",
            "claude-2.1.322-ask-single-closed-herdr-359-1",
        ];
        /// Native dialogs drawn with the AskUserQuestion component.
        const SAME_COMPONENT: [&str; 1] = ["claude-2.1.302-auto-model-switch-question-1"];
        const TRANSCRIPT_VIEWERS: [&str; 2] = [
            "claude-2.1.302-auto-transcript-viewer-1",
            "claude-2.1.302-auto-verbose-transcript-viewer-1",
        ];

        fn observe(id: &str) -> ScreenObservation {
            observe_bundled(
                CodingAgent::Claude,
                ASK_VERSION,
                &replay_as_captured(id, |_| {}, None),
            )
        }

        fn assert_choiceless_question(id: &str, rule: &str) {
            let observed = observe(id);
            assert_eq!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::Question),
                "{id}"
            );
            assert_eq!(observed.rule_id, rule, "{id}");
            assert_eq!(observed.match_evidence, MatchEvidence::Complete, "{id}");
            assert!(!observed.verified_noninteraction, "{id}");
            assert_eq!(observed.raw_evidence["operation"], Value::Null, "{id}");
            assert_eq!(observed.raw_evidence["choices"], json!([]), "{id}");
            assert!(observed.fingerprint.is_some(), "{id}");
        }

        fn assert_no_question(id: &str) {
            let observed = observe(id);
            assert_ne!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::Question),
                "{id} is decided by {}",
                observed.rule_id
            );
            assert!(
                observed.rule_id != FORM_RULE && observed.rule_id != REVIEW_RULE,
                "{id} is decided by {}",
                observed.rule_id
            );
        }

        #[test]
        fn every_ask_user_question_form_capture_opens_a_choiceless_question() {
            for id in FORMS {
                assert_choiceless_question(id, FORM_RULE);
            }
        }

        #[test]
        fn a_native_dialog_drawn_with_the_same_component_opens_the_same_choiceless_question() {
            for id in SAME_COMPONENT {
                assert_choiceless_question(id, FORM_RULE);
            }
        }

        #[test]
        fn every_review_your_answers_capture_opens_a_choiceless_question() {
            for id in REVIEWS {
                assert_choiceless_question(id, REVIEW_RULE);
            }
        }

        #[test]
        fn a_form_narrower_than_its_w_min_is_unmatched() {
            for id in BELOW_W_MIN {
                let observed = observe(id);
                assert_eq!(observed.kind, ObservationKind::Unknown, "{id}");
                assert_eq!(observed.rule_id, "unmatched-screen", "{id}");
            }
        }

        #[test]
        fn the_quoted_form_and_the_transcript_viewers_yield_no_question() {
            for id in std::iter::once(QUOTED).chain(TRANSCRIPT_VIEWERS) {
                assert_no_question(id);
            }
        }

        #[test]
        fn the_answered_form_returns_to_a_verified_idle_composer() {
            for id in CLOSED {
                let observed = observe(id);
                assert_eq!(observed.kind, ObservationKind::ComposerEmpty, "{id}");
                assert!(observed.verified_noninteraction, "{id}");
            }
        }

        /// The bundled catalog without the two AskUserQuestion rules.
        fn without_ask_rules() -> ProfileCatalog {
            let mut definitions = bundled_definitions();
            let claude = definitions["profiles"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|profile| profile["backend"] == "claude")
                .expect("the bundled claude profile");
            let ask = |id: &Value| id == FORM_RULE || id == REVIEW_RULE;
            claude["rules"]
                .as_array_mut()
                .unwrap()
                .retain(|rule| !ask(&rule["id"]));
            claude["capabilities"]["detection"]["rule_ids"]
                .as_array_mut()
                .unwrap()
                .retain(|id| !ask(id));
            ProfileCatalog::from_definitions(&definitions).unwrap()
        }

        #[test]
        fn every_existing_claude_fixture_keeps_its_observation_with_the_ask_rules_present() {
            let ask_captures: BTreeSet<&str> = FORMS
                .into_iter()
                .chain(REVIEWS)
                .chain(BELOW_W_MIN)
                .chain(CLOSED)
                .chain(SAME_COMPONENT)
                .chain([QUOTED])
                .collect();

            let without = without_ask_rules();
            let definitions = bundled_definitions();
            let existing: Vec<(&str, &str)> = definitions["fixtures"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|fixture| fixture["backend"] == "claude")
                .map(|fixture| {
                    (
                        fixture["id"].as_str().unwrap(),
                        fixture["native_version"].as_str().unwrap(),
                    )
                })
                .filter(|(id, _)| !ask_captures.contains(id))
                .collect();
            assert!(!existing.is_empty());
            for (id, version) in existing {
                let screen = replay_as_captured(id, |_| {}, None);
                let with = Detector::new(
                    ProfileCatalog::bundled(),
                    CodingAgent::Claude,
                    Some(version),
                )
                .observe(&screen)
                .unwrap();
                let before = Detector::new(&without, CodingAgent::Claude, Some(version))
                    .observe(&screen)
                    .unwrap();
                assert_eq!(with.kind, before.kind, "{id}");
                assert_eq!(with.rule_id, before.rule_id, "{id}");
                assert_eq!(
                    with.verified_noninteraction, before.verified_noninteraction,
                    "{id}"
                );
                for field in ["operation", "choices"] {
                    assert_eq!(
                        with.raw_evidence[field], before.raw_evidence[field],
                        "{id}: {field}"
                    );
                }
                assert!(with.fingerprint == before.fingerprint, "{id}");
            }
        }
    }

    mod bundled_backends {
        use crate::coding_agent::catalog::tests::*;

        #[test]
        fn hermes_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages() {
            let bundled = ProfileCatalog::bundled();
            for (operation, name, literal, typed) in [
                (ComposerOperation::Prompt, "prompt", "ok", b"ok".to_vec()),
                (
                    ComposerOperation::Exec,
                    "exec",
                    "echo x",
                    b"! echo x".to_vec(),
                ),
            ] {
                let plan = bundled
                    .composer_plan(CodingAgent::Hermes, operation)
                    .unwrap_or_else(|| panic!("{name} plan"));
                assert_eq!(
                    plan.stages(literal, LiteralFraming::Plain).unwrap(),
                    [typed, b"\r".to_vec()]
                );
            }
        }

        #[test]
        fn pi_composer_plans_submit_a_prompt_and_a_native_exec_in_separate_stages() {
            let bundled = ProfileCatalog::bundled();
            for (operation, name, literal, typed) in [
                (
                    ComposerOperation::Prompt,
                    "prompt",
                    "-leading dash",
                    b"-leading dash".to_vec(),
                ),
                (
                    ComposerOperation::Exec,
                    "exec",
                    "echo cafleet-220-exec",
                    b"! echo cafleet-220-exec".to_vec(),
                ),
            ] {
                let plan = bundled
                    .composer_plan(CodingAgent::Pi, operation)
                    .unwrap_or_else(|| panic!("{name} plan"));
                assert_eq!(
                    plan.stages(literal, LiteralFraming::Plain).unwrap(),
                    [typed, b"\r".to_vec()]
                );
            }
        }

        #[test]
        fn bundled_submission_fixtures_replay_their_composer_stages_through_the_production_catalog()
        {
            let bundled = ProfileCatalog::bundled();
            let definitions = bundled_definitions();
            for (backend, run) in [
                (CodingAgent::Claude, "2.1.308"),
                (CodingAgent::Codex, "0.163.0"),
                (CodingAgent::Opencode, "2.1.4"),
                (CodingAgent::Pi, "1.5.0"),
            ] {
                let detector = Detector::new(bundled, backend, Some(run));
                let profile = definitions["profiles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|profile| profile["backend"] == backend.name())
                    .expect("bundled backend profile");
                for (operation, name) in [
                    (ComposerOperation::Prompt, "prompt"),
                    (ComposerOperation::Exec, "exec"),
                ] {
                    bundled
                        .composer_plan(backend, operation)
                        .unwrap_or_else(|| panic!("{backend:?} {name} plan"));
                    let cited =
                        profile["capabilities"]["composer"]["submission"][name]["fixture_ids"]
                            .as_array()
                            .unwrap();
                    for id in cited.iter().map(|id| id.as_str().unwrap()) {
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
                    }
                }
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
                ProfileCatalog::bundled(),
                CodingAgent::Opencode,
                Some(OPENCODE_VERSION),
            )
        }

        fn observe(screen: &ScreenSnapshot) -> ScreenObservation {
            detector().observe(screen).unwrap()
        }

        fn plan(decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::bundled()
                .answer_plan(
                    CodingAgent::Opencode,
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
            let region = observation.matched_region.expect("live form region");
            assert_eq!((region.top, region.bottom), (10, 22), "{case}");
        }

        fn plan_for(request: &str, decision: PermissionDecision) -> AnswerPlan {
            ProfileCatalog::bundled()
                .answer_plan(
                    CodingAgent::Opencode,
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
    }

    mod single_profile {
        use crate::coding_agent::catalog::tests::*;

        fn catalog(profiles: &[(CodingAgent, &str)]) -> ProfileCatalog {
            crate::coding_agent::catalog::test_support::load(&single_profiles(profiles)).unwrap()
        }

        /// The label of `backend`'s profile, named by its allow plan.
        fn label(catalog: &ProfileCatalog, backend: CodingAgent) -> Option<String> {
            let plan = catalog.answer_plan(
                backend,
                RULE,
                &AnswerKey::Decision(PermissionDecision::Allow),
            )?;
            let label = plan
                .capability()
                .answer_rule_id()
                .strip_suffix("-allow-plan")
                .expect("a single-profile answer plan is named after its label");
            Some(label.to_owned())
        }

        fn rejected(value: &Value, label: &str) -> String {
            match crate::coding_agent::catalog::test_support::load(value) {
                Err(error) => {
                    assert_eq!(error.exit_code(), 2, "{label}: {}", error.message());
                    assert!(
                        error.message().starts_with("invalid native profile: "),
                        "{label}: {}",
                        error.message()
                    );
                    error.message().to_string()
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
        fn catalog_rejects_a_second_profile_for_one_backend() {
            type Case = (&'static str, &'static [(CodingAgent, &'static str)]);
            const CASES: [Case; 3] = [
                (
                    "two codex profiles",
                    &[
                        (CodingAgent::Codex, "first"),
                        (CodingAgent::Codex, "second"),
                    ],
                ),
                (
                    "a second codex profile after a claude profile",
                    &[
                        (CodingAgent::Codex, "first"),
                        (CodingAgent::Claude, "claude"),
                        (CodingAgent::Codex, "second"),
                    ],
                ),
                (
                    "two claude profiles",
                    &[
                        (CodingAgent::Claude, "first"),
                        (CodingAgent::Claude, "second"),
                    ],
                ),
            ];
            for (name, profiles) in CASES {
                let value = single_profiles(profiles);
                match ProfileCatalog::from_definitions(&value) {
                    Err(error) => {
                        assert_eq!(error.exit_code(), 2, "{name}");
                        assert_eq!(
                            error.message(),
                            "invalid native profile: more than one profile for one backend",
                            "{name}"
                        );
                    }
                    Ok(_) => panic!("{name} accepted"),
                }
            }
        }

        #[test]
        fn catalog_profile_keys_are_backend_rules_and_capabilities() {
            let value = single_profiles(&[(CodingAgent::Codex, "only")]);
            let keys = value["profiles"][0]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            assert_eq!(keys, BTreeSet::from(["backend", "capabilities", "rules"]));
            assert!(crate::coding_agent::catalog::test_support::load(&value).is_ok());
        }

        #[test]
        fn validation_rejects_malformed_fixture_capture_versions() {
            assert!(crate::coding_agent::catalog::test_support::load(&definitions()).is_ok());
            for malformed in MALFORMED {
                let mut value = definitions();
                value["fixtures"][0]["native_version"] = json!(malformed);
                rejected(&value, &format!("fixture native_version {malformed:?}"));
            }
        }

        #[test]
        fn provenance_accepts_a_citation_of_its_backends_fixture_at_any_capture_version() {
            for citation in CITATIONS {
                for captured in ["0.0.1", "1.0.0-rc.1", "2.0.0+build.1", "99.0.0"] {
                    let value = citing(citation, CodingAgent::Codex, captured);
                    assert!(
                        crate::coding_agent::catalog::test_support::load(&value).is_ok(),
                        "{citation} {captured}"
                    );
                }
            }
        }

        #[test]
        fn provenance_rejects_a_citation_of_another_backends_fixture() {
            for citation in CITATIONS {
                for other in CodingAgent::all()
                    .iter()
                    .copied()
                    .filter(|backend| *backend != CodingAgent::Codex)
                {
                    let label = format!("{citation} citing a {other:?} fixture");
                    let message = rejected(&citing(citation, other, VERSION), &label);
                    assert!(message.contains("fixture backend"), "{label}: {message}");
                }
            }
        }

        const CITATIONS: [&str; 5] = [
            "detection/fixture_ids",
            "composer/fixture_ids",
            "composer/submission/prompt/fixture_ids",
            "composer/submission/exec/fixture_ids",
            "answers/0/fixture_ids",
        ];

        /// The single Codex profile with `citation` naming one extra fixture of `backend`
        /// captured on `captured`.
        fn citing(citation: &str, backend: CodingAgent, captured: &str) -> Value {
            let mut value = single_profiles(&[(CodingAgent::Codex, "only")]);
            let mut fixture = fixture_at("citation-probe", captured);
            fixture["backend"] = json!(backend.name());
            value["fixtures"].as_array_mut().unwrap().push(fixture);
            *value
                .pointer_mut(&format!("/profiles/0/capabilities/{citation}"))
                .unwrap() = json!(["citation-probe"]);
            value
        }

        #[test]
        fn each_backend_resolves_only_its_own_profile() {
            let catalog = catalog(&[
                (CodingAgent::Codex, "codex-only"),
                (CodingAgent::Claude, "claude-only"),
            ]);
            for (backend, expected) in [
                (CodingAgent::Codex, Some("codex-only")),
                (CodingAgent::Claude, Some("claude-only")),
                (CodingAgent::Opencode, None),
                (CodingAgent::Hermes, None),
                (CodingAgent::Pi, None),
            ] {
                assert_eq!(label(&catalog, backend).as_deref(), expected, "{backend:?}");
                let tested = if expected.is_some() {
                    Coverage::Tested
                } else {
                    Coverage::Unknown
                };
                assert_eq!(catalog.detection_coverage(backend), tested, "{backend:?}");
            }
        }

        #[test]
        fn every_run_version_resolves_every_capability_through_the_backends_profile() {
            let catalog = catalog(&[(CodingAgent::Codex, "only")]);
            let mut rows = vec![
                "Live footer".to_owned(),
                "Refused: write outside.txt".to_owned(),
                "Refusal end".to_owned(),
            ];
            rows.extend(lines("write disposable.txt"));
            let screen = terminal(&rows, 80, false).snapshot();
            for run in [Some("0.0.1"), Some("1.4.2"), Some("99.0.0-beta.1"), None] {
                let observed = Detector::new(&catalog, CodingAgent::Codex, run)
                    .observe(&screen)
                    .unwrap();
                assert_eq!(
                    observed.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest),
                    "{run:?}"
                );
                assert_eq!(
                    observed.raw_evidence["native_version"],
                    run.map_or(Value::Null, |run| json!(run)),
                    "{run:?}"
                );
            }
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(
                    catalog.verified_composer(CodingAgent::Codex, operation),
                    "{operation:?}"
                );
                catalog
                    .composer_plan(CodingAgent::Codex, operation)
                    .unwrap_or_else(|| panic!("{operation:?} plan"));
            }
            assert_eq!(label(&catalog, CodingAgent::Codex).as_deref(), Some("only"));
            assert_eq!(
                catalog.detection_coverage(CodingAgent::Codex),
                Coverage::Tested
            );
        }
    }

    use crate::broker::runtime::PermissionDecision;

    #[test]
    fn composer_nullable_operations_and_absent_composer_keep_executable_coverage_unknown() {
        for (catalog, observation) in [
            (composer_catalog(None, None), Coverage::Tested),
            (
                crate::coding_agent::catalog::test_support::load(&definitions()).unwrap(),
                Coverage::Unknown,
            ),
        ] {
            capabilities(
                &catalog,
                CodingAgent::Codex,
                observation,
                (Coverage::Unknown, Coverage::Unknown),
            );
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(
                    catalog
                        .composer_plan(CodingAgent::Codex, operation)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn composer_prompt_plan_retains_exact_selected_metadata_and_independent_coverage() {
        for (operation, other, value, coverage) in [
            (
                ComposerOperation::Prompt,
                ComposerOperation::Exec,
                composer_catalog(Some("7e"), None),
                (Coverage::Tested, Coverage::Unknown),
            ),
            (
                ComposerOperation::Exec,
                ComposerOperation::Prompt,
                composer_catalog(None, Some("6b6c")),
                (Coverage::Unknown, Coverage::Tested),
            ),
        ] {
            let catalog = value;
            plan(&catalog, operation);
            capabilities(&catalog, CodingAgent::Codex, Coverage::Tested, coverage);
            assert!(
                catalog.composer_plan(CodingAgent::Codex, other).is_none(),
                "{operation:?}"
            );
            let actual = catalog.capabilities(CodingAgent::Codex);
            assert_eq!(actual.detection, Coverage::Unknown);
            assert!(actual.answer_decisions.is_empty());
        }
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
        let catalog = crate::coding_agent::catalog::test_support::load(&value).unwrap();
        let prompt = plan(&catalog, ComposerOperation::Prompt);
        let exec = plan(&catalog, ComposerOperation::Exec);
        assert_eq!(prompt.encode("literal").unwrap(), b"literal~");
        assert_eq!(exec.encode("literal").unwrap(), b"! literalkl");
        capabilities(
            &catalog,
            CodingAgent::Codex,
            Coverage::Tested,
            (Coverage::Tested, Coverage::Tested),
        );
        let actual = catalog.capabilities(CodingAgent::Codex);
        assert_eq!(actual.detection, Coverage::Tested);
        assert_eq!(
            actual.answer_decisions,
            [PermissionDecision::Allow, PermissionDecision::Deny]
        );
        for &backend in CodingAgent::all() {
            if backend == CodingAgent::Codex {
                continue;
            }
            for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
                assert!(catalog.composer_plan(backend, operation).is_none());
            }
            capabilities(
                &catalog,
                backend,
                Coverage::Unknown,
                (Coverage::Unknown, Coverage::Unknown),
            );
        }
    }

    #[test]
    fn composer_prompt_preserves_literal_utf8_between_empty_prefix_and_tested_submit_bytes() {
        let catalog = composer_catalog(Some("7e"), None);
        let literal = "  literal 世界 '$x' `backticks` \\\"quotes\\\"  ";
        let bytes = plan(&catalog, ComposerOperation::Prompt)
            .encode(literal)
            .unwrap();
        assert_eq!(bytes, expected(ComposerOperation::Prompt, literal, b"~"));
        assert_eq!(&bytes[..literal.len()], literal.as_bytes());
    }

    #[test]
    fn composer_exec_owns_exact_bang_space_prefix_without_changing_the_caller_literal() {
        let catalog = composer_catalog(None, Some("6b6c"));
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
                let catalog = crate::coding_agent::catalog::test_support::load(&value).unwrap();
                let selected = plan(&catalog, operation);
                let literal_size = 65536 - operation.prefix().len() - submit_length;
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
        assert_not_impl!(ComposerPlan: std::fmt::Debug, serde::Serialize);
        const SUBMIT: &str = "E16_PRIVATE_SUBMIT";
        const LITERAL: &str = "E16_PRIVATE_LITERAL";
        let hex = crate::digest::hex(SUBMIT.as_bytes());
        for operation in [ComposerOperation::Prompt, ComposerOperation::Exec] {
            let good = match operation {
                ComposerOperation::Prompt => definition(Some(&hex), None),
                ComposerOperation::Exec => definition(None, Some(&hex)),
            };
            let catalog = crate::coding_agent::catalog::test_support::load(&good).unwrap();
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
