// Transliterated from vt100 0.18.1 src/perform.rs, src/callbacks.rs and the WrappedScreen half of
// src/screen.rs (MIT, Copyright (c) 2016 Jesse Luehrs).

package vt

import (
	"bytes"

	"github.com/himkt/cafleet/internal/vt/vte"
)

// Callbacks receives events that do not change the screen directly. Embed NopCallbacks to
// implement only the events you need. The byte slices alias parser buffers; copy to retain them.
type Callbacks interface {
	AudibleBell(s *Screen)
	VisualBell(s *Screen)
	Resize(s *Screen, rows, cols uint16)
	SetWindowIconName(s *Screen, name []byte)
	SetWindowTitle(s *Screen, title []byte)
	CopyToClipboard(s *Screen, ty, data []byte)
	PasteFromClipboard(s *Screen, ty []byte)
	UnhandledChar(s *Screen, c rune)
	UnhandledControl(s *Screen, b byte)
	UnhandledEscape(s *Screen, i1, i2 int, b byte)
	UnhandledCsi(s *Screen, i1, i2 int, params *vte.Params, c byte)
	UnhandledOsc(s *Screen, params [][]byte)
}

// NopCallbacks ignores every event. Intermediate arguments are -1 when absent.
type NopCallbacks struct{}

func (NopCallbacks) AudibleBell(*Screen)                               {}
func (NopCallbacks) VisualBell(*Screen)                                {}
func (NopCallbacks) Resize(*Screen, uint16, uint16)                    {}
func (NopCallbacks) SetWindowIconName(*Screen, []byte)                 {}
func (NopCallbacks) SetWindowTitle(*Screen, []byte)                    {}
func (NopCallbacks) CopyToClipboard(*Screen, []byte, []byte)           {}
func (NopCallbacks) PasteFromClipboard(*Screen, []byte)                {}
func (NopCallbacks) UnhandledChar(*Screen, rune)                       {}
func (NopCallbacks) UnhandledControl(*Screen, byte)                    {}
func (NopCallbacks) UnhandledEscape(*Screen, int, int, byte)           {}
func (NopCallbacks) UnhandledCsi(*Screen, int, int, *vte.Params, byte) {}
func (NopCallbacks) UnhandledOsc(*Screen, [][]byte)                    {}

type wrappedScreen struct {
	screen    Screen
	callbacks Callbacks
}

func (w *wrappedScreen) text(c rune) {
	s := &w.screen
	pos := s.activeGrid().pos
	attrs := s.attrs
	width, ok := s.calculateWidth(c)
	if !ok {
		return
	}
	var base Pos
	var cellWasWide bool
	if width > 0 {
		wrap := s.newCharacterShouldWrap(width)
		g := s.activeGrid()
		g.colWrap(width, wrap, noRow)
		base = g.pos
		cell := g.mustCell(base)
		cellWasWide = cell.IsWide()
		cell.set(c, attrs)
		s.lastChar, s.hasLastChar = c, true
		g.colInc(width)
	} else {
		g := s.activeGrid()
		base = Pos{pos.Row, pos.Col - 1}
		if g.mustCell(base).IsWideContinuation() {
			base = Pos{pos.Row, pos.Col - 2}
		}
		cell := g.mustCell(base)
		cellWasWide = cell.IsWide()
		cell.append(c)
		if !cellWasWide && cell.IsWide() {
			g.colWrap(1, false, noRow)
			newPos := g.pos
			s.wrapWidenedCell(base, newPos)
			if newPos.Col == 0 {
				base = newPos
			}
		}
	}
	cellIsWide := s.activeGrid().mustCell(base).IsWide()
	s.textCleanup(base, cellWasWide, cellIsWide)
}

func (w *wrappedScreen) ris() {
	w.screen = newScreen(w.screen.grid.size)
}

func (w *wrappedScreen) decsc() { w.screen.saveCursor() }
func (w *wrappedScreen) decrc() { w.screen.restoreCursor() }

func (w *wrappedScreen) Print(c rune) {
	if c == 0xFFFD || (c >= 0x80 && c < 0xA0) {
		w.callbacks.UnhandledChar(&w.screen, c)
		return
	}
	w.text(c)
}

func (w *wrappedScreen) Execute(b byte) {
	g := w.screen.activeGrid()
	switch b {
	case 7:
		w.callbacks.AudibleBell(&w.screen)
	case 8:
		g.colDec(1)
	case 9:
		g.colTab()
	case 10, 11, 12:
		g.rowIncScroll(1, noRow)
	case 13:
		g.colSet(0)
	case 14, 15:
	default:
		w.callbacks.UnhandledControl(&w.screen, b)
	}
}

