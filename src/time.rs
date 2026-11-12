use crate::error::CafleetError;
use chrono::DateTime;
use chrono::Utc;
use nix::sys::signal::SigSet;
use nix::sys::signal::SigmaskHow;
use nix::sys::signal::Signal;
use nix::sys::signal::pthread_sigmask;
use std::time::Duration;
use std::time::Instant;

pub(crate) fn now_stamp() -> String {
    format_utc(Utc::now())
}

/// Emit the canonical storage form: fixed-width 6-digit microseconds and the
/// literal `+00:00` offset, so rows sort lexicographically.
pub(crate) fn format_utc(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}

/// Parses a stored canonical timestamp.
pub(crate) fn parse_lenient(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Blocks SIGINT, SIGTERM and SIGHUP for the calling thread until dropped;
/// a signal received meanwhile takes its default action when the previous mask is restored.
pub(crate) struct DeferTermination {
    previous: SigSet,
}

impl DeferTermination {
    pub(crate) fn begin() -> Result<Self, CafleetError> {
        let mut deferred = SigSet::empty();
        for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
            deferred.add(signal);
        }
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&deferred), Some(&mut previous)).map_err(
            |error| CafleetError::App(format!("cannot defer termination signals: {error}")),
        )?;
        Ok(Self { previous })
    }
}

impl Drop for DeferTermination {
    fn drop(&mut self) {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.previous), None)
            .expect("restoring a mask previously returned by pthread_sigmask succeeds");
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline(Instant);

impl Deadline {
    pub(crate) fn instant(self) -> Instant {
        self.0
    }
    pub(crate) fn inherited_or_new() -> Self {
        crate::db::inherited_deadline().unwrap_or_else(Deadline::new)
    }
    pub(crate) fn new() -> Self {
        Self::until(Instant::now() + Duration::from_secs(5))
    }

    pub(crate) fn until(instant: Instant) -> Self {
        Self(instant)
    }

    pub(crate) fn remaining(self) -> Result<Duration, CafleetError> {
        self.remaining_at(Instant::now())
    }

    pub(crate) fn remaining_at(self, now: Instant) -> Result<Duration, CafleetError> {
        self.0
            .checked_duration_since(now)
            .filter(|left| !left.is_zero())
            .ok_or(CafleetError::DeadlineExceeded)
    }

    /// The deadline `reserve` earlier; already expired when less than `reserve` remains.
    pub(crate) fn reserving(self, reserve: Duration) -> Self {
        Self(
            self.0
                .checked_sub(reserve)
                .expect("a monotonic deadline lies far past the clock origin"),
        )
    }
}

