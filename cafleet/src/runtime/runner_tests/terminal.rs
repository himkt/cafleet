use super::*;

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
        assert_eq!(actual.contents, expected.contents, "width={width}");
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
    assert!(alternate.contents.contains("alternate request"));
    assert!(!alternate.contents.contains("primary composer"));
    assert_eq!(alternate.title, "Action Required");
    terminal.feed(b"\x1b[?1049l").unwrap();
    let restored = terminal.snapshot();
    assert!(!restored.alternate_screen);
    assert_eq!(restored.contents, primary.contents);
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
    assert!(!current.contents.contains("old permission request"));
    assert!(current.contents.contains("empty composer"));
    assert!(old.contents.contains("old permission request"));
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
    assert!(resized.contents.contains("choice"));
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
