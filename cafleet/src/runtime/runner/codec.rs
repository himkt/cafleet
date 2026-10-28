use super::*;
use std::time::Instant;

#[cfg(test)]
mod input_frames_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }
    fn later(origin: Instant, millis: u64) -> Instant {
        origin.checked_add(Duration::from_millis(millis)).unwrap()
    }
    fn legacy() {
        let mut decoder = ControlDecoder::new();
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        assert_eq!(decoder.feed(&frame(body)).unwrap().len(), 1);
        decoder.finish().unwrap();
    }

    #[test]
    fn step5_wire_f1_every_split_and_single_byte_feed_retains_first_consumption_time() {
        legacy();
        let body = br#"{"protocol_version":1,"kind":"health","text":"literal \u4e16\u754c"}"#;
        let bytes = frame(body);
        let t = Instant::now();
        for cut in 0..bytes.len() {
            let mut decoder = ControlDecoder::new();
            assert!(decoder.feed_admitted(&bytes[..cut], t).unwrap().is_empty());
            let frames = decoder.feed_admitted(&bytes[cut..], later(t, 100)).unwrap();
            assert_eq!(frames.len(), 1, "split {cut}");
            assert_eq!(frames[0].body(), body, "split {cut}");
            assert_eq!(
                frames[0].admitted_at(),
                if cut == 0 { later(t, 100) } else { t },
                "split {cut}"
            );
            decoder.finish().unwrap();
        }
        let mut decoder = ControlDecoder::new();
        let mut frames = Vec::new();
        for (index, byte) in bytes.iter().enumerate() {
            frames.extend(
                decoder
                    .feed_admitted(&[*byte], later(t, u64::try_from(index).unwrap()))
                    .unwrap(),
            );
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].body(), body);
        assert_eq!(frames[0].admitted_at(), t);
        decoder.finish().unwrap();
    }

    #[test]
    fn step5_wire_f2_empty_feeds_leave_admission_unset_and_preserve_partial_frame_time() {
        legacy();
        let t = Instant::now();
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        let bytes = frame(body);
        let mut decoder = ControlDecoder::new();
        assert!(decoder.feed_admitted(&[], t).unwrap().is_empty());
        decoder.finish().unwrap();
        assert!(
            decoder
                .feed_admitted(&bytes[..1], later(t, 10))
                .unwrap()
                .is_empty()
        );
        assert!(decoder.finish().is_err());
        assert!(decoder.feed_admitted(&[], later(t, 20)).unwrap().is_empty());
        let frames = decoder.feed_admitted(&bytes[1..], later(t, 30)).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].body(), body);
        assert_eq!(frames[0].admitted_at(), later(t, 10));
        decoder.finish().unwrap();
    }

    #[test]
    fn step5_wire_f3_coalesced_frames_share_read_time_without_aggregate_cap() {
        legacy();
        let t = Instant::now();
        let body = vec![b'x'; 65536];
        let mut decoder = ControlDecoder::new();
        let frames = decoder.feed_admitted(&frame(&body).repeat(3), t).unwrap();
        assert_eq!(frames.len(), 3);
        for observed in frames {
            assert_eq!(observed.body(), body);
            assert_eq!(observed.admitted_at(), t);
        }
        decoder.finish().unwrap();
    }

    #[test]
    fn step5_wire_f4_partial_complete_and_new_tail_keep_each_first_header_time() {
        legacy();
        let bodies = [b"first".as_slice(), b"second", b"third"];
        let encoded = bodies.map(frame);
        let t = Instant::now();
        let mut decoder = ControlDecoder::new();
        assert!(
            decoder
                .feed_admitted(&encoded[0][..2], t)
                .unwrap()
                .is_empty()
        );
        let mut next = encoded[0][2..].to_vec();
        next.extend_from_slice(&encoded[1]);
        next.extend_from_slice(&encoded[2][..1]);
        let frames = decoder.feed_admitted(&next, later(t, 10)).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].body(), bodies[0]);
        assert_eq!(frames[0].admitted_at(), t);
        assert_eq!(frames[1].body(), bodies[1]);
        assert_eq!(frames[1].admitted_at(), later(t, 10));
        assert!(decoder.finish().is_err());
        let tail = decoder
            .feed_admitted(&encoded[2][1..], later(t, 20))
            .unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].body(), bodies[2]);
        assert_eq!(tail[0].admitted_at(), later(t, 10));
        decoder.finish().unwrap();
    }

    #[test]
    fn step5_wire_f5_inclusive_body_limit_and_header_only_oversize_rejection() {
        legacy();
        let t = Instant::now();
        let body = vec![b'x'; 65536];
        let mut decoder = ControlDecoder::new();
        let frames = decoder.feed_admitted(&frame(&body), t).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].body(), body);
        assert_eq!(frames[0].admitted_at(), t);
        decoder.finish().unwrap();
        for size in [65537u32, u32::MAX] {
            let mut decoder = ControlDecoder::new();
            let header = size.to_be_bytes();
            for byte in &header[..3] {
                assert!(decoder.feed_admitted(&[*byte], t).unwrap().is_empty());
            }
            assert!(
                decoder.feed_admitted(&header[3..], later(t, 10)).is_err(),
                "declared {size}"
            );
        }
    }

    #[test]
    fn step5_wire_f6_eof_rejects_each_partial_boundary_and_empty_body_reaches_consumer() {
        legacy();
        let t = Instant::now();
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        let bytes = frame(body);
        ControlDecoder::new().finish().unwrap();
        for cut in 1..bytes.len() {
            let mut decoder = ControlDecoder::new();
            assert!(decoder.feed_admitted(&bytes[..cut], t).unwrap().is_empty());
            assert!(decoder.finish().is_err(), "prefix {cut}");
        }
        let mut decoder = ControlDecoder::new();
        let frames = decoder.feed_admitted(&frame(&[]), t).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].body().is_empty());
        assert_eq!(frames[0].admitted_at(), t);
        decoder.finish().unwrap();
        assert!(ControlDecoder::new().feed(&frame(&[])).is_err());
    }

    #[test]
    fn step5_wire_f7_raw_bytes_preserve_whitespace_and_invalid_json_for_consumer_validation() {
        legacy();
        let t = Instant::now();
        for (body, valid) in [
            (
                b"{\n \"protocol_version\":1,\"kind\":\"health\",\"text\":\"literal \\u00e9\"\n}"
                    .as_slice(),
                true,
            ),
            (b"{".as_slice(), false),
            (b"\xff".as_slice(), false),
            (b"null".as_slice(), false),
        ] {
            let mut decoder = ControlDecoder::new();
            let frames = decoder.feed_admitted(&frame(body), t).unwrap();
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].body(), body);
            assert_eq!(frames[0].admitted_at(), t);
            decoder.finish().unwrap();
            assert_eq!(ControlDecoder::new().feed(&frame(body)).is_ok(), valid);
        }
    }
}

