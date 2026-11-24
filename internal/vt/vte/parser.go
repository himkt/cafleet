// Transliterated from vte 0.15.0 src/lib.rs with the "std" feature (Apache-2.0 OR MIT, used under
// MIT: Copyright (c) 2016 Joe Wilm).

// Package vte is a Paul Williams ANSI parser state machine that delegates actions to a Perform.
package vte

import (
	"bytes"
	"unicode/utf8"
)

const (
	maxIntermediates = 2
	maxOscParams     = 16
)

type state uint8

const (
	stateGround state = iota
	stateCsiEntry
	stateCsiIgnore
	stateCsiIntermediate
	stateCsiParam
	stateDcsEntry
	stateDcsIgnore
	stateDcsIntermediate
	stateDcsParam
	stateDcsPassthrough
	stateEscape
	stateEscapeIntermediate
	stateOscString
	stateSosPmApcString
)

// Perform receives the actions the Parser recognizes.
type Perform interface {
	Print(c rune)
	Execute(b byte)
	Hook(params *Params, intermediates []byte, ignore bool, action byte)
	Put(b byte)
	Unhook()
	OscDispatch(params [][]byte, bellTerminated bool)
	CsiDispatch(params *Params, intermediates []byte, ignore bool, action byte)
	EscDispatch(intermediates []byte, ignore bool, b byte)
}

type Parser struct {
	state           state
	intermediates   [maxIntermediates]byte
	intermediateIdx int
	params          Params
	param           uint16
	oscRaw          []byte
	oscParams       [maxOscParams][2]int
	oscNumParams    int
	ignoring        bool
	partialUTF8     [4]byte
	partialUTF8Len  int
	oscSlices       [maxOscParams][]byte
}

func (p *Parser) intermediatesSlice() []byte { return p.intermediates[:p.intermediateIdx] }

// Advance feeds bytes through the state machine.
func (p *Parser) Advance(performer Perform, b []byte) {
	i := 0
	if p.partialUTF8Len != 0 {
		i += p.advancePartialUTF8(performer, b)
	}
	for i != len(b) {
		if p.state == stateGround {
			i += p.advanceGround(performer, b[i:])
		} else {
			p.changeState(performer, b[i])
			i++
		}
	}
}

func (p *Parser) changeState(performer Perform, b byte) {
	switch p.state {
	case stateCsiEntry:
		p.advanceCsiEntry(performer, b)
	case stateCsiIgnore:
		p.advanceCsiIgnore(performer, b)
	case stateCsiIntermediate:
		p.advanceCsiIntermediate(performer, b)
	case stateCsiParam:
		p.advanceCsiParam(performer, b)
	case stateDcsEntry:
		p.advanceDcsEntry(performer, b)
	case stateDcsIgnore:
		p.anywhere(performer, b)
	case stateDcsIntermediate:
		p.advanceDcsIntermediate(performer, b)
	case stateDcsParam:
		p.advanceDcsParam(performer, b)
	case stateDcsPassthrough:
		p.advanceDcsPassthrough(performer, b)
	case stateEscape:
		p.advanceEsc(performer, b)
	case stateEscapeIntermediate:
		p.advanceEscIntermediate(performer, b)
	case stateOscString:
		p.advanceOscString(performer, b)
	case stateSosPmApcString:
		p.anywhere(performer, b)
	case stateGround:
		panic("vte: changeState in ground state, which advance handles itself")
	default:
		panic("vte: invalid parser state")
	}
}

func isC0Execute(b byte) bool {
	return b <= 0x17 || b == 0x19 || (b >= 0x1C && b <= 0x1F)
}