func (w *wrappedScreen) Hook(*vte.Params, []byte, bool, byte) {}
func (w *wrappedScreen) Put(byte)                             {}
func (w *wrappedScreen) Unhook()                              {}

func intermediate(intermediates []byte, i int) int {
	if i < len(intermediates) {
		return int(intermediates[i])
	}
	return -1
}

func (w *wrappedScreen) EscDispatch(intermediates []byte, _ bool, b byte) {
	if len(intermediates) > 0 {
		w.callbacks.UnhandledEscape(&w.screen, int(intermediates[0]), intermediate(intermediates, 1), b)
		return
	}
	switch b {
	case '7':
		w.decsc()
	case '8':
		w.decrc()
	case '=':
		w.screen.setMode(modeApplicationKeypad)
	case '>':
		w.screen.clearMode(modeApplicationKeypad)
	case 'M':
		w.screen.activeGrid().rowDecScroll(1)
	case 'c':
		w.ris()
	case 'g':
		w.callbacks.VisualBell(&w.screen)
	default:
		w.callbacks.UnhandledEscape(&w.screen, -1, -1, b)
	}
}

func firstParam(params *vte.Params, n int) uint16 {
	it := params.Iter()
	var v uint16
	for i := 0; i <= n; i++ {
		p, ok := it.Next()
		if !ok {
			return 0
		}
		v = 0
		if len(p) > 0 {
			v = p[0]
		}
	}
	return v
}

func canonicalize1(params *vte.Params, def uint16) uint16 {
	if v := firstParam(params, 0); v != 0 {
		return v
	}
	return def
}

func canonicalize2(params *vte.Params, def1, def2 uint16) (uint16, uint16) {
	a, b := firstParam(params, 0), firstParam(params, 1)
	if a == 0 {
		a = def1
	}
	if b == 0 {
		b = def2
	}
	return a, b
}

func (w *wrappedScreen) CsiDispatch(params *vte.Params, intermediates []byte, _ bool, c byte) {
	s := &w.screen
	unhandled := func() {
		w.callbacks.UnhandledCsi(&w.screen, intermediate(intermediates, 0), intermediate(intermediates, 1), params, c)
	}
	if len(intermediates) == 0 {
		g := s.activeGrid()
		switch c {
		case '@':
			g.insertCells(canonicalize1(params, 1))
		case 'A':
			g.rowDecClamp(canonicalize1(params, 1))
		case 'B':
			g.rowIncClamp(canonicalize1(params, 1))
		case 'C':
			g.colIncClamp(canonicalize1(params, 1))
		case 'D':
			g.colDec(canonicalize1(params, 1))
		case 'E':
			n := canonicalize1(params, 1)
			g.colSet(0)
			g.rowIncClamp(n)
		case 'F':
			n := canonicalize1(params, 1)
			g.colSet(0)
			g.rowDecClamp(n)
		case 'G', '`':
			g.colSet(canonicalize1(params, 1) - 1)
		case 'H', 'f':
			row, col := canonicalize2(params, 1, 1)
			g.setPos(Pos{row - 1, col - 1})
		case 'J':
			w.ed(canonicalize1(params, 0), unhandled)
		case 'K':
			w.el(canonicalize1(params, 0), unhandled)
		case 'L':
			g.insertLines(canonicalize1(params, 1))
		case 'M':
			g.deleteLines(canonicalize1(params, 1))
		case 'P':
			g.deleteCells(canonicalize1(params, 1))
		case 'S':
			g.scrollUp(canonicalize1(params, 1), noRow)
		case 'T':
			g.scrollDown(canonicalize1(params, 1))
		case 'X':
			g.eraseCells(canonicalize1(params, 1), s.attrs)
		case 'b':
			w.rep(canonicalize1(params, 1))
		case 'd':
			g.rowSet(canonicalize1(params, 1) - 1)
		case 'm':
			w.sgr(params, unhandled)
		case 'r':
			top, bottom := canonicalize2(params, 1, g.size.Rows)
			g.setScrollRegion(top-1, bottom-1)
		case 't':
			it := params.Iter()
			op, ok := it.Next()
			if ok && len(op) > 0 && op[0] == 8 {
				rows, cols := s.Size()
				if p, ok := it.Next(); ok && len(p) > 0 {
					rows = p[0]
				}
				if p, ok := it.Next(); ok && len(p) > 0 {
					cols = p[0]
				}
				w.callbacks.Resize(&w.screen, rows, cols)
			} else {
				w.callbacks.UnhandledCsi(&w.screen, -1, -1, params, c)
			}
		default:
			w.callbacks.UnhandledCsi(&w.screen, -1, -1, params, c)
		}
		return
	}
	switch intermediates[0] {
	case ' ':
		if c == 'q' {
			w.decscusr(canonicalize1(params, 0), unhandled)
		} else {
			w.callbacks.UnhandledCsi(&w.screen, -1, -1, params, c)
		}
	case '?':
		switch c {
		case 'J':
			w.ed(canonicalize1(params, 0), unhandled)
		case 'K':
			w.el(canonicalize1(params, 0), unhandled)
		case 'h':
			w.decset(params, unhandled)
		case 'l':
			w.decrst(params, unhandled)
		default:
			w.callbacks.UnhandledCsi(&w.screen, '?', intermediate(intermediates, 1), params, c)
		}
	default:
		unhandled()
	}
}

