package core

import (
	"math"

	"github.com/himkt/cafleet/internal/vt"
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
