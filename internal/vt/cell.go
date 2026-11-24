// Transliterated from vt100 0.18.1 src/attrs.rs and src/cell.rs (MIT, Copyright (c) 2016 Jesse Luehrs).

package vt

import (
	"unicode/utf8"

	"github.com/himkt/cafleet/internal/vt/unicodewidth"
)

type ColorKind uint8

const (
	ColorDefault ColorKind = iota
	ColorIdx
	ColorRgb
)

// Color is a cell foreground or background color; Idx uses R as the index.
type Color struct {
	Kind    ColorKind
	R, G, B uint8
}

func Idx(i uint8) Color { return Color{Kind: ColorIdx, R: i} }

func Rgb(r, g, b uint8) Color { return Color{Kind: ColorRgb, R: r, G: g, B: b} }

const (
	textModeIntensity uint8 = 0b0000_0011
	textModeBold      uint8 = 0b0000_0001
	textModeDim       uint8 = 0b0000_0010
	textModeItalic    uint8 = 0b0000_0100
	textModeUnderline uint8 = 0b0000_1000
	textModeInverse   uint8 = 0b0001_0000
)

type Attrs struct {
	Fg, Bg Color
	Mode   uint8
}

func (a Attrs) Bold() bool      { return a.Mode&textModeBold != 0 }
func (a Attrs) Dim() bool       { return a.Mode&textModeDim != 0 }
func (a Attrs) Italic() bool    { return a.Mode&textModeItalic != 0 }
func (a Attrs) Underline() bool { return a.Mode&textModeUnderline != 0 }
func (a Attrs) Inverse() bool   { return a.Mode&textModeInverse != 0 }

func (a *Attrs) setBold()            { a.Mode = a.Mode&^textModeIntensity | textModeBold }
func (a *Attrs) setDim()             { a.Mode = a.Mode&^textModeIntensity | textModeDim }
func (a *Attrs) setNormalIntensity() { a.Mode &^= textModeIntensity }

func (a *Attrs) setFlag(flag uint8, on bool) {
	if on {
		a.Mode |= flag
	} else {
		a.Mode &^= flag
	}
}

const (
	contentBytes       = 22
	isWideBit    uint8 = 0b1000_0000
	isWideCont   uint8 = 0b0100_0000
	lenBits      uint8 = 0b0001_1111
)

// Cell is one terminal cell: a base scalar plus zero-width scalars, the wide flags and attributes.
type Cell struct {
	contents [contentBytes]byte
	len      uint8
	attrs    Attrs
}

func (c *Cell) byteLen() int { return int(c.len & lenBits) }

func charWidthOr1(r rune) int {
	if w, ok := unicodewidth.CharWidth(r); ok {
		return w
	}
	return 1
}

func (c *Cell) set(r rune, a Attrs) {
	c.len = 0
	c.appendChar(0, r)
	c.setWide(charWidthOr1(r) > 1)
	c.attrs = a
}

func (c *Cell) append(r rune) {
	n := c.byteLen()
	if n >= contentBytes-4 {
		return
	}
	if n == 0 {
		c.contents[0] = ' '
		c.len++
	}
	c.appendChar(c.byteLen(), r)
}

func (c *Cell) setContents(s []byte, a Attrs) {
	if len(s) == 0 {
		return
	}
	r, size := utf8.DecodeRune(s)
	c.set(r, a)
	for i := size; i < len(s); {
		r, size = utf8.DecodeRune(s[i:])
		c.append(r)
		i += size
	}
}

func (c *Cell) appendChar(start int, r rune) {
	n := utf8.EncodeRune(c.contents[start:], r)
	if n < 1 || n > utf8.UTFMax {
		panic("vt: a rune encodes to 1 to 4 bytes")
	}
	c.len += uint8(n)
	if r == 0xFE0F {
		c.setWide(unicodewidth.StrWidth(c.contents[:c.byteLen()]) > 1)
	}
}

func (c *Cell) clear(a Attrs) {
	c.len = 0
	c.attrs = a
}

// Contents returns the cell text: at most one scalar of nonzero width plus combining scalars.
func (c *Cell) Contents() string { return string(c.contents[:c.byteLen()]) }

// ContentsBytes returns the cell text without allocating; the slice aliases the cell.
func (c *Cell) ContentsBytes() []byte { return c.contents[:c.byteLen()] }

func (c *Cell) HasContents() bool        { return c.byteLen() > 0 }
func (c *Cell) IsWide() bool             { return c.len&isWideBit != 0 }
func (c *Cell) IsWideContinuation() bool { return c.len&isWideCont != 0 }

func (c *Cell) setWide(on bool) {
	if on {
		c.len |= isWideBit
	} else {
		c.len &^= isWideBit
	}
}

func (c *Cell) setWideContinuation(on bool) {
	if on {
		c.len |= isWideCont
	} else {
		c.len &^= isWideCont
	}
}

func (c *Cell) Attrs() Attrs    { return c.attrs }
func (c *Cell) FgColor() Color  { return c.attrs.Fg }
func (c *Cell) BgColor() Color  { return c.attrs.Bg }
func (c *Cell) Bold() bool      { return c.attrs.Bold() }
func (c *Cell) Dim() bool       { return c.attrs.Dim() }
func (c *Cell) Italic() bool    { return c.attrs.Italic() }
func (c *Cell) Underline() bool { return c.attrs.Underline() }
func (c *Cell) Inverse() bool   { return c.attrs.Inverse() }
