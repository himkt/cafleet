use super::CodingAgent;
use super::catalog::{
    Capture, CursorRule, MarginRows, MatcherScope, Profile, ProfileCatalog, Rule, ScreenSelector,
};
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
type SemanticPart = (String, String, Option<String>, Vec<String>);
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
    if (rule.region.screen == ScreenSelector::Primary && screen.alternate_screen)
        || (rule.region.screen == ScreenSelector::Alternate && !screen.alternate_screen)
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
            let qualified = match rule.region.cursor {
                CursorRule::AtEnd => {
                    last.boundaries.get(found.end())
                        == Some(&(screen.cursor_row, screen.cursor_col))
                }
                CursorRule::Inside => inside,
                CursorRule::Outside => !inside,
                CursorRule::Any => true,
            };
            if qualified {
                regions.push((start, end, region));
            }
        }
    }
    regions
}
/// Removes the trailer, then the margin from the rows `margin_rows` selects, then trailing
/// whitespace; trimming last keeps a whitespace-only margined row a match.
fn displayed_rows(value: &str, capture: &Capture) -> Option<String> {
    value
        .split('\n')
        .enumerate()
        .map(|(index, row)| {
            let row = match &capture.row_trailer {
                Some(trailer) => row
                    .trim_end()
                    .strip_suffix(trailer.as_str())
                    .map_or(row, str::trim_end),
                None => row,
            };
            let margined = index > 0 || capture.margin_rows == MarginRows::All;
            let row = match capture.row_margin.as_deref().filter(|_| margined) {
                Some(margin) => row.strip_prefix(margin)?,
                None => row,
            };
            Some(row.trim_end())
        })
        .collect::<Option<Vec<_>>>()
        .map(|rows| rows.join("\n"))
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
        let texts = if matcher.scope == MatcherScope::Region {
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
        if matcher.scope == MatcherScope::Line {
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
            let plain = reference.row_margin.is_none() && reference.row_trailer.is_none();
            Some(if plain || rule.kind.is_annotation() {
                value
            } else {
                displayed_rows(&value, reference)?
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
/// The background covering more than half of the row's cells; a layout without a left margin
/// paints its highlight over the edge cells, so no single edge cell is the row background.
fn majority_background(screen: &ScreenSnapshot, row: u16) -> Option<vt100::Color> {
    let mut counts: Vec<(vt100::Color, usize)> = Vec::new();
    for col in 0..screen.cols {
        let background = screen.screen.cell(row, col)?.bgcolor();
        match counts.iter_mut().find(|(seen, _)| *seen == background) {
            Some((_, count)) => *count += 1,
            None => counts.push((background, 1)),
        }
    }
    counts
        .into_iter()
        .find(|&(_, count)| count * 2 > usize::from(screen.cols))
        .map(|(background, _)| background)
}
/// The focused choice is the one label painted on a single background that differs from its
/// row's majority background, with no other label carrying such a background.
fn background_focus(screen: &ScreenSnapshot, matched: &Matched) -> Option<String> {
    let mut focused = Vec::new();
    for (choice, cells) in &matched.label_cells {
        let &(row, _) = cells.first()?;
        let row_background = majority_background(screen, row)?;
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
    ) -> Self {
        Self {
            backend,
            version: version.map(str::to_owned),
            profile: catalog.matching(backend, version).cloned(),
        }
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
        let mut observation = ScreenObservation {
            kind: ObservationKind::Unknown,
            rule_id: "unmatched-screen".into(),
            match_evidence: MatchEvidence::Absent,
            matched_region: None,
            raw_evidence: json!({"backend":self.backend.name(),"native_version":self.version,"rule_id":"unmatched-screen","screen_seq":screen.screen_seq,"operation":null,"operation_rows":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}),
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
                        .any(|cap| cap.rules.contains(&rule.id))
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
        VERSION, bundled_fixture, simple_definition, synthetic_definitions,
    };
    use crate::runtime::runner::{TerminalSize, TerminalState};

    pub(crate) fn bundled_screen(id: &str) -> &'static str {
        std::str::from_utf8(bundled_fixture(id).1).expect("bundled fixtures are UTF-8 JSON")
    }

    pub(crate) fn bundled_observe(
        backend: CodingAgent,
        version: &str,
        id: &str,
    ) -> ScreenObservation {
        let screen = replay_native_fixture(bundled_screen(id), |_| {}, true, None);
        Detector::new(&ProfileCatalog::bundled().unwrap(), backend, Some(version))
            .observe(&screen)
            .unwrap()
    }

    pub(crate) fn replay_native_fixture(
        fixture: &str,
        change: impl FnOnce(&mut Vec<String>),
        alternate: bool,
        cursor: Option<(u16, u16)>,
    ) -> ScreenSnapshot {
        let screen: Value = serde_json::from_str(fixture).unwrap();
        let mut terminal = TerminalState::new(fixture_size(&screen)).unwrap();
        terminal
            .feed(&fixture_bytes(&screen, change, alternate, cursor))
            .unwrap();
        terminal.snapshot()
    }

    /// The byte stream that paints a committed native screen, using its own screen mode and cursor.
    pub(crate) fn native_fixture_bytes(fixture: &str) -> Vec<u8> {
        let screen: Value = serde_json::from_str(fixture).unwrap();
        let alternate = screen["alternate_screen"].as_bool().unwrap();
        fixture_bytes(&screen, |_| {}, alternate, None)
    }

    fn fixture_size(screen: &Value) -> TerminalSize {
        TerminalSize {
            rows: u16::try_from(screen["rows"].as_u64().unwrap()).unwrap(),
            cols: u16::try_from(screen["cols"].as_u64().unwrap()).unwrap(),
        }
    }

    fn fixture_bytes(
        screen: &Value,
        change: impl FnOnce(&mut Vec<String>),
        alternate: bool,
        cursor: Option<(u16, u16)>,
    ) -> Vec<u8> {
        let mut lines: Vec<String> = screen["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        change(&mut lines);
        let mut bytes = Vec::new();
        if alternate {
            bytes.extend(b"\x1b[?1049h");
        }
        bytes.extend(b"\x1b[2J");
        for (row, line) in lines.iter().enumerate() {
            if !line.is_empty() {
                bytes.extend(format!("\x1b[{};1H{line}", row + 1).into_bytes());
            }
        }
        let mut written = TerminalState::new(fixture_size(screen)).unwrap();
        written.feed(&bytes).unwrap();
        bytes.extend(background_rewrite(screen, &written.snapshot()));
        let (row, col) = cursor.unwrap_or((
            u16::try_from(screen["cursor_row"].as_u64().unwrap()).unwrap(),
            u16::try_from(screen["cursor_col"].as_u64().unwrap()).unwrap(),
        ));
        bytes.extend(format!("\x1b[{};{}H", row + 1, col + 1).into_bytes());
        bytes
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

    /// The Claude auto-tip permission form; `padding` written spaces follow the question and
    /// operation rows, as Claude leaves them when it repaints only the changed cells of a row
    /// that was earlier wider.
    fn auto_tip_form(padding: usize) -> ScreenSnapshot {
        let pad = " ".repeat(padding);
        let separator = "╌".repeat(120);
        let rows = [
            " Bash command".to_owned(),
            " Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below"
                .to_owned(),
            " Create empty file wp-mark".to_owned(),
            separator.clone(),
            format!(" touch wp-mark{pad}"),
            separator,
            format!(" Do you want to proceed?{pad}"),
            " ❯ 1. Yes".to_owned(),
            "   2. Yes, and always allow access to /tmp/x from this project".to_owned(),
            "   3. Yes, and switch to auto mode · auto mode handles these prompts for you"
                .to_owned(),
            "   4. No".to_owned(),
            String::new(),
            " Esc to cancel · Tab to amend".to_owned(),
        ];
        let bytes = format!(
            "\x1b[?1049h\x1b[2J\x1b[13;1H{}\x1b[20;2H",
            rows.join("\r\n")
        );
        let mut terminal = TerminalState::new(TerminalSize {
            rows: 35,
            cols: 149,
        })
        .unwrap();
        terminal.feed(bytes.as_bytes()).unwrap();
        terminal.snapshot()
    }

    #[test]
    fn bundled_claude_auto_tip_form_with_written_padding_rows_keeps_its_request_identity() {
        let catalog = ProfileCatalog::bundled().unwrap();
        let detector = Detector::new(&catalog, CodingAgent::Claude, Some("2.1.308"));
        let clean = detector.observe(&auto_tip_form(0)).unwrap();
        assert_eq!(
            clean.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert_eq!(clean.rule_id, "claude-bash-permission-auto-tip");
        assert_eq!(clean.raw_evidence["operation"], "touch wp-mark");
        let padded = detector.observe(&auto_tip_form(40)).unwrap();
        assert_eq!(padded.kind, clean.kind, "padding keeps the form detected");
        assert_eq!(padded.rule_id, clean.rule_id);
        assert_eq!(
            padded.raw_evidence["operation"],
            clean.raw_evidence["operation"]
        );
        assert_eq!(
            padded.raw_evidence["operation_rows"],
            clean.raw_evidence["operation_rows"]
        );
        assert_eq!(
            padded.raw_evidence["choices"],
            clean.raw_evidence["choices"]
        );
        assert!(
            padded.fingerprint == clean.fingerprint,
            "written padding does not change the request identity"
        );
    }

    const READ_OUTSIDE: &str = "claude-2.1.309-read-outside-permission-1";
    const READ_OUTSIDE_RULE: &str = "claude-read-outside-permission";
    const AUTO_IDLE: &str = "claude-2.1.302-auto-idle-composer-1";

    fn read_outside_observation(screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Claude, Some("2.1.309"))
            .observe(screen)
            .unwrap()
    }

    fn read_outside_screen(
        change: impl FnOnce(&mut Vec<String>),
        alternate: bool,
    ) -> ScreenSnapshot {
        replay_native_fixture(bundled_screen(READ_OUTSIDE), change, alternate, None)
    }

    #[test]
    fn bundled_claude_read_outside_dialog_with_trailing_repaint_padding_keeps_its_request_identity()
    {
        let clean = read_outside_observation(&read_outside_screen(|_| {}, true));
        assert_eq!(
            clean.kind,
            ObservationKind::Interaction(InteractionClass::PermissionRequest)
        );
        assert_eq!(clean.rule_id, READ_OUTSIDE_RULE);
        assert_eq!(clean.match_evidence, MatchEvidence::Complete);
        let padded = read_outside_observation(&read_outside_screen(
            |lines| {
                for row in [16, 18, 27, 28, 29, 30, 31, 33] {
                    lines[row].push_str(&" ".repeat(40));
                }
            },
            true,
        ));
        assert_eq!(padded.kind, clean.kind, "padding keeps the dialog detected");
        assert_eq!(padded.rule_id, clean.rule_id);
        assert_eq!(padded.match_evidence, MatchEvidence::Complete);
        for field in ["operation", "operation_rows", "choices"] {
            assert_eq!(
                padded.raw_evidence[field], clean.raw_evidence[field],
                "{field}"
            );
        }
        assert!(
            padded.fingerprint == clean.fingerprint,
            "written padding does not change the request identity"
        );
    }

    #[test]
    fn bundled_claude_read_outside_dialog_outside_a_live_region_or_on_the_idle_composer_is_no_request()
     {
        let permission = ObservationKind::Interaction(InteractionClass::PermissionRequest);
        let dialog: Vec<String> = serde_json::from_str::<Value>(bundled_screen(READ_OUTSIDE))
            .unwrap()["lines"]
            .as_array()
            .unwrap()[16..=33]
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        let quoted = read_outside_observation(&replay_native_fixture(
            bundled_screen(AUTO_IDLE),
            |lines| {
                for (offset, row) in dialog.iter().enumerate() {
                    lines[12 + offset] = row.clone();
                }
            },
            true,
            None,
        ));
        assert_ne!(quoted.kind, permission, "dialog quoted above the composer");
        assert_ne!(quoted.rule_id, READ_OUTSIDE_RULE);
        assert_eq!(quoted.kind, ObservationKind::ComposerEmpty);
        assert!(quoted.fingerprint.is_none());

        let primary = read_outside_observation(&read_outside_screen(|_| {}, false));
        assert_ne!(primary.kind, permission, "primary-screen transcript copy");
        assert_ne!(primary.rule_id, READ_OUTSIDE_RULE);

        let cursor_outside = read_outside_observation(&replay_native_fixture(
            bundled_screen(READ_OUTSIDE),
            |_| {},
            true,
            Some((2, 0)),
        ));
        assert_ne!(
            cursor_outside.rule_id, READ_OUTSIDE_RULE,
            "the cursor lies outside the dialog region"
        );

        let idle = read_outside_observation(&replay_native_fixture(
            bundled_screen(AUTO_IDLE),
            |_| {},
            true,
            None,
        ));
        assert_eq!(idle.kind, ObservationKind::ComposerEmpty);
        assert_eq!(idle.rule_id, "claude-idle-composer");
        assert!(idle.fingerprint.is_none());
    }

    #[test]
    fn permission_inspection_answer_match_uses_complete_current_rule_choice_and_focus() {
        let catalog = ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
        let detector = Detector::new(&catalog, CodingAgent::Codex, Some("0.0.1-synthetic"));
        let plan = catalog
            .answer_plan(
                CodingAgent::Codex,
                "0.0.1-synthetic",
                "synthetic-permission",
                PermissionDecision::Allow,
            )
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

    fn native_screen(change: impl FnOnce(&mut Vec<String>), alternate: bool) -> ScreenSnapshot {
        replay_native_fixture(
            bundled_screen("claude-2.1.302-default-permission-1"),
            change,
            alternate,
            None,
        )
    }
    fn claude_observation(screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Claude, Some(CLAUDE_VERSION))
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
        let reused = Detector::new(&catalog, CodingAgent::Claude, Some("2.1.308"))
            .observe(&screen)
            .unwrap();
        assert_eq!(reused.kind, observation.kind);
        assert_eq!(reused.rule_id, observation.rule_id);
        assert!(reused.fingerprint == observation.fingerprint);
        assert_eq!(
            reused.raw_evidence["operation"],
            observation.raw_evidence["operation"]
        );
        assert_eq!(reused.raw_evidence["native_version"], "2.1.308");
        let below = Detector::new(&catalog, CodingAgent::Claude, Some("2.1.301"))
            .observe(&screen)
            .unwrap();
        assert_eq!(below.kind, ObservationKind::Unknown);
        assert_eq!(below.match_evidence, MatchEvidence::Absent);
        assert!(below.fingerprint.is_none());
        let unprobed = Detector::new(&catalog, CodingAgent::Claude, None)
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

    const WRAPPED_OPERATION: &str =
        "/tmp/cafleet-194/claude-wrap-cells-001/permission-\nprobe allow";
    const WRAPPED_COMMAND: &str = "/tmp/cafleet-194/claude-wrap-cells-001/permission-probe allow";
    const NATIVE_OPERATION: &str = "/private/tmp/cafleet-194/cells-002/permission-probe allow";

    fn wrapped_screen(change: impl FnOnce(&mut Vec<String>), alternate: bool) -> ScreenSnapshot {
        replay_native_fixture(
            bundled_screen("claude-2.1.302-wrapped-permission-1"),
            change,
            alternate,
            None,
        )
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
        assert_eq!(observation.raw_evidence["native_version"], CLAUDE_VERSION);
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
                bundled_screen("claude-2.1.302-wrapped-allow-pre-stage-1"),
                "/tmp/cafleet-194/claude-wrap-ans-allow-001/permiss\nion-probe allow",
            ),
            (
                bundled_screen("claude-2.1.302-wrapped-deny-pre-stage-1"),
                "/tmp/cafleet-194/claude-wrap-ans-deny-001/permissi\non-probe deny",
            ),
        ] {
            let pre_stage = claude_observation(&replay_native_fixture(fixture, |_| {}, true, None));
            assert_claude_request(&pre_stage, operation);
            assert_eq!(pre_stage.raw_evidence["operation_rows"], 2);
        }
        let catalog = ProfileCatalog::bundled().unwrap();
        let unprobed = Detector::new(&catalog, CodingAgent::Claude, None)
            .observe(&screen)
            .unwrap();
        assert_eq!(unprobed.kind, ObservationKind::Unknown);
        let fallback = Detector::new(&catalog, CodingAgent::Claude, Some("2.1.306"))
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
                bundled_screen("claude-2.1.302-allow-pre-stage-1"),
                "/private/tmp/cafleet-194/ans-allow-001/permission-probe allow",
            ),
            (
                bundled_screen("claude-2.1.302-deny-pre-stage-1"),
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
        let screen =
            serde_json::from_str::<Value>(bundled_screen("claude-2.1.302-wrapped-permission-1"))
                .unwrap();
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

    const BOXED_PATH: &str =
        "/tmp/cafleet-194/claude-boxed-cells-089-wide-operation-probe-01/permission-probe";
    const BOXED_SINGLE_ROW_COMMAND: &str =
        "/tmp/cafleet-194/claude-boxed-cells-120-wide-operation-probe-01/permission-probe allow";

    #[test]
    fn bundled_claude_detects_the_native_boxed_form_and_reports_its_displayed_gutter_rows() {
        let screen = replay_native_fixture(
            bundled_screen("claude-2.1.302-boxed-permission-1"),
            |_| {},
            true,
            None,
        );
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
        let fixture =
            serde_json::from_str::<Value>(bundled_screen("claude-2.1.302-boxed-permission-1"))
                .unwrap();
        assert_eq!(fixture["lines"][32], "   4. No");
        assert_eq!(
            fixture["lines"][33], "  ",
            "the native row holds two spaces"
        );
        assert_eq!(fixture["lines"][34], " Esc to cancel · Tab to amend");
        let operation = format!("│ {BOXED_PATH}\n│ allow");
        let cleared = claude_observation(&replay_native_fixture(
            bundled_screen("claude-2.1.302-boxed-permission-1"),
            |lines| lines[33].clear(),
            true,
            None,
        ));
        assert_claude_request(&cleared, &operation);
        assert!(
            cleared.fingerprint
                == claude_observation(&replay_native_fixture(
                    bundled_screen("claude-2.1.302-boxed-permission-1"),
                    |_| {},
                    true,
                    None
                ))
                .fingerprint
        );
    }

    #[test]
    fn bundled_claude_detects_the_native_single_row_box_and_keeps_its_gutter() {
        let screen = replay_native_fixture(
            bundled_screen("claude-2.1.302-boxed-single-row-permission-1"),
            |_| {},
            true,
            None,
        );
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
            bundled_screen("claude-2.1.302-wrapped-permission-1"),
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

    const CODEX_VERSION: &str = "0.163.0";
    const CODEX_CHOICES: [&str; 2] = ["Yes, proceed", "No, and tell Codex what to do differently"];

    fn codex_observation(screen: &ScreenSnapshot) -> ScreenObservation {
        let catalog = ProfileCatalog::bundled().unwrap();
        Detector::new(&catalog, CodingAgent::Codex, Some(CODEX_VERSION))
            .observe(screen)
            .unwrap()
    }

    fn codex_form(
        change: impl FnOnce(&mut Vec<String>),
        cursor: Option<(u16, u16)>,
    ) -> ScreenSnapshot {
        replay_native_fixture(
            bundled_screen("codex-0.163.0-on-request-permission-1"),
            change,
            true,
            cursor,
        )
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
        let reused = Detector::new(&catalog, CodingAgent::Codex, Some("0.164.0"))
            .observe(&screen)
            .unwrap();
        assert_eq!(reused.kind, observation.kind);
        assert_eq!(reused.rule_id, observation.rule_id);
        assert!(reused.fingerprint == observation.fingerprint);
        assert_eq!(reused.raw_evidence["native_version"], "0.164.0");
        let below = Detector::new(&catalog, CodingAgent::Codex, Some("0.162.0"))
            .observe(&screen)
            .unwrap();
        assert_eq!(below.kind, ObservationKind::Unknown);
        assert_eq!(below.match_evidence, MatchEvidence::Absent);
        assert!(below.fingerprint.is_none());
        let unprobed = Detector::new(&catalog, CodingAgent::Codex, None)
            .observe(&screen)
            .unwrap();
        assert_eq!(unprobed.kind, ObservationKind::Unknown);
    }

    #[test]
    fn bundled_codex_detection_counts_a_pending_wrap_cursor_on_the_full_width_closing_row_as_inside()
     {
        let screen = replay_with_pending_wrap(
            bundled_screen("codex-0.163.0-on-request-permission-1"),
            |_| {},
        );
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
        for fixture in [
            bundled_screen("codex-0.163.0-allow-pre-stage-1"),
            bundled_screen("codex-0.163.0-deny-pre-stage-1"),
        ] {
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
            bundled_screen("codex-0.163.0-on-request-permission-1"),
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
                bundled_screen("codex-0.163.0-allow-pre-stage-1"),
                "/tmp/cafleet-194/codex-ans-allow-001/permission-probe allow",
            ),
            (
                bundled_screen("codex-0.163.0-deny-pre-stage-1"),
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

    fn bundled_detector(backend: CodingAgent, version: &str) -> Detector {
        Detector::new(&ProfileCatalog::bundled().unwrap(), backend, Some(version))
    }

    struct PreStageForms {
        backend: CodingAgent,
        version: &'static str,
        rule: &'static str,
        answers: [(PermissionDecision, &'static str, &'static str, u8); 2],
        choices: [&'static str; 2],
        glyph: char,
        focus_row: usize,
        second_choice: (&'static str, &'static str),
        cursor_follows_focus: bool,
    }

    const PRE_STAGE_FORMS: [PreStageForms; 3] = [
        PreStageForms {
            backend: CodingAgent::Claude,
            version: CLAUDE_VERSION,
            rule: "claude-bash-permission",
            answers: [
                (
                    PermissionDecision::Allow,
                    "claude-2.1.302-allow-pre-stage-1",
                    "allow-once",
                    b'1',
                ),
                (
                    PermissionDecision::Deny,
                    "claude-2.1.302-deny-pre-stage-1",
                    "deny-once",
                    b'4',
                ),
            ],
            choices: ["Yes", "No"],
            glyph: '❯',
            focus_row: 23,
            second_choice: ("   2.", " ❯ 2."),
            cursor_follows_focus: true,
        },
        PreStageForms {
            backend: CodingAgent::Claude,
            version: CLAUDE_VERSION,
            rule: "claude-bash-permission",
            answers: [
                (
                    PermissionDecision::Allow,
                    "claude-2.1.302-wrapped-allow-pre-stage-1",
                    "allow-once",
                    b'1',
                ),
                (
                    PermissionDecision::Deny,
                    "claude-2.1.302-wrapped-deny-pre-stage-1",
                    "deny-once",
                    b'4',
                ),
            ],
            choices: ["Yes", "No"],
            glyph: '❯',
            focus_row: 27,
            second_choice: ("   2.", " ❯ 2."),
            cursor_follows_focus: true,
        },
        PreStageForms {
            backend: CodingAgent::Codex,
            version: CODEX_VERSION,
            rule: "codex-exec-approval",
            answers: [
                (
                    PermissionDecision::Allow,
                    "codex-0.163.0-allow-pre-stage-1",
                    "allow-once",
                    b'y',
                ),
                (
                    PermissionDecision::Deny,
                    "codex-0.163.0-deny-pre-stage-1",
                    "deny-once",
                    b'3',
                ),
            ],
            choices: CODEX_CHOICES,
            glyph: '›',
            focus_row: 30,
            second_choice: ("  2.", "› 2."),
            cursor_follows_focus: false,
        },
    ];

    #[test]
    fn bundled_answer_plans_match_their_measured_pre_stage_forms_and_bind_focus() {
        for forms in PRE_STAGE_FORMS {
            let detector = bundled_detector(forms.backend, forms.version);
            let answer = |screen: &ScreenSnapshot, plan: &AnswerPlan| {
                detector.match_answer(screen, plan).unwrap()
            };
            let focus = u16::try_from(forms.focus_row).unwrap();
            for (decision, id, choice, byte) in forms.answers {
                let plan = ProfileCatalog::bundled()
                    .unwrap()
                    .answer_plan(forms.backend, forms.version, forms.rule, decision)
                    .expect("measured bundled answer plan");
                assert_eq!(plan.stages().len(), 1, "{id}");
                assert_eq!(plan.stages()[0].bytes(), &[byte], "{id}");
                let fixture = bundled_screen(id);
                let screen = replay_native_fixture(fixture, |_| {}, true, None);
                if forms.cursor_follows_focus {
                    assert_eq!((screen.cursor_row, screen.cursor_col), (focus, 1), "{id}");
                }
                let matched = answer(&screen, &plan).expect("pre-stage form matches its plan");
                assert_eq!(
                    matched.choices(),
                    forms.choices.map(str::to_owned).as_slice(),
                    "{id}"
                );
                assert_eq!(matched.plan().choice_id(), choice, "{id}");
                assert!(!matched.focus().is_empty(), "{id}");
                assert!(
                    Some(matched.semantic())
                        == detector.observe(&screen).unwrap().fingerprint.as_ref(),
                    "{id}"
                );
                let (choice_two, focused_choice_two) = forms.second_choice;
                let moved = replay_native_fixture(
                    fixture,
                    |lines| {
                        lines[forms.focus_row] =
                            lines[forms.focus_row].replacen(forms.glyph, " ", 1);
                        lines[forms.focus_row + 1] =
                            lines[forms.focus_row + 1].replacen(choice_two, focused_choice_two, 1);
                    },
                    true,
                    forms.cursor_follows_focus.then_some((focus + 1, 1)),
                );
                let refocused =
                    answer(&moved, &plan).expect("a moved focus still identifies the form");
                assert!(refocused.semantic() == matched.semantic(), "{id}");
                assert_ne!(
                    refocused.focus(),
                    matched.focus(),
                    "{id}: token binding sees the focus move"
                );
                let unfocused = replay_native_fixture(
                    fixture,
                    |lines| {
                        lines[forms.focus_row] =
                            lines[forms.focus_row].replacen(forms.glyph, " ", 1)
                    },
                    true,
                    None,
                );
                assert!(
                    answer(&unfocused, &plan).is_none(),
                    "{id} without its focus glyph"
                );
                let primary = replay_native_fixture(fixture, |_| {}, false, None);
                assert!(answer(&primary, &plan).is_none(), "{id} primary screen");
            }
        }
    }

    struct IdleComposer {
        backend: CodingAgent,
        version: &'static str,
        idle: &'static str,
        idle_rule: &'static str,
        composer_top: u16,
        prompt_row: usize,
        draft: &'static str,
        form: &'static str,
        form_rule: &'static str,
    }

    const IDLE_COMPOSERS: [IdleComposer; 2] = [
        IdleComposer {
            backend: CodingAgent::Claude,
            version: CLAUDE_VERSION,
            idle: "claude-2.1.302-idle-composer-1",
            idle_rule: "claude-idle-composer",
            composer_top: 12,
            prompt_row: 31,
            draft: "❯\u{a0}hello",
            form: "claude-2.1.302-default-permission-1",
            form_rule: "claude-bash-permission",
        },
        IdleComposer {
            backend: CodingAgent::Codex,
            version: CODEX_VERSION,
            idle: "codex-0.163.0-idle-composer-1",
            idle_rule: "codex-idle-composer",
            composer_top: 32,
            prompt_row: 32,
            draft: "› hello",
            form: "codex-0.163.0-on-request-permission-1",
            form_rule: "codex-exec-approval",
        },
    ];

    #[test]
    fn bundled_idle_composers_are_verified_non_interactions_distinct_from_the_form() {
        for composer in IDLE_COMPOSERS {
            let detector = bundled_detector(composer.backend, composer.version);
            let observe = |id: &str| {
                detector
                    .observe(&replay_native_fixture(
                        bundled_screen(id),
                        |_| {},
                        true,
                        None,
                    ))
                    .unwrap()
            };
            let idle = observe(composer.idle);
            assert_eq!(
                idle.kind,
                ObservationKind::ComposerEmpty,
                "{}",
                composer.idle
            );
            assert_eq!(
                idle.match_evidence,
                MatchEvidence::Complete,
                "{}",
                composer.idle
            );
            assert!(idle.verified_noninteraction, "{}", composer.idle);
            assert_eq!(idle.rule_id, composer.idle_rule);
            assert_eq!(
                idle.raw_evidence["operation"],
                Value::Null,
                "{}",
                composer.idle
            );
            assert_eq!(
                idle.raw_evidence["operation_rows"],
                Value::Null,
                "{}",
                composer.idle
            );
            let region = idle
                .matched_region
                .expect("the idle rule locates its composer");
            assert!(
                region.top >= composer.composer_top,
                "{}: {region:?}",
                composer.idle
            );
            let form = observe(composer.form);
            assert_eq!(form.rule_id, composer.form_rule);
            assert_eq!(
                form.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest),
                "{}",
                composer.form
            );
            assert_eq!(
                form.match_evidence,
                MatchEvidence::Complete,
                "{}",
                composer.form
            );
            assert!(!form.verified_noninteraction, "{}", composer.form);
        }
    }

    #[test]
    fn bundled_idle_rules_reject_drafts_primary_copies_and_a_cursor_off_the_prefix_end() {
        for composer in IDLE_COMPOSERS {
            let detector = bundled_detector(composer.backend, composer.version);
            let row = u16::try_from(composer.prompt_row).unwrap();
            let observe =
                |change: &dyn Fn(&mut Vec<String>), alternate: bool, cursor: Option<(u16, u16)>| {
                    detector
                        .observe(&replay_native_fixture(
                            bundled_screen(composer.idle),
                            change,
                            alternate,
                            cursor,
                        ))
                        .unwrap()
                };
            let draft = observe(
                &|lines: &mut Vec<String>| lines[composer.prompt_row] = composer.draft.into(),
                true,
                Some((row, 7)),
            );
            assert_ne!(
                draft.kind,
                ObservationKind::ComposerEmpty,
                "{} typed draft",
                composer.idle
            );
            assert!(!draft.verified_noninteraction, "{}", composer.idle);
            let primary = observe(&|_: &mut Vec<String>| {}, false, None);
            assert!(
                !primary.verified_noninteraction,
                "{} primary-screen copy",
                composer.idle
            );
            let away = observe(&|_: &mut Vec<String>| {}, true, Some((row, 0)));
            assert!(
                !away.verified_noninteraction,
                "{} cursor before the prefix end",
                composer.idle
            );
            assert_ne!(
                away.kind,
                ObservationKind::ComposerEmpty,
                "{}",
                composer.idle
            );
        }
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
            Detector::new(&gated_catalog(), CodingAgent::Codex, Some(VERSION))
        }

        fn gated_plan(decision: PermissionDecision) -> AnswerPlan {
            gated_catalog()
                .answer_plan(CodingAgent::Codex, VERSION, GATED_RULE, decision)
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

        const ROW_SGR: &str = "\x1b[48;2;30;30;30m";

        /// The gated form on a `cols`-wide screen whose choice row is `row`, written from
        /// column 0 after the screen is cleared to the default background.
        fn sized_form(cols: u16, row: &str) -> ScreenSnapshot {
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols }).unwrap();
            state
                .feed(
                    format!(
                        "\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: write disposable.txt\r\n{row}\x1b[0m\r\nSelection: "
                    )
                    .as_bytes(),
                )
                .unwrap();
            state.snapshot()
        }

        fn focused_on(screen: &ScreenSnapshot) -> Option<String> {
            let allow = detector()
                .match_answer(screen, &gated_plan(PermissionDecision::Allow))
                .unwrap()
                .expect("the gated form matches regardless of its focus");
            allow.focused_choice_id().map(str::to_owned)
        }

        #[test]
        fn focus_row_background_is_the_majority_background_not_the_left_edge() {
            let edge = sized_form(
                80,
                &format!(
                    "{ROW_SGR}\x1b[K{FOCUS_SGR}  Allow once {ROW_SGR}  Always allow   Deny once"
                ),
            );
            assert_eq!(
                focused_on(&edge).as_deref(),
                Some("allow-once"),
                "a highlight over column 0 leaves the row background to the majority"
            );
            let last_column = sized_form(
                39,
                &format!("{ROW_SGR}\x1b[K  Allow once   Always allow   {FOCUS_SGR}Deny once"),
            );
            assert_eq!(
                focused_on(&last_column).as_deref(),
                Some("deny-once"),
                "a focused label may reach the last column"
            );
        }

        #[test]
        fn focus_resolves_no_choice_without_a_background_covering_more_than_half_the_row() {
            let half = "\x1b[48;5;236m";
            let split_row = |painted: usize| {
                sized_form(
                    80,
                    &format!(
                        "{half}\x1b[K  {FOCUS_SGR}Allow once{half}   Always allow   Deny once\x1b[{}G\x1b[0m\x1b[K",
                        painted + 1
                    ),
                )
            };
            assert_eq!(
                focused_on(&split_row(40)),
                None,
                "40 painted cells of 80 are not a majority"
            );
            assert_eq!(
                focused_on(&split_row(51)).as_deref(),
                Some("allow-once"),
                "41 cells of the painted background are a majority"
            );
        }

        #[test]
        fn focus_requires_every_cell_of_the_label_to_differ_from_the_row_background() {
            let partial = sized_form(
                80,
                &format!(
                    "{ROW_SGR}\x1b[K  {FOCUS_SGR}Allow{ROW_SGR} once   Always allow   Deny once"
                ),
            );
            assert_eq!(focused_on(&partial), None);
            let whole = sized_form(
                80,
                &format!(
                    "{ROW_SGR}\x1b[K  {FOCUS_SGR}Allow once{ROW_SGR}   Always allow   Deny once"
                ),
            );
            assert_eq!(focused_on(&whole).as_deref(), Some("allow-once"));
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
        assert_eq!(value.raw_evidence["native_version"], VERSION);
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
        definition["profiles"][0]["min_version"] = json!("0.0.0-synthetic");
        assert!(
            value.fingerprint
                == observation(
                    &definition,
                    &terminal(&lines("write disposable.txt"), 80, false).snapshot()
                )
                .fingerprint,
            "the selected profile's boundary is not part of the request identity"
        );
    }

    #[test]
    fn runtime_detector_semantic_fingerprint_is_backend_class_operation_and_choices() {
        let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
        let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
        let at_boundary = Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
            .observe(&screen)
            .unwrap();
        let later = Detector::new(&catalog, CodingAgent::Codex, Some("0.0.2-synthetic"))
            .observe(&screen)
            .unwrap();
        assert_permission(&later);
        assert_eq!(later.raw_evidence["native_version"], "0.0.2-synthetic");
        assert!(
            at_boundary.fingerprint == later.fingerprint,
            "a later native version on the same profile keeps the request identity"
        );
        let changed = Detector::new(&catalog, CodingAgent::Codex, Some("0.0.2-synthetic"))
            .observe(&terminal(&lines("write another-disposable.txt"), 80, false).snapshot())
            .unwrap();
        assert!(at_boundary.fingerprint != changed.fingerprint);
    }

    #[test]
    fn runtime_detector_fingerprints_carry_no_profile_element_across_profile_boundaries() {
        let catalog = ProfileCatalog::from_definitions(&thresholds(&[
            ("first", "1.0.0"),
            ("second", "2.0.0"),
        ]))
        .unwrap();
        let mut rows = vec![
            "Live footer".to_owned(),
            "Refused: write outside.txt".to_owned(),
            "Refusal end".to_owned(),
        ];
        rows.extend(lines("write disposable.txt"));
        let screen = terminal(&rows, 80, false).snapshot();
        let [first, second] = ["1.4.2", "3.1.4"].map(|run| {
            Detector::new(&catalog, CodingAgent::Codex, Some(run))
                .observe(&screen)
                .unwrap()
        });
        for observed in [&first, &second] {
            assert_eq!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert!(observed.fingerprint.is_some());
            assert_eq!(observed.annotations.len(), 1);
        }
        assert!(
            first.fingerprint == second.fingerprint,
            "the permission request keeps its identity across profile boundaries"
        );
        assert!(
            first.annotations[0].fingerprint == second.annotations[0].fingerprint,
            "the annotation keeps its identity across profile boundaries"
        );
    }

    #[test]
    fn runtime_detector_raw_evidence_names_the_backend_and_native_version_only() {
        let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
        let screen = terminal(&lines("write disposable.txt"), 80, false).snapshot();
        for version in [Some(VERSION), None] {
            let observed = Detector::new(&catalog, CodingAgent::Codex, version)
                .observe(&screen)
                .unwrap();
            let keys = observed
                .raw_evidence
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                keys,
                std::collections::BTreeSet::from([
                    "backend",
                    "choices",
                    "excerpt",
                    "native_request_id",
                    "native_version",
                    "operation",
                    "operation_rows",
                    "rule_id",
                    "screen_seq",
                    "sensitive",
                    "truncated",
                ]),
                "{version:?}"
            );
            assert_eq!(observed.raw_evidence["backend"], "codex");
            assert_eq!(observed.raw_evidence["native_version"], json!(version));
        }
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
            .observe(&terminal(&lines("write disposable.txt"), 80, false).snapshot())
            .unwrap();
        assert!(matches!(value.kind, ObservationKind::Unknown));
        assert!(matches!(value.match_evidence, MatchEvidence::Absent));
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
                .observe(&terminal(&rows, 80, false).snapshot())
                .unwrap();
            assert!(observed.annotations.is_empty());
        }
    }

    mod operation_rows {
        use super::*;

        const MARGIN: &str = "    ";

        /// The synthetic form with a region-scoped operation that captures every row between
        /// the `Operation:` prefix and the choice rows.
        fn rowed(margin_rows: Option<&str>, trailer: Option<&str>) -> Value {
            let mut value = definitions();
            let rule = &mut value["profiles"][0]["rules"][0];
            rule["matchers"][0] = json!({
                "id":"operation","scope":"region",
                "pattern":"(?m)^Operation: (?P<operation>[^\\n]+(?:\\n[^>\\n][^\\n]*)*)\\n> Allow once$"
            });
            let operation = &mut rule["extract"]["operation"];
            operation["row_margin"] = json!(MARGIN);
            if let Some(margin_rows) = margin_rows {
                operation["margin_rows"] = json!(margin_rows);
            }
            if let Some(trailer) = trailer {
                operation["row_trailer"] = json!(trailer);
            }
            value
        }

        fn form(rows: &[&str]) -> ScreenSnapshot {
            let mut lines = vec!["Synthetic permission request".to_owned()];
            lines.push(format!("Operation: {}", rows[0]));
            lines.extend(rows[1..].iter().map(|row| (*row).to_owned()));
            lines.extend([
                "> Allow once".to_owned(),
                "  Deny once".to_owned(),
                "Request form: synthetic-only".to_owned(),
                "Selection: ".to_owned(),
            ]);
            terminal(&lines, 80, false).snapshot()
        }

        fn operation(definition: &Value, rows: &[&str]) -> Option<(String, u64)> {
            let observed = observation(definition, &form(rows));
            (observed.kind == ObservationKind::Interaction(InteractionClass::PermissionRequest))
                .then(|| {
                    (
                        observed.raw_evidence["operation"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        observed.raw_evidence["operation_rows"].as_u64().unwrap(),
                    )
                })
        }

        #[test]
        fn continuation_margin_rows_exempt_the_first_row_and_strip_every_later_margin() {
            let definition = rowed(Some("continuation"), None);
            assert_eq!(
                operation(&definition, &["cat /long/path/", "    to/the/file.txt"]),
                Some(("cat /long/path/\nto/the/file.txt".to_owned(), 2))
            );
            assert_eq!(
                operation(&definition, &["printf one", "    two   ", "    three"]),
                Some(("printf one\ntwo\nthree".to_owned(), 3))
            );
            assert_eq!(
                operation(&definition, &["write disposable.txt"]),
                Some(("write disposable.txt".to_owned(), 1))
            );
        }

        #[test]
        fn a_selected_row_without_its_margin_fails_the_match() {
            let continuation = rowed(Some("continuation"), None);
            assert_eq!(
                operation(&continuation, &["cat /long/path/", "  to/the/file.txt"]),
                None
            );
            for all in [rowed(Some("all"), None), rowed(None, None)] {
                assert_eq!(
                    operation(&all, &["cat /long/path/", "    to/the/file.txt"]),
                    None,
                    "the default margin rows include the first row"
                );
            }
        }

        #[test]
        fn the_default_margin_rows_keep_the_shipped_all_rows_extraction() {
            let mut definition = rowed(None, None);
            definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] = json!(
                "(?m)^Operation:(?P<operation> [^\\n]+(?:\\n[^>\\n][^\\n]*)*)\\n> Allow once$"
            );
            definition["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] =
                json!(" ");
            let explicit = {
                let mut value = definition.clone();
                value["profiles"][0]["rules"][0]["extract"]["operation"]["margin_rows"] =
                    json!("all");
                value
            };
            for value in [&definition, &explicit] {
                assert_eq!(
                    operation(value, &["cat /long/path/", " to/the/file.txt"]),
                    Some(("cat /long/path/\nto/the/file.txt".to_owned(), 2))
                );
            }
        }

        #[test]
        fn a_trailer_is_removed_with_its_preceding_whitespace_before_the_margin() {
            let definition = rowed(Some("continuation"), Some("█"));
            assert_eq!(
                operation(
                    &definition,
                    &["cat /long/path/      █", "    to/the/file.txt     █  "]
                ),
                Some(("cat /long/path/\nto/the/file.txt".to_owned(), 2))
            );
            assert_eq!(
                operation(&definition, &["cat /long/path/█", "    to/the/file.txt"]),
                Some(("cat /long/path/\nto/the/file.txt".to_owned(), 2)),
                "a row without the trailer keeps its text"
            );
            assert_eq!(
                operation(&definition, &["cat a█b", "    c █ d"]),
                Some(("cat a█b\nc █ d".to_owned(), 2)),
                "the trailer is removed only at the row end"
            );
        }

        #[test]
        fn a_trailer_applies_to_a_single_row_operation_without_a_margin() {
            let mut definition = definitions();
            definition["profiles"][0]["rules"][0]["extract"]["operation"]["row_trailer"] =
                json!("█");
            let observed = observation(
                &definition,
                &terminal(&lines("write disposable.txt   █"), 80, false).snapshot(),
            );
            assert_permission(&observed);
            assert_eq!(observed.raw_evidence["operation"], "write disposable.txt");
            assert_eq!(observed.raw_evidence["operation_rows"], 1);
        }

        #[test]
        fn a_blank_margined_row_inside_a_multi_line_command_still_matches() {
            let mut definition = rowed(None, None);
            definition["profiles"][0]["rules"][0]["matchers"][0]["pattern"] =
                json!("(?m)^Operation:(?P<operation> [^\\n]+(?:\\n [^\\n]*)*)\\n> Allow once$");
            definition["profiles"][0]["rules"][0]["extract"]["operation"]["row_margin"] =
                json!(" ");
            assert_eq!(
                operation(&definition, &["printf one", " ", " printf two"]),
                Some(("printf one\n\nprintf two".to_owned(), 3)),
                "margin removal precedes trimming, so a margin-only row is an empty row"
            );
        }
    }

    mod opencode_layouts {
        use super::*;

        const VERSION: &str = "2.1.4";
        const PERMISSION_RULE: &str = "opencode-shell-permission";
        const REASON_RULE: &str = "opencode-reject-reason";
        const CHOICES: &str = "   Allow once   Always allow   Reject";
        const CONTROLS: &str = "⇆ select  enter confirm  esc reject";
        const PROBE_DIRECTORY: &str = "/tmp/cafleet-194/opencode-narrow-cells-057/";

        enum Layout {
            Wide,
            Split,
            Broken,
            ControlsOnly,
            InterposedRow,
        }

        fn shell_rule() -> Value {
            let definitions = bundled_definitions();
            definitions["profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|profile| profile["backend"] == "opencode")
                .expect("bundled OpenCode profile")["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rule| rule["id"] == PERMISSION_RULE)
                .expect("bundled OpenCode shell permission rule")
                .clone()
        }

        fn continuation_margin() -> String {
            shell_rule()["extract"]["operation"]["row_margin"]
                .as_str()
                .expect("the OpenCode operation declares its continuation margin")
                .to_owned()
        }

        /// A row of the command box: `text`, then the scrollbar column four cells before the
        /// right edge, as the 120-column capture shows it.
        fn boxed(cols: usize, text: &str) -> String {
            let width = cols - 4;
            assert!(text.chars().count() < width, "{text:?} fits the box");
            format!("{text:<width$}█   ")
        }

        fn form_rows(cols: usize, command: &[&str], layout: Layout) -> Vec<String> {
            let margin = continuation_margin();
            let mut rows = vec![
                "  △ Permission required".to_owned(),
                String::new(),
                boxed(cols, "   # Shell command"),
                boxed(cols, ""),
                boxed(cols, &format!("  $ {}", command[0])),
            ];
            rows.extend(
                command[1..]
                    .iter()
                    .map(|row| boxed(cols, &format!("{margin}{row}"))),
            );
            rows.extend([
                boxed(cols, ""),
                boxed(cols, ""),
                String::new(),
                String::new(),
            ]);
            let controls = format!("  {CONTROLS}");
            match layout {
                Layout::Wide => {
                    let padding = cols - CHOICES.chars().count() - CONTROLS.chars().count() - 3;
                    rows.push(format!("{CHOICES}{}{CONTROLS}   ", " ".repeat(padding)));
                }
                Layout::Split => rows.extend([CHOICES.to_owned(), String::new(), controls]),
                Layout::Broken => rows.extend([
                    "   Allow once   Always allow".to_owned(),
                    "   Reject".to_owned(),
                    controls,
                ]),
                Layout::ControlsOnly => rows.extend([String::new(), controls]),
                Layout::InterposedRow => {
                    rows.extend([CHOICES.to_owned(), "   Press a key".to_owned(), controls])
                }
            }
            rows
        }

        fn paint(cols: usize, rows: &[String]) -> ScreenSnapshot {
            let mut state = TerminalState::new(TerminalSize {
                rows: 36,
                cols: u16::try_from(cols).unwrap(),
            })
            .unwrap();
            state.feed(b"\x1b[2J").unwrap();
            for (index, row) in rows.iter().enumerate() {
                if !row.is_empty() {
                    state
                        .feed(format!("\x1b[{};1H{row}", index + 11).as_bytes())
                        .unwrap();
                }
            }
            state.snapshot()
        }

        fn form(cols: usize, command: &[&str], layout: Layout) -> ScreenSnapshot {
            paint(cols, &form_rows(cols, command, layout))
        }

        fn observe(screen: &ScreenSnapshot) -> ScreenObservation {
            bundled_detector(CodingAgent::Opencode, VERSION)
                .observe(screen)
                .unwrap()
        }

        fn assert_request(observed: &ScreenObservation, operation: &str) {
            assert_eq!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_eq!(observed.rule_id, PERMISSION_RULE);
            assert_eq!(observed.match_evidence, MatchEvidence::Complete);
            assert_eq!(observed.raw_evidence["operation"], operation);
            assert_eq!(
                observed.raw_evidence["operation_rows"],
                operation.split('\n').count()
            );
            assert_eq!(
                observed.raw_evidence["choices"],
                json!(["Allow once", "Reject"])
            );
            assert!(observed.fingerprint.is_some());
        }

        fn assert_no_request(observed: &ScreenObservation, case: &str) {
            assert_ne!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest),
                "{case}"
            );
            assert_ne!(observed.rule_id, PERMISSION_RULE, "{case}");
            assert!(observed.fingerprint.is_none(), "{case}");
        }

        fn allow_match(screen: &ScreenSnapshot) -> AnswerMatch {
            let plan = ProfileCatalog::bundled()
                .unwrap()
                .answer_plan(
                    CodingAgent::Opencode,
                    VERSION,
                    PERMISSION_RULE,
                    PermissionDecision::Allow,
                )
                .unwrap();
            bundled_detector(CodingAgent::Opencode, VERSION)
                .match_answer(screen, &plan)
                .unwrap()
                .expect("the form matches the allow plan")
        }

        fn wrapped() -> [&'static str; 2] {
            [PROBE_DIRECTORY, "permission-probe allow"]
        }

        #[test]
        fn opencode_split_layout_reports_every_displayed_command_row() {
            let observed = observe(&form(57, &wrapped(), Layout::Split));
            assert_request(
                &observed,
                &format!("{PROBE_DIRECTORY}\npermission-probe allow"),
            );
            let single = observe(&form(57, &["/tmp/x/permission-probe allow"], Layout::Split));
            assert_request(&single, "/tmp/x/permission-probe allow");
        }

        #[test]
        fn opencode_wide_layout_keeps_its_single_row_request() {
            let command = "/tmp/cafleet-194/opencode-cells-001/permission-probe allow";
            assert_request(&observe(&form(120, &[command], Layout::Wide)), command);
        }

        #[test]
        fn opencode_forms_outside_the_supported_layouts_yield_no_request() {
            let mut headless = form_rows(57, &wrapped(), Layout::Split);
            headless[0] = String::new();
            let mut stray = form_rows(57, &wrapped(), Layout::Split);
            stray.insert(6, boxed(57, "stray text"));
            for (case, screen) in [
                ("broken layout", form(57, &wrapped(), Layout::Broken)),
                (
                    "controls without a choice row",
                    form(57, &wrapped(), Layout::ControlsOnly),
                ),
                (
                    "a row between the choices and the controls",
                    form(57, &wrapped(), Layout::InterposedRow),
                ),
                (
                    "a continuation row indented past the margin",
                    form(
                        57,
                        &[PROBE_DIRECTORY, " permission-probe allow"],
                        Layout::Split,
                    ),
                ),
                (
                    "a row without the margin before the box end",
                    paint(57, &stray),
                ),
                ("form text outside a live region", paint(57, &headless)),
            ] {
                assert_no_request(&observe(&screen), case);
            }
        }

        #[test]
        fn opencode_layouts_differ_in_focus_capture_and_rewraps_differ_in_identity() {
            let command = format!("{PROBE_DIRECTORY}permission-probe allow");
            let wide = form(120, &[command.as_str()], Layout::Wide);
            let split = form(57, &wrapped(), Layout::Split);
            assert_ne!(
                allow_match(&wide).focus(),
                allow_match(&split).focus(),
                "the focus capture is part of the request identity"
            );
            assert!(observe(&wide).fingerprint != observe(&split).fingerprint);
            let rewrapped = form(
                57,
                &[
                    "/tmp/cafleet-194/opencode-narrow-",
                    "cells-057/permission-probe allow",
                ],
                Layout::Split,
            );
            assert!(
                observe(&split).fingerprint != observe(&rewrapped).fingerprint,
                "a moved row break is a different request"
            );
            assert!(
                observe(&split).fingerprint
                    == observe(&form(57, &wrapped(), Layout::Split)).fingerprint,
                "an unchanged form keeps its identity"
            );
            assert!(
                allow_match(&split).focus()
                    == allow_match(&form(57, &wrapped(), Layout::Split)).focus()
            );
        }

        fn dialog(placeholder: Option<&str>) -> ScreenSnapshot {
            let mut rows = vec![
                "  △ Reject permission".to_owned(),
                String::new(),
                "  Tell OpenCode what to do differently".to_owned(),
            ];
            rows.extend(std::iter::repeat_n(String::new(), 7));
            if let Some(text) = placeholder {
                rows.push(format!("  {text}"));
            }
            rows.push("  enter confirm  esc cancel".to_owned());
            paint(57, &rows)
        }

        #[test]
        fn opencode_split_reject_dialog_requires_the_empty_placeholder() {
            let empty = observe(&dialog(Some("Tell OpenCode what to do differently")));
            assert_eq!(empty.kind, ObservationKind::PermissionContinuation);
            assert_eq!(empty.rule_id, REASON_RULE);
            assert_eq!(empty.match_evidence, MatchEvidence::Complete);
            for (case, screen) in [
                ("a prompt-only dialog", dialog(None)),
                (
                    "a typed reason",
                    dialog(Some("run it in the sandbox instead")),
                ),
                (
                    "a typed copy of the placeholder with more text",
                    dialog(Some("Tell OpenCode what to do differently!")),
                ),
            ] {
                let observed = observe(&screen);
                assert_ne!(
                    observed.kind,
                    ObservationKind::PermissionContinuation,
                    "{case}"
                );
                assert_ne!(observed.rule_id, REASON_RULE, "{case}");
            }
        }

        const COMPACT_RULE: &str = "opencode-shell-permission-compact";
        const COMPACT_CHOICES: &str = " Allow once   Always allow   Reject";
        const COMPACT_CONTROLS: &str = "pgup/pgdn scroll";

        enum Compact {
            SharedControls,
            OwnControls,
            Broken,
            NoControls,
        }

        /// A compact command-box row: `text` at column 0 and the scrollbar column at the right
        /// edge, as the 36-, 52- and 55-column captures show it.
        fn compact_boxed(cols: usize, text: &str) -> String {
            let width = cols - 1;
            assert!(text.chars().count() <= width, "{text:?} fits the box");
            format!("{text:<width$}█")
        }

        fn compact_rows(cols: usize, command: &[&str], layout: Compact) -> Vec<String> {
            let mut rows = vec![
                "△ Permission required".to_owned(),
                compact_boxed(cols, "# Shell command"),
                compact_boxed(cols, &format!("$ {}", command[0])),
            ];
            rows.extend(command[1..].iter().map(|row| compact_boxed(cols, row)));
            rows.extend((0..3).map(|_| compact_boxed(cols, "")));
            match layout {
                Compact::SharedControls => {
                    let padding = cols - COMPACT_CHOICES.chars().count() - COMPACT_CONTROLS.len();
                    rows.push(format!(
                        "{COMPACT_CHOICES}{}{COMPACT_CONTROLS}",
                        " ".repeat(padding)
                    ));
                }
                Compact::OwnControls => {
                    rows.extend([COMPACT_CHOICES.to_owned(), COMPACT_CONTROLS.to_owned()])
                }
                Compact::Broken => rows.extend([
                    " Allow once   Always allow".to_owned(),
                    " Reject".to_owned(),
                    COMPACT_CONTROLS.to_owned(),
                ]),
                Compact::NoControls => rows.push(COMPACT_CHOICES.to_owned()),
            }
            rows
        }

        fn compact(cols: usize, command: &[&str], layout: Compact) -> ScreenSnapshot {
            paint(cols, &compact_rows(cols, command, layout))
        }

        fn assert_compact_request(observed: &ScreenObservation, operation: &str) {
            assert_eq!(
                observed.kind,
                ObservationKind::Interaction(InteractionClass::PermissionRequest)
            );
            assert_eq!(observed.rule_id, COMPACT_RULE);
            assert_eq!(observed.match_evidence, MatchEvidence::Complete);
            assert_eq!(observed.raw_evidence["operation"], operation);
            assert_eq!(
                observed.raw_evidence["operation_rows"],
                operation.split('\n').count()
            );
            assert_eq!(
                observed.raw_evidence["choices"],
                json!(["Allow once", "Reject"])
            );
            assert!(observed.fingerprint.is_some());
        }

        #[test]
        fn opencode_compact_layout_reports_every_displayed_command_row() {
            for (cols, command, layout) in [
                (
                    55,
                    [
                        "/tmp/cafleet-194/opencode-survey-055/permission-",
                        "probe allow",
                    ],
                    Compact::SharedControls,
                ),
                (
                    52,
                    [
                        "/tmp/cafleet-194/opencode-survey-052/permission-",
                        "probe allow",
                    ],
                    Compact::OwnControls,
                ),
                (
                    36,
                    [
                        "/tmp/cafleet-194/opencode-survey-",
                        "036/permission-probe allow",
                    ],
                    Compact::OwnControls,
                ),
            ] {
                assert_compact_request(
                    &observe(&compact(cols, &command, layout)),
                    &command.join("\n"),
                );
            }
            assert_compact_request(
                &observe(&compact(
                    55,
                    &["/tmp/x/permission-probe allow"],
                    Compact::SharedControls,
                )),
                "/tmp/x/permission-probe allow",
            );
        }

        #[test]
        fn opencode_compact_forms_outside_the_layout_yield_no_request() {
            let command = [
                "/tmp/cafleet-194/opencode-survey-",
                "036/permission-probe allow",
            ];
            let mut stray = compact_rows(36, &command, Compact::OwnControls);
            stray.insert(5, compact_boxed(36, "stray text"));
            let mut headless = compact_rows(36, &command, Compact::OwnControls);
            headless[0] = String::new();
            let mut margined = compact_rows(36, &command, Compact::OwnControls);
            margined[0] = "  △ Permission required".to_owned();
            for (case, screen) in [
                (
                    "broken layout",
                    compact(35, &["/tmp/x/permission-probe allow"], Compact::Broken),
                ),
                (
                    "no control text",
                    compact(36, &command, Compact::NoControls),
                ),
                (
                    "a continuation row beginning with a space",
                    compact(
                        36,
                        &["/tmp/cafleet-194/opencode-survey-", " 036/permission-probe"],
                        Compact::OwnControls,
                    ),
                ),
                ("a row after the box end", paint(36, &stray)),
                ("form text outside a live region", paint(36, &headless)),
                ("a wide/split heading margin", paint(36, &margined)),
            ] {
                assert_no_request(&observe(&screen), case);
            }
        }

        #[test]
        fn opencode_exactly_one_request_rule_matches_each_supported_layout() {
            for (case, screen, rule) in [
                (
                    "wide",
                    form(120, &["/tmp/x/permission-probe allow"], Layout::Wide),
                    PERMISSION_RULE,
                ),
                (
                    "split",
                    form(57, &wrapped(), Layout::Split),
                    PERMISSION_RULE,
                ),
                (
                    "compact",
                    compact(
                        55,
                        &["/tmp/x/permission-probe allow"],
                        Compact::SharedControls,
                    ),
                    COMPACT_RULE,
                ),
            ] {
                let observed = observe(&screen);
                assert_eq!(observed.rule_id, rule, "{case}");
                assert_eq!(observed.match_evidence, MatchEvidence::Complete, "{case}");
            }
        }
    }
}
