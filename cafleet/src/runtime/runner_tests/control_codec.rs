use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;

fn independently_framed(body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}

pub(super) fn read_fixture_bytes(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) {
    while !bytes.is_empty() {
        assert!(Instant::now() < deadline, "fixture request read timed out");
        match stream.read(bytes) {
            Ok(0) => panic!("actual health client closed an incomplete request"),
            Ok(count) => bytes = &mut bytes[count..],
            Err(cause)
                if matches!(
                    cause.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(
                    Duration::from_millis(2)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(cause) => panic!("fixture request read failed: {cause}"),
        }
    }
}

pub(super) fn write_fixture_bytes(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) {
    while !bytes.is_empty() {
        assert!(Instant::now() < deadline, "fixture reply write timed out");
        match stream.write(bytes) {
            Ok(0) => panic!("fixture reply write made no progress"),
            Ok(count) => bytes = &bytes[count..],
            Err(cause)
                if matches!(
                    cause.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(
                    Duration::from_millis(2)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(cause) => panic!("fixture reply write failed: {cause}"),
        }
    }
}

#[test]
fn runner_review_codec_encoding_pins_big_endian_length_and_required_version() {
    assert_eq!(CONTROL_PROTOCOL_VERSION, 1);
    assert_eq!(MAX_CONTROL_FRAME, 65536);
    for value in [
        json!({"protocol_version":1,"kind":"health","identity":{},"text":"literal\n世界"}),
        json!({"protocol_version":1,"result":{"text":"literal\nreply"}}),
        json!({"protocol_version":1,"error":"literal\nerror"}),
    ] {
        let body = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            encode_control_frame(&value).unwrap(),
            independently_framed(&body)
        );
    }
}

#[test]
fn runner_review_codec_decodes_every_header_and_body_split() {
    let value = json!({"protocol_version":1,"kind":"health","text":"literal\n世界"});
    let bytes = independently_framed(&serde_json::to_vec(&value).unwrap());
    for cut in 0..bytes.len() {
        let mut decoder = ControlDecoder::new();
        assert!(decoder.feed(&bytes[..cut]).unwrap().is_empty());
        assert_eq!(
            decoder.feed(&bytes[cut..]).unwrap().as_slice(),
            std::slice::from_ref(&value)
        );
        decoder.finish().unwrap();
    }
    let mut decoder = ControlDecoder::new();
    let mut observed = Vec::new();
    for byte in bytes {
        observed.extend(decoder.feed(&[byte]).unwrap());
    }
    assert_eq!(observed, [value]);
    decoder.finish().unwrap();
}

#[test]
fn runner_review_codec_keeps_json_whitespace_and_multiple_frames_independent() {
    let request = b"{\n  \"protocol_version\": 1,\n  \"kind\": \"health\"\n}";
    let reply = json!({"protocol_version":1,"result":"line one\nline two"});
    let mut bytes = independently_framed(request);
    bytes.extend(independently_framed(&serde_json::to_vec(&reply).unwrap()));
    let mut decoder = ControlDecoder::new();
    assert_eq!(
        decoder.feed(&bytes).unwrap(),
        [serde_json::from_slice::<Value>(request).unwrap(), reply]
    );
    decoder.finish().unwrap();
}

#[test]
fn runner_review_codec_requires_integer_supported_versions_in_requests_and_replies() {
    for envelope in [
        json!({"kind":"health"}),
        json!({"result":null}),
        json!({"error":"failure"}),
    ] {
        for version in [
            None,
            Some(Value::Null),
            Some(json!("1")),
            Some(json!(1.0)),
            Some(json!(2)),
            Some(json!(-1)),
        ] {
            let mut value = envelope.clone();
            if let Some(version) = version {
                value["protocol_version"] = version;
            }
            assert!(encode_control_frame(&value).is_err(), "{value}");
            let mut decoder = ControlDecoder::new();
            assert!(
                decoder
                    .feed(&independently_framed(&serde_json::to_vec(&value).unwrap()))
                    .is_err(),
                "{value}"
            );
        }
    }
}

#[test]
fn runner_review_codec_rejects_empty_malformed_and_nonobject_bodies() {
    for body in [b"".as_slice(), b"{", b"\xff", b"[]", b"null", b"1"] {
        let mut decoder = ControlDecoder::new();
        assert!(
            decoder.feed(&independently_framed(body)).is_err(),
            "{body:?}"
        );
    }
    for value in [Value::Null, json!([]), json!(1)] {
        assert!(encode_control_frame(&value).is_err());
    }
}

#[test]
fn runner_review_codec_accepts_the_inclusive_body_cap_without_an_aggregate_frame_cap() {
    let mut value = json!({"protocol_version":1,"result":""});
    let overhead = serde_json::to_vec(&value).unwrap().len();
    value["result"] = json!("x".repeat(65536 - overhead));
    let body = serde_json::to_vec(&value).unwrap();
    assert_eq!(body.len(), 65536);
    let encoded = encode_control_frame(&value).unwrap();
    assert_eq!(encoded.len(), 65540);
    assert_eq!(&encoded[..4], &65536u32.to_be_bytes());
    let frames = independently_framed(&body).repeat(3);
    let mut decoder = ControlDecoder::new();
    assert_eq!(decoder.feed(&frames).unwrap(), vec![value.clone(); 3]);
    decoder.finish().unwrap();
    value["result"] = json!("x".repeat(65537 - overhead));
    assert!(encode_control_frame(&value).is_err());
}

#[test]
fn runner_review_codec_rejects_oversized_lengths_at_the_fourth_header_byte() {
    for size in [65537u32, u32::MAX] {
        let header = size.to_be_bytes();
        let mut decoder = ControlDecoder::new();
        for byte in &header[..3] {
            assert!(decoder.feed(&[*byte]).unwrap().is_empty());
        }
        assert!(decoder.feed(&header[3..]).is_err());
    }
}

#[test]
fn runner_review_codec_eof_rejects_partial_headers_and_bodies() {
    let bytes = independently_framed(br#"{"protocol_version":1,"result":null}"#);
    ControlDecoder::new().finish().unwrap();
    for cut in 1..bytes.len() {
        let mut decoder = ControlDecoder::new();
        assert!(decoder.feed(&bytes[..cut]).unwrap().is_empty());
        assert!(decoder.finish().is_err(), "prefix {cut}");
    }
    let mut decoder = ControlDecoder::new();
    assert_eq!(decoder.feed(&bytes).unwrap().len(), 1);
    decoder.finish().unwrap();
}

#[test]
fn runner_review_actual_health_client_uses_framing_and_validates_reply_versions() {
    for version in [
        Some(json!(1)),
        None,
        Some(json!(2)),
        Some(json!("1")),
        Some(json!(1.0)),
    ] {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .unwrap();
        let process = ProcessIdentity::current().unwrap();
        lock.transfer_to_runner(&mut fixture.conn, &fixture.run.owner(), process.clone())
            .unwrap();
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET runner_pid=?1,runner_start_identity=?2 WHERE run_id=?3",
                rusqlite::params![process.pid, process.start_identity, fixture.run.run_id],
            )
            .unwrap();
        let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let listener = UnixListener::bind(&artifacts.paths.control_socket).unwrap();
        std::fs::set_permissions(
            &artifacts.paths.control_socket,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut reply = json!({"result": {"identity":ControlIdentity::from_run(&run),"runner":process,"child":null,"output_seq":7}});
        if let Some(version) = &version {
            reply["protocol_version"] = version.clone();
        }
        let expected_identity = ControlIdentity::from_run(&run);
        let valid = version == Some(json!(1));
        let deadline = Instant::now() + Duration::from_secs(2);
        std::thread::scope(|scope| {
            let server = scope.spawn(move || {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(cause) => panic!("fixture socket accept failed: {cause}"),
                    }
                    assert!(Instant::now() < deadline, "fixture socket accept timed out");
                    std::thread::sleep(
                        Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                };
                stream.set_nonblocking(true).unwrap();
                let flags = nix::fcntl::OFlag::from_bits_retain(
                    nix::fcntl::fcntl(&stream, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
                );
                assert!(flags.contains(nix::fcntl::OFlag::O_NONBLOCK));
                let read_deadline = deadline.min(Instant::now() + Duration::from_secs(1));
                let mut header = [0; 4];
                read_fixture_bytes(&mut stream, &mut header, read_deadline);
                let length = usize::try_from(u32::from_be_bytes(header)).unwrap();
                assert!(
                    length > 0 && length <= 65536,
                    "actual client sent invalid length {length}"
                );
                let mut body = vec![0; length];
                read_fixture_bytes(&mut stream, &mut body, read_deadline);
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["protocol_version"], 1);
                assert_eq!(request["kind"], "health");
                assert_eq!(
                    request["identity"],
                    serde_json::to_value(expected_identity).unwrap()
                );
                for byte in independently_framed(&serde_json::to_vec(&reply).unwrap()) {
                    write_fixture_bytes(&mut stream, &[byte], deadline);
                }
            });
            let observed = probe_health(&run, deadline);
            assert_eq!(observed.is_ok(), valid, "{observed:?}");
            if let Ok(health) = observed {
                assert_eq!(health.output_seq, 7);
            }
            server.join().unwrap();
        });
    }
}
