use super::*;
use crate::runtime::runner::codec::AdmittedControlFrame;
use crate::runtime::runner::input_clock::receive_input_expiration;
use crate::runtime::runner::{InputClockSample, InputDeadlineClock};
use std::io::Write;
use std::net::Shutdown;

fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}
fn pair() -> (Client, UnixStream) {
    let (stream, peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    peer.set_nonblocking(true).unwrap();
    (
        Client {
            id: 1,
            stream,
            decoder: ControlDecoder::new(),
            output: VecDeque::new(),
            closed: false,
        },
        peer,
    )
}
fn send(peer: &mut UnixStream, mut bytes: &[u8], deadline: Instant) {
    while !bytes.is_empty() {
        assert!(Instant::now() < deadline, "bounded peer send");
        match peer.write(bytes) {
            Ok(0) => panic!("peer send progresses"),
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                std::thread::yield_now()
            }
            Err(error) => panic!("peer send: {error}"),
        }
    }
}
fn after(previous: Instant, deadline: Instant) {
    while Instant::now() <= previous {
        assert!(
            Instant::now() < deadline,
            "fixture clock advances within its bound"
        );
        std::thread::yield_now();
    }
}
fn collect(client: &mut Client, count: usize, deadline: Instant) -> Vec<AdmittedControlFrame> {
    let mut frames = Vec::new();
    while frames.len() < count {
        assert!(Instant::now() < deadline, "bounded actual client polling");
        frames.extend(client.poll().unwrap());
        std::thread::yield_now();
    }
    assert_eq!(frames.len(), count);
    frames
}
fn first(
    client: &mut Client,
    peer: &mut UnixStream,
    byte: u8,
    deadline: Instant,
) -> (Instant, Instant) {
    client.decoder.finish().unwrap();
    send(peer, &[byte], deadline);
    let before = Instant::now();
    assert!(client.poll().unwrap().is_empty());
    let after = Instant::now();
    assert!(
        client.decoder.finish().is_err(),
        "this actual poll consumed the first header byte"
    );
    (before, after)
}

#[test]
fn step5_wire_a1_actual_poll_proves_first_byte_before_later_completion() {
    let (mut client, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_secs(1);
    let body = br#"{"protocol_version":1,"kind":"health"}"#;
    let bytes = frame(body);
    let (before, first_after) = first(&mut client, &mut peer, bytes[0], deadline);
    after(first_after, deadline);
    send(&mut peer, &bytes[1..], deadline);
    let frames = collect(&mut client, 1, deadline);
    assert_eq!(frames[0].body(), body);
    assert!(before <= frames[0].admitted_at() && frames[0].admitted_at() <= first_after);
    client.decoder.finish().unwrap();
}
#[test]
fn step5_wire_a2_empty_actual_poll_does_not_supply_admission_time() {
    let (mut client, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_secs(1);
    client.decoder.finish().unwrap();
    let empty_at = Instant::now();
    assert!(client.poll().unwrap().is_empty());
    client.decoder.finish().unwrap();
    after(empty_at, deadline);
    let body = br#"{"protocol_version":1,"kind":"health"}"#;
    let bytes = frame(body);
    let (before, end) = first(&mut client, &mut peer, bytes[0], deadline);
    after(end, deadline);
    send(&mut peer, &bytes[1..], deadline);
    let frames = collect(&mut client, 1, deadline);
    assert!(frames[0].admitted_at() > empty_at);
    assert!(before <= frames[0].admitted_at() && frames[0].admitted_at() <= end);
    assert_eq!(frames[0].body(), body);
    client.decoder.finish().unwrap();
}
struct Clock {
    sample: Option<std::result::Result<InputClockSample, CafleetError>>,
    calls: usize,
}
impl Clock {
    fn at(instant: Instant) -> Self {
        Self {
            sample: Some(Ok(InputClockSample {
                before: instant,
                monotonic_ns: 1_000_000_000,
                after: instant,
            })),
            calls: 0,
        }
    }
}
impl InputDeadlineClock for Clock {
    fn sample(&mut self) -> Result<InputClockSample> {
        self.calls += 1;
        self.sample.take().expect("one original paired sample")
    }
}
#[test]
fn step5_wire_a3_actual_frames_keep_first_time_through_coalescence_queue_and_conversion() {
    let (mut client, mut peer) = pair();
    let deadline = Instant::now() + Duration::from_secs(1);
    let first_body = br#"{"protocol_version":1,"kind":"health"}"#;
    let second_body = br#"{"protocol_version":1,"kind":"shutdown"}"#;
    let bytes = frame(first_body);
    let (before, first_after) = first(&mut client, &mut peer, bytes[0], deadline);
    after(first_after, deadline);
    let later_before = Instant::now();
    let mut rest = bytes[1..].to_vec();
    rest.extend(frame(second_body));
    send(&mut peer, &rest, deadline);
    let frames = collect(&mut client, 2, deadline);
    let later_after = Instant::now();
    assert_eq!(frames[0].body(), first_body);
    assert_eq!(frames[1].body(), second_body);
    let times = [frames[0].admitted_at(), frames[1].admitted_at()];
    assert!(before <= times[0] && times[0] <= first_after);
    assert!(later_before <= times[1] && times[1] <= later_after);
    let mut queue: VecDeque<_> = frames.into();
    after(later_after, deadline);
    for expected in times {
        let actual = queue.pop_front().unwrap();
        assert_eq!(actual.admitted_at(), expected);
        let now = Instant::now();
        let mut clock = Clock::at(now);
        let converted =
            receive_input_expiration(11_000_000_000, actual.admitted_at(), None, &mut clock)
                .unwrap();
        assert_eq!(converted, actual.admitted_at() + Duration::from_secs(5));
        assert_eq!(clock.calls, 1);
    }
    assert!(queue.is_empty());
    client.decoder.finish().unwrap();
}
#[test]
fn step5_wire_a4_actual_client_rejects_partial_eof_and_oversized_header() {
    let deadline = Instant::now() + Duration::from_secs(1);
    let (mut good, mut peer) = pair();
    let body = br#"{"protocol_version":1,"kind":"health"}"#;
    send(&mut peer, &frame(body), deadline);
    assert_eq!(collect(&mut good, 1, deadline)[0].body(), body);
    good.decoder.finish().unwrap();
    let (mut partial, mut peer) = pair();
    first(&mut partial, &mut peer, 0, deadline);
    peer.shutdown(Shutdown::Write).unwrap();
    assert!(partial.poll().is_err());
    for size in [65537u32, u32::MAX] {
        let (mut client, mut peer) = pair();
        send(&mut peer, &size.to_be_bytes(), deadline);
        assert!(client.poll().is_err());
    }
}
