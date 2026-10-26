use super::*;
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeCaptureExportBoundary {
    Encode,
    ArtifactPublish,
    ManifestPublish,
}
pub(crate) trait NativeCaptureExportHooks: Send + Sync {
    fn before(&self, boundary: NativeCaptureExportBoundary, deadline: Instant) -> Result<()>;
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
            .shutdown_until
            .map_or(requested, |until| until.min(requested))
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
    let mut reader = NativeCaptureArmReader::new(&config.arm_path, identity, limits.clone())?;
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
    match control {
        Some(control) => control.acquire(until).map(Some),
        None => {
            check_deadline(Instant::now(), until)?;
            Ok(None)
        }
    }
}
