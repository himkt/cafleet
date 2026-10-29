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
    PermissionContinuation,
    Working,
    ComposerEmpty,
    ComposerDraft,
    Unknown,
    AutomaticRefusal,
    AutomaticReviewUnavailable,
    AutomaticReviewAvailable,
}

impl ObservationKind {
    pub(super) fn parse(value: &str) -> Result<Self> {
        if let Ok(class) = InteractionClass::try_from(value) {
            return Ok(Self::Interaction(class));
        }
        Ok(match value {
            "permission_denied" => Self::PermissionDenied,
            "permission_continuation" => Self::PermissionContinuation,
            "working" => Self::Working,
            "composer_empty" => Self::ComposerEmpty,
            "composer_draft" => Self::ComposerDraft,
            "unknown" => Self::Unknown,
            "automatic_refusal" => Self::AutomaticRefusal,
            "automatic_review_unavailable" => Self::AutomaticReviewUnavailable,
            "automatic_review_available" => Self::AutomaticReviewAvailable,
            _ => return Err(CafleetError::Usage("invalid profile observation".into())),
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Interaction(class) => class.as_str(),
            Self::PermissionDenied => "permission_denied",
            Self::PermissionContinuation => "permission_continuation",
            Self::Working => "working",
            Self::ComposerEmpty => "composer_empty",
            Self::ComposerDraft => "composer_draft",
            Self::Unknown => "unknown",
            Self::AutomaticRefusal => "automatic_refusal",
            Self::AutomaticReviewUnavailable => "automatic_review_unavailable",
            Self::AutomaticReviewAvailable => "automatic_review_available",
        }
    }
    pub(crate) fn is_annotation(self) -> bool {
        matches!(
            self,
            Self::AutomaticRefusal
                | Self::AutomaticReviewUnavailable
                | Self::AutomaticReviewAvailable
        )
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
    pub(crate) top: u16,
    pub(crate) left: u16,
    pub(crate) bottom: u16,
    pub(crate) right: u16,
}
#[derive(Clone)]
pub(crate) struct ScreenObservation {
    pub(crate) kind: ObservationKind,
    pub(crate) profile_version: Option<String>,
    pub(crate) rule_id: String,
    pub(crate) match_evidence: MatchEvidence,
    pub(crate) matched_region: Option<ScreenRegion>,
    pub(crate) raw_evidence: Value,
    pub(crate) fingerprint: Option<SemanticFingerprint>,
    pub(crate) verified_noninteraction: bool,
    pub(crate) native_correlation: Option<String>,
    pub(crate) output_seq: u64,
    pub(crate) continues_rule_id: Option<String>,
    pub(crate) annotations: Vec<AnnotationBlock>,
    pub(crate) guarded_annotation_rules: Vec<String>,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AnnotationFingerprint {
    backend: String,
    profile_version: String,
    rule_id: String,
    shown: String,
}
#[derive(Clone)]
pub(crate) struct AnnotationBlock {
    pub(crate) kind: ObservationKind,
    pub(crate) rule_id: String,
    pub(crate) matched_region: ScreenRegion,
    pub(crate) raw_evidence: Value,
    pub(crate) fingerprint: AnnotationFingerprint,
}
fn normalized_shown(text: &str, row_margin: Option<&str>) -> String {
    let mut rows = text.split('\n');
    let mut joined = rows.next().expect("split yields a first row").to_owned();
    for row in rows {
        joined.push_str(
            row_margin
                .and_then(|margin| row.strip_prefix(margin))
                .unwrap_or(row),
        );
    }
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
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
    focused_choice: Option<String>,
    output_seq: u64,
}
impl AnswerMatch {
    pub(crate) fn focused_choice_id(&self) -> Option<&str> {
        self.focused_choice.as_deref()
    }
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
    mode: Option<String>,
    excerpt: String,
    focuses: BTreeMap<String, String>,
    label_cells: Vec<(String, Vec<(u16, u16)>)>,
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
type Candidate = (usize, usize, ScreenRegion);
fn match_rule(rule: &Rule, screen: &ScreenSnapshot, lines: &[LogicalLine]) -> Option<Matched> {
    match qualifying_regions(rule, screen, lines).as_slice() {
        [candidate] => extract(rule, lines, *candidate),
        _ => None,
    }
}
fn frame_guard_holds(rule: &Rule, lines: &[LogicalLine]) -> bool {
    let frame = rule
        .frame
        .as_ref()
        .expect("annotation rules declare a frame");
    frame
        .require
        .iter()
        .all(|pattern| lines.iter().any(|line| pattern.is_match(&line.text)))
        && !frame
            .reject
            .iter()
            .any(|pattern| lines.iter().any(|line| pattern.is_match(&line.text)))
}
fn match_annotation_blocks(
    rule: &Rule,
    screen: &ScreenSnapshot,
    lines: &[LogicalLine],
) -> Vec<Matched> {
    if !frame_guard_holds(rule, lines) {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let mut next_free = 0;
    for candidate in qualifying_regions(rule, screen, lines) {
        if candidate.0 < next_free {
            continue;
        }
        if let Some(block) = extract(rule, lines, candidate) {
            next_free = candidate.1 + 1;
            blocks.push(block);
        }
    }
    blocks
}
fn qualifying_regions(
    rule: &Rule,
    screen: &ScreenSnapshot,
    lines: &[LogicalLine],
) -> Vec<Candidate> {
    if (rule.region.screen == "primary" && screen.alternate_screen)
        || (rule.region.screen == "alternate" && !screen.alternate_screen)
    {
        return Vec::new();
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
    regions
}
fn extract(rule: &Rule, lines: &[LogicalLine], (start, end, region): Candidate) -> Option<Matched> {
    let selected = &lines[start..=end];
    let region_text = selected
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = BTreeMap::new();
    let mut focuses = BTreeMap::new();
    let mut spans = BTreeMap::new();
    for matcher in &rule.matchers {
        let texts = if matcher.scope == "region" {
            vec![region_text.as_str()]
        } else {
            selected.iter().map(|line| line.text.as_str()).collect()
        };
        let mut captured_values = Vec::new();
        for (line_index, text) in texts.into_iter().enumerate() {
            for captures in matcher.regex.captures_iter(text) {
                let mut located = BTreeMap::new();
                let named = matcher
                    .regex
                    .capture_names()
                    .flatten()
                    .filter_map(|name| {
                        captures.name(name).map(|v| {
                            located.insert(name.to_owned(), (line_index, v.start(), v.end()));
                            (name.into(), v.as_str().into())
                        })
                    })
                    .collect::<BTreeMap<String, String>>();
                let full = captures
                    .get(0)
                    .expect("regex match has full capture")
                    .as_str();
                captured_values.push((named, full.to_owned(), located));
            }
        }
        if captured_values.len() != 1 {
            return None;
        }
        let (named, full, located) = captured_values.pop().expect("single matcher capture");
        focuses.insert(
            matcher.id.clone(),
            serde_json::to_string(&(&matcher.id, &full, &named)).expect("focus strings serialize"),
        );
        if matcher.scope == "line" {
            spans.insert(matcher.id.clone(), located);
        }
        values.insert(matcher.id.clone(), named);
    }
    let label_cells = rule
        .choices
        .iter()
        .filter_map(|choice| {
            let &(line, start, end) = spans.get(&choice.label.matcher)?.get(&choice.label.name)?;
            let boundaries = &selected[line].boundaries;
            let mut cells = boundaries[start..end].to_vec();
            cells.dedup();
            Some((choice.id.clone(), cells))
        })
        .collect();
    let operation = match &rule.operation {
        Some(reference) => {
            let value = captured(reference, &values)?;
            Some(match &reference.row_margin {
                None => value,
                Some(_) if rule.kind.is_annotation() => value,
                Some(margin) => value
                    .split('\n')
                    .map(|row| row.strip_prefix(margin.as_str()))
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
            })
        }
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
    let mode = values
        .values()
        .find_map(|named| named.get("mode"))
        .filter(|mode| !mode.is_empty())
        .cloned();
    Some(Matched {
        rule: rule.clone(),
        region,
        operation,
        choices,
        native,
        mode,
        excerpt: region_text,
        focuses,
        label_cells,
    })
}
/// The focused choice is the one label painted on a single background that differs from its
/// row's background at the region's left edge, with no other label carrying such a background.
fn background_focus(screen: &ScreenSnapshot, matched: &Matched) -> Option<String> {
    let mut focused = Vec::new();
    for (choice, cells) in &matched.label_cells {
        let &(row, _) = cells.first()?;
        let row_background = screen.screen.cell(row, matched.region.left)?.bgcolor();
        let backgrounds = cells
            .iter()
            .map(|&(row, col)| screen.screen.cell(row, col).map(|cell| cell.bgcolor()))
            .collect::<Option<Vec<_>>>()?;
        if backgrounds.iter().all(|bg| *bg == row_background) {
            continue;
        }
        if backgrounds.iter().any(|bg| *bg != backgrounds[0]) {
            return None;
        }
        focused.push(choice);
    }
    match focused.as_slice() {
        [choice] => Some((*choice).clone()),
        _ => None,
    }
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
        let capture = matched
            .focuses
            .get(plan.focus_matcher_id())
            .expect("validated focus matcher")
            .clone();
        let background = rule
            .matchers
            .iter()
            .find(|matcher| matcher.id == plan.focus_matcher_id())
            .expect("validated focus matcher")
            .background_focus;
        let focused_choice = background
            .then(|| background_focus(screen, &matched))
            .flatten();
        let focus = if background {
            serde_json::to_string(&(&capture, &focused_choice)).expect("focus strings serialize")
        } else {
            capture
        };
        Ok(Some(AnswerMatch {
            plan: plan.clone(),
            semantic: observed
                .fingerprint
                .expect("complete permission fingerprint"),
            choices: matched.choices,
            focus,
            focused_choice,
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
    fn annotation_blocks(
        &self,
        profile: &Profile,
        screen: &ScreenSnapshot,
        lines: &[LogicalLine],
        unmatched_evidence: &Value,
    ) -> Vec<AnnotationBlock> {
        let Some(group) = &profile.annotations else {
            return Vec::new();
        };
        let mut blocks = profile
            .rules
            .iter()
            .filter(|rule| group.rules.contains(&rule.id))
            .flat_map(|rule| match_annotation_blocks(rule, screen, lines))
            .map(|matched| {
                let mut raw_evidence = unmatched_evidence.clone();
                raw_evidence["rule_id"] = json!(matched.rule.id);
                raw_evidence["operation"] = json!(matched.operation);
                raw_evidence["operation_rows"] = json!(
                    matched
                        .operation
                        .as_ref()
                        .map(|text| text.split('\n').count())
                );
                raw_evidence["excerpt"] = json!(matched.excerpt);
                raw_evidence["sensitive"] = json!(matched.rule.sensitive);
                raw_evidence["observed_mode"] = json!(matched.mode);
                AnnotationBlock {
                    kind: matched.rule.kind,
                    fingerprint: AnnotationFingerprint {
                        backend: self.backend.name().into(),
                        profile_version: profile.version.clone(),
                        rule_id: matched.rule.id.clone(),
                        shown: normalized_shown(
                            matched.operation.as_deref().unwrap_or(&matched.excerpt),
                            matched
                                .rule
                                .operation
                                .as_ref()
                                .and_then(|capture| capture.row_margin.as_deref()),
                        ),
                    },
                    rule_id: matched.rule.id,
                    matched_region: matched.region,
                    raw_evidence,
                }
            })
            .collect::<Vec<_>>();
        blocks.sort_by_key(|block| (block.matched_region.top, block.rule_id.clone()));
        blocks
    }
    pub(crate) fn observe(&self, screen: &ScreenSnapshot) -> Result<ScreenObservation> {
        let profile_version = self.profile.as_ref().map(|p| p.version.clone());
        let mut observation = ScreenObservation {
            kind: ObservationKind::Unknown,
            profile_version: profile_version.clone(),
            rule_id: "unmatched-screen".into(),
            match_evidence: MatchEvidence::Absent,
            matched_region: None,
            raw_evidence: json!({"backend":self.backend.name(),"native_version":self.version,"profile_version":profile_version,"rule_id":"unmatched-screen","screen_seq":screen.screen_seq,"operation":null,"operation_rows":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}),
            fingerprint: None,
            verified_noninteraction: false,
            native_correlation: None,
            output_seq: screen.screen_seq,
            continues_rule_id: None,
            annotations: Vec::new(),
            guarded_annotation_rules: Vec::new(),
        };
        let Some(profile) = &self.profile else {
            return Ok(observation);
        };
        let lines = logical_lines(screen)?;
        observation.annotations =
            self.annotation_blocks(profile, screen, &lines, &observation.raw_evidence);
        observation.guarded_annotation_rules = profile
            .rules
            .iter()
            .filter(|rule| {
                profile
                    .annotations
                    .as_ref()
                    .is_some_and(|group| group.rules.contains(&rule.id))
                    && frame_guard_holds(rule, &lines)
            })
            .map(|rule| rule.id.clone())
            .collect();
        let mut matched = profile
            .rules
            .iter()
            .filter(|rule| {
                rule.continuation.is_some()
                    || [profile.detection.as_ref(), profile.composer.as_ref()]
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
        if observation.kind == ObservationKind::PermissionContinuation {
            observation.continues_rule_id = best
                .rule
                .continuation
                .as_ref()
                .map(|continuation| continuation.rule_id.clone());
        }
        observation.raw_evidence["rule_id"] = json!(best.rule.id);
        let operation = if conflict {
            None
        } else {
            best.operation.as_deref()
        };
        observation.raw_evidence["operation"] = json!(operation);
        observation.raw_evidence["operation_rows"] =
            json!(operation.map(|text| text.split('\n').count()));
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
            ObservationKind::Interaction(_)
                | ObservationKind::PermissionDenied
                | ObservationKind::PermissionContinuation
        ) {
            observation.fingerprint = Some(SemanticFingerprint { parts });
        }
        Ok(observation)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::coding_agent::profiles::catalog::test_support::{
        VERSION, simple_definition, synthetic_definitions,
    };
    use crate::runtime::runner::{TerminalSize, TerminalState};
    pub(crate) fn replay_native_fixture(
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
            rows: u16::try_from(screen["rows"].as_u64().unwrap()).unwrap(),
            cols: u16::try_from(screen["cols"].as_u64().unwrap()).unwrap(),
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
        let backgrounds = background_rewrite(&screen, &terminal.snapshot());
        terminal.feed(&backgrounds).unwrap();
        let (row, col) = cursor.unwrap_or((
            u16::try_from(screen["cursor_row"].as_u64().unwrap()).unwrap(),
            u16::try_from(screen["cursor_col"].as_u64().unwrap()).unwrap(),
        ));
        terminal
            .feed(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes())
            .unwrap();
        terminal.snapshot()
    }

    /// Repaints each `cell_backgrounds` run over the already written text, so the replayed
    /// cells keep their captured contents and gain the captured background.
    pub(crate) fn background_rewrite(fixture: &Value, text: &ScreenSnapshot) -> Vec<u8> {
        let mut bytes = Vec::new();
        let Some(runs) = fixture.get("cell_backgrounds") else {
            return bytes;
        };
        for run in runs.as_array().unwrap() {
            let row = u16::try_from(run["row"].as_u64().unwrap()).unwrap();
            let start = u16::try_from(run["start_col"].as_u64().unwrap()).unwrap();
            let end = u16::try_from(run["end_col"].as_u64().unwrap()).unwrap();
            let bg = &run["bg"];
            let sgr = match bg["kind"].as_str().unwrap() {
                "rgb" => format!(
                    "\x1b[48;2;{};{};{}m",
                    bg["r"].as_u64().unwrap(),
                    bg["g"].as_u64().unwrap(),
                    bg["b"].as_u64().unwrap()
                ),
                "indexed" => format!("\x1b[48;5;{}m", bg["index"].as_u64().unwrap()),
                other => panic!("unknown fixture colour kind {other}"),
            };
            bytes.extend(format!("\x1b[{};{}H{sgr}", row + 1, start + 1).into_bytes());
            for col in start..end {
                let cell = text.screen.cell(row, col).expect("run inside geometry");
                if cell.is_wide_continuation() {
                    continue;
                }
                if cell.has_contents() {
                    bytes.extend(cell.contents().as_bytes());
                } else {
                    bytes.push(b' ');
                }
            }
            bytes.extend(b"\x1b[0m");
        }
        bytes
    }
    pub(crate) fn lines(operation: &str) -> Vec<String> {
        vec![
            "Synthetic permission request".into(),
            format!("Operation: {operation}"),
            "> Allow once".into(),
            "  Deny once".into(),
            "Request form: synthetic-only".into(),
            "Selection: ".into(),
        ]
    }

    pub(crate) fn terminal(lines: &[String], cols: u16, alternate: bool) -> TerminalState {
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

    pub(crate) fn observation(definition: &Value, screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::from_definitions(definition).unwrap();
        Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
            .unwrap()
            .observe(screen)
            .unwrap()
    }

    pub(crate) fn permission(operation: &str) -> ScreenObservation {
        observation(
            &synthetic_definitions(),
            &terminal(&lines(operation), 80, false).snapshot(),
        )
    }
    pub(crate) fn state_observation(kind: &str, verified: bool) -> ScreenObservation {
        observation(
            &simple_definition(kind, verified),
            &terminal(&["Synthetic state".into(), "State end".into()], 80, false).snapshot(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::broker::runtime::{InteractionClass, PermissionDecision};
    use crate::coding_agent::profiles::catalog::test_support::synthetic_definitions as definitions;
    use crate::coding_agent::profiles::catalog::test_support::*;
    use crate::coding_agent::profiles::*;
    use crate::runtime::runner::*;
    use serde_json::{Value, json};

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
        assert_eq!(idle.raw_evidence["operation"], Value::Null);
        assert_eq!(idle.raw_evidence["operation_rows"], Value::Null);
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
        assert_eq!(observation.raw_evidence["operation_rows"], 1);
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
        let catalog = ProfileCatalog::bundled().unwrap();
        for version in ["2.1.301", "2.1.306"] {
            let fallback = Detector::new(&catalog, CodingAgent::Claude, Some(version))
                .unwrap()
                .observe(&screen)
                .unwrap();
            assert_eq!(fallback.kind, observation.kind, "{version}");
            assert_eq!(fallback.rule_id, observation.rule_id, "{version}");
            assert_eq!(
                fallback.profile_version, observation.profile_version,
                "{version}"
            );
            assert_eq!(
                fallback.raw_evidence["operation"], observation.raw_evidence["operation"],
                "{version}"
            );
            assert_eq!(fallback.raw_evidence["native_version"], version);
        }
        let unprobed = Detector::new(&catalog, CodingAgent::Claude, None)
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_eq!(unprobed.kind, ObservationKind::Unknown);
        assert_eq!(unprobed.raw_evidence["operation"], Value::Null);
        assert_eq!(unprobed.raw_evidence["operation_rows"], Value::Null);
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

    const WRAPPED_SCREEN: &str = include_str!("native/claude-2.1.302-wrapped-permission-1.json");
    const WRAPPED_ALLOW_SCREEN: &str =
        include_str!("native/claude-2.1.302-wrapped-allow-pre-stage-1.json");
    const WRAPPED_DENY_SCREEN: &str =
        include_str!("native/claude-2.1.302-wrapped-deny-pre-stage-1.json");
    const WRAPPED_OPERATION: &str =
        "/tmp/cafleet-194/claude-wrap-cells-001/permission-\nprobe allow";
    const WRAPPED_COMMAND: &str = "/tmp/cafleet-194/claude-wrap-cells-001/permission-probe allow";
    const NATIVE_OPERATION: &str = "/private/tmp/cafleet-194/cells-002/permission-probe allow";

    fn wrapped_screen(change: impl FnOnce(&mut Vec<String>), alternate: bool) -> ScreenSnapshot {
        replay_native_fixture(WRAPPED_SCREEN, change, alternate, None)
    }

    // Replaces the 120-column fixture's single operation row (row 18) with `rows`, taking the extra
    // rows from the empty masked header so every later row keeps its position.
    fn native_with_operation_rows(rows: &[&str]) -> ScreenSnapshot {
        native_screen(
            |lines| {
                let at = 18 - (rows.len() - 1);
                lines.drain(0..rows.len() - 1);
                lines.remove(at);
                for (offset, &row) in rows.iter().enumerate() {
                    lines.insert(at + offset, row.to_owned());
                }
            },
            true,
        )
    }

    fn unwrapped_rendering(operation: &str) -> ScreenSnapshot {
        native_with_operation_rows(&[&format!(" {operation}")])
    }

    fn assert_claude_request(observation: &ScreenObservation, operation: &str) {
        assert_eq!(
            observation.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest),
            "{operation}"
        );
        assert_eq!(
            observation.match_evidence,
            MatchEvidence::Complete,
            "{operation}"
        );
        assert_eq!(observation.rule_id, "claude-bash-permission");
        assert_eq!(
            observation.profile_version.as_deref(),
            Some("claude-2.1.302-auto-1")
        );
        assert_eq!(observation.raw_evidence["operation"], operation);
        assert_eq!(
            observation.raw_evidence["operation_rows"],
            operation.split('\n').count(),
            "each displayed operation row is one line: {operation}"
        );
        assert_eq!(observation.raw_evidence["choices"], json!(["Yes", "No"]));
        assert!(observation.fingerprint.is_some());
        assert!(!observation.verified_noninteraction);
        assert!(observation.native_correlation.is_none());
    }

    #[test]
    fn bundled_claude_detects_the_wrapped_native_form_and_reports_its_displayed_operation_rows() {
        let screen = wrapped_screen(|_| {}, true);
        assert_eq!((screen.rows, screen.cols), (36, 52));
        assert_eq!((screen.cursor_row, screen.cursor_col), (28, 1));
        let observation = claude_observation(&screen);
        assert_claude_request(&observation, WRAPPED_OPERATION);
        let region = observation
            .matched_region
            .expect("a wrapped permission match locates its live region");
        assert_eq!(
            (region.top, region.bottom),
            (17, 36),
            "the region runs from the heading to the closing line: {region:?}"
        );
        for (fixture, operation) in [
            (
                WRAPPED_ALLOW_SCREEN,
                "/tmp/cafleet-194/claude-wrap-ans-allow-001/permiss\nion-probe allow",
            ),
            (
                WRAPPED_DENY_SCREEN,
                "/tmp/cafleet-194/claude-wrap-ans-deny-001/permissi\non-probe deny",
            ),
        ] {
            let pre_stage = claude_observation(&replay_native_fixture(fixture, |_| {}, true, None));
            assert_claude_request(&pre_stage, operation);
            assert_eq!(pre_stage.raw_evidence["operation_rows"], 2);
        }
        let catalog = ProfileCatalog::bundled().unwrap();
        let unprobed = Detector::new(&catalog, CodingAgent::Claude, None)
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_eq!(unprobed.kind, ObservationKind::Unknown);
        let fallback = Detector::new(&catalog, CodingAgent::Claude, Some("2.1.306"))
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_ne!(fallback.kind, ObservationKind::Unknown);
    }

    #[test]
    fn bundled_claude_unwrapped_native_forms_stay_detected_under_the_wrapped_profile() {
        assert_claude_request(
            &claude_observation(&native_screen(|_| {}, true)),
            NATIVE_OPERATION,
        );
        for (fixture, operation) in [
            (
                ALLOW_SCREEN,
                "/private/tmp/cafleet-194/ans-allow-001/permission-probe allow",
            ),
            (
                DENY_SCREEN,
                "/private/tmp/cafleet-194/ans-deny-001/permission-probe deny",
            ),
        ] {
            assert_claude_request(
                &claude_observation(&replay_native_fixture(fixture, |_| {}, true, None)),
                operation,
            );
        }
    }

    #[test]
    fn bundled_claude_wrapped_identity_follows_the_displayed_rows_so_a_rewrap_replaces_it() {
        let wrapped = claude_observation(&wrapped_screen(|_| {}, true));
        let unwrapped = claude_observation(&unwrapped_rendering(WRAPPED_COMMAND));
        assert_claude_request(&unwrapped, WRAPPED_COMMAND);
        assert!(
            wrapped.fingerprint != unwrapped.fingerprint,
            "the 52-column and 120-column renderings display different rows"
        );
        assert!(
            wrapped.fingerprint == claude_observation(&wrapped_screen(|_| {}, true)).fingerprint,
            "a redraw of the same displayed rows keeps the identity"
        );
        let native = claude_observation(&native_screen(|_| {}, true));
        let rewrapped = claude_observation(&wrapped_screen(
            |lines| {
                lines[22] = " /private/tmp/cafleet-194/cells-002/permission-prob".into();
                lines[23] = " e allow".into();
            },
            true,
        ));
        assert_claude_request(
            &rewrapped,
            "/private/tmp/cafleet-194/cells-002/permission-prob\ne allow",
        );
        assert!(
            rewrapped.fingerprint != native.fingerprint,
            "the native 120-column request rewrapped at 52 columns is a new episode"
        );
        let rows = [
            " /tmp/cafleet-194/claude-wrap-cells-001",
            " permission-probe allow",
        ];
        let narrow = claude_observation(&wrapped_screen(
            |lines| {
                lines[22] = rows[0].into();
                lines[23] = rows[1].into();
            },
            true,
        ));
        let wide = claude_observation(&native_with_operation_rows(&rows));
        assert_claude_request(
            &narrow,
            "/tmp/cafleet-194/claude-wrap-cells-001\npermission-probe allow",
        );
        assert!(
            narrow.fingerprint == wide.fingerprint,
            "the same displayed rows at 52 and 120 columns are one request"
        );
    }

    #[test]
    fn bundled_claude_plain_operation_rows_keep_their_line_break_wherever_a_row_ends() {
        let full = " /tmp/cafleet-194/claude-wrap-cells-001/permission-";
        let full_at_space = " /tmp/cafleet-194/claude-wrap-cells-001/probe-boxes";
        for row in [full, full_at_space] {
            assert_eq!(row.chars().count(), 51, "reaches the last usable column");
        }
        for (first, second) in [
            (
                " /tmp/cafleet-194/claude-wrap-cells-001",
                " permission-probe allow",
            ),
            (full, " probe allow"),
            (full_at_space, " allow"),
        ] {
            let observation = claude_observation(&wrapped_screen(
                |lines| {
                    lines[22] = first.into();
                    lines[23] = second.into();
                },
                true,
            ));
            assert_claude_request(&observation, &format!("{}\n{}", &first[1..], &second[1..]));
        }
        for filler in [28, 27] {
            let row = format!(" /tmp/cafleet-194/界界/{}", "a".repeat(filler));
            assert_eq!(
                row.chars().count() + 2,
                23 + filler,
                "each 界 occupies two display cells"
            );
            let observation = claude_observation(&wrapped_screen(
                |lines| {
                    lines[22] = row;
                    lines[23] = " b allow".into();
                },
                true,
            ));
            assert_claude_request(
                &observation,
                &format!("/tmp/cafleet-194/界界/{}\nb allow", "a".repeat(filler)),
            );
        }
    }

    #[test]
    fn bundled_claude_operation_rows_remove_exactly_one_margin_space() {
        for (rows, operation) in [
            (
                &["  indented continuation", " plain"][..],
                " indented continuation\nplain",
            ),
            (
                &[" │ echo plain bar", " │ next"][..],
                "│ echo plain bar\n│ next",
            ),
        ] {
            let observation = claude_observation(&native_with_operation_rows(rows));
            assert_claude_request(&observation, operation);
        }
    }

    #[test]
    fn bundled_claude_operation_is_only_the_rows_between_the_dashed_rules() {
        let screen = serde_json::from_str::<Value>(WRAPPED_SCREEN).unwrap();
        let transcript = screen["lines"][13].as_str().unwrap();
        assert!(transcript.starts_with("  ⎿  $ "));
        assert_eq!(
            transcript.chars().count(),
            52,
            "the transcript row above the heading fills the width"
        );
        let base = " Run the permission probe with the allow argument";
        let description = format!("{base}{}", "x".repeat(51 - base.chars().count()));
        let observation = claude_observation(&wrapped_screen(
            |lines| {
                assert_eq!(lines[20], base);
                lines[20] = description;
            },
            true,
        ));
        assert_claude_request(&observation, WRAPPED_OPERATION);
    }

    #[test]
    fn bundled_claude_synthetic_boxed_operation_rows_keep_their_gutter_and_each_line_break() {
        for rows in [
            &[
                " │ /tmp/cafleet-194/claude-wrap-cells-001/permission-probe",
                " │ allow",
            ][..],
            &[
                " │ /tmp/cafleet-194/claude-wrap-cells-001/permission-probe",
                " │ allow --reason",
                " │ boxed",
            ][..],
        ] {
            let operation = rows
                .iter()
                .map(|row| row.strip_prefix(' ').unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let boxed = claude_observation(&native_with_operation_rows(rows));
            assert_claude_request(&boxed, &operation);
            assert_eq!(boxed.raw_evidence["operation_rows"], rows.len());
            let command = operation.replace("\n│ ", " ").replacen("│ ", "", 1);
            assert!(
                boxed.fingerprint != claude_observation(&unwrapped_rendering(&command)).fingerprint,
                "a boxed wrap of {} rows is width-dependent",
                rows.len()
            );
        }
    }

    #[test]
    fn bundled_claude_boxed_row_reaching_the_last_usable_column_keeps_its_line_break() {
        let wide_path = format!("/tmp/cafleet-194/{}/permission-probe", "w".repeat(82));
        let wide_row = format!(" │ {wide_path}");
        assert_eq!(wide_row.chars().count(), 119, "reaches column 119 of 120");
        let wide = claude_observation(&native_with_operation_rows(&[&wide_row, " │ allow"]));
        assert_claude_request(&wide, &format!("│ {wide_path}\n│ allow"));
        let narrow_row = " │ /tmp/cafleet-194/claude-wrap-cells-001/probe-box";
        assert_eq!(narrow_row.chars().count(), 51, "reaches column 51 of 52");
        let narrow = claude_observation(&wrapped_screen(
            |lines| {
                lines[22] = narrow_row.into();
                lines[23] = " │ allow".into();
            },
            true,
        ));
        assert_claude_request(
            &narrow,
            "│ /tmp/cafleet-194/claude-wrap-cells-001/probe-box\n│ allow",
        );
    }

    #[test]
    fn bundled_claude_multi_line_operation_keeps_its_lines_in_the_plain_and_boxed_shapes() {
        let plain = claude_observation(&wrapped_screen(
            |lines| {
                lines[22] = " echo ok # cleanup".into();
                lines[23] = " rm -rf ~/x".into();
            },
            true,
        ));
        let unwrapped_plain = claude_observation(&native_with_operation_rows(&[
            " echo ok # cleanup",
            " rm -rf ~/x",
        ]));
        let boxed = claude_observation(&native_with_operation_rows(&[
            " │ echo ok # cleanup",
            " │ rm -rf ~/x",
        ]));
        let one_line = claude_observation(&unwrapped_rendering("echo ok # cleanup rm -rf ~/x"));
        for (shape, observation, operation) in [
            (
                "plain at 52 columns",
                &plain,
                "echo ok # cleanup\nrm -rf ~/x",
            ),
            (
                "plain at 120 columns",
                &unwrapped_plain,
                "echo ok # cleanup\nrm -rf ~/x",
            ),
            (
                "boxed at 120 columns",
                &boxed,
                "│ echo ok # cleanup\n│ rm -rf ~/x",
            ),
        ] {
            assert_claude_request(observation, operation);
            assert!(
                observation.fingerprint != one_line.fingerprint,
                "{shape} never reads as one commented-out line"
            );
        }
        assert!(plain.fingerprint == unwrapped_plain.fingerprint);
        assert!(
            plain.fingerprint != boxed.fingerprint,
            "the boxed rows display their gutter"
        );
    }

    const BOXED_SCREEN: &str = include_str!("native/claude-2.1.302-boxed-permission-1.json");
    const BOXED_SINGLE_ROW_SCREEN: &str =
        include_str!("native/claude-2.1.302-boxed-single-row-permission-1.json");
    const BOXED_PATH: &str =
        "/tmp/cafleet-194/claude-boxed-cells-089-wide-operation-probe-01/permission-probe";
    const BOXED_SINGLE_ROW_COMMAND: &str =
        "/tmp/cafleet-194/claude-boxed-cells-120-wide-operation-probe-01/permission-probe allow";

    #[test]
    fn bundled_claude_detects_the_native_boxed_form_and_reports_its_displayed_gutter_rows() {
        let screen = replay_native_fixture(BOXED_SCREEN, |_| {}, true, None);
        assert_eq!((screen.rows, screen.cols), (36, 89));
        assert_eq!((screen.cursor_row, screen.cursor_col), (27, 1));
        let observation = claude_observation(&screen);
        assert_claude_request(&observation, &format!("│ {BOXED_PATH}\n│ allow"));
        assert_eq!(observation.raw_evidence["operation_rows"], 2);
        let region = observation
            .matched_region
            .expect("a boxed permission match locates its live region");
        assert_eq!((region.top, region.bottom), (17, 35), "{region:?}");
        assert!(
            observation.fingerprint
                != claude_observation(&unwrapped_rendering(&format!("{BOXED_PATH} allow")))
                    .fingerprint,
            "a boxed wrap is width-dependent"
        );
    }

    #[test]
    fn bundled_claude_boxed_form_tolerates_the_written_spaces_before_its_closing_line() {
        let fixture = serde_json::from_str::<Value>(BOXED_SCREEN).unwrap();
        assert_eq!(fixture["lines"][32], "   4. No");
        assert_eq!(
            fixture["lines"][33], "  ",
            "the native row holds two spaces"
        );
        assert_eq!(fixture["lines"][34], " Esc to cancel · Tab to amend");
        let operation = format!("│ {BOXED_PATH}\n│ allow");
        let cleared = claude_observation(&replay_native_fixture(
            BOXED_SCREEN,
            |lines| lines[33].clear(),
            true,
            None,
        ));
        assert_claude_request(&cleared, &operation);
        assert!(
            cleared.fingerprint
                == claude_observation(&replay_native_fixture(BOXED_SCREEN, |_| {}, true, None))
                    .fingerprint
        );
    }

    #[test]
    fn bundled_claude_detects_the_native_single_row_box_and_keeps_its_gutter() {
        let screen = replay_native_fixture(BOXED_SINGLE_ROW_SCREEN, |_| {}, true, None);
        assert_eq!((screen.rows, screen.cols), (36, 120));
        assert_eq!((screen.cursor_row, screen.cursor_col), (23, 1));
        let observation = claude_observation(&screen);
        assert_claude_request(&observation, &format!("│ {BOXED_SINGLE_ROW_COMMAND}"));
        assert_eq!(observation.raw_evidence["operation_rows"], 1);
        let region = observation
            .matched_region
            .expect("a single-row box locates its live region");
        assert_eq!((region.top, region.bottom), (14, 30), "{region:?}");
        assert!(
            observation.fingerprint
                != claude_observation(&unwrapped_rendering(BOXED_SINGLE_ROW_COMMAND)).fingerprint,
            "a single-row box displays a different row from a plain row of one command"
        );
    }

    #[test]
    fn bundled_claude_wrapped_form_keeps_the_false_positive_guards() {
        let permission = ObservationKind::Interaction(InteractionClass::PermissionRequest);
        let primary = claude_observation(&wrapped_screen(|_| {}, false));
        assert_ne!(primary.kind, permission, "primary-screen transcript copy");
        let quoted = claude_observation(&wrapped_screen(
            |lines| lines[15] = " Bash command".into(),
            true,
        ));
        assert_ne!(quoted.kind, permission, "a heading copy above the form");
        let doubled = claude_observation(&wrapped_screen(
            |lines| lines[26] = " Bash command".into(),
            true,
        ));
        assert_ne!(doubled.kind, permission, "a second heading inside the form");
        let unfocused = claude_observation(&wrapped_screen(
            |lines| lines[28] = lines[28].replace('❯', " "),
            true,
        ));
        assert_ne!(unfocused.kind, permission, "form without a focus marker");
        let headless = claude_observation(&wrapped_screen(|lines| lines[17].clear(), true));
        assert_ne!(headless.kind, permission, "form without its heading");
        let outside = claude_observation(&replay_native_fixture(
            WRAPPED_SCREEN,
            |_| {},
            true,
            Some((12, 0)),
        ));
        assert_ne!(outside.kind, permission, "cursor outside the form");
        let grown = |extra: usize| {
            wrapped_screen(
                |lines| {
                    lines.drain(0..extra);
                    for _ in 0..extra {
                        lines.insert(26 - extra, String::new());
                    }
                    assert_eq!(lines[17 - extra], " Bash command");
                    assert_eq!(lines[28], " ❯ 1. Yes");
                },
                true,
            )
        };
        assert_claude_request(&claude_observation(&grown(5)), WRAPPED_OPERATION);
        assert_ne!(
            claude_observation(&grown(6)).kind,
            permission,
            "a form taller than 24 rows"
        );
    }

    #[test]
    fn bundled_claude_answer_plans_match_the_wrapped_pre_stage_forms_and_bind_focus() {
        for (decision, fixture, choice, byte) in [
            (
                PermissionDecision::Allow,
                WRAPPED_ALLOW_SCREEN,
                "allow-once",
                b'1',
            ),
            (
                PermissionDecision::Deny,
                WRAPPED_DENY_SCREEN,
                "deny-once",
                b'4',
            ),
        ] {
            let plan = allow_or_deny_plan(decision);
            assert_eq!(plan.stages().len(), 1);
            assert_eq!(plan.stages()[0].bytes(), &[byte]);
            let screen = replay_native_fixture(fixture, |_| {}, true, None);
            assert_eq!((screen.cursor_row, screen.cursor_col), (27, 1));
            let matched =
                claude_answer(&screen, &plan).expect("wrapped pre-stage form matches its plan");
            assert_eq!(matched.choices(), &["Yes".to_owned(), "No".to_owned()]);
            assert_eq!(matched.plan().choice_id(), choice);
            assert!(!matched.focus().is_empty());
            assert!(Some(matched.semantic()) == claude_observation(&screen).fingerprint.as_ref());
            let moved = replay_native_fixture(
                fixture,
                |lines| {
                    lines[27] = lines[27].replacen('❯', " ", 1);
                    lines[28] = lines[28].replacen("   2.", " ❯ 2.", 1);
                },
                true,
                Some((28, 1)),
            );
            let refocused = claude_answer(&moved, &plan)
                .expect("a moved focus still identifies the wrapped form");
            assert!(refocused.semantic() == matched.semantic());
            assert_ne!(
                refocused.focus(),
                matched.focus(),
                "token binding sees the focus move"
            );
            let unfocused = replay_native_fixture(
                fixture,
                |lines| lines[27] = lines[27].replacen('❯', " ", 1),
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
        assert_eq!(observation.raw_evidence["operation_rows"], 1);
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
        for version in ["0.162.0", "0.164.0"] {
            let fallback = Detector::new(&catalog, CodingAgent::Codex, Some(version))
                .unwrap()
                .observe(&screen)
                .unwrap();
            assert_eq!(fallback.kind, observation.kind, "{version}");
            assert_eq!(fallback.rule_id, observation.rule_id, "{version}");
            assert_eq!(
                fallback.profile_version, observation.profile_version,
                "{version}"
            );
            assert_eq!(fallback.raw_evidence["native_version"], version);
        }
        let unprobed = Detector::new(&catalog, CodingAgent::Codex, None)
            .unwrap()
            .observe(&screen)
            .unwrap();
        assert_eq!(unprobed.kind, ObservationKind::Unknown);
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
        assert_no_debug_or_serialize!(AnswerMatch);
    }
    mod focus_resolution {
        use super::*;

        fn gated_catalog() -> ProfileCatalog {
            ProfileCatalog::from_definitions(&gated_fixture_definition()).unwrap()
        }

        fn screen(bytes: &[u8]) -> ScreenSnapshot {
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            state.feed(bytes).unwrap();
            state.snapshot()
        }

        fn detector() -> Detector {
            Detector::new(&gated_catalog(), CodingAgent::Codex, Some(VERSION)).unwrap()
        }

        fn gated_plan(decision: PermissionDecision) -> AnswerPlan {
            gated_catalog()
                .answer_plan(CodingAgent::Codex, VERSION, GATED_RULE, decision)
                .unwrap()
                .unwrap()
        }

        fn matched(bytes: &[u8], decision: PermissionDecision) -> AnswerMatch {
            detector()
                .match_answer(&screen(bytes), &gated_plan(decision))
                .unwrap()
                .expect("the gated form matches regardless of its focus")
        }

        fn focused(bytes: &[u8]) -> Option<String> {
            let allow = matched(bytes, PermissionDecision::Allow);
            let deny = matched(bytes, PermissionDecision::Deny);
            assert_eq!(
                allow.focused_choice_id(),
                deny.focused_choice_id(),
                "both plans read the same focus matcher"
            );
            allow.focused_choice_id().map(str::to_owned)
        }

        #[test]
        fn focus_style_background_resolves_the_one_focused_choice() {
            for (focus, expected) in [("allow", "allow-once"), ("deny", "deny-once")] {
                let bytes = gated_fixture_form("write disposable.txt", focus);
                assert_eq!(focused(&bytes).as_deref(), Some(expected), "{focus}");
                let observation = detector().observe(&screen(&bytes)).unwrap();
                assert_eq!(
                    observation.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest)
                );
                assert_eq!(observation.rule_id, GATED_RULE);
                assert_eq!(observation.match_evidence, MatchEvidence::Complete);
                assert_eq!(
                    observation.raw_evidence["choices"],
                    json!(["Allow once", "Deny once"])
                );
            }
        }

        #[test]
        fn focus_style_background_resolves_no_choice_for_none_several_or_an_unlisted_option() {
            for (case, bytes) in [
                ("no styled label", gated_fixture_form("write disposable.txt", "none")),
                (
                    "an option without a choice entry",
                    gated_fixture_form("write disposable.txt", "always"),
                ),
                (
                    "several styled choices",
                    gated_fixture_form_styled(
                        "write disposable.txt",
                        true,
                        &["Allow once", "Deny once"],
                        "",
                        FOCUS_SGR,
                    ),
                ),
                (
                    "a label only partly styled",
                    format!(
                        "\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: write disposable.txt\r\n  {FOCUS_SGR}Allow\x1b[0m once   Always allow   Deny once\r\nSelection: "
                    )
                    .into_bytes(),
                ),
            ] {
                assert_eq!(focused(&bytes), None, "{case}");
            }
        }

        #[test]
        fn focus_style_background_compares_backgrounds_not_theme_colours() {
            for (case, always, row, focus) in [
                ("indexed theme", true, "\x1b[48;5;236m", "\x1b[48;5;214m"),
                (
                    "rgb theme",
                    true,
                    "\x1b[48;2;30;30;30m",
                    "\x1b[48;2;10;200;10m",
                ),
                ("two-option layout", false, "\x1b[48;2;30;30;30m", FOCUS_SGR),
                ("default row background", false, "", "\x1b[48;5;4m"),
            ] {
                for (label, expected) in [("Allow once", "allow-once"), ("Deny once", "deny-once")]
                {
                    let bytes = gated_fixture_form_styled(
                        "write disposable.txt",
                        always,
                        &[label],
                        row,
                        focus,
                    );
                    assert_eq!(focused(&bytes).as_deref(), Some(expected), "{case} {label}");
                }
                let unfocused =
                    gated_fixture_form_styled("write disposable.txt", always, &[], row, focus);
                assert_eq!(focused(&unfocused), None, "{case} on the row background");
            }
        }

        #[test]
        fn styling_only_focus_change_changes_the_request_focus_but_not_its_semantics() {
            let allow = gated_fixture_form("write disposable.txt", "allow");
            let deny = gated_fixture_form("write disposable.txt", "deny");
            let always = gated_fixture_form("write disposable.txt", "always");
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let first = matched(&allow, decision);
                let moved = matched(&deny, decision);
                let unlisted = matched(&always, decision);
                assert!(first.semantic() == moved.semantic());
                assert_eq!(first.choices(), moved.choices());
                assert_ne!(first.focus(), moved.focus(), "{decision:?}");
                assert_ne!(first.focus(), unlisted.focus(), "{decision:?}");
                assert!(!first.same_request(&moved));
                assert!(first.same_request(&matched(&allow, decision)));
            }
            let detector = detector();
            let a = detector.observe(&screen(&allow)).unwrap();
            let b = detector.observe(&screen(&deny)).unwrap();
            assert!(
                a.fingerprint == b.fingerprint,
                "a focus move keeps the episode identity"
            );
        }

        #[test]
        fn continuation_rule_observes_only_the_empty_reason_dialog() {
            let detector = detector();
            let empty = detector
                .observe(&screen(&gated_reason_dialog(None)))
                .unwrap();
            assert_eq!(empty.kind, ObservationKind::PermissionContinuation);
            assert_eq!(empty.rule_id, GATED_REASON_RULE);
            assert_eq!(empty.match_evidence, MatchEvidence::Complete);
            assert!(!empty.verified_noninteraction);
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(
                    detector
                        .match_answer(&screen(&gated_reason_dialog(None)), &gated_plan(decision))
                        .unwrap()
                        .is_none(),
                    "a continuation is not an answerable request form"
                );
            }
            for reason in ["typed reason", "(empty) and more"] {
                let typed = detector
                    .observe(&screen(&gated_reason_dialog(Some(reason))))
                    .unwrap();
                assert_ne!(
                    typed.kind,
                    ObservationKind::PermissionContinuation,
                    "{reason}"
                );
                assert_ne!(typed.rule_id, GATED_REASON_RULE, "{reason}");
            }
        }
    }

    fn assert_permission(value: &ScreenObservation) {
        assert!(matches!(
            value.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        ));
        assert!(matches!(value.match_evidence, MatchEvidence::Complete));
        assert!(value.fingerprint.is_some());
    }

    fn assert_incomplete(value: &ScreenObservation) {
        assert!(!matches!(
            value.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        ));
    }

    fn wrapped(operation: &str, cols: u16) -> ScreenSnapshot {
        let mut state = TerminalState::new(TerminalSize { rows: 24, cols }).unwrap();
        state
            .feed(format!("\x1b[2J\x1b[H{}", lines(operation).join("\r\n")).as_bytes())
            .unwrap();
        state.snapshot()
    }

    #[test]
    fn runtime_detector_complete_synthetic_form_returns_its_live_region_and_semantics() {
        let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
        let value = observation(&definitions(), &screen);
        assert_permission(&value);
        assert_eq!(value.profile_version.as_deref(), Some(PROFILE));
        assert_eq!(value.rule_id, RULE);
        assert_eq!(value.output_seq, screen.screen_seq);
        let region = value.matched_region.unwrap();
        assert_eq!(
            (region.top, region.left, region.bottom, region.right),
            (18, 0, 24, 80)
        );
        assert_eq!(value.raw_evidence["operation"], "write disposable.txt");
        assert_eq!(
            value.raw_evidence["choices"],
            json!(["Allow once", "Deny once"])
        );
        assert!(value.raw_evidence["native_request_id"].is_null());
        assert!(value.raw_evidence.get("cwd").is_none_or(Value::is_null));
        assert!(!value.verified_noninteraction);
    }

    #[test]
    fn runtime_detector_every_required_live_component_is_indispensable() {
        let original = lines("write disposable.txt");
        assert_permission(&observation(
            &definitions(),
            &terminal(&original, 80, false).snapshot(),
        ));
        for index in 0..original.len() {
            let mut changed = original.clone();
            changed[index] = "unrelated output".into();
            assert_incomplete(&observation(
                &definitions(),
                &terminal(&changed, 80, false).snapshot(),
            ));
        }
    }

    #[test]
    fn runtime_detector_missing_or_empty_configured_capture_keeps_the_rule_incomplete() {
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
            json!("^Operation: (?P<operation>.*)$");
        assert_permission(&observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
        ));
        assert_incomplete(&observation(
            &definition,
            &terminal(&lines(""), 80, false).snapshot(),
        ));
        definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
            json!("^Operation: (?:write disposable.txt|(?P<operation>other))$");
        assert_incomplete(&observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
        ));
    }

    #[test]
    fn runtime_detector_duplicate_matchers_or_ambiguous_live_regions_remain_incomplete() {
        let mut duplicate = lines("write disposable.txt");
        duplicate.insert(2, "Operation: another disposable.txt".into());
        assert_incomplete(&observation(
            &definitions(),
            &terminal(&duplicate, 80, false).snapshot(),
        ));
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("any");
        let one = lines("write disposable.txt");
        assert_permission(&observation(
            &definition,
            &terminal(&one, 80, false).snapshot(),
        ));
        let mut two = one.clone();
        two.extend(one);
        assert_incomplete(&observation(
            &definition,
            &terminal(&two, 80, false).snapshot(),
        ));
    }

    #[test]
    fn runtime_detector_region_limits_screen_selection_and_cursor_are_enforced() {
        let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["region"]["max_rows"] = json!(6);
        assert_permission(&observation(&definition, &screen));
        definition["profiles"][0]["rules"][0]["region"]["max_rows"] = json!(5);
        assert_incomplete(&observation(&definition, &screen));
        definition = definitions();
        definition["profiles"][0]["rules"][0]["region"]["screen"] = json!("alternate");
        assert_incomplete(&observation(&definition, &screen));
        assert_permission(&observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, true).snapshot(),
        ));
        let mut terminal = terminal(&lines("write disposable.txt"), 80, false);
        terminal.feed(b"\x1b[1;1H").unwrap();
        assert_incomplete(&observation(&definitions(), &terminal.snapshot()));
        definition = definitions();
        definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("outside");
        assert_permission(&observation(&definition, &terminal.snapshot()));
        definition["profiles"][0]["rules"][0]["region"]["cursor"] = json!("inside");
        assert_incomplete(&observation(&definition, &terminal.snapshot()));
    }

    #[test]
    fn runtime_detector_cursor_endpoint_uses_wide_display_cells() {
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["region"]["end_pattern"] = json!("^Selection界: $");
        let mut rows = lines("write disposable.txt");
        *rows.last_mut().unwrap() = "Selection界: ".into();
        let mut state = terminal(&rows, 80, false);
        assert_eq!(state.snapshot().cursor_col, 13);
        assert_permission(&observation(&definition, &state.snapshot()));
        state.feed(b"\x1b[24;13H").unwrap();
        assert_incomplete(&observation(&definition, &state.snapshot()));
    }

    #[test]
    fn runtime_detector_soft_wrap_preserves_identity_while_significant_spaces_change_it() {
        let operation = "printf 'a significant quoted operation with repeated  internal spaces'";
        let wide = observation(&definitions(), &wrapped(operation, 100));
        let narrow = observation(&definitions(), &wrapped(operation, 40));
        assert_permission(&wide);
        assert_permission(&narrow);
        assert!(wide.fingerprint == narrow.fingerprint);
        assert_eq!(
            wide.raw_evidence["operation"],
            narrow.raw_evidence["operation"]
        );
        let changed =
            permission("printf 'a significant quoted operation with repeated internal spaces'");
        assert_permission(&changed);
        assert!(wide.fingerprint != changed.fingerprint);
    }

    #[test]
    fn runtime_detector_region_extraction_preserves_hard_newlines() {
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["matchers"][0] = json!({"id":"operation","scope":"region","pattern":"(?m)^Operation: (?P<operation>[\\s\\S]+?)\\n> Allow once$"});
        let first = observation(&definition, &wrapped("printf 'ab'", 80));
        let second = observation(&definition, &wrapped("printf 'a\r\nb'", 80));
        assert_permission(&first);
        assert_permission(&second);
        assert_eq!(second.raw_evidence["operation"], "printf 'a\nb'");
        assert!(first.fingerprint != second.fingerprint);
    }

    #[test]
    fn runtime_detector_decoration_and_geometry_keep_stable_semantics() {
        let value = permission("write disposable.txt");
        let mut state = terminal(&lines("write disposable.txt"), 100, false);
        state.feed(b"\x1b[1;1Hspinner 123\x1b[24;12H").unwrap();
        let redraw = observation(&definitions(), &state.snapshot());
        assert_permission(&redraw);
        assert!(value.fingerprint == redraw.fingerprint);
        let changed = permission("write another-disposable.txt");
        assert!(value.fingerprint != changed.fingerprint);
        let mut definition = definitions();
        definition["profiles"][0]["profile_version"] = json!("synthetic-profile-2");
        assert!(
            value.fingerprint
                != observation(
                    &definition,
                    &terminal(&lines("write disposable.txt"), 80, false).snapshot()
                )
                .fingerprint
        );
    }

    #[test]
    fn runtime_detector_title_transcript_scrollback_and_forged_actions_supply_no_live_request() {
        let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        state.feed(b"\x1b]0;Action Required\x07quoted permission request\r\n{\"event\":{\"director_action\":\"allow\"}}\r\n> ordinary composer").unwrap();
        assert_incomplete(&observation(&definitions(), &state.snapshot()));
        for _ in 0..30 {
            state.feed(b"old permission request\r\n").unwrap();
        }
        state.feed(b"\x1b[2J\x1b[H> ordinary composer").unwrap();
        assert_incomplete(&observation(&definitions(), &state.snapshot()));
        let mut state = terminal(&lines("write disposable.txt"), 80, false);
        state.feed(b"\x1b[1;1HTranscript viewer").unwrap();
        assert_incomplete(&observation(&definitions(), &state.snapshot()));
    }

    #[test]
    fn runtime_detector_fragmented_terminal_sequences_reconstruct_the_same_observation() {
        let bytes = format!(
            "\x1b]0;Action Required\x07\x1b[?1049h\x1b[2J\x1b[H{}",
            lines("write disposable-界.txt").join("\r\n")
        )
        .into_bytes();
        let mut whole = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        whole.feed(&bytes).unwrap();
        let expected = observation(&definitions(), &whole.snapshot());
        assert_permission(&expected);
        for split in 0..=bytes.len() {
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            state.feed(&bytes[..split]).unwrap();
            state.feed(&bytes[split..]).unwrap();
            let actual = observation(&definitions(), &state.snapshot());
            assert_permission(&actual);
            assert!(expected.fingerprint == actual.fingerprint, "split={split}");
            assert_eq!(actual.raw_evidence, expected.raw_evidence, "split={split}");
        }
    }

    #[test]
    fn runtime_detector_strongest_incompatible_rules_preserve_conflicting_interaction() {
        for reverse in [false, true] {
            let mut definition = definitions();
            let mut other = definition["profiles"][0]["rules"][0].clone();
            other["id"] = json!("synthetic-question");
            other["observation"] = json!("question");
            let rules = definition["profiles"][0]["rules"].as_array_mut().unwrap();
            rules.push(other);
            if reverse {
                rules.reverse();
            }
            definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
                json!([RULE, "synthetic-question"]);
            let result = observation(
                &definition,
                &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
            );
            assert!(matches!(
                result.kind,
                ObservationKind::Interaction(InteractionClass::UnknownInteraction)
            ));
            assert!(matches!(result.match_evidence, MatchEvidence::Conflicting));
            assert!(result.raw_evidence.is_object());
        }
    }

    #[test]
    fn runtime_detector_lower_priority_rule_cannot_override_the_stronger_permission_form() {
        let mut definition = definitions();
        let mut other = definition["profiles"][0]["rules"][0].clone();
        other["id"] = json!("synthetic-lower");
        other["observation"] = json!("question");
        other["priority"] = json!(99);
        definition["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .insert(0, other);
        definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
            json!([RULE, "synthetic-lower"]);
        assert_permission(&observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
        ));
    }

    #[test]
    fn runtime_detector_equal_priority_compatible_rules_require_equal_extraction() {
        let mut definition = definitions();
        let mut other = definition["profiles"][0]["rules"][0].clone();
        other["id"] = json!("synthetic-other-extraction");
        other["extract"]["operation"] = json!({"matcher_id":"allow","capture":"label"});
        definition["profiles"][0]["rules"]
            .as_array_mut()
            .unwrap()
            .push(other);
        definition["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
            json!([RULE, "synthetic-other-extraction"]);
        let value = observation(
            &definition,
            &terminal(&lines("write disposable.txt"), 80, false).snapshot(),
        );
        assert!(matches!(
            value.kind,
            ObservationKind::Interaction(InteractionClass::UnknownInteraction)
        ));
        assert!(matches!(value.match_evidence, MatchEvidence::Conflicting));
    }

    #[test]
    fn runtime_detector_other_interaction_classes_remain_distinct() {
        for (kind, class) in [
            ("question", InteractionClass::Question),
            ("trust", InteractionClass::Trust),
            ("update", InteractionClass::Update),
            ("unknown_interaction", InteractionClass::UnknownInteraction),
        ] {
            let value = state_observation(kind, false);
            assert!(matches!(value.kind,ObservationKind::Interaction(actual) if actual==class));
            assert!(matches!(value.match_evidence, MatchEvidence::Complete));
        }
    }

    #[test]
    fn runtime_detector_activity_refusal_and_composer_require_their_own_positive_evidence() {
        assert!(matches!(
            state_observation("working", true).kind,
            ObservationKind::Working
        ));
        assert!(matches!(
            state_observation("permission_denied", true).kind,
            ObservationKind::PermissionDenied
        ));
        assert!(matches!(
            state_observation("composer_empty", true).kind,
            ObservationKind::ComposerEmpty
        ));
        assert!(matches!(
            state_observation("composer_draft", true).kind,
            ObservationKind::ComposerDraft
        ));
        let empty = terminal(&[], 80, false).snapshot();
        let unknown = observation(&definitions(), &empty);
        assert!(matches!(unknown.kind, ObservationKind::Unknown));
        assert!(matches!(unknown.match_evidence, MatchEvidence::Absent));
        assert!(unknown.fingerprint.is_none());
    }

    #[test]
    fn runtime_detector_unsupported_version_reports_absent_evidence_without_nearby_profile() {
        let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
        let value = Detector::new(&catalog, CodingAgent::Claude, Some(VERSION))
            .unwrap()
            .observe(&terminal(&lines("write disposable.txt"), 80, false).snapshot())
            .unwrap();
        assert!(matches!(value.kind, ObservationKind::Unknown));
        assert!(matches!(value.match_evidence, MatchEvidence::Absent));
        assert!(value.profile_version.is_none());
        assert!(value.fingerprint.is_none());
    }

    mod annotation {
        use super::*;
        use crate::coding_agent::profiles::catalog::test_support::annotation::*;
        fn observe_lines(definition: &Value, rows: &[&str]) -> ScreenObservation {
            let rows = rows.iter().map(|row| (*row).to_owned()).collect::<Vec<_>>();
            observation(definition, &terminal(&rows, 80, false).snapshot())
        }

        fn refusal_screen(operations: &[&str]) -> Vec<String> {
            let mut rows = Vec::new();
            for operation in operations {
                rows.push(format!("Refused: {operation}"));
                rows.push("Refusal end".to_owned());
            }
            rows.push("Live footer".to_owned());
            rows
        }
        #[test]
        fn annotation_kinds_parse_from_their_exact_profile_strings() {
            for (name, kind) in [
                ("automatic_refusal", ObservationKind::AutomaticRefusal),
                (
                    "automatic_review_unavailable",
                    ObservationKind::AutomaticReviewUnavailable,
                ),
                (
                    "automatic_review_available",
                    ObservationKind::AutomaticReviewAvailable,
                ),
            ] {
                assert_eq!(ObservationKind::parse(name).unwrap(), kind, "{name}");
            }
            for name in [
                "automatic_refusal_observed",
                "Automatic_Refusal",
                "auto_refusal",
            ] {
                assert!(ObservationKind::parse(name).is_err(), "{name}");
            }
        }
        #[test]
        fn annotation_detector_yields_every_qualifying_block_in_row_order_beside_the_primary() {
            let definition = annotated();
            let observed = observe_lines(
                &definition,
                &refusal_screen(&["write one.txt", "write two.txt"])
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(observed.kind, ObservationKind::Unknown);
            assert_eq!(observed.rule_id, "unmatched-screen");
            assert_eq!(observed.annotations.len(), 2);
            let operations = observed
                .annotations
                .iter()
                .map(|block| block.raw_evidence["operation"].clone())
                .collect::<Vec<_>>();
            assert_eq!(operations, [json!("write one.txt"), json!("write two.txt")]);
            for block in &observed.annotations {
                assert_eq!(block.kind, ObservationKind::AutomaticRefusal);
                assert_eq!(block.rule_id, REFUSAL);
                assert_eq!(block.raw_evidence["rule_id"], REFUSAL);
            }
            let (first, second) = (
                observed.annotations[0].matched_region,
                observed.annotations[1].matched_region,
            );
            assert!(first.bottom <= second.top, "{first:?} {second:?}");
            assert!(observed.annotations[0].fingerprint != observed.annotations[1].fingerprint);
            let single = observe_lines(
                &definition,
                &refusal_screen(&["write one.txt"])
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(single.annotations.len(), 1);
            assert!(single.annotations[0].fingerprint == observed.annotations[0].fingerprint);
        }

        #[test]
        fn annotation_detector_leaves_the_primary_observation_unchanged() {
            let mut rows = vec![
                "Live footer".to_owned(),
                "Refused: write outside.txt".to_owned(),
                "Refusal end".to_owned(),
            ];
            rows.extend(lines("write disposable.txt"));
            let screen = terminal(&rows, 80, false).snapshot();
            let plain = observation(&definitions(), &screen);
            let annotated = observation(&annotated(), &screen);
            assert_eq!(
                annotated.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_eq!(annotated.kind, plain.kind);
            assert_eq!(annotated.rule_id, plain.rule_id);
            assert_eq!(annotated.match_evidence, plain.match_evidence);
            assert_eq!(annotated.raw_evidence, plain.raw_evidence);
            assert_eq!(
                annotated.verified_noninteraction,
                plain.verified_noninteraction
            );
            assert!(annotated.fingerprint == plain.fingerprint);
            assert!(plain.annotations.is_empty());
            assert_eq!(annotated.annotations.len(), 1);
            assert_eq!(
                annotated.annotations[0].raw_evidence["operation"],
                "write outside.txt"
            );
        }

        #[test]
        fn annotation_detector_reports_each_annotation_kind_with_its_own_rule() {
            let observed = observe_lines(
                &annotated(),
                &[
                    "Automatic review unavailable",
                    "Mode: automatic",
                    "Live footer",
                ],
            );
            let mut kinds = observed
                .annotations
                .iter()
                .map(|block| (block.rule_id.clone(), block.kind))
                .collect::<Vec<_>>();
            kinds.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(
                kinds,
                [
                    (
                        AVAILABLE.to_owned(),
                        ObservationKind::AutomaticReviewAvailable
                    ),
                    (
                        UNAVAILABLE.to_owned(),
                        ObservationKind::AutomaticReviewUnavailable
                    ),
                ]
            );
            for block in &observed.annotations {
                assert_eq!(block.raw_evidence["operation"], Value::Null);
            }
        }

        #[test]
        fn annotation_detector_frame_guard_rejects_non_live_frames_and_undecorated_quotes() {
            let definition = annotated();
            let without_footer =
                observe_lines(&definition, &["Refused: write one.txt", "Refusal end"]);
            assert!(without_footer.annotations.is_empty());
            let viewer = observe_lines(
                &definition,
                &[
                    "Transcript viewer",
                    "Refused: write one.txt",
                    "Refusal end",
                    "Live footer",
                ],
            );
            assert!(viewer.annotations.is_empty());
            let quoted = observe_lines(
                &definition,
                &["The model said Refused: write one.txt", "Live footer"],
            );
            assert!(quoted.annotations.is_empty());
            let undecorated =
                observe_lines(&definition, &["Refused: write one.txt", "Live footer"]);
            assert!(undecorated.annotations.is_empty());
        }

        #[test]
        fn annotation_detector_yields_nothing_without_a_selected_profile() {
            let catalog = ProfileCatalog::from_definitions(&annotated()).unwrap();
            let rows = refusal_screen(&["write one.txt"]);
            let observed = Detector::new(&catalog, CodingAgent::Claude, Some(VERSION))
                .unwrap()
                .observe(&terminal(&rows, 80, false).snapshot())
                .unwrap();
            assert!(observed.annotations.is_empty());
        }
    }
}
