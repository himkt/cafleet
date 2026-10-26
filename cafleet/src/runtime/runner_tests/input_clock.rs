use crate::broker::runtime::InputKind;
use crate::error::CafleetError;
use crate::runtime::runner::{
    InputClockSample, InputDeadlineClock, InputExpiration, PreparedSubmission, SubmitInput,
};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const EXPIRATION: u64 = 10_000_000_000;

struct ScriptClock {
    samples: VecDeque<Result<InputClockSample, CafleetError>>,
    calls: usize,
}
impl ScriptClock {
    fn new(samples: Vec<Result<InputClockSample, CafleetError>>) -> Self {
        Self {
            samples: samples.into(),
            calls: 0,
        }
    }
    fn one(sample: InputClockSample) -> Self {
        Self::new(vec![Ok(sample)])
    }
    fn consumed(&self, calls: usize, remaining: usize) {
        assert_eq!(self.calls, calls);
        assert_eq!(self.samples.len(), remaining);
    }
}
impl InputDeadlineClock for ScriptClock {
    fn sample(&mut self) -> Result<InputClockSample, CafleetError> {
        self.calls += 1;
        self.samples
            .pop_front()
            .expect("the operation consumes only its specified paired clock samples")
    }
}

fn anchor() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(60))
        .expect("the platform represents the fixed future clock anchor")
}
fn ms(origin: Instant, millis: u64) -> Instant {
    origin
        .checked_add(Duration::from_millis(millis))
        .expect("the platform represents the fixed millisecond fixture offset")
}
fn ns(origin: Instant, nanos: u64) -> Instant {
    origin
        .checked_add(Duration::from_nanos(nanos))
        .expect("the platform represents the fixed nanosecond fixture offset")
}
fn sample(before: Instant, monotonic_ns: u64, after: Instant) -> InputClockSample {
    InputClockSample {
        before,
        monotonic_ns,
        after,
    }
}
fn unavailable() -> CafleetError {
    CafleetError::App("fixture clock unavailable".into())
}
fn captured(deadline: Instant, clock: &mut ScriptClock, expected: u64) -> InputExpiration {
    let expiration = InputExpiration::capture(deadline, clock).unwrap();
    assert_eq!(expiration.monotonic_ns(), expected);
    assert_eq!(expiration.monotonic_ns(), expected);
    clock.consumed(1, 0);
    expiration
}
fn expiration(origin: Instant) -> InputExpiration {
    captured(
        ms(origin, 1000),
        &mut ScriptClock::one(sample(origin, 9_000_000_000, origin)),
        EXPIRATION,
    )
}
fn one_failure<T>(result: Result<T, CafleetError>, clock: &ScriptClock, remaining: usize) {
    assert!(
        result.is_err(),
        "the invalid clock operation returns an error"
    );
    clock.consumed(1, remaining);
}
fn received(
    expiration: &InputExpiration,
    admitted_at: Instant,
    shutdown: Option<Instant>,
    clock: &mut ScriptClock,
    expected: Instant,
) {
    assert_eq!(
        expiration.receive(admitted_at, shutdown, clock).unwrap(),
        expected
    );
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    clock.consumed(1, 0);
}
fn request() -> SubmitInput {
    SubmitInput {
        operation_key: format!("prompt:1:{}", "a".repeat(64)),
        input_kind: InputKind::Prompt,
        message_id: None,
        literal: "literal clock fixture".into(),
    }
}

#[test]
fn step5_b0_clock_capture_uses_later_instant_remaining_without_bracket_extension() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 30)));
    captured(ms(t, 100), &mut clock, 1_070_000_000);
}

#[test]
fn step5_b0_clock_capture_equal_bracket_preserves_one_nanosecond() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 10)));
    captured(ns(ms(t, 10), 1), &mut clock, 1_000_000_001);
}

#[test]
fn step5_b0_clock_capture_monotonic_origin_yields_positive_expiration() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(t, 0, t));
    captured(ns(t, 1), &mut clock, 1);
}

#[test]
fn step5_b0_clock_capture_accepts_exact_unsigned_expiration_upper_bound() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
    captured(ns(t, 5), &mut clock, u64::MAX);
}

#[test]
fn step5_b0_clock_capture_rejects_unsigned_expiration_overflow() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
    one_failure(InputExpiration::capture(ns(t, 6), &mut clock), &clock, 0);
}

#[test]
fn step5_b0_clock_capture_rejects_equal_or_expired_deadline_without_resampling() {
    for after_ms in [100, 101] {
        let t = anchor();
        let mut clock = ScriptClock::new(vec![
            Ok(sample(ms(t, 90), 1_000_000_000, ms(t, after_ms))),
            Ok(sample(t, 1_000_000_000, t)),
        ]);
        one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 1);
    }
}

#[test]
fn step5_b0_clock_capture_rejects_reversed_instant_bracket() {
    let t = anchor();
    let mut clock = ScriptClock::one(sample(ms(t, 30), 1_000_000_000, ms(t, 10)));
    one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 0);
}

