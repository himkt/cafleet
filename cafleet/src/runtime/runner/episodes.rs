use super::*;
use crate::broker::runtime::{
    AutomaticReview, EffectivePolicyObservation, InteractionClass, InteractionCloseReason,
    InteractionEvidenceUpdate, ObservedTransition, RuntimeEventKind, evidence::EvidencePolicy,
};
use crate::coding_agent::{AnnotationBlock, AnnotationFingerprint};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

fn same_evidence_apart_from_sequence(stored: &Value, current: &Value) -> bool {
    let stored = stored.as_object().expect("policy evidence is an object");
    let current = current.as_object().expect("policy evidence is an object");
    // evidence_digest covers screen_seq, so it changes whenever screen_seq does.
    stored.len() == current.len()
        && stored.iter().all(|(key, value)| {
            matches!(key.as_str(), "screen_seq" | "evidence_digest")
                || current.get(key) == Some(value)
        })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Visibility {
    Visible,
    Obscured,
    NotVisible,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateReason {
    Candidate,
    Interaction,
    Working,
    Draft,
    Unknown,
    ObserverUnavailable,
    Gap,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum InputEligibility {
    ComposerEmpty,
    Hold(GateReason),
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TransitionDraft {
    pub run_id: i64,
    pub episode_seq: Option<i64>,
    pub kind: RuntimeEventKind,
    pub classification: Option<InteractionClass>,
    pub observed_at: String,
    pub evidence_json: Value,
    pub close_reason: Option<InteractionCloseReason>,
    pub cause_json: Option<Value>,
}
#[derive(Debug)]
pub(crate) struct EpisodeUpdate {
    pub visibility: Visibility,
    pub eligibility: InputEligibility,
    pub transitions: Vec<TransitionDraft>,
    pub latest_evidence: Option<InteractionEvidenceUpdate>,
    pub next_deadline: Option<Instant>,
    pub effective_policy: Option<EffectivePolicyObservation>,
}
fn earliest(deadline: &mut Option<Instant>, until: Instant) {
    *deadline = Some(deadline.map_or(until, |old| old.min(until)));
}
#[derive(Default)]
struct AnnotationState {
    candidate_since: Option<Instant>,
    emitted: bool,
    absent_since: Option<Instant>,
}
#[derive(Default)]
struct AnnotationTracker {
    states: BTreeMap<String, BTreeMap<AnnotationFingerprint, AnnotationState>>,
    unavailable_emitted: BTreeSet<String>,
    last_policy: Option<(AutomaticReview, Option<String>)>,
}
impl AnnotationTracker {
    fn age_absent(&mut self, value: &ScreenObservation, now: Instant, update: &mut EpisodeUpdate) {
        for (rule, states) in &mut self.states {
            let resettable =
                value.verified_noninteraction && value.guarded_annotation_rules.contains(rule);
            states.retain(|fingerprint, state| {
                let shown = value
                    .annotations
                    .iter()
                    .any(|block| &block.rule_id == rule && &block.fingerprint == fingerprint);
                if shown {
                    state.absent_since = None;
                    return true;
                }
                state.candidate_since = None;
                if !state.emitted {
                    return false;
                }
                if !resettable {
                    state.absent_since = None;
                    return true;
                }
                let since = *state.absent_since.get_or_insert(now);
                if elapsed(now, since, Duration::from_millis(250)) {
                    return false;
                }
                earliest(
                    &mut update.next_deadline,
                    since + Duration::from_millis(250),
                );
                true
            });
        }
    }
    fn stable(
        &mut self,
        block: &AnnotationBlock,
        now: Instant,
        update: &mut EpisodeUpdate,
    ) -> bool {
        let state = self
            .states
            .entry(block.rule_id.clone())
            .or_default()
            .entry(block.fingerprint.clone())
            .or_default();
        if state.emitted {
            return true;
        }
        let since = *state.candidate_since.get_or_insert(now);
        if elapsed(now, since, Duration::from_millis(100)) {
            return true;
        }
        earliest(
            &mut update.next_deadline,
            since + Duration::from_millis(100),
        );
        false
    }
    fn mark_emitted(&mut self, block: &AnnotationBlock) -> bool {
        let state = self
            .states
            .get_mut(&block.rule_id)
            .and_then(|states| states.get_mut(&block.fingerprint))
            .expect("a stable annotation has its state");
        !std::mem::replace(&mut state.emitted, true)
    }
    fn forget_candidates(&mut self) {
        for states in self.states.values_mut() {
            states.retain(|_, state| {
                state.candidate_since = None;
                state.absent_since = None;
                state.emitted
            });
        }
    }
}
struct Candidate {
    fingerprint: SemanticFingerprint,
    since: Instant,
}
struct Active {
    sequence: i64,
    class: InteractionClass,
    rule_id: String,
    fingerprint: SemanticFingerprint,
    evidence: Value,
    native: Option<String>,
}
pub(crate) struct EpisodeTracker {
    run_id: i64,
    next_episode: Option<i64>,
    policy: EvidencePolicy,
    active: Option<Active>,
    candidate: Option<Candidate>,
    boundary: Option<Instant>,
    refusal_boundary: Option<Instant>,
    obscurity: Option<Instant>,
    obscured: bool,
    refusal_candidate: Option<Candidate>,
    last_refusal: Option<SemanticFingerprint>,
    last_output: Option<u64>,
    last_time: Option<Instant>,
    finished: bool,
    current_evidence: Option<Value>,
    annotations: AnnotationTracker,
}
fn elapsed(now: Instant, since: Instant, duration: Duration) -> bool {
    now.saturating_duration_since(since) >= duration
}
fn wall_time(value: &str) -> Result<()> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| error("observation requires RFC3339 UTC time"))?;
    ensure(
        parsed.offset().local_minus_utc() == 0,
        "observation requires UTC time",
    )
}
impl EpisodeTracker {
    pub(crate) fn new(run_id: i64, next_episode_seq: i64, policy: EvidencePolicy) -> Result<Self> {
        ensure(
            run_id > 0 && next_episode_seq > 0,
            "positive run/episode identities required",
        )?;
        Ok(Self {
            run_id,
            next_episode: Some(next_episode_seq),
            policy,
            active: None,
            candidate: None,
            boundary: None,
            refusal_boundary: None,
            obscurity: None,
            obscured: false,
            refusal_candidate: None,
            last_refusal: None,
            last_output: None,
            last_time: None,
            finished: false,
            current_evidence: None,
            annotations: AnnotationTracker::default(),
        })
    }
    pub(crate) fn active_episode(&self) -> Option<(i64, InteractionClass)> {
        self.active
            .as_ref()
            .map(|active| (active.sequence, active.class))
    }
    pub(super) fn current_evidence(&self) -> Option<&Value> {
        self.current_evidence.as_ref()
    }
    pub(super) fn continues_active(&self, value: &ScreenObservation) -> bool {
        value.kind == ObservationKind::PermissionContinuation
            && value.match_evidence == MatchEvidence::Complete
            && self.active.as_ref().is_some_and(|active| {
                active.class == InteractionClass::PermissionRequest
                    && value.continues_rule_id.as_ref() == Some(&active.rule_id)
            })
    }
    fn draft(
        &self,
        kind: RuntimeEventKind,
        episode: Option<(i64, InteractionClass)>,
        evidence: Value,
        reason: Option<InteractionCloseReason>,
        at: &str,
    ) -> TransitionDraft {
        TransitionDraft {
            run_id: self.run_id,
            episode_seq: episode.map(|p| p.0),
            classification: episode.map(|p| p.1),
            kind,
            observed_at: at.into(),
            evidence_json: evidence,
            close_reason: reason,
            cause_json: None,
        }
    }
    fn close(
        &mut self,
        reason: InteractionCloseReason,
        evidence: Option<&Value>,
        at: &str,
    ) -> Option<TransitionDraft> {
        let active = self.active.take()?;
        Some(self.draft(
            RuntimeEventKind::InteractionClosed,
            Some((active.sequence, active.class)),
            evidence.cloned().unwrap_or(active.evidence),
            Some(reason),
            at,
        ))
    }
    fn open(
        &mut self,
        value: &ScreenObservation,
        class: InteractionClass,
        evidence: Value,
        at: &str,
    ) -> Result<TransitionDraft> {
        let sequence = self
            .next_episode
            .ok_or_else(|| error("episode sequence exhausted"))?;
        self.next_episode = sequence.checked_add(1);
        let draft = self.draft(
            RuntimeEventKind::InteractionOpened,
            Some((sequence, class)),
            evidence.clone(),
            None,
            at,
        );
        self.active = Some(Active {
            sequence,
            class,
            rule_id: value.rule_id.clone(),
            fingerprint: value
                .fingerprint
                .clone()
                .ok_or_else(|| error("interaction fingerprint required"))?,
            evidence,
            native: value.native_correlation.clone(),
        });
        self.candidate = None;
        Ok(draft)
    }
    pub(crate) fn observe(
        &mut self,
        value: &ScreenObservation,
        now: Instant,
        at: &str,
    ) -> Result<EpisodeUpdate> {
        let mut update = self.observe_primary(value, now, at)?;
        self.observe_annotations(value, now, at, &mut update)?;
        Ok(update)
    }
    fn observe_annotations(
        &mut self,
        value: &ScreenObservation,
        now: Instant,
        at: &str,
        update: &mut EpisodeUpdate,
    ) -> Result<()> {
        self.annotations.age_absent(value, now, update);
        let mut unavailable_policy = false;
        for block in &value.annotations {
            if !self.annotations.stable(block, now, update) {
                continue;
            }
            let first = self.annotations.mark_emitted(block);
            let observed_mode = || {
                self.policy.displayed_mode(
                    &block.rule_id,
                    block.raw_evidence["sensitive"]
                        .as_bool()
                        .expect("detector evidence carries its sensitivity"),
                    block.raw_evidence["observed_mode"].as_str(),
                )
            };
            match block.kind {
                ObservationKind::AutomaticReviewAvailable => {
                    self.annotations.unavailable_emitted.clear();
                    let mode = observed_mode()?;
                    let state = (AutomaticReview::Available, mode.clone());
                    if !unavailable_policy && self.annotations.last_policy.as_ref() != Some(&state)
                    {
                        update.effective_policy = Some(EffectivePolicyObservation {
                            automatic_review: AutomaticReview::Available,
                            observed_mode: mode,
                            rule_id: block.rule_id.clone(),
                            observed_at: at.into(),
                        });
                        self.annotations.last_policy = Some(state);
                    }
                }
                ObservationKind::AutomaticRefusal if first => {
                    let evidence = self.policy.apply(&block.raw_evidence)?;
                    update.transitions.push(self.draft(
                        RuntimeEventKind::AutomaticRefusalObserved,
                        None,
                        evidence,
                        None,
                        at,
                    ));
                }
                ObservationKind::AutomaticReviewUnavailable
                    if first
                        && self
                            .annotations
                            .unavailable_emitted
                            .insert(block.rule_id.clone()) =>
                {
                    let evidence = self.policy.apply(&block.raw_evidence)?;
                    update.transitions.push(self.draft(
                        RuntimeEventKind::AutomaticReviewUnavailable,
                        None,
                        evidence,
                        None,
                        at,
                    ));
                    if !unavailable_policy {
                        let mode = observed_mode()?;
                        update.effective_policy = Some(EffectivePolicyObservation {
                            automatic_review: AutomaticReview::Unavailable,
                            observed_mode: mode.clone(),
                            rule_id: block.rule_id.clone(),
                            observed_at: at.into(),
                        });
                        self.annotations.last_policy = Some((AutomaticReview::Unavailable, mode));
                        unavailable_policy = true;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn observe_primary(
        &mut self,
        value: &ScreenObservation,
        now: Instant,
        at: &str,
    ) -> Result<EpisodeUpdate> {
        ensure(!self.finished, "closed tracker accepts no observations")?;
        wall_time(at)?;
        ensure(
            self.last_output.is_none_or(|old| value.output_seq >= old),
            "observation output sequence regressed",
        )?;
        ensure(
            self.last_time.is_none_or(|old| now >= old),
            "monotonic observation time regressed",
        )?;
        ensure(
            value.raw_evidence["screen_seq"].as_u64() == Some(value.output_seq),
            "observation evidence sequence mismatch",
        )?;
        let evidence = self.policy.apply(&value.raw_evidence)?;
        self.current_evidence = Some(evidence.clone());
        if matches!(
            value.kind,
            ObservationKind::Interaction(_) | ObservationKind::PermissionDenied
        ) {
            ensure(
                value.fingerprint.is_some(),
                "semantic observation requires fingerprint",
            )?;
        }
        self.last_output = Some(value.output_seq);
        self.last_time = Some(now);
        let mut update = EpisodeUpdate {
            visibility: Visibility::Unknown,
            eligibility: InputEligibility::Hold(GateReason::Unknown),
            transitions: Vec::new(),
            latest_evidence: None,
            next_deadline: None,
            effective_policy: None,
        };
        if let ObservationKind::Interaction(class) = value.kind {
            self.boundary = None;
            self.refusal_boundary = None;
            self.obscurity = None;
            self.obscured = false;
            self.refusal_candidate = None;
            update.visibility = Visibility::Visible;
            let fingerprint = value
                .fingerprint
                .as_ref()
                .expect("validated semantic fingerprint");
            if self
                .active
                .as_ref()
                .is_some_and(|active| &active.fingerprint == fingerprint)
            {
                self.candidate = None;
                let active = self.active.as_mut().expect("matching active episode");
                if !same_evidence_apart_from_sequence(&active.evidence, &evidence) {
                    update.latest_evidence = Some(InteractionEvidenceUpdate {
                        run_id: self.run_id,
                        episode_seq: active.sequence,
                        output_seq: value.output_seq,
                        evidence_json: evidence.clone(),
                    });
                    active.evidence = evidence;
                }
                update.eligibility = InputEligibility::Hold(GateReason::Interaction);
            } else {
                if self
                    .candidate
                    .as_ref()
                    .is_none_or(|c| &c.fingerprint != fingerprint)
                {
                    self.candidate = Some(Candidate {
                        fingerprint: fingerprint.clone(),
                        since: now,
                    });
                }
                let since = self.candidate.as_ref().expect("candidate installed").since;
                if elapsed(now, since, Duration::from_millis(100)) {
                    if let Some(closed) = self.close(InteractionCloseReason::Superseded, None, at) {
                        update.transitions.push(closed);
                    }
                    update
                        .transitions
                        .push(self.open(value, class, evidence, at)?);
                    update.eligibility = InputEligibility::Hold(GateReason::Interaction);
                } else {
                    update.eligibility = InputEligibility::Hold(GateReason::Candidate);
                    update.next_deadline = Some(since + Duration::from_millis(100));
                }
            }
            return Ok(update);
        }
        if self.continues_active(value) {
            self.candidate = None;
            self.boundary = None;
            self.refusal_boundary = None;
            self.obscurity = None;
            self.obscured = false;
            self.refusal_candidate = None;
            update.visibility = Visibility::Visible;
            update.eligibility = InputEligibility::Hold(GateReason::Interaction);
            return Ok(update);
        }
        self.candidate = None;
        if value.verified_noninteraction {
            self.obscurity = None;
            self.obscured = false;
            if self.active.is_some() {
                let since = *self.boundary.get_or_insert(now);
                if elapsed(now, since, Duration::from_millis(250)) {
                    if let Some(closed) =
                        self.close(InteractionCloseReason::NotVisible, Some(&evidence), at)
                    {
                        update.transitions.push(closed);
                    }
                    self.boundary = None;
                } else {
                    update.next_deadline = Some(since + Duration::from_millis(250));
                }
            }
        } else {
            self.boundary = None;
            if let Some(active) = &self.active {
                let sequence = active.sequence;
                let class = active.class;
                let since = *self.obscurity.get_or_insert(now);
                if !self.obscured {
                    if elapsed(now, since, Duration::from_secs(2)) {
                        update.transitions.push(self.draft(
                            RuntimeEventKind::InteractionObscured,
                            Some((sequence, class)),
                            evidence.clone(),
                            None,
                            at,
                        ));
                        self.obscured = true;
                    } else {
                        update.next_deadline = Some(since + Duration::from_secs(2));
                    }
                }
            }
        }
        if value.kind == ObservationKind::PermissionDenied {
            self.refusal_boundary = None;
            let fingerprint = value
                .fingerprint
                .as_ref()
                .expect("validated refusal fingerprint");
            if self.last_refusal.as_ref() != Some(fingerprint) {
                if self
                    .refusal_candidate
                    .as_ref()
                    .is_none_or(|candidate| &candidate.fingerprint != fingerprint)
                {
                    self.refusal_candidate = Some(Candidate {
                        fingerprint: fingerprint.clone(),
                        since: now,
                    });
                }
                let since = self
                    .refusal_candidate
                    .as_ref()
                    .expect("refusal candidate installed")
                    .since;
                if elapsed(now, since, Duration::from_millis(100)) {
                    let episode = self
                        .active
                        .as_ref()
                        .filter(|active| {
                            active.native.is_some() && active.native == value.native_correlation
                        })
                        .map(|active| (active.sequence, active.class));
                    update.transitions.push(self.draft(
                        RuntimeEventKind::PermissionDeniedObserved,
                        episode,
                        evidence.clone(),
                        None,
                        at,
                    ));
                    self.last_refusal = Some(fingerprint.clone());
                    self.refusal_candidate = None;
                } else {
                    let until = since + Duration::from_millis(100);
                    update.next_deadline =
                        Some(update.next_deadline.map_or(until, |old| old.min(until)));
                }
            }
        } else {
            self.refusal_candidate = None;
            if value.verified_noninteraction && self.last_refusal.is_some() {
                let since = *self.refusal_boundary.get_or_insert(now);
                if elapsed(now, since, Duration::from_millis(250)) {
                    self.last_refusal = None;
                    self.refusal_boundary = None;
                } else {
                    let until = since + Duration::from_millis(250);
                    update.next_deadline =
                        Some(update.next_deadline.map_or(until, |old| old.min(until)));
                }
            } else {
                self.refusal_boundary = None;
            }
        }
        update.visibility = if self.active.is_some() {
            Visibility::Obscured
        } else {
            Visibility::NotVisible
        };
        update.eligibility = match value.kind {
            ObservationKind::Working => InputEligibility::Hold(GateReason::Working),
            ObservationKind::ComposerDraft => InputEligibility::Hold(GateReason::Draft),
            ObservationKind::ComposerEmpty if self.active.is_some() => {
                InputEligibility::Hold(GateReason::Interaction)
            }
            ObservationKind::ComposerEmpty if value.match_evidence == MatchEvidence::Complete => {
                InputEligibility::ComposerEmpty
            }
            _ => InputEligibility::Hold(if self.active.is_some() {
                GateReason::Interaction
            } else {
                GateReason::Unknown
            }),
        };
        Ok(update)
    }
    pub(crate) fn reset_observation(&mut self, reason: GateReason) -> Result<EpisodeUpdate> {
        self.current_evidence = None;
        ensure(
            matches!(reason, GateReason::Gap | GateReason::ObserverUnavailable),
            "reset requires observation-gap cause",
        )?;
        self.candidate = None;
        self.boundary = None;
        self.refusal_boundary = None;
        self.obscurity = None;
        self.obscured = false;
        self.refusal_candidate = None;
        self.annotations.forget_candidates();
        Ok(EpisodeUpdate {
            visibility: Visibility::Unknown,
            eligibility: InputEligibility::Hold(reason),
            transitions: Vec::new(),
            latest_evidence: None,
            next_deadline: None,
            effective_policy: None,
        })
    }
    pub(crate) fn finish(
        &mut self,
        reason: InteractionCloseReason,
        at: &str,
    ) -> Result<EpisodeUpdate> {
        ensure(
            reason == InteractionCloseReason::ProcessExited,
            "live finish requires process exit",
        )?;
        wall_time(at)?;
        let transitions = self.close(reason, None, at).into_iter().collect();
        self.finished = true;
        self.candidate = None;
        Ok(EpisodeUpdate {
            visibility: Visibility::NotVisible,
            eligibility: InputEligibility::Hold(GateReason::ObserverUnavailable),
            transitions,
            latest_evidence: None,
            next_deadline: None,
            effective_policy: None,
        })
    }
}
pub(crate) struct EventSequencePool {
    range: Option<RangeInclusive<i64>>,
    high_water: i64,
}
impl EventSequencePool {
    pub(crate) fn exhausted(&self) -> bool {
        self.range.as_ref().is_none_or(|range| range.is_empty())
    }
    pub(crate) fn new() -> Self {
        Self {
            range: None,
            high_water: 0,
        }
    }
    pub(crate) fn supply(&mut self, reserved: RangeInclusive<i64>) -> Result<()> {
        ensure(
            self.range.as_ref().is_none_or(|range| range.is_empty()),
            "consume the current sequence grant first",
        )?;
        let first = *reserved.start();
        let last = *reserved.end();
        ensure(
            first > 0 && first > self.high_water && last.checked_sub(first) == Some(255),
            "positive increasing 256-value sequence grant required",
        )?;
        self.high_water = last;
        self.range = Some(reserved);
        Ok(())
    }
    pub(crate) fn materialize(&mut self, draft: &TransitionDraft) -> Result<ObservedTransition> {
        ensure(
            draft.run_id > 0
                && draft.episode_seq.is_some() == draft.classification.is_some()
                && draft.episode_seq.is_none_or(|seq| seq > 0),
            "valid transition identity required",
        )?;
        crate::broker::runtime::evidence::validate(&draft.evidence_json)?;
        wall_time(&draft.observed_at)?;
        let sequence = self
            .range
            .as_mut()
            .and_then(Iterator::next)
            .ok_or_else(|| error("reserved event sequence range exhausted"))?;
        Ok(ObservedTransition {
            run_id: draft.run_id,
            event_seq: sequence,
            episode_seq: draft.episode_seq,
            kind: draft.kind,
            classification: draft.classification,
            observed_at: draft.observed_at.clone(),
            evidence_json: draft.evidence_json.clone(),
            close_reason: draft.close_reason,
            cause_json: draft.cause_json.clone(),
        })
    }
}
