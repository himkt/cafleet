use super::detector::ObservationKind;
use super::{AnswerCapability, CodingAgent, PermissionDecision};
use crate::broker::runtime::Coverage;
use crate::error::CafleetError;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
type Result<T> = std::result::Result<T, CafleetError>;
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
#[derive(Clone)]
pub(super) struct Fixture {
    pub id: String,
    pub backend: CodingAgent,
    pub version: String,
}
#[derive(Clone)]
pub(super) struct Capture {
    pub matcher: String,
    pub name: String,
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
}
#[derive(Clone)]
pub(super) struct Profile {
    pub backend: CodingAgent,
    pub versions: Vec<String>,
    pub version: String,
    pub rules: Vec<Rule>,
    pub detection: Option<Capability>,
    pub composer: Option<Capability>,
    pub answers: Vec<Answer>,
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
    pub detection: Coverage,
    pub composer: Coverage,
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
    Ok(Capture { matcher, name })
}
fn optional_capture(value: &Value, matchers: &[Matcher]) -> Result<Option<Capture>> {
    if value.is_null() {
        Ok(None)
    } else {
        capture(value, matchers).map(Some)
    }
}
fn compile_rule(value: &Value) -> Result<Rule> {
    let m = object(
        value,
        &[
            "id",
            "priority",
            "observation",
            "region",
            "matchers",
            "extract",
            "verified_noninteraction",
            "sensitive",
        ],
    )?;
    let id = text(&m["id"])?;
    let priority = m["priority"]
        .as_i64()
        .ok_or_else(|| invalid("integer priority required"))?;
    let kind = ObservationKind::parse(&text(&m["observation"])?)?;
    let verified = boolean(&m["verified_noninteraction"])?;
    require(
        !verified
            || !matches!(
                kind,
                ObservationKind::Interaction(_) | ObservationKind::Unknown
            ),
        "interaction/unknown cannot verify noninteraction",
    )?;
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
        let m = object(v, &["id", "scope", "pattern"])?;
        let id = text(&m["id"])?;
        unique(&mut ids, &id)?;
        matchers.push(Matcher {
            id,
            scope: one_of(&m["scope"], &["line", "region"])?,
            regex: expression(&m["pattern"])?,
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
    Ok(Rule {
        id,
        priority,
        kind,
        region,
        operation: optional_capture(&ex["operation"], &matchers)?,
        native: optional_capture(&ex["native_request_id"], &matchers)?,
        choices,
        matchers,
        verified,
        sensitive: boolean(&m["sensitive"])?,
    })
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
fn capability(
    value: &Value,
    composer: bool,
    rules: &[Rule],
    backend: CodingAgent,
    versions: &[String],
    fixtures: &[Fixture],
) -> Result<Option<Capability>> {
    if value.is_null() {
        return Ok(None);
    }
    let m = object(value, &["rule_ids", "fixture_ids"])?;
    let ids = texts(&m["rule_ids"])?;
    require(!ids.is_empty(), "capability rules required")?;
    for id in &ids {
        let rule = rules
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| invalid("unknown capability rule"))?;
        require(
            matches!(
                rule.kind,
                ObservationKind::ComposerEmpty | ObservationKind::ComposerDraft
            ) == composer,
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
impl ProfileCatalog {
    pub fn bundled() -> Result<Self> {
        let catalog =
            Self::from_definitions(&json!({"schema_version":1,"fixtures":[],"profiles":[]}))?;
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
            one_of(&m["kind"], &["synthetic", "native"])?;
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
            let c = object(&m["capabilities"], &["detection", "composer", "answers"])?;
            let detection = capability(
                &c["detection"],
                false,
                &rules,
                backend,
                &versions,
                &fixtures,
            )?;
            let composer = capability(&c["composer"], true, &rules, backend, &versions, &fixtures)?;
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
                require(!stages.is_empty(), "measured answer stages required")?;
                for v in stages {
                    let s = object(v, &["bytes_hex", "required_rule_id", "required_choice_id"])?;
                    let bytes = text(&s["bytes_hex"])?;
                    require(
                        bytes.len().is_multiple_of(2) && lower_hex(&bytes),
                        "nonempty lowercase stage bytes required",
                    )?;
                    require(
                        text(&s["required_rule_id"])? == request
                            && text(&s["required_choice_id"])? == choice_id,
                        "answer stage request/decision mismatch",
                    )?;
                }
                let fixture_ids = texts(&a["fixture_ids"])?;
                provenance(&fixture_ids, backend, &versions, &fixtures)?;
                answers.push(Answer {
                    decision,
                    request,
                    id,
                    fixtures: fixture_ids,
                });
            }
            profiles.push(Profile {
                backend,
                versions,
                version,
                rules,
                detection,
                composer,
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
        version.and_then(|v| {
            self.profiles
                .iter()
                .find(|p| p.backend == backend && p.versions.iter().any(|s| s == v))
        })
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
        let profile = self.matching(backend, version);
        NativeCapabilities {
            backend,
            native_version: version.map(str::to_owned),
            profile_version: profile.map(|p| p.version.clone()),
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
            answer_decisions: profile
                .map(|p| p.answers.iter().map(|a| a.decision).collect())
                .unwrap_or_default(),
            capability_reason: if profile.is_none() {
                Some("exact native profile unavailable".into())
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
}