#[test]
fn step5_b0_clock_capture_source_failure_consumes_one_sample_without_fallback() {
    let t = anchor();
    let mut clock = ScriptClock::new(vec![Err(unavailable()), Ok(sample(t, 1_000_000_000, t))]);
    one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 1);
}

#[test]
fn step5_b0_clock_receive_uses_earlier_instant_for_conservative_deadline() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
    received(&expiration, t, None, &mut clock, ms(t, 3000));
}

#[test]
fn step5_b0_clock_receive_caps_expiration_at_original_five_second_admission() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 9_000_000_000, ms(t, 1100)));
    received(&expiration, t, None, &mut clock, ms(t, 5000));
}

#[test]
fn step5_b0_clock_receive_earlier_shutdown_beats_admission_and_expiration() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 9_000_000_000, ms(t, 1100)));
    received(&expiration, t, Some(ms(t, 4000)), &mut clock, ms(t, 4000));
}

#[test]
fn step5_b0_clock_receive_later_caps_preserve_shorter_expiration() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
    received(&expiration, t, Some(ms(t, 9000)), &mut clock, ms(t, 3000));
}

#[test]
fn step5_b0_clock_receive_rejects_equal_or_past_monotonic_expiration() {
    for monotonic_ns in [EXPIRATION, EXPIRATION + 1] {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::one(sample(ms(t, 1000), monotonic_ns, ms(t, 1100)));
        one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
        assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    }
}

#[test]
fn step5_b0_clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline() {
    for after_ms in [3000, 4000] {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::one(sample(
            ms(t, 1000),
            EXPIRATION - 2_000_000_000,
            ms(t, after_ms),
        ));
        one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
        assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    }
}

#[test]
fn step5_b0_clock_receive_rejects_already_expired_original_admission_cap() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 6000), EXPIRATION - 2_000_000_000, ms(t, 6100)));
    one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
}

#[test]
fn step5_b0_clock_receive_rejects_expired_shutdown_despite_positive_expiration() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
    one_failure(
        expiration.receive(t, Some(ms(t, 500)), &mut clock),
        &clock,
        0,
    );
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
}

#[test]
fn step5_b0_clock_receive_rejects_reversed_instant_bracket() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 2000), EXPIRATION - 2_000_000_000, ms(t, 1000)));
    one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
}

#[test]
fn step5_b0_clock_receive_source_failure_consumes_one_sample_without_fallback() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::new(vec![
        Err(unavailable()),
        Ok(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100))),
    ]);
    one_failure(expiration.receive(t, None, &mut clock), &clock, 1);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
}

#[test]
fn step5_b0_clock_receive_reuses_immutable_expiration_without_phase_extension() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::new(vec![
        Ok(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100))),
        Ok(sample(ms(t, 2000), EXPIRATION - 1_000_000_000, ms(t, 2100))),
        Ok(sample(ms(t, 3000), EXPIRATION, ms(t, 3100))),
    ]);
    assert_eq!(
        expiration.receive(t, None, &mut clock).unwrap(),
        ms(t, 3000)
    );
    clock.consumed(1, 2);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    assert_eq!(
        expiration.receive(t, None, &mut clock).unwrap(),
        ms(t, 3000)
    );
    clock.consumed(2, 1);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    assert!(expiration.receive(t, None, &mut clock).is_err());
    clock.consumed(3, 0);
    assert_eq!(expiration.monotonic_ns(), EXPIRATION);
}

#[test]
fn step5_b0_clock_receive_preserves_positive_single_nanosecond() {
    let t = anchor();
    let expiration = expiration(t);
    let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 1, ms(t, 1000)));
    received(&expiration, t, None, &mut clock, ns(ms(t, 1000), 1));
}

#[test]
fn step5_b0_clock_prepared_submission_captures_one_paired_sample() {
    let t = anchor();
    let request = request();
    let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 30)));
    let _prepared = PreparedSubmission::new(&request, ms(t, 100), &mut clock).unwrap();
    clock.consumed(1, 0);
}

#[test]
fn step5_b0_clock_prepared_submission_source_failure_preserves_unused_later_sample() {
    let t = anchor();
    let request = request();
    let mut clock = ScriptClock::new(vec![Err(unavailable()), Ok(sample(t, 1_000_000_000, t))]);
    one_failure(
        PreparedSubmission::new(&request, ms(t, 100), &mut clock),
        &clock,
        1,
    );
}

#[test]
fn step5_b0_clock_prepared_submission_expiry_rejects_without_resampling_new_budget() {
    let t = anchor();
    let request = request();
    let mut clock = ScriptClock::new(vec![
        Ok(sample(ms(t, 90), 1_000_000_000, ms(t, 100))),
        Ok(sample(t, 1_000_000_000, t)),
    ]);
    one_failure(
        PreparedSubmission::new(&request, ms(t, 100), &mut clock),
        &clock,
        1,
    );
}
