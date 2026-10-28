use super::*;
use crate::runtime::runner_tests::owned_child::OwnedChild;
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub(super) const LITERAL: &str = "ordinary fixture λ";
pub(super) const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
pub(super) const VERSION: &str = "synthetic-0.0.1";
const PROFILE: &str = "synthetic-profile-1";

pub(in crate::runtime::runner::input) fn catalog_definition() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../runner_tests/fixtures/synthetic-profile.json"
    ))
    .unwrap();
    let fixture = value["fixtures"][0].clone();
    let mut executable = fixture.clone();
    executable["id"] = serde_json::json!("synthetic-prompt-plan");
    value["fixtures"].as_array_mut().unwrap().push(executable);
    let rule = &mut value["profiles"][0]["rules"][0];
    rule["id"] = serde_json::json!("synthetic-composer-empty");
    rule["observation"] = serde_json::json!("composer_empty");
    rule["region"]["start_pattern"] = serde_json::json!("^Synthetic state$");
    rule["region"]["end_pattern"] = serde_json::json!("^State end$");
    rule["matchers"] =
        serde_json::json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
    rule["extract"] = serde_json::json!({"operation":null,"choices":[],"native_request_id":null});
    rule["verified_noninteraction"] = serde_json::json!(true);
    value["profiles"][0]["capabilities"] = serde_json::json!({
        "detection":null,"answers":[],"composer":{
            "rule_ids":["synthetic-composer-empty"],"fixture_ids":[fixture["id"]],
            "submission":{"prompt":{"prefix_hex":"","submit_hex":"7e","fixture_ids":["synthetic-prompt-plan"]},"exec":null}
        }
    });
    value
}

pub(super) fn catalog() -> ProfileCatalog {
    ProfileCatalog::from_definitions(&catalog_definition()).unwrap()
}

pub(super) fn encoded() -> Vec<u8> {
    let mut bytes = LITERAL.as_bytes().to_vec();
    bytes.push(b'~');
    bytes
}

pub(in crate::runtime::runner::input) struct HostProfile {
    pub policy: storage::evidence::EvidencePolicy,
    pub event_evidence: &'static str,
    pub backend: crate::coding_agent::CodingAgent,
    pub native_version: &'static str,
    pub profile_version: &'static str,
    pub size: TerminalSize,
    pub screen: Vec<u8>,
}
impl HostProfile {
    pub fn synthetic(
        policy: storage::evidence::EvidencePolicy,
        event_evidence: &'static str,
    ) -> Self {
        Self {
            policy,
            event_evidence,
            backend: crate::coding_agent::CodingAgent::Codex,
            native_version: VERSION,
            profile_version: PROFILE,
            size: TerminalSize { rows: 24, cols: 80 },
            screen: SCREEN.to_vec(),
        }
    }
}

struct Tokens;
impl RequestTokenSource for Tokens {
    fn fill(&mut self, _: &mut [u8; 32]) -> Result<()> {
        panic!("ordinary Prompt creates no permission token")
    }
}

#[derive(Clone, Debug)]
pub(in crate::runtime::runner::input) struct Point {
    pub boundary: &'static str,
    pub id: Option<i64>,
    pub count: u64,
    pub seq: u64,
}

pub(in crate::runtime::runner::input) struct Hooks {
    pub points: Mutex<Vec<Point>>,
    pub until: Instant,
}
impl Hooks {
    pub fn count(&self, boundary: &str) -> usize {
        self.points
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.boundary == boundary)
            .count()
    }
    pub fn input_id(&self) -> i64 {
        self.points
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.boundary == "reserved")
            .expect("real Reserved checkpoint")
            .id
            .expect("actual durable input ID")
    }
}
impl InputHooks for Hooks {
    fn checkpoint(
        &self,
        boundary: InputBoundary,
        state: &InputCheckpoint,
        deadline: Instant,
    ) -> Result<()> {
        assert!(deadline <= self.until);
        let boundary = match boundary {
            InputBoundary::Reserved => "reserved",
            InputBoundary::BeforeWrite => "before",
            InputBoundary::AfterWrite => "after",
            InputBoundary::TerminalState => "terminal",
            InputBoundary::Settled => "settled",
            InputBoundary::Retired => "retired",
        };
        self.points.lock().unwrap().push(Point {
            boundary,
            id: state.input_id,
            count: state.written_bytes,
            seq: state.output_seq,
        });
        Ok(())
    }
}

