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

#[cfg(test)]
mod controller_tests {
    use super::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::coding_agent::ProfileCatalog;
    use crate::runtime::runner::writer::contract_tests::support::*;
    use crate::runtime::runner::writer::contract_tests::worker::{
        Hooks, Release, committed, start, wait_until, wait_until_within,
    };
    use crate::runtime::runner::writer::contract_tests::{WriterFixture, deadline};
    use std::os::unix::process::ExitStatusExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    mod review {
        use super::*;

        mod observation_failures {
            use super::*;
            use std::sync::atomic::AtomicBool;

            struct Fault {
                site: ObservationSite,
                armed: AtomicBool,
                calls: AtomicU64,
            }
            impl Fault {
                fn new(site: ObservationSite) -> Self {
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
                    site: ObservationSite,
                    operation: ObservationOperation,
                ) -> Result<()> {
                    if site == self.site && operation == ObservationOperation::Detector {
                        self.calls.fetch_add(1, Ordering::AcqRel);
                        if self.armed.swap(false, Ordering::AcqRel) {
                            return Err(error("private detector failure review-canary-194"));
                        }
                    }
                    Ok(())
                }
            }

            #[test]
            fn runner_review_r3_observation_hooks_install_once_before_the_matching_native_start() {
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
                    .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
                    .unwrap();
                assert!(
                    controller
                        .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
                        .is_err()
                );
                let mut case = LiveCase::new();
                assert!(
                    case.controller
                        .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
                        .is_err()
                );
            }

            #[test]
            fn runner_review_r3_quiet_detector_failure_returns_typed_held_status_and_preserves_the_episode()
             {
                let fault = Arc::new(Fault::new(ObservationSite::QuietTimer));
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
                assert!(case.child.identity.matches().unwrap());
            }

            #[test]
            fn runner_review_r3_ordered_adapter_preserves_detector_attribution_and_bounded_metadata()
             {
                let fault = Arc::new(Fault::new(ObservationSite::ChildOutput));
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
                assert!(case.child.identity.matches().unwrap());
            }

            #[test]
            fn runner_review_r3_clock_and_output_sequence_invariants_propagate_outside_recoverable_failures()
             {
                for invariant in ["clock", "sequence"] {
                    let fault = Arc::new(Fault::new(ObservationSite::ChildOutput));
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
            child: ChildGuard,
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
                let mut controller =
                    ObservationController::new(&fixture.fixture.run, &catalog(), policy, 1..=256)
                        .unwrap();
                if let Some(hooks) = observation_hooks {
                    controller.set_observation_hooks(hooks).unwrap();
                }
                let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
                running(&mut fixture, &child.identity);
                let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
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
                assert!(self.child.identity.matches().unwrap());
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(i32::try_from(self.child.identity.pid).unwrap()),
                    nix::sys::signal::Signal::SIGTERM,
                )
                .unwrap();
                let until = deadline();
                loop {
                    if let Some(exit) = self.child.child.try_wait().unwrap() {
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
                    if !self.controller.has_pending() && self.writer.status().pending_count == 0 {
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
                    eprintln!("owned review fixture retained writer work: {unfinished:?}");
                }
            }
        }

        #[test]
        fn runner_review_r1_degraded_exit_records_gap_before_original_closure_and_terminal_fence() {
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
            let run = storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
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
        fn runner_review_r1_expired_storage_budget_reports_unsaved_gap_and_pending_exit_work() {
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
            let run = storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
                .unwrap();
            assert_eq!(run.status, RunStatus::Running);
            assert!(case.child.child.try_wait().unwrap().is_some());
            assert_eq!(
                case.events()[0].event.kind,
                RuntimeEventKind::InteractionOpened
            );
        }

        #[test]
        fn runner_review_r1_failure_after_terminal_enqueue_never_appends_a_live_post_terminal_gap()
        {
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
            let run = storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id)
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
    fn runner_batch2_controller_accepts_only_the_exact_fresh_initial_committed_grant() {
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
    fn runner_batch2_controller_quiet_poll_opens_one_episode_with_the_unchanged_parser_sequence() {
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
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
    fn runner_batch2_controller_actual_adapter_observes_transient_form_before_clear_without_opening()
     {
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
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
    fn runner_batch2_controller_typed_failures_keep_metadata_only_diagnostics_and_hold_input() {
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
    fn runner_batch2_controller_started_validates_current_identity_and_is_single_use() {
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
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
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
    fn runner_batch2_controller_exit_closes_the_open_episode_before_recording_the_real_native_signal()
     {
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
        let mut child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
        assert!(child.identity.matches().unwrap());
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(child.identity.pid).unwrap()),
            nix::sys::signal::Signal::SIGTERM,
        )
        .unwrap();
        let until = deadline();
        let exit = loop {
            if let Some(exit) = child.child.try_wait().unwrap() {
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
        let kinds: Vec<String> = fixture
            .fixture
            .conn
            .prepare("SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq")
            .unwrap()
            .query_map([fixture.fixture.run.run_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
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
    fn runner_batch2_controller_refills_shared_lifecycle_sequences_only_from_committed_worker_grants()
     {
        let until = Instant::now() + Duration::from_secs(15);
        let mut fixture = WriterFixture::new();
        configure(&mut fixture);
        let mut writer = start(&fixture, Arc::new(Hooks::default()));
        let clock = Clock::new();
        let mut controller = ObservationController::new(
            &fixture.fixture.run,
            &catalog(),
            storage::evidence::EvidencePolicy::metadata(),
            1..=256,
        )
        .unwrap();
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
            assert!(event.event_seq > last && event.event_seq <= run.event_seq_reserved_through);
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
                    interactions.push((event.kind, view.interaction.as_ref().unwrap().episode_seq));
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
    fn runner_batch2_controller_retains_both_superseded_originals_when_the_first_submit_hits_pressure()
     {
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
        let mut controller = ObservationController::new(
            &fixture.fixture.run,
            &catalog,
            storage::evidence::EvidencePolicy::metadata(),
            1..=256,
        )
        .unwrap();
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
    fn runner_batch2_controller_actual_adapter_retains_foreign_episode_refresh_and_holds_until_drain()
     {
        let mut fixture = WriterFixture::new();
        configure_redacted(&mut fixture);
        let hooks = Arc::new(Hooks::default());
        hooks.pause(WriterBoundary::Receive);
        let mut writer = start(&fixture, hooks.clone());
        let release = Release(hooks.clone());
        hooks.wait(WriterBoundary::Receive, 1);
        let clock = Clock::new();
        let mut controller = ObservationController::new(
            &fixture.fixture.run,
            &catalog(),
            redacted_policy(),
            1..=256,
        )
        .unwrap();
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
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
