package vt

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"testing"
)

type titleCallbacks struct {
	NopCallbacks
	title, icon []byte
}

func (c *titleCallbacks) SetWindowTitle(_ *Screen, b []byte)    { c.title = append(c.title[:0], b...) }
func (c *titleCallbacks) SetWindowIconName(_ *Screen, b []byte) { c.icon = append(c.icon[:0], b...) }

type replayStep struct {
	Hex    *string  `json:"hex"`
	Resize []uint16 `json:"resize"`
}

type replayCase struct {
	Name  string       `json:"name"`
	Rows  uint16       `json:"rows"`
	Cols  uint16       `json:"cols"`
	Steps []replayStep `json:"steps"`
}

type observationStep struct {
	bytes      []byte
	resize     bool
	rows, cols uint16
}

func isASCIIControl(b byte) bool { return b < 0x20 || b == 0x7F }

// observationSteps splits a case the way cafleet's TerminalState observes a chunk: every ASCII
// control byte alone, and every run of other bytes between them.
func observationSteps(t *testing.T, c *replayCase) []observationStep {
	t.Helper()
	var out []observationStep
	for _, s := range c.Steps {
		switch {
		case s.Hex != nil:
			b, err := hex.DecodeString(*s.Hex)
			if err != nil {
				t.Fatalf("%s: %v", c.Name, err)
			}
			start := 0
			for i, x := range b {
				if isASCIIControl(x) {
					if i > start {
						out = append(out, observationStep{bytes: b[start:i]})
					}
					out = append(out, observationStep{bytes: b[i : i+1]})
					start = i + 1
				}
			}
			if start < len(b) {
				out = append(out, observationStep{bytes: b[start:]})
			}
		case len(s.Resize) == 2:
			out = append(out, observationStep{resize: true, rows: s.Resize[0], cols: s.Resize[1]})
		default:
			t.Fatalf("%s: a step is hex or a two-element resize", c.Name)
		}
	}
	return out
}

func appendColor(out []byte, c Color) []byte {
	switch c.Kind {
	case ColorDefault:
		return append(out, 'd')
	case ColorIdx:
		return strconv.AppendUint(append(out, 'i'), uint64(c.R), 10)
	case ColorRgb:
		return fmt.Appendf(out, "r%02x%02x%02x", c.R, c.G, c.B)
	default:
		panic("invalid color kind")
	}
}

func b2i(b bool) int {
	if b {
		return 1
	}
	return 0
}

func modeBits(b, d, i, u, v bool) int {
	return b2i(b) | b2i(d)<<1 | b2i(i)<<2 | b2i(u)<<3 | b2i(v)<<4
}

// canonicalDump is migration/vtoracle's dump: size, cursor, modes, current attrs, raw title and
// icon, contents, and every row with its wrap flag and non-plain cells.
func canonicalDump(s *Screen, cb *titleCallbacks, out []byte) []byte {
	r, c := s.Size()
	cr, cc := s.CursorPosition()
	out = fmt.Appendf(out, "size %d %d\ncursor %d %d\n", r, c, cr, cc)
	out = fmt.Appendf(out, "modes %d %d %d %d %d %d %d %d\n", b2i(s.AlternateScreen()), b2i(s.BracketedPaste()),
		b2i(s.HideCursor()), b2i(s.ApplicationKeypad()), b2i(s.ApplicationCursor()),
		s.MouseProtocolMode(), s.MouseProtocolEncoding(), s.CursorStyle())
	a := s.Attrs()
	out = append(out, "attrs "...)
	out = appendColor(out, a.Fg)
	out = append(out, ' ')
	out = appendColor(out, a.Bg)
	out = fmt.Appendf(out, " %02x\n", modeBits(a.Bold(), a.Dim(), a.Italic(), a.Underline(), a.Inverse()))
	out = append(out, "title "...)
	out = hex.AppendEncode(out, cb.title)
	out = append(out, " icon "...)
	out = hex.AppendEncode(out, cb.icon)
	out = append(out, "\ncontents "...)
	out = hex.AppendEncode(out, []byte(s.Contents()))
	out = append(out, '\n')
	for i, text := range s.Rows(0, c) {
		row := colIndex(i)
		out = fmt.Appendf(out, "row %d %d ", row, b2i(s.RowWrapped(row)))
		out = hex.AppendEncode(out, []byte(text))
		out = append(out, '\n')
		for col := range c {
			cell := s.Cell(row, col)
			if cell == nil {
				panic("cell in range")
			}
			bits := modeBits(cell.Bold(), cell.Dim(), cell.Italic(), cell.Underline(), cell.Inverse())
			plain := !cell.HasContents() && !cell.IsWide() && !cell.IsWideContinuation() &&
				cell.FgColor().Kind == ColorDefault && cell.BgColor().Kind == ColorDefault && bits == 0
			if plain {
				continue
			}
			out = fmt.Appendf(out, "cell %d %d %d%d%d ", row, col, b2i(cell.HasContents()), b2i(cell.IsWide()), b2i(cell.IsWideContinuation()))
			out = appendColor(out, cell.FgColor())
			out = append(out, ' ')
			out = appendColor(out, cell.BgColor())
			out = fmt.Appendf(out, " %02x ", bits)
			out = hex.AppendEncode(out, cell.ContentsBytes())
			out = append(out, '\n')
		}
	}
	return out
}