func (p *Parser) advanceCsiEntry(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
		p.state = stateCsiIntermediate
	case b >= 0x30 && b <= 0x39:
		p.actionParamnext(b)
		p.state = stateCsiParam
	case b == 0x3A:
		p.actionSubparam()
		p.state = stateCsiParam
	case b == 0x3B:
		p.actionParam()
		p.state = stateCsiParam
	case b >= 0x3C && b <= 0x3F:
		p.actionCollect(b)
		p.state = stateCsiParam
	case b >= 0x40 && b <= 0x7E:
		p.actionCsiDispatch(performer, b)
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceCsiIgnore(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x3F:
	case b >= 0x40 && b <= 0x7E:
		p.state = stateGround
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceCsiIntermediate(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
	case b >= 0x30 && b <= 0x3F:
		p.state = stateCsiIgnore
	case b >= 0x40 && b <= 0x7E:
		p.actionCsiDispatch(performer, b)
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceCsiParam(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
		p.state = stateCsiIntermediate
	case b >= 0x30 && b <= 0x39:
		p.actionParamnext(b)
	case b == 0x3A:
		p.actionSubparam()
	case b == 0x3B:
		p.actionParam()
	case b >= 0x3C && b <= 0x3F:
		p.state = stateCsiIgnore
	case b >= 0x40 && b <= 0x7E:
		p.actionCsiDispatch(performer, b)
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceDcsEntry(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
		p.state = stateDcsIntermediate
	case b >= 0x30 && b <= 0x39:
		p.actionParamnext(b)
		p.state = stateDcsParam
	case b == 0x3A:
		p.actionSubparam()
		p.state = stateDcsParam
	case b == 0x3B:
		p.actionParam()
		p.state = stateDcsParam
	case b >= 0x3C && b <= 0x3F:
		p.actionCollect(b)
		p.state = stateDcsParam
	case b >= 0x40 && b <= 0x7E:
		p.actionHook(performer, b)
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceDcsIntermediate(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
	case b >= 0x30 && b <= 0x3F:
		p.state = stateDcsIgnore
	case b >= 0x40 && b <= 0x7E:
		p.actionHook(performer, b)
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceDcsParam(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
		p.state = stateDcsIntermediate
	case b >= 0x30 && b <= 0x39:
		p.actionParamnext(b)
	case b == 0x3A:
		p.actionSubparam()
	case b == 0x3B:
		p.actionParam()
	case b >= 0x3C && b <= 0x3F:
		p.state = stateDcsIgnore
	case b >= 0x40 && b <= 0x7E:
		p.actionHook(performer, b)
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceDcsPassthrough(performer Perform, b byte) {
	switch {
	case b <= 0x17 || b == 0x19 || (b >= 0x1C && b <= 0x7E):
		performer.Put(b)
	case b == 0x18 || b == 0x1A:
		performer.Unhook()
		performer.Execute(b)
		p.state = stateGround
	case b == 0x1B:
		performer.Unhook()
		p.resetParams()
		p.state = stateEscape
	case b == 0x7F:
	case b == 0x9C:
		performer.Unhook()
		p.state = stateGround
	}
}

func (p *Parser) advanceEsc(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
		p.state = stateEscapeIntermediate
	case b >= 0x30 && b <= 0x4F:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x50:
		p.resetParams()
		p.state = stateDcsEntry
	case b >= 0x51 && b <= 0x57:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x58:
		p.state = stateSosPmApcString
	case b >= 0x59 && b <= 0x5A:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x5B:
		p.resetParams()
		p.state = stateCsiEntry
	case b == 0x5C:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x5D:
		p.oscRaw = p.oscRaw[:0]
		p.oscNumParams = 0
		p.state = stateOscString
	case b >= 0x5E && b <= 0x5F:
		p.state = stateSosPmApcString
	case b >= 0x60 && b <= 0x7E:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x18 || b == 0x1A:
		performer.Execute(b)
		p.state = stateGround
	}
}

func (p *Parser) advanceEscIntermediate(performer Perform, b byte) {
	switch {
	case isC0Execute(b):
		performer.Execute(b)
	case b >= 0x20 && b <= 0x2F:
		p.actionCollect(b)
	case b >= 0x30 && b <= 0x7E:
		performer.EscDispatch(p.intermediatesSlice(), p.ignoring, b)
		p.state = stateGround
	case b == 0x7F:
	default:
		p.anywhere(performer, b)
	}
}

func (p *Parser) advanceOscString(performer Perform, b byte) {
	switch {
	case b <= 0x06 || (b >= 0x08 && b <= 0x17) || b == 0x19 || (b >= 0x1C && b <= 0x1F):
	case b == 0x07:
		p.oscEnd(performer, b)
		p.state = stateGround
	case b == 0x18 || b == 0x1A:
		p.oscEnd(performer, b)
		performer.Execute(b)
		p.state = stateGround
	case b == 0x1B:
		p.oscEnd(performer, b)
		p.resetParams()
		p.state = stateEscape
	case b == 0x3B:
		p.actionOscPutParam()
	default:
		p.oscRaw = append(p.oscRaw, b)
	}
}

func (p *Parser) anywhere(performer Perform, b byte) {
	switch b {
	case 0x18, 0x1A:
		performer.Execute(b)
		p.state = stateGround
	case 0x1B:
		p.resetParams()
		p.state = stateEscape
	}
}

func (p *Parser) actionCsiDispatch(performer Perform, b byte) {
	if p.params.isFull() {
		p.ignoring = true
	} else {
		p.params.push(p.param)
	}
	performer.CsiDispatch(&p.params, p.intermediatesSlice(), p.ignoring, b)
	p.state = stateGround
}

func (p *Parser) actionHook(performer Perform, b byte) {
	if p.params.isFull() {
		p.ignoring = true
	} else {
		p.params.push(p.param)
	}
	performer.Hook(&p.params, p.intermediatesSlice(), p.ignoring, b)
	p.state = stateDcsPassthrough
}

func (p *Parser) actionCollect(b byte) {
	if p.intermediateIdx == maxIntermediates {
		p.ignoring = true
	} else {
		p.intermediates[p.intermediateIdx] = b
		p.intermediateIdx++
	}
}

func (p *Parser) actionSubparam() {
	if p.params.isFull() {
		p.ignoring = true
	} else {
		p.params.extend(p.param)
		p.param = 0
	}
}

func (p *Parser) actionParam() {
	if p.params.isFull() {
		p.ignoring = true
	} else {
		p.params.push(p.param)
		p.param = 0
	}
}

func (p *Parser) actionParamnext(b byte) {
	if p.params.isFull() {
		p.ignoring = true
	} else {
		p.param = saturatingAddU16(saturatingMulU16(p.param, 10), uint16(b-'0'))
	}
}

func saturatingMulU16(a, b uint16) uint16 {
	r := uint32(a) * uint32(b)
	if r > 0xFFFF {
		return 0xFFFF
	}
	return uint16(r)
}

func saturatingAddU16(a, b uint16) uint16 {
	r := uint32(a) + uint32(b)
	if r > 0xFFFF {
		return 0xFFFF
	}
	return uint16(r)
}

func (p *Parser) actionOscPutParam() {
	idx := len(p.oscRaw)
	paramIdx := p.oscNumParams
	switch paramIdx {
	case 0:
		p.oscParams[paramIdx] = [2]int{0, idx}
	case maxOscParams:
		return
	default:
		begin := p.oscParams[paramIdx-1][1]
		p.oscParams[paramIdx] = [2]int{begin, idx}
	}
	p.oscNumParams++
}

func (p *Parser) oscEnd(performer Perform, b byte) {
	p.actionOscPutParam()
	p.oscDispatch(performer, b)
	p.oscRaw = p.oscRaw[:0]
	p.oscNumParams = 0
}

func (p *Parser) resetParams() {
	p.intermediateIdx = 0
	p.ignoring = false
	p.param = 0
	p.params.clear()
}

func (p *Parser) oscDispatch(performer Perform, b byte) {
	slices := p.oscSlices[:p.oscNumParams]
	for i := range slices {
		idx := p.oscParams[i]
		slices[i] = p.oscRaw[idx[0]:idx[1]:idx[1]]
	}
	performer.OscDispatch(slices, b == 0x07)
}

// validateUTF8 mirrors Rust's core::str::from_utf8 error reporting: it returns the length of the
// valid prefix and the length of the invalid sequence that follows it, where errLen 0 means the
// input ends inside an incomplete but so-far valid sequence (Rust's `error_len() == None`).
func validateUTF8(b []byte) (validUpTo int, errLen int, ok bool) {
	i := 0
	n := len(b)
	for i < n {
		first := b[i]
		if first < 0x80 {
			i++
			continue
		}
		var w int
		switch {
		case first >= 0xC2 && first <= 0xDF:
			w = 2
		case first >= 0xE0 && first <= 0xEF:
			w = 3
		case first >= 0xF0 && first <= 0xF4:
			w = 4
		default:
			return i, 1, false
		}
		if i+1 >= n {
			return i, 0, false
		}
		second := b[i+1]
		switch w {
		case 2:
			if !isCont(second) {
				return i, 1, false
			}
		case 3:
			valid := (first == 0xE0 && second >= 0xA0 && second <= 0xBF) ||
				(first >= 0xE1 && first <= 0xEC && isCont(second)) ||
				(first == 0xED && second >= 0x80 && second <= 0x9F) ||
				(first >= 0xEE && first <= 0xEF && isCont(second))
			if !valid {
				return i, 1, false
			}
			if i+2 >= n {
				return i, 0, false
			}
			if !isCont(b[i+2]) {
				return i, 2, false
			}
		case 4:
			valid := (first == 0xF0 && second >= 0x90 && second <= 0xBF) ||
				(first >= 0xF1 && first <= 0xF3 && isCont(second)) ||
				(first == 0xF4 && second >= 0x80 && second <= 0x8F)
			if !valid {
				return i, 1, false
			}
			if i+2 >= n {
				return i, 0, false
			}
			if !isCont(b[i+2]) {
				return i, 2, false
			}
			if i+3 >= n {
				return i, 0, false
			}
			if !isCont(b[i+3]) {
				return i, 3, false
			}
		}
		i += w
	}
	return n, 0, true
}

func isCont(b byte) bool { return b&0xC0 == 0x80 }

func (p *Parser) advanceGround(performer Perform, b []byte) int {
	numBytes := len(b)
	plainChars := bytes.IndexByte(b, 0x1B)
	if plainChars < 0 {
		plainChars = numBytes
	}
	if plainChars == 0 {
		p.state = stateEscape
		p.resetParams()
		return 1
	}
	validUpTo, errLen, ok := validateUTF8(b[:plainChars])
	if ok {
		groundDispatch(performer, b[:plainChars])
		processed := plainChars
		if processed < numBytes {
			p.state = stateEscape
			p.resetParams()
			processed++
		}
		return processed
	}
	groundDispatch(performer, b[:validUpTo])
	if errLen != 0 {
		if errLen == 1 && b[validUpTo] <= 0x9F {
			performer.Execute(b[validUpTo])
		} else {
			performer.Print('�')
		}
		return validUpTo + errLen
	}
	if plainChars < numBytes {
		performer.Print('�')
		p.state = stateEscape
		p.resetParams()
		return plainChars + 1
	}
	extra := numBytes - validUpTo
	partialLen := p.partialUTF8Len + extra
	copy(p.partialUTF8[p.partialUTF8Len:partialLen], b[validUpTo:validUpTo+extra])
	p.partialUTF8Len = partialLen
	return numBytes
}

func (p *Parser) advancePartialUTF8(performer Perform, b []byte) int {
	oldBytes := p.partialUTF8Len
	toCopy := min(len(b), len(p.partialUTF8)-oldBytes)
	copy(p.partialUTF8[oldBytes:oldBytes+toCopy], b[:toCopy])
	p.partialUTF8Len += toCopy

	buf := p.partialUTF8[:p.partialUTF8Len]
	validUpTo, errLen, ok := validateUTF8(buf)
	if ok {
		c, size := utf8.DecodeRune(buf)
		performer.Print(c)
		p.partialUTF8Len = 0
		return size - oldBytes
	}
	if validUpTo > 0 {
		c, _ := utf8.DecodeRune(buf[:validUpTo])
		performer.Print(c)
		p.partialUTF8Len = 0
		return validUpTo - oldBytes
	}
	if errLen != 0 {
		performer.Print('�')
		p.partialUTF8Len = 0
		return errLen - oldBytes
	}
	return toCopy
}

func groundDispatch(performer Perform, text []byte) {
	for i := 0; i < len(text); {
		c, size := utf8.DecodeRune(text[i:])
		i += size
		if c <= 0x1F || (c >= 0x80 && c <= 0x9F) {
			performer.Execute(byte(c & 0xFF))
		} else {
			performer.Print(c)
		}
	}
}
