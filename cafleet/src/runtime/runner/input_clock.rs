use super::*;
use std::time::{Duration, Instant};

pub(crate) struct InputClockSample {
    pub before: Instant,
    pub monotonic_ns: u64,
    pub after: Instant,
}
impl InputClockSample {
    fn validate(&self) -> Result<()> {
        ensure(self.before <= self.after, "input clock bracket is reversed")
    }
}

pub(crate) trait InputDeadlineClock {
    fn sample(&mut self) -> Result<InputClockSample>;
}

pub(super) struct RealInputDeadlineClock;
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
        let sample = InputClockSample {
            before,
            monotonic_ns,
            after,
        };
        sample.validate()?;
        Ok(sample)
    }
}

pub(crate) struct InputExpiration(u64);
impl InputExpiration {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed live input client captures the sender expiration"
        )
    )]
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

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed live input client copies the captured wire expiration"
        )
    )]
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
