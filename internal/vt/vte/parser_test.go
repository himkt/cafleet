// Transliterated from vte 0.15.0 src/lib.rs tests with the "std" feature (Apache-2.0 OR MIT, used
// under MIT: Copyright (c) 2016 Joe Wilm).

package vte

import (
	"bytes"
	"reflect"
	"strings"
	"testing"
)

const maxOscRaw = 1024

var oscBytes = []byte("\x1b]2;jwilm@jwilm-desk: ~/code/alacritty\x07")

type seqKind uint8

const (
	seqOsc seqKind = iota + 1
	seqCsi
	seqEsc
	seqDcsHook
	seqDcsPut
	seqPrint
	seqExecute
	seqDcsUnhook
)

type sequence struct {
	kind          seqKind
	osc           [][]byte
	params        [][]uint16
	intermediates []byte
	flag          bool
	action        byte
	char          rune
}

type dispatcher struct{ dispatched []sequence }

func copyParams(p *Params) [][]uint16 {
	var out [][]uint16
	for _, sub := range p.Collect() {
		out = append(out, append([]uint16(nil), sub...))
	}
	return out
}

func (d *dispatcher) OscDispatch(params [][]byte, bellTerminated bool) {
	osc := make([][]byte, len(params))
	for i, p := range params {
		osc[i] = append([]byte{}, p...)
	}
	d.dispatched = append(d.dispatched, sequence{kind: seqOsc, osc: osc, flag: bellTerminated})
}

func (d *dispatcher) CsiDispatch(params *Params, intermediates []byte, ignore bool, action byte) {
	d.dispatched = append(d.dispatched, sequence{kind: seqCsi, params: copyParams(params), intermediates: append([]byte{}, intermediates...), flag: ignore, action: action})
}

func (d *dispatcher) EscDispatch(intermediates []byte, ignore bool, b byte) {
	d.dispatched = append(d.dispatched, sequence{kind: seqEsc, intermediates: append([]byte{}, intermediates...), flag: ignore, action: b})
}

func (d *dispatcher) Hook(params *Params, intermediates []byte, ignore bool, action byte) {
	d.dispatched = append(d.dispatched, sequence{kind: seqDcsHook, params: copyParams(params), intermediates: append([]byte{}, intermediates...), flag: ignore, action: action})
}

func (d *dispatcher) Put(b byte) {
	d.dispatched = append(d.dispatched, sequence{kind: seqDcsPut, action: b})
}

func (d *dispatcher) Unhook() { d.dispatched = append(d.dispatched, sequence{kind: seqDcsUnhook}) }

func (d *dispatcher) Print(c rune) {
	d.dispatched = append(d.dispatched, sequence{kind: seqPrint, char: c})
}

func (d *dispatcher) Execute(b byte) {
	d.dispatched = append(d.dispatched, sequence{kind: seqExecute, action: b})
}

func parse(chunks ...[]byte) []sequence {
	var d dispatcher
	var p Parser
	for _, c := range chunks {
		p.Advance(&d, c)
	}
	return d.dispatched
}

func requireLen(t *testing.T, got []sequence, n int) {
	t.Helper()
	if len(got) != n {
		t.Fatalf("dispatched %d sequences, want %d: %+v", len(got), n, got)
	}
}

func requireKind(t *testing.T, s sequence, kind seqKind) {
	t.Helper()
	if s.kind != kind {
		t.Fatalf("dispatched %+v, want kind %d", s, kind)
	}
}

func requireEqual(t *testing.T, got, want sequence) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("dispatched %+v, want %+v", got, want)
	}
}

func print1(c rune) sequence  { return sequence{kind: seqPrint, char: c} }
func execute(b byte) sequence { return sequence{kind: seqExecute, action: b} }
func dcsPut(b byte) sequence  { return sequence{kind: seqDcsPut, action: b} }
func esc(i []byte, b byte) sequence {
	return sequence{kind: seqEsc, intermediates: i, action: b}
}

func TestParser_ParseOsc(t *testing.T) {
	t.Parallel()
	got := parse(oscBytes)
	requireLen(t, got, 1)
	requireKind(t, got[0], seqOsc)
	if len(got[0].osc) != 2 || !bytes.Equal(got[0].osc[0], oscBytes[2:3]) || !bytes.Equal(got[0].osc[1], oscBytes[4:len(oscBytes)-1]) {
		t.Fatalf("osc params %q", got[0].osc)
	}
}

func TestParser_ParseEmptyOsc(t *testing.T) {
	t.Parallel()
	got := parse([]byte{0x1B, 0x5D, 0x07})
	requireLen(t, got, 1)
	requireKind(t, got[0], seqOsc)
}

func TestParser_ParseOscMaxParams(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b]" + strings.Repeat(";", MaxParams+1) + "\x1b"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqOsc)
	if len(got[0].osc) != maxOscParams {
		t.Fatalf("%d osc params, want %d", len(got[0].osc), maxOscParams)
	}
	for i, p := range got[0].osc {
		if len(p) != 0 {
			t.Fatalf("osc param %d is %q, want empty", i, p)
		}
	}
}

