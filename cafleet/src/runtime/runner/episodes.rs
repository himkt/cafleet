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
                    // The worker may drop a queued available write, so each fresh emission of
                    // the indicator resubmits it; the storage write keeps the change-only rule.
                    if !unavailable_policy
                        && (first || self.annotations.last_policy.as_ref() != Some(&state))
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

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::coding_agent::ScreenObservation;
    pub(crate) const NOW: &str = "2026-10-26T15:35:00.000000+00:00";
    pub(crate) fn tracker() -> EpisodeTracker {
        EpisodeTracker::new(1, 1, EvidencePolicy::metadata()).unwrap()
    }

    pub(crate) fn open(
        tracker: &mut EpisodeTracker,
        value: &ScreenObservation,
        origin: Instant,
    ) -> EpisodeUpdate {
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
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::coding_agent::CodingAgent;
    use crate::coding_agent::profiles::catalog::test_support::synthetic_definitions as definitions;
    use crate::coding_agent::profiles::catalog::test_support::*;
    use crate::coding_agent::profiles::detector::test_support::*;
    use crate::coding_agent::profiles::*;
    use serde_json::json;
    mod continuation_episodes {
        use super::*;

        fn gated(bytes: &[u8], sequence: u64) -> ScreenObservation {
            let catalog = ProfileCatalog::from_definitions(&gated_fixture_definition()).unwrap();
            let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            state.feed(bytes).unwrap();
            let mut value = Detector::new(&catalog, CodingAgent::Codex, Some(VERSION))
                .unwrap()
                .observe(&state.snapshot())
                .unwrap();
            value.output_seq = sequence;
            value.raw_evidence["screen_seq"] = json!(sequence);
            value
        }

        fn request(sequence: u64) -> ScreenObservation {
            gated(
                &gated_fixture_form("write disposable.txt", "deny"),
                sequence,
            )
        }

        fn dialog(sequence: u64) -> ScreenObservation {
            let value = gated(&gated_reason_dialog(None), sequence);
            assert_eq!(value.kind, ObservationKind::PermissionContinuation);
            value
        }

        #[test]
        fn continuation_keeps_the_active_episode_open_and_visible_without_events() {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &request(1), origin);
            let episode = tracker.active_episode();
            assert!(episode.is_some());
            for (index, millis) in [200u64, 400, 2300, 4500].into_iter().enumerate() {
                let update = tracker
                    .observe(
                        &dialog(2 + index as u64),
                        origin + Duration::from_millis(millis),
                        NOW,
                    )
                    .unwrap();
                assert!(
                    update.transitions.is_empty(),
                    "a continuation emits no event at {millis} ms: {:?}",
                    update.transitions
                );
                assert_eq!(update.visibility, Visibility::Visible, "{millis} ms");
                assert_eq!(
                    update.eligibility,
                    InputEligibility::Hold(GateReason::Interaction),
                    "{millis} ms"
                );
                assert!(
                    update.latest_evidence.is_none(),
                    "the stored request evidence is unchanged"
                );
                assert_eq!(tracker.active_episode(), episode);
            }
            let returned = tracker
                .observe(&request(9), origin + Duration::from_secs(5), NOW)
                .unwrap();
            assert!(returned.transitions.is_empty());
            assert_eq!(returned.visibility, Visibility::Visible);
            assert_eq!(tracker.active_episode(), episode);
        }

        #[test]
        fn continuation_starts_no_obscured_timer_that_a_later_uncertainty_inherits() {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(&mut tracker, &request(1), origin);
            tracker
                .observe(&dialog(2), origin + Duration::from_millis(200), NOW)
                .unwrap();
            tracker
                .observe(&dialog(3), origin + Duration::from_millis(3000), NOW)
                .unwrap();
            let blank = gated(b"\x1b[0m\x1b[2J", 4);
            let uncertain = tracker
                .observe(&blank, origin + Duration::from_millis(3100), NOW)
                .unwrap();
            assert!(
                uncertain.transitions.is_empty(),
                "uncertainty after a continuation starts its own two-second interval"
            );
            let obscured = tracker
                .observe(&blank, origin + Duration::from_millis(5100), NOW)
                .unwrap();
            assert_eq!(obscured.transitions.len(), 1);
            assert_eq!(
                obscured.transitions[0].kind,
                RuntimeEventKind::InteractionObscured
            );
        }

        #[test]
        fn continuation_without_an_active_episode_is_an_unknown_screen() {
            let origin = Instant::now();
            let mut continuation = tracker();
            let mut unknown = tracker();
            let blank = gated(b"\x1b[0m\x1b[2J", 1);
            for millis in [0u64, 150, 400, 2500] {
                let at = origin + Duration::from_millis(millis);
                let left = continuation.observe(&dialog(1 + millis), at, NOW).unwrap();
                let mut blank = blank.clone();
                blank.output_seq = 1 + millis;
                blank.raw_evidence["screen_seq"] = json!(1 + millis);
                let right = unknown.observe(&blank, at, NOW).unwrap();
                assert!(left.transitions.is_empty(), "{millis} ms");
                assert_eq!(left.visibility, right.visibility, "{millis} ms");
                assert_eq!(left.eligibility, right.eligibility, "{millis} ms");
                assert_eq!(continuation.active_episode(), None);
            }
        }
    }
    #[test]
    fn runtime_detector_parser_bridge_visits_a_transient_candidate_before_a_later_clear() {
        struct ParserAlgorithmBridge {
            detector: Detector,
            tracker: EpisodeTracker,
            now: Instant,
            visits: Vec<bool>,
            candidate_holds: usize,
            transitions: usize,
        }
        impl ScreenObserver for ParserAlgorithmBridge {
            fn observe(
                &mut self,
                screen: &ScreenSnapshot,
            ) -> std::result::Result<(), crate::error::CafleetError> {
                let value = self.detector.observe(screen)?;
                self.visits.push(matches!(
                    value.kind,
                    ObservationKind::Interaction(InteractionClass::PermissionRequest)
                ));
                let update = self.tracker.observe(&value, self.now, NOW)?;
                self.candidate_holds += usize::from(matches!(
                    update.eligibility,
                    InputEligibility::Hold(GateReason::Candidate)
                ));
                self.transitions += update.transitions.len();
                Ok(())
            }
        }
        let catalog = ProfileCatalog::from_definitions(&definitions()).unwrap();
        let mut bridge = ParserAlgorithmBridge {
            detector: Detector::new(&catalog, CodingAgent::Codex, Some(VERSION)).unwrap(),
            tracker: tracker(),
            now: Instant::now(),
            visits: Vec::new(),
            candidate_holds: 0,
            transitions: 0,
        };
        let mut state = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let bytes = format!(
            "\x1b[2J\x1b[H{}\x1b[2J\x1b[Hordinary working output",
            lines("write disposable.txt").join("\r\n")
        );
        state.feed_observed(bytes.as_bytes(), &mut bridge).unwrap();
        let candidate = bridge
            .visits
            .iter()
            .position(|&value| value)
            .expect("transient complete form reaches the algorithms");
        assert!(bridge.visits[candidate + 1..].iter().any(|&value| !value));
        assert!(bridge.candidate_holds > 0);
        assert_eq!(bridge.transitions, 0);
    }
    fn sequenced(mut value: ScreenObservation, sequence: u64) -> ScreenObservation {
        value.output_seq = sequence;
        value.raw_evidence["screen_seq"] = json!(sequence);
        value
    }

    fn unknown(sequence: u64) -> ScreenObservation {
        sequenced(
            observation(&definitions(), &terminal(&[], 80, false).snapshot()),
            sequence,
        )
    }

    fn correlated(kind: &str, identity: Option<&str>, sequence: u64) -> ScreenObservation {
        let mut definition = definitions();
        definition["profiles"][0]["rules"][0]["observation"] = json!(kind);
        definition["profiles"][0]["rules"][0]["verified_noninteraction"] =
            json!(kind == "permission_denied");
        let mut rows = lines("write disposable.txt");
        if let Some(identity) = identity {
            rows.insert(5, format!("Native correlation: {identity}"));
            definition["profiles"][0]["rules"][0]["matchers"].as_array_mut().unwrap().push(json!({"id":"native","scope":"line","pattern":"^Native correlation: (?P<identity>[a-z0-9-]+)$"}));
            definition["profiles"][0]["rules"][0]["extract"]["native_request_id"] =
                json!({"matcher_id":"native","capture":"identity"});
        }
        sequenced(
            observation(&definition, &terminal(&rows, 80, false).snapshot()),
            sequence,
        )
    }

    fn secret_observation(
        secret: &str,
        sensitive: bool,
        cols: u16,
        sequence: u64,
    ) -> ScreenObservation {
        let mut definition = definitions();
        let rule = &mut definition["profiles"][0]["rules"][0];
        rule["sensitive"] = json!(sensitive);
        rule["matchers"][1]["pattern"] =
            json!(format!("^> (?P<label>Allow {})$", regex::escape(secret)));
        rule["matchers"][2]["pattern"] =
            json!(format!("^  (?P<label>Deny {})$", regex::escape(secret)));
        rule["matchers"].as_array_mut().unwrap().push(json!({"id":"native","scope":"line","pattern":format!("^Native correlation: (?P<identity>{})$",regex::escape(secret))}));
        rule["extract"]["native_request_id"] = json!({"matcher_id":"native","capture":"identity"});
        let mut rows = lines(secret);
        rows[2] = format!("> Allow {secret}");
        rows[3] = format!("  Deny {secret}");
        rows.insert(5, format!("Native correlation: {secret}"));
        let value = sequenced(
            observation(&definition, &terminal(&rows, cols, false).snapshot()),
            sequence,
        );
        for field in ["operation", "choices", "native_request_id", "excerpt"] {
            assert!(
                value.raw_evidence[field].to_string().contains(secret),
                "canary missing from raw {field}"
            );
        }
        value
    }

    #[test]
    fn runtime_episode_first_candidate_gates_before_the_100ms_opening_boundary() {
        let origin = Instant::now();
        let value = permission("write disposable.txt");
        let mut tracker = tracker();
        let first = tracker.observe(&value, origin, NOW).unwrap();
        assert!(first.transitions.is_empty());
        assert!(matches!(
            first.eligibility,
            InputEligibility::Hold(GateReason::Candidate)
        ));
        assert_eq!(
            first.next_deadline,
            Some(origin + Duration::from_millis(100))
        );
        assert!(
            tracker
                .observe(&value, origin + Duration::from_millis(99), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let stable = tracker
            .observe(&value, origin + Duration::from_millis(100), NOW)
            .unwrap();
        assert_eq!(stable.transitions.len(), 1);
        assert_eq!(stable.transitions[0].episode_seq, Some(1));
        assert_eq!(
            stable.transitions[0].kind,
            RuntimeEventKind::InteractionOpened
        );
        assert!(matches!(
            stable.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
    }

    #[test]
    fn runtime_episode_quiet_timer_recheck_uses_the_same_output_sequence() {
        let origin = Instant::now();
        let value = permission("write disposable.txt");
        let mut tracker = tracker();
        let opening = open(&mut tracker, &value, origin);
        assert_eq!(
            opening.transitions[0].evidence_json["screen_seq"],
            value.output_seq
        );
        assert_eq!(opening.transitions[0].observed_at, NOW);
        assert!(
            tracker
                .observe(&value, origin + Duration::from_secs(1), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
    }

    #[test]
    fn runtime_episode_incompatible_initial_candidate_restarts_stability() {
        let origin = Instant::now();
        let a = sequenced(permission("write first.txt"), 1);
        let b = sequenced(permission("write other.txt"), 2);
        let mut tracker = tracker();
        tracker.observe(&a, origin, NOW).unwrap();
        assert!(
            tracker
                .observe(&b, origin + Duration::from_millis(99), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&b, origin + Duration::from_millis(100), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let result = tracker
            .observe(&b, origin + Duration::from_millis(199), NOW)
            .unwrap();
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(result.transitions[0].episode_seq, Some(1));
    }

    #[test]
    fn runtime_episode_stable_replacement_closes_old_episode_before_opening_the_next() {
        let origin = Instant::now();
        let a = sequenced(permission("write first.txt"), 1);
        let b = sequenced(permission("write other.txt"), 2);
        let mut tracker = tracker();
        open(&mut tracker, &a, origin);
        let pending = tracker
            .observe(&b, origin + Duration::from_millis(200), NOW)
            .unwrap();
        assert!(pending.transitions.is_empty());
        assert!(pending.latest_evidence.is_none());
        assert!(matches!(pending.eligibility, InputEligibility::Hold(_)));
        assert!(
            tracker
                .observe(&b, origin + Duration::from_millis(299), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let stable = tracker
            .observe(&b, origin + Duration::from_millis(300), NOW)
            .unwrap();
        assert_eq!(stable.transitions.len(), 2);
        assert_eq!(
            stable.transitions[0].kind,
            RuntimeEventKind::InteractionClosed
        );
        assert_eq!(
            stable.transitions[0].close_reason,
            Some(InteractionCloseReason::Superseded)
        );
        assert_eq!(stable.transitions[0].episode_seq, Some(1));
        assert_eq!(
            stable.transitions[1].kind,
            RuntimeEventKind::InteractionOpened
        );
        assert_eq!(stable.transitions[1].episode_seq, Some(2));
    }

    #[test]
    fn runtime_episode_class_and_stable_choice_changes_supersede_after_stability() {
        for change in ["class", "choice"] {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(
                &mut tracker,
                &sequenced(permission("write disposable.txt"), 1),
                origin,
            );
            let mut definition = definitions();
            let mut rows = lines("write disposable.txt");
            if change == "class" {
                definition["profiles"][0]["rules"][0]["observation"] = json!("question");
            } else {
                definition["profiles"][0]["rules"][0]["matchers"][1]["pattern"] =
                    json!("^> (?P<label>Allow this request)$");
                rows[2] = "> Allow this request".into();
            }
            let next = sequenced(
                observation(&definition, &terminal(&rows, 80, false).snapshot()),
                2,
            );
            assert!(
                tracker
                    .observe(&next, origin + Duration::from_millis(200), NOW)
                    .unwrap()
                    .transitions
                    .is_empty()
            );
            let stable = tracker
                .observe(&next, origin + Duration::from_millis(300), NOW)
                .unwrap();
            assert_eq!(stable.transitions.len(), 2);
            assert_eq!(
                stable.transitions[0].close_reason,
                Some(InteractionCloseReason::Superseded)
            );
            assert_eq!(stable.transitions[1].episode_seq, Some(2));
            assert_eq!(
                stable.transitions[1].classification,
                Some(if change == "class" {
                    InteractionClass::Question
                } else {
                    InteractionClass::PermissionRequest
                })
            );
        }
    }

    #[test]
    fn runtime_episode_return_to_original_cancels_replacement_and_third_form_restarts_it() {
        let origin = Instant::now();
        let a = sequenced(permission("write first.txt"), 1);
        let b = sequenced(permission("write other.txt"), 2);
        let mut tracker = tracker();
        open(&mut tracker, &a, origin);
        tracker
            .observe(&b, origin + Duration::from_millis(200), NOW)
            .unwrap();
        assert!(
            tracker
                .observe(
                    &sequenced(permission("write first.txt"), 3),
                    origin + Duration::from_millis(250),
                    NOW
                )
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(
                    &sequenced(permission("write other.txt"), 4),
                    origin + Duration::from_millis(300),
                    NOW
                )
                .unwrap()
                .transitions
                .is_empty()
        );
        let c = sequenced(permission("write third.txt"), 5);
        assert!(
            tracker
                .observe(&c, origin + Duration::from_millis(350), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&c, origin + Duration::from_millis(400), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert_eq!(
            tracker
                .observe(&c, origin + Duration::from_millis(450), NOW)
                .unwrap()
                .transitions
                .len(),
            2
        );
    }

    #[test]
    fn runtime_episode_verified_composer_remains_held_until_the_250ms_episode_closure() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        let empty = sequenced(state_observation("composer_empty", true), 2);
        let first = tracker
            .observe(&empty, origin + Duration::from_millis(200), NOW)
            .unwrap();
        assert!(first.transitions.is_empty());
        assert!(matches!(empty.kind, ObservationKind::ComposerEmpty));
        assert!(matches!(
            first.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
        assert_eq!(
            first.next_deadline,
            Some(origin + Duration::from_millis(450))
        );
        let before = tracker
            .observe(&empty, origin + Duration::from_millis(449), NOW)
            .unwrap();
        assert!(before.transitions.is_empty());
        assert!(matches!(
            before.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
        let closed = tracker
            .observe(&empty, origin + Duration::from_millis(450), NOW)
            .unwrap();
        assert_eq!(closed.transitions.len(), 1);
        assert_eq!(
            closed.transitions[0].kind,
            RuntimeEventKind::InteractionClosed
        );
        assert_eq!(
            closed.transitions[0].close_reason,
            Some(InteractionCloseReason::NotVisible)
        );
        assert!(matches!(
            closed.eligibility,
            InputEligibility::ComposerEmpty
        ));
        assert!(closed.latest_evidence.is_none());
    }

    #[test]
    fn runtime_episode_uncertainty_interrupts_the_verified_boundary() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        tracker
            .observe(
                &sequenced(state_observation("composer_empty", true), 2),
                origin + Duration::from_millis(200),
                NOW,
            )
            .unwrap();
        tracker
            .observe(&unknown(3), origin + Duration::from_millis(449), NOW)
            .unwrap();
        let empty = sequenced(state_observation("composer_empty", true), 4);
        assert!(
            tracker
                .observe(&empty, origin + Duration::from_millis(450), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&empty, origin + Duration::from_millis(699), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert_eq!(
            tracker
                .observe(&empty, origin + Duration::from_millis(700), NOW)
                .unwrap()
                .transitions
                .len(),
            1
        );
    }

    #[test]
    fn runtime_episode_continuous_uncertainty_emits_one_obscured_event_at_two_seconds() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        let obscured = unknown(2);
        let first = tracker
            .observe(&obscured, origin + Duration::from_millis(200), NOW)
            .unwrap();
        assert!(first.transitions.is_empty());
        assert!(matches!(first.visibility, Visibility::Obscured));
        assert!(matches!(first.eligibility, InputEligibility::Hold(_)));
        assert!(
            tracker
                .observe(&obscured, origin + Duration::from_millis(2199), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let stable = tracker
            .observe(&obscured, origin + Duration::from_millis(2200), NOW)
            .unwrap();
        assert_eq!(stable.transitions.len(), 1);
        assert_eq!(
            stable.transitions[0].kind,
            RuntimeEventKind::InteractionObscured
        );
        assert_eq!(stable.transitions[0].episode_seq, Some(1));
        assert!(
            tracker
                .observe(&obscured, origin + Duration::from_secs(5), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let returned = tracker
            .observe(
                &sequenced(permission("write disposable.txt"), 3),
                origin + Duration::from_secs(6),
                NOW,
            )
            .unwrap();
        assert!(returned.transitions.is_empty());
        assert!(matches!(returned.visibility, Visibility::Visible));
        assert!(matches!(
            returned.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
        assert!(returned.latest_evidence.is_none());
    }

    #[test]
    fn runtime_episode_equivalent_redraw_schedules_no_refresh_and_no_opening() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        let redraw = sequenced(
            observation(
                &definitions(),
                &terminal(&lines("write disposable.txt"), 100, false).snapshot(),
            ),
            2,
        );
        for (sequence, millis) in [(2, 200), (3, 300)] {
            let update = tracker
                .observe(
                    &sequenced(redraw.clone(), sequence),
                    origin + Duration::from_millis(millis),
                    NOW,
                )
                .unwrap();
            assert!(update.transitions.is_empty());
            assert!(matches!(update.visibility, Visibility::Visible));
            assert!(update.latest_evidence.is_none());
        }
    }

    fn detailed(detail: &str, sequence: u64) -> ScreenObservation {
        let mut rows = lines("write disposable.txt");
        rows.insert(5, format!("Detail: {detail}"));
        sequenced(
            observation(&definitions(), &terminal(&rows, 80, false).snapshot()),
            sequence,
        )
    }

    #[test]
    fn runtime_episode_excerpt_only_change_refreshes_policy_evidence_without_another_opening() {
        let secret = "synthetic-secret-194";
        let policy = EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":[secret],"sensitive_rules":[]}),
            &[RULE],
        )
        .unwrap();
        let mut tracker = EpisodeTracker::new(1, 1, policy).unwrap();
        let origin = Instant::now();
        let first = detailed("first", 1);
        assert!(first.fingerprint == detailed("second", 1).fingerprint);
        open(&mut tracker, &first, origin);
        let unchanged = tracker
            .observe(
                &detailed("first", 2),
                origin + Duration::from_millis(200),
                NOW,
            )
            .unwrap();
        assert!(unchanged.transitions.is_empty());
        assert!(unchanged.latest_evidence.is_none());
        let changed = tracker
            .observe(
                &detailed("second", 3),
                origin + Duration::from_millis(300),
                NOW,
            )
            .unwrap();
        assert!(changed.transitions.is_empty());
        let refresh = changed.latest_evidence.unwrap();
        assert_eq!(
            (refresh.run_id, refresh.episode_seq, refresh.output_seq),
            (1, 1, 3)
        );
        assert_eq!(refresh.evidence_json["screen_seq"], 3);
        assert!(
            refresh.evidence_json["excerpt"]
                .as_str()
                .unwrap()
                .contains("Detail: second")
        );
        let secret_detail = tracker
            .observe(
                &detailed(secret, 4),
                origin + Duration::from_millis(400),
                NOW,
            )
            .unwrap();
        assert!(secret_detail.transitions.is_empty());
        let refresh = secret_detail.latest_evidence.unwrap();
        assert_eq!(refresh.evidence_json["excerpt"], json!("[redacted]"));
        assert!(!refresh.evidence_json.to_string().contains(secret));
        let settled = tracker
            .observe(
                &detailed(secret, 5),
                origin + Duration::from_millis(500),
                NOW,
            )
            .unwrap();
        assert!(settled.latest_evidence.is_none());
    }

    #[test]
    fn runtime_episode_identical_requests_are_distinct_only_after_a_verified_boundary() {
        let origin = Instant::now();
        let mut tracker = tracker();
        let first = sequenced(permission("write disposable.txt"), 1);
        open(&mut tracker, &first, origin);
        assert!(
            tracker
                .observe(
                    &sequenced(permission("write disposable.txt"), 2),
                    origin + Duration::from_millis(200),
                    NOW
                )
                .unwrap()
                .transitions
                .is_empty()
        );
        let empty = sequenced(state_observation("composer_empty", true), 3);
        tracker
            .observe(&empty, origin + Duration::from_millis(300), NOW)
            .unwrap();
        tracker
            .observe(&empty, origin + Duration::from_millis(550), NOW)
            .unwrap();
        let second = sequenced(permission("write disposable.txt"), 4);
        assert!(
            tracker
                .observe(&second, origin + Duration::from_millis(600), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let opened = tracker
            .observe(&second, origin + Duration::from_millis(700), NOW)
            .unwrap();
        assert_eq!(opened.transitions.len(), 1);
        assert_eq!(opened.transitions[0].episode_seq, Some(2));
    }

    #[test]
    fn runtime_episode_refusal_requires_stability_and_retains_deduplication_across_uncertainty() {
        let origin = Instant::now();
        let mut tracker = tracker();
        let refusal = correlated("permission_denied", None, 1);
        assert!(
            tracker
                .observe(&refusal, origin, NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&refusal, origin + Duration::from_millis(99), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let result = tracker
            .observe(&refusal, origin + Duration::from_millis(100), NOW)
            .unwrap();
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(
            result.transitions[0].kind,
            RuntimeEventKind::PermissionDeniedObserved
        );
        assert_eq!(result.transitions[0].episode_seq, None);
        tracker
            .observe(&unknown(2), origin + Duration::from_millis(150), NOW)
            .unwrap();
        let repeated = correlated("permission_denied", None, 3);
        assert!(
            tracker
                .observe(&repeated, origin + Duration::from_millis(200), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&repeated, origin + Duration::from_millis(300), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
    }

    #[test]
    fn runtime_episode_refusal_dedup_resets_after_a_verified_250ms_boundary() {
        let origin = Instant::now();
        let mut tracker = tracker();
        let first = correlated("permission_denied", None, 1);
        tracker.observe(&first, origin, NOW).unwrap();
        tracker
            .observe(&first, origin + Duration::from_millis(100), NOW)
            .unwrap();
        let empty = sequenced(state_observation("composer_empty", true), 2);
        tracker
            .observe(&empty, origin + Duration::from_millis(200), NOW)
            .unwrap();
        tracker
            .observe(&empty, origin + Duration::from_millis(450), NOW)
            .unwrap();
        let next = correlated("permission_denied", None, 3);
        tracker
            .observe(&next, origin + Duration::from_millis(500), NOW)
            .unwrap();
        let result = tracker
            .observe(&next, origin + Duration::from_millis(600), NOW)
            .unwrap();
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(
            result.transitions[0].kind,
            RuntimeEventKind::PermissionDeniedObserved
        );
    }

    #[test]
    fn runtime_episode_uncertainty_restarts_a_pending_refusal_before_its_first_emission() {
        let origin = Instant::now();
        let mut tracker = tracker();
        tracker
            .observe(&correlated("permission_denied", None, 1), origin, NOW)
            .unwrap();
        tracker
            .observe(&unknown(2), origin + Duration::from_millis(99), NOW)
            .unwrap();
        let refusal = correlated("permission_denied", None, 3);
        assert!(
            tracker
                .observe(&refusal, origin + Duration::from_millis(100), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&refusal, origin + Duration::from_millis(199), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let result = tracker
            .observe(&refusal, origin + Duration::from_millis(200), NOW)
            .unwrap();
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(
            result.transitions[0].kind,
            RuntimeEventKind::PermissionDeniedObserved
        );
    }

    #[test]
    fn runtime_episode_refusal_linkage_requires_a_matching_native_correlation() {
        for (identity, expected) in [
            (None, None),
            (Some("request-2"), None),
            (Some("request-1"), Some(1)),
        ] {
            let origin = Instant::now();
            let mut tracker = tracker();
            let request = correlated("permission_request", Some("request-1"), 1);
            open(&mut tracker, &request, origin);
            let result = correlated("permission_denied", identity, 2);
            tracker
                .observe(&result, origin + Duration::from_millis(200), NOW)
                .unwrap();
            let stable = tracker
                .observe(&result, origin + Duration::from_millis(300), NOW)
                .unwrap();
            let event = stable
                .transitions
                .iter()
                .find(|event| event.kind == RuntimeEventKind::PermissionDeniedObserved)
                .unwrap();
            assert_eq!(event.episode_seq, expected);
        }
    }

    #[test]
    fn runtime_episode_gap_reset_preserves_open_identity_and_discards_pending_timers() {
        for reason in [GateReason::Gap, GateReason::ObserverUnavailable] {
            let origin = Instant::now();
            let mut tracker = tracker();
            open(
                &mut tracker,
                &sequenced(permission("write first.txt"), 1),
                origin,
            );
            tracker
                .observe(
                    &sequenced(permission("write other.txt"), 2),
                    origin + Duration::from_millis(200),
                    NOW,
                )
                .unwrap();
            let reset = tracker.reset_observation(reason).unwrap();
            assert!(reset.transitions.is_empty());
            assert!(reset.next_deadline.is_none());
            assert!(matches!(reset.visibility, Visibility::Unknown));
            assert!(matches!(reset.eligibility, InputEligibility::Hold(_)));
            let returned = tracker
                .observe(
                    &sequenced(permission("write first.txt"), 3),
                    origin + Duration::from_secs(5),
                    NOW,
                )
                .unwrap();
            assert!(returned.transitions.is_empty());
            assert!(matches!(returned.visibility, Visibility::Visible));
        }
    }

    #[test]
    fn runtime_episode_gap_reset_restarts_an_uncommitted_candidate() {
        let origin = Instant::now();
        let mut tracker = tracker();
        let value = permission("write disposable.txt");
        tracker.observe(&value, origin, NOW).unwrap();
        tracker.reset_observation(GateReason::Gap).unwrap();
        assert!(
            tracker
                .observe(&value, origin + Duration::from_secs(5), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert_eq!(
            tracker
                .observe(&value, origin + Duration::from_millis(5100), NOW)
                .unwrap()
                .transitions
                .len(),
            1
        );
    }

    #[test]
    fn runtime_episode_gap_reset_restarts_closure_debounce_and_keeps_the_active_episode_held() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        let empty = sequenced(state_observation("composer_empty", true), 2);
        tracker
            .observe(&empty, origin + Duration::from_millis(200), NOW)
            .unwrap();
        tracker.reset_observation(GateReason::Gap).unwrap();
        let fresh = tracker
            .observe(&empty, origin + Duration::from_secs(5), NOW)
            .unwrap();
        assert!(fresh.transitions.is_empty());
        assert!(matches!(
            fresh.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
        let before = tracker
            .observe(&empty, origin + Duration::from_millis(5249), NOW)
            .unwrap();
        assert!(before.transitions.is_empty());
        assert!(matches!(
            before.eligibility,
            InputEligibility::Hold(GateReason::Interaction)
        ));
        let closed = tracker
            .observe(&empty, origin + Duration::from_millis(5250), NOW)
            .unwrap();
        assert_eq!(closed.transitions.len(), 1);
        assert!(matches!(
            closed.eligibility,
            InputEligibility::ComposerEmpty
        ));
    }

    #[test]
    fn runtime_episode_gap_reset_discards_the_unverified_obscurity_interval() {
        let origin = Instant::now();
        let mut tracker = tracker();
        open(
            &mut tracker,
            &sequenced(permission("write disposable.txt"), 1),
            origin,
        );
        tracker
            .observe(&unknown(2), origin + Duration::from_millis(200), NOW)
            .unwrap();
        tracker.reset_observation(GateReason::Gap).unwrap();
        let unseen = unknown(3);
        assert!(
            tracker
                .observe(&unseen, origin + Duration::from_millis(2200), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        assert!(
            tracker
                .observe(&unseen, origin + Duration::from_millis(4199), NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
        let result = tracker
            .observe(&unseen, origin + Duration::from_millis(4200), NOW)
            .unwrap();
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(
            result.transitions[0].kind,
            RuntimeEventKind::InteractionObscured
        );
    }

    #[test]
    fn runtime_episode_reset_and_finish_validate_their_lifecycle_scope() {
        for reason in [
            GateReason::Candidate,
            GateReason::Interaction,
            GateReason::Working,
            GateReason::Draft,
            GateReason::Unknown,
        ] {
            assert!(tracker().reset_observation(reason).is_err());
        }
        for reason in [
            InteractionCloseReason::MonitorLost,
            InteractionCloseReason::NotVisible,
            InteractionCloseReason::Superseded,
        ] {
            assert!(tracker().finish(reason, NOW).is_err());
        }
        let origin = Instant::now();
        let mut tracker = tracker();
        open(&mut tracker, &permission("write disposable.txt"), origin);
        let closed = tracker
            .finish(InteractionCloseReason::ProcessExited, NOW)
            .unwrap();
        assert_eq!(closed.transitions.len(), 1);
        assert_eq!(
            closed.transitions[0].close_reason,
            Some(InteractionCloseReason::ProcessExited)
        );
        assert!(
            tracker
                .finish(InteractionCloseReason::ProcessExited, NOW)
                .unwrap()
                .transitions
                .is_empty()
        );
    }

    #[test]
    fn runtime_episode_noninteraction_states_gate_independently_without_allocating_rows() {
        for (kind, reason) in [
            ("working", Some(GateReason::Working)),
            ("composer_draft", Some(GateReason::Draft)),
            ("composer_empty", None),
        ] {
            let mut tracker = tracker();
            let result = tracker
                .observe(&state_observation(kind, true), Instant::now(), NOW)
                .unwrap();
            assert!(result.transitions.is_empty());
            assert!(result.latest_evidence.is_none());
            match reason {
                Some(GateReason::Working) => assert!(matches!(
                    result.eligibility,
                    InputEligibility::Hold(GateReason::Working)
                )),
                Some(_) => assert!(matches!(
                    result.eligibility,
                    InputEligibility::Hold(GateReason::Draft)
                )),
                None => assert!(matches!(
                    result.eligibility,
                    InputEligibility::ComposerEmpty
                )),
            }
        }
        let result = tracker().observe(&unknown(1), Instant::now(), NOW).unwrap();
        assert!(matches!(
            result.eligibility,
            InputEligibility::Hold(GateReason::Unknown)
        ));
    }

    #[test]
    fn runtime_episode_rejects_regressing_output_sequences_and_invalid_identity() {
        assert!(EpisodeTracker::new(0, 1, EvidencePolicy::metadata()).is_err());
        assert!(EpisodeTracker::new(1, 0, EvidencePolicy::metadata()).is_err());
        let origin = Instant::now();
        let mut tracker = tracker();
        tracker
            .observe(
                &sequenced(permission("write disposable.txt"), 2),
                origin,
                NOW,
            )
            .unwrap();
        assert!(
            tracker
                .observe(
                    &sequenced(permission("write disposable.txt"), 1),
                    origin + Duration::from_millis(100),
                    NOW
                )
                .is_err()
        );
    }

    #[test]
    fn runtime_episode_evidence_policy_precedes_every_transition_and_redraw_update() {
        let secret = "synthetic-secret-194";
        for (sensitive, named_sensitive) in [(false, false), (true, false), (false, true)] {
            let value = secret_observation(secret, sensitive, 80, 1);
            let policy = EvidencePolicy::redacted(
                    &json!({"schema_version":1,"patterns":[secret],"sensitive_rules":if named_sensitive {vec![RULE]}else{vec![]}}),
                    &[RULE],
                )
                .unwrap();
            let mut tracker = EpisodeTracker::new(1, 1, policy).unwrap();
            let origin = Instant::now();
            let result = open(&mut tracker, &value, origin);
            let evidence = &result.transitions[0].evidence_json;
            assert!(!evidence.to_string().contains(secret));
            assert!(evidence.get("semantic_fingerprint").is_none());
            assert_eq!(
                evidence["operation"],
                if sensitive || named_sensitive {
                    Value::Null
                } else {
                    json!("[redacted]")
                }
            );
            let redraw = secret_observation(secret, sensitive, 100, 2);
            let update = tracker
                .observe(&redraw, origin + Duration::from_millis(200), NOW)
                .unwrap();
            assert!(update.transitions.is_empty());
            assert!(update.latest_evidence.is_none());
        }
    }

    #[test]
    fn runtime_episode_metadata_removes_each_screen_field_and_leaves_no_redraw_refresh() {
        let secret = "synthetic-secret-194";
        let origin = Instant::now();
        let mut tracker = tracker();
        let value = secret_observation(secret, false, 80, 1);
        let opened = open(&mut tracker, &value, origin);
        for field in ["operation", "choices", "native_request_id", "excerpt"] {
            assert!(opened.transitions[0].evidence_json[field].is_null());
        }
        assert!(
            !opened.transitions[0]
                .evidence_json
                .to_string()
                .contains(secret)
        );
        let mut pool = EventSequencePool::new();
        pool.supply(1..=256).unwrap();
        let event = pool.materialize(&opened.transitions[0]).unwrap();
        assert!(!serde_json::to_string(&event).unwrap().contains(secret));
        for (sequence, detail) in [(2, "first"), (3, secret)] {
            let mut redraw = secret_observation(secret, false, 100, sequence);
            redraw.raw_evidence["excerpt"] = json!(format!("Detail: {detail}"));
            let update = tracker
                .observe(
                    &redraw,
                    origin + Duration::from_millis(100 + sequence * 100),
                    NOW,
                )
                .unwrap();
            assert!(update.transitions.is_empty());
            assert!(update.latest_evidence.is_none());
        }
    }

    #[test]
    fn runtime_episode_metadata_digest_contains_no_hash_of_hidden_semantics() {
        let origin = Instant::now();
        let first = sequenced(permission("write first-secret.txt"), 1);
        let second = sequenced(permission("write other-secret.txt"), 1);
        assert!(first.fingerprint != second.fingerprint);
        let a = open(&mut tracker(), &first, origin);
        let b = open(&mut tracker(), &second, origin);
        assert_eq!(
            a.transitions[0].evidence_json,
            b.transitions[0].evidence_json
        );
        assert_eq!(
            a.transitions[0].evidence_json["evidence_digest"],
            b.transitions[0].evidence_json["evidence_digest"]
        );
    }

    #[test]
    fn runtime_episode_sequence_pool_uses_only_committed_grants_and_retains_borrowed_drafts() {
        let origin = Instant::now();
        let mut tracker = tracker();
        let update = open(&mut tracker, &permission("write disposable.txt"), origin);
        let draft = &update.transitions[0];
        let mut pool = EventSequencePool::new();
        assert!(pool.materialize(draft).is_err());
        assert_eq!(draft.observed_at, NOW);
        pool.supply(1..=256).unwrap();
        assert!(pool.supply(257..=512).is_err());
        for expected in 1..=256 {
            let event = pool.materialize(draft).unwrap();
            assert_eq!(event.event_seq, expected);
            assert_eq!(event.observed_at, draft.observed_at);
            assert_eq!(event.evidence_json, draft.evidence_json);
        }
        assert!(pool.materialize(draft).is_err());
        pool.supply(513..=768).unwrap();
        assert_eq!(pool.materialize(draft).unwrap().event_seq, 513);
    }

    #[test]
    fn runtime_episode_sequence_pool_rejects_invalid_and_overlapping_reservations() {
        for range in [
            0..=255,
            1..=255,
            1..=257,
            (-255)..=0,
            (i64::MAX - 1)..=i64::MAX,
        ] {
            assert!(EventSequencePool::new().supply(range).is_err());
        }
        let origin = Instant::now();
        let mut tracker = tracker();
        let update = open(&mut tracker, &permission("write disposable.txt"), origin);
        let mut pool = EventSequencePool::new();
        pool.supply(257..=512).unwrap();
        for _ in 0..256 {
            pool.materialize(&update.transitions[0]).unwrap();
        }
        assert!(pool.supply(257..=512).is_err());
        assert!(pool.supply(1..=256).is_err());
        pool.supply(513..=768).unwrap();
        assert_eq!(
            pool.materialize(&update.transitions[0]).unwrap().event_seq,
            513
        );
    }
}
