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