pub(crate) trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::time::Clock;
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::time::Duration;
    use std::time::Instant;

    pub(crate) struct TestClock {
        origin: Instant,
        pub(crate) elapsed: Cell<Duration>,
        pub(crate) sleeps: RefCell<Vec<Duration>>,
    }

    impl TestClock {
        pub(crate) fn new() -> Self {
            Self::starting_at(Instant::now())
        }

        pub(crate) fn starting_at(origin: Instant) -> Self {
            Self {
                origin,
                elapsed: Cell::new(Duration::ZERO),
                sleeps: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn advance(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.origin + self.elapsed.get()
        }

        fn sleep(&self, duration: Duration) {
            assert!(!duration.is_zero(), "idle checks advance the clock");
            self.sleeps.borrow_mut().push(duration);
            self.advance(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::time::*;
    use chrono::Duration;
    use chrono::TimeZone;

    #[test]
    fn format_emits_six_digit_microseconds_and_offset() {
        let second = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
        for (micros, expected) in [
            (198_561, "2026-07-30T09:53:43.198561+00:00"),
            (0, "2026-07-30T09:53:43.000000+00:00"),
            (7, "2026-07-30T09:53:43.000007+00:00"),
        ] {
            assert_eq!(
                format_utc(second + Duration::microseconds(micros)),
                expected
            );
        }
    }

    #[test]
    fn format_round_trips_through_parse() {
        let dt =
            Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap() + Duration::microseconds(198_561);
        assert_eq!(parse_lenient(&format_utc(dt)).unwrap(), dt);
    }

    #[test]
    fn parse_accepts_missing_fractional_part() {
        let expected = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
        assert_eq!(
            parse_lenient("2026-07-30T09:53:43+00:00").unwrap(),
            expected
        );
    }

    #[test]
    fn parse_accepts_z_offset_spelling() {
        let expected = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
        assert_eq!(parse_lenient("2026-07-30T09:53:43Z").unwrap(), expected);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_lenient("yesterday at noon").is_none());
        assert!(parse_lenient("").is_none());
    }

    #[test]
    fn formatted_timestamps_order_lexicographically() {
        let base = Utc.with_ymd_and_hms(2026, 7, 30, 9, 53, 43).unwrap();
        let earlier = format_utc(base + Duration::microseconds(99_999));
        let later = format_utc(base + Duration::microseconds(100_000));
        assert!(earlier < later);
        let end_of_second = format_utc(base + Duration::microseconds(999_999));
        let next_second = format_utc(base + Duration::seconds(1));
        assert!(end_of_second < next_second);
    }
    use crate::time::DeferTermination;
    use nix::sys::signal::SigSet;
    use nix::sys::signal::SigmaskHow;
    use nix::sys::signal::Signal;
    use nix::sys::signal::pthread_sigmask;

    const DEFERRED: [Signal; 3] = [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP];

    fn blocked() -> SigSet {
        SigSet::thread_get_mask().unwrap()
    }

    fn defers_all(mask: &SigSet) -> bool {
        DEFERRED.iter().all(|signal| mask.contains(*signal))
    }

    fn defers_none(mask: &SigSet) -> bool {
        DEFERRED.iter().all(|signal| !mask.contains(*signal))
    }

    fn on_fresh_thread(test: impl FnOnce() + Send + 'static) {
        std::thread::spawn(move || {
            pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None).unwrap();
            test();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_span_blocks_sigint_sigterm_and_sighup_until_dropped() {
        on_fresh_thread(|| {
            let span = DeferTermination::begin().unwrap();
            assert!(defers_all(&blocked()));
            drop(span);
            assert!(defers_none(&blocked()));
        });
    }

    #[test]
    fn a_span_restores_signals_blocked_before_it_began() {
        on_fresh_thread(|| {
            let mut previous = SigSet::empty();
            previous.add(Signal::SIGUSR1);
            previous.add(Signal::SIGTERM);
            pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&previous), None).unwrap();
            drop(DeferTermination::begin().unwrap());
            let mask = blocked();
            assert!(mask.contains(Signal::SIGUSR1));
            assert!(
                mask.contains(Signal::SIGTERM),
                "a signal the caller had blocked stays blocked"
            );
            assert!(!mask.contains(Signal::SIGINT));
            assert!(!mask.contains(Signal::SIGHUP));
        });
    }

    #[test]
    fn a_helper_thread_started_inside_a_span_inherits_the_blocked_set() {
        on_fresh_thread(|| {
            let _span = DeferTermination::begin().unwrap();
            let inherited = std::thread::spawn(blocked).join().unwrap();
            assert!(defers_all(&inherited));
        });
    }

    #[test]
    fn unwinding_restores_the_callers_signal_mask() {
        on_fresh_thread(|| {
            let previous = blocked();
            assert!(
                std::panic::catch_unwind(|| {
                    let _span = DeferTermination::begin().unwrap();
                    assert!(defers_all(&blocked()));
                    panic!("unwind protected work");
                })
                .is_err()
            );
            assert_eq!(blocked(), previous);
        });
    }
    mod deadline_reservation {
        use crate::time::Deadline;
        use std::time::Duration;
        use std::time::Instant;
        const TEST_WORK_RESERVE: Duration = Duration::from_millis(3500);
        #[test]
        fn reserving_moves_the_deadline_earlier_by_the_reserve() {
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(5));
            let left = deadline.reserving(TEST_WORK_RESERVE).remaining().unwrap();
            assert!(left <= Duration::from_millis(1500), "{left:?}");
            assert!(left > Duration::from_millis(1400), "{left:?}");
        }
        #[test]
        fn reserving_more_than_remains_is_already_expired() {
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(3));
            assert!(deadline.reserving(TEST_WORK_RESERVE).remaining().is_err());
        }
    }
}
