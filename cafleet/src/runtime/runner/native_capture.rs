use super::*;
use crate::broker::records::Placement;
use crate::broker::runtime::{OwnerKind, RunRecord, RunStatus};
use crate::coding_agent::ScreenRegion;
use crate::coding_agent::{Detector, ProfileCatalog};
use regex::Regex;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
mod exporter;
mod files;
use exporter::{CaptureExporterTask, InertCaptureExportHooks};
pub(crate) use exporter::{NativeCaptureExportBoundary, NativeCaptureExportHooks};
#[cfg(test)]
mod review_service_tests;
mod verification;
pub(crate) use files::*;
pub(crate) use verification::{
    run_native_capture_verification, run_native_capture_verification_with_export_hooks,
};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NativeCaptureLimits {
    pub max_cells: usize,
    pub max_source_bytes: usize,
    pub max_title_bytes: usize,
    pub max_patterns: usize,
    pub max_pattern_bytes: usize,
    pub max_total_pattern_bytes: usize,
    pub max_compiled_regex_bytes: usize,
    pub max_pattern_input_bytes: usize,
    pub max_scratch_bytes: usize,
    pub max_projected_heap_bytes: usize,
    pub max_encoded_sample_bytes: usize,
    pub max_encoded_result_bytes: usize,
    pub max_arm_bytes: usize,
    pub max_config_bytes: usize,
    pub max_producer_bytes: usize,
    pub max_diagnostic_bytes: usize,
    pub max_interval: Duration,
    pub max_request_duration: Duration,
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
    pub screen: CaptureScreen,
    pub rows: u16,
    pub cols: u16,
    pub region: ScreenRegion,
    pub reason: CaptureMaskReason,
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
                && max_compiled_bytes <= NativeCaptureLimits::default().max_compiled_regex_bytes,
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
    pub capture_id: String,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub minimum_interval: Duration,
    pub deadline: Instant,
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
        if let Some(index) = result
            .samples
            .iter()
            .position(|sample| encoded_size(sample, self.limits.max_encoded_sample_bytes).is_err())
        {
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
        self.state
            .lock()
            .expect("capture mailbox")
            .shutdown_until
            .map_or(until, |end| end.min(until))
    }
}
impl NativeCapturePort {
    pub(super) fn begin_shutdown(&mut self, until: Instant) {
        let mut state = self.state.lock().expect("capture mailbox");
        state.closed = true;
        let until = state.shutdown_until.map_or(until, |old| old.min(until));
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
                && (self.seed.owner_kind == OwnerKind::Creator || self.seed.owner() == run.owner()),
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
        let selection =
            backend.resolve_policy(run.native_version.as_deref(), run.requested_mode.as_deref())?;
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
        )?);
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
                            masked
                                [usize::from(row) * usize::from(screen.cols) + usize::from(col)] =
                                true;
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
struct SizeWriter {
    bytes: usize,
    limit: usize,
}
impl std::io::Write for SizeWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("encoded size overflow"))?;
        if end > self.limit {
            return Err(std::io::Error::other("encoded capture capacity"));
        }
        self.bytes = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encoded_size(value: &impl Serialize, limit: usize) -> Result<usize> {
    let mut counter = SizeWriter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(io_error)?;
    Ok(counter.bytes)
}