#[cfg(test)]
mod control_codec_tests {
    use super::*;
    use crate::broker::runtime as storage;
    use crate::runtime::runner_tests::{Fixture, read_fixture_bytes, write_fixture_bytes};
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};

    fn independently_framed(body: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
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
}

pub(crate) const CONTROL_PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAX_CONTROL_FRAME: usize = 65536;

fn validate_version(frame: &Value) -> Result<()> {
    ensure(
        frame.is_object()
            && frame.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(CONTROL_PROTOCOL_VERSION)),
        "control frame requires integer protocol_version 1",
    )
}

pub(crate) fn encode_control_frame(frame: &Value) -> Result<Vec<u8>> {
    validate_version(frame)?;
    let body = serde_json::to_vec(frame).map_err(io_error)?;
    frame_control_body(&body)
}

pub(super) fn frame_control_body(body: &[u8]) -> Result<Vec<u8>> {
    ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    let mut bytes = u32::try_from(body.len())
        .map_err(io_error)?
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(body);
    Ok(bytes)
}

pub(super) fn decode_legacy_control_body(body: &[u8]) -> Result<Value> {
    let frame = serde_json::from_slice(body).map_err(|_| error("malformed control JSON body"))?;
    validate_version(&frame)?;
    Ok(frame)
}

pub(super) struct AdmittedControlFrame {
    body: Vec<u8>,
    admitted_at: Instant,
}
impl AdmittedControlFrame {
    pub(super) fn body(&self) -> &[u8] {
        &self.body
    }
    pub(super) fn admitted_at(&self) -> Instant {
        self.admitted_at
    }
}

pub(crate) struct ControlDecoder {
    header: [u8; 4],
    header_len: usize,
    body: Vec<u8>,
    expected: Option<usize>,
    admitted_at: Option<Instant>,
}

impl ControlDecoder {
    pub(crate) fn new() -> Self {
        Self {
            header: [0; 4],
            header_len: 0,
            body: Vec::new(),
            expected: None,
            admitted_at: None,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the compatible Value decoder remains available beside the shared admitted framer"
        )
    )]
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.feed_admitted(bytes, Instant::now())?
            .into_iter()
            .map(|frame| decode_legacy_control_body(frame.body()))
            .collect()
    }

    pub(super) fn feed_admitted(
        &mut self,
        mut bytes: &[u8],
        observed_at: Instant,
    ) -> Result<Vec<AdmittedControlFrame>> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if self.expected.is_none() {
                if self.header_len == 0 {
                    self.admitted_at = Some(observed_at);
                }
                let count = (4 - self.header_len).min(bytes.len());
                self.header[self.header_len..self.header_len + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_len += count;
                bytes = &bytes[count..];
                if self.header_len < 4 {
                    continue;
                }
                let length = usize::try_from(u32::from_be_bytes(self.header)).map_err(io_error)?;
                ensure(
                    length <= MAX_CONTROL_FRAME,
                    "control frame exceeds size limit",
                )?;
                self.expected = Some(length);
            }
            let length = self
                .expected
                .expect("a complete header declares the body length");
            let count = (length - self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.body.len() == length {
                frames.push(AdmittedControlFrame {
                    body: std::mem::take(&mut self.body),
                    admitted_at: self
                        .admitted_at
                        .take()
                        .expect("a complete frame consumed its first header byte"),
                });
                self.header_len = 0;
                self.expected = None;
            }
        }
        Ok(frames)
    }

    pub(crate) fn finish(&self) -> Result<()> {
        ensure(
            self.header_len == 0 && self.expected.is_none() && self.admitted_at.is_none(),
            "truncated control frame",
        )
    }
}
