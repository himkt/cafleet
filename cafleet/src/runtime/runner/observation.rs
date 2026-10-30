use super::*;
use crate::broker::runtime::{
    self as storage, InteractionCloseReason, OwnerKind, RunRecord, RunStatus, RuntimeCauseKind,
    RuntimeEventKind, evidence::EvidencePolicy,
};
use crate::coding_agent::{AnswerMatch, Detector, ProfileCatalog};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

pub(crate) struct ObservationMoment {
    pub(crate) monotonic: Instant,
    pub(crate) observed_at: String,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationClockKind {
    Real,
    Deterministic,
}
pub(crate) trait ObservationClock: Send + Sync {
    fn sample(&self) -> ObservationMoment;
    #[cfg(test)]
    fn kind(&self) -> ObservationClockKind;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationPhase {
    ChildOutput,
    QuietTimer,
    Resize,
    ShutdownDrain,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationOperation {
    Parser,
    Detector,
}
#[cfg(test)]
pub(crate) trait ObservationHooks: Send + Sync {
    fn before(&self, site: ObservationPhase, operation: ObservationOperation) -> Result<()>;
    fn order_shutdown_parsing_after_native_exit(&self, _deadline: Instant) -> bool {
        false
    }
}
pub(crate) struct RealObservationClock;
impl ObservationClock for RealObservationClock {
    fn sample(&self) -> ObservationMoment {
        ObservationMoment {
            monotonic: Instant::now(),
            observed_at: crate::time::now_stamp(),
        }
    }
    #[cfg(test)]
    fn kind(&self) -> ObservationClockKind {
        ObservationClockKind::Real
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum ObservationFailure {
    Parser,
    Detector,
    Sequence,
    Sink,
}
impl ObservationFailure {
    pub(crate) const ALL: [Self; 4] = [Self::Parser, Self::Detector, Self::Sequence, Self::Sink];
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Parser => "parser_unavailable",
            Self::Detector => "detector_unavailable",
            Self::Sequence => "sequence_unavailable",
            Self::Sink => "sink_unavailable",
        }
    }
}
#[derive(Debug)]
pub(crate) struct ObservationStatus {
    pub(crate) visibility: Visibility,
    pub(crate) eligibility: InputEligibility,
    pub(crate) next_deadline: Option<Instant>,
    pub(crate) retained_transition: bool,
    pub(crate) retained_draft: bool,
    pub(crate) retained_evidence: bool,
    pub(crate) gap_count: u64,
    pub(crate) diagnostic: Option<String>,
}
struct PendingEpisodeUpdate {
    update: EpisodeUpdate,
    next_draft: usize,
    materialized: Option<storage::ObservedTransition>,
}
struct PendingGap {
    started_at: String,
    count: u64,
    reason: ObservationFailure,
}
struct PendingExit {
    exit: ChildExit,
    at: String,
    screen_seq: u64,
}
pub(crate) struct ObservationController {
    run: RunRecord,
    detector: Detector,
    tracker: EpisodeTracker,
    pool: EventSequencePool,
    pending: Option<PendingEpisodeUpdate>,
    gap: Option<PendingGap>,
    gap_count: u64,
    terminal_fenced: bool,
    terminal_observation_loss: u64,
    diagnostic: Option<String>,
    visibility: Visibility,
    eligibility: InputEligibility,
    next_deadline: Option<Instant>,
    started: bool,
    exit: Option<PendingExit>,
    exit_recorded: Option<ChildExit>,
    last_time: Option<Instant>,
    last_output: Option<u64>,
    catalog: ProfileCatalog,
    request_generation: u64,
    request_generation_failed: bool,
    current_request: Option<ObservedRequest>,
    current_continuation: Option<ObservedContinuation>,
    transit_request: Option<ObservedRequest>,
    request_region: Option<RequestRegion>,
    current_classification: Option<storage::InteractionClass>,
    pending_error: Option<CafleetError>,
    #[cfg(test)]
    capture: Option<NativeCapturePort>,
    #[cfg(test)]
    observation_hooks: Option<std::sync::Arc<dyn ObservationHooks>>,
}
struct RequestRegion {
    top: usize,
    rows: Vec<String>,
}
impl RequestRegion {
    fn of(screen: &ScreenSnapshot, top: u16, bottom: u16) -> Self {
        let top = usize::from(top);
        Self {
            top,
            rows: screen
                .screen
                .rows(0, screen.cols)
                .skip(top)
                .take(usize::from(bottom) - top)
                .collect(),
        }
    }
    fn unchanged(&self, screen: &ScreenSnapshot) -> bool {
        screen
            .screen
            .rows(0, screen.cols)
            .skip(self.top)
            .take(self.rows.len())
            .eq(self.rows.iter().cloned())
    }
}
#[derive(Clone)]
pub(crate) struct ObservedRequest {
    pub(super) request_generation: u64,
    pub(super) answer_matches: Vec<AnswerMatch>,
    pub(super) episode_seq: i64,
    pub(super) classification: storage::InteractionClass,
    pub(super) native_version: String,
    pub(super) detector_profile_version: String,
    pub(super) request_rule_id: String,
    pub(super) semantic: SemanticFingerprint,
    pub(super) choices: Vec<String>,
    pub(super) focus: String,
    pub(super) native_correlation: Option<String>,
    pub(super) output_seq: u64,
}
impl ObservedRequest {
    pub(super) fn same_request(&self, other: &Self) -> bool {
        self.episode_seq == other.episode_seq
            && self.classification == other.classification
            && self.native_version == other.native_version
            && self.detector_profile_version == other.detector_profile_version
            && self.request_rule_id == other.request_rule_id
            && self.semantic == other.semantic
            && self.choices == other.choices
            && self.focus == other.focus
            && self.native_correlation == other.native_correlation
            && self.answer_matches.len() == other.answer_matches.len()
            && self
                .answer_matches
                .iter()
                .zip(&other.answer_matches)
                .all(|(a, b)| a.same_request(b))
    }
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ObservedContinuation {
    pub(super) request_generation: u64,
    pub(super) episode_seq: i64,
    pub(super) rule_id: String,
    pub(super) fingerprint: SemanticFingerprint,
}
pub(crate) struct OrderedObservation<'a> {
    controller: &'a mut ObservationController,
    writer: &'a EventWriter,
    clock: &'a dyn ObservationClock,
    phase: ObservationPhase,
}
impl ScreenObserver for OrderedObservation<'_> {
    fn input_health(&self) -> Option<ObservationHealth> {
        Some(self.controller.health(self.writer))
    }
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        self.controller
            .observe_screen(screen, self.writer, self.clock, self.phase)
    }
    fn failures_are_fatal(&self) -> bool {
        true
    }
    fn parser_failure(&mut self) -> Result<()> {
        self.controller
            .observer_failed(ObservationFailure::Parser, self.clock)
    }
}
impl ObservationController {
    pub(crate) fn new(
        run: &RunRecord,
        catalog: &ProfileCatalog,
        policy: EvidencePolicy,
        initial: RangeInclusive<i64>,
    ) -> Result<Self> {
        ensure(
            run.run_id > 0
                && run.owner_epoch > 0
                && run.owner_kind == OwnerKind::Runner
                && run.status == RunStatus::Starting
                && run.child_pid.is_none()
                && run.child_start_identity.is_none()
                && run.started_at.is_none()
                && run.ended_at.is_none()
                && run.recovery_origin_status.is_none()
                && run.event_seq_reserved_through == 256
                && initial == (1..=256),
            "observation controller requires fresh committed initial grant",
        )?;
        let backend = crate::coding_agent::coding_agent(&run.backend)
            .ok_or_else(|| error("unknown observer backend"))?;
        let capabilities = catalog.capabilities(backend, run.native_version.as_deref());
        ensure(
            capabilities.profile_version == run.profile_version
                && capabilities.detection == run.coverage,
            "observer profile/coverage mismatch",
        )?;
        let mut pool = EventSequencePool::default();
        pool.supply(initial)?;
        Ok(Self {
            run: run.clone(),
            detector: Detector::new(catalog, backend, run.native_version.as_deref()),
            tracker: EpisodeTracker::new(run.run_id, 1, policy)?,
            pool,
            pending: None,
            gap: None,
            gap_count: 0,
            terminal_fenced: false,
            terminal_observation_loss: 0,
            diagnostic: None,
            visibility: Visibility::Unknown,
            eligibility: InputEligibility::Hold(GateReason::Unknown),
            next_deadline: None,
            started: false,
            exit: None,
            exit_recorded: None,
            last_time: None,
            last_output: None,
            catalog: catalog.clone(),
            request_generation: 0,
            request_generation_failed: false,
            current_request: None,
            current_continuation: None,
            transit_request: None,
            request_region: None,
            current_classification: None,
            pending_error: None,
            #[cfg(test)]
            capture: None,
            #[cfg(test)]
            observation_hooks: None,
        })
    }
    pub(crate) fn current_request(&self) -> Option<&ObservedRequest> {
        self.current_request.as_ref()
    }
    pub(super) fn current_continuation(&self) -> Option<&ObservedContinuation> {
        self.current_continuation.as_ref()
    }
    pub(super) fn gap_count(&self) -> u64 {
        self.gap_count
    }
    pub(super) fn active_episode_seq(&self) -> Option<i64> {
        self.tracker.active_episode().map(|(sequence, _)| sequence)
    }
    pub(super) fn request_generation(&self) -> u64 {
        self.request_generation
    }
    pub(super) fn current_evidence(&self) -> Option<&Value> {
        self.tracker.current_evidence()
    }
    pub(super) fn current_classification(&self) -> Option<storage::InteractionClass> {
        self.current_classification
    }
    fn invalidate_request(&mut self) -> Result<()> {
        self.current_request = None;
        self.current_continuation = None;
        self.transit_request = None;
        self.request_region = None;
        match self.request_generation.checked_add(1) {
            Some(next) if !self.request_generation_failed => {
                self.request_generation = next;
                Ok(())
            }
            _ => {
                self.request_generation_failed = true;
                Err(error("permission request generation exhausted"))
            }
        }
    }
    fn update_request(
        &mut self,
        screen: &ScreenSnapshot,
        observed: &ScreenObservation,
    ) -> Result<()> {
        ensure(
            !self.request_generation_failed,
            "permission request generation exhausted",
        )?;
        self.current_classification = match observed.kind {
            ObservationKind::Interaction(class) => Some(class),
            _ => None,
        };
        let next = (|| -> Result<Option<ObservedRequest>> {
            if observed.kind
                != ObservationKind::Interaction(storage::InteractionClass::PermissionRequest)
                || observed.match_evidence != MatchEvidence::Complete
                || self.eligibility != InputEligibility::Hold(GateReason::Interaction)
            {
                return Ok(None);
            }
            let Some((episode_seq, classification)) = self.tracker.active_episode() else {
                return Ok(None);
            };
            let Some(version) = &self.run.native_version else {
                return Ok(None);
            };
            let Some(profile_version) = &observed.profile_version else {
                return Ok(None);
            };
            let backend = crate::coding_agent::coding_agent(&self.run.backend)
                .expect("validated observer backend");
            let mut answer_matches = Vec::new();
            for decision in [
                storage::PermissionDecision::Allow,
                storage::PermissionDecision::Deny,
            ] {
                if let Some(plan) =
                    self.catalog
                        .answer_plan(backend, version, &observed.rule_id, decision)
                    && let Some(matched) = self.detector.match_answer(screen, &plan)?
                {
                    answer_matches.push(matched);
                }
            }
            let Some(first) = answer_matches.first() else {
                return Ok(None);
            };
            Ok(Some(ObservedRequest {
                request_generation: self.request_generation,
                episode_seq,
                classification,
                native_version: version.clone(),
                detector_profile_version: profile_version.clone(),
                request_rule_id: observed.rule_id.clone(),
                semantic: first.semantic().clone(),
                choices: first.choices().to_vec(),
                focus: first.focus().to_owned(),
                native_correlation: observed.native_correlation.clone(),
                output_seq: first.output_seq(),
                answer_matches,
            }))
        })()?;
        if next.is_none() && self.cursor_transit(screen, observed) {
            if let Some(request) = self.current_request.take() {
                self.transit_request = Some(request);
            }
            return Ok(());
        }
        let resumed = self.transit_request.take();
        let same = match (self.current_request.as_ref().or(resumed.as_ref()), &next) {
            (Some(old), Some(new)) => old.same_request(new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.invalidate_request()?;
            if next.is_some() {
                let region = observed
                    .matched_region
                    .as_ref()
                    .expect("a complete permission match has its live region");
                self.request_region = Some(RequestRegion::of(screen, region.top, region.bottom));
            }
        }
        self.current_request = next.map(|mut request| {
            request.request_generation = self.request_generation;
            request
        });
        self.current_continuation = match (&self.current_request, self.tracker.active_episode()) {
            (None, Some((episode_seq, _))) if self.tracker.continues_active(observed) => {
                Some(ObservedContinuation {
                    request_generation: self.request_generation,
                    episode_seq,
                    rule_id: observed.rule_id.clone(),
                    fingerprint: observed
                        .fingerprint
                        .clone()
                        .expect("a continuation observation has its fingerprint"),
                })
            }
            _ => None,
        };
        Ok(())
    }
    fn cursor_transit(&self, screen: &ScreenSnapshot, observed: &ScreenObservation) -> bool {
        (self.current_request.is_some() || self.transit_request.is_some())
            && !observed.verified_noninteraction
            && !matches!(observed.kind, ObservationKind::Interaction(_))
            && self
                .pending
                .as_ref()
                .is_none_or(|pending| pending.update.transitions.is_empty())
            && self
                .request_region
                .as_ref()
                .is_some_and(|region| region.unchanged(screen))
    }
    fn moment(&mut self, clock: &dyn ObservationClock) -> Result<ObservationMoment> {
        let now = clock.sample();
        ensure(
            self.last_time.is_none_or(|old| now.monotonic >= old),
            "observation clock regressed",
        )?;
        self.last_time = Some(now.monotonic);
        Ok(now)
    }
    fn validate_output(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        ensure(
            self.last_output.is_none_or(|old| screen.screen_seq >= old),
            "observation output sequence regressed",
        )?;
        self.last_output = Some(screen.screen_seq);
        Ok(())
    }
    fn lifecycle(
        &self,
        kind: RuntimeEventKind,
        cause: Value,
        at: &str,
        screen_seq: u64,
    ) -> Result<TransitionDraft> {
        let evidence = storage::evidence::run_metadata(&self.run, "runner-lifecycle", screen_seq)?;
        Ok(TransitionDraft {
            run_id: self.run.run_id,
            episode_seq: None,
            kind,
            classification: None,
            observed_at: at.into(),
            evidence_json: evidence,
            close_reason: None,
            cause_json: Some(cause),
        })
    }
    fn retain(&mut self, update: EpisodeUpdate) -> Result<()> {
        let primary = update
            .transitions
            .iter()
            .filter(|draft| {
                !matches!(
                    draft.kind,
                    RuntimeEventKind::AutomaticRefusalObserved
                        | RuntimeEventKind::AutomaticReviewUnavailable
                )
            })
            .count();
        ensure(
            self.pending.is_none() && primary <= 2,
            "one bounded observation update required",
        )?;
        self.visibility = update.visibility;
        self.eligibility = update.eligibility;
        self.next_deadline = update.next_deadline;
        if !update.transitions.is_empty()
            || update.latest_evidence.is_some()
            || update.effective_policy.is_some()
        {
            self.pending = Some(PendingEpisodeUpdate {
                update,
                next_draft: 0,
                materialized: None,
            });
        }
        Ok(())
    }
    fn update(&self, transitions: Vec<TransitionDraft>) -> EpisodeUpdate {
        EpisodeUpdate {
            transitions,
            ..EpisodeUpdate::new(self.visibility, self.eligibility)
        }
    }
    fn gap(&mut self, reason: ObservationFailure, now: &ObservationMoment) -> Result<()> {
        self.invalidate_request()?;
        self.current_classification = None;
        self.gap_count = self
            .gap_count
            .checked_add(1)
            .ok_or_else(|| error("observation gap count overflow"))?;
        self.diagnostic = Some(reason.code().into());
        self.visibility = Visibility::Unknown;
        self.eligibility = InputEligibility::Hold(GateReason::Gap);
        self.next_deadline = None;
        if self.terminal_fenced {
            self.terminal_observation_loss = self
                .terminal_observation_loss
                .checked_add(1)
                .ok_or_else(|| error("terminal observation loss overflow"))?;
            return Ok(());
        }
        let gap = self.gap.get_or_insert_with(|| PendingGap {
            started_at: now.observed_at.clone(),
            count: 0,
            reason,
        });
        gap.count = gap
            .count
            .checked_add(1)
            .ok_or_else(|| error("gap interval count overflow"))?;
        self.tracker.reset_observation(GateReason::Gap)?;
        Ok(())
    }
    fn delivery_pressure(
        &mut self,
        reason: ObservationFailure,
        now: &ObservationMoment,
    ) -> Result<()> {
        if self.terminal_fenced {
            self.eligibility = InputEligibility::Hold(GateReason::Gap);
            self.next_deadline = None;
            Ok(())
        } else {
            self.gap(reason, now)
        }
    }
    fn route(&mut self, writer: &EventWriter, now: &ObservationMoment) -> Result<()> {
        if self.pool.exhausted()
            && let Some(range) = writer.take_sequence_range()
        {
            self.pool.supply(range)?;
        }
        while let Some(pending) = &mut self.pending {
            if pending.next_draft < pending.update.transitions.len() {
                if pending.materialized.is_none() {
                    if self.pool.exhausted() {
                        writer.request_sequence_range()?;
                        self.delivery_pressure(ObservationFailure::Sequence, now)?;
                        return Ok(());
                    }
                    pending.materialized = Some(
                        self.pool
                            .materialize(&pending.update.transitions[pending.next_draft])?,
                    );
                }
                let value = pending.materialized.take().expect("materialized original");
                let carried = pending
                    .update
                    .effective_policy
                    .as_ref()
                    .filter(|observation| {
                        observation.automatic_review == storage::AutomaticReview::Unavailable
                            && value.kind == RuntimeEventKind::AutomaticReviewUnavailable
                    })
                    .cloned();
                let carries = carried.is_some();
                match writer.try_submit_with_policy(value, carried) {
                    Ok(()) => {
                        pending.next_draft += 1;
                        if carries {
                            pending.update.effective_policy = None;
                        }
                    }
                    Err(retained) => {
                        pending.materialized = Some(retained.transition);
                        self.pending_error = Some(retained.cause);
                        self.delivery_pressure(ObservationFailure::Sink, now)?;
                        return Ok(());
                    }
                }
            } else if let Some(value) = pending.update.latest_evidence.take() {
                match writer.try_refresh(value, Instant::now() + Duration::from_millis(10)) {
                    Ok(()) => {}
                    Err(retained) => {
                        pending.update.latest_evidence = Some(retained.update);
                        self.pending_error = Some(retained.cause);
                        self.delivery_pressure(ObservationFailure::Sink, now)?;
                        return Ok(());
                    }
                }
            } else {
                if let Some(observation) = pending.update.effective_policy.take() {
                    ensure(
                        observation.automatic_review == storage::AutomaticReview::Available,
                        "an unavailable effective policy travels with its event",
                    )?;
                    writer.try_record_policy(observation);
                }
                self.pending = None;
                self.pending_error = None;
            }
        }
        Ok(())
    }
    pub(crate) fn started(
        &mut self,
        run: &RunRecord,
        current: &ScreenSnapshot,
        writer: &EventWriter,
        clock: &dyn ObservationClock,
    ) -> Result<()> {
        ensure(
            !self.started
                && run.owner() == self.run.owner()
                && run.launch_nonce == self.run.launch_nonce
                && run.database_identity_json == self.run.database_identity_json
                && run.backend == self.run.backend
                && run.native_version == self.run.native_version
                && run.profile_version == self.run.profile_version
                && run.status == RunStatus::Running
                && run.child_pid.is_some()
                && run.child_start_identity.is_some()
                && run.started_at.is_some(),
            "started requires matching committed native child",
        )?;
        self.started = true;
        self.run = run.clone();
        let now = self.moment(clock)?;
        if run.coverage == storage::Coverage::Unknown {
            let value = self.lifecycle(
                RuntimeEventKind::CoverageUnknown,
                serde_json::json!({"kind":"coverage_unknown"}),
                &now.observed_at,
                current.screen_seq,
            )?;
            self.retain(self.update(vec![value]))?;
            self.route(writer, &now)?;
        }
        Ok(())
    }
    pub(crate) fn observer<'a>(
        &'a mut self,
        writer: &'a EventWriter,
        clock: &'a dyn ObservationClock,
    ) -> OrderedObservation<'a> {
        self.observer_for(writer, clock, ObservationPhase::ChildOutput)
    }
    pub(crate) fn observer_for<'a>(
        &'a mut self,
        writer: &'a EventWriter,
        clock: &'a dyn ObservationClock,
        phase: ObservationPhase,
    ) -> OrderedObservation<'a> {
        OrderedObservation {
            controller: self,
            writer,
            clock,
            phase,
        }
    }
    fn observation(
        &mut self,
        screen: &ScreenSnapshot,
        now: &ObservationMoment,
        phase: ObservationPhase,
        #[cfg(test)] clock: &dyn ObservationClock,
    ) -> Result<Option<ScreenObservation>> {
        #[cfg(not(test))]
        let _ = phase;
        #[cfg(test)]
        {
            let failed = self
                .observation_hooks
                .as_ref()
                .is_some_and(|hooks| hooks.before(phase, ObservationOperation::Detector).is_err());
            if phase == ObservationPhase::Resize
                && let Some(capture) = &mut self.capture
            {
                capture.observe(screen, CaptureTrigger::Resize, clock)?;
            }
            if failed {
                self.gap(ObservationFailure::Detector, now)?;
                return Ok(None);
            }
        }
        let value = match self.detector.observe(screen) {
            Ok(value) => value,
            Err(_) => {
                self.gap(ObservationFailure::Detector, now)?;
                return Ok(None);
            }
        };
        ensure(
            value.profile_version == self.run.profile_version,
            "observed profile changed within run",
        )?;
        Ok(Some(value))
    }
    fn observe_screen(
        &mut self,
        screen: &ScreenSnapshot,
        writer: &EventWriter,
        clock: &dyn ObservationClock,
        phase: ObservationPhase,
    ) -> Result<()> {
        let now = self.moment(clock)?;
        self.validate_output(screen)?;
        #[cfg(test)]
        if phase != ObservationPhase::Resize
            && let Some(capture) = &mut self.capture
        {
            capture.observe(screen, CaptureTrigger::ChildOutput, clock)?;
        }
        if !self.started || self.exit_recorded.is_some() {
            return Ok(());
        }
        if self.pending.is_some() || self.gap.is_some() || writer.status().degraded {
            self.gap(ObservationFailure::Sink, &now)?;
            return Ok(());
        }
        let Some(observed) = self.observation(
            screen,
            &now,
            phase,
            #[cfg(test)]
            clock,
        )?
        else {
            return Ok(());
        };
        let update = self
            .tracker
            .observe(&observed, now.monotonic, &now.observed_at)?;
        self.retain(update)?;
        self.update_request(screen, &observed)?;
        self.route(writer, &now)
    }
    pub(crate) fn poll(
        &mut self,
        current: &ScreenSnapshot,
        writer: &EventWriter,
        clock: &dyn ObservationClock,
    ) -> Result<ObservationStatus> {
        self.poll_for(current, writer, clock, ObservationPhase::QuietTimer)
    }
    pub(crate) fn poll_for(
        &mut self,
        current: &ScreenSnapshot,
        writer: &EventWriter,
        clock: &dyn ObservationClock,
        phase: ObservationPhase,
    ) -> Result<ObservationStatus> {
        let now = self.moment(clock)?;
        self.validate_output(current)?;
        #[cfg(test)]
        if let Some(capture) = &mut self.capture {
            capture.observe(current, CaptureTrigger::Timer, clock)?;
        }
        if !self.terminal_fenced && writer.status().degraded && self.gap.is_none() {
            self.gap(ObservationFailure::Sink, &now)?;
        }
        self.route(writer, &now)?;
        if self.pending.is_some() {
            return Ok(self.status());
        }
        if self.gap.is_some() && !writer.status().degraded {
            let gap = self.gap.take().expect("gap present");
            let cause = |kind: RuntimeCauseKind| serde_json::json!({"kind":kind.as_str(),"gap_count":gap.count,"gap_started_at":gap.started_at,"gap_ended_at":if kind==RuntimeCauseKind::StorageRecovered{Some(&now.observed_at)}else{None}});
            let first = self.lifecycle(
                RuntimeEventKind::RunnerDegraded,
                cause(
                    if matches!(
                        gap.reason,
                        ObservationFailure::Parser | ObservationFailure::Detector
                    ) {
                        RuntimeCauseKind::ObserverError
                    } else {
                        RuntimeCauseKind::StoragePressure
                    },
                ),
                &gap.started_at,
                current.screen_seq,
            )?;
            let last = self.lifecycle(
                RuntimeEventKind::RunnerDegraded,
                cause(RuntimeCauseKind::StorageRecovered),
                &now.observed_at,
                current.screen_seq,
            )?;
            self.retain(self.update(vec![first, last]))?;
            self.route(writer, &now)?;
            if self.pending.is_none() {
                self.diagnostic = None;
                self.next_deadline = Some(now.monotonic);
            }
        }
        if self.pending.is_none()
            && self.gap.is_none()
            && !writer.status().degraded
            && let Some(exit) = self.exit.take()
        {
            self.terminal_fenced = true;
            let mut update = self
                .tracker
                .finish(InteractionCloseReason::ProcessExited, &exit.at)?;
            let cause = match exit.exit {
                ChildExit::Code(code) => {
                    serde_json::json!({"kind":RuntimeCauseKind::NativeExit.as_str(),"exit_code":code,"exit_signal":null})
                }
                ChildExit::Signal(signal) => {
                    serde_json::json!({"kind":RuntimeCauseKind::NativeExit.as_str(),"exit_code":null,"exit_signal":signal})
                }
            };
            update.transitions.push(self.lifecycle(
                RuntimeEventKind::RunnerExited,
                cause,
                &exit.at,
                exit.screen_seq,
            )?);
            self.retain(update)?;
            self.route(writer, &now)?;
        }
        if self.started
            && self.exit_recorded.is_none()
            && self.pending.is_none()
            && self.gap.is_none()
            && !writer.status().degraded
            && self.next_deadline.is_some_and(|at| now.monotonic >= at)
        {
            let Some(value) = self.observation(
                current,
                &now,
                phase,
                #[cfg(test)]
                clock,
            )?
            else {
                return Ok(self.status());
            };
            let update = self
                .tracker
                .observe(&value, now.monotonic, &now.observed_at)?;
            self.retain(update)?;
            self.update_request(current, &value)?;
            self.route(writer, &now)?;
        }
        Ok(self.status())
    }
    pub(crate) fn child_exited(
        &mut self,
        exit: ChildExit,
        current: &ScreenSnapshot,
        writer: &EventWriter,
        clock: &dyn ObservationClock,
    ) -> Result<()> {
        if let Some(old) = self.exit_recorded {
            ensure(old == exit, "native exit identity changed")?;
        } else {
            let now = self.moment(clock)?;
            self.invalidate_request()?;
            self.exit_recorded = Some(exit);
            self.exit = Some(PendingExit {
                exit,
                at: now.observed_at,
                screen_seq: current.screen_seq,
            });
        }
        #[cfg(test)]
        if let Some(capture) = &mut self.capture {
            capture.cancel(NativeCaptureStatus::OwnerExit)?;
        }
        self.poll(current, writer, clock).map(|_| ())
    }
    pub(crate) fn observer_failed(
        &mut self,
        reason: ObservationFailure,
        clock: &dyn ObservationClock,
    ) -> Result<()> {
        let now = self.moment(clock)?;
        self.gap(reason, &now)
    }
    pub(crate) fn pending_error(&self) -> Option<&CafleetError> {
        self.pending_error.as_ref()
    }
    pub(crate) fn pending_transition_count(&self) -> u64 {
        self.pending
            .as_ref()
            .map_or(0, |p| p.update.transitions.len() - p.next_draft) as u64
    }
    pub(crate) fn unsaved_gap_count(&self) -> u64 {
        self.gap
            .as_ref()
            .map_or(0, |gap| gap.count)
            .checked_add(self.terminal_observation_loss)
            .expect("unsaved observation count overflow")
    }
    pub(crate) fn has_pending(&self) -> bool {
        let status = self.status();
        status.retained_transition
            || status.retained_draft
            || status.retained_evidence
            || self.exit.is_some()
            || self.gap.is_some()
    }
    pub(crate) fn status(&self) -> ObservationStatus {
        ObservationStatus {
            visibility: self.visibility,
            eligibility: self.eligibility,
            next_deadline: self.next_deadline,
            retained_transition: self
                .pending
                .as_ref()
                .is_some_and(|p| p.materialized.is_some()),
            retained_draft: self
                .pending
                .as_ref()
                .is_some_and(|p| p.next_draft < p.update.transitions.len()),
            retained_evidence: self
                .pending
                .as_ref()
                .is_some_and(|p| p.update.latest_evidence.is_some()),
            gap_count: self.gap_count,
            diagnostic: self.diagnostic.clone(),
        }
    }
    pub(crate) fn health(&self, writer: &EventWriter) -> ObservationHealth {
        let observer = self.status();
        let state = writer.status();
        let pending = self.pending_transition_count();
        let evidence = state.pending_evidence || observer.retained_evidence;
        let degraded = state.degraded || self.gap.is_some() || observer.diagnostic.is_some();
        let mut eligibility = observer.eligibility;
        if degraded || pending + state.pending_count > 0 || evidence {
            eligibility = InputEligibility::Hold(GateReason::Gap);
        }
        ObservationHealth {
            visibility: observer.visibility,
            eligibility,
            pending_count: pending + state.pending_count,
            pending_evidence: evidence,
            gap_count: observer
                .gap_count
                .checked_add(state.gap_count)
                .expect("health gap count overflow"),
            degraded,
            diagnostic: if state.degraded {
                Some("sink_unavailable".into())
            } else {
                observer.diagnostic
            },
        }
    }
    #[cfg(test)]
    pub(crate) fn set_observation_hooks(
        &mut self,
        hooks: std::sync::Arc<dyn ObservationHooks>,
    ) -> Result<()> {
        ensure(
            !self.started && self.observation_hooks.is_none(),
            "observation hooks install once before start",
        )?;
        self.observation_hooks = Some(hooks);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn begin_capture_shutdown(&mut self, until: Instant) {
        if let Some(capture) = &mut self.capture {
            capture.begin_shutdown(until);
        }
    }
    #[cfg(test)]
    pub(crate) fn attach_capture(&mut self, port: NativeCapturePort) -> Result<()> {
        ensure(self.capture.is_none(), "capture port already attached")?;
        self.capture = Some(port);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) use native_capture::*;

#[cfg(test)]
pub(crate) mod native_capture {
    use super::*;
    use crate::broker::records::Placement;
    use crate::broker::runtime::{OwnerKind, RunRecord, RunStatus};
    use crate::coding_agent::ScreenRegion;
    use crate::coding_agent::{Detector, ProfileCatalog};
    pub(crate) use exporter::tests::runner_collector_export_child_path;
    use exporter::{CaptureExporterTask, InertCaptureExportHooks};
    pub(crate) use exporter::{NativeCaptureExportBoundary, NativeCaptureExportHooks};
    pub(crate) use files::*;
    use regex::Regex;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    pub(crate) use verification::tests::runner_collector_resize_child_path;
    pub(crate) use verification::{
        native_capture_service_child_path, run_native_capture_verification_with_export_hooks,
    };

    #[derive(Clone, Debug, Serialize)]
    pub(crate) struct NativeCaptureLimits {
        pub(crate) max_cells: usize,
        pub(crate) max_source_bytes: usize,
        pub(crate) max_title_bytes: usize,
        pub(crate) max_patterns: usize,
        pub(crate) max_pattern_bytes: usize,
        pub(crate) max_total_pattern_bytes: usize,
        pub(crate) max_compiled_regex_bytes: usize,
        pub(crate) max_pattern_input_bytes: usize,
        pub(crate) max_scratch_bytes: usize,
        pub(crate) max_projected_heap_bytes: usize,
        pub(crate) max_encoded_sample_bytes: usize,
        pub(crate) max_encoded_result_bytes: usize,
        pub(crate) max_arm_bytes: usize,
        pub(crate) max_config_bytes: usize,
        pub(crate) max_producer_bytes: usize,
        pub(crate) max_diagnostic_bytes: usize,
        pub(crate) max_interval: Duration,
        pub(crate) max_request_duration: Duration,
    }
    impl Default for NativeCaptureLimits {
        fn default() -> Self {
            Self {
                max_cells: 32768,
                max_source_bytes: 1048576,
                max_title_bytes: 16384,
                max_patterns: 64,
                max_pattern_bytes: 4096,
                max_total_pattern_bytes: 65536,
                max_compiled_regex_bytes: 262144,
                max_pattern_input_bytes: 67108864,
                max_scratch_bytes: 2097152,
                max_projected_heap_bytes: 8388608,
                max_encoded_sample_bytes: 2097152,
                max_encoded_result_bytes: 4194304,
                max_arm_bytes: 16384,
                max_config_bytes: 65536,
                max_producer_bytes: 65536,
                max_diagnostic_bytes: 128,
                max_interval: Duration::from_secs(5),
                max_request_duration: Duration::from_secs(30),
            }
        }
    }
    impl NativeCaptureLimits {
        pub(crate) fn validate(&self) -> Result<()> {
            let defaults = serde_json::to_value(Self::default()).map_err(io_error)?;
            let current = serde_json::to_value(self).map_err(io_error)?;
            for (key, value) in current.as_object().expect("serialized limits") {
                if let Some(value) = value.as_u64() {
                    ensure(
                        value > 0 && value <= defaults[key].as_u64().expect("integer limit"),
                        "capture limit outside approved bounds",
                    )?;
                }
            }
            ensure(
                self.max_interval >= Duration::from_millis(100)
                    && self.max_interval <= Duration::from_secs(5)
                    && self.max_request_duration > Duration::from_millis(100)
                    && self.max_request_duration <= Duration::from_secs(30),
                "capture duration outside bounds",
            )
        }
    }
    pub(crate) const NATIVE_CAPTURE_SAMPLE_COUNT: usize = 2;
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum CaptureScreen {
        Primary,
        Alternate,
        Either,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum CaptureMaskReason {
        ReviewedPrivacy,
    }
    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct CaptureMask {
        pub(crate) screen: CaptureScreen,
        pub(crate) rows: u16,
        pub(crate) cols: u16,
        pub(crate) region: ScreenRegion,
        pub(crate) reason: CaptureMaskReason,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum CaptureTrigger {
        ChildOutput,
        Resize,
        Timer,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum NativeCaptureStatus {
        Complete,
        Deadline,
        StaleIdentity,
        Capacity,
        OwnerExit,
        Cancelled,
    }
    pub(crate) trait NativeCapturePatternCompiler: Send + Sync {
        fn compile(&self, pattern: &str, max_compiled_bytes: usize) -> Result<Regex>;
    }
    pub(crate) struct BoundedCapturePatternCompiler;
    impl NativeCapturePatternCompiler for BoundedCapturePatternCompiler {
        fn compile(&self, pattern: &str, max_compiled_bytes: usize) -> Result<Regex> {
            ensure(
                max_compiled_bytes > 0
                    && max_compiled_bytes
                        <= NativeCaptureLimits::default().max_compiled_regex_bytes,
                "capture compiled pattern limit outside bounds",
            )?;
            regex::RegexBuilder::new(pattern)
                .size_limit(max_compiled_bytes)
                .build()
                .map_err(|_| error("invalid bounded capture pattern"))
        }
    }
    pub(crate) struct NativeCapturePolicy {
        metadata: bool,
        patterns: Vec<Regex>,
        sensitive: Vec<String>,
        masks: Vec<CaptureMask>,
        catalog: ProfileCatalog,
        definition: Value,
    }
    impl NativeCapturePolicy {
        fn apply_limits(&mut self, limits: &NativeCaptureLimits) -> Result<()> {
            ensure(
                self.patterns.len() <= limits.max_patterns,
                "capture pattern count exceeds selected limit",
            )?;
            let mut total = 0usize;
            for pattern in &self.patterns {
                total = capture_size(&[total, pattern.as_str().len()])?;
                ensure(
                    pattern.as_str().len() <= limits.max_pattern_bytes
                        && total <= limits.max_total_pattern_bytes,
                    "capture pattern exceeds selected limit",
                )?;
            }
            for pattern in &mut self.patterns {
                *pattern = BoundedCapturePatternCompiler
                    .compile(pattern.as_str(), limits.max_compiled_regex_bytes)?;
            }
            Ok(())
        }
        pub(crate) fn from_launch(
            spawn: &SpawnSpec,
            masks: &[CaptureMask],
            catalog: &ProfileCatalog,
        ) -> Result<Self> {
            Self::from_launch_with_compiler(spawn, masks, catalog, &BoundedCapturePatternCompiler)
        }
        pub(crate) fn from_launch_with_compiler(
            spawn: &SpawnSpec,
            masks: &[CaptureMask],
            catalog: &ProfileCatalog,
            compiler: &dyn NativeCapturePatternCompiler,
        ) -> Result<Self> {
            let defaults = NativeCaptureLimits::default();
            let metadata = match spawn.evidence_policy["mode"].as_str() {
                Some("metadata") => true,
                Some("redacted") => false,
                _ => return Err(error("invalid capture launch evidence policy")),
            };
            let mut raw_patterns = Vec::new();
            let mut sensitive = Vec::new();
            if !metadata {
                let config = spawn.evidence_policy["config"]
                    .as_object()
                    .ok_or_else(|| error("capture redaction configuration required"))?;
                ensure(
                    config.get("schema_version") == Some(&json!(1)),
                    "capture redaction schema mismatch",
                )?;
                let raw = config
                    .get("patterns")
                    .and_then(Value::as_array)
                    .ok_or_else(|| error("capture patterns required"))?;
                ensure(
                    !raw.is_empty() && raw.len() <= defaults.max_patterns,
                    "capture pattern count outside bounds",
                )?;
                let mut total = 0usize;
                for pattern in raw {
                    let pattern = pattern
                        .as_str()
                        .ok_or_else(|| error("capture pattern string required"))?;
                    total = capture_size(&[total, pattern.len()])?;
                    ensure(
                        pattern.len() <= defaults.max_pattern_bytes
                            && total <= defaults.max_total_pattern_bytes,
                        "capture pattern budget exceeded",
                    )?;
                    raw_patterns.push(pattern);
                }
                let known = catalog.rule_ids();
                let rules = config
                    .get("sensitive_rules")
                    .and_then(Value::as_array)
                    .ok_or_else(|| error("capture sensitive rules required"))?;
                for rule in rules {
                    let rule = rule
                        .as_str()
                        .ok_or_else(|| error("capture sensitive rule string required"))?;
                    ensure(
                        known.iter().any(|known| known == rule),
                        "unknown capture sensitive rule",
                    )?;
                    sensitive.push(rule.to_owned());
                }
            }
            for mask in masks {
                let r = mask.region;
                ensure(
                    mask.rows > 0
                        && mask.cols > 0
                        && r.top < r.bottom
                        && r.bottom <= mask.rows
                        && r.left < r.right
                        && r.right <= mask.cols,
                    "invalid capture mask geometry",
                )?;
            }
            let patterns = raw_patterns
                .into_iter()
                .map(|pattern| compiler.compile(pattern, defaults.max_compiled_regex_bytes))
                .collect::<Result<Vec<_>>>()?;
            Ok(Self {
                metadata,
                patterns,
                sensitive,
                masks: masks.to_vec(),
                catalog: catalog.clone(),
                definition: spawn.evidence_policy.clone(),
            })
        }
    }
    #[derive(Clone)]
    pub(crate) struct NativeCaptureRequest {
        pub(crate) capture_id: String,
        pub(crate) run_id: i64,
        pub(crate) owner_epoch: i64,
        pub(crate) minimum_interval: Duration,
        pub(crate) deadline: Instant,
    }
    pub(crate) fn validate_capture_id(id: &str) -> Result<()> {
        ensure(
            !id.trim().is_empty()
                && id.len() <= 128
                && id.is_ascii()
                && !id.chars().any(|c| c.is_control() || c == '/' || c == '\\'),
            "invalid capture identity",
        )
    }
    struct CaptureState {
        identity: Option<ControlIdentity>,
        clock_kind: ObservationClockKind,
        now: Instant,
        used: bool,
        closed: bool,
        shutdown_until: Option<Instant>,
        request: Option<NativeCaptureRequest>,
        result: Option<NativeCaptureResult>,
    }
    impl CaptureState {
        fn clip(&self, until: Instant) -> Instant {
            self.shutdown_until.map_or(until, |end| end.min(until))
        }
    }
    pub(crate) struct NativeCapturePort {
        seed: RunRecord,
        service_producer: Option<NativeCaptureProducer>,
        export_start: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
        prepared: Option<(RunRecord, Placement)>,
        policy: NativeCapturePolicy,
        limits: NativeCaptureLimits,
        state: Arc<Mutex<CaptureState>>,
        origin: Option<ObservationMoment>,
        source: Option<Value>,
        producer_ref: Option<String>,
        detector: Option<Detector>,
        samples: Vec<NativeCaptureSample>,
        projected_bytes: usize,
        next_sample: Option<Instant>,
    }
    pub(crate) struct NativeCaptureReceiver {
        state: Arc<Mutex<CaptureState>>,
        limits: NativeCaptureLimits,
    }
    pub(crate) struct NativeCaptureLease {
        result: NativeCaptureResult,
    }
    impl NativeCaptureLease {
        pub(crate) fn result(&self) -> &NativeCaptureResult {
            &self.result
        }
    }
    #[derive(Serialize)]
    pub(crate) struct NativeCaptureResult {
        schema_version: u32,
        capture_id: String,
        source: Value,
        policy: Value,
        limits: NativeCaptureLimits,
        status: NativeCaptureStatus,
        samples: Vec<NativeCaptureSample>,
        diagnostic: Option<String>,
        #[serde(skip)]
        producer_ref: String,
    }
    #[derive(Serialize)]
    struct NativeCaptureSample {
        sample_index: usize,
        trigger: CaptureTrigger,
        monotonic_start_ns: u64,
        monotonic_end_ns: u64,
        observed_at: String,
        screen_seq: u64,
        rows: u16,
        cols: u16,
        cursor_row: u16,
        cursor_col: u16,
        alternate_screen: bool,
        title: Option<String>,
        projection: &'static str,
        unavailable_regions: Vec<UnavailableRegion>,
        grid: Option<Vec<ProjectedRow>>,
    }
    #[derive(Serialize)]
    struct ProjectedRow {
        row: u16,
        row_wrapped: Option<bool>,
        unavailability_reason: Option<&'static str>,
        cells: Vec<ProjectedCell>,
    }
    #[derive(Serialize)]
    struct UnavailableRegion {
        top: u16,
        left: u16,
        bottom: u16,
        right: u16,
        reason: &'static str,
    }
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum ProjectedColor {
        Default,
        Indexed { index: u8 },
        Rgb { r: u8, g: u8, b: u8 },
    }
    #[derive(Serialize)]
    struct ProjectedCell {
        availability: &'static str,
        contents: Option<String>,
        has_contents: Option<bool>,
        wide: Option<bool>,
        wide_continuation: Option<bool>,
        fg: Option<ProjectedColor>,
        bg: Option<ProjectedColor>,
        bold: Option<bool>,
        dim: Option<bool>,
        italic: Option<bool>,
        underline: Option<bool>,
        inverse: Option<bool>,
    }
    fn same_capture_run(left: &RunRecord, right: &RunRecord) -> bool {
        left.run_id == right.run_id
            && left.database_identity_json == right.database_identity_json
            && left.launch_nonce == right.launch_nonce
            && left.member_id == right.member_id
            && left.fleet_id == right.fleet_id
            && left.director_member_id == right.director_member_id
            && left.backend == right.backend
            && left.native_version == right.native_version
            && left.profile_version == right.profile_version
            && left.requested_mode == right.requested_mode
            && left.resolved_mode == right.resolved_mode
            && left.mode_source == right.mode_source
            && left.permission_argv_json == right.permission_argv_json
            && left.event_evidence == right.event_evidence
    }
    pub(crate) fn native_capture_channel(
        run: &RunRecord,
        mut policy: NativeCapturePolicy,
        limits: NativeCaptureLimits,
    ) -> Result<(NativeCapturePort, NativeCaptureReceiver)> {
        limits.validate()?;
        policy.apply_limits(&limits)?;
        ensure(
            run.run_id > 0
                && run.owner_epoch > 0
                && matches!(run.status, RunStatus::Starting | RunStatus::Prepared)
                && run.child_pid.is_none()
                && run.child_start_identity.is_none()
                && run.started_at.is_none()
                && run.ended_at.is_none()
                && run.recovery_origin_status.is_none(),
            "capture seed requires fresh startup state",
        )?;
        let state = Arc::new(Mutex::new(CaptureState {
            identity: None,
            clock_kind: ObservationClockKind::Deterministic,
            now: Instant::now(),
            used: false,
            closed: false,
            shutdown_until: None,
            request: None,
            result: None,
        }));
        Ok((
            NativeCapturePort {
                seed: run.clone(),
                service_producer: None,
                export_start: None,
                prepared: None,
                policy,
                limits: limits.clone(),
                state: state.clone(),
                origin: None,
                source: None,
                producer_ref: None,
                detector: None,
                samples: Vec::with_capacity(NATIVE_CAPTURE_SAMPLE_COUNT),
                projected_bytes: 0,
                next_sample: None,
            },
            NativeCaptureReceiver { state, limits },
        ))
    }
    impl NativeCaptureReceiver {
        pub(crate) fn finalized_identity(&self) -> Option<ControlIdentity> {
            self.state.lock().expect("capture mailbox").identity.clone()
        }
        pub(crate) fn request(&self, request: NativeCaptureRequest) -> Result<()> {
            let mut state = self.state.lock().expect("capture mailbox");
            let identity = state
                .identity
                .as_ref()
                .ok_or_else(|| error("capture source is not finalized"))?;
            ensure(
                !state.closed && !state.used,
                "capture channel is closed or already used",
            )?;
            validate_capture_id(&request.capture_id)?;
            ensure(
                request.run_id == identity.run_id && request.owner_epoch == identity.owner_epoch,
                "capture owner mismatch",
            )?;
            let now = if state.clock_kind == ObservationClockKind::Real {
                Instant::now()
            } else {
                state.now
            };
            ensure(
                request.minimum_interval >= Duration::from_millis(100)
                    && request.minimum_interval <= self.limits.max_interval
                    && request.deadline > now
                    && request.deadline.duration_since(now) <= self.limits.max_request_duration,
                "invalid capture interval/deadline",
            )?;
            if let Some(until) = state.shutdown_until {
                ensure(
                    request.deadline <= until,
                    "capture request exceeds shutdown deadline",
                )?;
            }
            state.used = true;
            state.request = Some(request);
            Ok(())
        }
        pub(crate) fn try_take(&self) -> Result<Option<NativeCaptureLease>> {
            let result = self.state.lock().expect("capture mailbox").result.take();
            let Some(mut result) = result else {
                return Ok(None);
            };
            if let Some(index) = result.samples.iter().position(|sample| {
                encoded_size(sample, self.limits.max_encoded_sample_bytes).is_err()
            }) {
                result.samples.truncate(index);
                result.status = NativeCaptureStatus::Capacity;
                result.diagnostic = Some("capacity".into());
            }
            while !result.samples.is_empty()
                && encoded_size(&result, self.limits.max_encoded_result_bytes).is_err()
            {
                result.samples.pop();
                result.status = NativeCaptureStatus::Capacity;
                result.diagnostic = Some("capacity".into());
            }
            if encoded_size(&result, self.limits.max_encoded_result_bytes).is_err() {
                result.status = NativeCaptureStatus::Capacity;
                result.diagnostic = Some("capacity".into());
            }
            Ok(Some(NativeCaptureLease { result }))
        }
        pub(super) fn publication_deadline(&self, until: Instant) -> Instant {
            self.state.lock().expect("capture mailbox").clip(until)
        }
    }
    impl NativeCapturePort {
        pub(super) fn begin_shutdown(&mut self, until: Instant) {
            let mut state = self.state.lock().expect("capture mailbox");
            state.closed = true;
            let until = state.clip(until);
            state.shutdown_until = Some(until);
            if let Some(request) = &mut state.request {
                request.deadline = request.deadline.min(until);
            }
        }
        pub(crate) fn bind_service_prepared(
            &mut self,
            run: &RunRecord,
            placement: &Placement,
            spawn: &SpawnSpec,
            clock: &dyn ObservationClock,
        ) -> Result<()> {
            let producer = self
                .service_producer
                .take()
                .ok_or_else(|| error("capture service producer configuration required"))?;
            self.bind_prepared(run, placement, spawn, producer, clock)
        }
        pub(crate) fn bind_prepared(
            &mut self,
            run: &RunRecord,
            placement: &Placement,
            spawn: &SpawnSpec,
            producer: NativeCaptureProducer,
            clock: &dyn ObservationClock,
        ) -> Result<()> {
            ensure(
                self.prepared.is_none()
                    && same_capture_run(&self.seed, run)
                    && run.status == RunStatus::Prepared
                    && run.owner_kind == OwnerKind::Runner
                    && run.child_pid.is_none()
                    && run.child_start_identity.is_none()
                    && run.started_at.is_none()
                    && run.ended_at.is_none()
                    && run.recovery_origin_status.is_none(),
                "capture requires validated prepared source",
            )?;
            let epoch = if self.seed.owner_kind == OwnerKind::Creator {
                self.seed
                    .owner_epoch
                    .checked_add(1)
                    .ok_or_else(|| error("capture epoch overflow"))?
            } else {
                self.seed.owner_epoch
            };
            ensure(
                run.owner_epoch == epoch
                    && (self.seed.owner_kind == OwnerKind::Creator
                        || self.seed.owner() == run.owner()),
                "capture prepared owner mismatch",
            )?;
            ensure(
                placement.mux_pane_id.is_some() && spawn.evidence_policy == self.policy.definition,
                "capture prepared placement/policy missing",
            )?;
            let native = &producer.value["native_executable"];
            let path = std::path::Path::new(&spawn.argv[0]);
            ensure(
                path.is_absolute()
                    && native["path"].as_str() == path.to_str()
                    && native["sha256"].as_str() == Some(file_digest(path)?.as_str())
                    && native["version"] == json!(run.native_version),
                "capture actual native executable mismatch",
            )?;
            let backend = crate::coding_agent::coding_agent(&run.backend)
                .ok_or_else(|| error("capture backend unavailable"))?;
            let selection = backend
                .resolve_policy(run.native_version.as_deref(), run.requested_mode.as_deref())?;
            ensure(
                selection.resolved_mode == run.resolved_mode
                    && selection.mode_source == run.mode_source
                    && json!(selection.permission_argv) == run.permission_argv_json,
                "capture recorded permission policy mismatch",
            )?;
            backend.validate_spawn_policy_argv(&spawn.argv, &selection)?;
            ensure(
                producer.value["catalog_origin"] == self.policy.catalog.capture_origin(),
                "capture selected catalog origin mismatch",
            )?;
            self.detector = Some(Detector::new(
                &self.policy.catalog,
                backend,
                run.native_version.as_deref(),
            ));
            let origin = clock.sample();
            {
                let mut state = self.state.lock().expect("capture mailbox");
                state.now = origin.monotonic;
                state.clock_kind = clock.kind();
            }
            self.source = Some(
                json!({"database_identity_json":run.database_identity_json,"run_id":run.run_id,"owner_epoch":run.owner_epoch,"launch_nonce":run.launch_nonce,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,"runner":{"pid":run.owner_pid,"start_identity":run.owner_start_identity},"child":null,"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"requested_mode":run.requested_mode,"resolved_mode":run.resolved_mode,"mode_source":run.mode_source,"permission_argv":run.permission_argv_json,"executable":native,"cwd":spawn.cwd,"environment_evidence_ref":producer.value["environment_evidence_ref"],"placement":crate::presentation::placement(placement),"producer":producer.value,"clock":{"kind":if clock.kind()==ObservationClockKind::Real{"real"}else{"deterministic"},"monotonic_api":"std::time::Instant","utc_api":"chrono::Utc::now","origin":{"phase":"prepared","observed_at":origin.observed_at}}}),
            );
            self.producer_ref = Some(producer.reference.to_string_lossy().into_owned());
            self.origin = Some(origin);
            self.prepared = Some((run.clone(), placement.clone()));
            Ok(())
        }
        pub(crate) fn finalize_source(
            &mut self,
            run: &RunRecord,
            placement: &Placement,
            child: &ProcessIdentity,
        ) -> Result<()> {
            let (previous, expected) = self
                .prepared
                .as_ref()
                .ok_or_else(|| error("capture source is not prepared"))?;
            let mut state = self.state.lock().expect("capture mailbox");
            ensure(
                state.identity.is_none()
                    && same_capture_run(previous, run)
                    && previous.owner() == run.owner()
                    && expected == placement
                    && run.status == RunStatus::Running
                    && run.started_at.is_some()
                    && run.child_pid == Some(child.pid)
                    && run.child_start_identity.as_deref() == Some(child.start_identity.as_str())
                    && child.matches()?,
                "capture actual running source mismatch",
            )?;
            self.source.as_mut().expect("prepared source")["child"] =
                serde_json::to_value(child).map_err(io_error)?;
            state.identity = Some(ControlIdentity::from_run(run));
            drop(state);
            if let Some(start) = self.export_start.take() {
                start()?;
            }
            Ok(())
        }
        fn complete(&mut self, request: &NativeCaptureRequest, status: NativeCaptureStatus) {
            let result = NativeCaptureResult {
                producer_ref: self
                    .producer_ref
                    .as_ref()
                    .expect("prepared producer reference")
                    .clone(),
                schema_version: 1,
                capture_id: request.capture_id.clone(),
                source: self.source.as_ref().expect("finalized source").clone(),
                policy: json!({"mode":if self.policy.metadata{"metadata"}else{"redacted"},"policy_id":"launch_event_evidence","masks":self.policy.masks}),
                limits: self.limits.clone(),
                status,
                samples: std::mem::take(&mut self.samples),
                diagnostic: if status == NativeCaptureStatus::Complete {
                    None
                } else {
                    Some(
                        match status {
                            NativeCaptureStatus::Deadline => "deadline",
                            NativeCaptureStatus::Capacity => "capacity",
                            NativeCaptureStatus::OwnerExit => "owner_exit",
                            NativeCaptureStatus::StaleIdentity => "stale_identity",
                            _ => "cancelled",
                        }
                        .into(),
                    )
                },
            };
            let mut state = self.state.lock().expect("capture mailbox");
            state.request = None;
            state.result = Some(result);
            self.next_sample = None;
        }
        pub(crate) fn next_deadline(&self) -> Option<Instant> {
            let state = self.state.lock().expect("capture mailbox");
            state.request.as_ref().map(|request| {
                self.next_sample
                    .map_or(request.deadline, |at| at.min(request.deadline))
            })
        }
        pub(crate) fn cancel(&mut self, reason: NativeCaptureStatus) -> Result<()> {
            ensure(
                matches!(
                    reason,
                    NativeCaptureStatus::OwnerExit | NativeCaptureStatus::Cancelled
                ),
                "invalid external capture cancellation",
            )?;
            let request = {
                let mut state = self.state.lock().expect("capture mailbox");
                state.closed = true;
                state.request.clone()
            };
            if let Some(request) = request {
                self.complete(&request, reason);
            }
            Ok(())
        }
        pub(crate) fn observe(
            &mut self,
            screen: &ScreenSnapshot,
            trigger: CaptureTrigger,
            clock: &dyn ObservationClock,
        ) -> Result<()> {
            let request = self.state.lock().expect("capture mailbox").request.clone();
            let Some(request) = request else {
                return Ok(());
            };
            let start = clock.sample();
            {
                let mut state = self.state.lock().expect("capture mailbox");
                ensure(start.monotonic >= state.now, "capture clock regressed")?;
                state.now = start.monotonic;
            }
            if start.monotonic >= request.deadline {
                self.complete(&request, NativeCaptureStatus::Deadline);
                return Ok(());
            }
            if self.next_sample.is_some_and(|next| start.monotonic < next) {
                return Ok(());
            }
            let sample = match self.project(screen, trigger, &start, clock, request.deadline) {
                Ok(sample) => sample,
                Err(CafleetError::DeadlineExceeded) => {
                    self.complete(&request, NativeCaptureStatus::Deadline);
                    return Ok(());
                }
                Err(cause) => return Err(cause),
            };
            let Some((sample, projected)) = sample else {
                self.complete(&request, NativeCaptureStatus::Capacity);
                return Ok(());
            };
            let end = clock.sample();
            ensure(
                end.monotonic >= start.monotonic,
                "capture end clock regressed",
            )?;
            if end.monotonic >= request.deadline {
                self.complete(&request, NativeCaptureStatus::Deadline);
                return Ok(());
            }
            self.next_sample = Some(
                end.monotonic
                    .checked_add(request.minimum_interval)
                    .ok_or_else(|| error("capture interval overflow"))?,
            );
            self.samples.push(sample);
            self.projected_bytes = projected;
            if self.samples.len() == NATIVE_CAPTURE_SAMPLE_COUNT {
                self.complete(&request, NativeCaptureStatus::Complete);
            }
            Ok(())
        }
        fn project(
            &self,
            screen: &ScreenSnapshot,
            trigger: CaptureTrigger,
            start: &ObservationMoment,
            clock: &dyn ObservationClock,
            deadline: Instant,
        ) -> Result<Option<(NativeCaptureSample, usize)>> {
            let limits = &self.limits;
            let cells = usize::from(screen.rows)
                .checked_mul(usize::from(screen.cols))
                .ok_or_else(|| error("capture geometry overflow"))?;
            if cells > limits.max_cells || screen.title.len() > limits.max_title_bytes {
                return Ok(None);
            }
            let mut source_bytes = 0usize;
            for row in 0..screen.rows {
                capture_deadline(clock, deadline)?;
                for col in 0..screen.cols {
                    let cell = screen
                        .screen
                        .cell(row, col)
                        .ok_or_else(|| error("missing in-range capture cell"))?;
                    source_bytes = source_bytes
                        .checked_add(if cell.is_wide_continuation() {
                            0
                        } else {
                            cell.contents().len().max(1)
                        })
                        .ok_or_else(|| error("capture text size overflow"))?;
                }
            }
            let rows = usize::from(screen.rows);
            let scratch = capture_size(&[
                source_bytes,
                rows,
                rows.checked_mul(std::mem::size_of::<(usize, usize, u16, u16)>())
                    .ok_or_else(|| error("capture field allocation overflow"))?,
                cells,
            ])?;
            let projected = capture_size(&[
                self.projected_bytes,
                std::mem::size_of::<NativeCaptureSample>(),
                source_bytes,
                screen.title.len(),
                start.observed_at.len(),
                cells
                    .checked_mul(std::mem::size_of::<ProjectedCell>())
                    .ok_or_else(|| error("capture cell allocation overflow"))?,
                rows.checked_mul(
                    std::mem::size_of::<ProjectedRow>() + std::mem::size_of::<UnavailableRegion>(),
                )
                .ok_or_else(|| error("capture row allocation overflow"))?,
            ])?;
            if source_bytes > limits.max_source_bytes
                || scratch > limits.max_scratch_bytes
                || projected > limits.max_projected_heap_bytes
            {
                return Ok(None);
            }
            capture_deadline(clock, deadline)?;
            let observation = self
                .detector
                .as_ref()
                .expect("prepared capture detector")
                .observe(screen)?;
            let sensitive = observation.raw_evidence["sensitive"].as_bool() == Some(true)
                || self.policy.sensitive.contains(&observation.rule_id);
            let layout = self.policy.masks.iter().any(|mask| {
                (mask.screen == CaptureScreen::Either
                    || (mask.screen == CaptureScreen::Alternate) == screen.alternate_screen)
                    && (mask.rows != screen.rows || mask.cols != screen.cols)
            });
            let metadata = self.policy.metadata || sensitive || layout;
            let mut unavailable = Vec::with_capacity(if metadata { 1 } else { rows });
            let mut grid = None;
            if metadata {
                unavailable.push(UnavailableRegion {
                    top: 0,
                    left: 0,
                    bottom: screen.rows,
                    right: screen.cols,
                    reason: if layout {
                        "layout_mismatch"
                    } else if sensitive {
                        "sensitive_form"
                    } else {
                        "metadata_policy"
                    },
                });
            } else {
                let mut text = String::with_capacity(source_bytes + usize::from(screen.rows));
                let mut fields =
                    Vec::<(usize, usize, u16, u16)>::with_capacity(usize::from(screen.rows));
                let mut top = 0;
                let mut beginning = 0;
                for row in 0..screen.rows {
                    capture_deadline(clock, deadline)?;
                    for col in 0..screen.cols {
                        let cell = screen
                            .screen
                            .cell(row, col)
                            .expect("validated capture grid");
                        if !cell.is_wide_continuation() {
                            if cell.has_contents() {
                                text.push_str(cell.contents())
                            } else {
                                text.push(' ')
                            }
                        }
                    }
                    if !screen.screen.row_wrapped(row) {
                        fields.push((beginning, text.len(), top, row + 1));
                        text.push('\n');
                        beginning = text.len();
                        top = row + 1;
                    }
                }
                if top < screen.rows {
                    fields.push((beginning, text.len(), top, screen.rows));
                }
                if text.len() > limits.max_source_bytes
                    || capture_size(&[text.len(), screen.title.len()])?
                        .checked_mul(self.policy.patterns.len())
                        .is_none_or(|size| size > limits.max_pattern_input_bytes)
                {
                    return Ok(None);
                }
                let mut masked = vec![false; cells];
                for pattern in &self.policy.patterns {
                    capture_deadline(clock, deadline)?;
                    for found in pattern.find_iter(&text) {
                        capture_deadline(clock, deadline)?;
                        for &(begin, end, top, bottom) in &fields {
                            if found.start() <= end && found.end() > begin {
                                for row in top..bottom {
                                    capture_deadline(clock, deadline)?;
                                    for col in 0..screen.cols {
                                        masked[usize::from(row) * usize::from(screen.cols)
                                            + usize::from(col)] = true;
                                    }
                                }
                            }
                        }
                    }
                }
                for mask in &self.policy.masks {
                    capture_deadline(clock, deadline)?;
                    if mask.screen == CaptureScreen::Either
                        || (mask.screen == CaptureScreen::Alternate) == screen.alternate_screen
                    {
                        for row in mask.region.top..mask.region.bottom {
                            capture_deadline(clock, deadline)?;
                            for col in mask.region.left..mask.region.right {
                                masked[usize::from(row) * usize::from(screen.cols)
                                    + usize::from(col)] = true;
                            }
                        }
                    }
                }
                for row in 0..screen.rows {
                    capture_deadline(clock, deadline)?;
                    for col in 0..screen.cols {
                        let index = usize::from(row) * usize::from(screen.cols) + usize::from(col);
                        if masked[index] {
                            let cell = screen
                                .screen
                                .cell(row, col)
                                .expect("validated capture cell");
                            if cell.is_wide() && col + 1 < screen.cols {
                                masked[index + 1] = true;
                            }
                            if cell.is_wide_continuation() && col > 0 {
                                masked[index - 1] = true;
                            }
                        }
                    }
                }
                let mut rows = Vec::with_capacity(usize::from(screen.rows));
                for row in 0..screen.rows {
                    capture_deadline(clock, deadline)?;
                    let mut projected = Vec::with_capacity(usize::from(screen.cols));
                    let mut complete = true;
                    for col in 0..screen.cols {
                        let cell = screen
                            .screen
                            .cell(row, col)
                            .expect("validated capture cell");
                        let masked =
                            masked[usize::from(row) * usize::from(screen.cols) + usize::from(col)];
                        complete &= !masked;
                        projected.push(if masked {
                            ProjectedCell {
                                availability: "masked",
                                contents: None,
                                has_contents: None,
                                wide: None,
                                wide_continuation: None,
                                fg: None,
                                bg: None,
                                bold: None,
                                dim: None,
                                italic: None,
                                underline: None,
                                inverse: None,
                            }
                        } else {
                            ProjectedCell {
                                availability: "available",
                                contents: Some(cell.contents().into()),
                                has_contents: Some(cell.has_contents()),
                                wide: Some(cell.is_wide()),
                                wide_continuation: Some(cell.is_wide_continuation()),
                                fg: Some(color(cell.fgcolor())),
                                bg: Some(color(cell.bgcolor())),
                                bold: Some(cell.bold()),
                                dim: Some(cell.dim()),
                                italic: Some(cell.italic()),
                                underline: Some(cell.underline()),
                                inverse: Some(cell.inverse()),
                            }
                        });
                    }
                    if !complete {
                        unavailable.push(UnavailableRegion {
                            top: row,
                            left: 0,
                            bottom: row + 1,
                            right: screen.cols,
                            reason: "masked_cells",
                        });
                    }
                    rows.push(ProjectedRow {
                        row,
                        row_wrapped: if complete {
                            Some(screen.screen.row_wrapped(row))
                        } else {
                            None
                        },
                        unavailability_reason: if complete { None } else { Some("masked_cells") },
                        cells: projected,
                    });
                }
                grid = Some(rows);
            }
            let mut title = None;
            if !metadata {
                let mut masked = false;
                for pattern in &self.policy.patterns {
                    capture_deadline(clock, deadline)?;
                    if pattern.is_match(&screen.title) {
                        masked = true;
                        break;
                    }
                }
                if !masked {
                    title = Some(screen.title.clone());
                }
            }
            let origin = self.origin.as_ref().expect("capture origin");
            let end = clock.sample();
            let elapsed = |at: Instant| -> Result<u64> {
                let duration = at
                    .checked_duration_since(origin.monotonic)
                    .ok_or_else(|| error("capture origin regressed"))?;
                u64::try_from(duration.as_nanos()).map_err(io_error)
            };
            let sample = NativeCaptureSample {
                sample_index: self.samples.len(),
                trigger,
                monotonic_start_ns: elapsed(start.monotonic)?,
                monotonic_end_ns: elapsed(end.monotonic)?,
                observed_at: start.observed_at.clone(),
                screen_seq: screen.screen_seq,
                rows: screen.rows,
                cols: screen.cols,
                cursor_row: screen.cursor_row,
                cursor_col: screen.cursor_col,
                alternate_screen: screen.alternate_screen,
                title,
                projection: if metadata { "metadata" } else { "redacted" },
                unavailable_regions: unavailable,
                grid,
            };
            capture_deadline(clock, deadline)?;
            Ok(Some((sample, projected)))
        }
    }
    fn capture_size(parts: &[usize]) -> Result<usize> {
        parts.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(*value)
                .ok_or_else(|| error("capture allocation overflow"))
        })
    }
    fn capture_deadline(clock: &dyn ObservationClock, until: Instant) -> Result<()> {
        check_deadline(clock.sample().monotonic, until)
    }
    fn color(color: vt100::Color) -> ProjectedColor {
        match color {
            vt100::Color::Default => ProjectedColor::Default,
            vt100::Color::Idx(index) => ProjectedColor::Indexed { index },
            vt100::Color::Rgb(r, g, b) => ProjectedColor::Rgb { r, g, b },
        }
    }
    struct BoundedWriter {
        written: usize,
        limit: usize,
        buffer: Option<Vec<u8>>,
    }
    impl std::io::Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let end = self
                .written
                .checked_add(bytes.len())
                .filter(|end| *end <= self.limit)
                .ok_or_else(|| std::io::Error::other("encoded capture capacity"))?;
            if let Some(buffer) = &mut self.buffer {
                buffer.extend_from_slice(bytes);
            }
            self.written = end;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn bounded_json(
        value: &impl Serialize,
        limit: usize,
        buffer: Option<Vec<u8>>,
    ) -> Result<BoundedWriter> {
        let mut writer = BoundedWriter {
            written: 0,
            limit,
            buffer,
        };
        serde_json::to_writer(&mut writer, value).map_err(io_error)?;
        Ok(writer)
    }
    fn encoded_size(value: &impl Serialize, limit: usize) -> Result<usize> {
        bounded_json(value, limit, None).map(|writer| writer.written)
    }
    fn encode_bounded(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
        let writer = bounded_json(value, limit, Some(Vec::new()))?;
        Ok(writer.buffer.expect("the buffered writer keeps its bytes"))
    }

    pub(crate) mod test_support {
        use serde_json::{Value, json};

        use super::*;
        use crate::broker::records::Placement;
        use crate::broker::runtime as storage;
        use crate::coding_agent::ScreenRegion;
        use crate::digest::sha256_hex;
        use crate::runtime::runner::test_support::*;
        use crate::runtime::system::test_support::TestChild;
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::path::{Path, PathBuf};
        use std::process::{Command, Stdio};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, Instant};

        /// Publishes `bytes` at `path` atomically: waiters poll these marker
        /// paths from other processes, so the name must never be visible
        /// before its contents. `hard_link` keeps the create-new refusal.
        pub(crate) fn private_write(path: &Path, bytes: &[u8]) {
            static STAGED: AtomicU64 = AtomicU64::new(0);
            let staged = path.with_file_name(format!(
                ".{}.partial-{}-{}",
                path.file_name().unwrap().to_string_lossy(),
                std::process::id(),
                STAGED.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&staged)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            std::fs::hard_link(&staged, path).unwrap();
            std::fs::remove_file(&staged).unwrap();
        }
        pub(crate) fn digest(path: &Path) -> String {
            sha256_hex(&std::fs::read(path).unwrap())
        }
        pub(super) use crate::coding_agent::profiles::catalog::test_support::synthetic_catalog as catalog;
        pub(crate) fn redacted() -> Value {
            json!({"mode":"redacted","config":{"schema_version":1,"patterns":["PRIVATE-[A-Z]+"],"sensitive_rules":[]}})
        }
        pub(crate) fn mask(top: u16, left: u16, bottom: u16, right: u16) -> CaptureMask {
            CaptureMask {
                screen: CaptureScreen::Either,
                rows: 24,
                cols: 80,
                region: ScreenRegion {
                    top,
                    left,
                    bottom,
                    right,
                },
                reason: CaptureMaskReason::ReviewedPrivacy,
            }
        }

        pub(crate) struct Clock {
            pub(crate) origin: Instant,
            pub(crate) millis: AtomicU64,
        }
        impl Clock {
            pub(crate) fn new() -> Self {
                Self {
                    origin: Instant::now(),
                    millis: AtomicU64::new(0),
                }
            }
            pub(crate) fn set(&self, millis: u64) {
                self.millis.store(millis, Ordering::Release);
            }
        }
        impl ObservationClock for Clock {
            fn sample(&self) -> ObservationMoment {
                ObservationMoment {
                    monotonic: self.origin
                        + Duration::from_millis(self.millis.load(Ordering::Acquire)),
                    observed_at: "2026-10-26T17:00:00.000000+00:00".into(),
                }
            }
            fn kind(&self) -> ObservationClockKind {
                ObservationClockKind::Deterministic
            }
        }

        pub(crate) struct CaptureFixture {
            pub(crate) fixture: Fixture,
            pub(crate) artifacts: RunArtifacts,
            pub(crate) spec: SpawnSpec,
            pub(crate) placement: Placement,
            pub(crate) producer_path: PathBuf,
            pub(crate) producer_json: Value,
            pub(crate) clock: Clock,
            pub(crate) _lock: RunLock,
            pub(crate) child: Option<TestChild>,
        }
        impl CaptureFixture {
            pub(crate) fn new(policy: Value) -> Self {
                let mut fixture = Fixture::new();
                std::fs::set_permissions(
                    fixture._dir.path(),
                    std::fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                let artifacts = fixture.artifacts();
                let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap();
                held.transfer_to_runner(
                    &mut fixture.conn,
                    &fixture.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
                fixture.conn.execute("UPDATE member_runs SET runner_pid=owner_pid,runner_start_identity=owner_start_identity,native_version='0.0.1-synthetic',profile_version='synthetic-profile-1',coverage='tested',event_evidence=?1 WHERE run_id=?2",rusqlite::params![policy["mode"].as_str().unwrap(),fixture.run.run_id]).unwrap();
                fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                let executable = fixture._dir.path().join("owned-synthetic-process");
                private_write(&executable,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf '0.0.1-synthetic\\n'; exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; *) printf '%s\\n' \"$line\" ;; esac; done\n");
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                let mut spec = fixture.spec();
                spec.evidence_policy = policy;
                spec.argv = vec![
                    executable.to_str().unwrap().into(),
                    "--approve-for-me".into(),
                    "private assignment canary".into(),
                ];
                artifacts
                    .write_descriptor(&LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap())
                    .unwrap();
                let source = fixture._dir.path().join("source-evidence.json");
                private_write(
                    &source,
                    b"{\"kind\":\"synthetic\",\"version\":\"0.0.1-synthetic\"}",
                );
                let current = std::env::current_exe().unwrap();
                let producer_json = json!({"schema_version":1,"kind":"synthetic","source_manifest_ref":source,"source_manifest_sha256":digest(&source),"libtest_executable_path":current,"libtest_executable_sha256":digest(&current),"parser_version":"0.18.1","collector_version":"1","catalog_origin":"synthetic","native_executable":{"path":executable,"sha256":digest(&executable),"version":"0.0.1-synthetic","version_evidence_ref":source},"launch_evidence_ref":source,"environment_evidence_ref":source});
                let producer_path = fixture._dir.path().join("producer.json");
                private_write(&producer_path, &serde_json::to_vec(&producer_json).unwrap());
                let placement = crate::broker::get_member(
                    &fixture.conn,
                    fixture.run.member_id,
                    fixture.run.fleet_id,
                )
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
                Self {
                    fixture,
                    artifacts,
                    spec,
                    placement,
                    producer_path,
                    producer_json,
                    clock: Clock::new(),
                    _lock: held,
                    child: None,
                }
            }
            pub(crate) fn channel(
                &self,
                limits: NativeCaptureLimits,
                masks: &[CaptureMask],
            ) -> (NativeCapturePort, NativeCaptureReceiver) {
                self.channel_with(limits, masks, &catalog())
            }
            pub(crate) fn channel_with(
                &self,
                limits: NativeCaptureLimits,
                masks: &[CaptureMask],
                catalog: &crate::coding_agent::ProfileCatalog,
            ) -> (NativeCapturePort, NativeCaptureReceiver) {
                let policy = NativeCapturePolicy::from_launch(&self.spec, masks, catalog).unwrap();
                native_capture_channel(&self.fixture.run, policy, limits).unwrap()
            }
            pub(crate) fn mark_prepared(&mut self) {
                self.fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                        [self.fixture.run.run_id],
                    )
                    .unwrap();
                self.fixture.run =
                    storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
            }
            pub(crate) fn producer(&self) -> NativeCaptureProducer {
                read_native_capture_producer(&self.producer_path, &NativeCaptureLimits::default())
                    .unwrap()
            }
            pub(crate) fn prepare(&mut self, port: &mut NativeCapturePort) {
                self.mark_prepared();
                let producer = self.producer();
                port.bind_prepared(
                    &self.fixture.run,
                    &self.placement,
                    &self.spec,
                    producer,
                    &self.clock,
                )
                .unwrap();
            }
            pub(crate) fn finalize(&mut self, port: &mut NativeCapturePort) {
                let child = TestChild::new(
                    Command::new(&self.spec.argv[0])
                        .args(&self.spec.argv[1..])
                        .current_dir(&self.spec.cwd)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap(),
                );
                self.child = Some(child);
                let child = self
                    .child
                    .as_ref()
                    .expect("owned child guard before identity validation");
                let identity = ProcessIdentity::read(i64::from(child.id()))
                    .unwrap()
                    .unwrap();
                self.fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at='2026-10-26T17:00:00Z' WHERE run_id=?3",rusqlite::params![identity.pid,identity.start_identity,self.fixture.run.run_id]).unwrap();
                self.fixture.run =
                    storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
                port.finalize_source(&self.fixture.run, &self.placement, &identity)
                    .unwrap();
            }
            pub(crate) fn ready(
                &mut self,
                limits: NativeCaptureLimits,
                masks: &[CaptureMask],
            ) -> (NativeCapturePort, NativeCaptureReceiver) {
                let (mut port, receiver) = self.channel(limits, masks);
                self.prepare(&mut port);
                self.finalize(&mut port);
                (port, receiver)
            }
            pub(crate) fn request(&self, id: &str) -> NativeCaptureRequest {
                NativeCaptureRequest {
                    capture_id: id.into(),
                    run_id: self.fixture.run.run_id,
                    owner_epoch: self.fixture.run.owner_epoch,
                    minimum_interval: Duration::from_millis(100),
                    deadline: self.clock.sample().monotonic + Duration::from_secs(1),
                }
            }
            pub(crate) fn pair(
                &mut self,
                port: &mut NativeCapturePort,
                receiver: &NativeCaptureReceiver,
                bytes: &[u8],
            ) -> NativeCaptureLease {
                let mut terminal = TerminalState::new(self.spec.size).unwrap();
                terminal.feed(bytes).unwrap();
                receiver.request(self.request("pair")).unwrap();
                port.observe(
                    &terminal.snapshot(),
                    CaptureTrigger::ChildOutput,
                    &self.clock,
                )
                .unwrap();
                self.clock.set(100);
                port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &self.clock)
                    .unwrap();
                receiver
                    .try_take()
                    .unwrap()
                    .expect("completed post-policy pair")
            }
        }

        pub(crate) fn value(lease: &NativeCaptureLease) -> Value {
            serde_json::to_value(lease.result()).unwrap()
        }
        impl Drop for CaptureFixture {
            fn drop(&mut self) {
                self.child.take();
            }
        }
        pub(crate) fn objects<'a>(value: &'a Value, key: &str, result: &mut Vec<&'a Value>) {
            match value {
                Value::Object(map) => {
                    if map.contains_key(key) {
                        result.push(value);
                    }
                    for child in map.values() {
                        objects(child, key, result);
                    }
                }
                Value::Array(values) => {
                    for child in values {
                        objects(child, key, result);
                    }
                }
                _ => {}
            }
        }
        pub(crate) fn assert_keys(value: &Value, keys: &[&str]) {
            let actual: std::collections::BTreeSet<_> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(actual, keys.iter().copied().collect());
        }
        pub(crate) fn arm(identity: &ControlIdentity) -> NativeCaptureArm {
            NativeCaptureArm::for_identity("review-pair", identity, 1000)
        }
        pub(crate) fn config(fixture: &CaptureFixture) -> NativeCaptureConfig {
            let policy = fixture.fixture._dir.path().join("capture-policy.json");
            private_write(
                &policy,
                &serde_json::to_vec(&json!({"schema_version":1,"masks":[]})).unwrap(),
            );
            NativeCaptureConfig {
                schema_version: 1,
                database_url: format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                descriptor_path: fixture.artifacts.paths.descriptor.clone(),
                expected_run_id: fixture.fixture.run.run_id,
                expected_launch_nonce: fixture.fixture.run.launch_nonce.clone(),
                arm_path: fixture.fixture._dir.path().join("arm.json"),
                output_directory: fixture.fixture._dir.path().to_path_buf(),
                producer_manifest_path: fixture.producer_path.clone(),
                capture_policy_path: policy,
            }
        }
    }

    mod tests {
        use std::result::Result;

        use serde_json::{Value, json};

        use super::test_support::*;
        use super::*;
        use crate::broker::runtime::{self as storage, *};
        use crate::digest::sha256_hex;
        use crate::runtime::runner::service::test_support::*;

        mod collection {
            use super::*;
            use std::time::Duration;

            #[test]
            fn capture_seed_prepared_and_finalized_boundaries_enable_exactly_one_request() {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
                assert!(receiver.finalized_identity().is_none());
                assert!(receiver.request(fixture.request("early")).is_err());
                fixture.prepare(&mut port);
                assert!(receiver.finalized_identity().is_none());
                assert!(receiver.request(fixture.request("prepared")).is_err());
                let producer = read_native_capture_producer(
                    &fixture.producer_path,
                    &NativeCaptureLimits::default(),
                )
                .unwrap();
                assert!(
                    port.bind_prepared(
                        &fixture.fixture.run,
                        &fixture.placement,
                        &fixture.spec,
                        producer,
                        &fixture.clock
                    )
                    .is_err()
                );
                fixture.finalize(&mut port);
                let identity = receiver.finalized_identity().unwrap();
                assert_eq!(identity.run_id, fixture.fixture.run.run_id);
                assert_eq!(identity.owner_epoch, fixture.fixture.run.owner_epoch);
                let child = ProcessIdentity {
                    pid: fixture.fixture.run.child_pid.unwrap(),
                    start_identity: fixture.fixture.run.child_start_identity.clone().unwrap(),
                };
                assert!(
                    port.finalize_source(&fixture.fixture.run, &fixture.placement, &child)
                        .is_err()
                );
                receiver.request(fixture.request("accepted")).unwrap();
                assert!(receiver.request(fixture.request("second")).is_err());
                let screen = TerminalState::new(fixture.spec.size).unwrap().snapshot();
                port.observe(&screen, CaptureTrigger::Timer, &fixture.clock)
                    .unwrap();
                fixture.clock.set(100);
                port.observe(&screen, CaptureTrigger::Timer, &fixture.clock)
                    .unwrap();
                let lease = receiver.try_take().unwrap().unwrap();
                assert!(receiver.request(fixture.request("leased")).is_err());
                assert!(receiver.try_take().unwrap().is_none());
                drop(lease);
                assert!(receiver.request(fixture.request("consumed")).is_err());
            }

            #[test]
            fn capture_source_rejects_running_seed_and_mismatched_child_or_placement() {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
                fixture.prepare(&mut port);
                let identity = ProcessIdentity::current().unwrap();
                let mut run = fixture.fixture.run.clone();
                run.status = RunStatus::Running;
                run.child_pid = Some(identity.pid);
                run.child_start_identity = Some(identity.start_identity.clone());
                run.started_at = Some("2026-10-26T17:00:00Z".into());
                let mut wrong = identity.clone();
                wrong.start_identity.push_str("-wrong");
                assert!(
                    port.finalize_source(&run, &fixture.placement, &wrong)
                        .is_err()
                );
                assert!(receiver.finalized_identity().is_none());
                let mut placement = fixture.placement.clone();
                placement.mux_pane_id = Some("%999".into());
                assert!(port.finalize_source(&run, &placement, &identity).is_err());
                assert!(receiver.finalized_identity().is_none());
                let policy =
                    NativeCapturePolicy::from_launch(&fixture.spec, &[], &catalog()).unwrap();
                assert!(
                    native_capture_channel(&run, policy, NativeCaptureLimits::default()).is_err()
                );
                fixture.finalize(&mut port);
                assert!(receiver.finalized_identity().is_some());
            }

            #[test]
            fn capture_invalid_request_leaves_the_finalized_channel_unused() {
                for field in [
                    "blank",
                    "path",
                    "control",
                    "long",
                    "unicode",
                    "run",
                    "epoch",
                    "interval_low",
                    "interval_high",
                    "past",
                    "deadline_high",
                ] {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let (_port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                    let mut request = fixture.request("valid");
                    match field {
                        "blank" => request.capture_id = " ".into(),
                        "path" => request.capture_id = "../private".into(),
                        "control" => request.capture_id = "a\u{7f}".into(),
                        "long" => request.capture_id = "a".repeat(129),
                        "unicode" => request.capture_id = "世界".into(),
                        "run" => request.run_id += 1,
                        "epoch" => request.owner_epoch += 1,
                        "interval_low" => request.minimum_interval = Duration::from_millis(99),
                        "interval_high" => request.minimum_interval = Duration::from_millis(5001),
                        "past" => {
                            request.deadline =
                                fixture.clock.sample().monotonic - Duration::from_millis(1)
                        }
                        _ => {
                            request.deadline =
                                fixture.clock.sample().monotonic + Duration::from_secs(31)
                        }
                    }
                    assert!(receiver.request(request).is_err(), "{field}");
                    receiver.request(fixture.request("valid")).unwrap();
                }
            }

            #[test]
            fn capture_slow_consumer_holds_one_immutable_pair_while_observation_keeps_progressing()
            {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let lease = fixture.pair(&mut port, &receiver, b"first");
                let original = value(&lease);
                let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
                let began = std::time::Instant::now();
                for _ in 0..100 {
                    terminal.feed(b"more native output\r\n").unwrap();
                    port.observe(
                        &terminal.snapshot(),
                        CaptureTrigger::ChildOutput,
                        &fixture.clock,
                    )
                    .unwrap();
                }
                assert!(began.elapsed() < Duration::from_millis(500));
                assert_eq!(value(&lease), original);
                assert!(receiver.try_take().unwrap().is_none());
                assert!(receiver.request(fixture.request("replacement")).is_err());
            }

            #[test]
            fn capture_pair_uses_atomic_snapshots_and_second_selection_after_first_end() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
                terminal
                    .feed(b"\x1b]2;first title\x07\x1b[2;4Hfirst frame")
                    .unwrap();
                let first = terminal.snapshot();
                receiver.request(fixture.request("atomic")).unwrap();
                port.observe(&first, CaptureTrigger::ChildOutput, &fixture.clock)
                    .unwrap();
                assert!(receiver.try_take().unwrap().is_none());
                assert!(port.next_deadline().is_some());
                terminal
                    .resize(TerminalSize { rows: 25, cols: 90 })
                    .unwrap();
                terminal
                    .feed(b"\x1b[?1049h\x1b]2;second title\x07\x1b[3;7Hsecond frame")
                    .unwrap();
                let second = terminal.snapshot();
                fixture.clock.set(99);
                port.observe(&second, CaptureTrigger::Resize, &fixture.clock)
                    .unwrap();
                assert!(receiver.try_take().unwrap().is_none());
                fixture.clock.set(100);
                port.observe(&second, CaptureTrigger::Timer, &fixture.clock)
                    .unwrap();
                let lease = receiver.try_take().unwrap().unwrap();
                let result = value(&lease);
                assert_keys(
                    &result,
                    &[
                        "schema_version",
                        "capture_id",
                        "source",
                        "policy",
                        "limits",
                        "status",
                        "samples",
                        "diagnostic",
                    ],
                );
                assert_eq!(result["schema_version"], 1);
                assert_eq!(result["status"], "complete");
                let samples = result["samples"].as_array().unwrap();
                assert_eq!(samples.len(), 2);
                for (index, screen) in [&first, &second].into_iter().enumerate() {
                    let sample = &samples[index];
                    assert_keys(
                        sample,
                        &[
                            "sample_index",
                            "trigger",
                            "monotonic_start_ns",
                            "monotonic_end_ns",
                            "observed_at",
                            "screen_seq",
                            "rows",
                            "cols",
                            "cursor_row",
                            "cursor_col",
                            "alternate_screen",
                            "title",
                            "projection",
                            "unavailable_regions",
                            "grid",
                        ],
                    );
                    assert_eq!(sample["screen_seq"], screen.screen_seq);
                    assert_eq!(sample["rows"], screen.rows);
                    assert_eq!(sample["cols"], screen.cols);
                    assert_eq!(sample["cursor_row"], screen.cursor_row);
                    assert_eq!(sample["cursor_col"], screen.cursor_col);
                    assert_eq!(sample["alternate_screen"], screen.alternate_screen);
                    assert_eq!(sample["title"], screen.title);
                    assert!(
                        sample["monotonic_end_ns"].as_u64().unwrap()
                            >= sample["monotonic_start_ns"].as_u64().unwrap()
                    );
                }
                assert!(
                    samples[1]["monotonic_start_ns"].as_u64().unwrap()
                        - samples[0]["monotonic_end_ns"].as_u64().unwrap()
                        >= 100_000_000
                );
                assert!(
                    serde_json::to_string(&samples[0])
                        .unwrap()
                        .contains("first")
                );
                assert!(
                    !serde_json::to_string(&samples[0])
                        .unwrap()
                        .contains("second")
                );
                assert!(port.next_deadline().is_none());
            }

            #[test]
            fn capture_metadata_projects_no_grid_title_secret_or_private_source_arguments() {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let lease = fixture.pair(
                    &mut port,
                    &receiver,
                    b"\x1b]2;PRIVATE-TITLE\x07PRIVATE-CONTENT",
                );
                let result = value(&lease);
                let encoded = serde_json::to_string(&result).unwrap();
                assert!(!encoded.contains("PRIVATE-"));
                assert!(!encoded.contains("private assignment canary"));
                assert!(!encoded.contains("fingerprint"));
                assert!(!encoded.contains("raw_bytes"));
                for sample in result["samples"].as_array().unwrap() {
                    assert!(sample["grid"].is_null());
                    assert!(sample["title"].is_null());
                    assert!(!sample["unavailable_regions"].as_array().unwrap().is_empty());
                }
                let source = &result["source"];
                assert_eq!(source["run_id"], fixture.fixture.run.run_id);
                assert_eq!(source["owner_epoch"], fixture.fixture.run.owner_epoch);
                assert_eq!(
                    source["child"]["pid"],
                    fixture.fixture.run.child_pid.unwrap()
                );
                assert_eq!(
                    source["permission_argv"],
                    fixture.fixture.run.permission_argv_json
                );
                assert_eq!(source["resolved_mode"], "approve-for-me");
                assert_eq!(source["producer"]["kind"], "synthetic");
                assert!(
                    serde_json::to_string(&source["clock"])
                        .unwrap()
                        .contains("deterministic")
                );
            }

            #[test]
            fn capture_patterns_mask_whole_logical_fields_across_softwraps_and_cells() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let mut bytes = vec![b'x'; 75];
                bytes.extend_from_slice(b"PRIVATE-SECRET");
                let lease = fixture.pair(&mut port, &receiver, &bytes);
                let result = value(&lease);
                let encoded = serde_json::to_string(&result).unwrap();
                assert!(!encoded.contains("PRIVATE-SECRET"));
                assert!(!encoded.contains("[REDACTED]"));
                assert!(!encoded.contains("PRIVATE-[A-Z]+"));
                let mut cells = Vec::new();
                objects(&result["samples"][0]["grid"], "has_contents", &mut cells);
                assert_eq!(cells.len(), 24 * 80);
                assert!(
                    cells
                        .iter()
                        .take(89)
                        .all(|cell| cell["availability"] == "masked")
                );
                assert!(
                    cells
                        .iter()
                        .filter(|cell| cell["availability"] == "masked")
                        .count()
                        >= 89
                );
                for cell in cells.iter().filter(|cell| cell["availability"] == "masked") {
                    for field in [
                        "contents",
                        "has_contents",
                        "wide",
                        "wide_continuation",
                        "fg",
                        "bg",
                        "bold",
                        "dim",
                        "italic",
                        "underline",
                        "inverse",
                    ] {
                        assert!(
                            cell.get(field)
                                .expect("required masked cell field")
                                .is_null(),
                            "masked {field}"
                        );
                    }
                }
            }

            #[test]
            fn capture_missing_in_range_parser_cell_fails_at_the_projection_boundary() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                receiver.request(fixture.request("missing-cell")).unwrap();
                let mut screen = TerminalState::new(fixture.spec.size).unwrap().snapshot();
                screen.rows += 1;
                assert!(
                    port.observe(&screen, CaptureTrigger::ChildOutput, &fixture.clock)
                        .is_err()
                );
            }

            #[test]
            fn capture_coordinate_mask_includes_wide_partner_and_preserves_empty_cells() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) =
                    fixture.ready(NativeCaptureLimits::default(), &[mask(0, 2, 1, 3)]);
                let lease = fixture.pair(&mut port, &receiver, "A界Z".as_bytes());
                let result = value(&lease);
                let mut cells = Vec::new();
                objects(&result["samples"][0]["grid"], "has_contents", &mut cells);
                assert_eq!(cells.len(), 1920);
                assert_eq!(
                    cells
                        .iter()
                        .filter(|cell| cell["availability"] == "masked")
                        .count(),
                    2
                );
                assert!(cells.iter().any(|cell| cell["contents"] == "A"));
                assert!(cells.iter().any(|cell| cell["contents"] == "Z"));
                assert!(
                    cells
                        .iter()
                        .any(|cell| cell["contents"] == "" && cell["has_contents"] == false)
                );
                assert!(!serde_json::to_string(&result).unwrap().contains('界'));
            }

            #[test]
            fn capture_layout_mismatch_masks_whole_content_instead_of_clipping_coordinates() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) =
                    fixture.ready(NativeCaptureLimits::default(), &[mask(0, 0, 1, 1)]);
                let mut terminal = TerminalState::new(TerminalSize { rows: 25, cols: 80 }).unwrap();
                terminal.feed(b"PRIVATE-LAYOUT").unwrap();
                receiver.request(fixture.request("layout")).unwrap();
                port.observe(&terminal.snapshot(), CaptureTrigger::Resize, &fixture.clock)
                    .unwrap();
                fixture.clock.set(100);
                port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
                    .unwrap();
                let result = value(&receiver.try_take().unwrap().unwrap());
                assert!(
                    !serde_json::to_string(&result)
                        .unwrap()
                        .contains("PRIVATE-LAYOUT")
                );
                assert!(
                    !result["samples"][0]["unavailable_regions"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }

            #[test]
            fn capture_invalid_mask_geometry_fails_before_collection() {
                let fixture = CaptureFixture::new(redacted());
                for invalid in [
                    mask(0, 0, 0, 1),
                    mask(0, 0, 1, 0),
                    mask(0, 0, 25, 1),
                    mask(0, 0, 1, 81),
                ] {
                    assert!(
                        NativeCapturePolicy::from_launch(&fixture.spec, &[invalid], &catalog())
                            .is_err()
                    );
                }
            }

            #[test]
            fn capture_sensitive_rule_narrows_redacted_launch_to_metadata() {
                let mut policy = redacted();
                policy["config"]["sensitive_rules"] = json!(["synthetic-permission"]);
                let mut fixture = CaptureFixture::new(policy);
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let lease=fixture.pair(&mut port,&receiver,b"\x1b[19;1HSynthetic permission request\r\nOperation: PRIVATE-SECRET\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ");
                let result = value(&lease);
                for sample in result["samples"].as_array().unwrap() {
                    assert!(sample["grid"].is_null());
                }
                assert!(
                    !serde_json::to_string(&result)
                        .unwrap()
                        .contains("PRIVATE-SECRET")
                );
            }

            #[test]
            fn capture_lowered_budgets_report_capacity_without_a_padded_complete_pair() {
                for limits in [
                    NativeCaptureLimits {
                        max_cells: 10,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_source_bytes: 4,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_title_bytes: 4,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_scratch_bytes: 1,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_projected_heap_bytes: 1,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_encoded_sample_bytes: 1,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_encoded_result_bytes: 1,
                        ..Default::default()
                    },
                ] {
                    let mut fixture = CaptureFixture::new(redacted());
                    let (mut port, receiver) = fixture.ready(limits, &[]);
                    receiver.request(fixture.request("capacity")).unwrap();
                    let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
                    terminal
                        .feed(b"\x1b]2;title too large\x07content too large")
                        .unwrap();
                    port.observe(
                        &terminal.snapshot(),
                        CaptureTrigger::ChildOutput,
                        &fixture.clock,
                    )
                    .unwrap();
                    fixture.clock.set(100);
                    port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
                        .unwrap();
                    let result = value(
                        &receiver
                            .try_take()
                            .unwrap()
                            .expect("explicit bounded capacity result"),
                    );
                    assert_eq!(result["status"], "capacity");
                    assert!(result["samples"].as_array().unwrap().len() < 2);
                }
            }

            #[test]
            fn capture_limits_reject_zero_and_expansion_but_allow_lowered_budgets() {
                assert!(NativeCaptureLimits::default().validate().is_ok());
                assert!(
                    NativeCaptureLimits {
                        max_cells: 1,
                        ..Default::default()
                    }
                    .validate()
                    .is_ok()
                );
                for limits in [
                    NativeCaptureLimits {
                        max_cells: 0,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_cells: 32769,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_source_bytes: 1048577,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_patterns: 65,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_pattern_bytes: 4097,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_total_pattern_bytes: 65537,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_compiled_regex_bytes: 262145,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_pattern_input_bytes: 67108865,
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_interval: Duration::from_millis(99),
                        ..Default::default()
                    },
                    NativeCaptureLimits {
                        max_request_duration: Duration::from_millis(100),
                        ..Default::default()
                    },
                ] {
                    assert!(limits.validate().is_err());
                }
            }

            #[test]
            fn capture_deadline_owner_exit_and_cancellation_preserve_actual_incomplete_status() {
                for status in [
                    NativeCaptureStatus::Deadline,
                    NativeCaptureStatus::OwnerExit,
                    NativeCaptureStatus::Cancelled,
                ] {
                    let mut fixture = CaptureFixture::new(redacted());
                    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                    receiver.request(fixture.request("incomplete")).unwrap();
                    let terminal = TerminalState::new(fixture.spec.size).unwrap();
                    port.observe(
                        &terminal.snapshot(),
                        CaptureTrigger::ChildOutput,
                        &fixture.clock,
                    )
                    .unwrap();
                    if status == NativeCaptureStatus::Deadline {
                        fixture.clock.set(1001);
                        port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
                            .unwrap();
                    } else {
                        port.cancel(status).unwrap();
                    }
                    let result = value(&receiver.try_take().unwrap().unwrap());
                    assert_eq!(result["status"], serde_json::to_value(status).unwrap());
                    assert_eq!(result["samples"].as_array().unwrap().len(), 1);
                    assert!(receiver.request(fixture.request("replacement")).is_err());
                }
            }
        }

        mod capture_policy {
            use super::*;
            use std::sync::Mutex;

            #[derive(Default)]
            struct Compiler {
                calls: Mutex<Vec<(String, usize)>>,
            }

            /// A launch spec carrying only `policy`: `NativeCapturePolicy::from_launch` reads
            /// nothing else, so these tests need no database, run lock or native executable.
            fn policy_spec(policy: Value) -> SpawnSpec {
                SpawnSpec {
                    argv: vec!["/bin/cat".into()],
                    cwd: std::path::PathBuf::from("/"),
                    environment: Vec::new(),
                    evidence_policy: policy,
                    size: TerminalSize { rows: 24, cols: 80 },
                }
            }
            impl NativeCapturePatternCompiler for Compiler {
                fn compile(
                    &self,
                    pattern: &str,
                    max_compiled_bytes: usize,
                ) -> Result<regex::Regex, crate::error::CafleetError> {
                    self.calls
                        .lock()
                        .unwrap()
                        .push((pattern.into(), max_compiled_bytes));
                    BoundedCapturePatternCompiler.compile(pattern, max_compiled_bytes)
                }
            }

            #[test]
            fn runner_collector_whole_raw_policy_is_validated_before_the_first_compiler_invocation()
            {
                let limits = NativeCaptureLimits::default();
                for invalid in [
                    "schema",
                    "empty",
                    "later_type",
                    "count",
                    "later_bytes",
                    "aggregate",
                    "sensitive",
                    "later_sensitive_type",
                ] {
                    let mut policy = redacted();
                    policy["config"]["patterns"] = json!(["safe first pattern"]);
                    match invalid {
                        "schema" => policy["config"]["schema_version"] = json!(2),
                        "empty" => policy["config"]["patterns"] = json!([]),
                        "later_type" => {
                            policy["config"]["patterns"] = json!(["safe first pattern", 7])
                        }
                        "count" => {
                            let mut patterns = vec!["safe".to_owned(); limits.max_patterns + 1];
                            patterns[0] = "[".into();
                            policy["config"]["patterns"] = json!(patterns);
                        }
                        "later_bytes" => {
                            policy["config"]["patterns"] = json!([
                                "safe first pattern",
                                "x".repeat(limits.max_pattern_bytes + 1)
                            ])
                        }
                        "aggregate" => {
                            let width = limits.max_pattern_bytes / 2;
                            let count = limits.max_total_pattern_bytes / width + 1;
                            assert!(count <= limits.max_patterns);
                            policy["config"]["patterns"] = json!(vec!["x".repeat(width); count]);
                        }
                        "sensitive" => {
                            policy["config"]["sensitive_rules"] = json!(["unknown-reviewed-rule"])
                        }
                        "later_sensitive_type" => {
                            policy["config"]["sensitive_rules"] = json!(["synthetic-permission", 7])
                        }
                        _ => unreachable!(),
                    }
                    let compiler = Compiler::default();
                    assert!(
                        NativeCapturePolicy::from_launch_with_compiler(
                            &policy_spec(policy),
                            &[],
                            &catalog(),
                            &compiler
                        )
                        .is_err(),
                        "{invalid}"
                    );
                    assert!(compiler.calls.lock().unwrap().is_empty(), "{invalid}");
                }
            }

            #[test]
            fn runner_collector_valid_patterns_compile_once_in_source_order_with_the_approved_cap()
            {
                let patterns = ["PRIVATE-[A-Z]+", "[0-9]+", "(?i)secret"];
                let mut policy = redacted();
                policy["config"]["patterns"] = json!(patterns);
                policy["config"]["sensitive_rules"] = json!(["synthetic-permission"]);
                let compiler = Compiler::default();
                assert!(
                    NativeCapturePolicy::from_launch_with_compiler(
                        &policy_spec(policy),
                        &[],
                        &catalog(),
                        &compiler
                    )
                    .is_ok()
                );
                let expected: Vec<_> = patterns
                    .into_iter()
                    .map(|pattern| {
                        (
                            pattern.to_owned(),
                            NativeCaptureLimits::default().max_compiled_regex_bytes,
                        )
                    })
                    .collect();
                assert_eq!(*compiler.calls.lock().unwrap(), expected);
            }

            #[test]
            fn runner_collector_compact_pattern_expansion_stops_at_the_first_bounded_compilation() {
                let mut policy = redacted();
                policy["config"]["patterns"] = json!(["[a-z]{100000}"]);
                let compiler = Compiler::default();
                assert!(
                    NativeCapturePolicy::from_launch_with_compiler(
                        &policy_spec(policy),
                        &[],
                        &catalog(),
                        &compiler
                    )
                    .is_err()
                );
                assert_eq!(
                    *compiler.calls.lock().unwrap(),
                    vec![(
                        "[a-z]{100000}".into(),
                        NativeCaptureLimits::default().max_compiled_regex_bytes
                    )]
                );
            }

            #[test]
            fn runner_collector_metadata_policy_uses_no_regex_compiler() {
                let compiler = Compiler::default();
                assert!(
                    NativeCapturePolicy::from_launch_with_compiler(
                        &policy_spec(json!({"mode":"metadata"})),
                        &[],
                        &catalog(),
                        &compiler
                    )
                    .is_ok()
                );
                assert!(compiler.calls.lock().unwrap().is_empty());
            }

            #[test]
            fn runner_collector_channel_selected_compiled_cap_remains_bounded_after_valid_construction()
             {
                let mut policy = redacted();
                policy["config"]["patterns"] = json!(["[a-z]{100}"]);
                let fixture = CaptureFixture::new(policy);
                let compiler = Compiler::default();
                let policy = NativeCapturePolicy::from_launch_with_compiler(
                    &fixture.spec,
                    &[],
                    &catalog(),
                    &compiler,
                )
                .unwrap();
                let limits = NativeCaptureLimits {
                    max_compiled_regex_bytes: 128,
                    ..NativeCaptureLimits::default()
                };
                assert!(native_capture_channel(&fixture.fixture.run, policy, limits).is_err());
                assert_eq!(compiler.calls.lock().unwrap().len(), 1);
            }
        }

        mod binding {
            use super::*;
            #[test]
            fn capture_bind_checks_actual_native_digest_before_any_observation() {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
                let mut producer = fixture.producer_json.clone();
                producer["native_executable"]["sha256"] = json!("0".repeat(64));
                let path = fixture.fixture._dir.path().join("wrong-native.json");
                private_write(&path, &serde_json::to_vec(&producer).unwrap());
                let producer =
                    read_native_capture_producer(&path, &NativeCaptureLimits::default()).unwrap();
                fixture.mark_prepared();
                assert!(
                    port.bind_prepared(
                        &fixture.fixture.run,
                        &fixture.placement,
                        &fixture.spec,
                        producer,
                        &fixture.clock
                    )
                    .is_err()
                );
                assert!(receiver.finalized_identity().is_none());
                assert!(receiver.request(fixture.request("unbound")).is_err());
            }
        }

        mod recorded_policy {
            use super::*;
            use crate::coding_agent::ProfileCatalog;

            fn select_catalog(fixture: &mut CaptureFixture, bundled: bool) -> ProfileCatalog {
                if bundled {
                    let selected = ProfileCatalog::bundled().unwrap().capabilities(
                        crate::coding_agent::CodingAgent::Codex,
                        fixture.fixture.run.native_version.as_deref(),
                    );
                    fixture
                        .fixture
                        .conn
                        .execute(
                            "UPDATE member_runs SET profile_version=?1,coverage=?2 WHERE run_id=?3",
                            rusqlite::params![
                                selected.profile_version,
                                selected.detection.as_str(),
                                fixture.fixture.run.run_id
                            ],
                        )
                        .unwrap();
                    fixture.fixture.run =
                        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                            .unwrap();
                    ProfileCatalog::bundled().unwrap()
                } else {
                    catalog()
                }
            }

            #[test]
            fn runner_collector_actual_permission_options_match_the_recorded_emitted_policy_before_binding()
             {
                for mutation in [
                    "approval",
                    "missing_approval",
                    "repeat_approval",
                    "added_sandbox",
                    "trailing_sandbox",
                ] {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let (mut port, receiver) =
                        fixture.channel_with(NativeCaptureLimits::default(), &[], &catalog());
                    match mutation {
                        "approval" => fixture.spec.argv[1] = "--full-auto".into(),
                        "missing_approval" => {
                            fixture.spec.argv.remove(1);
                        }
                        "repeat_approval" => fixture.spec.argv.push("--approve-for-me".into()),
                        "added_sandbox" => fixture
                            .spec
                            .argv
                            .splice(2..2, ["--sandbox".into(), "workspace-write".into()])
                            .for_each(drop),
                        "trailing_sandbox" => fixture
                            .spec
                            .argv
                            .extend(["--sandbox".into(), "danger-full-access".into()]),
                        _ => unreachable!(),
                    }
                    fixture.mark_prepared();
                    assert!(
                        port.bind_prepared(
                            &fixture.fixture.run,
                            &fixture.placement,
                            &fixture.spec,
                            fixture.producer(),
                            &fixture.clock
                        )
                        .is_err(),
                        "{mutation}"
                    );
                    assert!(receiver.finalized_identity().is_none());
                    assert!(receiver.request(fixture.request("unbound-policy")).is_err());
                    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                        .unwrap();
                    assert!(run.child_pid.is_none());
                }
            }

            #[test]
            fn runner_collector_flag_like_prompt_is_opaque_and_preserves_truthful_permission_source()
             {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                *fixture.spec.argv.last_mut().unwrap() =
                    "explain --ask-for-approval on-request and --sandbox read-only".into();
                let (mut port, receiver) =
                    fixture.channel_with(NativeCaptureLimits::default(), &[], &catalog());
                fixture.prepare(&mut port);
                fixture.finalize(&mut port);
                let lease = fixture.pair(&mut port, &receiver, b"safe source control");
                let result = value(&lease);
                assert_eq!(
                    result["source"]["permission_argv"],
                    fixture.fixture.run.permission_argv_json
                );
                assert_eq!(result["source"]["producer"]["catalog_origin"], "synthetic");
                assert!(
                    !serde_json::to_string(&result)
                        .unwrap()
                        .contains("explain --ask-for-approval")
                );
            }

            #[test]
            fn runner_collector_producer_origin_matches_the_selected_catalog_constructor_after_clone()
             {
                for bundled in [false, true] {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let actual = select_catalog(&mut fixture, bundled);
                    let selected = actual.clone();
                    let (mut port, receiver) =
                        fixture.channel_with(NativeCaptureLimits::default(), &[], &selected);
                    fixture.producer_json["catalog_origin"] = json!(if bundled {
                        "synthetic"
                    } else {
                        "bundled_unknown"
                    });
                    std::fs::write(
                        &fixture.producer_path,
                        serde_json::to_vec(&fixture.producer_json).unwrap(),
                    )
                    .unwrap();
                    fixture.mark_prepared();
                    assert!(
                        port.bind_prepared(
                            &fixture.fixture.run,
                            &fixture.placement,
                            &fixture.spec,
                            fixture.producer(),
                            &fixture.clock
                        )
                        .is_err(),
                        "bundled={bundled}"
                    );
                    assert!(receiver.finalized_identity().is_none());
                    assert!(receiver.request(fixture.request("unbound-origin")).is_err());
                    assert!(fixture.fixture.run.child_pid.is_none());
                }
            }

            #[test]
            fn runner_collector_matching_synthetic_and_bundled_origins_publish_only_their_actual_provenance()
             {
                for bundled in [false, true] {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let selected = select_catalog(&mut fixture, bundled);
                    let origin = if bundled {
                        "bundled_unknown"
                    } else {
                        "synthetic"
                    };
                    fixture.producer_json["catalog_origin"] = json!(origin);
                    std::fs::write(
                        &fixture.producer_path,
                        serde_json::to_vec(&fixture.producer_json).unwrap(),
                    )
                    .unwrap();
                    let (mut port, receiver) =
                        fixture.channel_with(NativeCaptureLimits::default(), &[], &selected);
                    fixture.prepare(&mut port);
                    fixture.finalize(&mut port);
                    let lease = fixture.pair(&mut port, &receiver, b"safe matching origin");
                    let result = value(&lease);
                    assert_eq!(result["source"]["producer"]["catalog_origin"], origin);
                    assert_eq!(
                        result["source"]["profile_version"],
                        json!(fixture.fixture.run.profile_version)
                    );
                    assert_eq!(
                        result["source"]["permission_argv"],
                        fixture.fixture.run.permission_argv_json
                    );
                }
            }
        }

        mod service_capture {
            use super::*;
            use crate::runtime::runner::service::test_support::observation_services::{
                capture, private_write, wait_until,
            };
            #[test]
            fn service_private_capture_publishes_manifest_last_and_keeps_native_work_running() {
                let mut fixture = ServiceFixture::configured(false, capture);
                let started = fixture.start_running();
                let request =
                    NativeCaptureArm::for_identity("service-pair", &started.identity, 2000);
                let arm_at = chrono::Utc::now();
                private_write(
                    &fixture.fixture._dir.path().join("capture-arm.json"),
                    &serde_json::to_vec(&request).unwrap(),
                );
                let output = fixture.fixture._dir.path().join("capture-output");
                let manifest_path = output.join("service-pair.manifest.json");
                wait_until(|| manifest_path.exists());
                let manifest: Value =
                    serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
                assert_eq!(manifest["publication_status"], "complete");
                let artifact = std::fs::read(output.join("service-pair.complete.json"))
                    .expect("manifest publication follows complete synced artifact");
                assert_eq!(manifest["artifact_bytes"], artifact.len() as u64);
                assert_eq!(manifest["artifact_sha256"], sha256_hex(&artifact));
                let result: Value = serde_json::from_slice(&artifact).unwrap();
                assert_eq!(result["status"], "complete");
                assert_eq!(result["samples"].as_array().unwrap().len(), 2);
                assert_eq!(result["source"]["producer"]["kind"], "synthetic");
                assert_eq!(
                    result["source"]["producer"]["catalog_origin"],
                    "bundled_unknown"
                );
                assert_eq!(
                    result["source"]["runner"],
                    serde_json::to_value(&started.runner).unwrap()
                );
                assert_eq!(
                    result["source"]["launch_nonce"],
                    started.identity.launch_nonce
                );
                let clock = serde_json::to_string(&result["source"]["clock"])
                    .unwrap()
                    .to_lowercase();
                assert!(result["source"]["clock"].is_object());
                for attribution in ["real", "instant", "utc", "origin"] {
                    assert!(
                        clock.contains(attribution),
                        "real service clock requires {attribution}: {clock}"
                    );
                }
                assert!(!clock.contains("deterministic"));
                let samples = result["samples"].as_array().unwrap();
                let first_end = samples[0]["monotonic_end_ns"].as_u64().unwrap();
                let second_start = samples[1]["monotonic_start_ns"].as_u64().unwrap();
                assert!(second_start.checked_sub(first_end).unwrap() >= 100_000_000);
                let collected_at = chrono::Utc::now();
                for sample in samples {
                    assert!(
                        sample["monotonic_end_ns"].as_u64().unwrap()
                            >= sample["monotonic_start_ns"].as_u64().unwrap()
                    );
                    let observed = chrono::DateTime::parse_from_rfc3339(
                        sample["observed_at"].as_str().unwrap(),
                    )
                    .unwrap();
                    assert!(observed >= arm_at && observed <= collected_at);
                }
                assert_eq!(result["source"]["child"]["pid"], started.child.pid);
                assert_eq!(
                    result["source"]["owner_epoch"],
                    started.identity.owner_epoch
                );
                assert!(fixture.child.try_wait().unwrap().is_none());
                fixture.health();
                fixture.assert_manual_io();
                fixture.finish();
            }
        }
    }

    mod exporter {
        use super::*;
        use std::thread::JoinHandle;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum NativeCaptureExportBoundary {
            Encode,
            ArtifactPublish,
            ManifestPublish,
        }
        pub(crate) trait NativeCaptureExportHooks: Send + Sync {
            fn before(
                &self,
                boundary: NativeCaptureExportBoundary,
                deadline: Instant,
            ) -> Result<()>;
            fn service_returned(&self, _storage_deadline: Option<Instant>, _result: &Result<i32>) {}
        }
        pub(super) struct InertCaptureExportHooks;
        impl NativeCaptureExportHooks for InertCaptureExportHooks {
            fn before(&self, _: NativeCaptureExportBoundary, _: Instant) -> Result<()> {
                Ok(())
            }
        }
        struct PublicationAuthority {
            revoked: bool,
            acknowledged: bool,
            active: usize,
        }
        pub(super) struct CaptureExportControl {
            capture: Arc<Mutex<CaptureState>>,
            authority: Mutex<PublicationAuthority>,
        }
        pub(super) struct PublicationPermit<'a> {
            control: &'a CaptureExportControl,
        }
        impl Drop for PublicationPermit<'_> {
            fn drop(&mut self) {
                let mut state = self
                    .control
                    .authority
                    .lock()
                    .expect("publication authority");
                state.active = state
                    .active
                    .checked_sub(1)
                    .expect("owned publication permit");
            }
        }
        impl CaptureExportControl {
            fn new(capture: Arc<Mutex<CaptureState>>) -> Self {
                Self {
                    capture,
                    authority: Mutex::new(PublicationAuthority {
                        revoked: false,
                        acknowledged: false,
                        active: 0,
                    }),
                }
            }
            pub(super) fn deadline(&self, requested: Instant) -> Instant {
                self.capture
                    .lock()
                    .expect("capture mailbox")
                    .clip(requested)
            }
            pub(super) fn acquire(&self, requested: Instant) -> Result<PublicationPermit<'_>> {
                let until = self.deadline(requested);
                let mut state = self.authority.lock().expect("publication authority");
                ensure(
                    !state.revoked,
                    "capture exporter unfinished after publication cancellation",
                )?;
                check_deadline(Instant::now(), until)?;
                state.active = state
                    .active
                    .checked_add(1)
                    .expect("publication permit count overflow");
                Ok(PublicationPermit { control: self })
            }
            fn cancel(&self) -> Option<Instant> {
                let until = self.capture.lock().expect("capture mailbox").shutdown_until;
                self.authority
                    .lock()
                    .expect("publication authority")
                    .revoked = true;
                until
            }
            fn cancelled(&self) -> bool {
                self.authority
                    .lock()
                    .expect("publication authority")
                    .revoked
            }
            pub(super) fn acknowledge(&self) -> Result<()> {
                let mut state = self.authority.lock().expect("publication authority");
                ensure(
                    !state.revoked,
                    "capture exporter unfinished: publication completed after cancellation",
                )?;
                state.acknowledged = true;
                Ok(())
            }
        }

        type ExportThread = JoinHandle<Result<()>>;
        pub(super) struct CaptureExporterTask {
            control: Arc<CaptureExportControl>,
            thread: Arc<Mutex<Option<ExportThread>>>,
            hooks: Arc<dyn NativeCaptureExportHooks>,
        }
        impl CaptureExporterTask {
            pub(super) fn install(
                port: &mut NativeCapturePort,
                receiver: NativeCaptureReceiver,
                config: NativeCaptureConfig,
                limits: NativeCaptureLimits,
                hooks: Arc<dyn NativeCaptureExportHooks>,
            ) -> Self {
                let control = Arc::new(CaptureExportControl::new(receiver.state.clone()));
                let thread = Arc::new(Mutex::new(None));
                let worker_control = control.clone();
                let worker_thread = thread.clone();
                let worker_hooks = hooks.clone();
                port.export_start = Some(Box::new(move || {
                    let worker = std::thread::Builder::new()
                        .name("cafleet-capture-export".into())
                        .spawn(move || {
                            export_loop(
                                receiver,
                                &config,
                                &limits,
                                &worker_control,
                                worker_hooks.as_ref(),
                            )
                        })
                        .map_err(io_error)?;
                    let mut handle = worker_thread.lock().expect("capture exporter thread");
                    ensure(handle.is_none(), "capture exporter starts once")?;
                    *handle = Some(worker);
                    Ok(())
                }));
                Self {
                    control,
                    thread,
                    hooks,
                }
            }
            pub(super) fn finish(&self, service: &Result<i32>) -> Result<()> {
                let until = self.control.cancel();
                self.hooks.service_returned(until, service);
                let thread = self.thread.lock().expect("capture exporter thread").take();
                let Some(thread) = thread else {
                    return Ok(());
                };
                while !thread.is_finished() {
                    let Some(until) = until else {
                        break;
                    };
                    let remaining = until.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2).min(remaining));
                }
                if !thread.is_finished() {
                    let acknowledged = self
                        .control
                        .authority
                        .lock()
                        .expect("publication authority")
                        .acknowledged;
                    return Err(error(format!(
                        "capture exporter unfinished; acknowledged publication={acknowledged}; an in-flight operation may outlive helper return"
                    )));
                }
                thread
                    .join()
                    .map_err(|_| error("capture exporter panicked"))?
            }
        }
        impl Drop for CaptureExporterTask {
            fn drop(&mut self) {
                self.control.cancel();
            }
        }
        fn export_loop(
            receiver: NativeCaptureReceiver,
            config: &NativeCaptureConfig,
            limits: &NativeCaptureLimits,
            control: &CaptureExportControl,
            hooks: &dyn NativeCaptureExportHooks,
        ) -> Result<()> {
            let identity = receiver
                .finalized_identity()
                .expect("exporter starts after finalized source");
            let mut reader =
                NativeCaptureArmReader::new(&config.arm_path, identity, limits.clone())?;
            let mut accepted = None;
            let mut published = false;
            loop {
                if control.cancelled() {
                    return Ok(());
                }
                if let Some(until) = accepted {
                    if !published {
                        if let Some(lease) = receiver.try_take()? {
                            publish_native_capture_with_control(
                                &config.output_directory,
                                &lease,
                                limits,
                                receiver.publication_deadline(until),
                                Some(control),
                                hooks,
                            )?;
                            published = true;
                        }
                    } else if config.arm_path.try_exists().map_err(io_error)? {
                        return Err(error("one-shot capture arm replaced after completion"));
                    }
                } else if let Some(request) = reader.poll(Instant::now())? {
                    let until = request.deadline;
                    receiver.request(request)?;
                    accepted = Some(until);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        pub(super) fn publication_permit<'a>(
            control: Option<&'a CaptureExportControl>,
            hooks: &dyn NativeCaptureExportHooks,
            boundary: NativeCaptureExportBoundary,
            until: Instant,
        ) -> Result<Option<PublicationPermit<'a>>> {
            let current = control.map_or(until, |control| control.deadline(until));
            hooks.before(boundary, current)?;
            super::files::operation_permit(control, until)
        }

        pub(super) mod tests {
            use std::fs::File;
            use std::io::{ErrorKind, Read, Write};
            use std::path::Path;
            use std::result::Result;

            use serde_json::{Value, json};

            use super::*;
            use crate::broker::runtime::{self as storage, *};
            use crate::digest::sha256_hex;
            use crate::runtime::runner::service::test_support::observation_services::{
                capture, private_write,
            };
            use crate::runtime::runner::service::test_support::*;
            use crate::runtime::runner::test_support::*;
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            use std::path::PathBuf;
            use std::sync::{Arc, Mutex};

            struct ServiceReturn {
                at: Instant,
                deadline: Option<Instant>,
                status: Option<i32>,
                error: Option<String>,
            }

            struct ExportHooks {
                directory: PathBuf,
                stall: Option<NativeCaptureExportBoundary>,
                returned: Mutex<Option<ServiceReturn>>,
            }

            fn write_record(path: &Path, value: &Value) {
                let pending = path.with_extension("pending");
                private_write(&pending, &serde_json::to_vec(value).unwrap());
                std::fs::rename(pending, path).unwrap();
            }

            impl NativeCaptureExportHooks for ExportHooks {
                fn service_returned(
                    &self,
                    storage_deadline: Option<Instant>,
                    result: &Result<i32, crate::error::CafleetError>,
                ) {
                    let at = Instant::now();
                    let (status, error) = match result {
                        Ok(status) => (Some(*status), None),
                        Err(cause) => (None, Some(cause.message().into_owned())),
                    };
                    let mut observed = self.returned.lock().unwrap();
                    assert!(observed.is_none(), "one actual service-return notification");
                    *observed = Some(ServiceReturn {
                        at,
                        deadline: storage_deadline,
                        status,
                        error,
                    });
                }
                fn before(
                    &self,
                    boundary: NativeCaptureExportBoundary,
                    deadline: Instant,
                ) -> Result<(), crate::error::CafleetError> {
                    private_write(
                &self.directory.join(format!("export-entered-{boundary:?}")),
                &serde_json::to_vec(&json!({
                    "boundary": format!("{boundary:?}"),
                    "remaining_ms": deadline.saturating_duration_since(Instant::now()).as_millis(),
                }))
                .unwrap(),
            );
                    if self.stall == Some(boundary) {
                        let emergency = Instant::now() + Duration::from_secs(10);
                        while !self
                            .directory
                            .join("export-release")
                            .try_exists()
                            .map_err(io_error)?
                        {
                            ensure(
                                Instant::now() < emergency,
                                "owned export stall exceeded its emergency bound",
                            )?;
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                    Ok(())
                }
            }

            impl Drop for ExportHooks {
                fn drop(&mut self) {
                    if let Err(cause) = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(self.directory.join("export-hooks-dropped"))
                    {
                        eprintln!("owned exporter completion marker failed: {cause}");
                    }
                }
            }

            pub(crate) fn runner_collector_export_child_path() -> &'static str {
                test_path!(runner_collector_export_child)
            }

            #[test]
            #[ignore = "Owned synthetic exporter fixture selects this private entry"]
            fn runner_collector_export_child() {
                let path = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
                    .expect("owned exporter configuration");
                let path = Path::new(&path);
                let directory = path.parent().expect("owned exporter directory");
                let plan: Value = serde_json::from_slice(
                    &std::fs::read(directory.join("export-test-plan.json")).unwrap(),
                )
                .unwrap();
                let stall = match plan["stall"].as_str() {
                    Some("encode") => Some(NativeCaptureExportBoundary::Encode),
                    Some("manifest") => Some(NativeCaptureExportBoundary::ManifestPublish),
                    None if plan["stall"].is_null() => None,
                    other => panic!("owned export stall selection required: {other:?}"),
                };
                let hooks = Arc::new(ExportHooks {
                    directory: directory.to_path_buf(),
                    stall,
                    returned: Mutex::new(None),
                });
                let begin = Instant::now();
                let result = run_native_capture_verification_with_export_hooks(path, hooks.clone());
                let completed = Instant::now();
                let service = match hooks.returned.lock().unwrap().as_ref() {
                    Some(observed) => json!({
                        "returned_ns":observed.at.duration_since(begin).as_nanos(),
                        "storage_deadline_ns":observed.deadline.map(|deadline|deadline.duration_since(begin).as_nanos()),
                        "deadline_exhausted":observed.deadline.map(|deadline|observed.at >= deadline),
                        "status":observed.status,
                        "error":observed.error,
                        "completion_after_service_ns":completed.duration_since(observed.at).as_nanos(),
                    }),
                    None => Value::Null,
                };
                let mut record = match &result {
                    Ok(status) => {
                        json!({"result":"success", "status":status, "elapsed_ms":begin.elapsed().as_millis()})
                    }
                    Err(cause) => {
                        json!({"result":"error", "message":cause.message(), "elapsed_ms":begin.elapsed().as_millis()})
                    }
                };
                record["completed_ns"] = json!(completed.duration_since(begin).as_nanos());
                record["service"] = service;
                write_record(&directory.join("export-helper-returned.json"), &record);
                drop(hooks);
                let until = Instant::now() + Duration::from_secs(10);
                while !directory.join("export-hooks-dropped").exists() {
                    assert!(
                        Instant::now() < until,
                        "owned exporter did not release its final hook reference"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                if plan["startup_refusal"] == true {
                    assert!(
                        result.is_err(),
                        "invalid producer fails before native startup"
                    );
                    std::process::exit(1);
                }
            }

            struct ReleaseExport {
                path: PathBuf,
                released: bool,
            }
            impl ReleaseExport {
                fn new(fixture: &ServiceFixture) -> Self {
                    Self {
                        path: fixture.fixture._dir.path().join("export-release"),
                        released: false,
                    }
                }
                fn release(&mut self) {
                    private_write(&self.path, b"owned exporter release after helper return");
                    self.released = true;
                }
            }
            impl Drop for ReleaseExport {
                fn drop(&mut self) {
                    if !self.released
                        && let Err(cause) = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(&self.path)
                    {
                        eprintln!("owned export emergency release failed: {cause}");
                    }
                }
            }

            struct StorageBlocker(rusqlite::Connection);
            impl StorageBlocker {
                fn new(fixture: &ServiceFixture) -> Self {
                    let connection = crate::db::connect(&format!(
                        "sqlite:///{}",
                        fixture.fixture.conn.path().unwrap()
                    ))
                    .unwrap();
                    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
                    Self(connection)
                }
            }
            impl Drop for StorageBlocker {
                fn drop(&mut self) {
                    if let Err(cause) = self.0.execute_batch("ROLLBACK") {
                        eprintln!("owned storage lock release failed: {cause}");
                    }
                }
            }

            fn configure(
                fixture: &mut Fixture,
                spec: &mut SpawnSpec,
                stall: Option<&str>,
                startup_refusal: bool,
            ) {
                capture(fixture, spec);
                private_write(
                    &fixture._dir.path().join("collector-review-entry"),
                    b"export",
                );
                private_write(
                    &fixture._dir.path().join("export-test-plan.json"),
                    &serde_json::to_vec(&json!({"stall":stall,"startup_refusal":startup_refusal}))
                        .unwrap(),
                );
            }

            fn wait_path(path: &Path, until: Instant) {
                crate::runtime::runner::writer::test_support::wait_until_within(
                    until.saturating_duration_since(Instant::now()),
                    || path.exists(),
                );
            }

            fn arm_capture(fixture: &ServiceFixture, started: &Started) {
                private_write(
                    &fixture.fixture._dir.path().join("capture-arm.json"),
                    &serde_json::to_vec(&NativeCaptureArm::for_identity(
                        "export-pair",
                        &started.identity,
                        30000,
                    ))
                    .unwrap(),
                );
            }

            fn helper_record(fixture: &ServiceFixture) -> Value {
                serde_json::from_slice(
                    &std::fs::read(
                        fixture
                            .fixture
                            ._dir
                            .path()
                            .join("export-helper-returned.json"),
                    )
                    .unwrap(),
                )
                .unwrap()
            }

            fn assert_unfinished(record: &Value) {
                assert_eq!(record["result"], "error", "{record}");
                let message = record["message"]
                    .as_str()
                    .expect("explicit unfinished exporter error")
                    .to_ascii_lowercase();
                assert!(
                    message.contains("unfinished")
                        && (message.contains("export") || message.contains("capture")),
                    "{record}"
                );
            }

            fn assert_original_completion_bound(record: &Value) {
                let service = record["service"]
                    .as_object()
                    .expect("actual service-return observation");
                let deadline = service["storage_deadline_ns"]
                    .as_u64()
                    .expect("original service storage deadline");
                let completed = record["completed_ns"].as_u64().unwrap();
                assert!(
                    completed <= deadline + 100_000_000,
                    "helper completion stays within original deadline plus 100 ms scheduling allowance: {record}"
                );
            }

            fn assert_expired_service_failure(record: &Value) {
                assert_original_completion_bound(record);
                let service = &record["service"];
                assert_eq!(
                    service["deadline_exhausted"], true,
                    "actual service return exhausts its own storage deadline: {record}"
                );
                assert_eq!(
                    service["status"], 1,
                    "native zero plus blocked storage yields actual incomplete-service status: {record}"
                );
                assert!(
                    service["error"].is_null(),
                    "record the uncombined actual Ok(1) service result: {record}"
                );
                assert!(
                    service["returned_ns"].as_u64().unwrap()
                        >= service["storage_deadline_ns"].as_u64().unwrap()
                );
                assert!(
                    service["completion_after_service_ns"].as_u64().unwrap() <= 100_000_000,
                    "expired original deadline permits only 100 ms scheduling allowance after actual service return: {record}"
                );
                assert_unfinished(record);
                let message = record["message"].as_str().unwrap().to_ascii_lowercase();
                assert!(
                    message.lines().any(|line| line.contains("service")
                        && line
                            .split(|character: char| !character.is_ascii_digit())
                            .any(|number| number == "1")),
                    "combined helper result retains the independently observed service status alongside unfinished export: {record}"
                );
            }

            fn final_manifest(fixture: &ServiceFixture) -> PathBuf {
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/export-pair.manifest.json")
            }

            fn artifact_path(fixture: &ServiceFixture) -> PathBuf {
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/export-pair.complete.json")
            }

            struct OutputFile {
                path: PathBuf,
                device: u64,
                inode: u64,
                bytes: Vec<u8>,
                _identity_pin: File,
            }
            impl OutputFile {
                fn capture(path: PathBuf) -> Self {
                    let mut file = std::fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(nix::libc::O_NOFOLLOW)
                        .open(&path)
                        .unwrap();
                    let metadata = file.metadata().unwrap();
                    assert!(
                        metadata.is_file(),
                        "owned publication snapshot is a regular file"
                    );
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).unwrap();
                    Self {
                        path,
                        device: metadata.dev(),
                        inode: metadata.ino(),
                        bytes,
                        _identity_pin: file,
                    }
                }
                fn assert_unchanged(&self) {
                    let metadata = std::fs::symlink_metadata(&self.path).unwrap();
                    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
                    assert_eq!(
                        (metadata.dev(), metadata.ino()),
                        (self.device, self.inode),
                        "preserved identity: {}",
                        self.path.display()
                    );
                    assert_eq!(
                        std::fs::read(&self.path).unwrap(),
                        self.bytes,
                        "preserved bytes: {}",
                        self.path.display()
                    );
                }
            }

            fn seed_unrelated(fixture: &ServiceFixture) -> OutputFile {
                let path = fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/unrelated-output.txt");
                private_write(&path, b"pre-existing unrelated output identity");
                OutputFile::capture(path)
            }

            fn output_paths(fixture: &ServiceFixture) -> std::collections::BTreeSet<PathBuf> {
                std::fs::read_dir(fixture.fixture._dir.path().join("capture-output"))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect()
            }

            fn temporary_outputs(
                fixture: &ServiceFixture,
                preserved: &[&OutputFile],
            ) -> Vec<OutputFile> {
                let kept: std::collections::BTreeSet<_> =
                    preserved.iter().map(|file| file.path.clone()).collect();
                output_paths(fixture)
                    .difference(&kept)
                    .cloned()
                    .map(OutputFile::capture)
                    .collect()
            }

            fn assert_removed(temporaries: &[OutputFile]) {
                for file in temporaries {
                    assert!(
                        matches!(std::fs::symlink_metadata(&file.path),Err(cause) if cause.kind() == ErrorKind::NotFound),
                        "worker removes its still-matching temporary before fixture cleanup: {}",
                        file.path.display()
                    );
                }
            }

            fn assert_only_preserved(fixture: &ServiceFixture, preserved: &[&OutputFile]) {
                for file in preserved {
                    file.assert_unchanged();
                }
                let kept: std::collections::BTreeSet<_> =
                    preserved.iter().map(|file| file.path.clone()).collect();
                assert_eq!(
                    output_paths(fixture),
                    kept,
                    "worker leaves no task-created temporary or late output before TempDir cleanup"
                );
            }

            #[test]
            fn runner_collector_encode_and_prepermit_manifest_stalls_return_unfinished_and_revoke_late_publication()
             {
                for (selected, boundary) in [("encode", "Encode"), ("manifest", "ManifestPublish")]
                {
                    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                        configure(fixture, spec, Some(selected), false)
                    });
                    let mut release = ReleaseExport::new(&fixture);
                    let unrelated = seed_unrelated(&fixture);
                    let started = fixture.start_running();
                    arm_capture(&fixture, &started);
                    wait_path(
                        &fixture
                            .fixture
                            ._dir
                            .path()
                            .join(format!("export-entered-{boundary}")),
                        Instant::now() + Duration::from_secs(3),
                    );
                    assert!(!final_manifest(&fixture).exists());
                    let published_artifact = (selected == "manifest")
                        .then(|| OutputFile::capture(artifact_path(&fixture)));
                    let mut preserved = vec![&unrelated];
                    if let Some(artifact) = &published_artifact {
                        preserved.push(artifact);
                    }
                    let temporaries = temporary_outputs(&fixture, &preserved);
                    assert!(started.child.matches().unwrap());
                    fixture.assert_manual_io();
                    let shutdown = Instant::now();
                    fixture.master.write_all(b"finish\r").unwrap();
                    wait_path(
                        &fixture
                            .fixture
                            ._dir
                            .path()
                            .join("export-helper-returned.json"),
                        shutdown + Duration::from_secs(5),
                    );
                    assert_unfinished(&helper_record(&fixture));
                    assert_original_completion_bound(&helper_record(&fixture));
                    assert!(!started.child.matches().unwrap());
                    assert!(!final_manifest(&fixture).exists());
                    release.release();
                    wait_path(
                        &fixture.fixture._dir.path().join("export-hooks-dropped"),
                        Instant::now() + Duration::from_secs(2),
                    );
                    assert_eq!(fixture.wait().code(), Some(0));
                    assert_removed(&temporaries);
                    assert_only_preserved(&fixture, &preserved);
                    assert!(!final_manifest(&fixture).exists());
                    if selected == "encode" {
                        assert!(
                            !fixture
                                .fixture
                                ._dir
                                .path()
                                .join("export-entered-ArtifactPublish")
                                .exists()
                        );
                    }
                    let run =
                        storage::get_run(&fixture.fixture.conn, started.identity.run_id).unwrap();
                    assert_eq!(run.status, RunStatus::Exited);
                    assert_eq!(run.exit_code, Some(0));
                }
            }

            #[test]
            fn runner_collector_confirmed_pair_remains_accepted_while_native_work_continues_and_after_exit()
             {
                let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                    configure(fixture, spec, None, false)
                });
                let started = fixture.start_running();
                arm_capture(&fixture, &started);
                let manifest_path = final_manifest(&fixture);
                wait_path(&manifest_path, Instant::now() + Duration::from_secs(3));
                let original_manifest = std::fs::read(&manifest_path).unwrap();
                let accepted_manifest = OutputFile::capture(manifest_path.clone());
                let manifest: Value = serde_json::from_slice(&original_manifest).unwrap();
                let artifact_path = fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/export-pair.complete.json");
                let original_artifact = std::fs::read(&artifact_path).unwrap();
                let accepted_artifact = OutputFile::capture(artifact_path.clone());
                assert_eq!(manifest["publication_status"], "complete");
                assert_eq!(manifest["artifact_sha256"], sha256_hex(&original_artifact));
                assert_eq!(manifest["artifact_bytes"], original_artifact.len() as u64);
                assert!(started.child.matches().unwrap());
                assert!(
                    !fixture
                        .fixture
                        ._dir
                        .path()
                        .join("export-helper-returned.json")
                        .exists()
                );
                fixture.health();
                fixture.assert_manual_io();
                fixture.finish();
                let record = helper_record(&fixture);
                assert_eq!(record["result"], "success", "{record}");
                assert_eq!(record["status"], 0);
                assert_eq!(std::fs::read(manifest_path).unwrap(), original_manifest);
                assert_eq!(std::fs::read(artifact_path).unwrap(), original_artifact);
                accepted_manifest.assert_unchanged();
                accepted_artifact.assert_unchanged();
            }

            #[test]
            fn runner_collector_cancelled_manifest_worker_preserves_foreign_identities_and_cleans_its_matching_temporaries()
             {
                let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                    configure(fixture, spec, Some("manifest"), false)
                });
                let mut release = ReleaseExport::new(&fixture);
                let unrelated = seed_unrelated(&fixture);
                let started = fixture.start_running();
                arm_capture(&fixture, &started);
                wait_path(
                    &fixture
                        .fixture
                        ._dir
                        .path()
                        .join("export-entered-ManifestPublish"),
                    Instant::now() + Duration::from_secs(3),
                );
                assert!(!final_manifest(&fixture).exists());
                let artifact = OutputFile::capture(artifact_path(&fixture));
                let mut temporaries = temporary_outputs(&fixture, &[&unrelated, &artifact]);
                let shutdown = Instant::now();
                fixture.master.write_all(b"finish\r").unwrap();
                wait_path(
                    &fixture
                        .fixture
                        ._dir
                        .path()
                        .join("export-helper-returned.json"),
                    shutdown + Duration::from_secs(5),
                );
                let original_result = helper_record(&fixture);
                assert_unfinished(&original_result);
                assert_original_completion_bound(&original_result);
                assert!(!started.child.matches().unwrap());

                let replaceable =
                    temporaries.iter().position(|file| {
                        match std::fs::symlink_metadata(&file.path) {
                            Ok(metadata) => {
                                (metadata.dev(), metadata.ino()) == (file.device, file.inode)
                            }
                            Err(cause) if cause.kind() == ErrorKind::NotFound => false,
                            Err(cause) => panic!("owned temporary inspection failed: {cause}"),
                        }
                    });
                let (foreign_temporary, displaced) = if let Some(index) = replaceable {
                    let original = temporaries.remove(index);
                    let displaced_path = fixture
                        .fixture
                        ._dir
                        .path()
                        .join("displaced-exporter-temporary");
                    std::fs::rename(&original.path, &displaced_path).unwrap();
                    let displaced = OutputFile::capture(displaced_path);
                    assert_eq!(
                        (displaced.device, displaced.inode),
                        (original.device, original.inode)
                    );
                    private_write(
                        &original.path,
                        b"foreign replacement at an exporter temporary name",
                    );
                    let replacement = OutputFile::capture(original.path.clone());
                    assert_ne!(
                        (replacement.device, replacement.inode),
                        (original.device, original.inode)
                    );
                    (replacement, Some((original, displaced)))
                } else {
                    let path = final_manifest(&fixture).with_extension("provisional");
                    assert!(
                        !path.exists(),
                        "no current owned temporary occupies the collision name"
                    );
                    private_write(&path, b"foreign file at the publisher temporary name");
                    (OutputFile::capture(path), None)
                };
                let collision_path = final_manifest(&fixture);
                private_write(
                    &collision_path,
                    b"pre-existing collision output before revoked worker resumes",
                );
                let collision = OutputFile::capture(collision_path);
                let unrelated_temporary_path = fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/foreign.provisional");
                private_write(&unrelated_temporary_path, b"unrelated provisional identity");
                let unrelated_temporary = OutputFile::capture(unrelated_temporary_path);

                release.release();
                wait_path(
                    &fixture.fixture._dir.path().join("export-hooks-dropped"),
                    Instant::now() + Duration::from_secs(2),
                );
                assert_eq!(fixture.wait().code(), Some(0));
                assert_removed(&temporaries);
                assert_only_preserved(
                    &fixture,
                    &[
                        &unrelated,
                        &artifact,
                        &foreign_temporary,
                        &collision,
                        &unrelated_temporary,
                    ],
                );
                assert_eq!(
                    helper_record(&fixture),
                    original_result,
                    "later worker completion does not acknowledge the unfinished invocation"
                );
                if let Some((original, displaced)) = displaced {
                    displaced.assert_unchanged();
                    assert_eq!(displaced.bytes, original.bytes);
                    std::fs::remove_file(&displaced.path).unwrap();
                }
            }

            #[test]
            fn runner_collector_startup_failure_has_no_export_wait_or_publication() {
                let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
                    configure(fixture, spec, None, true);
                    let path = fixture._dir.path().join("capture-producer.json");
                    let mut producer: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    producer["collector_version"] = json!("invalid-owned-version");
                    std::fs::write(path, serde_json::to_vec(&producer).unwrap()).unwrap();
                });
                let record = helper_record(&fixture);
                assert_eq!(record["result"], "error", "{record}");
                assert!(
                    record["service"].is_null(),
                    "pre-worker refusal invokes no actual service-return callback: {record}"
                );
                assert!(
                    record["elapsed_ms"].as_u64().unwrap() < 1000,
                    "absent service deadline grants no new exporter budget: {record}"
                );
                assert!(!record["message"].as_str().unwrap().contains("unfinished"));
                for boundary in ["Encode", "ArtifactPublish", "ManifestPublish"] {
                    assert!(
                        !fixture
                            .fixture
                            ._dir
                            .path()
                            .join(format!("export-entered-{boundary}"))
                            .exists()
                    );
                }
                assert!(!final_manifest(&fixture).exists());
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert!(run.child_pid.is_none());
            }

            #[test]
            fn runner_collector_exhausted_service_storage_budget_adds_no_fresh_exporter_wait() {
                let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                    configure(fixture, spec, Some("encode"), false)
                });
                let mut release = ReleaseExport::new(&fixture);
                let unrelated = seed_unrelated(&fixture);
                let started = fixture.start_running();
                arm_capture(&fixture, &started);
                wait_path(
                    &fixture.fixture._dir.path().join("export-entered-Encode"),
                    Instant::now() + Duration::from_secs(3),
                );
                let temporaries = temporary_outputs(&fixture, &[&unrelated]);
                let blocker = StorageBlocker::new(&fixture);
                let shutdown = Instant::now();
                fixture.master.write_all(b"finish\r").unwrap();
                wait_path(
                    &fixture
                        .fixture
                        ._dir
                        .path()
                        .join("export-helper-returned.json"),
                    shutdown + Duration::from_secs(5),
                );
                assert_expired_service_failure(&helper_record(&fixture));
                assert!(!started.child.matches().unwrap());
                assert!(!final_manifest(&fixture).exists());
                drop(blocker);
                release.release();
                wait_path(
                    &fixture.fixture._dir.path().join("export-hooks-dropped"),
                    Instant::now() + Duration::from_secs(2),
                );
                assert_eq!(fixture.wait().code(), Some(0));
                assert_removed(&temporaries);
                assert_only_preserved(&fixture, &[&unrelated]);
                assert!(
                    !fixture
                        .fixture
                        ._dir
                        .path()
                        .join("export-entered-ArtifactPublish")
                        .exists()
                );
                assert!(!final_manifest(&fixture).exists());
            }
        }
    }

    mod files {
        use super::*;
        use std::fs::{File, OpenOptions};
        use std::io::{Read, Write};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        use std::path::{Path, PathBuf};

        pub(super) fn file_digest(path: &Path) -> Result<String> {
            crate::digest::sha256_file_hex(path).map_err(io_error)
        }
        fn private_directory(path: &Path) -> Result<(u64, u64)> {
            ensure(path.is_absolute(), "private directory must be absolute")?;
            let meta = crate::runtime::runner::artifacts::private_metadata(
                path,
                nix::unistd::geteuid().as_raw(),
                0o700,
                true,
            )?;
            Ok((meta.dev(), meta.ino()))
        }
        pub(super) fn read_private(path: &Path, limit: usize) -> Result<Vec<u8>> {
            ensure(path.is_absolute(), "private file path must be absolute")?;
            validate_private_file(path, nix::unistd::geteuid().as_raw(), 0o600)?;
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(nix::libc::O_NOFOLLOW)
                .open(path)
                .map_err(io_error)?;
            let meta = file.metadata().map_err(io_error)?;
            let named = std::fs::symlink_metadata(path).map_err(io_error)?;
            ensure(
                meta.dev() == named.dev()
                    && meta.ino() == named.ino()
                    && meta.len() <= limit as u64,
                "private file identity/size mismatch",
            )?;
            let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).map_err(io_error)?);
            Read::by_ref(&mut file)
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(io_error)?;
            ensure(bytes.len() <= limit, "private file exceeds budget")?;
            Ok(bytes)
        }
        fn reference(value: &Value) -> Result<&str> {
            let text = value
                .as_str()
                .ok_or_else(|| error("evidence reference required"))?;
            ensure(
                !text.trim().is_empty()
                    && text.len() <= 2048
                    && !text.chars().any(|c| c.is_control()),
                "invalid evidence reference",
            )?;
            Ok(text)
        }
        fn lower_digest(value: &Value) -> Result<&str> {
            let text = value
                .as_str()
                .ok_or_else(|| error("digest string required"))?;
            ensure(
                text.len() == 64
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "lowercase digest required",
            )?;
            Ok(text)
        }
        fn exact(value: &Value, keys: &[&str]) -> Result<()> {
            let object = value
                .as_object()
                .ok_or_else(|| error("capture object required"))?;
            ensure(
                object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)),
                "capture object fields mismatch",
            )
        }
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct NativeCaptureConfig {
            pub(crate) schema_version: u32,
            pub(crate) database_url: String,
            pub(crate) descriptor_path: PathBuf,
            pub(crate) expected_run_id: i64,
            pub(crate) expected_launch_nonce: String,
            pub(crate) arm_path: PathBuf,
            pub(crate) output_directory: PathBuf,
            pub(crate) producer_manifest_path: PathBuf,
            pub(crate) capture_policy_path: PathBuf,
        }
        pub(crate) fn read_native_capture_config(
            path: &Path,
            limits: &NativeCaptureLimits,
        ) -> Result<NativeCaptureConfig> {
            limits.validate()?;
            let value: NativeCaptureConfig =
                serde_json::from_slice(&read_private(path, limits.max_config_bytes)?)
                    .map_err(io_error)?;
            ensure(
                value.schema_version == 1 && value.expected_run_id > 0,
                "invalid capture config schema/run",
            )?;
            lower_digest(&json!(value.expected_launch_nonce))?;
            private_directory(&value.output_directory)?;
            private_directory(
                value
                    .arm_path
                    .parent()
                    .ok_or_else(|| error("arm parent required"))?,
            )?;
            ensure(value.arm_path.is_absolute(), "absolute arm path required")?;
            for path in [
                &value.descriptor_path,
                &value.producer_manifest_path,
                &value.capture_policy_path,
            ] {
                validate_private_file(path, nix::unistd::geteuid().as_raw(), 0o600)?;
                ensure(
                    path.is_absolute(),
                    "absolute capture evidence path required",
                )?;
            }
            let database = value
                .database_url
                .strip_prefix("sqlite:///")
                .ok_or_else(|| error("capture database URL required"))?;
            ensure(
                Path::new(database).is_absolute() && Path::new(database).is_file(),
                "capture database file required",
            )?;
            Ok(value)
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ProducerNativeExecutable {
            path: String,
            sha256: String,
            version: Value,
            version_evidence_ref: String,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ProducerDocument {
            schema_version: u32,
            kind: String,
            source_manifest_ref: String,
            source_manifest_sha256: String,
            libtest_executable_path: String,
            libtest_executable_sha256: String,
            parser_version: String,
            collector_version: String,
            catalog_origin: String,
            native_executable: ProducerNativeExecutable,
            launch_evidence_ref: String,
            environment_evidence_ref: String,
        }
        pub(crate) struct NativeCaptureProducer {
            pub(super) value: Value,
            pub(super) reference: PathBuf,
        }
        pub(crate) fn read_native_capture_producer(
            path: &Path,
            limits: &NativeCaptureLimits,
        ) -> Result<NativeCaptureProducer> {
            let document: ProducerDocument =
                serde_json::from_slice(&read_private(path, limits.max_producer_bytes)?)
                    .map_err(io_error)?;
            let value = serde_json::to_value(document).map_err(io_error)?;
            exact(
                &value,
                &[
                    "schema_version",
                    "kind",
                    "source_manifest_ref",
                    "source_manifest_sha256",
                    "libtest_executable_path",
                    "libtest_executable_sha256",
                    "parser_version",
                    "collector_version",
                    "catalog_origin",
                    "native_executable",
                    "launch_evidence_ref",
                    "environment_evidence_ref",
                ],
            )?;
            ensure(
                value["schema_version"] == 1
                    && matches!(
                        value["kind"].as_str(),
                        Some("synthetic" | "verification_libtest")
                    )
                    && matches!(
                        value["catalog_origin"].as_str(),
                        Some("synthetic" | "bundled_unknown")
                    )
                    && value["parser_version"] == "0.18.1"
                    && value["collector_version"] == "1",
                "invalid capture producer identity",
            )?;
            for key in [
                "source_manifest_ref",
                "launch_evidence_ref",
                "environment_evidence_ref",
            ] {
                reference(&value[key])?;
            }
            lower_digest(&value["source_manifest_sha256"])?;
            let current = std::env::current_exe().map_err(io_error)?;
            let selected = Path::new(reference(&value["libtest_executable_path"])?);
            ensure(
                selected.is_absolute()
                    && std::fs::canonicalize(selected).map_err(io_error)?
                        == std::fs::canonicalize(current).map_err(io_error)?
                    && lower_digest(&value["libtest_executable_sha256"])? == file_digest(selected)?,
                "capture libtest provenance mismatch",
            )?;
            let native = &value["native_executable"];
            exact(
                native,
                &["path", "sha256", "version", "version_evidence_ref"],
            )?;
            ensure(
                Path::new(reference(&native["path"])?).is_absolute(),
                "absolute native executable required",
            )?;
            lower_digest(&native["sha256"])?;
            reference(&native["version_evidence_ref"])?;
            if !native["version"].is_null() {
                reference(&native["version"])?;
            }
            let source = Path::new(reference(&value["source_manifest_ref"])?);
            if source.is_absolute() {
                ensure(
                    file_digest(source)? == lower_digest(&value["source_manifest_sha256"])?,
                    "capture source manifest digest mismatch",
                )?;
            }
            Ok(NativeCaptureProducer {
                value,
                reference: path.into(),
            })
        }
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct NativeCaptureArm {
            pub(crate) schema_version: u32,
            pub(crate) capture_id: String,
            pub(crate) run_id: i64,
            pub(crate) owner_epoch: i64,
            pub(crate) minimum_interval_ms: u64,
            pub(crate) timeout_ms: u64,
        }
        impl NativeCaptureArm {
            /// A schema-1 arm for `identity` with the standard 100 ms minimum interval.
            pub(crate) fn for_identity(
                capture_id: &str,
                identity: &ControlIdentity,
                timeout_ms: u64,
            ) -> Self {
                Self {
                    schema_version: 1,
                    capture_id: capture_id.into(),
                    run_id: identity.run_id,
                    owner_epoch: identity.owner_epoch,
                    minimum_interval_ms: 100,
                    timeout_ms,
                }
            }
            pub(crate) fn to_request(
                &self,
                identity: &ControlIdentity,
                now: Instant,
                limits: &NativeCaptureLimits,
            ) -> Result<NativeCaptureRequest> {
                limits.validate()?;
                validate_capture_id(&self.capture_id)?;
                ensure(
                    self.schema_version == 1
                        && self.run_id > 0
                        && self.owner_epoch > 0
                        && self.run_id == identity.run_id
                        && self.owner_epoch == identity.owner_epoch
                        && self.minimum_interval_ms >= 100
                        && Duration::from_millis(self.minimum_interval_ms) <= limits.max_interval
                        && self.timeout_ms > self.minimum_interval_ms
                        && Duration::from_millis(self.timeout_ms) <= limits.max_request_duration,
                    "invalid one-shot capture arm",
                )?;
                Ok(NativeCaptureRequest {
                    capture_id: self.capture_id.clone(),
                    run_id: self.run_id,
                    owner_epoch: self.owner_epoch,
                    minimum_interval: Duration::from_millis(self.minimum_interval_ms),
                    deadline: now
                        .checked_add(Duration::from_millis(self.timeout_ms))
                        .ok_or_else(|| error("capture deadline overflow"))?,
                })
            }
        }
        pub(crate) struct NativeCaptureArmReader {
            path: PathBuf,
            parent: (u64, u64),
            identity: ControlIdentity,
            limits: NativeCaptureLimits,
            used: bool,
        }
        impl NativeCaptureArmReader {
            pub(crate) fn new(
                path: &Path,
                identity: ControlIdentity,
                limits: NativeCaptureLimits,
            ) -> Result<Self> {
                limits.validate()?;
                ensure(path.is_absolute(), "absolute arm path required")?;
                let parent =
                    private_directory(path.parent().ok_or_else(|| error("arm parent required"))?)?;
                Ok(Self {
                    path: path.into(),
                    parent,
                    identity,
                    limits,
                    used: false,
                })
            }
            pub(crate) fn poll(&mut self, now: Instant) -> Result<Option<NativeCaptureRequest>> {
                ensure(
                    private_directory(self.path.parent().expect("validated arm parent"))?
                        == self.parent,
                    "arm parent replaced",
                )?;
                ensure(!self.used, "one-shot arm already consumed")?;
                let meta = match std::fs::symlink_metadata(&self.path) {
                    Ok(meta) => meta,
                    Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(cause) => return Err(io_error(cause)),
                };
                self.used = true;
                validate_private_file(&self.path, nix::unistd::geteuid().as_raw(), 0o600)?;
                let claimed = self.path.with_extension("claimed");
                std::fs::hard_link(&self.path, &claimed).map_err(io_error)?;
                let owned = std::fs::symlink_metadata(&claimed).map_err(io_error)?;
                ensure(
                    owned.dev() == meta.dev() && owned.ino() == meta.ino(),
                    "arm changed during claim",
                )?;
                let current = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
                ensure(
                    current.dev() == owned.dev() && current.ino() == owned.ino(),
                    "arm pathname replaced",
                )?;
                std::fs::remove_file(&self.path).map_err(io_error)?;
                let arm: NativeCaptureArm =
                    serde_json::from_slice(&read_private(&claimed, self.limits.max_arm_bytes)?)
                        .map_err(io_error)?;
                arm.to_request(&self.identity, now, &self.limits).map(Some)
            }
        }
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum CapturePublicationStatus {
            Complete,
            Incomplete,
        }
        pub(crate) struct NativeCapturePublication {
            pub(crate) artifact_path: PathBuf,
            pub(crate) manifest_path: PathBuf,
            pub(crate) artifact_sha256: String,
            pub(crate) artifact_bytes: u64,
            pub(crate) collection_status: NativeCaptureStatus,
            pub(crate) publication_status: CapturePublicationStatus,
        }
        struct PublicationTemporary {
            path: PathBuf,
            file: File,
            identity: (u64, u64),
            active: bool,
        }
        impl PublicationTemporary {
            fn create(path: &Path) -> Result<Self> {
                let path = path.with_extension("provisional");
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(nix::libc::O_NOFOLLOW)
                    .open(&path)
                    .map_err(io_error)?;
                let metadata = file.metadata().map_err(io_error)?;
                Ok(Self {
                    path,
                    file,
                    identity: (metadata.dev(), metadata.ino()),
                    active: true,
                })
            }
            fn validate(&self) -> Result<()> {
                let metadata = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
                ensure(
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && (metadata.dev(), metadata.ino()) == self.identity,
                    "capture temporary identity changed",
                )
            }
            fn cleanup(&mut self) -> Result<()> {
                if !self.active {
                    return Ok(());
                }
                self.active = false;
                match std::fs::symlink_metadata(&self.path) {
                    Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(cause) => return Err(io_error(cause)),
                    Ok(_) => self.validate()?,
                }
                std::fs::remove_file(&self.path).map_err(io_error)
            }
        }
        impl Drop for PublicationTemporary {
            fn drop(&mut self) {
                if let Err(cause) = self.cleanup() {
                    eprintln!("capture temporary cleanup: {cause}");
                }
            }
        }
        pub(super) fn operation_permit(
            control: Option<&exporter::CaptureExportControl>,
            until: Instant,
        ) -> Result<Option<exporter::PublicationPermit<'_>>> {
            match control {
                Some(control) => control.acquire(until).map(Some),
                None => {
                    check_deadline(Instant::now(), until)?;
                    Ok(None)
                }
            }
        }
        fn publish_file(
            path: &Path,
            bytes: &[u8],
            until: Instant,
            control: Option<&exporter::CaptureExportControl>,
            hooks: &dyn NativeCaptureExportHooks,
            boundary: NativeCaptureExportBoundary,
        ) -> Result<()> {
            let directory = path.parent().expect("publication directory");
            let directory_identity = private_directory(directory)?;
            let mut temporary = {
                let _permit = operation_permit(control, until)?;
                PublicationTemporary::create(path)?
            };
            let result = (|| {
                for chunk in bytes.chunks(65536) {
                    let _permit = operation_permit(control, until)?;
                    temporary.file.write_all(chunk).map_err(io_error)?;
                }
                {
                    let _permit = operation_permit(control, until)?;
                    temporary.file.sync_all().map_err(io_error)?;
                }
                hooks.before(
                    boundary,
                    control.map_or(until, |control| control.deadline(until)),
                )?;
                {
                    let _check = operation_permit(control, until)?;
                }
                ensure(
                    private_directory(directory)? == directory_identity,
                    "capture publication directory changed",
                )?;
                temporary.validate()?;
                {
                    let _permit = operation_permit(control, until)?;
                    std::fs::hard_link(&temporary.path, path).map_err(io_error)?;
                }
                temporary.cleanup()?;
                let directory = {
                    let _permit = operation_permit(control, until)?;
                    File::open(directory).map_err(io_error)?
                };
                let _permit = operation_permit(control, until)?;
                directory.sync_all().map_err(io_error)
            })();
            let cleanup = temporary.cleanup();
            CafleetError::merge_cleanup(result, cleanup)
        }
        pub(super) fn publish_private(path: &Path, bytes: &[u8], until: Instant) -> Result<()> {
            publish_file(
                path,
                bytes,
                until,
                None,
                &InertCaptureExportHooks,
                NativeCaptureExportBoundary::ArtifactPublish,
            )
        }
        pub(crate) fn publish_native_capture(
            directory: &Path,
            lease: &NativeCaptureLease,
            limits: &NativeCaptureLimits,
            until: Instant,
        ) -> Result<NativeCapturePublication> {
            publish_native_capture_with_control(
                directory,
                lease,
                limits,
                until,
                None,
                &InertCaptureExportHooks,
            )
        }
        pub(super) fn publish_native_capture_with_control(
            directory: &Path,
            lease: &NativeCaptureLease,
            limits: &NativeCaptureLimits,
            until: Instant,
            control: Option<&exporter::CaptureExportControl>,
            hooks: &dyn NativeCaptureExportHooks,
        ) -> Result<NativeCapturePublication> {
            check_deadline(Instant::now(), until)?;
            limits.validate()?;
            let directory_identity = private_directory(directory)?;
            let result = lease.result();
            validate_capture_id(&result.capture_id)?;
            let complete = result.status == NativeCaptureStatus::Complete;
            ensure(
                !complete || result.samples.len() == 2,
                "complete capture requires a pair",
            )?;
            let artifact_path = directory.join(format!(
                "{}.{}.json",
                result.capture_id,
                if complete { "complete" } else { "incomplete" }
            ));
            let manifest_path = directory.join(format!("{}.manifest.json", result.capture_id));
            ensure(
                !artifact_path.try_exists().map_err(io_error)?
                    && !manifest_path.try_exists().map_err(io_error)?,
                "capture publication collision",
            )?;
            let encoded = {
                let _permit = exporter::publication_permit(
                    control,
                    hooks,
                    NativeCaptureExportBoundary::Encode,
                    until,
                )?;
                super::encode_bounded(result, limits.max_encoded_result_bytes)?
            };
            let artifact_sha256 = crate::digest::sha256_hex(&encoded);
            let artifact_bytes = encoded.len() as u64;
            ensure(
                private_directory(directory)? == directory_identity,
                "capture publication directory changed",
            )?;
            publish_file(
                &artifact_path,
                &encoded,
                until,
                control,
                hooks,
                NativeCaptureExportBoundary::ArtifactPublish,
            )?;
            let publication_status = if complete {
                CapturePublicationStatus::Complete
            } else {
                CapturePublicationStatus::Incomplete
            };
            let manifest = json!({"schema_version":1,"capture_id":result.capture_id,"artifact_ref":artifact_path.file_name().expect("artifact name").to_string_lossy(),"artifact_sha256":artifact_sha256,"artifact_bytes":artifact_bytes,"collection_status":result.status,"publication_status":publication_status,"producer_ref":result.producer_ref,"diagnostic":result.diagnostic});
            let bytes = serde_json::to_vec(&manifest).map_err(io_error)?;
            ensure(
                private_directory(directory)? == directory_identity,
                "capture publication directory changed",
            )?;
            publish_file(
                &manifest_path,
                &bytes,
                until,
                control,
                hooks,
                NativeCaptureExportBoundary::ManifestPublish,
            )?;
            if let Some(control) = control {
                control.acknowledge()?;
            }
            Ok(NativeCapturePublication {
                artifact_path,
                manifest_path,
                artifact_sha256,
                artifact_bytes,
                collection_status: result.status,
                publication_status,
            })
        }

        mod tests {
            use serde_json::{Value, json};

            use super::super::test_support::*;
            use super::*;
            use std::os::unix::fs::{PermissionsExt, symlink};
            use std::time::{Duration, Instant};
            #[test]
            fn capture_arm_validates_schema_identity_duration_and_safe_capture_id() {
                let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let identity = ControlIdentity::from_run(&fixture.fixture.run);
                let now = Instant::now();
                let limits = NativeCaptureLimits::default();
                let valid = arm(&identity);
                let request = valid.to_request(&identity, now, &limits).unwrap();
                assert_eq!(request.deadline, now + Duration::from_millis(1000));
                assert_eq!(request.minimum_interval, Duration::from_millis(100));
                for field in [
                    "schema",
                    "run",
                    "epoch",
                    "blank",
                    "slash",
                    "control",
                    "length",
                    "interval_low",
                    "interval_high",
                    "equal_timeout",
                    "long_timeout",
                    "overflow",
                ] {
                    let mut invalid = arm(&identity);
                    match field {
                        "schema" => invalid.schema_version = 2,
                        "run" => invalid.run_id += 1,
                        "epoch" => invalid.owner_epoch += 1,
                        "blank" => invalid.capture_id = "".into(),
                        "slash" => invalid.capture_id = "a/b".into(),
                        "control" => invalid.capture_id = "a\n".into(),
                        "length" => invalid.capture_id = "a".repeat(129),
                        "interval_low" => invalid.minimum_interval_ms = 99,
                        "interval_high" => invalid.minimum_interval_ms = 5001,
                        "equal_timeout" => invalid.timeout_ms = 100,
                        "long_timeout" => invalid.timeout_ms = 30001,
                        _ => invalid.timeout_ms = u64::MAX,
                    }
                    assert!(
                        invalid.to_request(&identity, now, &limits).is_err(),
                        "{field}"
                    );
                }
                let mut value = serde_json::to_value(valid).unwrap();
                value["native_input"] = json!("forbidden private key");
                assert!(serde_json::from_value::<NativeCaptureArm>(value).is_err());
            }

            #[test]
            fn capture_arm_reader_consumes_once_and_detects_replacement_after_claim() {
                let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let identity = ControlIdentity::from_run(&fixture.fixture.run);
                let path = fixture.fixture._dir.path().join("one-shot-arm.json");
                let mut reader = NativeCaptureArmReader::new(
                    &path,
                    identity.clone(),
                    NativeCaptureLimits::default(),
                )
                .unwrap();
                assert!(reader.poll(Instant::now()).unwrap().is_none());
                private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                let request = reader.poll(Instant::now()).unwrap().unwrap();
                assert_eq!(request.capture_id, "review-pair");
                if path.exists() {
                    std::fs::remove_file(&path).unwrap();
                }
                private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                assert!(reader.poll(Instant::now()).is_err());
            }

            #[test]
            fn capture_arm_reader_claims_malformed_or_oversized_input_without_accepting_repair() {
                for bytes in [b"{\"schema_version\":".to_vec(), vec![b'x'; 16385]] {
                    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let identity = ControlIdentity::from_run(&fixture.fixture.run);
                    let path = fixture.fixture._dir.path().join("bad-arm.json");
                    let mut reader = NativeCaptureArmReader::new(
                        &path,
                        identity.clone(),
                        NativeCaptureLimits::default(),
                    )
                    .unwrap();
                    private_write(&path, &bytes);
                    assert!(reader.poll(Instant::now()).is_err());
                    if path.exists() {
                        std::fs::remove_file(&path).unwrap();
                    }
                    private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                    assert!(reader.poll(Instant::now()).is_err());
                }
            }

            #[test]
            fn capture_arm_reader_rejects_symlinks_public_modes_and_replaced_parent() {
                for field in ["symlink", "mode", "parent"] {
                    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let parent = fixture.fixture._dir.path().join("arm-parent");
                    std::fs::create_dir(&parent).unwrap();
                    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let identity = ControlIdentity::from_run(&fixture.fixture.run);
                    let path = parent.join("arm.json");
                    let mut reader = NativeCaptureArmReader::new(
                        &path,
                        identity.clone(),
                        NativeCaptureLimits::default(),
                    )
                    .unwrap();
                    match field {
                        "symlink" => {
                            let source = fixture.fixture._dir.path().join("source-arm.json");
                            private_write(&source, &serde_json::to_vec(&arm(&identity)).unwrap());
                            symlink(source, &path).unwrap();
                        }
                        "mode" => {
                            private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                                .unwrap();
                        }
                        _ => {
                            std::fs::rename(
                                &parent,
                                fixture.fixture._dir.path().join("old-parent"),
                            )
                            .unwrap();
                            std::fs::create_dir(&parent).unwrap();
                            std::fs::set_permissions(
                                &parent,
                                std::fs::Permissions::from_mode(0o700),
                            )
                            .unwrap();
                            private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                        }
                    }
                    assert!(reader.poll(Instant::now()).is_err(), "{field}");
                }
            }

            #[test]
            fn capture_config_requires_private_bounded_exact_files_and_existing_evidence() {
                let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let config = config(&fixture);
                for field in [
                    "valid",
                    "unknown",
                    "run",
                    "nonce",
                    "relative",
                    "missing_evidence",
                    "public",
                    "symlink",
                    "oversized",
                ] {
                    let path = fixture
                        .fixture
                        ._dir
                        .path()
                        .join(format!("config-{field}.json"));
                    let mut value = serde_json::to_value(&config).unwrap();
                    match field {
                        "unknown" => value["raw_screen"] = json!("private"),
                        "run" => value["expected_run_id"] = json!(0),
                        "nonce" => value["expected_launch_nonce"] = json!("A".repeat(64)),
                        "relative" => value["descriptor_path"] = json!("relative.json"),
                        "missing_evidence" => {
                            value["producer_manifest_path"] =
                                json!(fixture.fixture._dir.path().join("absent.json"))
                        }
                        _ => {}
                    }
                    let bytes = if field == "oversized" {
                        vec![b' '; 65537]
                    } else {
                        serde_json::to_vec(&value).unwrap()
                    };
                    private_write(&path, &bytes);
                    if field == "public" {
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                            .unwrap();
                    }
                    let selected = if field == "symlink" {
                        let link = fixture.fixture._dir.path().join("config-link.json");
                        symlink(&path, &link).unwrap();
                        link
                    } else {
                        path
                    };
                    let result =
                        read_native_capture_config(&selected, &NativeCaptureLimits::default());
                    assert_eq!(result.is_ok(), field == "valid", "{field}");
                }
            }

            #[test]
            fn capture_producer_binds_actual_libtest_and_rejects_fabricated_build_evidence() {
                let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                for field in [
                    "valid",
                    "unknown",
                    "libtest_hash",
                    "libtest_path",
                    "native_hash",
                    "parser",
                    "collector",
                    "reference",
                    "oversized",
                ] {
                    let mut value = fixture.producer_json.clone();
                    match field {
                        "unknown" => value["private_argv"] = json!([]),
                        "libtest_hash" => {
                            value["libtest_executable_sha256"] = json!("0".repeat(64))
                        }
                        "libtest_path" => value["libtest_executable_path"] = json!("relative"),
                        "native_hash" => value["native_executable"]["sha256"] = json!("INVALID"),
                        "parser" => value["parser_version"] = json!("0.0.0"),
                        "collector" => value["collector_version"] = json!("unknown"),
                        "reference" => value["environment_evidence_ref"] = json!("\n"),
                        _ => {}
                    }
                    let path = fixture
                        .fixture
                        ._dir
                        .path()
                        .join(format!("producer-{field}.json"));
                    private_write(
                        &path,
                        &if field == "oversized" {
                            vec![b' '; 65537]
                        } else {
                            serde_json::to_vec(&value).unwrap()
                        },
                    );
                    assert_eq!(
                        read_native_capture_producer(&path, &NativeCaptureLimits::default())
                            .is_ok(),
                        field == "valid",
                        "{field}"
                    );
                }
            }
            #[test]
            fn capture_publication_accepts_only_the_final_manifest_and_matching_complete_artifact()
            {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                let lease = fixture.pair(&mut port, &receiver, b"safe fixture");
                let output = fixture.fixture._dir.path().join("publication");
                std::fs::create_dir(&output).unwrap();
                std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
                let publication = publish_native_capture(
                    &output,
                    &lease,
                    &NativeCaptureLimits::default(),
                    Instant::now() + Duration::from_secs(2),
                )
                .unwrap();
                assert_eq!(
                    publication.publication_status,
                    CapturePublicationStatus::Complete
                );
                assert_eq!(publication.collection_status, NativeCaptureStatus::Complete);
                assert!(publication.artifact_path.ends_with("pair.complete.json"));
                assert!(publication.manifest_path.ends_with("pair.manifest.json"));
                let artifact = std::fs::read(&publication.artifact_path).unwrap();
                assert_eq!(
                    publication.artifact_sha256,
                    digest(&publication.artifact_path)
                );
                assert_eq!(publication.artifact_bytes, artifact.len() as u64);
                assert_eq!(
                    serde_json::from_slice::<Value>(&artifact).unwrap(),
                    value(&lease)
                );
                let manifest: Value =
                    serde_json::from_slice(&std::fs::read(&publication.manifest_path).unwrap())
                        .unwrap();
                assert_keys(
                    &manifest,
                    &[
                        "schema_version",
                        "capture_id",
                        "artifact_ref",
                        "artifact_sha256",
                        "artifact_bytes",
                        "collection_status",
                        "publication_status",
                        "producer_ref",
                        "diagnostic",
                    ],
                );
                assert_eq!(manifest["artifact_sha256"], publication.artifact_sha256);
                assert_eq!(manifest["artifact_bytes"], publication.artifact_bytes);
                assert_eq!(manifest["collection_status"], "complete");
                assert_eq!(manifest["publication_status"], "complete");
                for path in [&publication.artifact_path, &publication.manifest_path] {
                    assert_eq!(
                        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                        0o600
                    );
                }
                assert!(receiver.request(fixture.request("while-exported")).is_err());
            }

            #[test]
            fn capture_publication_collision_and_expiry_preserve_existing_output_without_success() {
                for collision in ["artifact", "manifest", "expired"] {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                    let lease = fixture.pair(&mut port, &receiver, b"private");
                    let directory = fixture.fixture._dir.path();
                    let existing = directory.join(if collision == "artifact" {
                        "pair.complete.json"
                    } else {
                        "pair.manifest.json"
                    });
                    if collision != "expired" {
                        private_write(&existing, b"owned prior output");
                    }
                    let until = if collision == "expired" {
                        Instant::now() - Duration::from_millis(1)
                    } else {
                        Instant::now() + Duration::from_secs(2)
                    };
                    assert!(
                        publish_native_capture(
                            directory,
                            &lease,
                            &NativeCaptureLimits::default(),
                            until
                        )
                        .is_err()
                    );
                    if collision != "expired" {
                        assert_eq!(std::fs::read(&existing).unwrap(), b"owned prior output");
                    }
                    if collision != "manifest" {
                        assert!(!directory.join("pair.manifest.json").exists());
                    }
                }
            }

            #[test]
            fn capture_incomplete_publication_keeps_terminal_reason_and_incomplete_filename() {
                let mut fixture = CaptureFixture::new(redacted());
                let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
                receiver.request(fixture.request("cancelled-pair")).unwrap();
                port.cancel(NativeCaptureStatus::OwnerExit).unwrap();
                let lease = receiver.try_take().unwrap().unwrap();
                let publication = publish_native_capture(
                    fixture.fixture._dir.path(),
                    &lease,
                    &NativeCaptureLimits::default(),
                    Instant::now() + Duration::from_secs(2),
                )
                .unwrap();
                assert_eq!(
                    publication.collection_status,
                    NativeCaptureStatus::OwnerExit
                );
                assert_eq!(
                    publication.publication_status,
                    CapturePublicationStatus::Incomplete
                );
                assert!(
                    publication
                        .artifact_path
                        .ends_with("cancelled-pair.incomplete.json")
                );
                assert!(
                    !fixture
                        .fixture
                        ._dir
                        .path()
                        .join("cancelled-pair.complete.json")
                        .exists()
                );
            }

            mod duplicate_fields {
                use super::*;
                fn duplicate_object(
                    object: &serde_json::Map<String, Value>,
                    key: &str,
                    first: &Value,
                    last: &Value,
                ) -> String {
                    assert!(object.contains_key(key));
                    let mut fields: Vec<String> = object
                        .iter()
                        .filter(|(name, _)| name.as_str() != key)
                        .map(|(name, value)| {
                            format!(
                                "{}:{}",
                                serde_json::to_string(name).unwrap(),
                                serde_json::to_string(value).unwrap()
                            )
                        })
                        .collect();
                    let name = serde_json::to_string(key).unwrap();
                    fields.push(format!("{name}:{}", serde_json::to_string(first).unwrap()));
                    fields.push(format!("{name}:{}", serde_json::to_string(last).unwrap()));
                    format!("{{{}}}", fields.join(","))
                }
                fn with_native_document(producer: &Value, native: &str) -> String {
                    let fields: Vec<String> = producer
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(name, value)| {
                            let value = if name == "native_executable" {
                                native.to_owned()
                            } else {
                                serde_json::to_string(value).unwrap()
                            };
                            format!("{}:{value}", serde_json::to_string(name).unwrap())
                        })
                        .collect();
                    format!("{{{}}}", fields.join(","))
                }
                fn assert_duplicate_fields_rejected(nested: bool, conflicting: bool) {
                    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    assert!(
                        read_native_capture_producer(
                            &fixture.producer_path,
                            &NativeCaptureLimits::default()
                        )
                        .is_ok()
                    );
                    let object = if nested {
                        &fixture.producer_json["native_executable"]
                    } else {
                        &fixture.producer_json
                    };
                    for (key, valid) in object.as_object().unwrap() {
                        for invalid_last in [true, false] {
                            let invalid = if conflicting {
                                json!([])
                            } else {
                                valid.clone()
                            };
                            let (first, last) = if invalid_last {
                                (valid, &invalid)
                            } else {
                                (&invalid, valid)
                            };
                            let duplicate =
                                duplicate_object(object.as_object().unwrap(), key, first, last);
                            let bytes = if nested {
                                with_native_document(&fixture.producer_json, &duplicate)
                            } else {
                                duplicate
                            };
                            let path = fixture
                                .fixture
                                ._dir
                                .path()
                                .join(format!("duplicate-{key}-{invalid_last}.json"));
                            private_write(&path, bytes.as_bytes());
                            assert!(
                                read_native_capture_producer(
                                    &path,
                                    &NativeCaptureLimits::default()
                                )
                                .is_err(),
                                "nested={nested}, conflicting={conflicting}, field={key}, invalid_last={invalid_last}"
                            );
                        }
                    }
                }

                #[test]
                fn runner_collector_top_level_identical_duplicate_fields_fail_before_normalization()
                {
                    assert_duplicate_fields_rejected(false, false);
                }
                #[test]
                fn runner_collector_nested_identical_duplicate_fields_fail_before_normalization() {
                    assert_duplicate_fields_rejected(true, false);
                }
                #[test]
                fn runner_collector_top_level_conflicting_duplicates_reject_both_field_orders() {
                    assert_duplicate_fields_rejected(false, true);
                }
                #[test]
                fn runner_collector_nested_conflicting_duplicates_reject_both_field_orders() {
                    assert_duplicate_fields_rejected(true, true);
                }
                #[test]
                fn runner_collector_nullable_native_version_requires_presence_and_accepts_explicit_null()
                 {
                    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                    fixture.producer_json["native_executable"]["version"] = Value::Null;
                    let null = fixture.fixture._dir.path().join("null-version.json");
                    private_write(&null, &serde_json::to_vec(&fixture.producer_json).unwrap());
                    assert!(
                        read_native_capture_producer(&null, &NativeCaptureLimits::default())
                            .is_ok()
                    );
                    fixture.producer_json["native_executable"]
                        .as_object_mut()
                        .unwrap()
                        .remove("version")
                        .unwrap();
                    let missing = fixture.fixture._dir.path().join("missing-version.json");
                    private_write(
                        &missing,
                        &serde_json::to_vec(&fixture.producer_json).unwrap(),
                    );
                    assert!(
                        read_native_capture_producer(&missing, &NativeCaptureLimits::default())
                            .is_err()
                    );
                }
            }
        }
    }

    mod verification {
        use super::*;
        use crate::broker::runtime as storage;
        use crate::coding_agent::CodingAgent;
        use crate::config::Settings;
        use crate::digest::sha256_hex;
        use crate::multiplexer::{CommandRunner, MultiplexerContext, resolve_multiplexer};
        use crate::runtime::system::SystemRunner;
        use rusqlite::Connection;
        use std::os::unix::fs::MetadataExt;
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        use std::path::Path;
        use std::rc::Rc;

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CapturePolicyFile {
            schema_version: u32,
            masks: Vec<CaptureMask>,
        }
        pub(crate) fn run_native_capture_verification(config: &Path) -> Result<i32> {
            run_native_capture_verification_with_export_hooks(
                config,
                Arc::new(InertCaptureExportHooks),
            )
        }
        pub(crate) fn run_native_capture_verification_with_export_hooks(
            config: &Path,
            hooks: Arc<dyn NativeCaptureExportHooks>,
        ) -> Result<i32> {
            let limits = NativeCaptureLimits::default();
            let config = read_native_capture_config(config, &limits)?;
            let settings = Settings::for_database(
                Path::new(crate::db::database_path(&config.database_url)?),
                None,
            );
            let conn = crate::db::connect(&settings.database_url)?;
            let descriptor = load_descriptor(&conn, &config.descriptor_path)?;
            let run = storage::get_run(&conn, descriptor.run_id)?;
            ensure(
                run.run_id == config.expected_run_id
                    && run.launch_nonce == config.expected_launch_nonce,
                "verification configuration run/nonce mismatch",
            )?;
            let catalog = ProfileCatalog::bundled()?;
            let policy: CapturePolicyFile = serde_json::from_slice(&super::files::read_private(
                &config.capture_policy_path,
                limits.max_config_bytes,
            )?)
            .map_err(io_error)?;
            ensure(policy.schema_version == 1, "capture policy schema mismatch")?;
            let capture_policy =
                NativeCapturePolicy::from_launch(&descriptor.spawn, &policy.masks, &catalog)?;
            let (mut port, receiver) =
                native_capture_channel(&run, capture_policy, limits.clone())?;
            port.service_producer = Some(read_native_capture_producer(
                &config.producer_manifest_path,
                &limits,
            )?);
            let task =
                CaptureExporterTask::install(&mut port, receiver, config.clone(), limits, hooks);
            let result = run_service(
                &settings,
                &config.descriptor_path,
                RunnerDependencies {
                    catalog,
                    clock: Arc::new(RealObservationClock),
                    writer_hooks: Arc::new(InertWriterHooks),
                    capture: Some(port),
                    observation_hooks: None,
                },
            );
            let exported = task.finish(&result);
            match (result, exported) {
                (Ok(status), Ok(())) => Ok(status),
                (Ok(status), Err(cause)) if status != 0 => Err(error(format!(
                    "capture service exited with status {status}"
                ))
                .with_cleanup(cause.message())),
                (Ok(_), Err(cause)) => Err(cause),
                (Err(primary), Err(secondary)) => Err(primary.with_cleanup(secondary.message())),
                (Err(cause), Ok(())) => Err(cause),
            }
        }

        // Committed native fixtures derive from these captures, so their visible paths must not name a home directory.
        const DRIVER_NEUTRAL_ROOT: &str = "/tmp/cafleet-194";
        // The repository mount reports root-created files as owned by another user, which the private
        // ownership checks refuse, so case databases and sockets live on the local filesystem.
        const DRIVER_PRIVATE_ROOT: &str = "/tmp/cafleet-194-p";
        const DRIVER_ANSWER_WINDOW_MS: u64 = 60_000;
        const DRIVER_TOOL_FREE_PROMPT: &str = "Reply with the single word ready and then wait.";

        #[derive(PartialEq, Eq)]
        enum DriverMode {
            NoAnswer,
            AnswerStage,
        }
        struct ApprovedCase {
            id: &'static str,
            backend: CodingAgent,
            permission_mode: &'static str,
            mode: DriverMode,
            decision: Option<&'static str>,
            prefix_hex: &'static [&'static str],
            stage_hex: &'static [&'static str],
            probe_argument: Option<&'static str>,
            database: &'static str,
            socket: &'static str,
            size: TerminalSize,
        }
        const DRIVER_FULL_SIZE: TerminalSize = TerminalSize {
            rows: 36,
            cols: 120,
        };
        const DRIVER_WRAPPED_SIZE: TerminalSize = TerminalSize { rows: 36, cols: 52 };
        const DRIVER_BOXED_WRAP_SIZE: TerminalSize = TerminalSize { rows: 36, cols: 89 };
        const DRIVER_HERDR_ENV_SIZE: TerminalSize = TerminalSize {
            rows: 36,
            cols: 166,
        };
        // tmux sets its own TERM and TERM_PROGRAM in every pane, so the herdr terminal identity reaches the
        // native child only as spawn overrides; the values are those the live herdr child carried.
        const DRIVER_HERDR_CHILD_ENVIRONMENT: [(&str, &str); 5] = [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "herdr"),
            ("TERM_PROGRAM_VERSION", "0.9.5"),
            ("HERDR_ENV", "1"),
        ];
        const APPROVED_CASES: [ApprovedCase; 23] = [
            ApprovedCase {
                id: "cells-002",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "nc02.db",
                socket: "tc02",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "ans-allow-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::AnswerStage,
                decision: Some("allow"),
                prefix_hex: &[],
                stage_hex: &["31"],
                probe_argument: Some("allow"),
                database: "na01.db",
                socket: "ta01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "ans-deny-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &[],
                stage_hex: &["34"],
                probe_argument: Some("deny"),
                database: "nd01.db",
                socket: "td01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "idle-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: None,
                database: "ni01.db",
                socket: "ti01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "codex-trust-001",
                backend: CodingAgent::Codex,
                permission_mode: "on-request",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: None,
                database: "xt01.db",
                socket: "xt01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "codex-cells-001",
                backend: CodingAgent::Codex,
                permission_mode: "on-request",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "xc01.db",
                socket: "xc01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "codex-ans-allow-001",
                backend: CodingAgent::Codex,
                permission_mode: "on-request",
                mode: DriverMode::AnswerStage,
                decision: Some("allow"),
                prefix_hex: &[],
                stage_hex: &["79"],
                probe_argument: Some("allow"),
                database: "xa01.db",
                socket: "xa01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "codex-ans-deny-001",
                backend: CodingAgent::Codex,
                permission_mode: "on-request",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &[],
                stage_hex: &["33"],
                probe_argument: Some("deny"),
                database: "xd01.db",
                socket: "xd01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "codex-idle-001",
                backend: CodingAgent::Codex,
                permission_mode: "on-request",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: None,
                database: "xi01.db",
                socket: "xi01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "claude-wrap-cells-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "nwc1.db",
                socket: "twc1",
                size: DRIVER_WRAPPED_SIZE,
            },
            ApprovedCase {
                id: "claude-wrap-ans-allow-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::AnswerStage,
                decision: Some("allow"),
                prefix_hex: &[],
                stage_hex: &["31"],
                probe_argument: Some("allow"),
                database: "nwa1.db",
                socket: "twa1",
                size: DRIVER_WRAPPED_SIZE,
            },
            ApprovedCase {
                id: "claude-wrap-ans-deny-001",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &[],
                stage_hex: &["34"],
                probe_argument: Some("deny"),
                database: "nwd1.db",
                socket: "twd1",
                size: DRIVER_WRAPPED_SIZE,
            },
            ApprovedCase {
                id: "claude-boxed-cells-089-wide-operation-probe-01",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "nb89.db",
                socket: "tb89",
                size: DRIVER_BOXED_WRAP_SIZE,
            },
            ApprovedCase {
                id: "claude-boxed-cells-120-wide-operation-probe-01",
                backend: CodingAgent::Claude,
                permission_mode: "default",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "nb12.db",
                socket: "tb12",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-cells-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "oc01.db",
                socket: "oc01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-ans-allow-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("allow"),
                prefix_hex: &[],
                stage_hex: &["0d"],
                probe_argument: Some("allow"),
                database: "oa01.db",
                socket: "oa01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-ans-deny-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &[],
                stage_hex: &["68", "0d"],
                probe_argument: Some("deny"),
                database: "od01.db",
                socket: "od01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-ans-deny-002",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &["68"],
                stage_hex: &["0d", "0d"],
                probe_argument: Some("deny"),
                database: "od02.db",
                socket: "od02",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-ans-deny-003",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &["68", "0d"],
                stage_hex: &["0d"],
                probe_argument: Some("deny"),
                database: "od03.db",
                socket: "od03",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-focus-always-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &["6c"],
                stage_hex: &[],
                probe_argument: Some("deny"),
                database: "of01.db",
                socket: "of01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-focus-always-002",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::AnswerStage,
                decision: Some("deny"),
                prefix_hex: &["6c"],
                stage_hex: &[],
                probe_argument: Some("deny"),
                database: "of02.db",
                socket: "of02",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-idle-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: None,
                database: "oi01.db",
                socket: "oi01",
                size: DRIVER_FULL_SIZE,
            },
            ApprovedCase {
                id: "opencode-herdr-env-cells-001",
                backend: CodingAgent::Opencode,
                permission_mode: "cafleet-ask",
                mode: DriverMode::NoAnswer,
                decision: None,
                prefix_hex: &[],
                stage_hex: &[],
                probe_argument: Some("allow"),
                database: "oh01.db",
                socket: "oh01",
                size: DRIVER_HERDR_ENV_SIZE,
            },
        ];
        impl ApprovedCase {
            fn select(case: &DriverCase) -> Result<&'static Self> {
                let approved = APPROVED_CASES
                    .iter()
                    .find(|approved| approved.id == case.case_id)
                    .ok_or_else(|| error("unreviewed native case"))?;
                ensure(
                    (approved.mode == DriverMode::NoAnswer)
                        == (approved.prefix_hex.is_empty() && approved.stage_hex.is_empty())
                        && approved
                            .prefix_hex
                            .iter()
                            .chain(approved.stage_hex)
                            .all(|stage| stage.as_bytes().chunks(2).all(|byte| byte != b"1b")),
                    "reviewed case stages differ from its driver mode or contain Escape",
                )?;
                let (mode, window) = match approved.mode {
                    DriverMode::NoAnswer => ("no_answer", 0),
                    DriverMode::AnswerStage => ("answer_stage", DRIVER_ANSWER_WINDOW_MS),
                };
                ensure(
                    case.mode == mode
                        && case.answer_window_ms == window
                        && case.decision.as_deref() == approved.decision
                        && case.prefix_hex == approved.prefix_hex
                        && case.stage_hex == approved.stage_hex
                        && case.probe_argument.as_deref() == approved.probe_argument,
                    "case decision, stage or probe binding differs from its reviewed case",
                )?;
                Ok(approved)
            }
            fn child_environment(&self) -> &'static [(&'static str, &'static str)] {
                match self.id {
                    "opencode-herdr-env-cells-001" => &DRIVER_HERDR_CHILD_ENVIRONMENT,
                    _ => &[],
                }
            }
            fn member_name(&self) -> String {
                format!("cafleet-194-{}", self.id)
            }
            fn tmux_geometry(&self) -> String {
                format!("{}|{}", self.size.cols, self.size.rows)
            }
            fn requested_marker(&self) -> Option<String> {
                self.probe_argument
                    .map(|argument| format!("{argument}.executed"))
            }
            fn prompt(&self, helper: &Path) -> String {
                match (self.probe_argument, self.backend) {
                    (Some(argument), CodingAgent::Codex) => format!(
                        "Use the shell tool to run exactly {} {argument} as one command with sandbox_permissions set to require_escalated and the justification \"CAFleet permission probe\". Preserve any native approval request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                        helper.display()
                    ),
                    (Some(argument), CodingAgent::Opencode) => format!(
                        "Use the shell tool to run exactly {} {argument} as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                        helper.display()
                    ),
                    (Some(argument), CodingAgent::Claude) => format!(
                        "Use Bash to attempt exactly {} {argument} as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                        helper.display()
                    ),
                    (None, _) => DRIVER_TOOL_FREE_PROMPT.into(),
                }
            }
            fn forbidden_markers(&self) -> Vec<&'static str> {
                let allowed = (self.decision == Some("allow")).then_some("allow.executed");
                ["allow.executed", "deny.executed"]
                    .into_iter()
                    .filter(|marker| Some(*marker) != allowed)
                    .collect()
            }
        }

        #[derive(Clone, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverReference {
            path: PathBuf,
            sha256: String,
        }
        impl DriverReference {
            fn verify(&self) -> Result<()> {
                let metadata = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
                ensure(
                    self.path.is_absolute()
                        && metadata.is_file()
                        && metadata.len() <= 512 * 1024 * 1024,
                    "bounded absolute evidence file required",
                )?;
                ensure(
                    super::files::file_digest(&self.path)? == self.sha256,
                    "driver evidence digest changed",
                )
            }
        }

        #[derive(Clone, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverTmux {
            socket_path: PathBuf,
            socket_device: u64,
            socket_inode: u64,
            server_pid: i64,
            session: String,
            window_id: String,
            pane_id: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverCase {
            schema_version: u32,
            case_id: String,
            mode: String,
            answer_window_ms: u64,
            decision: Option<String>,
            prefix_hex: Vec<String>,
            stage_hex: Vec<String>,
            probe_argument: Option<String>,
            case_nonce: String,
            expires_at_mono_ns: u64,
            cleanup_reserve_ms: u64,
            database_url: String,
            config_path: PathBuf,
            cwd: PathBuf,
            output_directory: PathBuf,
            arm_path: PathBuf,
            producer_manifest_path: PathBuf,
            launch_evidence_path: PathBuf,
            driver_ownership_path: PathBuf,
            started_record_path: PathBuf,
            stop_path: PathBuf,
            privacy_confirmation_path: PathBuf,
            result_path: PathBuf,
            descriptor_copy_path: PathBuf,
            native_version: String,
            director_backend: String,
            native_executable: DriverReference,
            helper: DriverReference,
            libtest: DriverReference,
            source_manifest: DriverReference,
            version_evidence: DriverReference,
            environment_evidence: DriverReference,
            event_redaction: DriverReference,
            capture_policy: DriverReference,
            tmux: DriverTmux,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverOwnership {
            schema_version: u32,
            case_manifest_sha256: String,
            case_nonce: String,
            driver: ProcessIdentity,
            server: ProcessIdentity,
            tmux: DriverTmux,
            expires_at_mono_ns: u64,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverStarted {
            schema_version: u32,
            case_manifest_sha256: String,
            driver_ownership_sha256: String,
            identity: ControlIdentity,
            runner: ProcessIdentity,
            child: ProcessIdentity,
            pane_id: String,
            rows: u16,
            cols: u16,
            capture_policy_sha256: String,
            event_redaction_sha256: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverStop {
            schema_version: u32,
            case_manifest_sha256: String,
            driver_ownership_sha256: String,
            reason: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverPrivacy {
            schema_version: u32,
            case_manifest_sha256: String,
            started_record_sha256: String,
            rows: u16,
            cols: u16,
            capture_policy_sha256: String,
            event_redaction_sha256: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DriverVersion {
            schema_version: u32,
            executable: PathBuf,
            sha256: String,
            argv: Vec<String>,
            stdout: String,
            exit_code: i32,
        }
        fn driver_read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
            let bytes = super::files::read_private(path, 65536)
                .map_err(|cause| error(format!("{}: {}", path.display(), cause.message())))?;
            serde_json::from_slice(&bytes)
                .map_err(|cause| error(format!("{}: {cause}", path.display())))
        }
        fn driver_publish(path: &Path, value: &impl Serialize, until: Instant) -> Result<()> {
            let bytes = serde_json::to_vec(value).map_err(io_error)?;
            ensure(bytes.len() <= 65536, "private driver record exceeds bound")?;
            super::files::publish_private(path, &bytes, until)
        }
        fn driver_tmux(case: &DriverCase, parts: &[&str], until: Instant) -> Result<String> {
            let mut argv = vec![
                "tmux".into(),
                "-S".into(),
                case.tmux.socket_path.to_string_lossy().into_owned(),
            ];
            argv.extend(parts.iter().map(|s| (*s).to_owned()));
            SystemRunner
                .run_until(&argv, until)
                .map_err(|cause| error(format!("owned tmux command failed: {cause:?}")))
        }
        fn driver_validate_tmux(
            case: &DriverCase,
            ownership: &DriverOwnership,
            until: Instant,
        ) -> Result<()> {
            let metadata = std::fs::symlink_metadata(&case.tmux.socket_path).map_err(io_error)?;
            ensure(
                metadata.dev() == case.tmux.socket_device
                    && metadata.ino() == case.tmux.socket_inode,
                "dedicated tmux socket identity changed",
            )?;
            ensure(
                ownership.server.matches()?,
                "dedicated tmux server generation changed",
            )?;
            let actual = driver_tmux(
                case,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    &case.tmux.pane_id,
                    "#{pid}|#{session_name}|#{window_id}|#{pane_id}",
                ],
                until,
            )?;
            ensure(
                actual.trim()
                    == format!(
                        "{}|{}|{}|{}",
                        case.tmux.server_pid,
                        case.tmux.session,
                        case.tmux.window_id,
                        case.tmux.pane_id
                    ),
                "dedicated controller identity changed",
            )
        }
        fn driver_validate_run(
            conn: &Connection,
            launched: &LaunchOutcome,
        ) -> Result<storage::RunRecord> {
            let current = storage::get_run(conn, launched.run.run_id)?;
            launched.started.identity.validate(&current)?;
            ensure(
                current.owner() == launched.run.owner()
                    && current.child_pid == Some(launched.started.child.pid)
                    && current.child_start_identity.as_deref()
                        == Some(launched.started.child.start_identity.as_str()),
                "owned native generation changed",
            )?;
            let placement = crate::broker::get_member(conn, current.member_id, current.fleet_id)?
                .and_then(|member| member.placement)
                .ok_or_else(|| error("owned placement unavailable"))?;
            ensure(placement == launched.placement, "owned placement changed")?;
            Ok(current)
        }
        fn driver_validate_pane(
            case: &DriverCase,
            ownership: &DriverOwnership,
            launched: &LaunchOutcome,
            until: Instant,
        ) -> Result<()> {
            driver_validate_tmux(case, ownership, until)?;
            let pane = launched
                .placement
                .mux_pane_id
                .as_deref()
                .expect("launched pane identity");
            let actual = driver_tmux(
                case,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_pid}|#{session_name}|#{window_id}|#{pane_id}|#{pane_dead}",
                ],
                until,
            )?;
            let live = format!(
                "{}|{}|{}|{}|0",
                launched.started.runner.pid, case.tmux.session, case.tmux.window_id, pane
            );
            let dead = format!(
                "{}|{}|{}|{}|1",
                launched.started.runner.pid, case.tmux.session, case.tmux.window_id, pane
            );
            ensure(
                actual.trim() == dead
                    || (actual.trim() == live && launched.started.runner.matches()?),
                "runner pane generation changed",
            )
        }

        fn driver_armed(case: &DriverCase) -> Result<bool> {
            Ok(case.arm_path.try_exists().map_err(io_error)?
                || case
                    .arm_path
                    .with_extension("claimed")
                    .try_exists()
                    .map_err(io_error)?)
        }
        fn driver_validate_privacy(
            case: &DriverCase,
            approved: &ApprovedCase,
            ownership: &DriverOwnership,
        ) -> Result<()> {
            case.capture_policy.verify()?;
            case.event_redaction.verify()?;
            let privacy: DriverPrivacy = driver_read(&case.privacy_confirmation_path)?;
            ensure(
                privacy.schema_version == 1
                    && privacy.case_manifest_sha256 == ownership.case_manifest_sha256
                    && privacy.started_record_sha256
                        == super::files::file_digest(&case.started_record_path)?
                    && privacy.rows == approved.size.rows
                    && privacy.cols == approved.size.cols
                    && privacy.capture_policy_sha256 == case.capture_policy.sha256
                    && privacy.event_redaction_sha256 == case.event_redaction.sha256,
                "privacy confirmation binding differs",
            )
        }

        fn driver_capture_runner_pane(
            case: &DriverCase,
            pane: &str,
            until: Instant,
        ) -> Result<PathBuf> {
            const RETAINED_BYTES: usize = 48 * 1024;
            let mut text = driver_tmux(
                case,
                &["capture-pane", "-p", "-J", "-S", "-", "-t", pane],
                until,
            )?;
            let redaction: Value = driver_read(&case.event_redaction.path)?;
            for pattern in redaction["patterns"]
                .as_array()
                .ok_or_else(|| error("case redaction patterns required"))?
            {
                let pattern = regex::Regex::new(
                    pattern
                        .as_str()
                        .ok_or_else(|| error("case redaction pattern string required"))?,
                )
                .map_err(io_error)?;
                text = pattern.replace_all(&text, "[REDACTED]").into_owned();
            }
            let start = (text.len().saturating_sub(RETAINED_BYTES)..=text.len())
                .find(|index| text.is_char_boundary(*index))
                .expect("the string end is a character boundary");
            let path = case.cwd.join("runner-pane-capture.json");
            driver_publish(
                &path,
                &json!({"schema_version":1,"pane_id":pane,"redaction_sha256":case.event_redaction.sha256,
            "omitted_leading_bytes":start,"text":&text[start..]}),
                until,
            )?;
            Ok(path)
        }

        fn driver_complete_capture(
            case: &DriverCase,
            approved: &ApprovedCase,
            launched: &LaunchOutcome,
            service: &Value,
        ) -> Result<Value> {
            ensure(
                service["schema_version"] == 1
                    && service["result"] == "ok"
                    && service["status"]
                        .as_i64()
                        .is_some_and(|s| (0..=255).contains(&s))
                    && service["producer_sha256"]
                        == super::files::file_digest(&case.producer_manifest_path)?
                    && service["process"]
                        == serde_json::to_value(&launched.started.runner).map_err(io_error)?,
                "capture service/export did not return a bound actual status",
            )?;
            let artifact_name = format!("{}.complete.json", case.case_id);
            let manifest_path = case
                .output_directory
                .join(format!("{}.manifest.json", case.case_id));
            let manifest: Value = driver_read(&manifest_path)?;
            ensure(
                manifest["capture_id"] == case.case_id.as_str()
                    && manifest["publication_status"] == "complete"
                    && manifest["collection_status"] == "complete"
                    && manifest["artifact_ref"] == artifact_name.as_str()
                    && manifest["producer_ref"]
                        == case.producer_manifest_path.to_string_lossy().as_ref(),
                "complete capture manifest required",
            )?;
            let artifact = case.output_directory.join(&artifact_name);
            let bytes = super::files::read_private(
                &artifact,
                NativeCaptureLimits::default().max_encoded_result_bytes,
            )?;
            ensure(
                manifest["artifact_sha256"] == crate::digest::sha256_hex(&bytes)
                    && manifest["artifact_bytes"] == bytes.len(),
                "capture artifact/manifest digest mismatch",
            )?;
            let value: Value = serde_json::from_slice(&bytes).map_err(io_error)?;
            ensure(
                value["status"] == "complete"
                    && value["capture_id"] == case.case_id.as_str()
                    && value["source"]["run_id"] == launched.run.run_id
                    && value["source"]["owner_epoch"] == launched.run.owner_epoch
                    && value["source"]["launch_nonce"] == launched.run.launch_nonce
                    && value["source"]["runner"]
                        == serde_json::to_value(&launched.started.runner).map_err(io_error)?
                    && value["source"]["child"]
                        == serde_json::to_value(&launched.started.child).map_err(io_error)?
                    && value["samples"].as_array().is_some_and(|s| {
                        s.len() == 2
                            && s.iter().all(|sample| {
                                sample["rows"] == approved.size.rows
                                    && sample["cols"] == approved.size.cols
                            })
                    }),
                "actual complete parser pair required",
            )?;
            Ok(
                json!({"manifest_path":manifest_path,"manifest_sha256":super::files::file_digest(&manifest_path)?,
        "artifact_path":artifact,"artifact_sha256":crate::digest::sha256_hex(&bytes),"service_status":service["status"]}),
            )
        }

        struct CaptureVerificationLaunchHost<'a> {
            inner: NativeLaunchHost<'a>,
            case: &'a DriverCase,
            case_until: Instant,
        }
        impl LaunchHost for CaptureVerificationLaunchHost<'_> {
            fn now(&self) -> Instant {
                self.inner.now()
            }
            fn current_executable(&self) -> Result<PathBuf> {
                self.inner.current_executable()
            }
            fn split_runner(
                &mut self,
                argv: &[String],
                environment: &[(String, String)],
                until: Instant,
            ) -> Result<String> {
                let config = read_native_capture_config(
                    &self.case.config_path,
                    &NativeCaptureLimits::default(),
                )?;
                ensure(
                    argv.len() == 4
                        && argv[0] == self.case.libtest.path.to_string_lossy()
                        && argv[1] == "__member-run"
                        && argv[2] == "--descriptor"
                        && Path::new(&argv[3]) == config.descriptor_path
                        && environment
                            == [(
                                "CAFLEET_DATABASE_URL".into(),
                                self.case.database_url.clone(),
                            )],
                    "generated launch tuple changed",
                )?;
                let descriptor: LaunchDescriptor = driver_read(&config.descriptor_path)?;
                ensure(
                    descriptor.run_id == config.expected_run_id
                        && descriptor.launch_nonce == config.expected_launch_nonce
                        && descriptor.spawn.argv[0]
                            == self.case.native_executable.path.to_string_lossy(),
                    "generated descriptor binding changed",
                )?;
                driver_publish(
                    &self.case.descriptor_copy_path,
                    &descriptor,
                    self.case_until,
                )?;
                let child = vec![
                    argv[0].clone(),
                    "--exact".into(),
                    native_capture_service_child_path().into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                    "--test-threads=1".into(),
                ];
                let mut environment = environment.to_vec();
                environment.push((
                    "CAFLEET_NATIVE_CAPTURE_CONFIG".into(),
                    self.case.config_path.to_string_lossy().into_owned(),
                ));
                self.inner
                    .split_runner(&child, &environment, until.min(self.case_until))
            }
            fn wait_prepared(
                &mut self,
                descriptor: &LaunchDescriptor,
                until: Instant,
            ) -> Result<Prepared> {
                self.inner
                    .wait_prepared(descriptor, until.min(self.case_until))
            }
            fn start(
                &mut self,
                prepared: &Prepared,
                placement: &Placement,
                until: Instant,
            ) -> Result<Started> {
                self.inner
                    .start(prepared, placement, until.min(self.case_until))
            }
            fn shutdown_owned(
                &mut self,
                run: &storage::RunRecord,
                clock: ShutdownClock,
            ) -> Result<ShutdownOutcome> {
                self.inner
                    .shutdown_owned(run, clock.clipped(self.case_until))
            }
            fn close_pane(&mut self, pane: &str, until: Instant) -> Result<()> {
                self.inner.close_pane(pane, until.min(self.case_until))
            }
        }

        fn driver_prepare(
            conn: &mut Connection,
            pending: &PendingRegistration,
            case: &DriverCase,
            until: Instant,
        ) -> Result<(RunArtifacts, SpawnSpec)> {
            let approved = ApprovedCase::select(case)?;
            let artifacts = RunArtifacts::create_with_deadline(conn, pending.run.run_id, until)?;
            let policy = approved
                .backend
                .resolve_policy(Some(&case.native_version), Some(approved.permission_mode))?;
            let prompt = approved.prompt(&case.helper.path);
            let mut argv = approved.backend.build_spawn_argv_with_policy(
                &prompt,
                &approved.member_name(),
                None,
                None,
                &policy,
            )?;
            argv[0] = case.native_executable.path.to_string_lossy().into_owned();
            let config: Value = driver_read(&case.event_redaction.path)?;
            let catalog = ProfileCatalog::bundled()?;
            let rules = catalog.rule_ids();
            crate::broker::runtime::evidence::EvidencePolicy::redacted(
                &config,
                &rules.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            let capture: CapturePolicyFile = driver_read(&case.capture_policy.path)?;
            ensure(
                capture.schema_version == 1 && !capture.masks.is_empty(),
                "reviewed exact-geometry masks required",
            )?;
            ensure(
                capture
                    .masks
                    .iter()
                    .all(|m| m.rows == approved.size.rows && m.cols == approved.size.cols),
                "case mask geometry differs",
            )?;
            let spawn = SpawnSpec {
                argv,
                cwd: case.cwd.clone(),
                environment: std::iter::once((
                    "CAFLEET_DATABASE_URL".into(),
                    case.database_url.clone(),
                ))
                .chain(
                    approved
                        .child_environment()
                        .iter()
                        .map(|&(name, value)| (name.into(), value.into())),
                )
                .collect(),
                evidence_policy: json!({"mode":"redacted","config":config}),
                size: approved.size,
            };
            spawn.validate()?;
            NativeCapturePolicy::from_launch(&spawn, &capture.masks, &catalog)?;
            driver_publish(
                &case.launch_evidence_path,
                &json!({"schema_version":1,"run_id":pending.run.run_id,
        "launch_nonce":pending.run.launch_nonce,"argv":spawn.argv,"cwd":spawn.cwd,"policy":spawn.evidence_policy,
        "native_executable":case.native_executable,"version_evidence":case.version_evidence}),
                until,
            )?;
            let producer = json!({"schema_version":1,"kind":"verification_libtest",
        "source_manifest_ref":case.source_manifest.path,"source_manifest_sha256":case.source_manifest.sha256,
        "libtest_executable_path":case.libtest.path,"libtest_executable_sha256":case.libtest.sha256,
        "parser_version":"0.18.1","collector_version":"1","catalog_origin":"bundled_unknown",
        "native_executable":{"path":case.native_executable.path,"sha256":case.native_executable.sha256,
            "version":case.native_version,"version_evidence_ref":case.version_evidence.path},
        "launch_evidence_ref":case.launch_evidence_path,"environment_evidence_ref":case.environment_evidence.path});
            driver_publish(&case.producer_manifest_path, &producer, until)?;
            let config = NativeCaptureConfig {
                schema_version: 1,
                database_url: case.database_url.clone(),
                descriptor_path: artifacts.paths.descriptor.clone(),
                expected_run_id: pending.run.run_id,
                expected_launch_nonce: pending.run.launch_nonce.clone(),
                arm_path: case.arm_path.clone(),
                output_directory: case.output_directory.clone(),
                producer_manifest_path: case.producer_manifest_path.clone(),
                capture_policy_path: case.capture_policy.path.clone(),
            };
            driver_publish(&case.config_path, &config, until)?;
            Ok((artifacts, spawn))
        }

        fn driver_output_seq(
            conn: &Connection,
            launched: &LaunchOutcome,
            until: Instant,
        ) -> Result<u64> {
            let run = storage::get_run(conn, launched.run.run_id)?;
            Ok(crate::runtime::runner::probe_health(&run, until)?.output_seq)
        }

        fn driver_post_stage(
            conn: &Connection,
            case: &DriverCase,
            approved: &ApprovedCase,
            launched: &LaunchOutcome,
            armed_output_seq: Option<u64>,
        ) -> Result<Value> {
            let until = Instant::now() + Duration::from_secs(1);
            let stop_output_seq = driver_output_seq(conn, launched, until);
            let mut statement = conn
        .prepare("SELECT e.event_seq,e.kind,e.observed_at,e.interaction_id,i.classification,i.close_reason,i.closed_at FROM member_runtime_events e LEFT JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 ORDER BY e.event_seq DESC LIMIT 64")
        .map_err(io_error)?;
            let mut events = statement
        .query_map([launched.run.run_id], |row| {
            Ok(json!({"event_seq":row.get::<_, i64>(0)?,"kind":row.get::<_, String>(1)?,
                "observed_at":row.get::<_, String>(2)?,"interaction_id":row.get::<_, Option<i64>>(3)?,
                "classification":row.get::<_, Option<String>>(4)?,"close_reason":row.get::<_, Option<String>>(5)?,
                "closed_at":row.get::<_, Option<String>>(6)?}))
        })
        .map_err(io_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(io_error)?;
            events.reverse();
            let marker = case.cwd.join(
                approved
                    .requested_marker()
                    .expect("answer-stage cases name their probe"),
            );
            let marker_sha256 = if marker.try_exists().map_err(io_error)? {
                Some(super::files::file_digest(&marker)?)
            } else {
                None
            };
            Ok(
                json!({"decision":approved.decision,"prefix_hex":approved.prefix_hex,"stage_hex":approved.stage_hex,
        "output_seq_at_arm":armed_output_seq,
        "output_seq_at_stop":stop_output_seq.as_ref().ok(),
        "output_seq_error":stop_output_seq.as_ref().err().map(|e|e.message().into_owned()),
        "manual_input_count":"unavailable: runner health exposes no manual-input counter",
        "events":events,"requested_marker":approved.requested_marker(),
        "requested_marker_sha256":marker_sha256}),
            )
        }

        fn driver_work(
            conn: &Connection,
            case: &DriverCase,
            ownership: &DriverOwnership,
            launched: &LaunchOutcome,
            work_until: Instant,
        ) -> Result<Value> {
            driver_validate_pane(case, ownership, launched, work_until)?;
            driver_validate_run(conn, launched)?;
            let pane = launched
                .placement
                .mux_pane_id
                .as_deref()
                .ok_or_else(|| error("owned runner pane required"))?;
            let approved = ApprovedCase::select(case)?;
            let cols = approved.size.cols.to_string();
            let rows = approved.size.rows.to_string();
            driver_tmux(
                case,
                &[
                    "resize-window",
                    "-t",
                    &case.tmux.window_id,
                    "-x",
                    &cols,
                    "-y",
                    "80",
                ],
                work_until,
            )?;
            driver_tmux(
                case,
                &[
                    "select-layout",
                    "-t",
                    &case.tmux.window_id,
                    "main-horizontal",
                ],
                work_until,
            )?;
            driver_tmux(
                case,
                &["resize-pane", "-t", pane, "-x", &cols, "-y", &rows],
                work_until,
            )?;
            let geometry = driver_tmux(
                case,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_width}|#{pane_height}",
                ],
                work_until,
            )?;
            ensure(
                geometry.trim() == approved.tmux_geometry(),
                "actual native geometry differs",
            )?;
            let ownership_hash = super::files::file_digest(&case.driver_ownership_path)?;
            let started = DriverStarted {
                schema_version: 1,
                case_manifest_sha256: ownership.case_manifest_sha256.clone(),
                driver_ownership_sha256: ownership_hash.clone(),
                identity: launched.started.identity.clone(),
                runner: launched.started.runner.clone(),
                child: launched.started.child.clone(),
                pane_id: pane.into(),
                rows: approved.size.rows,
                cols: approved.size.cols,
                capture_policy_sha256: case.capture_policy.sha256.clone(),
                event_redaction_sha256: case.event_redaction.sha256.clone(),
            };
            driver_publish(&case.started_record_path, &started, work_until)?;
            let mut armed_validated_at: Option<Instant> = None;
            let mut armed_output_seq: Option<u64> = None;
            loop {
                let stopped = if case.stop_path.try_exists().map_err(io_error)? {
                    let stop: DriverStop = driver_read(&case.stop_path)?;
                    ensure(
                        stop.schema_version == 1
                            && stop.reason == "director_stop"
                            && stop.case_manifest_sha256 == ownership.case_manifest_sha256
                            && stop.driver_ownership_sha256 == ownership_hash,
                        "stop binding differs",
                    )?;
                    true
                } else {
                    Instant::now() >= work_until
                };
                if stopped {
                    return match approved.mode {
                        DriverMode::NoAnswer => Ok(Value::Null),
                        DriverMode::AnswerStage => {
                            driver_post_stage(conn, case, approved, launched, armed_output_seq)
                        }
                    };
                }
                ensure(
                    launched.started.runner.matches()? && launched.started.child.matches()?,
                    "native or runner ended before the requested capture completed",
                )?;
                let armed_validation_due = match armed_validated_at {
                    None => true,
                    Some(at) => at.elapsed() >= Duration::from_secs(1),
                };
                if armed_validation_due && driver_armed(case)? {
                    armed_validated_at = Some(Instant::now());
                    driver_validate_privacy(case, approved, ownership)?;
                    driver_validate_tmux(case, ownership, work_until)?;
                    driver_validate_run(conn, launched)?;
                    let geometry = driver_tmux(
                        case,
                        &[
                            "display-message",
                            "-p",
                            "-t",
                            pane,
                            "#{pane_width}|#{pane_height}",
                        ],
                        work_until,
                    )?;
                    ensure(
                        geometry.trim() == approved.tmux_geometry(),
                        "armed geometry changed",
                    )?;
                    if armed_output_seq.is_none() {
                        armed_output_seq = Some(driver_output_seq(conn, launched, work_until)?);
                    }
                }
                for marker in approved.forbidden_markers() {
                    ensure(
                        !case.cwd.join(marker).try_exists().map_err(io_error)?,
                        "a probe outside the reviewed decision executed",
                    )?;
                }
                std::thread::sleep(
                    Duration::from_millis(20)
                        .min(work_until.saturating_duration_since(Instant::now())),
                );
            }
        }

        fn run_native_capture_prepare_start(config_path: &Path) -> Result<()> {
            let case_path = config_path
                .parent()
                .ok_or_else(|| error("case parent required"))?
                .join("case.json");
            let case_bytes = super::files::read_private(&case_path, 65536)?;
            let case: DriverCase = serde_json::from_slice(&case_bytes).map_err(io_error)?;
            let approved = ApprovedCase::select(&case)?;
            ensure(
                case.schema_version == 1
                    && case.cleanup_reserve_ms == 4000
                    && case.config_path == config_path
                    && case.case_nonce.len() == 64,
                "private case identity differs",
            )?;
            let neutral = Path::new(DRIVER_NEUTRAL_ROOT);
            let private = Path::new(DRIVER_PRIVATE_ROOT);
            ensure(
                case.cwd == neutral.join(approved.id)
                    && case.database_url
                        == format!("sqlite:///{}", private.join(approved.database).display())
                    && case.tmux.socket_path == private.join(approved.socket)
                    && case.output_directory == neutral.join(format!("{}-capture", approved.id))
                    && case.helper.path == case.cwd.join("permission-probe"),
                "case resources differ from approved isolated paths",
            )?;
            for path in [
                &case.config_path,
                &case.arm_path,
                &case.producer_manifest_path,
                &case.launch_evidence_path,
                &case.driver_ownership_path,
                &case.started_record_path,
                &case.stop_path,
                &case.privacy_confirmation_path,
                &case.result_path,
                &case.descriptor_copy_path,
            ] {
                ensure(
                    path.parent() == Some(case.cwd.as_path()),
                    "driver output outside owned case directory",
                )?;
            }
            ensure(
                crate::coding_agent::coding_agent(&case.director_backend).is_some(),
                "known authorizing Director backend required",
            )?;
            for directory in [&case.cwd, &case.output_directory] {
                let metadata = std::fs::symlink_metadata(directory).map_err(io_error)?;
                ensure(
                    metadata.is_dir()
                        && metadata.uid() == nix::unistd::geteuid().as_raw()
                        && metadata.mode() & 0o777 == 0o700,
                    "private owned case directory required",
                )?;
            }
            validate_private_file(
                Path::new(crate::db::database_path(&case.database_url)?),
                nix::unistd::geteuid().as_raw(),
                0o600,
            )?;
            let sample = RealInputDeadlineClock.sample()?;
            let remaining = case
                .expires_at_mono_ns
                .checked_sub(sample.monotonic_ns)
                .filter(|n| {
                    *n > 4_000_000_000 && *n <= 120_000_000_000 + case.answer_window_ms * 1_000_000
                })
                .ok_or(CafleetError::DeadlineExceeded)?;
            let case_until = sample
                .before
                .checked_add(Duration::from_nanos(remaining))
                .ok_or_else(|| error("case clock range"))?;
            let work_until = case_until
                .checked_sub(Duration::from_secs(4))
                .ok_or_else(|| error("case cleanup reserve"))?;
            for reference in [
                &case.native_executable,
                &case.helper,
                &case.libtest,
                &case.source_manifest,
                &case.version_evidence,
                &case.environment_evidence,
                &case.event_redaction,
                &case.capture_policy,
            ] {
                reference.verify()?;
            }
            let version: DriverVersion = driver_read(&case.version_evidence.path)?;
            ensure(
                version.schema_version == 1
                    && version.exit_code == 0
                    && version.executable == case.native_executable.path
                    && version.sha256 == case.native_executable.sha256
                    && version.argv
                        == [
                            case.native_executable.path.to_string_lossy().into_owned(),
                            "--version".into(),
                        ]
                    && version.stdout.lines().count() == 1
                    && crate::coding_agent::version_token(&version.stdout)
                        == Ok(case.native_version.as_str()),
                "same-file actual version evidence differs",
            )?;
            ensure(
                std::env::current_exe().map_err(io_error)? == case.libtest.path,
                "selected libtest differs",
            )?;
            ensure(
                !case
                    .cwd
                    .join("allow.executed")
                    .try_exists()
                    .map_err(io_error)?
                    && !case
                        .cwd
                        .join("deny.executed")
                        .try_exists()
                        .map_err(io_error)?
                    && !case.arm_path.try_exists().map_err(io_error)?,
                "fresh case without probe markers or arm required",
            )?;
            let ownership = DriverOwnership {
                schema_version: 1,
                case_manifest_sha256: crate::digest::sha256_hex(&case_bytes),
                case_nonce: case.case_nonce.clone(),
                driver: ProcessIdentity::current()?,
                server: ProcessIdentity::read(case.tmux.server_pid)?
                    .ok_or_else(|| error("dedicated server absent"))?,
                tmux: case.tmux.clone(),
                expires_at_mono_ns: case.expires_at_mono_ns,
            };
            driver_validate_tmux(&case, &ownership, work_until)?;
            driver_publish(&case.driver_ownership_path, &ownership, work_until)?;
            let runner: Rc<dyn CommandRunner> = Rc::new(SystemRunner);
            let mux = resolve_multiplexer(Some("tmux"), std::env::vars().collect(), runner)
                .map_err(io_error)?;
            let context = MultiplexerContext {
                session: case.tmux.session.clone(),
                window_id: case.tmux.window_id.clone(),
                pane_id: case.tmux.pane_id.clone(),
            };
            let mut host = CaptureVerificationLaunchHost {
                inner: NativeLaunchHost::new(&mux, &context),
                case: &case,
                case_until,
            };
            let mut conn = crate::db::connect(&case.database_url)?;
            with_database_deadline(&mut conn, case_until, |conn| {
                crate::db::migrate_to_head(conn)?;
                let fleet = crate::broker::fleets::create_fleet(
                    conn,
                    Some(format!("native-{}", approved.id).as_str()),
                    &context.session,
                    &context.window_id,
                    &context.pane_id,
                    &case.director_backend,
                    "tmux",
                )?;
                let policy = approved
                    .backend
                    .resolve_policy(Some(&case.native_version), Some(approved.permission_mode))?;
                let capabilities = ProfileCatalog::bundled()?
                    .capabilities(approved.backend, Some(&case.native_version));
                let settings = RunSettings {
                    database_identity_json: storage::database_identity(conn)?,
                    creator: ownership.driver.clone(),
                    launch_nonce: case.case_nonce.clone(),
                    requested_mode: policy.requested_mode,
                    resolved_mode: policy.resolved_mode,
                    mode_source: policy.mode_source,
                    permission_argv_json: json!(policy.permission_argv),
                    event_evidence: storage::EventEvidence::Redacted,
                    native_version: Some(case.native_version.clone()),
                    profile_version: capabilities.profile_version,
                    coverage: capabilities.detection,
                };
                let member = PendingMemberSpec {
                    fleet_id: fleet.fleet_id,
                    name: approved.member_name(),
                    description: format!("private {} capture", case.mode),
                    placement: crate::broker::NewPlacement {
                        backend: "tmux".into(),
                        mux_session: context.session.clone(),
                        mux_window_id: context.window_id.clone(),
                        mux_pane_id: None,
                        coding_agent: approved.backend.name().into(),
                    },
                };
                let launch_until = work_until.min(Instant::now() + Duration::from_secs(10));
                let launched = create_registered(
                    conn,
                    &member,
                    &settings,
                    |conn, pending| driver_prepare(conn, pending, &case, launch_until),
                    &mut host,
                    launch_until,
                )?;
                let work = catch_unwind(AssertUnwindSafe(|| {
                    driver_work(conn, &case, &ownership, &launched, work_until)
                }));
                let (work_error, post_stage) = match &work {
                    Ok(Ok(post_stage)) => (None, post_stage.clone()),
                    Ok(Err(cause)) => (
                        Some(cause.message().chars().take(2048).collect::<String>()),
                        Value::Null,
                    ),
                    Err(_) => (Some("owned post-start work unwound".into()), Value::Null),
                };
                let cleanup = catch_unwind(AssertUnwindSafe(|| {
                    let clock = ShutdownClock::new(Instant::now()).clipped(case_until);
                    let shutdown = driver_validate_run(conn, &launched)
                        .and_then(|current| host.shutdown_owned(&current, clock));
                    let pane = launched
                        .placement
                        .mux_pane_id
                        .as_deref()
                        .expect("launched pane");
                    let observed = (|| {
                        driver_validate_pane(
                            &case,
                            &ownership,
                            &launched,
                            clock.storage_deadline(),
                        )?;
                        while launched.started.runner.matches()?
                            && Instant::now() < clock.storage_deadline()
                        {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        let service: Value =
                            driver_read(&case.config_path.with_extension("service-outcome.json"))?;
                        ensure(
                            service["config_sha256"]
                                == super::files::file_digest(&case.config_path)?
                                && service["run_id"] == launched.run.run_id
                                && service["launch_nonce"] == launched.run.launch_nonce,
                            "service outcome binding differs",
                        )?;
                        ensure(
                            driver_armed(&case)?,
                            "not_armed: the capture was never armed",
                        )?;
                        driver_validate_privacy(&case, approved, &ownership)?;
                        let capture =
                            driver_complete_capture(&case, approved, &launched, &service)?;
                        let pane_status = driver_tmux(
                            &case,
                            &[
                                "display-message",
                                "-p",
                                "-t",
                                pane,
                                "#{pane_dead}|#{pane_dead_status}",
                            ],
                            clock.storage_deadline(),
                        )?;
                        ensure(
                            pane_status.trim()
                                == format!(
                                    "1|{}",
                                    service["status"].as_i64().expect("validated status")
                                ),
                            "actual pane exit differs from service status",
                        )?;
                        Ok::<_, CafleetError>(
                            json!({"service":service,"pane_status":pane_status,"capture":capture}),
                        )
                    })();
                    let pane_capture = driver_validate_pane(
                        &case,
                        &ownership,
                        &launched,
                        clock.storage_deadline(),
                    )
                    .and_then(|()| {
                        driver_capture_runner_pane(&case, pane, clock.storage_deadline())
                    });
                    let close = driver_validate_pane(
                        &case,
                        &ownership,
                        &launched,
                        clock.storage_deadline(),
                    )
                    .and_then(|()| host.close_pane(pane, clock.storage_deadline()));
                    let runner_absent = launched.started.runner.matches().map(|exists| !exists);
                    let native_absent = launched.started.child.matches().map(|exists| !exists);
                    let summary = json!({"schema_version":1,"case_manifest_sha256":ownership.case_manifest_sha256,"work_error":work_error,
                "driver_ownership_sha256":super::files::file_digest(&case.driver_ownership_path)?,
                "shutdown":shutdown.as_ref().ok(),"shutdown_error":shutdown.as_ref().err().map(|e|e.message().into_owned()),
                "observation":observed.as_ref().ok(),"observation_error":observed.as_ref().err().map(|e|e.message().into_owned()),
                "pane_capture":pane_capture.as_ref().ok(),"pane_capture_error":pane_capture.as_ref().err().map(|e|e.message().into_owned()),
                "pane_closed":close.is_ok(),"close_error":close.as_ref().err().map(|e|e.message().into_owned()),
                "runner_absent":runner_absent.as_ref().ok(),"native_absent":native_absent.as_ref().ok(),
                "runner_identity_error":runner_absent.as_ref().err().map(|e|e.message().into_owned()),
                "native_identity_error":native_absent.as_ref().err().map(|e|e.message().into_owned()),
                "case_id":approved.id,"post_stage":post_stage,
                "requested_marker":approved.requested_marker(),
                "requested_marker_present":match approved.requested_marker() {
                    Some(marker) => Some(case.cwd.join(marker).try_exists().map_err(io_error)?),
                    None => None,
                },
                "allow_marker_absent":!case.cwd.join("allow.executed").try_exists().map_err(io_error)?,
                "deny_marker_absent":!case.cwd.join("deny.executed").try_exists().map_err(io_error)?});
                    driver_publish(&case.result_path, &summary, case_until)?;
                    let shutdown = shutdown?;
                    observed?;
                    close?;
                    let mut forbidden_absent = true;
                    for marker in approved.forbidden_markers() {
                        forbidden_absent &=
                            !case.cwd.join(marker).try_exists().map_err(io_error)?;
                    }
                    ensure(
                        shutdown.cleanup_state == storage::CleanupState::Complete
                            && runner_absent?
                            && native_absent?
                            && forbidden_absent,
                        "native case cleanup incomplete or a probe outside the reviewed decision executed",
                    )
                }));
                match work {
                    Err(panic) => {
                        if !matches!(cleanup, Ok(Ok(()))) {
                            eprintln!("native driver cleanup also failed");
                        }
                        resume_unwind(panic)
                    }
                    Ok(Err(primary)) => match cleanup {
                        Ok(Err(cause)) => Err(primary.with_cleanup(cause.message())),
                        Err(_) => Err(primary.with_cleanup("owned cleanup panicked")),
                        _ => Err(primary),
                    },
                    Ok(Ok(_)) => match cleanup {
                        Ok(result) => result,
                        Err(panic) => resume_unwind(panic),
                    },
                }
            })
        }

        #[test]
        #[ignore = "Director-owned native preparation requires an explicit private case"]
        fn native_capture_prepare_start_driver() {
            let config = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
                .expect("private capture configuration required");
            let config = Path::new(&config);
            assert!(
                config.is_absolute(),
                "absolute capture configuration required"
            );
            run_native_capture_prepare_start(config)
                .expect("actual native prepare/start/cleanup outcome");
        }

        pub(super) mod tests {
            use std::time::Duration;

            use serde_json::Value;

            use super::*;
            use crate::runtime::runner::service::test_support::*;
            #[test]
            fn capture_ignored_entry_requires_explicit_configuration_before_launching() {
                let directory = tempfile::tempdir().unwrap();
                let output_path = directory.path().join("entry-output");
                let output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&output_path)
                    .unwrap();
                let mut child = crate::runtime::system::test_support::TestChild::new(
                    std::process::Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            super::native_capture_service_child_path(),
                            "--ignored",
                            "--test-threads=1",
                        ])
                        .env_remove("CAFLEET_NATIVE_CAPTURE_CONFIG")
                        .stdout(std::process::Stdio::from(output.try_clone().unwrap()))
                        .stderr(std::process::Stdio::from(output))
                        .spawn()
                        .unwrap(),
                );
                let until = Instant::now() + Duration::from_secs(3);
                let status = child.poll_until(until - Duration::from_secs(1)).unwrap();
                if status.is_none() {
                    child.terminate_until(until);
                }
                assert!(
                    !status
                        .expect("configuration-free ignored entry completes within its bound")
                        .success()
                );
                assert!(std::fs::metadata(&output_path).unwrap().len() <= 65536);
                let bytes = std::fs::read(&output_path).unwrap();
                let combined = String::from_utf8_lossy(&bytes);
                assert!(
                    combined.contains("CAFLEET_NATIVE_CAPTURE_CONFIG"),
                    "{combined}"
                );
            }
            use crate::runtime::runner::service::test_support::observation_services::{
                capture, health, private_write, wait_until,
            };
            #[test]
            fn runner_collector_common_service_arms_on_actual_resize_and_preserves_trigger_geometry_and_sequence()
             {
                let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                    capture(fixture, spec);
                    private_write(
                        &fixture._dir.path().join("collector-review-entry"),
                        b"resize",
                    );
                });
                let started = fixture.start_running();
                let before = health(&mut fixture)["output_seq"].as_u64().unwrap();
                assert!(fixture.process.as_ref().unwrap().matches().unwrap());
                fixture.resize(25, 90);
                let pair_path = fixture
                    .fixture
                    ._dir
                    .path()
                    .join("capture-output/actual-resize-pair.json");
                wait_until(|| pair_path.exists());
                let pair: Value =
                    serde_json::from_slice(&std::fs::read(pair_path).unwrap()).unwrap();
                assert_eq!(pair["status"], "complete");
                let samples = pair["samples"].as_array().unwrap();
                assert_eq!(samples.len(), 2);
                assert_eq!(samples[0]["trigger"], "resize");
                assert_eq!(samples[1]["trigger"], "timer");
                for sample in samples {
                    assert_eq!(sample["rows"], 25);
                    assert_eq!(sample["cols"], 90);
                    assert_eq!(sample["screen_seq"], before + 1);
                }
                assert!(
                    samples[1]["monotonic_start_ns"]
                        .as_u64()
                        .unwrap()
                        .checked_sub(samples[0]["monotonic_end_ns"].as_u64().unwrap())
                        .unwrap()
                        >= 100_000_000
                );
                assert_eq!(pair["source"]["clock"]["kind"], "real");
                assert_eq!(fixture.health().child, Some(started.child));
                assert!(fixture.child.try_wait().unwrap().is_none());
                fixture.assert_manual_io();
                fixture.finish();
            }
            use crate::broker::runtime as storage;
            use crate::config::Settings;
            use std::os::unix::fs::OpenOptionsExt;
            use std::path::Path;
            use std::sync::atomic::{AtomicBool, Ordering};

            struct ResizeArm {
                receiver: Arc<Mutex<NativeCaptureReceiver>>,
                used: AtomicBool,
            }
            impl ObservationHooks for ResizeArm {
                fn before(
                    &self,
                    site: ObservationPhase,
                    operation: ObservationOperation,
                ) -> Result<()> {
                    if site == ObservationPhase::Resize
                        && operation == ObservationOperation::Detector
                        && !self.used.swap(true, Ordering::AcqRel)
                    {
                        let receiver = self.receiver.lock().unwrap();
                        let identity = receiver
                            .finalized_identity()
                            .expect("actual running resize source");
                        receiver.request(NativeCaptureRequest {
                            capture_id: "actual-resize".into(),
                            run_id: identity.run_id,
                            owner_epoch: identity.owner_epoch,
                            minimum_interval: Duration::from_millis(100),
                            deadline: Instant::now() + Duration::from_secs(2),
                        })?;
                    }
                    Ok(())
                }
            }
            pub(crate) fn runner_collector_resize_child_path() -> &'static str {
                test_path!(runner_collector_resize_child)
            }

            #[test]
            #[ignore = "Owned synthetic resize fixture selects this private entry"]
            fn runner_collector_resize_child() {
                let path = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
                    .expect("owned resize configuration");
                let limits = NativeCaptureLimits::default();
                let config = read_native_capture_config(Path::new(&path), &limits).unwrap();
                let settings = Settings::for_database(
                    Path::new(crate::db::database_path(&config.database_url).unwrap()),
                    None,
                );
                let conn = crate::db::connect(&settings.database_url).unwrap();
                let descriptor = load_descriptor(&conn, &config.descriptor_path).unwrap();
                let run = storage::get_run(&conn, descriptor.run_id).unwrap();
                assert_eq!(run.run_id, config.expected_run_id);
                assert_eq!(run.launch_nonce, config.expected_launch_nonce);
                let catalog = ProfileCatalog::bundled().unwrap();
                let policy =
                    NativeCapturePolicy::from_launch(&descriptor.spawn, &[], &catalog).unwrap();
                let (mut port, receiver) =
                    native_capture_channel(&run, policy, limits.clone()).unwrap();
                let producer =
                    read_native_capture_producer(&config.producer_manifest_path, &limits).unwrap();
                assert_eq!(producer.value["kind"], "synthetic");
                port.service_producer = Some(producer);
                let receiver = Arc::new(Mutex::new(receiver));
                let observer = Arc::new(ResizeArm {
                    receiver: receiver.clone(),
                    used: AtomicBool::new(false),
                });
                let stop = Arc::new(AtomicBool::new(false));
                let result_path = config.output_directory.join("actual-resize-pair.json");
                std::thread::scope(|scope| {
                    let stopped = stop.clone();
                    let collector = scope.spawn(move || -> Result<()> {
                        let until = Instant::now() + Duration::from_secs(8);
                        while !stopped.load(Ordering::Acquire) && Instant::now() < until {
                            if let Some(lease) = receiver.lock().unwrap().try_take()? {
                                let pair = serde_json::to_vec(lease.result()).map_err(io_error)?;
                                super::super::files::publish_private(&result_path, &pair, until)?;
                                return Ok(());
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error("owned resize pair was not completed"))
                    });
                    let status = run_service(
                        &settings,
                        &config.descriptor_path,
                        RunnerDependencies {
                            catalog,
                            clock: Arc::new(RealObservationClock),
                            writer_hooks: Arc::new(InertWriterHooks),
                            capture: Some(port),
                            observation_hooks: Some(observer.clone()),
                        },
                    );
                    stop.store(true, Ordering::Release);
                    collector.join().unwrap().unwrap();
                    assert!(observer.used.load(Ordering::Acquire));
                    assert_eq!(status.unwrap(), 0);
                });
            }
        }

        pub(crate) fn native_capture_service_child_path() -> &'static str {
            test_path!(native_capture_service_child)
        }

        #[test]
        #[ignore = "Director-owned native verification requires an explicit private configuration"]
        fn native_capture_service_child() {
            use std::io::Read;
            use std::os::unix::fs::OpenOptionsExt;
            let path = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG").expect(
                "CAFLEET_NATIVE_CAPTURE_CONFIG is required for explicit native verification",
            );
            let path = std::path::Path::new(&path);
            assert!(
                path.is_absolute(),
                "capture configuration path must be absolute"
            );
            let config = read_native_capture_config(path, &NativeCaptureLimits::default()).unwrap();
            let bounded_digest = |path: &std::path::Path| {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(nix::libc::O_NOFOLLOW)
                    .open(path)
                    .unwrap();
                let mut bytes = Vec::new();
                file.take(65537).read_to_end(&mut bytes).unwrap();
                assert!(
                    bytes.len() <= 65536,
                    "bounded private outcome evidence required"
                );
                sha256_hex(&bytes)
            };
            let config_sha256 = bounded_digest(path);
            let producer_sha256 = bounded_digest(&config.producer_manifest_path);
            let process = ProcessIdentity::current().unwrap();
            let result = run_native_capture_verification(path);
            let value = match &result {
                Ok(status) => serde_json::json!({"schema_version":1,"config_sha256":config_sha256,
            "run_id":config.expected_run_id,"launch_nonce":config.expected_launch_nonce,
            "producer_sha256":producer_sha256,"process":process,"result":"ok","status":status,"error":null}),
                Err(cause) => serde_json::json!({"schema_version":1,"config_sha256":config_sha256,
            "run_id":config.expected_run_id,"launch_nonce":config.expected_launch_nonce,
            "producer_sha256":producer_sha256,"process":process,"result":"error","status":null,
            "error":cause.message().chars().take(2048).collect::<String>()}),
            };
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(bytes.len() <= 16 * 1024);
            super::files::publish_private(
                &path.with_extension("service-outcome.json"),
                &bytes,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            match result {
                Ok(0) => {}
                Ok(status) => std::process::exit(status),
                Err(cause) => panic!("native capture service returned an error: {cause}"),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use crate::broker::runtime as storage;
    use crate::runtime::runner::SpawnSpec;
    use crate::runtime::runner::service::test_support::observation_services::private_write;
    use crate::runtime::runner::test_support::Fixture;
    pub(crate) fn synthetic(fixture: &mut Fixture, spec: &mut SpawnSpec) {
        spec.environment.push((
            "CAFLEET_REVIEW_NATIVE_START".into(),
            fixture
                ._dir
                .path()
                .join("native-started")
                .to_str()
                .unwrap()
                .into(),
        ));
        fixture.conn.execute("UPDATE member_runs SET native_version='0.0.1-synthetic',profile_version='synthetic-profile-1',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let script = fixture._dir.path().join("owned-observation-process");
        private_write(&script,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf '0.0.1-synthetic\\n'; exit 0; fi\n/usr/bin/touch \"$CAFLEET_REVIEW_NATIVE_START\"\nprintf 'review-native-ready\\n'\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; form*) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write %s.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: ' \"$line\" ;; transient) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write transient.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \\033[2J\\033[Hworking' ;; *) printf 'review-answer=%s\\n' \"$line\" ;; esac; done\n");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        spec.argv = vec![script.to_str().unwrap().into(), "--approve-for-me".into()];
        let definitions =
            crate::coding_agent::profiles::catalog::test_support::synthetic_definitions();
        private_write(
            &fixture._dir.path().join("observation-services.json"),
            &serde_json::to_vec(&json!({"catalog":definitions})).unwrap(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use crate::broker::runtime as storage;
    use crate::runtime::runner::service::test_support::*;

    mod observation_contract {
        use super::super::test_support::synthetic;
        use super::*;
        use crate::runtime::runner::service::test_support::observation_services::{
            count, health, wait_until,
        };
        #[test]
        fn service_quiet_timer_persists_one_opening_without_changing_output_sequence() {
            let mut fixture = ServiceFixture::configured(false, synthetic);
            fixture.start_running();
            fixture.master.write_all(b"form-one\r").unwrap();
            fixture.read_until("Operation: write form-one.txt");
            let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
            wait_until(|| count(&fixture, "interaction_opened") == 1);
            let after = health(&mut fixture);
            assert_eq!(after["output_seq"], sequence);
            assert_eq!(after["observation"]["visibility"], "visible");
            assert_eq!(after["observation"]["eligibility"]["state"], "hold");
            let messages:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events e JOIN messages m ON m.message_id=e.message_id WHERE e.run_id=?1 AND e.kind='interaction_opened'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
            assert_eq!(messages, 1);
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn service_transient_form_and_clear_in_one_native_write_produce_no_episode() {
            let mut fixture = ServiceFixture::configured(false, synthetic);
            fixture.start_running();
            fixture.master.write_all(b"transient\r").unwrap();
            fixture.read_until("working");
            let until = Instant::now() + Duration::from_millis(180);
            while Instant::now() < until {
                let status = health(&mut fixture);
                assert_eq!(status["observation"]["eligibility"]["state"], "hold");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(count(&fixture, "interaction_opened"), 0);
            assert_eq!(
                fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                        [fixture.fixture.run.run_id],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            fixture.assert_manual_io();
            fixture.finish();
        }
        #[test]
        fn service_shutdown_uses_the_active_controller_to_close_before_truthful_exit() {
            let mut fixture = ServiceFixture::configured(false, synthetic);
            fixture.start_running();
            fixture.master.write_all(b"form-shutdown\r").unwrap();
            fixture.read_until("Operation: write form-shutdown.txt");
            wait_until(|| count(&fixture, "interaction_opened") == 1);
            fixture.finish();
            let kinds = crate::runtime::runner::service::test_support::observation_services::kinds(
                &fixture.fixture.conn,
                fixture.fixture.run.run_id,
            );
            assert_eq!(
                kinds,
                ["interaction_opened", "interaction_closed", "runner_exited"]
            );
            let reason: String = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT close_reason FROM member_interactions WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(reason, "process_exited");
        }
        #[test]
        fn service_later_version_profile_emits_no_coverage_unknown_while_no_profile_still_does() {
            let reused = crate::coding_agent::ProfileCatalog::bundled()
                .unwrap()
                .capabilities(crate::coding_agent::CodingAgent::Codex, Some("0.164.0"));
            let reused_profile = reused.profile_version.unwrap();
            for (profile, expected) in [(Some(reused_profile), 0), (None, 1)] {
                let selected = profile.clone();
                let mut fixture = ServiceFixture::configured(false, move |fixture, _| {
                    if let Some(selected) = &selected {
                        fixture.conn.execute("UPDATE member_runs SET native_version='0.164.0',profile_version=?1,coverage='tested' WHERE run_id=?2",rusqlite::params![selected,fixture.run.run_id]).unwrap();
                    } else {
                        fixture.conn.execute("UPDATE member_runs SET native_version=NULL,profile_version=NULL,coverage='unknown' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
                    }
                    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                });
                fixture.start_running();
                for _ in 0..3 {
                    fixture.health();
                    fixture.assert_manual_io();
                }
                fixture.finish();
                let count:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='coverage_unknown'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
                assert_eq!(count, expected, "{profile:?}");
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert_eq!(run.profile_version, profile, "{profile:?}");
                if profile.is_none() {
                    assert_eq!(run.resolved_mode, "approve-for-me");
                    assert!(run.effective_policy_json.is_none());
                }
            }
        }
    }

    mod injected_failures {
        use super::*;
        use crate::runtime::runner::service::test_support::injected_failures::*;
        use crate::runtime::runner::service::test_support::observation_services::{
            count, health, wait_until,
        };
        #[test]
        fn runner_resize_detector_failure_keeps_actual_resized_service_and_native_child() {
            let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                configure(fixture, spec, "resize", "detector")
            });
            let started = fixture.start_running();
            fixture.master.write_all(b"form-resize-failure\r").unwrap();
            fixture.read_until("Operation: write form-resize-failure.txt");
            wait_until(|| count(&fixture, "interaction_opened") == 1);
            let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
            arm(&fixture);
            assert!(fixture.process.as_ref().unwrap().matches().unwrap());
            fixture.resize(25, 90);
            assert_fired(&fixture, "Resize/Detector");
            let status = fixture.health();
            assert_eq!(status.child, Some(started.child));
            assert!(health(&mut fixture)["output_seq"].as_u64().unwrap() > sequence);
            assert_live_after_fault(&mut fixture, "detector_unavailable");
            fixture.finish();
            assert_private_cause_stays_local(&fixture);
        }
        #[test]
        fn runner_child_output_parser_and_detector_failures_keep_transport_and_typed_metadata() {
            for (point, operation, form, fired, diagnostic) in [
                (
                    "child_output",
                    "parser",
                    "form-output-failure",
                    "ChildOutput/Parser",
                    "parser_unavailable",
                ),
                (
                    "child_output",
                    "detector",
                    "form-output-failure",
                    "ChildOutput/Detector",
                    "detector_unavailable",
                ),
                (
                    "quiet_timer",
                    "detector",
                    "form-quiet-failure",
                    "QuietTimer/Detector",
                    "detector_unavailable",
                ),
            ] {
                let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                    configure(fixture, spec, point, operation)
                });
                fixture.start_running();
                arm(&fixture);
                fixture
                    .master
                    .write_all(format!("{form}\r").as_bytes())
                    .unwrap();
                fixture.read_until(&format!("Operation: write {form}.txt"));
                assert_fired(&fixture, fired);
                assert_live_after_fault(&mut fixture, diagnostic);
                fixture.finish();
                assert_private_cause_stays_local(&fixture);
            }
        }
    }
    mod inspection_tests {
        use super::super::*;
        use crate::runtime::runner::test_support::Fixture;

        #[test]
        fn permission_inspection_request_generation_exhaustion_invalidates_without_wrap() {
            use crate::coding_agent::profiles::catalog::test_support::{
                inspection_fixture_definition, inspection_fixture_form,
            };
            use crate::runtime::system::test_support::TestChild;
            use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
            use std::sync::Arc;

            let until = Instant::now() + Duration::from_secs(5);
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let lock = Arc::new(
                RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap(),
            );
            lock.transfer_to_runner(
                &mut fixture.conn,
                &fixture.run.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner()).unwrap();
            fixture.conn.execute("UPDATE member_runs SET native_version='0.0.1-synthetic',profile_version='synthetic-profile-1',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let catalog =
                ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
            let mut controller = ObservationController::new(
                &fixture.run,
                &catalog,
                EvidencePolicy::metadata(),
                1..=256,
            )
            .unwrap();
            let mut writer = EventWriter::start(
                format!("sqlite:///{}", fixture.conn.path().unwrap()),
                fixture.run.clone(),
                lock,
                artifacts.paths.outbox.clone(),
            )
            .unwrap();
            writer.set_deadline(until);
            let mut child = TestChild::spawn(std::path::Path::new("/bin/cat"), &[]);
            let identity = child.identity();
            assert!(identity.matches().unwrap());
            fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![identity.pid,identity.start_identity,crate::time::now_stamp(),fixture.run.run_id]).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            let primary = catch_unwind(AssertUnwindSafe(|| {
                controller
                    .started(
                        &fixture.run,
                        &terminal.snapshot(),
                        &writer,
                        &RealObservationClock,
                    )
                    .unwrap();
                terminal
                    .feed(&inspection_fixture_form(
                        "overflow current request",
                        "synthetic-only",
                    ))
                    .unwrap();
                controller
                    .observer(&writer, &RealObservationClock)
                    .observe(&terminal.snapshot())
                    .unwrap();
                loop {
                    controller
                        .poll(&terminal.snapshot(), &writer, &RealObservationClock)
                        .unwrap();
                    if controller.current_request().is_some() {
                        break;
                    }
                    assert!(
                        Instant::now() < until,
                        "actual current authority before original fixture deadline"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert!(controller.tracker.active_episode().is_some());
                assert!(controller.current_request().is_some());
                controller.request_generation = u64::MAX;
                assert!(
                    controller
                        .observer_failed(ObservationFailure::Parser, &RealObservationClock)
                        .is_err()
                );
                assert_eq!(controller.request_generation(), u64::MAX);
                assert!(controller.current_request().is_none());
                terminal
                    .feed(&inspection_fixture_form(
                        "overflow current request",
                        "synthetic-only",
                    ))
                    .unwrap();
                let observed = Detector::new(
                    &catalog,
                    crate::coding_agent::CodingAgent::Codex,
                    Some("0.0.1-synthetic"),
                )
                .observe(&terminal.snapshot())
                .unwrap();
                assert_eq!(
                    observed.match_evidence,
                    crate::coding_agent::MatchEvidence::Complete
                );
                assert_eq!(
                    observed.kind,
                    crate::coding_agent::ObservationKind::Interaction(
                        storage::InteractionClass::PermissionRequest
                    )
                );
                let later = controller
                    .observer(&writer, &RealObservationClock)
                    .observe(&terminal.snapshot());
                if let Err(cause) = later {
                    assert!(
                        cause.message().contains("generation")
                            || cause.message().contains("exhaust"),
                        "explicit authority fence: {cause}"
                    );
                }
                assert_eq!(controller.request_generation(), u64::MAX);
                assert!(
                    controller.current_request().is_none(),
                    "a later genuine complete screen cannot restore exhausted authority"
                );
                let stable_at = Instant::now() + Duration::from_millis(110);
                assert!(
                    stable_at < until,
                    "later stability observation retains the original cap"
                );
                std::thread::sleep(stable_at.saturating_duration_since(Instant::now()));
                let later = controller.poll(&terminal.snapshot(), &writer, &RealObservationClock);
                if let Err(cause) = later {
                    assert!(
                        cause.message().contains("generation")
                            || cause.message().contains("exhaust"),
                        "explicit authority fence: {cause}"
                    );
                }
                assert_eq!(controller.request_generation(), u64::MAX);
                assert!(
                    controller.current_request().is_none(),
                    "stable later complete evidence stays fenced"
                );
                assert!(
                    controller
                        .observer_failed(ObservationFailure::Detector, &RealObservationClock)
                        .is_err()
                );
                assert_eq!(controller.request_generation(), u64::MAX);
                assert!(controller.current_request().is_none());
            }));
            let cleanup = catch_unwind(AssertUnwindSafe(|| {
                let counts = writer.finish(until);
                let status = writer.status();
                child.terminate_until(until);
                assert!(
                    child.try_wait().unwrap().is_some(),
                    "owned synthetic child reap confirmed"
                );
                assert_eq!(counts, (0, 0), "{status:?}");
                assert!(
                    !status.degraded && status.diagnostic.is_none(),
                    "{status:?}"
                );
            }));
            if let Err(primary) = primary {
                if let Err(cleanup) = &cleanup {
                    eprintln!("inspection overflow cleanup also failed: {cleanup:?}");
                }
                resume_unwind(primary);
            }
            if let Err(cleanup) = cleanup {
                resume_unwind(cleanup);
            }
        }

        #[test]
        fn permission_inspection_observed_authority_has_no_debug_or_serialized_representation() {
            assert_no_debug_or_serialize!(ObservedRequest);
        }
    }
    mod controller_tests {
        use super::super::*;
        use crate::broker::runtime::{self as storage, *};
        use crate::coding_agent::ProfileCatalog;
        use crate::runtime::runner::writer::test_support::*;
        use crate::runtime::system::test_support::TestChild;
        use std::os::unix::process::ExitStatusExt;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration, Instant};

        /// A controller started for a running `/bin/cat` native child on a 24×80 terminal.
        fn started_controller(
            fixture: &mut WriterFixture,
            writer: &EventWriter,
            clock: &dyn ObservationClock,
            catalog: &ProfileCatalog,
            policy: storage::evidence::EvidencePolicy,
        ) -> (ObservationController, TestChild, TerminalState) {
            let mut controller =
                ObservationController::new(&fixture.fixture.run, catalog, policy, 1..=256).unwrap();
            let child = TestChild::spawn(std::path::Path::new("/bin/cat"), &[]);
            running(fixture, &child.identity());
            let terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            controller
                .started(&fixture.fixture.run, &terminal.snapshot(), writer, clock)
                .unwrap();
            (controller, child, terminal)
        }

        mod bounded_updates {
            use super::*;

            fn draft(fixture: &WriterFixture, kind: RuntimeEventKind) -> TransitionDraft {
                TransitionDraft {
                    run_id: fixture.fixture.run.run_id,
                    episode_seq: None,
                    kind,
                    classification: None,
                    observed_at: NOW.into(),
                    evidence_json: serde_json::json!({}),
                    close_reason: None,
                    cause_json: None,
                }
            }

            fn update(transitions: Vec<TransitionDraft>) -> EpisodeUpdate {
                EpisodeUpdate {
                    transitions,
                    ..EpisodeUpdate::new(Visibility::NotVisible, InputEligibility::ComposerEmpty)
                }
            }

            fn controller(fixture: &WriterFixture) -> ObservationController {
                ObservationController::new(
                    &fixture.fixture.run,
                    &catalog(),
                    storage::evidence::EvidencePolicy::metadata(),
                    1..=256,
                )
                .unwrap()
            }

            #[test]
            fn runner_policy_bounded_update_counts_only_primary_transitions() {
                let mut fixture = WriterFixture::new();
                configure(&mut fixture);
                let primary = || {
                    vec![
                        draft(&fixture, RuntimeEventKind::InteractionClosed),
                        draft(&fixture, RuntimeEventKind::InteractionOpened),
                    ]
                };
                let mut annotated = primary();
                annotated.extend([
                    draft(&fixture, RuntimeEventKind::AutomaticRefusalObserved),
                    draft(&fixture, RuntimeEventKind::AutomaticRefusalObserved),
                    draft(&fixture, RuntimeEventKind::AutomaticReviewUnavailable),
                ]);
                let mut accepted = controller(&fixture);
                accepted.retain(update(annotated)).unwrap();
                assert_eq!(
                    accepted.pending.as_ref().unwrap().update.transitions.len(),
                    5
                );
                let mut excess = primary();
                excess.push(draft(&fixture, RuntimeEventKind::InteractionObscured));
                let mut rejected = controller(&fixture);
                let failure = rejected.retain(update(excess)).err().unwrap();
                assert!(
                    failure
                        .message()
                        .contains("one bounded observation update required"),
                    "{failure:?}"
                );
                assert!(rejected.pending.is_none());
            }

            #[test]
            fn runner_policy_an_update_carrying_only_an_effective_policy_is_retained_for_the_writer()
             {
                let mut fixture = WriterFixture::new();
                configure(&mut fixture);
                let mut only_policy = controller(&fixture);
                only_policy
                    .retain(EpisodeUpdate {
                        effective_policy: Some(storage::EffectivePolicyObservation {
                            automatic_review: storage::AutomaticReview::Available,
                            observed_mode: None,
                            rule_id: "codex-auto-review-indicator".into(),
                            observed_at: NOW.into(),
                        }),
                        ..update(Vec::new())
                    })
                    .unwrap();
                assert!(only_policy.pending.is_some());
                let mut nothing = controller(&fixture);
                nothing.retain(update(Vec::new())).unwrap();
                assert!(nothing.pending.is_none());
            }
        }

        mod failure_handling {
            use super::*;

            mod observation_failures {
                use super::*;
                use std::sync::atomic::AtomicBool;

                struct Fault {
                    site: ObservationPhase,
                    armed: AtomicBool,
                    calls: AtomicU64,
                }
                impl Fault {
                    fn new(site: ObservationPhase) -> Self {
                        Self {
                            site,
                            armed: AtomicBool::new(false),
                            calls: AtomicU64::new(0),
                        }
                    }
                    fn arm(&self) {
                        self.armed.store(true, Ordering::Release);
                    }
                }
                impl ObservationHooks for Fault {
                    fn before(
                        &self,
                        site: ObservationPhase,
                        operation: ObservationOperation,
                    ) -> Result<()> {
                        if site == self.site && operation == ObservationOperation::Detector {
                            self.calls.fetch_add(1, Ordering::AcqRel);
                            if self.armed.swap(false, Ordering::AcqRel) {
                                return Err(error("private detector failure canary"));
                            }
                        }
                        Ok(())
                    }
                }

                #[test]
                fn runner_observation_hooks_install_once_before_the_matching_native_start() {
                    let mut fixture = WriterFixture::new();
                    configure(&mut fixture);
                    let mut controller = ObservationController::new(
                        &fixture.fixture.run,
                        &catalog(),
                        storage::evidence::EvidencePolicy::metadata(),
                        1..=256,
                    )
                    .unwrap();
                    controller
                        .set_observation_hooks(Arc::new(Fault::new(ObservationPhase::QuietTimer)))
                        .unwrap();
                    assert!(
                        controller
                            .set_observation_hooks(Arc::new(Fault::new(
                                ObservationPhase::QuietTimer
                            )))
                            .is_err()
                    );
                    let mut case = LiveCase::new();
                    assert!(
                        case.controller
                            .set_observation_hooks(Arc::new(Fault::new(
                                ObservationPhase::QuietTimer
                            )))
                            .is_err()
                    );
                }

                #[test]
                fn runner_quiet_detector_failure_returns_typed_held_status_and_preserves_the_episode()
                 {
                    let fault = Arc::new(Fault::new(ObservationPhase::QuietTimer));
                    let mut case = LiveCase::with_observation_hooks(Some(fault.clone()));
                    case.clock.set(200);
                    case.terminal
                        .feed(&form("write candidate-after-open.txt"))
                        .unwrap();
                    case.controller
                        .observer(&case.writer, &case.clock)
                        .observe(&case.terminal.snapshot())
                        .unwrap();
                    let sequence = case.terminal.snapshot().screen_seq;
                    let calls = fault.calls.load(Ordering::Acquire);
                    fault.arm();
                    case.clock.set(300);
                    let status = case
                        .controller
                        .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                        .expect("recoverable quiet detector failure preserves the live controller");
                    assert_eq!(fault.calls.load(Ordering::Acquire), calls + 1);
                    assert!(!fault.armed.load(Ordering::Acquire));
                    assert_eq!(status.diagnostic.as_deref(), Some("detector_unavailable"));
                    assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
                    assert!(status.gap_count > 0);
                    assert_eq!(case.terminal.snapshot().screen_seq, sequence);
                    let active = case.events()[0].interaction.clone().unwrap();
                    assert_eq!(active.episode_seq, 1);
                    assert!(active.closed_at.is_none());
                    assert_eq!(
                    case.fixture.fixture.conn.query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                        [case.fixture.fixture.run.run_id],
                        |row| row.get::<_, i64>(0),
                    ).unwrap(),
                    1
                );
                    assert!(case.child.identity().matches().unwrap());
                }

                #[test]
                fn runner_ordered_adapter_preserves_detector_attribution_and_bounded_metadata() {
                    let fault = Arc::new(Fault::new(ObservationPhase::ChildOutput));
                    let mut case = LiveCase::with_observation_hooks(Some(fault.clone()));
                    fault.arm();
                    case.clock.set(200);
                    let calls = fault.calls.load(Ordering::Acquire);
                    let _attempt = case
                        .controller
                        .observer(&case.writer, &case.clock)
                        .observe(&case.terminal.snapshot());
                    assert_eq!(fault.calls.load(Ordering::Acquire), calls + 1);
                    assert!(!fault.armed.load(Ordering::Acquire));
                    let status = case.controller.status();
                    assert_eq!(status.diagnostic.as_deref(), Some("detector_unavailable"));
                    assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
                    assert!(status.gap_count > 0);
                    assert_eq!(
                    case.fixture.fixture.conn.query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                        [case.fixture.fixture.run.run_id],
                        |row| row.get::<_, i64>(0),
                    ).unwrap(),
                    1
                );
                    assert!(
                        case.events()[0]
                            .interaction
                            .as_ref()
                            .unwrap()
                            .closed_at
                            .is_none()
                    );
                    assert!(case.child.identity().matches().unwrap());
                }

                #[test]
                fn runner_clock_and_output_sequence_invariants_propagate_outside_recoverable_failures()
                 {
                    for invariant in ["clock", "sequence"] {
                        let fault = Arc::new(Fault::new(ObservationPhase::ChildOutput));
                        let mut case = LiveCase::with_observation_hooks(Some(fault));
                        let before = case.controller.status().gap_count;
                        let result = if invariant == "clock" {
                            case.clock.set(99);
                            case.controller
                                .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                                .map(|_| ())
                        } else {
                            case.clock.set(200);
                            let mut stale = case.terminal.snapshot();
                            stale.screen_seq = 0;
                            case.controller
                                .observer(&case.writer, &case.clock)
                                .observe(&stale)
                        };
                        let cause = result.expect_err(
                            "clock/sequence corruption remains an explicit invariant error",
                        );
                        assert!(cause.message().contains("regress"), "{invariant}: {cause}");
                        assert_eq!(case.controller.status().gap_count, before);
                        assert_eq!(case.controller.status().diagnostic, None);
                        assert!(
                            case.events()[0]
                                .interaction
                                .as_ref()
                                .unwrap()
                                .closed_at
                                .is_none()
                        );
                    }
                }
            }

            struct Timeline {
                origin: Instant,
                millis: AtomicU64,
            }
            impl Timeline {
                fn new() -> Self {
                    Self {
                        origin: Instant::now(),
                        millis: AtomicU64::new(0),
                    }
                }
                fn set(&self, millis: u64) {
                    self.millis.store(millis, Ordering::Release);
                }
                fn timestamp(millis: u64) -> String {
                    let origin = chrono::DateTime::parse_from_rfc3339(NOW)
                        .unwrap()
                        .with_timezone(&chrono::Utc);
                    crate::time::format_utc(
                        origin + chrono::Duration::milliseconds(i64::try_from(millis).unwrap()),
                    )
                }
            }
            impl ObservationClock for Timeline {
                fn sample(&self) -> ObservationMoment {
                    let millis = self.millis.load(Ordering::Acquire);
                    ObservationMoment {
                        monotonic: self.origin + Duration::from_millis(millis),
                        observed_at: Self::timestamp(millis),
                    }
                }
                fn kind(&self) -> ObservationClockKind {
                    ObservationClockKind::Deterministic
                }
            }

            struct LiveCase {
                writer: EventWriter,
                controller: ObservationController,
                hooks: Arc<Hooks>,
                child: TestChild,
                terminal: TerminalState,
                clock: Timeline,
                fixture: WriterFixture,
            }
            impl LiveCase {
                fn new() -> Self {
                    Self::with_observation_hooks(None)
                }
                fn with_observation_hooks(
                    observation_hooks: Option<Arc<dyn ObservationHooks>>,
                ) -> Self {
                    Self::build(
                        observation_hooks,
                        configure,
                        storage::evidence::EvidencePolicy::metadata(),
                        form("write terminal-order.txt"),
                    )
                }
                fn refreshable() -> Self {
                    Self::build(
                        None,
                        configure_redacted,
                        redacted_policy(),
                        detailed_form("write terminal-order.txt", "opened"),
                    )
                }
                fn build(
                    observation_hooks: Option<Arc<dyn ObservationHooks>>,
                    configure_run: fn(&mut WriterFixture),
                    policy: storage::evidence::EvidencePolicy,
                    opening: Vec<u8>,
                ) -> Self {
                    let mut fixture = WriterFixture::new();
                    configure_run(&mut fixture);
                    let hooks = Arc::new(Hooks::default());
                    let writer = start(&fixture, hooks.clone());
                    let mut controller = ObservationController::new(
                        &fixture.fixture.run,
                        &catalog(),
                        policy,
                        1..=256,
                    )
                    .unwrap();
                    if let Some(hooks) = observation_hooks {
                        controller.set_observation_hooks(hooks).unwrap();
                    }
                    let child = TestChild::spawn(std::path::Path::new("/bin/cat"), &[]);
                    running(&mut fixture, &child.identity());
                    let mut terminal =
                        TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
                    let clock = Timeline::new();
                    controller
                        .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
                        .unwrap();
                    terminal
                        .feed_observed(&opening, &mut controller.observer(&writer, &clock))
                        .unwrap();
                    clock.set(100);
                    controller
                        .poll(&terminal.snapshot(), &writer, &clock)
                        .unwrap();
                    wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
                    Self {
                        writer,
                        controller,
                        hooks,
                        child,
                        terminal,
                        clock,
                        fixture,
                    }
                }
                fn fail_refresh(&mut self) -> Release {
                    self.hooks.pause(WriterBoundary::Persist);
                    let release = Release(self.hooks.clone());
                    self.clock.set(200);
                    self.terminal
                        .feed_observed(
                            &detail_rewrite("refreshed"),
                            &mut self.controller.observer(&self.writer, &self.clock),
                        )
                        .unwrap();
                    self.hooks.wait(WriterBoundary::Persist, 2);
                    wait_until(|| self.writer.status().degraded);
                    assert!(self.writer.status().pending_evidence);
                    assert_eq!(committed(&self.fixture), 1);
                    release
                }
                fn exit(&mut self, millis: u64) -> u64 {
                    assert!(self.child.identity().matches().unwrap());
                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(
                            i32::try_from(self.child.identity().pid).unwrap(),
                        ),
                        nix::sys::signal::Signal::SIGTERM,
                    )
                    .unwrap();
                    let until = deadline();
                    loop {
                        if let Some(exit) = self.child.try_wait().unwrap() {
                            assert_eq!(exit.signal(), Some(15));
                            break;
                        }
                        assert!(Instant::now() < until, "owned child exit deadline");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    self.clock.set(millis);
                    let sequence = self.terminal.snapshot().screen_seq;
                    self.controller
                        .child_exited(
                            ChildExit::Signal(15),
                            &self.terminal.snapshot(),
                            &self.writer,
                            &self.clock,
                        )
                        .unwrap();
                    sequence
                }
                fn drain(&mut self, until: Instant) -> (u64, u64) {
                    self.writer.set_deadline(until);
                    while Instant::now() < until {
                        self.controller
                            .poll(&self.terminal.snapshot(), &self.writer, &self.clock)
                            .unwrap();
                        if !self.controller.has_pending() && self.writer.status().pending_count == 0
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    self.writer.finish(until)
                }
                fn events(&self) -> Vec<storage::RuntimeEventView> {
                    let ids: Vec<i64> = self
                    .fixture
                    .fixture
                    .conn
                    .prepare(
                        "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
                    )
                    .unwrap()
                    .query_map([self.fixture.fixture.run.run_id], |row| row.get(0))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                    ids.into_iter()
                        .map(|id| storage::get_event(&self.fixture.fixture.conn, id).unwrap())
                        .collect()
                }
            }
            impl Drop for LiveCase {
                fn drop(&mut self) {
                    let unfinished = self
                        .writer
                        .finish(Instant::now() + Duration::from_millis(250));
                    if unfinished != (0, 0) {
                        eprintln!("live case fixture retained writer work: {unfinished:?}");
                    }
                }
            }

            #[test]
            fn runner_degraded_exit_records_gap_before_original_closure_and_terminal_fence() {
                let mut case = LiveCase::refreshable();
                let release = case.fail_refresh();
                let exit_sequence = case.exit(300);
                assert!(case.controller.has_pending());
                assert!(case.controller.unsaved_gap_count() > 0);
                assert!(matches!(
                    case.controller.status().eligibility,
                    InputEligibility::Hold(_)
                ));
                case.clock.set(700);
                drop(release);
                let unfinished = case.drain(deadline());
                let events = case.events();
                let kinds: Vec<_> = events.iter().map(|view| view.event.kind).collect();
                assert_eq!(
                    kinds,
                    [
                        RuntimeEventKind::InteractionOpened,
                        RuntimeEventKind::RunnerDegraded,
                        RuntimeEventKind::RunnerDegraded,
                        RuntimeEventKind::InteractionClosed,
                        RuntimeEventKind::RunnerExited,
                    ]
                );
                assert_eq!(unfinished, (0, 0));
                assert!(!case.controller.has_pending());
                assert_eq!(case.controller.unsaved_gap_count(), 0);
                assert_eq!(
                    events[1].event.payload_json["cause"]["kind"],
                    "storage_pressure"
                );
                assert_eq!(events[1].event.observed_at, Timeline::timestamp(300));
                assert_eq!(
                    events[2].event.payload_json["cause"]["kind"],
                    "storage_recovered"
                );
                assert_eq!(events[2].event.observed_at, Timeline::timestamp(700));
                for view in &events[3..] {
                    assert_eq!(view.event.observed_at, Timeline::timestamp(300));
                    assert_eq!(
                        view.event.payload_json["evidence"]["screen_seq"],
                        exit_sequence
                    );
                }
                assert_eq!(
                    events[3].event.payload_json["close_reason"],
                    "process_exited"
                );
                assert_eq!(events[4].event.payload_json["cause"]["exit_signal"], 15);
                let run =
                    storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
                        .unwrap();
                assert_eq!(run.status, RunStatus::Exited);
                assert_eq!(run.exit_signal, Some(15));
                for _ in 0..3 {
                    case.controller
                        .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                        .unwrap();
                }
                assert_eq!(case.events().len(), 5);
                assert!(!case.controller.has_pending());
            }

            #[test]
            fn runner_expired_storage_budget_reports_unsaved_gap_and_pending_exit_work() {
                let mut case = LiveCase::refreshable();
                let _release = case.fail_refresh();
                let until = Instant::now() + Duration::from_millis(120);
                case.writer.set_deadline(until);
                case.exit(300);
                let begin = Instant::now();
                let unfinished = case.drain(until);
                assert!(begin.elapsed() < Duration::from_millis(500));
                assert!(case.controller.has_pending() || unfinished.0 > 0);
                assert!(case.controller.unsaved_gap_count() > 0 || unfinished.1 > 0);
                assert_eq!(committed(&case.fixture), 1);
                let run =
                    storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
                        .unwrap();
                assert_eq!(run.status, RunStatus::Running);
                assert!(case.child.try_wait().unwrap().is_some());
                assert_eq!(
                    case.events()[0].event.kind,
                    RuntimeEventKind::InteractionOpened
                );
            }

            #[test]
            fn runner_failure_after_terminal_enqueue_never_appends_a_live_post_terminal_gap() {
                let mut case = LiveCase::new();
                case.hooks.pause(WriterBoundary::Persist);
                let release = Release(case.hooks.clone());
                case.exit(300);
                case.hooks.wait(WriterBoundary::Persist, 2);
                wait_until(|| case.writer.status().degraded);
                case.clock.set(500);
                case.controller
                    .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                    .unwrap();
                case.clock.set(700);
                drop(release);
                let unfinished = case.drain(deadline());
                let events = case.events();
                assert_eq!(unfinished, (0, 0));
                assert!(!case.controller.has_pending());
                assert_eq!(case.controller.unsaved_gap_count(), 0);
                assert_eq!(case.controller.status().gap_count, 0);
                let run =
                    storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
                        .unwrap();
                assert_eq!(run.observation_gap_count, 0);
                assert_eq!(
                    events
                        .iter()
                        .map(|view| view.event.kind)
                        .collect::<Vec<_>>(),
                    [
                        RuntimeEventKind::InteractionOpened,
                        RuntimeEventKind::InteractionClosed,
                        RuntimeEventKind::RunnerExited,
                    ]
                );
                assert_eq!(
                    events.last().unwrap().event.kind,
                    RuntimeEventKind::RunnerExited
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|view| view.event.kind == RuntimeEventKind::RunnerExited)
                        .count(),
                    1
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|view| view.event.kind == RuntimeEventKind::InteractionClosed)
                        .count(),
                    1
                );
                assert_eq!(
                    events.last().unwrap().event.observed_at,
                    Timeline::timestamp(300)
                );
                for _ in 0..3 {
                    case.controller
                        .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                        .unwrap();
                }
                assert_eq!(case.events().len(), events.len());
                assert!(!case.controller.has_pending());
            }
        }

        // Draining 64 retained pressure transitions through SQLite exceeds the shared 2 s budget under a
        // concurrent full suite on slower storage.
        const PRESSURE_DRAIN_BUDGET: Duration = Duration::from_secs(10);

        fn assert_retained_pressure<T>(attempt: Result<T>, status: ObservationStatus) {
            if let Err(cause) = attempt {
                assert!(
                    status.retained_transition || status.retained_draft || status.gap_count > 0,
                    "observer error requires retained work or a recorded gap: {cause}"
                );
                assert!(
                    matches!(status.eligibility, InputEligibility::Hold(_)),
                    "observer error holds eligibility: {cause}"
                );
            }
        }

        fn event_at(fixture: &WriterFixture, sequence: i64) -> storage::RuntimeEventView {
            let id = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2",
                    rusqlite::params![fixture.fixture.run.run_id, sequence],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            storage::get_event(&fixture.fixture.conn, id).unwrap()
        }

        #[test]
        fn controller_accepts_only_the_exact_fresh_initial_committed_grant() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let catalog = catalog();
            assert!(
                ObservationController::new(
                    &fixture.fixture.run,
                    &catalog,
                    storage::evidence::EvidencePolicy::metadata(),
                    1..=256
                )
                .is_ok()
            );
            for range in [0..=255, 1..=255, 2..=257, 257..=512] {
                assert!(
                    ObservationController::new(
                        &fixture.fixture.run,
                        &catalog,
                        storage::evidence::EvidencePolicy::metadata(),
                        range
                    )
                    .is_err()
                );
            }
            for field in [
                "epoch",
                "reservation",
                "status",
                "child",
                "start",
                "end",
                "recovery",
            ] {
                let mut run = fixture.fixture.run.clone();
                match field {
                    "epoch" => run.owner_epoch = 0,
                    "reservation" => run.event_seq_reserved_through = 512,
                    "status" => run.status = RunStatus::Running,
                    "child" => run.child_pid = Some(1),
                    "start" => run.started_at = Some(NOW.into()),
                    "end" => run.ended_at = Some(NOW.into()),
                    _ => run.recovery_origin_status = Some(RunStatus::Running),
                }
                assert!(
                    ObservationController::new(
                        &run,
                        &catalog,
                        storage::evidence::EvidencePolicy::metadata(),
                        1..=256
                    )
                    .is_err(),
                    "{field}"
                );
            }
        }

        #[test]
        fn controller_quiet_poll_opens_one_episode_with_the_unchanged_parser_sequence() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let clock = Clock::new();
            let (mut controller, _child, mut terminal) = started_controller(
                &mut fixture,
                &writer,
                &clock,
                &catalog(),
                storage::evidence::EvidencePolicy::metadata(),
            );
            terminal
                .feed_observed(
                    &form("write disposable.txt"),
                    &mut controller.observer(&writer, &clock),
                )
                .unwrap();
            let sequence = terminal.snapshot().screen_seq;
            assert!(matches!(
                controller.status().eligibility,
                InputEligibility::Hold(GateReason::Candidate)
            ));
            clock.set(99);
            controller
                .poll(&terminal.snapshot(), &writer, &clock)
                .unwrap();
            assert_eq!(committed(&fixture), 0);
            clock.set(100);
            controller
                .poll(&terminal.snapshot(), &writer, &clock)
                .unwrap();
            wait_until(|| committed(&fixture) == 1);
            let view = storage::get_event(&fixture.fixture.conn, 1).unwrap();
            let interaction = view
                .interaction
                .as_ref()
                .expect("opening links its persisted episode");
            assert_eq!(interaction.run_id, fixture.fixture.run.run_id);
            assert_eq!(interaction.episode_seq, 1);
            let event = &view.event;
            assert_eq!(event.kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(event.interaction_id, Some(interaction.interaction_id));
            assert_eq!(event.payload_json["evidence"]["screen_seq"], sequence);
            assert_eq!(terminal.snapshot().screen_seq, sequence);
            writer.finish(deadline());
        }

        #[test]
        fn controller_actual_adapter_observes_transient_form_before_clear_without_opening() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let clock = Clock::new();
            let (mut controller, _child, mut terminal) = started_controller(
                &mut fixture,
                &writer,
                &clock,
                &catalog(),
                storage::evidence::EvidencePolicy::metadata(),
            );
            let mut bytes = form("write disposable.txt");
            bytes.extend_from_slice(b"\x1b[2J\x1b[Hworking");
            terminal
                .feed_observed(&bytes, &mut controller.observer(&writer, &clock))
                .unwrap();
            clock.set(100);
            let status = controller
                .poll(&terminal.snapshot(), &writer, &clock)
                .unwrap();
            assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
            assert_eq!(committed(&fixture), 0);
            assert!(!status.retained_transition);
            assert!(!status.retained_draft);
            writer.finish(deadline());
        }

        #[test]
        fn controller_typed_failures_keep_metadata_only_diagnostics_and_hold_input() {
            for (reason, code) in [
                (ObservationFailure::Parser, "parser_unavailable"),
                (ObservationFailure::Detector, "detector_unavailable"),
                (ObservationFailure::Sequence, "sequence_unavailable"),
                (ObservationFailure::Sink, "sink_unavailable"),
            ] {
                let mut fixture = WriterFixture::new();
                configure(&mut fixture);
                let mut controller = ObservationController::new(
                    &fixture.fixture.run,
                    &catalog(),
                    storage::evidence::EvidencePolicy::metadata(),
                    1..=256,
                )
                .unwrap();
                controller.observer_failed(reason, &Clock::new()).unwrap();
                let status = controller.status();
                assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
                assert_eq!(status.diagnostic.as_deref(), Some(code));
                assert!(status.gap_count > 0);
            }
        }

        #[test]
        fn controller_started_validates_current_identity_and_is_single_use() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let clock = Clock::new();
            let mut controller = ObservationController::new(
                &fixture.fixture.run,
                &catalog(),
                storage::evidence::EvidencePolicy::metadata(),
                1..=256,
            )
            .unwrap();
            let screen = TerminalState::new(TerminalSize { rows: 24, cols: 80 })
                .unwrap()
                .snapshot();
            assert!(
                controller
                    .started(&fixture.fixture.run, &screen, &writer, &clock)
                    .is_err()
            );
            let child = TestChild::spawn(std::path::Path::new("/bin/cat"), &[]);
            running(&mut fixture, &child.identity());
            let mut stale = fixture.fixture.run.clone();
            stale.owner_epoch += 1;
            assert!(
                controller
                    .started(&stale, &screen, &writer, &clock)
                    .is_err()
            );
            controller
                .started(&fixture.fixture.run, &screen, &writer, &clock)
                .unwrap();
            assert!(
                controller
                    .started(&fixture.fixture.run, &screen, &writer, &clock)
                    .is_err()
            );
            writer.finish(deadline());
        }

        #[test]
        fn controller_exit_closes_the_open_episode_before_recording_the_real_native_signal() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let clock = Clock::new();
            let (mut controller, mut child, mut terminal) = started_controller(
                &mut fixture,
                &writer,
                &clock,
                &catalog(),
                storage::evidence::EvidencePolicy::metadata(),
            );
            terminal
                .feed_observed(
                    &form("write disposable.txt"),
                    &mut controller.observer(&writer, &clock),
                )
                .unwrap();
            clock.set(100);
            controller
                .poll(&terminal.snapshot(), &writer, &clock)
                .unwrap();
            wait_until(|| committed(&fixture) == 1);
            assert!(child.identity().matches().unwrap());
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(child.identity().pid).unwrap()),
                nix::sys::signal::Signal::SIGTERM,
            )
            .unwrap();
            let until = deadline();
            let exit = loop {
                if let Some(exit) = child.try_wait().unwrap() {
                    break exit;
                }
                assert!(Instant::now() < until, "owned child signal exit");
                std::thread::sleep(Duration::from_millis(2));
            };
            assert_eq!(exit.signal(), Some(15));
            clock.set(200);
            controller
                .child_exited(ChildExit::Signal(15), &terminal.snapshot(), &writer, &clock)
                .unwrap();
            wait_until(|| committed(&fixture) == 3);
            let kinds = crate::runtime::runner::service::test_support::observation_services::kinds(
                &fixture.fixture.conn,
                fixture.fixture.run.run_id,
            );
            assert_eq!(
                kinds,
                ["interaction_opened", "interaction_closed", "runner_exited"]
            );
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert_eq!(run.exit_signal, Some(15));
            assert_eq!(run.exit_code, None);
            writer.finish(deadline());
        }

        #[test]
        fn controller_refills_shared_lifecycle_sequences_only_from_committed_worker_grants() {
            let until = Instant::now() + Duration::from_secs(15);
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let mut writer = start(&fixture, Arc::new(Hooks::default()));
            let clock = Clock::new();
            let (mut controller, _child, mut terminal) = started_controller(
                &mut fixture,
                &writer,
                &clock,
                &catalog(),
                storage::evidence::EvidencePolicy::metadata(),
            );
            let mut millis = 0u64;
            let mut original_sequences = Vec::new();
            for index in 0..130 {
                assert!(Instant::now() < until, "bounded controller refill fixture");
                clock.set(millis);
                terminal
                    .feed_observed(
                        &form(&format!("write range-{index}.txt")),
                        &mut controller.observer(&writer, &clock),
                    )
                    .unwrap();
                original_sequences.push(terminal.snapshot().screen_seq);
                millis = millis.checked_add(100).unwrap();
                clock.set(millis);
                let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
                assert_retained_pressure(attempt, controller.status());
                wait_until(|| {
                    assert!(Instant::now() < until, "original refill fixture deadline");
                    millis = millis.checked_add(1).unwrap();
                    clock.set(millis);
                    let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
                    assert_retained_pressure(attempt, controller.status());
                    let openings:i64=fixture.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",[fixture.fixture.run.run_id],|row|row.get(0)).unwrap();
                    openings == i64::from(index + 1)
                        && !controller.status().retained_transition
                        && !controller.status().retained_draft
                });
                millis = millis.checked_add(100).unwrap();
            }
            wait_until(|| {
                millis = millis.checked_add(1).unwrap();
                clock.set(millis);
                let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
                assert_retained_pressure(attempt, controller.status());
                writer.status().pending_count == 0
                    && !controller.status().retained_transition
                    && !controller.status().retained_draft
                    && controller.status().diagnostic.is_none()
            });
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert!(run.event_seq_reserved_through >= 512);
            assert_eq!(run.event_seq_reserved_through % 256, 0);
            let ids: Vec<i64> = fixture
                .fixture
                .conn
                .prepare(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
                )
                .unwrap()
                .query_map([run.run_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(
                ids.iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                ids.len()
            );
            let mut last = 0;
            let mut interactions = Vec::new();
            let mut gap = false;
            for id in ids {
                let view = storage::get_event(&fixture.fixture.conn, id).unwrap();
                let event = &view.event;
                assert!(
                    event.event_seq > last && event.event_seq <= run.event_seq_reserved_through
                );
                last = event.event_seq;
                match event.kind {
                    RuntimeEventKind::InteractionOpened => {
                        let sequence = view.interaction.as_ref().unwrap().episode_seq;
                        assert_eq!(
                            event.payload_json["evidence"]["screen_seq"],
                            original_sequences[usize::try_from(sequence - 1).unwrap()]
                        );
                        interactions.push((event.kind, sequence));
                    }
                    RuntimeEventKind::InteractionClosed => {
                        assert_eq!(event.payload_json["close_reason"], "superseded");
                        interactions
                            .push((event.kind, view.interaction.as_ref().unwrap().episode_seq));
                    }
                    RuntimeEventKind::RunnerDegraded => {
                        let cause = &event.payload_json["cause"];
                        assert!(cause["gap_count"].as_u64().is_some());
                        assert!(cause["gap_started_at"].is_string());
                        let kind = cause["kind"].as_str().unwrap();
                        if kind == "storage_recovered" {
                            assert!(gap);
                            assert!(cause["gap_ended_at"].is_string());
                            gap = false;
                        } else {
                            assert!(matches!(kind, "storage_pressure" | "observer_error"));
                            assert!(!gap);
                            gap = true;
                        }
                    }
                    other => panic!("unexpected refill event: {other:?}"),
                }
            }
            let mut expected = Vec::new();
            for episode in 1..=130 {
                if episode > 1 {
                    expected.push((RuntimeEventKind::InteractionClosed, episode - 1));
                }
                expected.push((RuntimeEventKind::InteractionOpened, episode));
            }
            assert_eq!(interactions, expected);
            assert!(!gap);
            assert_eq!(
                fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1",
                        [run.run_id],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                130
            );
            writer.finish(deadline());
        }

        #[test]
        fn controller_retains_both_superseded_originals_when_the_first_submit_hits_pressure() {
            let mut fixture = WriterFixture::new();
            configure(&mut fixture);
            let mut definitions = definitions();
            let mut refusal = definitions["profiles"][0]["rules"][0].clone();
            refusal["id"] = serde_json::json!("synthetic-refusal");
            refusal["observation"] = serde_json::json!("permission_denied");
            refusal["verified_noninteraction"] = serde_json::json!(true);
            refusal["region"]["start_pattern"] = serde_json::json!("^Synthetic refusal result$");
            definitions["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(refusal);
            definitions["profiles"][0]["capabilities"]["detection"]["rule_ids"] =
                serde_json::json!(["synthetic-permission", "synthetic-refusal"]);
            let catalog = ProfileCatalog::from_definitions(&definitions).unwrap();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            let clock = Clock::new();
            let (mut controller, _child, mut terminal) = started_controller(
                &mut fixture,
                &writer,
                &clock,
                &catalog,
                storage::evidence::EvidencePolicy::metadata(),
            );
            let mut last_opening_sequence = 0;
            for index in 0..32 {
                clock.set(index * 200);
                terminal
                    .feed(&form(&format!("write fixture-{index:02}.txt")))
                    .unwrap();
                last_opening_sequence = terminal.snapshot().screen_seq;
                controller
                    .observer(&writer, &clock)
                    .observe(&terminal.snapshot())
                    .unwrap();
                clock.set(index * 200 + 100);
                controller
                    .poll(&terminal.snapshot(), &writer, &clock)
                    .unwrap();
                assert_eq!(writer.status().pending_count, index * 2 + 1);
                assert!(!writer.status().pending_evidence);
                assert!(!controller.has_pending());
            }
            assert_eq!(writer.status().pending_count, 63);
            let refusal = String::from_utf8(form("write fixture-31.txt"))
                .unwrap()
                .replace("Synthetic permission request", "Synthetic refusal result");
            clock.set(6400);
            terminal.feed(refusal.as_bytes()).unwrap();
            controller
                .observer(&writer, &clock)
                .observe(&terminal.snapshot())
                .unwrap();
            clock.set(6500);
            controller
                .poll(&terminal.snapshot(), &writer, &clock)
                .unwrap();
            assert_eq!(writer.status().pending_count, 64);
            assert!(!writer.status().pending_evidence);
            assert!(!writer.status().degraded);
            assert!(!controller.has_pending());
            assert_eq!(committed(&fixture), 0);
            clock.set(6550);
            terminal
                .feed_observed(
                    &form("write fixture-31.txt"),
                    &mut controller.observer(&writer, &clock),
                )
                .unwrap();
            clock.set(6600);
            let replacement = form("write final-original.txt");
            assert_eq!(replacement[0], 0x1b);
            terminal
                .feed_observed(&replacement[..1], &mut controller.observer(&writer, &clock))
                .unwrap();
            assert!(terminal.snapshot().screen_seq > last_opening_sequence);
            terminal
                .feed_observed(&replacement[1..], &mut controller.observer(&writer, &clock))
                .unwrap();
            clock.set(6700);
            let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
            assert_retained_pressure(attempt, controller.status());
            let original_sequence = terminal.snapshot().screen_seq;
            assert!(controller.status().retained_transition);
            assert!(controller.status().retained_draft);
            assert_eq!(writer.status().pending_count, 64);
            assert!(matches!(
                controller.status().eligibility,
                InputEligibility::Hold(_)
            ));
            let attempt = terminal.feed_observed(
                b"\x1b[2J\x1b[Hlater output during the gap",
                &mut controller.observer(&writer, &clock),
            );
            assert_retained_pressure(attempt, controller.status());
            assert!(terminal.snapshot().screen_seq > original_sequence);
            assert!(controller.status().retained_transition);
            assert!(controller.status().retained_draft);
            drop(release);
            wait_until_within(PRESSURE_DRAIN_BUDGET, || {
                let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
                assert_retained_pressure(attempt, controller.status());
                committed(&fixture) >= 66
            });
            let closed = event_at(&fixture, 65);
            let opened = event_at(&fixture, 66);
            assert_eq!(closed.event.kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(closed.event.payload_json["close_reason"], "superseded");
            for (view, sequence, episode, screen) in [
                (&closed, 65, 32, last_opening_sequence),
                (&opened, 66, 33, original_sequence),
            ] {
                let interaction = view.interaction.as_ref().unwrap();
                assert_eq!(view.event.run_id, fixture.fixture.run.run_id);
                assert_eq!(view.event.event_seq, sequence);
                assert_eq!(view.event.interaction_id, Some(interaction.interaction_id));
                assert_eq!(interaction.episode_seq, episode);
                assert_eq!(view.event.observed_at, NOW);
                assert_eq!(view.event.payload_json["evidence"]["screen_seq"], screen);
            }
            assert_eq!(opened.event.kind, RuntimeEventKind::InteractionOpened);
            wait_until_within(PRESSURE_DRAIN_BUDGET, || {
                let attempt = controller.poll(&terminal.snapshot(), &writer, &clock);
                assert_retained_pressure(attempt, controller.status());
                !controller.has_pending() && writer.status().pending_count == 0
            });
            assert_eq!(
                writer.finish(Instant::now() + PRESSURE_DRAIN_BUDGET),
                (0, 0)
            );
            assert_eq!(event_at(&fixture, 65).event, closed.event);
            assert_eq!(event_at(&fixture, 66).event, opened.event);
        }

        #[test]
        fn controller_actual_adapter_retains_foreign_episode_refresh_and_holds_until_drain() {
            let mut fixture = WriterFixture::new();
            configure_redacted(&mut fixture);
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            let clock = Clock::new();
            let (mut controller, _child, mut terminal) =
                started_controller(&mut fixture, &writer, &clock, &catalog(), redacted_policy());
            let mut opening_sequences = Vec::new();
            let mut first_refresh_sequence = 0;
            for index in 0..2 {
                clock.set(index * 200);
                terminal
                    .feed_observed(
                        &detailed_form(&format!("write collision-{index}.txt"), "opened"),
                        &mut controller.observer(&writer, &clock),
                    )
                    .unwrap();
                opening_sequences.push(terminal.snapshot().screen_seq);
                clock.set(index * 200 + 100);
                controller
                    .poll(&terminal.snapshot(), &writer, &clock)
                    .unwrap();
                if index == 0 {
                    clock.set(150);
                    terminal
                        .feed_observed(
                            &detail_rewrite("refreshed"),
                            &mut controller.observer(&writer, &clock),
                        )
                        .unwrap();
                    first_refresh_sequence = terminal.snapshot().screen_seq;
                    assert!(writer.status().pending_evidence);
                }
            }
            assert_eq!(writer.status().pending_count, 3);
            assert!(writer.status().pending_evidence);
            assert!(!writer.status().degraded);
            assert!(!controller.has_pending());
            clock.set(400);
            terminal
                .feed_observed(b"\x1b", &mut controller.observer(&writer, &clock))
                .unwrap();
            assert!(
                !controller.status().retained_evidence,
                "a screen_seq-only redraw schedules no refresh"
            );
            assert!(!writer.status().degraded);
            terminal
                .feed_observed(
                    &detail_rewrite("collided"),
                    &mut controller.observer(&writer, &clock),
                )
                .unwrap();
            let retained_sequence = terminal.snapshot().screen_seq;
            let status = controller.status();
            assert!(status.retained_evidence);
            assert!(!status.retained_transition);
            assert!(!status.retained_draft);
            assert!(status.gap_count > 0);
            assert_eq!(status.eligibility, InputEligibility::Hold(GateReason::Gap));
            assert!(writer.status().pending_evidence);
            assert!(writer.status().degraded);
            assert!(
                writer
                    .status()
                    .diagnostic
                    .as_deref()
                    .unwrap()
                    .contains("refresh episode collision")
            );
            terminal
                .feed_observed(
                    &detailed_form("write later-unclassified.txt", "opened"),
                    &mut controller.observer(&writer, &clock),
                )
                .unwrap();
            assert!(terminal.snapshot().screen_seq > retained_sequence);
            assert!(controller.status().retained_evidence);
            assert_eq!(
                controller.status().eligibility,
                InputEligibility::Hold(GateReason::Gap)
            );
            assert_eq!(writer.status().pending_count, 3);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            wait_until(|| {
                controller
                    .poll(&terminal.snapshot(), &writer, &clock)
                    .unwrap();
                !controller.has_pending()
                    && !writer.status().pending_evidence
                    && writer.status().pending_count == 0
                    && !writer.status().degraded
            });
            let first = event_at(&fixture, 1);
            let closed = event_at(&fixture, 2);
            let second = event_at(&fixture, 3);
            assert_eq!(first.event.kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(closed.event.kind, RuntimeEventKind::InteractionClosed);
            assert_eq!(closed.event.payload_json["close_reason"], "superseded");
            assert_eq!(second.event.kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(first.interaction.as_ref().unwrap().episode_seq, 1);
            assert_eq!(closed.interaction.as_ref().unwrap().episode_seq, 1);
            assert_eq!(second.interaction.as_ref().unwrap().episode_seq, 2);
            assert_eq!(
                first.event.payload_json["evidence"]["screen_seq"],
                opening_sequences[0]
            );
            assert_eq!(
                closed.event.payload_json["evidence"]["screen_seq"],
                first_refresh_sequence
            );
            assert_eq!(
                second.event.payload_json["evidence"]["screen_seq"],
                opening_sequences[1]
            );
            assert_eq!(
                second.interaction.as_ref().unwrap().evidence_json["screen_seq"],
                retained_sequence
            );
            let episodes: i64 = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(episodes, 2);
            assert_eq!(writer.finish(deadline()), (0, 0));
        }
    }
}
