use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::rc::Rc;

use super::*;
use crate::error::CafleetError;

struct Bytes {
    chunks: VecDeque<Vec<u8>>,
    eof: bool,
}

impl Bytes {
    fn new(bytes: &[u8], width: usize) -> Self {
        assert!(width > 0);
        Self {
            chunks: bytes.chunks(width).map(<[u8]>::to_vec).collect(),
            eof: false,
        }
    }
}

impl ByteInput for Bytes {
    fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let Some(mut chunk) = self.chunks.pop_front() else {
            return if self.eof {
                Ok(0)
            } else {
                Err(ErrorKind::WouldBlock.into())
            };
        };
        let count = target.len().min(chunk.len());
        target[..count].copy_from_slice(&chunk[..count]);
        chunk.drain(..count);
        if !chunk.is_empty() {
            self.chunks.push_front(chunk);
        }
        Ok(count)
    }
}

struct Pty {
    output: Bytes,
    input: Vec<u8>,
    blocked: bool,
    max_write: usize,
    sizes: Vec<TerminalSize>,
    closed: bool,
}

impl Pty {
    fn new(output: &[u8], width: usize) -> Self {
        Self {
            output: Bytes::new(output, width),
            input: Vec::new(),
            blocked: false,
            max_write: usize::MAX,
            sizes: Vec::new(),
            closed: false,
        }
    }
}

impl ByteInput for Pty {
    fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        self.output.try_read(target)
    }
}

