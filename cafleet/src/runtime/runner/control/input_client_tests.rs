use super::*;
use crate::broker::runtime::InputKind;
use crate::runtime::runner::input_protocol::{
    InputControlOperation, InputControlReply, InputControlRequest, SubmissionStage,
    SubmissionStatus, SubmitInput,
};
use crate::runtime::runner_tests::Fixture;
use serde_json::json;
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

type PeerResult<T> = std::result::Result<T, String>;
fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}
fn request(fixture: &Fixture) -> InputControlRequest {
    InputControlRequest {
        protocol_version: 1,
        identity: ControlIdentity::from_run(&fixture.run),
        expires_at_mono_ns: 50_000_000_000,
        operation: InputControlOperation::SubmitPrompt(SubmitInput {
            operation_key: "fixture:typed-client".into(),
            input_kind: InputKind::Prompt,
            message_id: None,
            literal: "literal 世界 '$x'".into(),
        }),
    }
}
fn expected(fixture: &Fixture) -> Value {
    let run = &fixture.run;
    json!({"protocol_version":1,"kind":"submit_prompt","identity":{"database_identity_json":run.database_identity_json,"fleet_id":run.fleet_id,"member_id":run.member_id,"director_member_id":run.director_member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,"launch_nonce":run.launch_nonce},"expires_at_mono_ns":50_000_000_000u64,"operation_key":"fixture:typed-client","input_kind":"prompt","message_id":null,"literal":"literal 世界 '$x'"})
}
fn reply(status: &str, stage: &str) -> Vec<u8> {
    let reason = if status == "accepted" {
        Value::Null
    } else {
        json!("fixture uncertainty")
    };
    frame(
        &serde_json::to_vec(
            &json!({"protocol_version":1,"result":{"kind":"submission","submission":{
                "status":status,"input_id":13,"stage":stage,"output_seq":null,"reason":reason
            }}}),
        )
        .unwrap(),
    )
}
fn pair() -> (ControlClient, UnixStream) {
    let (stream, peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    (
        ControlClient {
            stream,
            decoder: ControlDecoder::new(),
        },
        peer,
    )
}
fn active(stop: &AtomicBool, deadline: Instant) -> PeerResult<()> {
    if stop.load(Ordering::Acquire) {
        return Err("owned peer cancelled".into());
    }
    if Instant::now() >= deadline {
        return Err("owned peer deadline expired".into());
    }
    Ok(())
}
fn write_peer(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    stop: &AtomicBool,
    deadline: Instant,
) -> PeerResult<()> {
    while !bytes.is_empty() {
        active(stop, deadline)?;
        match stream.write(bytes) {
            Ok(0) => return Err("peer write made no progress".into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                std::thread::yield_now()
            }
            Err(error) => return Err(format!("peer write: {error}")),
        }
    }
    Ok(())
}
fn read_peer(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    stop: &AtomicBool,
    deadline: Instant,
) -> PeerResult<()> {
    while !bytes.is_empty() {
        active(stop, deadline)?;
        match stream.read(bytes) {
            Ok(0) => return Err("peer received truncated request".into()),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                std::thread::yield_now()
            }
            Err(error) => return Err(format!("peer read: {error}")),
        }
    }
    Ok(())
}
fn read_request(
    stream: &mut UnixStream,
    stop: &AtomicBool,
    deadline: Instant,
) -> PeerResult<Value> {
    let mut header = [0; 4];
    read_peer(stream, &mut header, stop, deadline)?;
    let size = usize::try_from(u32::from_be_bytes(header)).map_err(|error| error.to_string())?;
    if size > 65536 {
        return Err("peer received oversized request".into());
    }
    let mut body = vec![0; size];
    read_peer(stream, &mut body, stop, deadline)?;
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}
struct Observed {
    request: Value,
    at: Instant,
    fragments_at: Option<(Instant, Instant)>,
    drained_after_return_at: Option<Instant>,
    extra_connections: usize,
    extra_request_bytes: usize,
}
static UNCONFIRMED_PEERS: Mutex<Vec<JoinHandle<PeerResult<Observed>>>> = Mutex::new(Vec::new());
struct Peer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<PeerResult<Observed>>>,
}
impl Peer {
    fn spawn(
        work: impl FnOnce(Arc<AtomicBool>, Instant) -> PeerResult<Observed> + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let control = stop.clone();
        let until = Instant::now() + Duration::from_secs(2);
        let handle = std::thread::spawn(move || work(control, until));
        Self {
            stop,
            handle: Some(handle),
        }
    }
    fn finish(&mut self) -> PeerResult<Observed> {
        self.finish_until(Instant::now() + Duration::from_secs(2))
    }
    fn finish_until(&mut self, until: Instant) -> PeerResult<Observed> {
        self.stop.store(true, Ordering::Release);
        while !self
            .handle
            .as_ref()
            .expect("owned peer finishes once")
            .is_finished()
        {
            if Instant::now() >= until {
                return Err(
                    "owned peer cleanup unconfirmed: deadline expired; handle retained".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.handle
            .take()
            .expect("completed peer remains owned until join")
            .join()
            .map_err(|_| "owned peer terminated with a panic; join failed".to_owned())?
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        if self.handle.is_some()
            && let Err(error) = self.finish()
        {
            if let Some(handle) = self.handle.take() {
                let mut retained = match UNCONFIRMED_PEERS.lock() {
                    Ok(retained) => retained,
                    Err(poisoned) => {
                        eprintln!("recovering poisoned unconfirmed-peer ownership registry");
                        poisoned.into_inner()
                    }
                };
                retained.push(handle);
                eprintln!(
                    "unconfirmed peer handle retained until process exit; thread termination remains unconfirmed"
                );
            }
            if std::thread::panicking() {
                eprintln!("owned peer cleanup: {error}");
            } else {
                panic!("owned peer cleanup: {error}");
            }
        }
    }
}
fn finish_exchange(peer: &mut Peer, result: &Result<InputControlReply>) -> Observed {
    match peer.finish() {
        Ok(observed) => observed,
        Err(cleanup) => {
            let primary = match result {
                Err(error) => format!("Err({error:?})"),
                Ok(InputControlReply::Submission(value)) => format!(
                    "Ok(Submission status={:?}, input_id={:?}, stage={:?}, output_seq={:?}, reason={:?})",
                    value.status, value.input_id, value.stage, value.output_seq, value.reason
                ),
                Ok(InputControlReply::Inspection(_)) => "Ok(Inspection)".into(),
                Ok(InputControlReply::PermissionResponse(_)) => "Ok(PermissionResponse)".into(),
                Ok(InputControlReply::Receipt(_)) => "Ok(Receipt)".into(),
            };
            panic!("primary exchange result: {primary}; owned peer cleanup: {cleanup}");
        }
    }
}
fn accepted(result: Result<InputControlReply>) {
    match result.unwrap() {
        InputControlReply::Submission(submission) => {
            assert_eq!(submission.status, SubmissionStatus::Accepted);
            assert_eq!(submission.input_id, Some(13));
            assert_eq!(submission.stage, SubmissionStage::Submitted);
            assert_eq!(submission.output_seq, None);
            assert_eq!(submission.reason, None);
        }
        _ => panic!("actual typed exchange returns submission"),
    }
}
fn preloaded(fixture: &Fixture, response: &[u8]) -> (Result<InputControlReply>, Value) {
    let (mut client, mut peer) = pair();
    let until = Instant::now() + Duration::from_secs(1);
    let stop = AtomicBool::new(false);
    write_peer(&mut peer, response, &stop, until).unwrap();
    let result = client.exchange_input(&request(fixture), until);
    let actual = read_request(&mut peer, &stop, until).unwrap();
    (result, actual)
}
fn baseline(fixture: &Fixture) {
    let (result, actual) = preloaded(fixture, &reply("accepted", "submitted"));
    accepted(result);
    assert_eq!(actual, expected(fixture));
}

#[test]
fn step5_wire_x1_actual_typed_exchange_sends_independent_flat_request_and_original_expiry() {
    let fixture = Fixture::new();
    baseline(&fixture);
}
#[test]
fn step5_wire_x2_fragmented_reply_and_raw_duplicate_or_wrong_kind_validation() {
    let fixture = Fixture::new();
    baseline(&fixture);
    let (mut client, mut stream) = pair();
    let response = reply("accepted", "submitted");
    let mut peer = Peer::spawn(move |stop, until| {
        let request = read_request(&mut stream, &stop, until)?;
        let at = Instant::now();
        for chunk in response.chunks(3) {
            write_peer(&mut stream, chunk, &stop, until)?;
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Observed {
            request,
            at,
            fragments_at: None,
            drained_after_return_at: None,
            extra_connections: 0,
            extra_request_bytes: 0,
        })
    });
    let result = client.exchange_input(&request(&fixture), Instant::now() + Duration::from_secs(1));
    let observed = finish_exchange(&mut peer, &result);
    accepted(result);
    assert_eq!(observed.request, expected(&fixture));
    let duplicate=frame(br#"{"protocol_version":1,"result":{"kind":"submission","submission":{"status":"accepted","input_id":13,"input_id":13,"stage":"submitted","output_seq":null,"reason":null}}}"#);
    let wrong=frame(br#"{"protocol_version":1,"result":{"kind":"receipt","receipt":{"status":"missing","input_id":null,"submission":null}}}"#);
    for response in [duplicate, wrong] {
        let (result, actual) = preloaded(&fixture, &response);
        assert!(result.is_err());
        assert_eq!(actual, expected(&fixture));
    }
}
#[test]
fn step5_wire_x3_extra_reply_trailing_partial_and_premature_eof_reject() {
    let fixture = Fixture::new();
    baseline(&fixture);
    let good = reply("accepted", "submitted");
    let mut extra = good.clone();
    extra.extend_from_slice(&good);
    let mut partial = good.clone();
    partial.extend_from_slice(&good[..2]);
    for response in [extra, partial] {
        let (result, actual) = preloaded(&fixture, &response);
        assert!(result.is_err());
        assert_eq!(actual, expected(&fixture));
    }
    let (mut client, mut peer) = pair();
    let until = Instant::now() + Duration::from_secs(1);
    let stop = AtomicBool::new(false);
    write_peer(&mut peer, &good[..good.len() - 1], &stop, until).unwrap();
    peer.shutdown(Shutdown::Write).unwrap();
    let result = client.exchange_input(&request(&fixture), until);
    let actual = read_request(&mut peer, &stop, until).unwrap();
    assert!(result.is_err());
    assert_eq!(actual, expected(&fixture));
}
#[test]
fn step5_wire_x4_expired_and_stalled_actual_exchange_keep_original_deadline() {
    let fixture = Fixture::new();
    baseline(&fixture);
    let (mut client, mut peer) = pair();
    let expired = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    assert!(client.exchange_input(&request(&fixture), expired).is_err());
    let mut byte = [0];
    let observation_deadline = Instant::now() + Duration::from_millis(100);
    loop {
        assert!(Instant::now() < observation_deadline);
        match peer.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => panic!("an expired invocation sends zero request bytes"),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => panic!("observe expired exchange bytes: {error}"),
        }
    }
    let (mut client, mut stream) = pair();
    let budget = Duration::from_millis(800);
    let fragment_delay = Duration::from_millis(350);
    let return_tolerance = Duration::from_millis(100);
    let started = Instant::now();
    let deadline = started + budget;
    let mut peer = Peer::spawn(move |stop, until| {
        let request = read_request(&mut stream, &stop, until)?;
        let at = Instant::now();
        write_peer(&mut stream, &[0, 0], &stop, until)?;
        let first_fragment_at = Instant::now();
        let next_fragment_at = first_fragment_at + fragment_delay;
        while Instant::now() < next_fragment_at {
            active(&stop, until)?;
            std::thread::sleep(Duration::from_millis(1));
        }
        write_peer(&mut stream, &[0, 20], &stop, until)?;
        let later_fragment_at = Instant::now();
        while !stop.load(Ordering::Acquire) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Observed {
            request,
            at,
            fragments_at: Some((first_fragment_at, later_fragment_at)),
            drained_after_return_at: None,
            extra_connections: 0,
            extra_request_bytes: 0,
        })
    });
    let result = client.exchange_input(&request(&fixture), deadline);
    let returned = Instant::now();
    let observed = finish_exchange(&mut peer, &result);
    let (first_fragment_at, later_fragment_at) = observed.fragments_at.unwrap();
    eprintln!(
        "X4 original budget={budget:?}; request={:?}; first fragment={:?}; later fragment={:?}; return={:?}; declared tolerance={return_tolerance:?}",
        observed.at.duration_since(started),
        first_fragment_at.duration_since(started),
        later_fragment_at.duration_since(started),
        returned.duration_since(started)
    );
    assert!(result.is_err());
    assert_eq!(observed.request, expected(&fixture));
    assert!(
        observed.at < deadline,
        "peer observed the actual request before the tested expiry"
    );
    assert!(observed.at <= first_fragment_at);
    assert!(first_fragment_at < later_fragment_at);
    assert!(later_fragment_at < deadline);
    assert!(later_fragment_at.duration_since(first_fragment_at) >= fragment_delay);
    assert!(later_fragment_at.duration_since(started) >= fragment_delay);
    assert!(return_tolerance < later_fragment_at.duration_since(first_fragment_at));
    assert!(returned >= deadline);
    assert!(
        returned < deadline + return_tolerance,
        "actual exchange shares its original budget"
    );
    assert!(returned < later_fragment_at + budget);
}
#[test]
fn step5_wire_x5_disconnect_after_real_request_has_no_reconnect_or_second_request() {
    let fixture = Fixture::new();
    baseline(&fixture);
    let directory = tempfile::Builder::new()
        .prefix(".pwire-")
        .tempdir_in("/tmp")
        .unwrap();
    let path = directory.path().join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let exchange_returned = Arc::new(AtomicBool::new(false));
    let returned_signal = exchange_returned.clone();
    let mut peer = Peer::spawn(move |stop, until| {
        let (mut stream, _) = loop {
            active(&stop, until)?;
            match listener.accept() {
                Ok(value) => break value,
                Err(error) if error.kind() == ErrorKind::WouldBlock => std::thread::yield_now(),
                Err(error) => return Err(error.to_string()),
            }
        };
        stream
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let request = read_request(&mut stream, &stop, until)?;
        let at = Instant::now();
        let mut trailing = [0; 4096];
        let mut extra_request_bytes = 0;
        stream
            .shutdown(Shutdown::Write)
            .map_err(|error| error.to_string())?;
        let mut extra_connections = 0;
        let drained_after_return_at = loop {
            if Instant::now() >= until {
                return Err("disconnect observer expired before final drain".into());
            }
            let after_return = exchange_returned.load(Ordering::Acquire);
            loop {
                if Instant::now() >= until {
                    return Err("disconnect request drain exceeded its bound".into());
                }
                match stream.read(&mut trailing) {
                    Ok(0) => break,
                    Ok(count) => extra_request_bytes += count,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            loop {
                if Instant::now() >= until {
                    return Err("disconnect listener drain exceeded its bound".into());
                }
                match listener.accept() {
                    Ok((extra, _)) => {
                        extra_connections += 1;
                        drop(extra);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            if after_return {
                break Instant::now();
            }
            if stop.load(Ordering::Acquire) && !exchange_returned.load(Ordering::Acquire) {
                return Err("disconnect observer cancelled before actual exchange return".into());
            }
            std::thread::yield_now();
        };
        Ok(Observed {
            request,
            at,
            fragments_at: None,
            drained_after_return_at: Some(drained_after_return_at),
            extra_connections,
            extra_request_bytes,
        })
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut client = ControlClient::connect(&path, deadline).unwrap();
    let result = client.exchange_input(&request(&fixture), deadline);
    let returned_at = Instant::now();
    returned_signal.store(true, Ordering::Release);
    let observed = finish_exchange(&mut peer, &result);
    assert!(result.is_err());
    assert_eq!(observed.request, expected(&fixture));
    assert_eq!(observed.extra_connections, 0);
    assert_eq!(observed.extra_request_bytes, 0);
    assert!(observed.drained_after_return_at.unwrap() >= returned_at);
}

#[test]
fn step5_wire_peer_cleanup_timeout_retains_ownership_until_release_and_confirmed_join() {
    let released = Arc::new(AtomicBool::new(false));
    let release = released.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let mut peer = Peer::spawn(move |_, until| {
        entered_tx.send(()).map_err(|error| error.to_string())?;
        while !released.load(Ordering::Acquire) {
            if Instant::now() >= until {
                return Err("ownership control release deadline expired".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Observed {
            request: json!({"fixture":"ownership-control"}),
            at: Instant::now(),
            fragments_at: None,
            drained_after_return_at: None,
            extra_connections: 0,
            extra_request_bytes: 0,
        })
    });
    let entered = entered_rx.recv_timeout(Duration::from_millis(500));
    let owner_id = peer.handle.as_ref().unwrap().thread().id();
    let timeout = peer.finish_until(Instant::now() + Duration::from_millis(20));
    let retained_id = peer.handle.as_ref().map(|handle| handle.thread().id());
    let remained_live = peer
        .handle
        .as_ref()
        .is_some_and(|handle| !handle.is_finished());
    release.store(true, Ordering::Release);
    let joined = if peer.handle.is_some() {
        Some(peer.finish())
    } else {
        None
    };
    let first_outcome = match &timeout {
        Ok(observed) => format!("Ok(request={:?})", observed.request),
        Err(error) => format!("Err({error:?})"),
    };
    assert!(
        entered.is_ok(),
        "worker entry was acknowledged: {entered:?}; first finish: {first_outcome}"
    );
    match timeout {
        Err(error) => assert_eq!(
            error, "owned peer cleanup unconfirmed: deadline expired; handle retained",
            "timeout scenario was established; first finish: {first_outcome}"
        ),
        Ok(_) => panic!(
            "unreleased worker must exceed the first cleanup deadline; first finish: {first_outcome}"
        ),
    }
    assert_eq!(retained_id, Some(owner_id), "first finish: {first_outcome}");
    assert!(remained_live, "first finish: {first_outcome}");
    match joined {
        Some(Ok(observed)) => {
            assert_eq!(observed.request, json!({"fixture":"ownership-control"}))
        }
        Some(Err(error)) => panic!(
            "released worker cleanup must confirm join: {error}; first finish: {first_outcome}"
        ),
        None => {
            panic!("timeout retains an owned worker for final join; first finish: {first_outcome}")
        }
    }
    assert!(peer.handle.is_none(), "released worker has been joined");
}
#[test]
fn step5_wire_x6_textual_error_remains_error_and_structured_uncertainty_remains_result() {
    let fixture = Fixture::new();
    baseline(&fixture);
    let error = frame(br#"{"protocol_version":1,"error":"fixture failure"}"#);
    let (result, actual) = preloaded(&fixture, &error);
    assert_eq!(actual, expected(&fixture));
    match result {
        Err(CafleetError::App(message)) => assert_eq!(message, "runner control: fixture failure"),
        Err(other) => panic!("ordinary application error expected: {other}"),
        Ok(_) => panic!("textual server failure is an error"),
    }
    for stage in ["partial", "unknown"] {
        let (result, actual) = preloaded(&fixture, &reply("uncertain", stage));
        assert_eq!(actual, expected(&fixture));
        match result.unwrap() {
            InputControlReply::Submission(submission) => {
                assert_eq!(submission.status, SubmissionStatus::Uncertain);
                assert_eq!(submission.stage.as_str(), stage);
                assert_eq!(submission.input_id, Some(13));
            }
            _ => panic!("structured uncertainty remains a submission result"),
        }
    }
}