func TestParser_OscBellTerminated(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b]11;ff/00/ff\x07"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqOsc)
	if !got[0].flag {
		t.Fatal("expected osc with bell terminator")
	}
}

func TestParser_OscC0StTerminated(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b]11;ff/00/ff\x1b\\"))
	requireLen(t, got, 2)
	requireKind(t, got[0], seqOsc)
	if got[0].flag {
		t.Fatal("expected osc with ST terminator")
	}
}

func TestParser_ParseOscWithUtf8Arguments(t *testing.T) {
	t.Parallel()
	input := []byte{
		0x0D, 0x1B, 0x5D, 0x32, 0x3B, 0x65, 0x63, 0x68, 0x6F, 0x20, 0x27, 0xC2, 0xAF, 0x5C,
		0x5F, 0x28, 0xE3, 0x83, 0x84, 0x29, 0x5F, 0x2F, 0xC2, 0xAF, 0x27, 0x20, 0x26, 0x26,
		0x20, 0x73, 0x6C, 0x65, 0x65, 0x70, 0x20, 0x31, 0x07,
	}
	got := parse(input)
	requireLen(t, got, 2)
	requireEqual(t, got[0], execute('\r'))
	requireEqual(t, got[1], sequence{kind: seqOsc, osc: [][]byte{{'2'}, input[5 : len(input)-1]}, flag: true})
}

func TestParser_OscContainingStringTerminator(t *testing.T) {
	t.Parallel()
	input := []byte("\x1b]2;\xe6\x9c\xab\x1b\\")
	got := parse(input)
	requireLen(t, got, 2)
	requireKind(t, got[0], seqOsc)
	if !bytes.Equal(got[0].osc[1], input[4:len(input)-2]) {
		t.Fatalf("osc param 1 is %q", got[0].osc[1])
	}
}

func TestParser_ExceedMaxBufferSize(t *testing.T) {
	t.Parallel()
	numBytes := maxOscRaw + 100
	inputEnd := []byte("\x07")
	got := parse([]byte("\x1b]52;s"), bytes.Repeat([]byte{'a'}, numBytes), inputEnd)
	requireLen(t, got, 1)
	requireKind(t, got[0], seqOsc)
	if len(got[0].osc) != 2 || string(got[0].osc[0]) != "52" || len(got[0].osc[1]) != numBytes+len(inputEnd) {
		t.Fatalf("osc params have lengths %d and %d", len(got[0].osc[0]), len(got[0].osc[1]))
	}
}

