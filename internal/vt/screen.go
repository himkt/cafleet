// Transliterated from vt100 0.18.1 src/screen.rs (MIT, Copyright (c) 2016 Jesse Luehrs).

package vt

import "github.com/himkt/cafleet/internal/vt/unicodewidth"

const (
	modeApplicationKeypad uint8 = 0b0000_0001
	modeApplicationCursor uint8 = 0b0000_0010
	modeHideCursor        uint8 = 0b0000_0100
	modeAlternateScreen   uint8 = 0b0000_1000
	modeBracketedPaste    uint8 = 0b0001_0000
)

type MouseProtocolMode uint8

const (
	MouseNone MouseProtocolMode = iota
	MousePress
	MousePressRelease
	MouseButtonMotion
	MouseAnyMotion
)

type MouseProtocolEncoding uint8

const (
	MouseEncodingDefault MouseProtocolEncoding = iota
	MouseEncodingUtf8
	MouseEncodingSgr
)

type CursorStyle uint8

const (
	CursorDefault CursorStyle = iota
	CursorBlinkBlock
	CursorSteadyBlock
	CursorBlinkUnderline
	CursorSteadyUnderline
	CursorBlinkVerticalBar
	CursorSteadyVerticalBar
)

// Screen is the terminal state: primary and alternate grids, drawing attributes and modes.
type Screen struct {
	grid          grid
	alternateGrid grid

	attrs      Attrs
	savedAttrs Attrs

	modes                 uint8
	mouseProtocolMode     MouseProtocolMode
	mouseProtocolEncoding MouseProtocolEncoding
	cursorStyle           CursorStyle

	lastChar    rune
	hasLastChar bool
}

func newScreen(size Size) Screen {
	s := Screen{
		grid:          newGrid(size, true),
		alternateGrid: newGrid(size, false),
	}
	s.grid.allocateRows()
	return s
}

// Clone deep-copies the screen.
func (s *Screen) Clone() *Screen {
	c := *s
	c.grid = s.grid.clone()
	c.alternateGrid = s.alternateGrid.clone()
	return &c
}

// CopyInto deep-copies the screen into dst, reusing dst's storage across calls.
func (s *Screen) CopyInto(dst *Screen) {
	g, alt := dst.grid, dst.alternateGrid
	*dst = *s
	dst.grid, dst.alternateGrid = g, alt
	s.grid.copyInto(&dst.grid)
	s.alternateGrid.copyInto(&dst.alternateGrid)
}

func (s *Screen) Size() (rows, cols uint16) {
	sz := s.activeGrid().size
	return sz.Rows, sz.Cols
}

func (s *Screen) Contents() string { return string(s.activeGrid().writeContents(nil)) }

// Rows returns the text of each visible row restricted to columns [start, start+width).
func (s *Screen) Rows(start, width uint16) []string {
	g := s.activeGrid()
	out := make([]string, len(g.rows))
	for i := range g.rows {
		out[i] = string(g.rows[i].writeContents(nil, start, width, false))
	}
	return out
}

func (s *Screen) CursorPosition() (row, col uint16) {
	p := s.activeGrid().pos
	return p.Row, p.Col
}

// Cell returns the visible cell at (row, col), or nil when out of range. The pointer aliases
// live screen state.
func (s *Screen) Cell(row, col uint16) *Cell { return s.activeGrid().cell(Pos{row, col}) }

func (s *Screen) RowWrapped(row uint16) bool {
	r := s.activeGrid().row(row)
	return r != nil && r.wrapped
}

func (s *Screen) AlternateScreen() bool   { return s.mode(modeAlternateScreen) }
func (s *Screen) ApplicationKeypad() bool { return s.mode(modeApplicationKeypad) }
func (s *Screen) ApplicationCursor() bool { return s.mode(modeApplicationCursor) }
func (s *Screen) HideCursor() bool        { return s.mode(modeHideCursor) }
func (s *Screen) BracketedPaste() bool    { return s.mode(modeBracketedPaste) }