func (w *wrappedScreen) decscusr(style uint16, unhandled func()) {
	if style > 6 {
		unhandled()
		return
	}
	w.screen.cursorStyle = CursorStyle(style)
}

func (w *wrappedScreen) rep(count uint16) {
	if !w.screen.hasLastChar {
		return
	}
	c := w.screen.lastChar
	for range count {
		w.text(c)
	}
}

func (w *wrappedScreen) ed(mode uint16, unhandled func()) {
	g := w.screen.activeGrid()
	attrs := w.screen.attrs
	switch mode {
	case 0:
		g.eraseAllForward(attrs)
	case 1:
		g.eraseAllBackward(attrs)
	case 2:
		g.eraseAll(attrs)
	case 3:
		g.eraseAllScrollback()
	default:
		unhandled()
	}
}

func (w *wrappedScreen) el(mode uint16, unhandled func()) {
	g := w.screen.activeGrid()
	attrs := w.screen.attrs
	switch mode {
	case 0:
		g.eraseRowForward(attrs)
	case 1:
		g.eraseRowBackward(attrs)
	case 2:
		g.eraseRow(attrs)
	default:
		unhandled()
	}
}

func single(p []uint16) (uint16, bool) {
	if len(p) == 1 {
		return p[0], true
	}
	return 0, false
}

func (w *wrappedScreen) decset(params *vte.Params, unhandled func()) {
	it := params.Iter()
	for {
		p, ok := it.Next()
		if !ok {
			return
		}
		s := &w.screen
		v, one := single(p)
		if !one {
			unhandled()
			continue
		}
		switch v {
		case 1:
			s.setMode(modeApplicationCursor)
		case 6:
			s.activeGrid().setOriginMode(true)
		case 9:
			s.mouseProtocolMode = MousePress
		case 25:
			s.clearMode(modeHideCursor)
		case 47:
			s.enterAlternateGrid()
		case 1000:
			s.mouseProtocolMode = MousePressRelease
		case 1002:
			s.mouseProtocolMode = MouseButtonMotion
		case 1003:
			s.mouseProtocolMode = MouseAnyMotion
		case 1005:
			s.mouseProtocolEncoding = MouseEncodingUtf8
		case 1006:
			s.mouseProtocolEncoding = MouseEncodingSgr
		case 1049:
			w.decsc()
			w.screen.alternateGrid.clear()
			w.screen.enterAlternateGrid()
		case 2004:
			s.setMode(modeBracketedPaste)
		default:
			unhandled()
		}
	}
}

func (w *wrappedScreen) decrst(params *vte.Params, unhandled func()) {
	it := params.Iter()
	for {
		p, ok := it.Next()
		if !ok {
			return
		}
		s := &w.screen
		v, one := single(p)
		if !one {
			unhandled()
			continue
		}
		switch v {
		case 1:
			s.clearMode(modeApplicationCursor)
		case 6:
			s.activeGrid().setOriginMode(false)
		case 9:
			s.clearMouseMode(MousePress)
		case 25:
			s.setMode(modeHideCursor)
		case 47:
			s.exitAlternateGrid()
		case 1000:
			s.clearMouseMode(MousePressRelease)
		case 1002:
			s.clearMouseMode(MouseButtonMotion)
		case 1003:
			s.clearMouseMode(MouseAnyMotion)
		case 1005:
			s.clearMouseEncoding(MouseEncodingUtf8)
		case 1006:
			s.clearMouseEncoding(MouseEncodingSgr)
		case 1049:
			s.exitAlternateGrid()
			w.decrc()
		case 2004:
			s.clearMode(modeBracketedPaste)
		default:
			unhandled()
		}
	}
}

func toU8(v uint16) (uint8, bool) {
	if v > 0xFF {
		return 0, false
	}
	return uint8(v), true
}

