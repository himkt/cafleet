use super::*;
use nix::sys::termios::{self, SetArg, Termios};
use std::fs::File;

#[cfg(test)]
#[derive(Default)]
struct Callbacks {
    title: String,
}
#[cfg(test)]
impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScreenSnapshot {
    #[cfg(test)]
    pub(crate) title: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) cursor_row: u16,
    pub(crate) cursor_col: u16,
    pub(crate) alternate_screen: bool,
    pub(crate) screen_seq: u64,
    pub(crate) screen: vt100::Screen,
}

pub(crate) struct TerminalState {
    #[cfg(test)]
    parser: vt100::Parser<Callbacks>,
    #[cfg(not(test))]
    parser: vt100::Parser,
    sequence: u64,
}
pub(crate) enum TerminalObservationError {
    Parser(CafleetError),
    Observer(CafleetError),
}

impl TerminalState {
    pub(crate) fn new(size: TerminalSize) -> Result<Self> {
        size.validate()?;
        let rows = size.rows.try_into().expect("validated rows");
        let cols = size.cols.try_into().expect("validated cols");
        #[cfg(test)]
        let parser = vt100::Parser::new_with_callbacks(rows, cols, 0, Callbacks::default());
        #[cfg(not(test))]
        let parser = vt100::Parser::new(rows, cols, 0);
        Ok(Self {
            parser,
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
                    self.step(&bytes[start..index], observer, &mut failure)?;
                }
                self.step(&[*byte], observer, &mut failure)?;
                start = index + 1;
            }
        }
        if start < bytes.len() {
            self.step(&bytes[start..], observer, &mut failure)?;
        }
        match failure {
            Some(cause) => Err(TerminalObservationError::Observer(cause)),
            None => Ok(()),
        }
    }
    fn step(
        &mut self,
        bytes: &[u8],
        observer: &mut impl ScreenObserver,
        failure: &mut Option<CafleetError>,
    ) -> std::result::Result<(), TerminalObservationError> {
        self.feed(bytes).map_err(TerminalObservationError::Parser)?;
        if let Err(cause) = observer.observe(&self.snapshot()) {
            if observer.failures_are_fatal() {
                return Err(TerminalObservationError::Observer(cause));
            }
            failure.get_or_insert(cause);
        }
        Ok(())
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
            #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::runner::transport::test_support::Observer;

    #[test]
    fn runner_terminal_fragmented_utf8_and_ansi_reconstruct_the_same_live_screen() {
        let bytes = "\u{1b}[2J\u{1b}[H許可しますか？ ✓\r\n> prompt".as_bytes();
        let size = TerminalSize { rows: 24, cols: 80 };
        let mut whole = TerminalState::new(size).unwrap();
        whole.feed(bytes).unwrap();
        for width in 1..=bytes.len() {
            let mut fragmented = TerminalState::new(size).unwrap();
            for part in bytes.chunks(width) {
                fragmented.feed(part).unwrap();
            }
            let expected = whole.snapshot();
            let actual = fragmented.snapshot();
            assert_eq!(
                actual.screen.contents(),
                expected.screen.contents(),
                "width={width}"
            );
            assert_eq!(
                (actual.cursor_row, actual.cursor_col),
                (expected.cursor_row, expected.cursor_col)
            );
            assert_eq!(actual.title, expected.title);
        }
    }

    #[test]
    fn runner_terminal_preserves_primary_and_alternate_screens_and_fragmented_titles() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 10, cols: 40 }).unwrap();
        terminal.feed(b"primary composer").unwrap();
        let primary = terminal.snapshot();
        for byte in b"\x1b]0;Action Required\x1b\\\x1b[?1049h\x1b[2J\x1b[Halternate request" {
            terminal.feed(&[*byte]).unwrap();
        }
        let alternate = terminal.snapshot();
        assert!(alternate.alternate_screen);
        assert!(alternate.screen.contents().contains("alternate request"));
        assert!(!alternate.screen.contents().contains("primary composer"));
        assert_eq!(alternate.title, "Action Required");
        terminal.feed(b"\x1b[?1049l").unwrap();
        let restored = terminal.snapshot();
        assert!(!restored.alternate_screen);
        assert_eq!(restored.screen.contents(), primary.screen.contents());
        assert!(restored.screen_seq > alternate.screen_seq);
    }

    #[test]
    fn runner_terminal_live_snapshot_excludes_scrollback_and_keeps_prior_snapshots_immutable() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 3, cols: 40 }).unwrap();
        terminal.feed(b"old permission request").unwrap();
        let old = terminal.snapshot();
        terminal
            .feed(b"\r\nline two\r\nline three\r\nempty composer")
            .unwrap();
        let current = terminal.snapshot();
        assert!(!current.screen.contents().contains("old permission request"));
        assert!(current.screen.contents().contains("empty composer"));
        assert!(old.screen.contents().contains("old permission request"));
        assert!(current.screen_seq > old.screen_seq);
    }

    #[test]
    fn runner_terminal_cursor_updates_and_resize_remain_consistent_with_dimensions() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        terminal.feed(b"\x1b[4;7Hchoice").unwrap();
        let before = terminal.snapshot();
        assert_eq!((before.cursor_row, before.cursor_col), (3, 12));
        terminal
            .resize(TerminalSize {
                rows: 40,
                cols: 120,
            })
            .unwrap();
        let resized = terminal.snapshot();
        assert_eq!((resized.rows, resized.cols), (40, 120));
        assert!(resized.screen.contents().contains("choice"));
        assert!(resized.screen_seq >= before.screen_seq);
    }

    #[test]
    fn runner_terminal_rejects_empty_dimensions_at_creation_and_resize() {
        for size in [
            TerminalSize { rows: 0, cols: 80 },
            TerminalSize { rows: 24, cols: 0 },
        ] {
            assert!(TerminalState::new(size).is_err());
            let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            assert!(terminal.resize(size).is_err());
            let snapshot = terminal.snapshot();
            assert_eq!((snapshot.rows, snapshot.cols), (24, 80));
        }
    }
    #[test]
    fn runner_transport_visits_a_transient_screen_before_a_later_clear_in_one_fragment() {
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let mut observer = Observer::default();
        terminal
            .feed_observed(b"candidate request\x1b[2J\x1b[Hcomposer", &mut observer)
            .unwrap();
        let candidate = observer
            .screens
            .iter()
            .position(|(_, screen)| screen.contains("candidate request"))
            .unwrap();
        let composer = observer
            .screens
            .iter()
            .rposition(|(_, screen)| {
                screen.contains("composer") && !screen.contains("candidate request")
            })
            .unwrap();
        assert!(candidate < composer);
        assert!(
            observer
                .screens
                .windows(2)
                .all(|pair| pair[1].0 > pair[0].0)
        );
    }
}
