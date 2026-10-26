use super::*;
use nix::sys::termios::{self, SetArg, Termios};
use std::fs::File;

#[derive(Default)]
struct Callbacks {
    title: String,
}
impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ScreenSnapshot {
    pub contents: String,
    pub title: String,
    pub rows: u16,
    pub cols: u16,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub alternate_screen: bool,
    pub screen_seq: u64,
    pub screen: vt100::Screen,
}

pub(crate) struct TerminalState {
    parser: vt100::Parser<Callbacks>,
    sequence: u64,
}
pub(crate) enum TerminalObservationError {
    Parser(CafleetError),
    Observer(CafleetError),
}

impl TerminalState {
    pub(crate) fn new(size: TerminalSize) -> Result<Self> {
        size.validate()?;
        Ok(Self {
            parser: vt100::Parser::new_with_callbacks(
                size.rows.try_into().expect("validated rows"),
                size.cols.try_into().expect("validated cols"),
                0,
                Callbacks::default(),
            ),
            sequence: 0,
        })
    }
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error("terminal sequence exhausted"))?;
        self.parser.process(bytes);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn feed_observed(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> Result<()> {
        self.feed_observed_classified(bytes, observer)
            .map_err(|failure| match failure {
                TerminalObservationError::Parser(cause)
                | TerminalObservationError::Observer(cause) => cause,
            })
    }
    pub(crate) fn feed_observed_classified(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
    ) -> std::result::Result<(), TerminalObservationError> {
        let mut failure = None;
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            if byte.is_ascii_control() {
                if index > start {
                    self.feed(&bytes[start..index])
                        .map_err(TerminalObservationError::Parser)?;
                    if let Err(cause) = observer.observe(&self.snapshot()) {
                        if observer.failures_are_fatal() {
                            return Err(TerminalObservationError::Observer(cause));
                        }
                        failure.get_or_insert(cause);
                    }
                }
                self.feed(&[*byte])
                    .map_err(TerminalObservationError::Parser)?;
                if let Err(cause) = observer.observe(&self.snapshot()) {
                    if observer.failures_are_fatal() {
                        return Err(TerminalObservationError::Observer(cause));
                    }
                    failure.get_or_insert(cause);
                }
                start = index + 1;
            }
        }
        if start < bytes.len() {
            self.feed(&bytes[start..])
                .map_err(TerminalObservationError::Parser)?;
            if let Err(cause) = observer.observe(&self.snapshot()) {
                if observer.failures_are_fatal() {
                    return Err(TerminalObservationError::Observer(cause));
                }
                failure.get_or_insert(cause);
            }
        }
        match failure {
            Some(cause) => Err(TerminalObservationError::Observer(cause)),
            None => Ok(()),
        }
    }
    pub(crate) fn resize(&mut self, size: TerminalSize) -> Result<()> {
        size.validate()?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| error("terminal sequence exhausted"))?;
        self.parser.set_size(
            size.rows.try_into().expect("validated rows"),
            size.cols.try_into().expect("validated cols"),
        );
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> ScreenSnapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (cursor_row, cursor_col) = screen.cursor_position();
        ScreenSnapshot {
            contents: screen.contents(),
            title: self.parser.callbacks().title.clone(),
            rows: rows.get(),
            cols: cols.get(),
            cursor_row,
            cursor_col,
            alternate_screen: screen.alternate_screen(),
            screen_seq: self.sequence,
            screen: screen.clone(),
        }
    }
}

pub(crate) struct RawTerminalGuard {
    terminal: File,
    original: Termios,
    restored: bool,
}
impl RawTerminalGuard {
    pub(crate) fn enter(terminal: &File) -> Result<Self> {
        let terminal = terminal.try_clone().map_err(io_error)?;
        let original = termios::tcgetattr(&terminal).map_err(io_error)?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&terminal, SetArg::TCSANOW, &raw).map_err(io_error)?;
        Ok(Self {
            terminal,
            original,
            restored: false,
        })
    }
    pub(crate) fn restore(&mut self) -> Result<()> {
        if !self.restored {
            termios::tcsetattr(&self.terminal, SetArg::TCSANOW, &self.original).map_err(
                |cause| {
                    error(format!(
                        "terminal restoration requires host recovery: {cause}"
                    ))
                },
            )?;
            self.restored = true;
        }
        Ok(())
    }
}
impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        if let Err(cause) = self.restore() {
            eprintln!("{cause}");
        }
    }
}