// sgr mirrors vt100's macro-driven SGR loop: a missing or malformed color component ends the
// whole sequence, while an unrecognized parameter is reported and skipped.
func (w *wrappedScreen) sgr(params *vte.Params, unhandled func()) {
	if params.IsEmpty() {
		w.screen.attrs = Attrs{}
		return
	}
	it := params.Iter()
	nextU8 := func() (uint8, bool) {
		p, ok := it.Next()
		if !ok {
			return 0, false
		}
		v, one := single(p)
		if !one {
			return 0, false
		}
		return toU8(v)
	}
	extColor := func() (Color, bool) {
		p, ok := it.Next()
		if !ok {
			return Color{}, false
		}
		switch v, one := single(p); {
		case one && v == 2:
			r, ok := nextU8()
			if !ok {
				return Color{}, false
			}
			g, ok := nextU8()
			if !ok {
				return Color{}, false
			}
			b, ok := nextU8()
			if !ok {
				return Color{}, false
			}
			return Rgb(r, g, b), true
		case one && v == 5:
			i, ok := nextU8()
			if !ok {
				return Color{}, false
			}
			return Idx(i), true
		default:
			unhandled()
			return Color{}, false
		}
	}
	inlineColor := func(p []uint16) (Color, bool, bool) {
		switch {
		case len(p) == 5 && p[1] == 2:
			r, ok1 := toU8(p[2])
			if !ok1 {
				return Color{}, true, false
			}
			g, ok2 := toU8(p[3])
			if !ok2 {
				return Color{}, true, false
			}
			b, ok3 := toU8(p[4])
			if !ok3 {
				return Color{}, true, false
			}
			return Rgb(r, g, b), true, true
		case len(p) == 3 && p[1] == 5:
			i, ok := toU8(p[2])
			if !ok {
				return Color{}, true, false
			}
			return Idx(i), true, true
		}
		return Color{}, false, false
	}
	for {
		p, ok := it.Next()
		if !ok {
			return
		}
		a := &w.screen.attrs
		if len(p) > 1 && (p[0] == 38 || p[0] == 48) {
			color, matched, valid := inlineColor(p)
			if matched {
				if !valid {
					return
				}
				if p[0] == 38 {
					a.Fg = color
				} else {
					a.Bg = color
				}
				continue
			}
			unhandled()
			continue
		}
		v, one := single(p)
		if !one {
			unhandled()
			continue
		}
		switch {
		case v == 0:
			*a = Attrs{}
		case v == 1:
			a.setBold()
		case v == 2:
			a.setDim()
		case v == 3:
			a.setFlag(textModeItalic, true)
		case v == 4:
			a.setFlag(textModeUnderline, true)
		case v == 7:
			a.setFlag(textModeInverse, true)
		case v == 22:
			a.setNormalIntensity()
		case v == 23:
			a.setFlag(textModeItalic, false)
		case v == 24:
			a.setFlag(textModeUnderline, false)
		case v == 27:
			a.setFlag(textModeInverse, false)
		case v >= 30 && v <= 37:
			a.Fg = Idx(uint8(v) - 30)
		case v == 38:
			color, ok := extColor()
			if !ok {
				return
			}
			w.screen.attrs.Fg = color
		case v == 39:
			a.Fg = Color{}
		case v >= 40 && v <= 47:
			a.Bg = Idx(uint8(v) - 40)
		case v == 48:
			color, ok := extColor()
			if !ok {
				return
			}
			w.screen.attrs.Bg = color
		case v == 49:
			a.Bg = Color{}
		case v >= 90 && v <= 97:
			a.Fg = Idx(uint8(v) - 82)
		case v >= 100 && v <= 107:
			a.Bg = Idx(uint8(v) - 92)
		default:
			unhandled()
		}
	}
}

var (
	base64Chars       = []byte("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=")
	clipboardSelector = []byte("cpqs01234567")
)

func allIn(s, set []byte) bool {
	for _, c := range s {
		if bytes.IndexByte(set, c) < 0 {
			return false
		}
	}
	return true
}

func (w *wrappedScreen) OscDispatch(params [][]byte, _ bool) {
	s := &w.screen
	switch {
	case len(params) == 2 && string(params[0]) == "0":
		w.callbacks.SetWindowIconName(s, params[1])
		w.callbacks.SetWindowTitle(s, params[1])
	case len(params) == 2 && string(params[0]) == "1":
		w.callbacks.SetWindowIconName(s, params[1])
	case len(params) == 2 && string(params[0]) == "2":
		w.callbacks.SetWindowTitle(s, params[1])
	case len(params) == 3 && string(params[0]) == "52":
		ty, data := params[1], params[2]
		switch {
		case allIn(ty, clipboardSelector) && string(data) == "?":
			w.callbacks.PasteFromClipboard(s, ty)
		case allIn(ty, clipboardSelector) && allIn(data, base64Chars):
			w.callbacks.CopyToClipboard(s, ty, data)
		default:
			w.callbacks.UnhandledOsc(s, params)
		}
	default:
		w.callbacks.UnhandledOsc(s, params)
	}
}
