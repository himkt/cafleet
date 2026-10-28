use super::*;
use std::time::Instant;

const ROUND_LIMIT: usize = 8;

struct DrainBudget {
    human: usize,
    child: usize,
    manual: usize,
    display: usize,
}

fn unsettled_health(health: &ObservationHealth) -> Option<String> {
    if health.degraded {
        Some(match &health.diagnostic {
            Some(code) => format!("observation degraded: {code}"),
            None => "observation degraded".into(),
        })
    } else if health.pending_count != 0 {
        Some("observation transitions pending".into())
    } else if health.pending_evidence {
        Some("observation evidence pending".into())
    } else {
        None
    }
}

fn before_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(CafleetError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

struct DeadlineObserver<'a, O> {
    inner: &'a mut O,
    deadline: Instant,
}
impl<O: ScreenObserver> ScreenObserver for DeadlineObserver<'_, O> {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()> {
        before_deadline(self.deadline)?;
        self.inner.observe(screen)?;
        before_deadline(self.deadline)
    }

    fn failures_are_fatal(&self) -> bool {
        true
    }

    fn parser_failure(&mut self) -> Result<()> {
        before_deadline(self.deadline)?;
        self.inner.parser_failure()?;
        before_deadline(self.deadline)
    }
}

impl Transport {
    pub(crate) fn quiesce_for_input(
        &mut self,
        human: &mut impl ByteInput,
        pty: &mut impl PtyIo,
        display: &mut impl Display,
        observer: &mut impl ScreenObserver,
        deadline: Instant,
    ) -> Result<InputDrain> {
        self.invalidate_input_drain();
        self.drain_blocker = None;
        before_deadline(deadline)?;
        let mut remaining = DrainBudget {
            human: self.limits.bytes_per_tick,
            child: self.limits.bytes_per_tick,
            manual: self.limits.bytes_per_tick,
            display: self.limits.bytes_per_tick,
        };
        for _ in 0..ROUND_LIMIT {
            before_deadline(deadline)?;
            let human_read = if self.outer_eof {
                AvailableRead::Skipped
            } else {
                read_classified(
                    human,
                    remaining
                        .human
                        .min(self.limits.input_bytes - self.input.len()),
                )
                .map_err(io_error)?
            };
            let human_quiet = matches!(human_read, AvailableRead::WouldBlock);
            let mut interrupted = matches!(human_read, AvailableRead::Interrupted);
            match human_read {
                AvailableRead::Data(bytes) => {
                    remaining.human -= bytes.len();
                    self.retain_human(bytes);
                }
                AvailableRead::Eof => self.outer_eof = true,
                _ => {}
            }
            before_deadline(deadline)?;
            if remaining.manual > 0 && !self.input.is_empty() {
                let written = write_pending(&mut self.input, remaining.manual, |bytes| {
                    pty.try_write(bytes)
                })?;
                remaining.manual -= written;
                before_deadline(deadline)?;
            }
            let child_read = self.read_drain_child(
                pty,
                remaining
                    .child
                    .min(self.limits.display_bytes - self.display.len()),
            )?;
            let child_quiet = matches!(child_read, AvailableRead::WouldBlock);
            interrupted |= matches!(child_read, AvailableRead::Interrupted);
            match child_read {
                AvailableRead::Data(bytes) => {
                    remaining.child -= bytes.len();
                    self.display.extend(bytes.iter().copied());
                    self.observe_drain_child(&bytes, observer, deadline)?;
                }
                AvailableRead::Eof => self.child_eof = true,
                _ => {}
            }
            before_deadline(deadline)?;
            if remaining.display > 0 && !self.display.is_empty() {
                let written = write_pending(&mut self.display, remaining.display, |bytes| {
                    display.try_write(bytes)
                })?;
                remaining.display -= written;
                before_deadline(deadline)?;
            }
            match display.try_flush() {
                Ok(()) => {}
                Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                Err(cause) => return Err(io_error(cause)),
            }
            before_deadline(deadline)?;
            let ended = if interrupted {
                Some("terminal input interrupted")
            } else if self.outer_eof {
                Some("outer terminal closed")
            } else if self.child_eof {
                Some("native output closed")
            } else {
                None
            };
            if let Some(reason) = ended {
                return Ok(self.pending(reason.into()));
            }
            if human_quiet
                && child_quiet
                && self.input.is_empty()
                && self.display.len() < self.limits.display_bytes
            {
                if !self.observation_complete {
                    return Ok(self.pending("observation incomplete".into()));
                }
                let health = observer.input_health();
                before_deadline(deadline)?;
                let Some(health) = health else {
                    return Ok(self.pending("observation health unavailable".into()));
                };
                if let Some(reason) = unsettled_health(&health) {
                    return Ok(self.pending(reason));
                }
                let output_seq = self.terminal.snapshot().screen_seq;
                before_deadline(deadline)?;
                self.input_drain = Some(DrainedInput {
                    instance: self.input_instance.clone(),
                    turn: self.input_turn,
                    human_generation: self.human_generation,
                    output_seq,
                    health,
                });
                return Ok(InputDrain::Quiescent {
                    human_generation: self.human_generation,
                    output_seq,
                });
            }
        }
        Ok(self.pending("terminal output still arriving".into()))
    }

    fn pending(&mut self, reason: String) -> InputDrain {
        self.drain_blocker = Some(reason);
        InputDrain::Pending
    }

    fn read_drain_child(&mut self, pty: &mut impl PtyIo, capacity: usize) -> Result<AvailableRead> {
        if capacity == 0 || self.child_eof {
            return Ok(AvailableRead::Skipped);
        }
        #[cfg(test)]
        if let Some(mut bytes) = self.deferred_child_bytes.take() {
            assert!(!bytes.is_empty(), "prefetch retains positive child bytes");
            let rest = bytes.split_off(bytes.len().min(capacity));
            if !rest.is_empty() {
                self.deferred_child_bytes = Some(rest);
            }
            return Ok(AvailableRead::Data(bytes));
        }
        read_classified(pty, capacity).map_err(io_error)
    }

    fn observe_drain_child(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
        deadline: Instant,
    ) -> Result<()> {
        let observation_complete_before = self.observation_complete;
        self.observation_complete = false;
        before_deadline(deadline)?;
        #[cfg(test)]
        if let Some(hooks) = &self.observation_hooks {
            hooks.before(self.observation_phase, ObservationOperation::Parser)?;
            before_deadline(deadline)?;
        }
        let mut bounded = DeadlineObserver {
            inner: observer,
            deadline,
        };
        match self.terminal.feed_observed_classified(bytes, &mut bounded) {
            Ok(()) => {
                self.observation_complete = observation_complete_before;
                before_deadline(deadline)
            }
            Err(TerminalObservationError::Observer(cause)) => Err(cause),
            Err(TerminalObservationError::Parser(cause)) => match bounded.parser_failure() {
                Ok(()) => Err(cause),
                Err(notification) => Err(CafleetError::WithDiagnostics {
                    cause: Box::new(cause),
                    diagnostics: vec![notification.message().into_owned()],
                }),
            },
        }
    }
}
