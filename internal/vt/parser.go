// Transliterated from vt100 0.18.1 src/parser.rs (MIT, Copyright (c) 2016 Jesse Luehrs).

// Package vt is a Go transliteration of the subset of the Rust vt100 0.18.1 crate that cafleet
// reads: a read-only in-memory screen model fed by a terminal byte stream. It never answers
// terminal queries. Scrollback capacity is fixed at 0.
package vt

import "github.com/himkt/cafleet/internal/vt/vte"

type Parser struct {
	parser vte.Parser
	screen wrappedScreen
}

// NewParser creates a parser of the given size; rows and cols must be nonzero.
func NewParser(rows, cols uint16, callbacks Callbacks) *Parser {
	if rows == 0 || cols == 0 {
		panic("vt100: terminal size must be nonzero")
	}
	if callbacks == nil {
		callbacks = NopCallbacks{}
	}
	return &Parser{screen: wrappedScreen{screen: newScreen(Size{rows, cols}), callbacks: callbacks}}
}

func (p *Parser) Process(b []byte) { p.parser.Advance(&p.screen, b) }

// SetSize resizes both grids; rows and cols must be nonzero.
func (p *Parser) SetSize(rows, cols uint16) {
	if rows == 0 || cols == 0 {
		panic("vt100: terminal size must be nonzero")
	}
	size := Size{rows, cols}
	p.screen.screen.grid.setSize(size, noRow)
	p.screen.screen.alternateGrid.setSize(size, noRow)
}

// Screen returns the live screen as a read-only view: every exported Screen and Cell method only
// reads, and the view reflects each later Process or SetSize. Use Screen.CopyInto with a reused
// destination for a snapshot that must outlive the next step.
func (p *Parser) Screen() *Screen { return &p.screen.screen }

func (p *Parser) Callbacks() Callbacks { return p.screen.callbacks }
