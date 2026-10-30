use super::*;
use std::time::{Duration, Instant};

pub(crate) struct InputClockSample {
    pub(crate) before: Instant,
    pub(crate) monotonic_ns: u64,
    pub(crate) after: Instant,
}
impl InputClockSample {
    fn validate(&self) -> Result<()> {
        ensure(self.before <= self.after, "input clock bracket is reversed")
    }
}

pub(crate) trait InputDeadlineClock {
    fn sample(&mut self) -> Result<InputClockSample>;
}

pub(crate) struct RealInputDeadlineClock;
impl InputDeadlineClock for RealInputDeadlineClock {
    fn sample(&mut self) -> Result<InputClockSample> {
        let before = Instant::now();
        let time = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Known(
            rustix::time::ClockId::Monotonic,
        ))
        .map_err(io_error)?;
        let after = Instant::now();
        let seconds = u64::try_from(time.tv_sec)
            .map_err(|_| error("monotonic clock seconds must be nonnegative"))?;
        let nanoseconds = u64::try_from(time.tv_nsec)
            .map_err(|_| error("monotonic clock nanoseconds must be nonnegative"))?;
        ensure(
            nanoseconds < 1_000_000_000,
            "monotonic clock nanoseconds exceed their range",
        )?;
        let monotonic_ns = seconds
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_add(nanoseconds))
            .ok_or_else(|| error("monotonic clock nanoseconds overflow"))?;
        Ok(InputClockSample {
            before,
            monotonic_ns,
            after,
        })
    }
}

pub(crate) struct InputExpiration(u64);
impl InputExpiration {
    pub(crate) fn capture(deadline: Instant, clock: &mut impl InputDeadlineClock) -> Result<Self> {
        let sample = clock.sample()?;
        sample.validate()?;
        let remaining = deadline
            .checked_duration_since(sample.after)
            .filter(|duration| !duration.is_zero())
            .ok_or(CafleetError::DeadlineExceeded)?;
        let remaining_ns = u64::try_from(remaining.as_nanos())
            .map_err(|_| error("input deadline duration exceeds monotonic nanoseconds"))?;
        let expiration = sample
            .monotonic_ns
            .checked_add(remaining_ns)
            .ok_or_else(|| error("input expiration overflows monotonic nanoseconds"))?;
        Ok(Self(expiration))
    }

    pub(crate) fn receive(
        &self,
        admitted_at: Instant,
        shutdown: Option<Instant>,
        clock: &mut impl InputDeadlineClock,
    ) -> Result<Instant> {
        let sample = clock.sample()?;
        sample.validate()?;
        let remaining_ns = self
            .0
            .checked_sub(sample.monotonic_ns)
            .filter(|remaining| *remaining > 0)
            .ok_or(CafleetError::DeadlineExceeded)?;
        let expiration = sample
            .before
            .checked_add(Duration::from_nanos(remaining_ns))
            .ok_or_else(|| error("input expiration exceeds the local clock range"))?;
        let admission = admitted_at
            .checked_add(Duration::from_secs(5))
            .ok_or_else(|| error("input admission exceeds the local clock range"))?;
        let mut deadline = expiration.min(admission);
        if let Some(shutdown) = shutdown {
            deadline = deadline.min(shutdown);
        }
        check_deadline(sample.after, deadline)?;
        Ok(deadline)
    }

    pub(crate) fn monotonic_ns(&self) -> u64 {
        self.0
    }
}