impl PtyIo for Pty {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.blocked {
            return Err(ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(self.max_write);
        self.input.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn resize(&mut self, size: TerminalSize) -> Result<(), CafleetError> {
        self.sizes.push(size);
        Ok(())
    }
    fn close(&mut self) -> Result<(), CafleetError> {
        self.closed = true;
        Ok(())
    }
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    blocked: bool,
    broken: bool,
    max_write: Option<usize>,
    ordering: Option<Rc<RefCell<Vec<&'static str>>>>,
}

impl Display for Output {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.broken {
            return Err(ErrorKind::BrokenPipe.into());
        }
        if self.blocked {
            return Err(ErrorKind::WouldBlock.into());
        }
        if let Some(ordering) = &self.ordering {
            ordering.borrow_mut().push("display");
        }
        let count = bytes.len().min(self.max_write.unwrap_or(bytes.len()));
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn try_flush(&mut self) -> io::Result<()> {
        if self.blocked {
            Err(ErrorKind::WouldBlock.into())
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Observer {
    screens: Vec<(u64, String)>,
    fail: bool,
    ordering: Option<Rc<RefCell<Vec<&'static str>>>>,
}

impl ScreenObserver for Observer {
    fn observe(&mut self, screen: &ScreenSnapshot) -> Result<(), CafleetError> {
        if let Some(ordering) = &self.ordering {
            ordering.borrow_mut().push("observe");
        }
        self.screens
            .push((screen.screen_seq, screen.contents.clone()));
        if self.fail {
            Err(CafleetError::App(
                "fixture observation queue unavailable".into(),
            ))
        } else {
            Ok(())
        }
    }
}

fn transport() -> Transport {
    Transport::new(
        TerminalSize { rows: 24, cols: 80 },
        TransportLimits {
            input_bytes: 32,
            display_bytes: 64,
            bytes_per_tick: 16,
        },
    )
    .unwrap()
}

#[test]
fn runner_transport_preserves_fragmented_child_and_human_bytes_through_partial_writes() {
    let child = "\u{1b}[2J許可 ✓\r\n\u{1b}]0;title\u{7}".as_bytes();
    let human = "\u{1b}[200~literal 世界\u{1b}[201~\r\u{3}".as_bytes();
    let mut transport = transport();
    let mut pty = Pty::new(child, 1);
    pty.max_write = 1;
    let mut human_input = Bytes::new(human, 2);
    let mut display = Output {
        max_write: Some(1),
        ..Default::default()
    };
    let mut observer = Observer::default();
    for _ in 0..1024 {
        transport
            .tick(&mut human_input, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    assert_eq!(pty.input, human);
    assert_eq!(display.bytes, child);
    assert_eq!(transport.pending_input_bytes(), 0);
    assert_eq!(transport.pending_display_bytes(), 0);
}

#[test]
fn runner_transport_observes_reconstructed_output_before_displaying_it() {
    let ordering = Rc::new(RefCell::new(Vec::new()));
    let mut transport = transport();
    let mut pty = Pty::new(b"request", 7);
    let mut human = Bytes::new(b"", 1);
    let mut display = Output {
        ordering: Some(ordering.clone()),
        ..Default::default()
    };
    let mut observer = Observer {
        ordering: Some(ordering.clone()),
        ..Default::default()
    };
    for _ in 0..10 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    let calls = ordering.borrow();
    let observe = calls.iter().position(|call| *call == "observe").unwrap();
    let displayed = calls.iter().position(|call| *call == "display").unwrap();
    assert!(observe < displayed);
    assert!(
        observer
            .screens
            .iter()
            .any(|(_, screen)| screen.contains("request"))
    );
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

#[test]
fn runner_transport_full_display_queue_bounds_memory_while_human_input_continues() {
    let payload = vec![b'x'; 4096];
    let mut transport = transport();
    let mut pty = Pty::new(&payload, 16);
    let mut human = Bytes::new(b"human native answer\r", 1);
    let mut display = Output {
        blocked: true,
        ..Default::default()
    };
    let mut observer = Observer::default();
    for _ in 0..128 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
        assert!(transport.pending_display_bytes() <= 64);
        assert!(transport.pending_input_bytes() <= 32);
    }
    assert_eq!(pty.input, b"human native answer\r");
    assert!(!pty.output.chunks.is_empty());
    display.blocked = false;
    for _ in 0..1024 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    assert_eq!(display.bytes, payload);
}

#[test]
fn runner_transport_bounded_input_backpressure_preserves_all_pending_human_bytes() {
    let payload = vec![b'a'; 512];
    let mut transport = transport();
    let mut pty = Pty::new(b"", 1);
    pty.blocked = true;
    let mut human = Bytes::new(&payload, 16);
    let mut display = Output::default();
    let mut observer = Observer::default();
    for _ in 0..64 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
        assert!(transport.pending_input_bytes() <= 32);
    }
    assert!(!human.chunks.is_empty());
    pty.blocked = false;
    pty.max_write = 1;
    for _ in 0..2048 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    assert_eq!(pty.input, payload);
}

#[test]
fn runner_transport_observation_queue_failure_reports_degradation_and_preserves_manual_io() {
    let mut transport = transport();
    let mut pty = Pty::new(b"native output", 4);
    let mut human = Bytes::new(b"yes\r", 1);
    let mut display = Output::default();
    let mut observer = Observer {
        fail: true,
        ..Default::default()
    };
    let mut reported = false;
    for _ in 0..64 {
        let progress = transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
        reported |= progress.observation_error.is_some();
    }
    assert!(reported);
    assert_eq!(pty.input, b"yes\r");
    assert_eq!(display.bytes, b"native output");
}

#[test]
fn runner_transport_forwards_one_outer_terminal_query_answer_without_parser_duplicates() {
    let mut transport = transport();
    let mut pty = Pty::new(b"\x1b[6n", 1);
    let mut human = Bytes::new(b"\x1b[1;1R", 1);
    let mut display = Output::default();
    let mut observer = Observer::default();
    for _ in 0..64 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    assert_eq!(pty.input, b"\x1b[1;1R");
    assert_eq!(display.bytes, b"\x1b[6n");
}

#[test]
fn runner_transport_resize_reaches_the_pty_and_screen_together() {
    let mut transport = transport();
    let mut pty = Pty::new(b"", 1);
    transport
        .resize(
            &mut pty,
            TerminalSize {
                rows: 40,
                cols: 100,
            },
        )
        .unwrap();
    assert_eq!(
        pty.sizes,
        [TerminalSize {
            rows: 40,
            cols: 100
        }]
    );
    assert_eq!(
        (transport.snapshot().rows, transport.snapshot().cols),
        (40, 100)
    );
}

#[test]
fn runner_transport_reports_outer_and_child_eof_as_distinct_observations() {
    for outer in [false, true] {
        let mut transport = transport();
        let mut pty = Pty::new(b"", 1);
        pty.output.eof = !outer;
        let mut human = Bytes::new(b"", 1);
        human.eof = outer;
        let progress = transport
            .tick(
                &mut human,
                &mut pty,
                &mut Output::default(),
                &mut Observer::default(),
            )
            .unwrap();
        assert_eq!(progress.outer_eof, outer);
        assert_eq!(progress.child_eof, !outer);
    }
}

#[test]
fn runner_transport_broken_display_fails_and_tail_discard_counts_only_pending_bytes() {
    let mut transport = transport();
    let mut pty = Pty::new(b"undelivered tail", 16);
    let mut human = Bytes::new(b"", 1);
    let mut display = Output {
        blocked: true,
        ..Default::default()
    };
    let mut observer = Observer::default();
    for _ in 0..4 {
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .unwrap();
    }
    let pending = transport.pending_display_bytes();
    assert!(pending > 0);
    assert_eq!(transport.discard_display_tail(), pending);
    assert_eq!(transport.discard_display_tail(), 0);
    pty.output = Bytes::new(b"broken", 6);
    display.blocked = false;
    display.broken = true;
    assert!(
        transport
            .tick(&mut human, &mut pty, &mut display, &mut observer)
            .is_err()
    );
}

#[test]
fn runner_transport_requires_positive_finite_limits() {
    for field in ["input", "display", "tick"] {
        let mut limits = TransportLimits {
            input_bytes: 32,
            display_bytes: 64,
            bytes_per_tick: 16,
        };
        match field {
            "input" => limits.input_bytes = 0,
            "display" => limits.display_bytes = 0,
            "tick" => limits.bytes_per_tick = 0,
            _ => unreachable!(),
        }
        assert!(
            Transport::new(TerminalSize { rows: 24, cols: 80 }, limits).is_err(),
            "{field}"
        );
    }
}