func fnv1a(b []byte) uint64 {
	h := uint64(0xcbf29ce484222325)
	for _, x := range b {
		h ^= uint64(x)
		h *= 0x100000001b3
	}
	return h
}

// applyStep reports false when the step panics; vt100 0.18.1 panics on some one-column grids and
// the oracle records the same step as PANIC.
func applyStep(p *Parser, s observationStep) (ok bool) {
	defer func() {
		if recover() != nil {
			ok = false
		}
	}()
	if s.resize {
		p.SetSize(s.rows, s.cols)
	} else {
		p.Process(s.bytes)
	}
	return true
}

// replay calls each after every observation step, with a nil parser for a panicking step, after
// which the case ends; each returns false to stop early.
func replay(t *testing.T, c *replayCase, each func(i int, p *Parser, cb *titleCallbacks) bool) {
	t.Helper()
	cb := &titleCallbacks{}
	p := NewParser(c.Rows, c.Cols, cb)
	for i, s := range observationSteps(t, c) {
		if !applyStep(p, s) {
			each(i, nil, cb)
			return
		}
		if !each(i, p, cb) {
			return
		}
	}
}

// TestParser_OracleParity is the Go side of migration/diffrun.py's emulator comparison. With
// VTPARITY_MODE hash, dump or final it reads the cases in VTPARITY_INPUT and writes to
// VTPARITY_OUT exactly what `vtoracle MODE INPUT ARGS` prints.
func TestParser_OracleParity(t *testing.T) {
	t.Parallel()
	mode := os.Getenv("VTPARITY_MODE")
	switch mode {
	case "":
		t.Skip("VTPARITY_MODE is unset; migration/diffrun.py drives this test")
	case "hash", "dump", "final":
	default:
		t.Skipf("VTPARITY_MODE %s belongs to the width parity test", mode)
	}
	input, err := os.ReadFile(os.Getenv("VTPARITY_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var dumpName string
	var dumpStep int
	if mode == "dump" {
		args := strings.Fields(os.Getenv("VTPARITY_ARGS"))
		if len(args) != 2 {
			t.Fatalf("VTPARITY_ARGS %q is not <case name> <step>", os.Getenv("VTPARITY_ARGS"))
		}
		dumpName = args[0]
		if dumpStep, err = strconv.Atoi(args[1]); err != nil {
			t.Fatal(err)
		}
	}
	var out, buf []byte
	for line := range bytes.Lines(input) {
		if len(bytes.TrimSpace(line)) == 0 {
			continue
		}
		var c replayCase
		if err := json.Unmarshal(line, &c); err != nil {
			t.Fatal(err)
		}
		switch mode {
		case "hash":
			out = append(out, c.Name...)
			replay(t, &c, func(_ int, p *Parser, cb *titleCallbacks) bool {
				if p == nil {
					out = append(out, " PANIC"...)
					return false
				}
				buf = canonicalDump(p.Screen(), cb, buf[:0])
				out = fmt.Appendf(out, " %016x", fnv1a(buf))
				return true
			})
			out = append(out, '\n')
		case "dump":
			if c.Name != dumpName {
				continue
			}
			replay(t, &c, func(i int, p *Parser, cb *titleCallbacks) bool {
				if i < dumpStep {
					return true
				}
				if p == nil {
					out = append(out, "PANIC\n"...)
				} else {
					out = canonicalDump(p.Screen(), cb, out)
				}
				return false
			})
		case "final":
			var last []byte
			replay(t, &c, func(_ int, p *Parser, cb *titleCallbacks) bool {
				if p == nil {
					last = append(last[:0], "PANIC\n"...)
					return false
				}
				last = canonicalDump(p.Screen(), cb, last[:0])
				return true
			})
			out = fmt.Appendf(out, "=== %s\n", c.Name)
			out = append(out, last...)
		}
	}
	if err := os.WriteFile(os.Getenv("VTPARITY_OUT"), out, 0o600); err != nil {
		t.Fatal(err)
	}
}
