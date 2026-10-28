use super::*;
use crate::broker::runtime::{
    self as storage, InteractionCloseReason, OwnerKind, RunRecord, RunStatus, RuntimeEventKind,
    evidence::EvidencePolicy,
};
use crate::coding_agent::{AnswerMatch, Detector, ProfileCatalog};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

#[cfg(test)]
mod inspection_tests {
    use super::*;
    use crate::runtime::runner_tests::Fixture;

    #[test]
    fn permission_inspection_request_generation_exhaustion_invalidates_without_wrap() {
        use crate::coding_agent::{inspection_fixture_definition, inspection_fixture_form};
        use crate::runtime::runner_tests::owned_child::OwnedChild;
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        use std::process::{Command, Stdio};
        use std::sync::Arc;

        struct Native {
            child: OwnedChild,
            until: Instant,
        }
        impl Drop for Native {
            fn drop(&mut self) {
                self.child.terminate_until(self.until);
            }
        }
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
        fixture.conn.execute("UPDATE member_runs SET native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let catalog = ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
        let mut controller =
            ObservationController::new(&fixture.run, &catalog, EvidencePolicy::metadata(), 1..=256)
                .unwrap();
        let mut writer = EventWriter::start(
            format!("sqlite:///{}", fixture.conn.path().unwrap()),
            fixture.run.clone(),
            lock,
            artifacts.paths.outbox.clone(),
        )
        .unwrap();
        writer.set_deadline(until);
        let mut native = Native {
            child: OwnedChild::new(
                Command::new("/bin/cat")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            ),
            until,
        };
        let identity = ProcessIdentity::read(i64::from(native.child.id()))
            .unwrap()
            .expect("owned live synthetic child");
        assert!(identity.matches().unwrap());
        fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![identity.pid,identity.start_identity,crate::time::format_utc(crate::time::now_utc()),fixture.run.run_id]).unwrap();
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
                Some("synthetic-0.0.1"),
            )
            .unwrap()
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
                    cause.message().contains("generation") || cause.message().contains("exhaust"),
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
                    cause.message().contains("generation") || cause.message().contains("exhaust"),
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
            native.child.terminate_until(until);
            assert!(
                native.child.try_wait().unwrap().is_some(),
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
        trait AmbiguousIfImplemented<Marker> {
            fn check() {}
        }
        struct DebugImplemented;
        struct SerializeImplemented;
        impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
        impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
        impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
        let _ = <ObservedRequest as AmbiguousIfImplemented<_>>::check;
    }
}

pub(crate) struct ObservationMoment {
    pub monotonic: Instant,
    pub observed_at: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationClockKind {
    Real,
    #[cfg(test)]
    Deterministic,
}
pub(crate) trait ObservationClock: Send + Sync {
    fn sample(&self) -> ObservationMoment;
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
pub(crate) type ObservationSite = ObservationPhase;
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservationOperation {
    Parser,
    Detector,
}
#[cfg(test)]
pub(crate) trait ObservationHooks: Send + Sync {
    fn before(&self, site: ObservationSite, operation: ObservationOperation) -> Result<()>;
    fn order_shutdown_parsing_after_native_exit(&self, _deadline: Instant) -> bool {
        false
    }
}
pub(crate) struct RealObservationClock;
impl ObservationClock for RealObservationClock {
    fn sample(&self) -> ObservationMoment {
        ObservationMoment {
            monotonic: Instant::now(),
            observed_at: crate::time::format_utc(crate::time::now_utc()),
        }
    }
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
    pub visibility: Visibility,
    pub eligibility: InputEligibility,
    pub next_deadline: Option<Instant>,
    pub retained_transition: bool,
    pub retained_draft: bool,
    pub retained_evidence: bool,
    pub gap_count: u64,
    pub diagnostic: Option<String>,
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
    fn same_request(&self, other: &Self) -> bool {
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
        let mut pool = EventSequencePool::new();
        pool.supply(initial)?;
        Ok(Self {
            run: run.clone(),
            detector: Detector::new(catalog, backend, run.native_version.as_deref())?,
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
                        .answer_plan(backend, version, &observed.rule_id, decision)?
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
        let evidence=EvidencePolicy::metadata().apply(&serde_json::json!({"backend":self.run.backend,"native_version":self.run.native_version,"profile_version":self.run.profile_version,"rule_id":"runner-lifecycle","screen_seq":screen_seq,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}))?;
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
        ensure(
            self.pending.is_none() && update.transitions.len() <= 2,
            "one bounded observation update required",
        )?;
        self.visibility = update.visibility;
        self.eligibility = update.eligibility;
        self.next_deadline = update.next_deadline;
        if !update.transitions.is_empty() || update.latest_evidence.is_some() {
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
            visibility: self.visibility,
            eligibility: self.eligibility,
            transitions,
            latest_evidence: None,
            next_deadline: None,
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
                match writer.try_submit(value) {
                    Ok(()) => pending.next_draft += 1,
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
        #[cfg(not(test))]
        ensure(
            clock.kind() == ObservationClockKind::Real,
            "live observation requires a real clock",
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
            let cause = |kind: &str| serde_json::json!({"kind":kind,"gap_count":gap.count,"gap_started_at":gap.started_at,"gap_ended_at":if kind=="storage_recovered"{Some(&now.observed_at)}else{None}});
            let first = self.lifecycle(
                RuntimeEventKind::RunnerDegraded,
                cause(
                    if matches!(
                        gap.reason,
                        ObservationFailure::Parser | ObservationFailure::Detector
                    ) {
                        "observer_error"
                    } else {
                        "storage_pressure"
                    },
                ),
                &gap.started_at,
                current.screen_seq,
            )?;
            let last = self.lifecycle(
                RuntimeEventKind::RunnerDegraded,
                cause("storage_recovered"),
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
                    serde_json::json!({"kind":"native_exit","exit_code":code,"exit_signal":null})
                }
                ChildExit::Signal(signal) => {
                    serde_json::json!({"kind":"native_exit","exit_code":null,"exit_signal":signal})
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
