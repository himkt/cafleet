use super::*;
use std::collections::VecDeque;
use std::io::{self, ErrorKind};

pub(crate) trait ByteInput {
    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
}
pub(crate) trait PtyIo: ByteInput {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn resize(&mut self, size: TerminalSize) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
pub(crate) trait Display {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    fn try_flush(&mut self) -> io::Result<()>;
}
pub(crate) trait ScreenObserver {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<()>;
}
#[derive(Default)]
pub(crate) struct TransportProgress {
    pub child_eof: bool,
    pub outer_eof: bool,
    pub child_bytes: usize,
    pub human_bytes: usize,
    pub display_bytes: usize,
    pub observation_error: Option<String>,
}
pub(crate) struct TransportLimits {
    pub input_bytes: usize,
    pub display_bytes: usize,
    pub bytes_per_tick: usize,
}
pub(crate) struct Transport {
    terminal: TerminalState,
    limits: TransportLimits,
    input: VecDeque<u8>,
    display: VecDeque<u8>,
    outer_eof: bool,
    child_eof: bool,
}
fn read_available(input: &mut impl ByteInput, capacity: usize) -> io::Result<Option<Vec<u8>>> {
    if capacity == 0 {
        return Ok(None);
    }
    let mut bytes = vec![0; capacity.min(16384)];
    match input.try_read(&mut bytes) {
        Ok(count) => {
            assert!(count <= bytes.len(), "reader respects supplied capacity");
            bytes.truncate(count);
            Ok(Some(bytes))
        }
        Err(cause) if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
            Ok(None)
        }
        Err(cause) => Err(cause),
    }
}
fn write_pending(
    queue: &mut VecDeque<u8>,
    limit: usize,
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
) -> Result<usize> {
    if queue.is_empty() {
        return Ok(0);
    }
    let bytes = queue.make_contiguous();
    let count = match write(&bytes[..bytes.len().min(limit)]) {
        Ok(0) => return Err(error("terminal write made no progress")),
        Ok(count) => count,
        Err(cause) if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => 0,
        Err(cause) => return Err(io_error(cause)),
    };
    ensure(
        count <= bytes.len().min(limit),
        "terminal writer exceeded supplied bytes",
    )?;
    queue.drain(..count);
    Ok(count)
}
impl Transport {
    pub(crate) fn new(size: TerminalSize, limits: TransportLimits) -> Result<Self> {
        ensure(
            limits.input_bytes > 0 && limits.display_bytes > 0 && limits.bytes_per_tick > 0,
            "transport limits must be positive",
        )?;
        Ok(Self {
            terminal: TerminalState::new(size)?,
            limits,
            input: VecDeque::new(),
            display: VecDeque::new(),
            outer_eof: false,
            child_eof: false,
        })
    }
    pub(crate) fn tick(
        &mut self,
        human: &mut impl ByteInput,
        pty: &mut impl PtyIo,
        display: &mut impl Display,
        observer: &mut impl ScreenObserver,
    ) -> Result<TransportProgress> {
        let mut progress = TransportProgress::default();
        if !self.outer_eof
            && let Some(bytes) = read_available(
                human,
                (self.limits.input_bytes - self.input.len()).min(self.limits.bytes_per_tick),
            )
            .map_err(io_error)?
        {
            self.outer_eof = bytes.is_empty();
            progress.human_bytes = bytes.len();
            self.input.extend(bytes);
        }
        write_pending(&mut self.input, self.limits.bytes_per_tick, |bytes| {
            pty.try_write(bytes)
        })?;
        if !self.child_eof
            && let Some(bytes) = read_available(
                pty,
                (self.limits.display_bytes - self.display.len()).min(self.limits.bytes_per_tick),
            )
            .map_err(io_error)?
        {
            self.child_eof = bytes.is_empty();
            progress.child_bytes = bytes.len();
            if let Err(cause) = self.terminal.feed_observed(&bytes, observer) {
                progress.observation_error = Some(cause.message().into_owned());
            }
            self.display.extend(bytes);
        }
        progress.display_bytes = self.flush_display(display)?;
        progress.outer_eof = self.outer_eof;
        progress.child_eof = self.child_eof;
        Ok(progress)
    }
    pub(crate) fn flush_display(&mut self, display: &mut impl Display) -> Result<usize> {
        let count = write_pending(&mut self.display, self.limits.bytes_per_tick, |bytes| {
            display.try_write(bytes)
        })?;
        match display.try_flush() {
            Ok(()) => {}
            Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
            Err(cause) => return Err(io_error(cause)),
        }
        Ok(count)
    }
    pub(crate) fn resize(&mut self, pty: &mut impl PtyIo, size: TerminalSize) -> Result<()> {
        size.validate()?;
        pty.resize(size)?;
        self.terminal.resize(size)
    }
    pub(crate) fn snapshot(&self) -> ScreenSnapshot {
        self.terminal.snapshot()
    }
    pub(crate) fn pending_input_bytes(&self) -> usize {
        self.input.len()
    }
    pub(crate) fn pending_display_bytes(&self) -> usize {
        self.display.len()
    }
    pub(crate) fn discard_display_tail(&mut self) -> usize {
        let count = self.display.len();
        self.display.clear();
        count
    }
}
