use super::*;
use crate::broker::runtime as storage;
use crate::config::Settings;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

struct ResizeArm {
    receiver: Arc<Mutex<NativeCaptureReceiver>>,
    used: AtomicBool,
}
impl ObservationHooks for ResizeArm {
    fn before(&self, site: ObservationSite, operation: ObservationOperation) -> Result<()> {
        if site == ObservationSite::Resize
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
fn write_pair(path: &Path, lease: &NativeCaptureLease) -> Result<()> {
    let pending = path.with_extension("pending");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&pending)
        .map_err(io_error)?;
    file.write_all(&serde_json::to_vec(lease.result()).map_err(io_error)?)
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    std::fs::rename(pending, path).map_err(io_error)
}

pub(crate) fn runner_collector_resize_child_path() -> &'static str {
    test_path!(runner_collector_resize_child)
}

#[test]
#[ignore = "Owned synthetic resize fixture selects this private entry"]
fn runner_collector_resize_child() {
    let path =
        std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG").expect("owned resize configuration");
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
    let policy = NativeCapturePolicy::from_launch(&descriptor.spawn, &[], &catalog).unwrap();
    let (mut port, receiver) = native_capture_channel(&run, policy, limits.clone()).unwrap();
    let producer = read_native_capture_producer(&config.producer_manifest_path, &limits).unwrap();
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
                    write_pair(&result_path, &lease)?;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error("owned resize pair was not completed"))
        });
        let status = run_with_test_services(
            &settings,
            &config.descriptor_path,
            RunnerTestServices {
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