func TestParser_ParseCsiMaxParams(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[" + strings.Repeat("1;", MaxParams-1) + "p"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqCsi)
	if len(got[0].params) != MaxParams || got[0].flag {
		t.Fatalf("%d params, ignore %v", len(got[0].params), got[0].flag)
	}
}

func TestParser_ParseCsiParamsIgnoreLongParams(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[" + strings.Repeat("1;", MaxParams) + "p"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqCsi)
	if len(got[0].params) != MaxParams || !got[0].flag {
		t.Fatalf("%d params, ignore %v", len(got[0].params), got[0].flag)
	}
}

func TestParser_ParseCsiParamsTrailingSemicolon(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[4;m"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqCsi)
	if !reflect.DeepEqual(got[0].params, [][]uint16{{4}, {0}}) {
		t.Fatalf("params %v", got[0].params)
	}
}

func TestParser_ParseCsiParamsLeadingSemicolon(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[;4m"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqCsi)
	if !reflect.DeepEqual(got[0].params, [][]uint16{{0}, {4}}) {
		t.Fatalf("params %v", got[0].params)
	}
}

func TestParser_ParseLongCsiParam(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[9223372036854775808m"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqCsi)
	if !reflect.DeepEqual(got[0].params, [][]uint16{{0xFFFF}}) {
		t.Fatalf("params %v", got[0].params)
	}
}

func TestParser_CsiReset(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[3;1\x1b[?1049h"))
	requireLen(t, got, 1)
	requireEqual(t, got[0], sequence{kind: seqCsi, params: [][]uint16{{1049}}, intermediates: []byte("?"), action: 'h'})
}

func TestParser_CsiSubparameters(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[38:2:255:0:255;1m"))
	requireLen(t, got, 1)
	requireEqual(t, got[0], sequence{kind: seqCsi, params: [][]uint16{{38, 2, 255, 0, 255}, {1}}, intermediates: []byte{}, action: 'm'})
}

func TestParser_ParseDcsMaxParams(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1bP" + strings.Repeat("1;", MaxParams+1) + "p"))
	requireLen(t, got, 1)
	requireKind(t, got[0], seqDcsHook)
	if len(got[0].params) != MaxParams || !got[0].flag {
		t.Fatalf("%d params, ignore %v", len(got[0].params), got[0].flag)
	}
	for i, p := range got[0].params {
		if !reflect.DeepEqual(p, []uint16{1}) {
			t.Fatalf("param %d is %v, want [1]", i, p)
		}
	}
}

func TestParser_DcsReset(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[3;1\x1bP1$tx\x9c"))
	requireLen(t, got, 3)
	requireEqual(t, got[0], sequence{kind: seqDcsHook, params: [][]uint16{{1}}, intermediates: []byte("$"), action: 't'})
	requireEqual(t, got[1], dcsPut('x'))
	requireEqual(t, got[2], sequence{kind: seqDcsUnhook})
}

func TestParser_ParseDcs(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1bP0;1|17/ab\x9c"))
	requireLen(t, got, 7)
	requireKind(t, got[0], seqDcsHook)
	if !reflect.DeepEqual(got[0].params, [][]uint16{{0}, {1}}) || got[0].action != '|' {
		t.Fatalf("hook %+v", got[0])
	}
	for i, b := range []byte("17/ab") {
		requireEqual(t, got[1+i], dcsPut(b))
	}
	requireEqual(t, got[6], sequence{kind: seqDcsUnhook})
}

func TestParser_IntermediateResetOnDcsExit(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1bP=1sZZZ\x1b+\x5c"))
	requireLen(t, got, 6)
	requireKind(t, got[5], seqEsc)
	if string(got[5].intermediates) != "+" {
		t.Fatalf("intermediates %q", got[5].intermediates)
	}
}

func TestParser_EscReset(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[3;1\x1b(A"))
	requireLen(t, got, 1)
	requireEqual(t, got[0], esc([]byte("("), 'A'))
}

func TestParser_EscResetIntermediates(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[?2004l\x1b#8"))
	requireLen(t, got, 2)
	requireEqual(t, got[0], sequence{kind: seqCsi, params: [][]uint16{{2004}}, intermediates: []byte{63}, action: 'l'})
	requireEqual(t, got[1], esc([]byte{35}, 56))
}

func TestParser_ParamsBufferFilledWithSubparam(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x1b[::::::::::::::::::::::::::::::::x\x1b"))
	requireLen(t, got, 1)
	requireEqual(t, got[0], sequence{kind: seqCsi, params: [][]uint16{make([]uint16, 32)}, intermediates: []byte{}, flag: true, action: 'x'})
}

func TestParser_Unicode(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\xF0\x9F\x8E\x89_\xF0\x9F\xA6\x80\xF0\x9F\xA6\x80_\xF0\x9F\x8E\x89"))
	requireLen(t, got, 6)
	for i, c := range []rune{'🎉', '_', '🦀', '🦀', '_', '🎉'} {
		requireEqual(t, got[i], print1(c))
	}
}

func TestParser_InvalidUtf8(t *testing.T) {
	t.Parallel()
	got := parse([]byte("a\xEF\xBCb"))
	requireLen(t, got, 3)
	for i, c := range []rune{'a', '�', 'b'} {
		requireEqual(t, got[i], print1(c))
	}
}

func TestParser_PartialUtf8(t *testing.T) {
	t.Parallel()
	input := []byte("\xF0\x9F\x9A\x80")
	got := parse(input[:1], input[1:2], input[2:3], input[3:])
	requireLen(t, got, 1)
	requireEqual(t, got[0], print1('🚀'))
}

func TestParser_PartialUtf8SeparatingUtf8(t *testing.T) {
	t.Parallel()
	input := []byte("\xC4\xB8\xF0\x9F\x8E\x89")
	got := parse(input[:1], input[1:])
	requireLen(t, got, 2)
	requireEqual(t, got[0], print1('ĸ'))
	requireEqual(t, got[1], print1('🎉'))
}

func TestParser_PartialInvalidUtf8(t *testing.T) {
	t.Parallel()
	input := []byte("a\xEF\xBCb")
	got := parse(input[:1], input[1:2], input[2:3], input[3:])
	requireLen(t, got, 3)
	for i, c := range []rune{'a', '�', 'b'} {
		requireEqual(t, got[i], print1(c))
	}
}

func TestParser_PartialInvalidUtf8Split(t *testing.T) {
	t.Parallel()
	input := []byte("\xE4\xBF\x99\xB5")
	got := parse(input[:2], input[2:])
	requireEqual(t, got[0], print1('俙'))
	requireEqual(t, got[1], print1('�'))
}

func TestParser_PartialUtf8IntoEsc(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\xD8\x1b012"))
	requireLen(t, got, 4)
	requireEqual(t, got[0], print1('�'))
	requireEqual(t, got[1], esc([]byte{}, '0'))
	requireEqual(t, got[2], print1('1'))
	requireEqual(t, got[3], print1('2'))
}

func TestParser_C1s(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x00\x1f\x80\x90\x98\x9b\x9c\x9d\x9e\x9fa"))
	requireLen(t, got, 11)
	for i, b := range []byte{0, 31, 128, 144, 152, 155, 156, 157, 158, 159} {
		requireEqual(t, got[i], execute(b))
	}
	requireEqual(t, got[10], print1('a'))
}

func TestParser_ExecuteAnywhere(t *testing.T) {
	t.Parallel()
	got := parse([]byte("\x18\x1a"))
	requireLen(t, got, 2)
	requireEqual(t, got[0], execute(0x18))
	requireEqual(t, got[1], execute(0x1A))
}