#[derive(Default)]
pub(in crate::runtime::runner::input) struct Reader {
    pub bytes: VecDeque<u8>,
    pub consumed: usize,
    pub calls: usize,
}
impl ByteInput for Reader {
    fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.bytes.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = target.len().min(self.bytes.len());
        assert!(count > 0);
        for byte in &mut target[..count] {
            *byte = self.bytes.pop_front().unwrap();
        }
        self.consumed += count;
        Ok(count)
    }
}

pub(in crate::runtime::runner::input) struct Pty {
    pub child: Reader,
    pub offers: Vec<Vec<u8>>,
    pub program: Vec<Vec<u8>>,
    pub manual: Vec<u8>,
    pub short: Option<usize>,
    expected_program: Vec<u8>,
    database_url: String,
    hooks: Arc<Hooks>,
}
impl ByteInput for Pty {
    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.child.try_read(bytes)
    }
}
impl PtyIo for Pty {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(!bytes.is_empty());
        self.offers.push(bytes.to_vec());
        if bytes == self.expected_program {
            let conn = crate::db::connect(&self.database_url).unwrap();
            let row = storage::get_input(&conn, self.hooks.input_id()).unwrap();
            assert!(matches!(row.state, storage::InputState::InProgress));
            assert!(self.hooks.count("before") > 0);
            self.program.push(bytes.to_vec());
            Ok(self.short.unwrap_or(bytes.len()))
        } else {
            self.manual.extend_from_slice(bytes);
            Ok(bytes.len())
        }
    }
    fn resize(&mut self, _: TerminalSize) -> Result<()> {
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
#[derive(Default)]
pub(in crate::runtime::runner::input) struct Output(pub Vec<u8>);
impl Display for Output {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn try_flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Child {
    native: OwnedChild,
    until: Instant,
}
impl Drop for Child {
    fn drop(&mut self) {
        self.native.terminate_until(self.until);
    }
}

pub(in crate::runtime::runner::input) struct Host {
    pub fixture: Fixture,
    _artifacts: RunArtifacts,
    pub coordinator: InputCoordinator,
    pub writer: EventWriter,
    pub controller: ObservationController,
    pub transport: Transport,
    pub human: Reader,
    pub pty: Pty,
    pub output: Output,
    pub hooks: Arc<Hooks>,
    pub until: Instant,
    expires: u64,
    child: Child,
}

impl Host {
    pub fn new() -> Self {
        Self::build(
            catalog,
            encoded(),
            HostProfile::synthetic(storage::evidence::EvidencePolicy::metadata(), "metadata"),
            Instant::now() + Duration::from_secs(5),
        )
    }
    fn with_catalog(
        catalog: ProfileCatalog,
        expected_program: Vec<u8>,
        profile: HostProfile,
    ) -> Self {
        Self::build(
            || catalog,
            expected_program,
            profile,
            Instant::now() + Duration::from_secs(5),
        )
    }
    fn build(
        catalog: impl FnOnce() -> ProfileCatalog,
        expected_program: Vec<u8>,
        profile: HostProfile,
        until: Instant,
    ) -> Self {
        assert!(
            Instant::now() < until,
            "original fixture construction deadline"
        );
        let expires = InputExpiration::capture(until, &mut RealInputDeadlineClock)
            .unwrap()
            .monotonic_ns();
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
        let catalog = catalog();
        let coverage = catalog
            .capabilities(profile.backend, Some(profile.native_version))
            .detection;
        fixture.conn.execute("UPDATE member_runs SET backend=?1,native_version=?2,profile_version=?3,coverage=?4,event_evidence=?5 WHERE run_id=?6",rusqlite::params![profile.backend.name(),profile.native_version,profile.profile_version,coverage.as_str(),profile.event_evidence,fixture.run.run_id]).unwrap();
        fixture
            .conn
            .execute(
                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                rusqlite::params![profile.backend.name(), fixture.run.member_id],
            )
            .unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let mut controller =
            ObservationController::new(&fixture.run, &catalog, profile.policy, 1..=256).unwrap();
        let url = format!("sqlite:///{}", fixture.conn.path().unwrap());
        let mut writer = EventWriter::start(
            url.clone(),
            fixture.run.clone(),
            lock.clone(),
            artifacts.paths.outbox.clone(),
        )
        .unwrap();
        writer.set_deadline(until);
        let child = Child {
            native: OwnedChild::new(
                Command::new("/bin/cat")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            ),
            until,
        };
        let identity = ProcessIdentity::read(i64::from(child.native.id()))
            .unwrap()
            .expect("owned live child");
        assert!(identity.matches().unwrap());
        fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![identity.pid,identity.start_identity,crate::time::format_utc(crate::time::now_utc()),fixture.run.run_id]).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let transport = Self::transport_sized(profile.size);
        controller
            .started(
                &fixture.run,
                &transport.snapshot(),
                &writer,
                &RealObservationClock,
            )
            .unwrap();
        let hooks = Arc::new(Hooks {
            points: Mutex::new(Vec::new()),
            until,
        });
        let coordinator = InputCoordinator::new(
            &mut fixture.conn,
            &fixture.run,
            lock,
            InputServices {
                catalog,
                token_source: Box::new(Tokens),
                hooks: hooks.clone(),
            },
            until,
        )
        .unwrap();
        let pty = Pty {
            child: Reader {
                bytes: profile.screen.into_iter().collect(),
                ..Reader::default()
            },
            offers: Vec::new(),
            program: Vec::new(),
            manual: Vec::new(),
            short: None,
            expected_program,
            database_url: url,
            hooks: hooks.clone(),
        };
        let mut host = Self {
            fixture,
            _artifacts: artifacts,
            coordinator,
            writer,
            controller,
            transport,
            human: Reader::default(),
            pty,
            output: Output::default(),
            hooks,
            until,
            expires,
            child,
        };
        host.drain();
        assert!(matches!(
            host.controller.health(&host.writer).eligibility,
            InputEligibility::ComposerEmpty
        ));
        host
    }
    fn transport_sized(size: TerminalSize) -> Transport {
        Transport::new(
            size,
            TransportLimits {
                input_bytes: 4096,
                display_bytes: 4096,
                bytes_per_tick: 4096,
            },
        )
        .unwrap()
    }
    pub fn request(&self, key: &str) -> InputControlRequest {
        let run = &self.fixture.run;
        InputControlRequest {
            protocol_version: 1,
            identity: ControlIdentity {
                database_identity_json: run.database_identity_json.clone(),
                fleet_id: run.fleet_id,
                member_id: run.member_id,
                director_member_id: run.director_member_id,
                run_id: run.run_id,
                owner_epoch: run.owner_epoch,
                launch_nonce: run.launch_nonce.clone(),
            },
            expires_at_mono_ns: self.expires,
            operation: InputControlOperation::SubmitPrompt(SubmitInput {
                operation_key: key.into(),
                input_kind: storage::InputKind::Prompt,
                message_id: None,
                literal: LITERAL.into(),
            }),
        }
    }
    pub fn drain(&mut self) {
        loop {
            assert!(
                Instant::now() < self.until,
                "actual fixture becomes healthy and quiescent within its original deadline"
            );
            self.controller
                .poll(
                    &self.transport.snapshot(),
                    &self.writer,
                    &RealObservationClock,
                )
                .unwrap();
            let result = self
                .coordinator
                .progress_with_manual(&mut self.transport, |transport| {
                    transport.quiesce_for_input(
                        &mut self.human,
                        &mut self.pty,
                        &mut self.output,
                        &mut self
                            .controller
                            .observer(&self.writer, &RealObservationClock),
                        self.until,
                    )
                })
                .unwrap();
            if matches!(result, InputDrain::Quiescent { .. }) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn advance(&mut self) -> Option<InputControlReply> {
        self.coordinator
            .advance(
                &mut self.fixture.conn,
                &self.fixture.run,
                &mut self.transport,
                &mut self.pty,
                &self.controller,
                self.until,
            )
            .unwrap()
    }
    pub fn arm(&mut self, key: &str) -> i64 {
        let request = self.request(key);
        assert!(
            self.coordinator
                .handle(
                    &mut self.fixture.conn,
                    &self.fixture.run,
                    request,
                    &self.controller,
                    self.until
                )
                .unwrap()
                .is_none()
        );
        for _ in 0..8 {
            self.drain();
            assert!(self.advance().is_none());
            assert!(self.pty.program.is_empty());
            if self.hooks.count("before") == 1 {
                let id = self.hooks.input_id();
                assert!(matches!(
                    storage::get_input(&self.fixture.conn, id).unwrap().state,
                    storage::InputState::InProgress
                ));
                return id;
            }
            if self.hooks.count("reserved") == 1 {
                assert!(self.advance().is_none());
                assert_eq!(
                    self.hooks.count("before"),
                    0,
                    "reservation consumes its old drain proof"
                );
            }
        }
        panic!("real reservation and BeforeWrite reach Armed within finite turns");
    }
    pub fn finish(&mut self) {
        let counts = self.writer.finish(self.until);
        let status = self.writer.status();
        self.child.native.terminate_until(self.until);
        let child = self.child.native.try_wait();
        assert_eq!(counts, (0, 0), "{status:?}");
        assert!(
            !status.degraded && status.diagnostic.is_none(),
            "writer join: {status:?}"
        );
        assert!(child.unwrap().is_some(), "owned child reap confirmed");
    }
}
pub(in crate::runtime::runner::input) fn with_host(body: impl FnOnce(&mut Host)) {
    with_owned_host(Host::new(), body);
}
pub(in crate::runtime::runner::input) fn with_host_until(
    until: Instant,
    body: impl FnOnce(&mut Host),
) {
    with_owned_host(
        Host::build(
            catalog,
            encoded(),
            HostProfile::synthetic(storage::evidence::EvidencePolicy::metadata(), "metadata"),
            until,
        ),
        body,
    );
}
pub(in crate::runtime::runner::input) fn with_catalog_host(
    catalog: ProfileCatalog,
    expected_program: Vec<u8>,
    body: impl FnOnce(&mut Host),
) {
    with_profile_host(
        catalog,
        expected_program,
        HostProfile::synthetic(storage::evidence::EvidencePolicy::metadata(), "metadata"),
        body,
    );
}
pub(in crate::runtime::runner::input) fn with_profile_host(
    catalog: ProfileCatalog,
    expected_program: Vec<u8>,
    profile: HostProfile,
    body: impl FnOnce(&mut Host),
) {
    with_owned_host(Host::with_catalog(catalog, expected_program, profile), body);
}
fn with_owned_host(mut host: Host, body: impl FnOnce(&mut Host)) {
    let primary = catch_unwind(AssertUnwindSafe(|| body(&mut host)));
    let cleanup = catch_unwind(AssertUnwindSafe(|| host.finish()));
    if let Err(primary) = primary {
        if let Err(cleanup) = &cleanup {
            eprintln!("ordinary Prompt cleanup also failed: {cleanup:?}");
        }
        resume_unwind(primary);
    }
    if let Err(cleanup) = cleanup {
        resume_unwind(cleanup);
    }
}