func (s *Screen) MouseProtocolMode() MouseProtocolMode         { return s.mouseProtocolMode }
func (s *Screen) MouseProtocolEncoding() MouseProtocolEncoding { return s.mouseProtocolEncoding }
func (s *Screen) CursorStyle() CursorStyle                     { return s.cursorStyle }

// Attrs returns the attributes newly drawn text receives.
func (s *Screen) Attrs() Attrs { return s.attrs }

func (s *Screen) activeGrid() *grid {
	if s.mode(modeAlternateScreen) {
		return &s.alternateGrid
	}
	return &s.grid
}

func (s *Screen) enterAlternateGrid() {
	s.setMode(modeAlternateScreen)
	s.alternateGrid.allocateRows()
}

func (s *Screen) exitAlternateGrid() { s.clearMode(modeAlternateScreen) }

func (s *Screen) saveCursor() {
	s.activeGrid().saveCursor()
	s.savedAttrs = s.attrs
}

func (s *Screen) restoreCursor() {
	s.activeGrid().restoreCursor()
	s.attrs = s.savedAttrs
}

func (s *Screen) setMode(m uint8)   { s.modes |= m }
func (s *Screen) clearMode(m uint8) { s.modes &^= m }
func (s *Screen) mode(m uint8) bool { return s.modes&m != 0 }

func (s *Screen) clearMouseMode(m MouseProtocolMode) {
	if s.mouseProtocolMode == m {
		s.mouseProtocolMode = MouseNone
	}
}

func (s *Screen) clearMouseEncoding(e MouseProtocolEncoding) {
	if s.mouseProtocolEncoding == e {
		s.mouseProtocolEncoding = MouseEncodingDefault
	}
}

func (s *Screen) calculateWidth(c rune) (uint16, bool) {
	g := s.activeGrid()
	w, ok := unicodewidth.CharWidth(c)
	if !ok && c < 256 {
		return 0, false
	}
	if !ok {
		w = 1
	}
	width := min(uint16(w), 2)
	if width > g.size.Cols {
		return 0, false
	}
	if width == 0 && g.pos.Col == 0 {
		return 0, false
	}
	return width, true
}

func (s *Screen) newCharacterShouldWrap(width uint16) bool {
	g := s.activeGrid()
	if g.pos.Col+width > g.size.Cols {
		last := g.mustCell(Pos{g.pos.Row, g.size.Cols - 1})
		if last.HasContents() || last.IsWideContinuation() {
			return true
		}
	}
	return false
}

func (s *Screen) wrapWidenedCell(oldPos, newPos Pos) {
	g := s.activeGrid()
	if newPos.Col == 0 {
		attrs := s.attrs
		old := g.mustCell(oldPos)
		var buf [contentBytes]byte
		contents := append(buf[:0], old.ContentsBytes()...)
		g.mustCell(newPos).setContents(contents, attrs)
		g.mustCell(oldPos).clear(attrs)
		g.colInc(2)
	} else {
		g.colInc(1)
	}
}

func (s *Screen) textCleanup(base Pos, cellWasWide, cellIsWide bool) {
	g := s.activeGrid()
	size := g.size
	attrs := s.attrs
	if base.Col > 0 {
		prev := g.mustCell(Pos{base.Row, base.Col - 1})
		if prev.IsWide() {
			prev.clear(prev.attrs)
		}
	}
	if cellWasWide && !cellIsWide {
		g.mustCell(Pos{base.Row, base.Col + 1}).set(' ', attrs)
	}
	if !cellWasWide && cellIsWide {
		next := g.mustCell(Pos{base.Row, base.Col + 1})
		nextWasWide := next.IsWide()
		next.clear(Attrs{})
		next.setWideContinuation(true)
		if nextWasWide {
			g.mustCell(Pos{base.Row, base.Col + 2}).clear(attrs)
			if base.Col+2 == size.Cols-1 {
				g.mustRow(base.Row).wrap(false)
			}
		}
	}
}
