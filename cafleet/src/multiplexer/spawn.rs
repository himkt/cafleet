//! Per-creation monotonic budget and precise subprocess execution.
use super::{AnyMultiplexer, Multiplexer, MultiplexerContext, MultiplexerError, RunError};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(crate) trait MonotonicClock {
    fn now(&self) -> Instant;
}
pub(crate) struct SystemClock;
impl MonotonicClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
pub(crate) trait TimedCommandRunner {
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError>;
}
pub(crate) struct Deadline {
    end: Instant,
}
impl Deadline {
    pub(crate) fn after(clock: &dyn MonotonicClock, budget: Duration) -> Self {
        Self {
            end: clock.now() + budget,
        }
    }
    pub(crate) fn remaining(&self, clock: &dyn MonotonicClock) -> Result<Duration, RunError> {
        let remaining = self.end.saturating_duration_since(clock.now());
        if remaining.is_zero() {
            Err(RunError::Timeout)
        } else {
            Ok(remaining)
        }
    }
}
pub(crate) struct SpawnExecution<'a> {
    pub clock: &'a dyn MonotonicClock,
    pub runner: &'a dyn TimedCommandRunner,
}
impl SpawnExecution<'_> {
    pub(crate) fn run(
        &self,
        backend: &str,
        argv: &[String],
        deadline: &Deadline,
    ) -> Result<String, MultiplexerError> {
        self.run_tolerating(backend, argv, deadline, &|_| false)
    }
    pub(crate) fn run_tolerating(
        &self,
        backend: &str,
        argv: &[String],
        deadline: &Deadline,
        missing: &dyn Fn(&str) -> bool,
    ) -> Result<String, MultiplexerError> {
        let joined = argv.join(" ");
        let remaining = deadline.remaining(self.clock).map_err(|_| {
            MultiplexerError::new(format!("{backend} spawn deadline exceeded: {joined}"))
                .with_timeout()
        })?;
        self.runner
            .run_for(argv, remaining)
            .or_else(|error| match error {
                RunError::Failed { ref stderr } if missing(stderr) => Ok(String::new()),
                other => Err(other),
            })
            .map_err(|error| match error {
                RunError::BinaryNotFound(detail) => {
                    MultiplexerError::new(format!("{backend} binary not found: {detail}"))
                }
                RunError::Failed { stderr } => MultiplexerError::new(format!(
                    "{backend} command failed: {joined}\nstderr: {}",
                    stderr.trim()
                )),
                RunError::Timeout => MultiplexerError::new(format!(
                    "{backend} command timed out after {}s: {joined}",
                    duration_seconds(remaining)
                ))
                .with_timeout(),
            })
    }
    pub(crate) fn check_transfer(
        &self,
        backend: &str,
        deadline: &Deadline,
    ) -> Result<(), MultiplexerError> {
        deadline.remaining(self.clock).map(|_| ()).map_err(|_| {
            MultiplexerError::new(format!("{backend} spawn deadline exceeded")).with_timeout()
        })
    }
}
fn duration_seconds(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        return duration.as_secs().to_string();
    }
    format!("{}.{:09}", duration.as_secs(), duration.subsec_nanos())
        .trim_end_matches('0')
        .to_string()
}
pub(crate) struct PaneSpawnRequest<'a> {
    pub reference: &'a MultiplexerContext,
    pub env: &'a [(String, String)],
    pub command: &'a [String],
    pub cwd: Option<&'a Path>,
}
pub(crate) trait SpawnMultiplexer: Multiplexer {
    fn split_prepared(
        &self,
        request: &PaneSpawnRequest<'_>,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<String, MultiplexerError>;
    fn kill_pane_with_deadline(
        &self,
        pane_id: &str,
        ignore_missing: bool,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<(), MultiplexerError>;
}
impl SpawnMultiplexer for AnyMultiplexer {
    fn split_prepared(
        &self,
        request: &PaneSpawnRequest<'_>,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<String, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.split_prepared(request, deadline, execution),
            Self::Herdr(mux) => mux.split_prepared(request, deadline, execution),
        }
    }
    fn kill_pane_with_deadline(
        &self,
        pane_id: &str,
        ignore_missing: bool,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => {
                mux.kill_pane_with_deadline(pane_id, ignore_missing, deadline, execution)
            }
            Self::Herdr(mux) => {
                mux.kill_pane_with_deadline(pane_id, ignore_missing, deadline, execution)
            }
        }
    }
}
// Both permanent legacy and bounded entry points use the same spawn algorithm.
pub(crate) type SpawnRun<'a> = dyn Fn(&[String]) -> Result<String, MultiplexerError> + 'a;
pub(crate) struct SpawnOps<'a> {
    pub run: &'a SpawnRun<'a>,
    pub kill: &'a dyn Fn(&str) -> Result<(), MultiplexerError>,
    pub transfer: &'a dyn Fn() -> Result<(), MultiplexerError>,
}
