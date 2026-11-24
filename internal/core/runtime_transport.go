package core

import (
	"fmt"
	"math"
	"os"
	"strings"

	"github.com/himkt/cafleet/internal/vt"
	"golang.org/x/sys/unix"
)

// screenObserver receives the screen after every observation step.
type screenObserver interface {
	observe(screen *screenSnapshot) error
	parserFailure() error
}

type screenSnapshot struct {
	rows uint16
	cols uint16
	// cursor is (row, col); absent for a capture whose source exposes no cursor.
	cursor opt[[2]uint16]
	// alternateScreen is absent for a capture whose source exposes no screen mode.
	alternateScreen opt[bool]
	screenSeq       uint64
	screen          *vt.Screen
}

type terminalState struct {
	parser   *vt.Parser
	sequence uint64
}

//sumtype:decl
type terminalObservationError interface{ isTerminalObservationError() }

type terminalObservationErrorParser struct{ cause error }

type terminalObservationErrorObserver struct {
	cause error
	// consumed is the failing step's end: its bytes reached the parser.
	consumed int
}

func (terminalObservationErrorParser) isTerminalObservationError()   {}
func (terminalObservationErrorObserver) isTerminalObservationError() {}

// minScreenCols keeps the screen model at two columns or more: vt100 panics when a wide or
// combining scalar lands on a one-column grid. The PTY keeps the pane's own size.
const minScreenCols uint16 = 2

func screenCols(size terminalSize) uint16 {
	if size.cols == 0 {
		panic("validated cols")
	}
	return max(size.cols, minScreenCols)
}

func newTerminalState(size terminalSize) (*terminalState, error) {
	if err := size.validate(); err != nil {
		return nil, err
	}
	return &terminalState{parser: vt.NewParser(size.rows, screenCols(size), nil)}, nil
}

func (t *terminalState) advanceSequence(by uint64) error {
	if t.sequence > math.MaxUint64-by {
		return runtimeError("terminal sequence exhausted")
	}
	t.sequence += by
	return nil
}

func (t *terminalState) feed(bytes []byte) error {
	if err := t.advanceSequence(uint64(len(bytes))); err != nil {
		return err
	}
	t.parser.Process(bytes)
	return nil
}

// feedObservedClassified feeds bytes one observation step at a time (every ASCII control byte
// alone, and every run of other bytes between them) and shows the observer the screen after
// each step. It returns nil when every step succeeds.
func (t *terminalState) feedObservedClassified(bytes []byte, observer screenObserver) terminalObservationError {
	start := 0
	for index, b := range bytes {
		if b < 0x20 || b == 0x7F {
			if index > start {
				if failure := t.step(bytes, start, index, observer); failure != nil {
					return failure
				}
			}
			if failure := t.step(bytes, index, index+1, observer); failure != nil {
				return failure
			}
			start = index + 1
		}
	}
	if start < len(bytes) {
		return t.step(bytes, start, len(bytes), observer)
	}
	return nil
}

func (t *terminalState) step(bytes []byte, start, end int, observer screenObserver) terminalObservationError {
	if err := t.feed(bytes[start:end]); err != nil {
		return terminalObservationErrorParser{cause: err}
	}
	view := t.view()
	if err := observer.observe(&view); err != nil {
		return terminalObservationErrorObserver{cause: err, consumed: end}
	}
	return nil
}

func (t *terminalState) resize(size terminalSize) error {
	if err := size.validate(); err != nil {
		return err
	}
	if err := t.advanceSequence(1); err != nil {
		return err
	}
	t.parser.SetSize(size.rows, screenCols(size))
	return nil
}

// terminalStateReplay is a fresh terminal of the capture's size with each captured row written at
// its row position, carrying exactly the cursor and screen mode the capture reports.
func terminalStateReplay(capture *screenCapture) (screenSnapshot, error) {
	terminal, err := newTerminalState(terminalSize{rows: capture.rows, cols: capture.cols})
	if err != nil {
		return screenSnapshot{}, err
	}
	var bytes []byte
	if alternate, known := capture.alternateScreen.get(); known && alternate {
		bytes = append(bytes, "\x1b[?1049h"...)
	}
	body := strings.TrimSuffix(capture.body, "\n")
	for row, line := range strings.Split(body, "\n") {
		if row >= int(capture.rows) {
			break
		}
		bytes = fmt.Appendf(bytes, "\x1b[%d;1H", row+1)
		bytes = append(bytes, strings.TrimSuffix(line, "\r")...)
		bytes = append(bytes, "\x1b[0m"...)
	}
	if cursor, known := capture.cursor.get(); known {
		bytes = fmt.Appendf(bytes, "\x1b[%d;%dH", int(cursor[0])+1, int(cursor[1])+1)
	}
	if err := terminal.feed(bytes); err != nil {
		return screenSnapshot{}, err
	}
	snapshot := terminal.snapshot()
	snapshot.cursor = capture.cursor
	snapshot.alternateScreen = capture.alternateScreen
	return snapshot, nil
}

// view is the current screen without a copy; it is valid until the next feed or resize.
func (t *terminalState) view() screenSnapshot {
	screen := t.parser.Screen()
	rows, cols := screen.Size()
	row, col := screen.CursorPosition()
	return screenSnapshot{
		rows:            rows,
		cols:            cols,
		cursor:          some([2]uint16{row, col}),
		alternateScreen: some(screen.AlternateScreen()),
		screenSeq:       t.sequence,
		screen:          screen,
	}
}

// snapshot is the current screen as an owned copy that later feeds and resizes leave unchanged.
func (t *terminalState) snapshot() screenSnapshot {
	s := t.view()
	s.screen = s.screen.Clone()
	return s
}

// rawTerminalGuard holds a terminal in raw mode (F16) until restore or close returns it to its
// original attributes.
type rawTerminalGuard struct {
	terminal *os.File
	original unix.Termios
	restored bool
}

func rawTerminalGuardEnter(terminal *os.File) (*rawTerminalGuard, error) {
	duplicate, err := unix.FcntlInt(terminal.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	guard := &rawTerminalGuard{terminal: os.NewFile(uintptr(duplicate), terminal.Name())}
	original, err := getTermios(duplicate)
	if err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	guard.original = *original
	raw := *original
	cfmakeraw(&raw)
	if err := setTermios(duplicate, &raw); err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	return guard, nil
}

func (g *rawTerminalGuard) restore() error {
	if g.restored {
		return nil
	}
	if err := setTermios(int(g.terminal.Fd()), &g.original); err != nil {
		return runtimeError("terminal restoration requires host recovery: " + errnoText(err))
	}
	g.restored = true
	return nil
}

// close restores the terminal, reporting a failed restoration on stderr, and releases the
// guard's descriptor.
func (g *rawTerminalGuard) close() {
	if err := g.restore(); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
	closeFile(g.terminal)
}