pub(super) fn receive_input_expiration(
    expires_at_mono_ns: u64,
    admitted_at: Instant,
    shutdown: Option<Instant>,
    clock: &mut impl InputDeadlineClock,
) -> Result<Instant> {
    ensure(expires_at_mono_ns > 0, "positive input expiration required")?;
    InputExpiration(expires_at_mono_ns).receive(admitted_at, shutdown, clock)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::VecDeque;

    pub(crate) struct ScriptClock {
        pub(crate) samples: VecDeque<Result<InputClockSample>>,
        pub(crate) calls: usize,
    }
    impl ScriptClock {
        pub(crate) fn new(samples: Vec<Result<InputClockSample>>) -> Self {
            Self {
                samples: samples.into(),
                calls: 0,
            }
        }
        pub(crate) fn one(sample: InputClockSample) -> Self {
            Self::new(vec![Ok(sample)])
        }
        pub(crate) fn consumed(&self, calls: usize, remaining: usize) {
            assert_eq!(self.calls, calls);
            assert_eq!(self.samples.len(), remaining);
        }
    }
    impl InputDeadlineClock for ScriptClock {
        fn sample(&mut self) -> Result<InputClockSample> {
            self.calls += 1;
            self.samples
                .pop_front()
                .expect("the operation consumes only its specified paired clock samples")
        }
    }
    pub(crate) fn anchor() -> Instant {
        Instant::now()
            .checked_add(Duration::from_secs(60))
            .expect("the platform represents the fixed future clock anchor")
    }
    pub(crate) fn ms(origin: Instant, millis: u64) -> Instant {
        origin
            .checked_add(Duration::from_millis(millis))
            .expect("the platform represents the fixed millisecond fixture offset")
    }
    pub(crate) fn sample(before: Instant, monotonic_ns: u64, after: Instant) -> InputClockSample {
        InputClockSample {
            before,
            monotonic_ns,
            after,
        }
    }
    pub(crate) fn unavailable() -> CafleetError {
        CafleetError::App("fixture clock unavailable".into())
    }
    pub(crate) fn one_failure<T>(result: Result<T>, clock: &ScriptClock, remaining: usize) {
        assert!(
            result.is_err(),
            "the invalid clock operation returns an error"
        );
        clock.consumed(1, remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use crate::runtime::runner::InputExpiration;
    use std::time::{Duration, Instant};

    const EXPIRATION: u64 = 10_000_000_000;

    fn ns(origin: Instant, nanos: u64) -> Instant {
        origin
            .checked_add(Duration::from_nanos(nanos))
            .expect("the platform represents the fixed nanosecond fixture offset")
    }
    fn captured(deadline: Instant, clock: &mut ScriptClock, expected: u64) -> InputExpiration {
        let expiration = InputExpiration::capture(deadline, clock).unwrap();
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
        clock.consumed(1, 0);
    }

    #[test]
    fn clock_capture_uses_later_instant_remaining_without_bracket_extension() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 30)));
        captured(ms(t, 100), &mut clock, 1_070_000_000);
    }

    #[test]
    fn clock_capture_equal_bracket_preserves_one_nanosecond() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 10)));
        captured(ns(ms(t, 10), 1), &mut clock, 1_000_000_001);
    }

    #[test]
    fn clock_capture_monotonic_origin_yields_positive_expiration() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(t, 0, t));
        captured(ns(t, 1), &mut clock, 1);
    }

    #[test]
    fn clock_capture_accepts_exact_unsigned_expiration_upper_bound() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
        captured(ns(t, 5), &mut clock, u64::MAX);
    }

    #[test]
    fn clock_capture_rejects_unsigned_expiration_overflow() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
        one_failure(InputExpiration::capture(ns(t, 6), &mut clock), &clock, 0);
    }

    #[test]
    fn clock_capture_rejects_equal_or_expired_deadline_without_resampling() {
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
    fn clock_capture_rejects_reversed_instant_bracket() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 30), 1_000_000_000, ms(t, 10)));
        one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 0);
    }

    #[test]
    fn clock_capture_source_failure_consumes_one_sample_without_fallback() {
        let t = anchor();
        let mut clock = ScriptClock::new(vec![Err(unavailable()), Ok(sample(t, 1_000_000_000, t))]);
        one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 1);
    }

    #[test]
    fn clock_receive_uses_earlier_instant_for_conservative_deadline() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
        received(&expiration, t, None, &mut clock, ms(t, 3000));
    }

    #[test]
    fn clock_receive_caps_expiration_at_original_five_second_admission() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 9_000_000_000, ms(t, 1100)));
        received(&expiration, t, None, &mut clock, ms(t, 5000));
    }

    #[test]
    fn clock_receive_earlier_shutdown_beats_admission_and_expiration() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 9_000_000_000, ms(t, 1100)));
        received(&expiration, t, Some(ms(t, 4000)), &mut clock, ms(t, 4000));
    }

    #[test]
    fn clock_receive_later_caps_preserve_shorter_expiration() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
        received(&expiration, t, Some(ms(t, 9000)), &mut clock, ms(t, 3000));
    }

    #[test]
    fn clock_receive_rejects_equal_or_past_monotonic_expiration() {
        for monotonic_ns in [EXPIRATION, EXPIRATION + 1] {
            let t = anchor();
            let expiration = expiration(t);
            let mut clock = ScriptClock::one(sample(ms(t, 1000), monotonic_ns, ms(t, 1100)));
            one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
        }
    }

    #[test]
    fn clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline() {
        for after_ms in [3000, 4000] {
            let t = anchor();
            let expiration = expiration(t);
            let mut clock = ScriptClock::one(sample(
                ms(t, 1000),
                EXPIRATION - 2_000_000_000,
                ms(t, after_ms),
            ));
            one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
        }
    }

    #[test]
    fn clock_receive_rejects_already_expired_original_admission_cap() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 6000), EXPIRATION - 2_000_000_000, ms(t, 6100)));
        one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
    }

    #[test]
    fn clock_receive_rejects_expired_shutdown_despite_positive_expiration() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
        one_failure(
            expiration.receive(t, Some(ms(t, 500)), &mut clock),
            &clock,
            0,
        );
    }

    #[test]
    fn clock_receive_rejects_reversed_instant_bracket() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 2000), EXPIRATION - 2_000_000_000, ms(t, 1000)));
        one_failure(expiration.receive(t, None, &mut clock), &clock, 0);
    }

    #[test]
    fn clock_receive_source_failure_consumes_one_sample_without_fallback() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::new(vec![
            Err(unavailable()),
            Ok(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100))),
        ]);
        one_failure(expiration.receive(t, None, &mut clock), &clock, 1);
    }

    #[test]
    fn clock_receive_reuses_immutable_expiration_without_phase_extension() {
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
    fn clock_receive_preserves_positive_single_nanosecond() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 1, ms(t, 1000)));
        received(&expiration, t, None, &mut clock, ns(ms(t, 1000), 1));
    }
}
