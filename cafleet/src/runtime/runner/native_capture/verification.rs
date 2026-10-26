use super::*;
use crate::broker::runtime as storage;
use crate::config::Settings;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePolicyFile {
    schema_version: u32,
    masks: Vec<CaptureMask>,
}
pub(crate) fn run_native_capture_verification(config: &Path) -> Result<i32> {
    run_native_capture_verification_with_export_hooks(config, Arc::new(InertCaptureExportHooks))
}
pub(crate) fn run_native_capture_verification_with_export_hooks(
    config: &Path,
    hooks: Arc<dyn NativeCaptureExportHooks>,
) -> Result<i32> {
    let limits = NativeCaptureLimits::default();
    let config = read_native_capture_config(config, &limits)?;
    let settings = Settings {
        database_url: config.database_url.clone(),
        max_text_len: 200,
        multiplexer: None,
    };
    let conn = crate::db::connect(&settings.database_url)?;
    let descriptor = load_descriptor(&conn, &config.descriptor_path)?;
    let run = storage::get_run(&conn, descriptor.run_id)?;
    ensure(
        run.run_id == config.expected_run_id && run.launch_nonce == config.expected_launch_nonce,
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
    let (mut port, receiver) = native_capture_channel(&run, capture_policy, limits.clone())?;
    port.service_producer = Some(read_native_capture_producer(
        &config.producer_manifest_path,
        &limits,
    )?);
    let task = CaptureExporterTask::install(&mut port, receiver, config.clone(), limits, hooks);
    let result = run_with_test_services(
        &settings,
        &config.descriptor_path,
        RunnerTestServices {
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
