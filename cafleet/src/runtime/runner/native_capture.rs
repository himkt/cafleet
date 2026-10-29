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
pub(crate) use exporter::tests::runner_collector_export_child_path;
use exporter::{CaptureExporterTask, InertCaptureExportHooks};
pub(crate) use exporter::{NativeCaptureExportBoundary, NativeCaptureExportHooks};
#[cfg(test)]
mod review_service_tests;
pub(crate) use review_service_tests::runner_collector_resize_child_path;
mod verification;
pub(crate) use files::*;
pub(crate) use verification::{
    native_capture_service_child_path, run_native_capture_verification_with_export_hooks,
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

pub(crate) mod test_support {
    use serde_json::{Value, json};

    use super::*;
    use crate::broker::records::Placement;
    use crate::broker::runtime as storage;
    use crate::coding_agent::ScreenRegion;
    use crate::runtime::runner::test_support::*;
    use crate::runtime::system::test_support::TestChild;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    pub(crate) fn private_write(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
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
                monotonic: self.origin + Duration::from_millis(self.millis.load(Ordering::Acquire)),
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
            std::fs::set_permissions(fixture._dir.path(), std::fs::Permissions::from_mode(0o700))
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
            fixture.conn.execute("UPDATE member_runs SET runner_pid=owner_pid,runner_start_identity=owner_start_identity,native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',profile_match='exact',coverage='tested',event_evidence=?1 WHERE run_id=?2",rusqlite::params![policy["mode"].as_str().unwrap(),fixture.run.run_id]).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let executable = fixture._dir.path().join("owned-synthetic-process");
            private_write(&executable,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'synthetic-0.0.1\\n'; exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; *) printf '%s\\n' \"$line\" ;; esac; done\n");
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
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
                b"{\"kind\":\"synthetic\",\"version\":\"synthetic-0.0.1\"}",
            );
            let current = std::env::current_exe().unwrap();
            let producer_json = json!({"schema_version":1,"kind":"synthetic","source_manifest_ref":source,"source_manifest_sha256":digest(&source),"libtest_executable_path":current,"libtest_executable_sha256":digest(&current),"parser_version":"0.18.1","collector_version":"1","catalog_origin":"synthetic","native_executable":{"path":executable,"sha256":digest(&executable),"version":"synthetic-0.0.1","version_evidence_ref":source},"launch_evidence_ref":source,"environment_evidence_ref":source});
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
            let policy = NativeCapturePolicy::from_launch(&self.spec, masks, &catalog()).unwrap();
            native_capture_channel(&self.fixture.run, policy, limits).unwrap()
        }
        pub(crate) fn prepare(&mut self, port: &mut NativeCapturePort) {
            self.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                    [self.fixture.run.run_id],
                )
                .unwrap();
            self.fixture.run =
                storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
            let producer =
                read_native_capture_producer(&self.producer_path, &NativeCaptureLimits::default())
                    .unwrap();
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
        NativeCaptureArm {
            schema_version: 1,
            capture_id: "review-pair".into(),
            run_id: identity.run_id,
            owner_epoch: identity.owner_epoch,
            minimum_interval_ms: 100,
            timeout_ms: 1000,
        }
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
    use crate::runtime::runner::service::test_support::*;
    use crate::runtime::runner::test_support::*;

    mod collection {
        use super::*;
        use std::time::Duration;

        #[test]
        fn runner_batch2_capture_seed_prepared_and_finalized_boundaries_enable_exactly_one_request()
        {
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
        fn runner_batch2_capture_source_rejects_running_seed_and_mismatched_child_or_placement() {
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
            let policy = NativeCapturePolicy::from_launch(&fixture.spec, &[], &catalog()).unwrap();
            assert!(native_capture_channel(&run, policy, NativeCaptureLimits::default()).is_err());
            fixture.finalize(&mut port);
            assert!(receiver.finalized_identity().is_some());
        }

        #[test]
        fn runner_batch2_capture_invalid_request_leaves_the_finalized_channel_unused() {
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
        fn runner_batch2_capture_slow_consumer_holds_one_immutable_pair_while_observation_keeps_progressing()
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
        fn runner_batch2_capture_pair_uses_atomic_snapshots_and_second_selection_after_first_end() {
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
        fn runner_batch2_capture_metadata_projects_no_grid_title_secret_or_private_source_arguments()
         {
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
        fn runner_batch2_capture_patterns_mask_whole_logical_fields_across_softwraps_and_cells() {
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
        fn runner_batch2_capture_missing_in_range_parser_cell_fails_at_the_projection_boundary() {
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
        fn runner_batch2_capture_pattern_configuration_is_bounded_before_observer_callbacks() {
            for patterns in [
                vec!["absent".to_owned(); 65],
                vec!["x".repeat(4097)],
                vec!["x".repeat(2048); 33],
            ] {
                let mut policy = redacted();
                policy["config"]["patterns"] = json!(patterns);
                let fixture = CaptureFixture::new(policy);
                assert!(NativeCapturePolicy::from_launch(&fixture.spec, &[], &catalog()).is_err());
            }
        }

        #[test]
        fn runner_batch2_capture_coordinate_mask_includes_wide_partner_and_preserves_empty_cells() {
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
        fn runner_batch2_capture_layout_mismatch_masks_whole_content_instead_of_clipping_coordinates()
         {
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
        fn runner_batch2_capture_invalid_mask_geometry_fails_before_collection() {
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
        fn runner_batch2_capture_sensitive_rule_narrows_redacted_launch_to_metadata() {
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
        fn runner_batch2_capture_lowered_budgets_report_capacity_without_a_padded_complete_pair() {
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
        fn runner_batch2_capture_limits_reject_zero_and_expansion_but_allow_lowered_budgets() {
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
        fn runner_batch2_capture_deadline_owner_exit_and_cancellation_preserve_actual_incomplete_status()
         {
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

    mod review_policy {
        use super::*;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Compiler {
            calls: Mutex<Vec<(String, usize)>>,
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
        fn runner_collector_f2_whole_raw_policy_is_validated_before_the_first_compiler_invocation()
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
                    "later_type" => policy["config"]["patterns"] = json!(["safe first pattern", 7]),
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
                let fixture = CaptureFixture::new(policy);
                let compiler = Compiler::default();
                assert!(
                    NativeCapturePolicy::from_launch_with_compiler(
                        &fixture.spec,
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
        fn runner_collector_f2_valid_patterns_compile_once_in_source_order_with_the_approved_cap() {
            let patterns = ["PRIVATE-[A-Z]+", "[0-9]+", "(?i)secret"];
            let mut policy = redacted();
            policy["config"]["patterns"] = json!(patterns);
            policy["config"]["sensitive_rules"] = json!(["synthetic-permission"]);
            let fixture = CaptureFixture::new(policy);
            let compiler = Compiler::default();
            assert!(
                NativeCapturePolicy::from_launch_with_compiler(
                    &fixture.spec,
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
        fn runner_collector_f2_compact_pattern_expansion_stops_at_the_first_bounded_compilation() {
            let mut policy = redacted();
            policy["config"]["patterns"] = json!(["[a-z]{100000}"]);
            let fixture = CaptureFixture::new(policy);
            let compiler = Compiler::default();
            assert!(
                NativeCapturePolicy::from_launch_with_compiler(
                    &fixture.spec,
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
        fn runner_collector_f2_metadata_policy_uses_no_regex_compiler() {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let compiler = Compiler::default();
            assert!(
                NativeCapturePolicy::from_launch_with_compiler(
                    &fixture.spec,
                    &[],
                    &catalog(),
                    &compiler
                )
                .is_ok()
            );
            assert!(compiler.calls.lock().unwrap().is_empty());
        }

        #[test]
        fn runner_collector_f2_channel_selected_compiled_cap_remains_bounded_after_valid_construction()
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
        fn runner_batch2_capture_bind_checks_actual_native_digest_before_any_observation() {
            let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
            let mut producer = fixture.producer_json.clone();
            producer["native_executable"]["sha256"] = json!("0".repeat(64));
            let path = fixture.fixture._dir.path().join("wrong-native.json");
            private_write(&path, &serde_json::to_vec(&producer).unwrap());
            let producer =
                read_native_capture_producer(&path, &NativeCaptureLimits::default()).unwrap();
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
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

    mod review_existing {
        use super::*;
        use crate::coding_agent::ProfileCatalog;

        fn prepared(fixture: &mut CaptureFixture) {
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }
        fn producer(fixture: &CaptureFixture) -> NativeCaptureProducer {
            read_native_capture_producer(&fixture.producer_path, &NativeCaptureLimits::default())
                .unwrap()
        }
        fn channel(
            fixture: &CaptureFixture,
            catalog: &ProfileCatalog,
        ) -> (NativeCapturePort, NativeCaptureReceiver) {
            native_capture_channel(
                &fixture.fixture.run,
                NativeCapturePolicy::from_launch(&fixture.spec, &[], catalog).unwrap(),
                NativeCaptureLimits::default(),
            )
            .unwrap()
        }
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
                "UPDATE member_runs SET profile_version=?1,profile_match=?2,coverage=?3 WHERE run_id=?4",
                rusqlite::params![
                    selected.profile_version,
                    selected.profile_match.map(|value| value.as_str()),
                    selected.detection.as_str(),
                    fixture.fixture.run.run_id
                ],
            )
            .unwrap();
                fixture.fixture.run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                ProfileCatalog::bundled().unwrap()
            } else {
                catalog()
            }
        }

        #[test]
        fn runner_collector_f1_actual_permission_options_match_the_recorded_emitted_policy_before_binding()
         {
            for mutation in [
                "approval",
                "missing_approval",
                "repeat_approval",
                "added_sandbox",
                "trailing_sandbox",
            ] {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let (mut port, receiver) = channel(&fixture, &catalog());
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
                prepared(&mut fixture);
                assert!(
                    port.bind_prepared(
                        &fixture.fixture.run,
                        &fixture.placement,
                        &fixture.spec,
                        producer(&fixture),
                        &fixture.clock
                    )
                    .is_err(),
                    "{mutation}"
                );
                assert!(receiver.finalized_identity().is_none());
                assert!(receiver.request(fixture.request("unbound-policy")).is_err());
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert!(run.child_pid.is_none());
            }
        }

        #[test]
        fn runner_collector_f1_flag_like_prompt_is_opaque_and_preserves_truthful_permission_source()
        {
            let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            *fixture.spec.argv.last_mut().unwrap() =
                "explain --ask-for-approval on-request and --sandbox read-only".into();
            let (mut port, receiver) = channel(&fixture, &catalog());
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
        fn runner_collector_f1_producer_origin_matches_the_selected_catalog_constructor_after_clone()
         {
            for bundled in [false, true] {
                let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
                let actual = select_catalog(&mut fixture, bundled);
                let selected = actual.clone();
                let (mut port, receiver) = channel(&fixture, &selected);
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
                prepared(&mut fixture);
                assert!(
                    port.bind_prepared(
                        &fixture.fixture.run,
                        &fixture.placement,
                        &fixture.spec,
                        producer(&fixture),
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
        fn runner_collector_f1_matching_synthetic_and_bundled_origins_publish_only_their_actual_provenance()
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
                let (mut port, receiver) = channel(&fixture, &selected.clone());
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
        fn runner_batch2_service_private_capture_publishes_manifest_last_and_keeps_native_work_running()
         {
            let mut fixture = ServiceFixture::configured(false, capture);
            let started = fixture.start_running();
            let request = NativeCaptureArm {
                schema_version: 1,
                capture_id: "service-pair".into(),
                run_id: started.identity.run_id,
                owner_epoch: started.identity.owner_epoch,
                minimum_interval_ms: 100,
                timeout_ms: 2000,
            };
            let arm_at = crate::time::now_utc();
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
            let collected_at = crate::time::now_utc();
            for sample in samples {
                assert!(
                    sample["monotonic_end_ns"].as_u64().unwrap()
                        >= sample["monotonic_start_ns"].as_u64().unwrap()
                );
                let observed =
                    chrono::DateTime::parse_from_rfc3339(sample["observed_at"].as_str().unwrap())
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
