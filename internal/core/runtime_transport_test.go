package core

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math"
	"math/rand/v2"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"

	"github.com/himkt/cafleet/internal/vt"
)

// terminalTitles records the raw window title and icon name the screen model reports.
type terminalTitles struct {
	vt.NopCallbacks
	title, icon []byte
}

func (c *terminalTitles) SetWindowTitle(_ *vt.Screen, title []byte) {
	c.title = append(c.title[:0], title...)
}

func (c *terminalTitles) SetWindowIconName(_ *vt.Screen, name []byte) {
	c.icon = append(c.icon[:0], name...)
}

// newTitledTerminalState is a terminalState whose screen model reports its title events to the
// returned recorder.
func newTitledTerminalState(t testing.TB, size terminalSize) (*terminalState, *terminalTitles) {
	t.Helper()
	terminal, err := newTerminalState(size)
	if err != nil {
		t.Fatalf("newTerminalState(%+v): %v", size, err)
	}
	titles := &terminalTitles{}
	terminal.parser = vt.NewParser(size.rows, screenCols(size), titles)
	return terminal, titles
}

func mustTerminalState(t testing.TB, size terminalSize) *terminalState {
	t.Helper()
	terminal, err := newTerminalState(size)
	if err != nil {
		t.Fatalf("newTerminalState(%+v): %v", size, err)
	}
	return terminal
}

func mustFeed(t testing.TB, terminal *terminalState, bytes []byte) {
	t.Helper()
	if err := terminal.feed(bytes); err != nil {
		t.Fatalf("feed(%q): %v", bytes, err)
	}
}

func mustResize(t testing.TB, terminal *terminalState, size terminalSize) {
	t.Helper()
	if err := terminal.resize(size); err != nil {
		t.Fatalf("resize(%+v): %v", size, err)
	}
}

type observedScreen struct {
	screenSeq uint64
	contents  string
}

// transportTestObserver records every observed screen; fail refuses each observation, and
// ordering logs each observation as "observe".
type transportTestObserver struct {
	screens  []observedScreen
	fail     bool
	ordering *[]string
}

func (o *transportTestObserver) observe(screen *screenSnapshot) error {
	if o.ordering != nil {
		*o.ordering = append(*o.ordering, "observe")
	}
	o.screens = append(o.screens, observedScreen{screenSeq: screen.screenSeq, contents: screen.screen.Contents()})
	if o.fail {
		return appError("fixture observation queue unavailable")
	}
	return nil
}

func (o *transportTestObserver) parserFailure() error {
	return nil
}

func (o *transportTestObserver) inputHealth() observationHealth {
	panic("the transport fixture observer is never asked to quiesce input")
}

// feedObserved is the Rust test-only TerminalState::feed_observed: it reports either failure
// class as its cause.
func (s *terminalState) feedObserved(bytes []byte, observer screenObserver) error {
	switch failure := s.feedObservedClassified(bytes, observer).(type) {
	case nil:
		return nil
	case terminalObservationErrorParser:
		return failure.cause
	case terminalObservationErrorObserver:
		return failure.cause
	default:
		panic("invalid terminalObservationError")
	}
}

func chunks(data []byte, width int) [][]byte {
	if width <= 0 {
		panic("chunk width must be positive")
	}
	var out [][]byte
	for len(data) > width {
		out = append(out, data[:width])
		data = data[width:]
	}
	if len(data) > 0 {
		out = append(out, data)
	}
	return out
}

func cursorAt(row, col uint16) opt[[2]uint16] {
	return some([2]uint16{row, col})
}

func TestRuntimeTransport_RunnerTerminalFragmentedUtf8AndAnsiReconstructTheSameLiveScreen(t *testing.T) {
	t.Parallel()
	bytes := []byte("\x1b[2J\x1b[H許可しますか？ ✓\r\n> prompt")
	size := terminalSize{rows: 24, cols: 80}
	whole, wholeTitles := newTitledTerminalState(t, size)
	mustFeed(t, whole, bytes)
	expected := whole.snapshot()
	for width := 1; width <= len(bytes); width++ {
		fragmented, fragmentedTitles := newTitledTerminalState(t, size)
		for _, part := range chunks(bytes, width) {
			mustFeed(t, fragmented, part)
		}
		actual := fragmented.snapshot()
		if got, want := actual.screen.Contents(), expected.screen.Contents(); got != want {
			t.Fatalf("width=%d: contents %q, want %q", width, got, want)
		}
		if actual.cursor != expected.cursor {
			t.Fatalf("width=%d: cursor %v, want %v", width, actual.cursor, expected.cursor)
		}
		if got, want := lossyUTF8(fragmentedTitles.title), lossyUTF8(wholeTitles.title); got != want {
			t.Fatalf("width=%d: title %q, want %q", width, got, want)
		}
	}
}

func TestRuntimeTransport_RunnerTerminalPreservesPrimaryAndAlternateScreensAndFragmentedTitles(t *testing.T) {
	t.Parallel()
	terminal, titles := newTitledTerminalState(t, terminalSize{rows: 10, cols: 40})
	mustFeed(t, terminal, []byte("primary composer"))
	primary := terminal.snapshot()
	for _, b := range []byte("\x1b]0;Action Required\x1b\\\x1b[?1049h\x1b[2J\x1b[Halternate request") {
		mustFeed(t, terminal, []byte{b})
	}
	alternate := terminal.snapshot()
	if alternate.alternateScreen != some(true) {
		t.Fatalf("alternate screen %v, want some(true)", alternate.alternateScreen)
	}
	if !strings.Contains(alternate.screen.Contents(), "alternate request") {
		t.Fatalf("alternate contents %q lack the alternate request", alternate.screen.Contents())
	}
	if strings.Contains(alternate.screen.Contents(), "primary composer") {
		t.Fatalf("alternate contents %q show the primary composer", alternate.screen.Contents())
	}
	if got := lossyUTF8(titles.title); got != "Action Required" {
		t.Fatalf("title %q, want %q", got, "Action Required")
	}
	mustFeed(t, terminal, []byte("\x1b[?1049l"))
	restored := terminal.snapshot()
	if restored.alternateScreen != some(false) {
		t.Fatalf("restored alternate screen %v, want some(false)", restored.alternateScreen)
	}
	if got, want := restored.screen.Contents(), primary.screen.Contents(); got != want {
		t.Fatalf("restored contents %q, want %q", got, want)
	}
	if restored.screenSeq <= alternate.screenSeq {
		t.Fatalf("restored screenSeq %d is not after %d", restored.screenSeq, alternate.screenSeq)
	}
}

func TestRuntimeTransport_RunnerTerminalLiveSnapshotExcludesScrollbackAndKeepsPriorSnapshotsImmutable(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 3, cols: 40})
	mustFeed(t, terminal, []byte("old permission request"))
	old := terminal.snapshot()
	mustFeed(t, terminal, []byte("\r\nline two\r\nline three\r\nempty composer"))
	current := terminal.snapshot()
	if strings.Contains(current.screen.Contents(), "old permission request") {
		t.Fatalf("current contents %q include scrollback", current.screen.Contents())
	}
	if !strings.Contains(current.screen.Contents(), "empty composer") {
		t.Fatalf("current contents %q lack the composer", current.screen.Contents())
	}
	if !strings.Contains(old.screen.Contents(), "old permission request") {
		t.Fatalf("the prior snapshot changed to %q", old.screen.Contents())
	}
	if current.screenSeq <= old.screenSeq {
		t.Fatalf("current screenSeq %d is not after %d", current.screenSeq, old.screenSeq)
	}
}

func TestRuntimeTransport_RunnerTerminalCursorUpdatesAndResizeRemainConsistentWithDimensions(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, terminal, []byte("\x1b[4;7Hchoice"))
	before := terminal.snapshot()
	if before.cursor != cursorAt(3, 12) {
		t.Fatalf("cursor %v, want (3, 12)", before.cursor)
	}
	mustResize(t, terminal, terminalSize{rows: 40, cols: 120})
	resized := terminal.snapshot()
	if resized.rows != 40 || resized.cols != 120 {
		t.Fatalf("resized to %dx%d, want 40x120", resized.rows, resized.cols)
	}
	if !strings.Contains(resized.screen.Contents(), "choice") {
		t.Fatalf("resized contents %q lost the text", resized.screen.Contents())
	}
	if resized.screenSeq < before.screenSeq {
		t.Fatalf("resized screenSeq %d is before %d", resized.screenSeq, before.screenSeq)
	}
}

func TestRuntimeTransport_RunnerTerminalRejectsEmptyDimensionsAtCreationAndResize(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		size terminalSize
	}{
		{name: "zero rows", size: terminalSize{rows: 0, cols: 80}},
		{name: "zero cols", size: terminalSize{rows: 24, cols: 0}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if _, err := newTerminalState(tc.size); err == nil || err.Error() != "terminal dimensions must be positive" {
				t.Fatalf("%s: creation error %v, want %q", tc.name, err, "terminal dimensions must be positive")
			}
			terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
			before := terminal.snapshot().screenSeq
			if err := terminal.resize(tc.size); err == nil || err.Error() != "terminal dimensions must be positive" {
				t.Fatalf("%s: resize error %v, want %q", tc.name, err, "terminal dimensions must be positive")
			}
			snapshot := terminal.snapshot()
			if snapshot.rows != 24 || snapshot.cols != 80 {
				t.Fatalf("%s: a refused resize changed the size to %dx%d", tc.name, snapshot.rows, snapshot.cols)
			}
			if snapshot.screenSeq != before {
				t.Fatalf("%s: a refused resize advanced screenSeq to %d", tc.name, snapshot.screenSeq)
			}
		})
	}
}

var emojiWithTwoPresentationSelectors = []byte("\xe2\x98\xba\xef\xb8\x8f\xef\xb8\x8f")

func TestRuntimeTransport_RunnerTerminalModelsAOneColumnTerminalAsTwoColumnsAtCreationAndEveryResize(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		created terminalSize
		resizes []terminalSize
	}{
		{
			name:    "created one column wide",
			created: terminalSize{rows: 1, cols: 1},
			resizes: []terminalSize{{rows: 1, cols: 2}, {rows: 1, cols: 1}},
		},
		{
			name:    "resized down to one column",
			created: terminalSize{rows: 24, cols: 80},
			resizes: []terminalSize{{rows: 24, cols: 1}, {rows: 24, cols: 2}, {rows: 24, cols: 1}},
		},
	}
	modelled := func(size terminalSize) [2]uint16 { return [2]uint16{size.rows, max(size.cols, 2)} }
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			terminal := mustTerminalState(t, tc.created)
			created := terminal.snapshot()
			if got := [2]uint16{created.rows, created.cols}; got != modelled(tc.created) {
				t.Fatalf("%s: creation size %v, want %v", tc.name, got, modelled(tc.created))
			}
			mustFeed(t, terminal, emojiWithTwoPresentationSelectors)
			for _, size := range tc.resizes {
				before := terminal.snapshot().screenSeq
				mustResize(t, terminal, size)
				mustFeed(t, terminal, emojiWithTwoPresentationSelectors)
				mustFeed(t, terminal, []byte("\r\nlive"))
				resized := terminal.snapshot()
				if got := [2]uint16{resized.rows, resized.cols}; got != modelled(size) {
					t.Fatalf("%s: resize to %+v gives %v, want %v", tc.name, size, got, modelled(size))
				}
				if resized.screenSeq <= before {
					t.Fatalf("%s: the terminal keeps consuming output after resize to %+v", tc.name, size)
				}
			}
		})
	}
}

func TestRuntimeTransport_CaptureReplay_EveryRunnerSnapshotCarriesItsCursorAndScreenMode(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 5, cols: 20})
	mustFeed(t, terminal, []byte("\x1b[2;4Hx"))
	primary := terminal.snapshot()
	if primary.cursor != cursorAt(1, 4) {
		t.Fatalf("cursor %v, want (1, 4)", primary.cursor)
	}
	if primary.alternateScreen != some(false) {
		t.Fatalf("alternate screen %v, want some(false)", primary.alternateScreen)
	}
	mustFeed(t, terminal, []byte("\x1b[?1049h"))
	if got := terminal.snapshot().alternateScreen; got != some(true) {
		t.Fatalf("alternate screen %v, want some(true)", got)
	}
}

func TestRuntimeTransport_RunnerTransportVisitsATransientScreenBeforeALaterClearInOneFragment(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	observer := &transportTestObserver{}
	if err := terminal.feedObserved([]byte("candidate request\x1b[2J\x1b[Hcomposer"), observer); err != nil {
		t.Fatal(err)
	}
	candidate, composer := -1, -1
	for index, screen := range observer.screens {
		if candidate < 0 && strings.Contains(screen.contents, "candidate request") {
			candidate = index
		}
		if strings.Contains(screen.contents, "composer") && !strings.Contains(screen.contents, "candidate request") {
			composer = index
		}
	}
	if candidate < 0 || composer < 0 || candidate >= composer {
		t.Fatalf("candidate screen at %d, composer screen at %d in %+v", candidate, composer, observer.screens)
	}
	for index := 1; index < len(observer.screens); index++ {
		if observer.screens[index].screenSeq <= observer.screens[index-1].screenSeq {
			t.Fatalf("screenSeq does not increase at step %d: %+v", index, observer.screens)
		}
	}
}

func TestRuntimeTransport_FeedObservedClassifiedObservesEveryControlByteAloneAndEachRunBetween(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		bytes string
		seqs  []uint64
	}{
		{name: "runs around controls", bytes: "ab\r\ncd", seqs: []uint64{2, 3, 4, 6}},
		{name: "leading and trailing controls", bytes: "\x1b[2Jx\x07", seqs: []uint64{1, 5, 6}},
		{name: "delete is a control", bytes: "a\x7fb", seqs: []uint64{1, 2, 3}},
		{name: "utf-8 continuation bytes stay in their run", bytes: "許\n可", seqs: []uint64{3, 4, 7}},
		{name: "no bytes", bytes: "", seqs: nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			terminal := mustTerminalState(t, terminalSize{rows: 4, cols: 20})
			observer := &transportTestObserver{}
			if failure := terminal.feedObservedClassified([]byte(tc.bytes), observer); failure != nil {
				t.Fatalf("%s: %v", tc.name, failure)
			}
			var seqs []uint64
			for _, screen := range observer.screens {
				seqs = append(seqs, screen.screenSeq)
			}
			if fmt.Sprint(seqs) != fmt.Sprint(tc.seqs) {
				t.Fatalf("%s: observed screenSeqs %v, want %v", tc.name, seqs, tc.seqs)
			}
		})
	}
}

func TestRuntimeTransport_FeedObservedClassifiedReportsAnObserverFailureWithTheStepEndConsumed(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 4, cols: 20})
	observer := &transportTestObserver{fail: true}
	failure := terminal.feedObservedClassified([]byte("ab\ncd"), observer)
	observerFailure, ok := failure.(terminalObservationErrorObserver)
	if !ok {
		t.Fatalf("failure %#v, want an observer failure", failure)
	}
	if observerFailure.consumed != 2 {
		t.Fatalf("consumed %d, want 2 (the end of the first step)", observerFailure.consumed)
	}
	if observerFailure.cause == nil || observerFailure.cause.Error() != "fixture observation queue unavailable" {
		t.Fatalf("cause %v, want the observer's error", observerFailure.cause)
	}
	if len(observer.screens) != 1 {
		t.Fatalf("observed %d screens after the failure, want 1", len(observer.screens))
	}
	if got := terminal.snapshot(); got.screenSeq != 2 || !strings.Contains(got.screen.Contents(), "ab") || strings.Contains(got.screen.Contents(), "cd") {
		t.Fatalf("after the failure the terminal holds seq %d and %q, want only the first step", got.screenSeq, got.screen.Contents())
	}
}

func TestRuntimeTransport_TerminalSequenceExhaustionFailsBeforeTheScreenChanges(t *testing.T) {
	t.Parallel()
	const exhausted = "terminal sequence exhausted"
	t.Run("feed", func(t *testing.T) {
		t.Parallel()
		terminal := mustTerminalState(t, terminalSize{rows: 4, cols: 20})
		terminal.sequence = math.MaxUint64 - 1
		if err := terminal.feed([]byte("ab")); err == nil || err.Error() != exhausted {
			t.Fatalf("feed error %v, want %q", err, exhausted)
		}
		if got := terminal.snapshot(); got.screenSeq != math.MaxUint64-1 || strings.Contains(got.screen.Contents(), "ab") {
			t.Fatalf("a refused feed left seq %d and %q", got.screenSeq, got.screen.Contents())
		}
		mustFeed(t, terminal, []byte("a"))
		if got := terminal.snapshot().screenSeq; got != math.MaxUint64 {
			t.Fatalf("screenSeq %d, want the maximum", got)
		}
	})
	t.Run("resize", func(t *testing.T) {
		t.Parallel()
		terminal := mustTerminalState(t, terminalSize{rows: 4, cols: 20})
		terminal.sequence = math.MaxUint64
		if err := terminal.resize(terminalSize{rows: 5, cols: 30}); err == nil || err.Error() != exhausted {
			t.Fatalf("resize error %v, want %q", err, exhausted)
		}
		if got := terminal.snapshot(); got.rows != 4 || got.cols != 20 {
			t.Fatalf("a refused resize changed the size to %dx%d", got.rows, got.cols)
		}
	})
	t.Run("observed feed", func(t *testing.T) {
		t.Parallel()
		terminal := mustTerminalState(t, terminalSize{rows: 4, cols: 20})
		terminal.sequence = math.MaxUint64
		observer := &transportTestObserver{}
		failure := terminal.feedObservedClassified([]byte("a"), observer)
		parserFailure, ok := failure.(terminalObservationErrorParser)
		if !ok {
			t.Fatalf("failure %#v, want a parser failure", failure)
		}
		if parserFailure.cause == nil || parserFailure.cause.Error() != exhausted {
			t.Fatalf("cause %v, want %q", parserFailure.cause, exhausted)
		}
		if len(observer.screens) != 0 {
			t.Fatalf("a parser failure was observed: %+v", observer.screens)
		}
		if err := terminal.feedObserved([]byte("a"), observer); err == nil || err.Error() != exhausted {
			t.Fatalf("feedObserved error %v, want the parser failure's cause %q", err, exhausted)
		}
	})
}

func appendScreenColor(out []byte, color vt.Color) []byte {
	switch color.Kind {
	case vt.ColorDefault:
		return append(out, 'd')
	case vt.ColorIdx:
		return strconv.AppendUint(append(out, 'i'), uint64(color.R), 10)
	case vt.ColorRgb:
		return fmt.Appendf(out, "r%02x%02x%02x", color.R, color.G, color.B)
	default:
		panic("invalid color kind")
	}
}

func dumpBit(b bool) int {
	if b {
		return 1
	}
	return 0
}

func dumpModeBits(bold, dim, italic, underline, inverse bool) int {
	return dumpBit(bold) | dumpBit(dim)<<1 | dumpBit(italic)<<2 | dumpBit(underline)<<3 | dumpBit(inverse)<<4
}

// canonicalScreenDump is migration/vtoracle's dump: size, cursor, modes, current attrs, raw
// title and icon, contents, and every row with its wrap flag and non-plain cells.
func canonicalScreenDump(screen *vt.Screen, titles *terminalTitles, out []byte) []byte {
	rows, cols := screen.Size()
	cursorRow, cursorCol := screen.CursorPosition()
	out = fmt.Appendf(out, "size %d %d\ncursor %d %d\n", rows, cols, cursorRow, cursorCol)
	out = fmt.Appendf(out, "modes %d %d %d %d %d %d %d %d\n", dumpBit(screen.AlternateScreen()), dumpBit(screen.BracketedPaste()),
		dumpBit(screen.HideCursor()), dumpBit(screen.ApplicationKeypad()), dumpBit(screen.ApplicationCursor()),
		screen.MouseProtocolMode(), screen.MouseProtocolEncoding(), screen.CursorStyle())
	attrs := screen.Attrs()
	out = append(out, "attrs "...)
	out = appendScreenColor(out, attrs.Fg)
	out = append(out, ' ')
	out = appendScreenColor(out, attrs.Bg)
	out = fmt.Appendf(out, " %02x\n", dumpModeBits(attrs.Bold(), attrs.Dim(), attrs.Italic(), attrs.Underline(), attrs.Inverse()))
	out = append(out, "title "...)
	out = hex.AppendEncode(out, titles.title)
	out = append(out, " icon "...)
	out = hex.AppendEncode(out, titles.icon)
	out = append(out, "\ncontents "...)
	out = hex.AppendEncode(out, []byte(screen.Contents()))
	out = append(out, '\n')
	for index, text := range screen.Rows(0, cols) {
		row := uint16(index)
		out = fmt.Appendf(out, "row %d %d ", row, dumpBit(screen.RowWrapped(row)))
		out = hex.AppendEncode(out, []byte(text))
		out = append(out, '\n')
		for col := range cols {
			cell := screen.Cell(row, col)
			if cell == nil {
				panic("every cell inside the screen size exists")
			}
			bits := dumpModeBits(cell.Bold(), cell.Dim(), cell.Italic(), cell.Underline(), cell.Inverse())
			plain := !cell.HasContents() && !cell.IsWide() && !cell.IsWideContinuation() &&
				cell.FgColor().Kind == vt.ColorDefault && cell.BgColor().Kind == vt.ColorDefault && bits == 0
			if plain {
				continue
			}
			out = fmt.Appendf(out, "cell %d %d %d%d%d ", row, col, dumpBit(cell.HasContents()), dumpBit(cell.IsWide()), dumpBit(cell.IsWideContinuation()))
			out = appendScreenColor(out, cell.FgColor())
			out = append(out, ' ')
			out = appendScreenColor(out, cell.BgColor())
			out = fmt.Appendf(out, " %02x ", bits)
			out = hex.AppendEncode(out, cell.ContentsBytes())
			out = append(out, '\n')
		}
	}
	return out
}

const (
	fnvOffset = 0xcbf29ce484222325
	fnvPrime  = 0x100000001b3
)

func fnv1aAppend(hash uint64, data []byte) uint64 {
	for _, b := range data {
		hash ^= uint64(b)
		hash *= fnvPrime
	}
	return hash
}

func fnv1a(data []byte) uint64 {
	return fnv1aAppend(fnvOffset, data)
}

// The seeded fuzz families of migration/fuzz.py, generated from fixed seeds with math/rand/v2.

const (
	seededGrammarSeed   = 1
	seededRandomSeed    = 2
	seededGrammarTarget = 1_400_000
	seededRandomTarget  = 700_000
)

type seededRNG struct{ rand *rand.Rand }

func newSeededRNG(seed uint64) *seededRNG {
	return &seededRNG{rand: rand.New(rand.NewPCG(seed, seed))}
}

func (g *seededRNG) intIn(lo, hi int) int { return lo + g.rand.IntN(hi-lo+1) }

func (g *seededRNG) float() float64 { return g.rand.Float64() }

func choose[T any](g *seededRNG, items []T) T { return items[g.rand.IntN(len(items))] }

var (
	fuzzWide      = []string{"日", "本", "語", "한", "中", "😀", "✅", "🎉", "ｱ", "Ｗ"}
	fuzzZero      = []string{"\U00000301", "\U00000308", "\U0000200d", "\U0000fe0f", "\U0000fe0e", "\U000020e3", "\U0001f3fb", "\U000017d2", "\U0000ff9e", "\U00000338"}
	fuzzEmojiBase = []string{"#", "1", "*", "❤", "☺", "✌", "👍", "🏳", "🇯", "🇵", "👨", "👩", "ل", "ا", "ל", "ក"}
	fuzzNarrow    = append(strings.Split("abcXYZ 019-=_|~", ""), "é", "─", "│", "✳", "✻", "“", "Ü", "М", "…", "→")
	fuzzControls  = []string{"\r", "\n", "\b", "\t", "\x0b", "\x0c", "\x07", "\x0e", "\x0f", "\x18", "\x1a", "\x7f", "\x00"}
	fuzzDecModes  = []int{1, 6, 9, 25, 47, 1000, 1002, 1003, 1005, 1006, 1049, 2004, 7, 12, 1004, 2026, 9999}
	fuzzSGRCodes  = []int{0, 1, 2, 3, 4, 7, 22, 23, 24, 27, 30, 37, 39, 40, 47, 49, 90, 97, 100, 107, 5, 8, 9, 53, 256}
	fuzzCSIFinals = strings.Split("@ABCDEFGHJKLMPSTXbdfmrt`hlqnsuc", "")
	fuzzSizes     = [][2]int{{36, 120}, {24, 80}, {1, 1}, {1, 2}, {2, 1}, {3, 3}, {5, 10}, {10, 4}, {2, 2}, {6, 7}, {40, 200}}
)

var fuzzC1Bytes = []byte("\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f")

func fuzzC1(g *seededRNG) []byte { return []byte{choose(g, fuzzC1Bytes)} }

func fuzzNum(g *seededRNG) string {
	r := g.float()
	switch {
	case r < 0.5:
		return strconv.Itoa(g.intIn(0, 9))
	case r < 0.8:
		return strconv.Itoa(g.intIn(0, 200))
	case r < 0.95:
		return strconv.Itoa(g.intIn(0, 70000))
	default:
		return strings.Repeat("9", g.intIn(6, 12))
	}
}

func fuzzNums(g *seededRNG, count int, separator string) string {
	parts := make([]string, count)
	for index := range parts {
		parts[index] = fuzzNum(g)
	}
	return strings.Join(parts, separator)
}

func fuzzParams(g *seededRNG, maxCount int) string {
	var out strings.Builder
	for index := range g.intIn(0, maxCount) {
		if index > 0 {
			out.WriteString(choose(g, []string{";", ";", ";", ":"}))
		}
		if g.float() < 0.85 {
			out.WriteString(fuzzNum(g))
		}
	}
	return out.String()
}

func fuzzSGR(g *seededRNG) string {
	var p string
	choice := g.float()
	switch {
	case choice < 0.3:
		p = choose(g, []string{"38;5;", "48;5;"}) + fuzzNum(g)
	case choice < 0.55:
		p = choose(g, []string{"38;2;", "48;2;"}) + fuzzNums(g, g.intIn(1, 4), ";")
	case choice < 0.65:
		p = choose(g, []string{"38:2::", "38:5:", "48:2:", "4:3"}) + fuzzNums(g, g.intIn(0, 3), ":")
	case choice < 0.7:
		p = choose(g, []string{"38", "48", "38;", "48;2"})
	default:
		codes := make([]string, g.intIn(0, 5))
		for index := range codes {
			codes[index] = strconv.Itoa(choose(g, fuzzSGRCodes))
		}
		p = strings.Join(codes, ";")
	}
	return "\x1b[" + p + "m"
}

func repeatChoice(g *seededRNG, items []string, count int) string {
	var out strings.Builder
	for range count {
		out.WriteString(choose(g, items))
	}
	return out.String()
}

func fuzzTextUnit(g *seededRNG, cols int) []byte {
	r := g.float()
	switch {
	case r < 0.45:
		return []byte(repeatChoice(g, fuzzNarrow, g.intIn(1, cols+3)))
	case r < 0.65:
		return []byte(repeatChoice(g, fuzzWide, g.intIn(1, cols/2+3)))
	case r < 0.85:
		base := choose(g, append(append(append([]string{}, fuzzEmojiBase...), fuzzNarrow...), fuzzWide...))
		return []byte(base + repeatChoice(g, fuzzZero, g.intIn(1, 7)))
	default:
		position := fmt.Sprintf("\x1b[%d;%dH", g.intIn(1, 40), max(1, cols-g.intIn(0, 2)))
		first := choose(g, append(append([]string{}, fuzzWide...), fuzzEmojiBase...))
		mark := choose(g, append(append([]string{}, fuzzZero...), ""))
		last := choose(g, append(append([]string{}, fuzzWide...), fuzzNarrow...))
		return []byte(position + first + mark + last)
	}
}

func fuzzOSC(g *seededRNG) []byte {
	body := choose(g, []string{"0", "1", "2", "52", "8", "10", "11", "133", "", "0;a;b"}) + choose(g, []string{";", ""})
	for range g.intIn(0, 3) {
		body += choose(g, []string{"Claude ✳ Code", "Ü“”", strings.Repeat("x", g.intIn(0, 40)), "c;SGVsbG8=", "?", "M—", "a;b;c;d;e;f;g;h;i;j;k;l;m;n;o;p;q;r"})
	}
	data := []byte(body)
	if g.float() < 0.3 {
		position := g.intIn(0, len(data))
		inserted := choose(g, []byte{0x9C, 0x07, 0x18, 0x1A, 0x1B, 0x9B, 0x90, 0x00, 0x0A})
		data = append(data[:position:position], append([]byte{inserted}, data[position:]...)...)
	}
	terminator := choose(g, []string{"\x07", "\x1b\\", "\x9c", "\x18", "", "\x1b", "\x1b[", "\xe2\x9c\xb3\x07"})
	return append(append([]byte("\x1b]"), data...), terminator...)
}

func fuzzDCSLike(g *seededRNG) []byte {
	intro := choose(g, []string{"\x1bP", "\x1bX", "\x1b^", "\x1b_", "\x90", "\x98", "\x9e", "\x9f"})
	body := choose(g, []string{"1$r0m", "+q544e", "q#0;2;0;0;0", "tmux;\x1b\x1b[31m", "", "\xe2\x9c\xb3\xe2\x9c\xb3\xe2\x9c\xb3"})
	terminator := choose(g, []string{"\x1b\\", "\x9c", "\x07", "\x18", ""})
	return []byte(intro + body + terminator)
}

func fuzzCSI(g *seededRNG) []byte {
	r := g.float()
	switch {
	case r < 0.25:
		modes := make([]string, g.intIn(1, 3))
		for index := range modes {
			modes[index] = strconv.Itoa(choose(g, fuzzDecModes))
		}
		return []byte("\x1b[?" + strings.Join(modes, ";") + choose(g, []string{"h", "l"}))
	case r < 0.45:
		return []byte(fuzzSGR(g))
	case r < 0.55:
		intermediate := choose(g, []string{"", " ", "$", "\"", "'"})
		prefix := choose(g, []string{"", "?", ">", "<", "=", " ", "$", "?$", "!", "  ", ">4;"})
		final := choose(g, append(append([]string{}, fuzzCSIFinals...), "p", "q", "z", "y", "~"))
		return []byte("\x1b[" + prefix + fuzzParams(g, 6) + intermediate + final)
	case r < 0.6:
		return []byte("\x1b[" + fuzzNums(g, g.intIn(30, 40), ";") + choose(g, []string{"m", "H", "r", "@"}))
	case r < 0.65:
		s := "\x1b[" + fuzzParams(g, 6)
		cut := g.intIn(0, len(s))
		return []byte(s[:cut] + choose(g, []string{"\x1b", "\x18", "\n", "\u009b", "\x7f", "日", "\x1b["}))
	case r < 0.75:
		return fmt.Appendf(nil, "\x1b[%d;%dr", g.intIn(1, 4), g.intIn(1, 40))
	default:
		return []byte("\x1b[" + fuzzParams(g, 3) + choose(g, fuzzCSIFinals))
	}
}

func fuzzESC(g *seededRNG) []byte {
	return []byte("\x1b" + choose(g, []string{"7", "8", "=", ">", "M", "M", "D", "E", "c", "g", "(B", "(0", ")0", "#8", " F", "%G", "H", "N", "O", "\\", "Z", "[", "]", "P", "1", "~"}))
}

func fuzzUTF8Noise(g *seededRNG) []byte {
	r := g.float()
	switch {
	case r < 0.4:
		return choose(g, [][]byte{{0xC3}, {0xE2, 0x9C}, {0xF0, 0x9F, 0x98}, {0xE6, 0x97}})
	case r < 0.7:
		return choose(g, [][]byte{{0xFF}, {0xC0, 0xAF}, {0xED, 0xA0, 0x80}, {0xF4, 0x90, 0x80, 0x80}, {0x80}, {0xBF}, {0xF8, 0x88}})
	default:
		return fuzzC1(g)
	}
}

// seededStep is one TerminalState input: a chunk of child output, or a resize.
type seededStep struct {
	data   []byte
	resize bool
	size   terminalSize
}

type seededStream struct {
	name  string
	size  terminalSize
	steps []seededStep
}

func uint16Size(rows, cols int) terminalSize {
	if rows < 1 || cols < 1 || rows > math.MaxUint16 || cols > math.MaxUint16 {
		panic(fmt.Sprintf("generated size %dx%d is outside the terminal range", rows, cols))
	}
	return terminalSize{rows: uint16(rows), cols: uint16(cols)}
}

func seededGrammarStream(g *seededRNG, index int) (seededStream, int) {
	initial := choose(g, fuzzSizes)
	rows, cols := initial[0], initial[1]
	stream := seededStream{name: fmt.Sprintf("fuzz:grammar:%05d", index), size: uint16Size(rows, cols)}
	total := 0
	var chunk []byte
	flush := func(count int) {
		stream.steps = append(stream.steps, seededStep{data: append([]byte(nil), chunk[:count]...)})
		total += count
		chunk = append([]byte(nil), chunk[count:]...)
	}
	for range g.intIn(30, 300) {
		var piece []byte
		r := g.float()
		switch {
		case r < 0.3:
			piece = fuzzTextUnit(g, cols)
		case r < 0.45:
			piece = fuzzCSI(g)
		case r < 0.55:
			piece = []byte(choose(g, fuzzControls))
		case r < 0.62:
			piece = fuzzOSC(g)
		case r < 0.67:
			piece = fuzzDCSLike(g)
		case r < 0.75:
			piece = fuzzESC(g)
		case r < 0.85:
			piece = fuzzUTF8Noise(g)
		case r < 0.9:
			piece = append(fuzzC1(g), choose(g, []string{"[31m", "]0;t\x07", "x", "2J"})...)
		default:
			if len(chunk) > 0 {
				flush(len(chunk))
			}
			if g.float() < 0.5 {
				candidates := [][2]int{{g.intIn(1, 50), g.intIn(1, 140)}, {1, 1}, {rows, max(1, cols-1)}, {max(1, rows-1), cols}, {rows + 3, cols + 2}}
				next := choose(g, candidates)
				rows, cols = next[0], next[1]
				stream.steps = append(stream.steps, seededStep{resize: true, size: uint16Size(rows, cols)})
			}
		}
		chunk = append(chunk, piece...)
		if g.float() < 0.15 && len(chunk) > 0 {
			flush(g.intIn(0, len(chunk)))
		}
	}
	if len(chunk) > 0 {
		flush(len(chunk))
	}
	return stream, total
}

var seededRandomAlphabet = func() []byte {
	var alphabet []byte
	for range 3 {
		for b := 0x20; b < 0x7F; b++ {
			alphabet = append(alphabet, byte(b))
		}
	}
	for b := 0x00; b < 0x20; b++ {
		alphabet = append(alphabet, byte(b))
	}
	for b := 0x80; b < 0x100; b++ {
		alphabet = append(alphabet, byte(b))
	}
	for range 40 {
		alphabet = append(alphabet, 0x1B)
	}
	for range 4 {
		alphabet = append(alphabet, "[]?;:0123456789mHJK"...)
	}
	return alphabet
}()

func seededRandomStream(g *seededRNG, index int) (seededStream, int) {
	stream := seededStream{name: fmt.Sprintf("fuzz:random:%05d", index), size: uint16Size(g.intIn(1, 40), g.intIn(1, 130))}
	data := make([]byte, g.intIn(100, 4000))
	for position := range data {
		data[position] = choose(g, seededRandomAlphabet)
	}
	for position := 0; position < len(data); {
		end := min(len(data), position+g.intIn(1, 600))
		stream.steps = append(stream.steps, seededStep{data: data[position:end]})
		position = end
		if g.float() < 0.05 {
			stream.steps = append(stream.steps, seededStep{resize: true, size: uint16Size(g.intIn(1, 40), g.intIn(1, 130))})
		}
	}
	return stream, len(data)
}

func seededFamily(seed uint64, target int, generate func(*seededRNG, int) (seededStream, int)) ([]seededStream, int) {
	g := newSeededRNG(seed)
	var streams []seededStream
	total := 0
	for total < target {
		stream, count := generate(g, len(streams))
		streams = append(streams, stream)
		total += count
	}
	return streams, total
}

// seededStreams is the grammar family followed by the random family; together they carry at
// least 2 MB of child output.
func seededStreams() ([]seededStream, int) {
	grammar, grammarBytes := seededFamily(seededGrammarSeed, seededGrammarTarget, seededGrammarStream)
	random, randomBytes := seededFamily(seededRandomSeed, seededRandomTarget, seededRandomStream)
	return append(grammar, random...), grammarBytes + randomBytes
}

type seededScreenRecord struct {
	name     string
	steps    int
	final    uint64
	chain    uint64
	panicked bool
}

// seededScreenObserver chains the FNV-1a hash of each observation step's canonical dump.
type seededScreenObserver struct {
	titles *terminalTitles
	buffer []byte
	steps  int
	chain  uint64
}

func (o *seededScreenObserver) record(screen *vt.Screen) {
	o.buffer = canonicalScreenDump(screen, o.titles, o.buffer[:0])
	step := fnv1a(o.buffer)
	o.chain = fnv1aAppend(o.chain, binary.BigEndian.AppendUint64(nil, step))
	o.steps++
}

func (o *seededScreenObserver) observe(screen *screenSnapshot) error {
	o.record(screen.screen)
	return nil
}

func (o *seededScreenObserver) parserFailure() error {
	return nil
}

func (o *seededScreenObserver) inputHealth() observationHealth {
	panic("the seeded screen observer is never asked to quiesce input")
}

// replaySeededStream feeds the stream through TerminalState with its per-step split, counting a
// resize as one observation step, and returns the record and the final canonical dump.
// A step on which vt100 0.18.1 panics panics in TerminalState too; the record keeps the steps
// observed before it and marks the stream panicked, as the oracle records PANIC.
func replaySeededStream(stream seededStream) (record seededScreenRecord, dump []byte, err error) {
	titles := &terminalTitles{}
	observer := &seededScreenObserver{titles: titles, chain: fnvOffset}
	defer func() {
		if recovered := recover(); recovered != nil {
			record = seededScreenRecord{name: stream.name, steps: observer.steps, chain: observer.chain, panicked: true}
			dump = []byte(fmt.Sprintf("the screen model panicked after %d observed steps: %v\n", observer.steps, recovered))
			err = nil
		}
	}()
	terminal, err := newTerminalState(stream.size)
	if err != nil {
		return record, nil, err
	}
	terminal.parser = vt.NewParser(stream.size.rows, screenCols(stream.size), titles)
	for _, step := range stream.steps {
		if step.resize {
			if err := terminal.resize(step.size); err != nil {
				return record, nil, fmt.Errorf("%s: resize: %w", stream.name, err)
			}
			snapshot := terminal.snapshot()
			observer.record(snapshot.screen)
			continue
		}
		if failure := terminal.feedObservedClassified(step.data, observer); failure != nil {
			return record, nil, fmt.Errorf("%s: feed: %v", stream.name, failure)
		}
	}
	final := terminal.snapshot()
	dump = canonicalScreenDump(final.screen, titles, nil)
	return seededScreenRecord{name: stream.name, steps: observer.steps, final: fnv1a(dump), chain: observer.chain}, dump, nil
}

func (r seededScreenRecord) goLiteral() string {
	if r.panicked {
		return fmt.Sprintf("\t{name: %q, steps: %d, chain: 0x%016x, panicked: true},\n", r.name, r.steps, r.chain)
	}
	return fmt.Sprintf("\t{name: %q, steps: %d, final: 0x%016x, chain: 0x%016x},\n", r.name, r.steps, r.final, r.chain)
}

func TestRuntimeTransport_SeededStreamsCarryAtLeastTwoMegabytesOfChildOutput(t *testing.T) {
	t.Parallel()
	streams, total := seededStreams()
	if total < 2_000_000 {
		t.Fatalf("the seeded families carry %d bytes, below 2,000,000", total)
	}
	again, againTotal := seededStreams()
	if againTotal != total || len(again) != len(streams) {
		t.Fatalf("the seeded families are not deterministic: %d/%d bytes, %d/%d streams", againTotal, total, len(again), len(streams))
	}
	for index := range streams {
		if streams[index].name != again[index].name || fmt.Sprint(streams[index].steps) != fmt.Sprint(again[index].steps) {
			t.Fatalf("stream %s differs between generations", streams[index].name)
		}
	}
}

func TestRuntimeTransport_SeededScreens(t *testing.T) {
	t.Parallel()
	streams, _ := seededStreams()
	records := make([]seededScreenRecord, len(streams))
	dumps := make([][]byte, len(streams))
	failures := make([]error, len(streams))
	var wait sync.WaitGroup
	next := make(chan int)
	for range runtime.GOMAXPROCS(0) {
		wait.Go(func() {
			for index := range next {
				records[index], dumps[index], failures[index] = replaySeededStream(streams[index])
			}
		})
	}
	for index := range streams {
		next <- index
	}
	close(next)
	wait.Wait()
	for _, failure := range failures {
		if failure != nil {
			t.Fatal(failure)
		}
	}
	if *updateGolden {
		var body strings.Builder
		for _, record := range records {
			body.WriteString(record.goLiteral())
		}
		updateGoldenBlock(t, "runtime_transport_test.go", body.String())
		return
	}
	if len(seededScreenGolden) != len(records) {
		t.Fatalf("the golden block records %d streams, the generator produces %d; regenerate with -update-golden", len(seededScreenGolden), len(records))
	}
	mismatches := 0
	for index, record := range records {
		want := seededScreenGolden[index]
		if record == want {
			continue
		}
		mismatches++
		if mismatches <= 3 {
			t.Errorf("%s: got steps %d final %016x chain %016x, golden %s steps %d final %016x chain %016x\nfinal screen dump:\n%s",
				record.name, record.steps, record.final, record.chain, want.name, want.steps, want.final, want.chain, dumps[index])
		}
	}
	if mismatches > 0 {
		t.Fatalf("%d of %d seeded streams differ from the golden block", mismatches, len(records))
	}
}

var seededScreenGolden = []seededScreenRecord{
	// BEGIN GENERATED: mise //:test -- -run 'TestRuntimeTransport_SeededScreens' -update-golden
	{name: "fuzz:grammar:00000", steps: 75, final: 0xb1afe37f294712f2, chain: 0xb3051ace710ceadf},
	{name: "fuzz:grammar:00001", steps: 319, final: 0xbc29e754bf55115a, chain: 0xdac734afdc29414f},
	{name: "fuzz:grammar:00002", steps: 384, final: 0xbd8a8544a3a4814c, chain: 0x6b2b9ded643e39cf},
	{name: "fuzz:grammar:00003", steps: 151, final: 0xbb64bb4910af9cb4, chain: 0x6033466691379eaf},
	{name: "fuzz:grammar:00004", steps: 266, final: 0x79725009122e521c, chain: 0xd84cd2daf8f361d2},
	{name: "fuzz:grammar:00005", steps: 227, final: 0xe77d3f3a93728631, chain: 0x5f03a6128e22780f},
	{name: "fuzz:grammar:00006", steps: 320, final: 0xa684c115767b0bd5, chain: 0xff3de0d0a3479b3f},
	{name: "fuzz:grammar:00007", steps: 264, final: 0x8cd1ec9b61c739f7, chain: 0xd03a2c36fe30bd8e},
	{name: "fuzz:grammar:00008", steps: 371, final: 0xe966a48759ce1f97, chain: 0xea7e33434024aeee},
	{name: "fuzz:grammar:00009", steps: 332, final: 0x7f583eaeb0b398a8, chain: 0xc1080c7bbd19a891},
	{name: "fuzz:grammar:00010", steps: 74, chain: 0x27a832963680bf1b, panicked: true},
	{name: "fuzz:grammar:00011", steps: 363, final: 0x9e992d8fa690b9dc, chain: 0xad788b762001165e},
	{name: "fuzz:grammar:00012", steps: 228, final: 0xfc16a7fdda1c59f2, chain: 0xb362a627abd3d559},
	{name: "fuzz:grammar:00013", steps: 185, final: 0x7a2d6989b0e2237e, chain: 0xa4502f867738a2c7},
	{name: "fuzz:grammar:00014", steps: 363, final: 0xd67404091dae8de4, chain: 0x229c7502c17adfed},
	{name: "fuzz:grammar:00015", steps: 312, final: 0x24dc8100f02dfa2d, chain: 0xd8540ae7c5900d08},
	{name: "fuzz:grammar:00016", steps: 224, final: 0x21e41765d9860e6c, chain: 0xf5ea83c3a8562b07},
	{name: "fuzz:grammar:00017", steps: 269, final: 0xa4af1a1f00210e8c, chain: 0xb7fee83c8cd51f74},
	{name: "fuzz:grammar:00018", steps: 362, final: 0x26b25f33436c4100, chain: 0x5879ab1fa92682ab},
	{name: "fuzz:grammar:00019", steps: 181, final: 0x0bddf103e3e86b3c, chain: 0x152f88453eac4c33},
	{name: "fuzz:grammar:00020", steps: 53, final: 0xead73d0e9af22041, chain: 0x71bcde14cc6dbedb},
	{name: "fuzz:grammar:00021", steps: 76, final: 0x30d883a1b84cfb88, chain: 0x2b93f54695966bdd},
	{name: "fuzz:grammar:00022", steps: 272, final: 0x23eae56e1e284544, chain: 0xd2885c6852f9d833},
	{name: "fuzz:grammar:00023", steps: 100, final: 0x34adcfd3b6bc9aa1, chain: 0x8e0b90e7ce911479},
	{name: "fuzz:grammar:00024", steps: 276, final: 0x84792d959f97c1b2, chain: 0x3bc471b5eca4d0b9},
	{name: "fuzz:grammar:00025", steps: 291, final: 0x9b662eb674dec5d3, chain: 0x8efd2cb6717f73be},
	{name: "fuzz:grammar:00026", steps: 326, final: 0x0f5053042eee625b, chain: 0xc57673c70a3991c9},
	{name: "fuzz:grammar:00027", steps: 164, final: 0xd41f0311d0b22452, chain: 0xc99b4864f75f05e3},
	{name: "fuzz:grammar:00028", steps: 321, final: 0x5c56d87b1999ae10, chain: 0xe66b36c5b21333eb},
	{name: "fuzz:grammar:00029", steps: 298, final: 0x0972a0e8dc781ee3, chain: 0xc5e4fb76e5e05121},
	{name: "fuzz:grammar:00030", steps: 127, final: 0x65b6c97f693f4f4c, chain: 0xfcade73ce80ecfa3},
	{name: "fuzz:grammar:00031", steps: 300, final: 0xca8fa353fe7a83bb, chain: 0xa34ccbbbc1dbf8c5},
	{name: "fuzz:grammar:00032", steps: 100, final: 0xbd5d0f4a43e063a5, chain: 0x0da4b98794b42aed},
	{name: "fuzz:grammar:00033", steps: 353, final: 0xd4525baafd1a73c2, chain: 0x6df315fe7b3dc971},
	{name: "fuzz:grammar:00034", steps: 337, final: 0x796474ff1756298a, chain: 0xde539878e12cbd25},
	{name: "fuzz:grammar:00035", steps: 245, final: 0x97df7df7dc46b3d4, chain: 0xa801ea0aa3926825},
	{name: "fuzz:grammar:00036", steps: 99, final: 0x42417dced763dccd, chain: 0x95aeb7de7268dec0},
	{name: "fuzz:grammar:00037", steps: 262, final: 0x7f25f450524ffd28, chain: 0x917f5304cdae7a6a},
	{name: "fuzz:grammar:00038", steps: 190, final: 0x81124f533bbaf5d5, chain: 0x55cc3e5200b617dd},
	{name: "fuzz:grammar:00039", steps: 172, final: 0x333cf4212d97097a, chain: 0xc945ab378e49281a},
	{name: "fuzz:grammar:00040", steps: 335, final: 0x29c41775463ba18e, chain: 0x9204fd5929e9b24e},
	{name: "fuzz:grammar:00041", steps: 302, final: 0x8aad489f10ee8e53, chain: 0xd818217580ec006e},
	{name: "fuzz:grammar:00042", steps: 202, final: 0x336d9163d98e7f19, chain: 0x5b25899cf1227739},
	{name: "fuzz:grammar:00043", steps: 92, final: 0x6e1feb1bb69458d2, chain: 0x40f223a0f785a12f},
	{name: "fuzz:grammar:00044", steps: 345, final: 0x1ea170b523af83cd, chain: 0x7d2bcadcce594c39},
	{name: "fuzz:grammar:00045", steps: 322, final: 0xadb23ef01b2d1315, chain: 0xd1af2af5add5c253},
	{name: "fuzz:grammar:00046", steps: 228, final: 0xb13b54470f2adea6, chain: 0xc194fe29b0b60ac0},
	{name: "fuzz:grammar:00047", steps: 201, final: 0x1fddba227d3d26b3, chain: 0x4501ef3a2fba97ea},
	{name: "fuzz:grammar:00048", steps: 308, final: 0xd912d236ab766f0d, chain: 0x51e90e3d3a210b30},
	{name: "fuzz:grammar:00049", steps: 324, final: 0x93a12f9fed2207b3, chain: 0x356f20bcac3d983d},
	{name: "fuzz:grammar:00050", steps: 282, final: 0x63081fb66e73a423, chain: 0xe6c200663e97b8df},
	{name: "fuzz:grammar:00051", steps: 218, final: 0xfe28d41a1bcb0b28, chain: 0x456c130f968b9a24},
	{name: "fuzz:grammar:00052", steps: 90, final: 0xc066b0c2e6f10c30, chain: 0xafb8eae0302b66bb},
	{name: "fuzz:grammar:00053", steps: 87, final: 0x26f761b958fb2172, chain: 0x178a7e51247eabac},
	{name: "fuzz:grammar:00054", steps: 204, final: 0x53c1571389ab0ba9, chain: 0x440069e9623dba64},
	{name: "fuzz:grammar:00055", steps: 355, final: 0x29f22c27d642f669, chain: 0x85d9cd4a46049180},
	{name: "fuzz:grammar:00056", steps: 53, final: 0x6070e065cdb797d4, chain: 0x909894580090ae44},
	{name: "fuzz:grammar:00057", steps: 96, final: 0x5586a248d65a6a10, chain: 0x3f6098f54fbfbd44},
	{name: "fuzz:grammar:00058", steps: 305, final: 0x70b188aa94d2a17c, chain: 0xb36df7e459d3ffe1},
	{name: "fuzz:grammar:00059", steps: 278, final: 0x9c08c72e654f604c, chain: 0x5bf1f622e1766c23},
	{name: "fuzz:grammar:00060", steps: 251, final: 0xffb2aa60f3a65572, chain: 0xe637f5cade83c679},
	{name: "fuzz:grammar:00061", steps: 183, final: 0x6adcf7a7c1f6f9c6, chain: 0xbdb910d24c5cc60c},
	{name: "fuzz:grammar:00062", steps: 402, final: 0xb75a10e0737b78c0, chain: 0x8ccd1099399bf170},
	{name: "fuzz:grammar:00063", steps: 99, final: 0x041e8361c8d01192, chain: 0xdaa8996aab5443ea},
	{name: "fuzz:grammar:00064", steps: 284, final: 0xccceea4c39fedff9, chain: 0x73f9d5f525e10df7},
	{name: "fuzz:grammar:00065", steps: 179, final: 0x03c758165d3b59d5, chain: 0x75af96574ca47289},
	{name: "fuzz:grammar:00066", steps: 243, final: 0x1fcb2d69890407ee, chain: 0x1d93a3330e21a890},
	{name: "fuzz:grammar:00067", steps: 89, final: 0x67d051a423e41f0e, chain: 0x02aaa369c32d6b9a},
	{name: "fuzz:grammar:00068", steps: 366, final: 0xd4809e28ec51765f, chain: 0x8a53e6e4dda0a586},
	{name: "fuzz:grammar:00069", steps: 336, final: 0xbafae6612a68cd36, chain: 0x1e12e7168e163517},
	{name: "fuzz:grammar:00070", steps: 315, final: 0x37838ff9b3c90731, chain: 0x79bb5c11ddfd5fb6},
	{name: "fuzz:grammar:00071", steps: 348, final: 0xabd162a4e7b57c1d, chain: 0x38c95090aee0673a},
	{name: "fuzz:grammar:00072", steps: 357, final: 0x471858b80318cfc7, chain: 0x5ac100ec67691180},
	{name: "fuzz:grammar:00073", steps: 220, final: 0x6f1e63a5f25f0fa9, chain: 0xeab67f090c5363cf},
	{name: "fuzz:grammar:00074", steps: 342, final: 0x4838822b1c482b7e, chain: 0x46bba8f1f937f0be},
	{name: "fuzz:grammar:00075", steps: 221, final: 0x378e7cadfa7a738d, chain: 0x860598efda80c789},
	{name: "fuzz:grammar:00076", steps: 347, final: 0xbc82c935601625cc, chain: 0x56aed47693e8b042},
	{name: "fuzz:grammar:00077", steps: 375, final: 0xba039caae58f08ce, chain: 0x07f752d0b00f05a3},
	{name: "fuzz:grammar:00078", steps: 288, final: 0x2ba6ba2e9bf1dce1, chain: 0xe98d82ca29a66174},
	{name: "fuzz:grammar:00079", steps: 105, final: 0xa1c36a98bfb5304d, chain: 0x7b2576ccec7a0978},
	{name: "fuzz:grammar:00080", steps: 109, final: 0x64551f22581fb190, chain: 0xed52677f7b447b36},
	{name: "fuzz:grammar:00081", steps: 235, final: 0xa6409eca1e1f3bfb, chain: 0x925f9e399f1020c3},
	{name: "fuzz:grammar:00082", steps: 133, final: 0x95701286e810b5dc, chain: 0x9e2008ffbcfc8584},
	{name: "fuzz:grammar:00083", steps: 174, final: 0xa89732892cd04ade, chain: 0x313ed1755e1c31b5},
	{name: "fuzz:grammar:00084", steps: 270, final: 0xea07e69b758e814a, chain: 0xbb878821695b27d1},
	{name: "fuzz:grammar:00085", steps: 365, final: 0xf903c98957bb7f56, chain: 0xe0903757c14b2696},
	{name: "fuzz:grammar:00086", steps: 373, final: 0xab66500fa396d21f, chain: 0xe73016d079af7f68},
	{name: "fuzz:grammar:00087", steps: 157, final: 0x19c0dc6ffaba2de2, chain: 0xe7d332fc7ee1a69d},
	{name: "fuzz:grammar:00088", steps: 92, final: 0x23e5ab67f072144c, chain: 0xb4c5bd9d1f3f8f10},
	{name: "fuzz:grammar:00089", steps: 339, final: 0xbd611e7a0ae74eef, chain: 0x00bd7d718d7389fe},
	{name: "fuzz:grammar:00090", steps: 342, final: 0xb2a2a65278566d52, chain: 0xd16fc0a11acd830c},
	{name: "fuzz:grammar:00091", steps: 159, final: 0x9f28c456d2646771, chain: 0x97d0f520b655fbdc},
	{name: "fuzz:grammar:00092", steps: 202, final: 0xe80b7e925af074d5, chain: 0xd39ffa7b179328b8},
	{name: "fuzz:grammar:00093", steps: 227, final: 0xd4dd8a41b9f02c95, chain: 0x2409a20e6161b9bf},
	{name: "fuzz:grammar:00094", steps: 276, final: 0x93261e5fba9e0a4a, chain: 0xa0936f172224bd2f},
	{name: "fuzz:grammar:00095", steps: 256, final: 0xca306d2b0a21e9d6, chain: 0x42446aa9189c1a59},
	{name: "fuzz:grammar:00096", steps: 90, final: 0xbc04e6ff93960048, chain: 0xfe0268ba931dc776},
	{name: "fuzz:grammar:00097", steps: 41, final: 0x2ea0234d1129801a, chain: 0xb1e7a5eda2987195},
	{name: "fuzz:grammar:00098", steps: 332, final: 0x0bf9790c9958fa25, chain: 0x0ef873e299e34198},
	{name: "fuzz:grammar:00099", steps: 69, final: 0x077e8cd8a0f5857d, chain: 0x0b2fdbc24ac5e92f},
	{name: "fuzz:grammar:00100", steps: 215, final: 0xb300eab5b0a8e761, chain: 0xe52a3f6078fc5a81},
	{name: "fuzz:grammar:00101", steps: 97, final: 0x9c90b3cb38f2e667, chain: 0xdfcd2fd47eaffcd6},
	{name: "fuzz:grammar:00102", steps: 339, final: 0x3ba66b0faf5a024b, chain: 0x50e14113c300cfb8},
	{name: "fuzz:grammar:00103", steps: 170, final: 0x108090d76295cf60, chain: 0x0ac25f47cd383b67},
	{name: "fuzz:grammar:00104", steps: 236, final: 0x495eda5fb77863ad, chain: 0x33d219067195843d},
	{name: "fuzz:grammar:00105", steps: 69, final: 0xb6f08a823b2662b9, chain: 0xeb8d6e082894b99a},
	{name: "fuzz:grammar:00106", steps: 74, final: 0x4f80e24702a43064, chain: 0x556ad16ba01c0dbc},
	{name: "fuzz:grammar:00107", steps: 47, final: 0xbe00dc962ec70235, chain: 0x7fd1a1ef7f9d1d81},
	{name: "fuzz:grammar:00108", steps: 376, final: 0xac32174572ea6613, chain: 0x0783f92166eacba5},
	{name: "fuzz:grammar:00109", steps: 117, final: 0x83bcec5b885f4da4, chain: 0x1882459a2912099e},
	{name: "fuzz:grammar:00110", steps: 173, final: 0xfa10a539968f6404, chain: 0x35e34b2bcb907dbc},
	{name: "fuzz:grammar:00111", steps: 126, final: 0x35ee23df97646787, chain: 0xa4d4b832fa03c664},
	{name: "fuzz:grammar:00112", steps: 192, final: 0xfba50d20b4a6cf62, chain: 0x65f83f0ecca5326f},
	{name: "fuzz:grammar:00113", steps: 194, final: 0x4fcbc321b32ee9e2, chain: 0x04b287a0aa774123},
	{name: "fuzz:grammar:00114", steps: 305, final: 0x9ef609be7667a21e, chain: 0xf3ac6286e791a5d4},
	{name: "fuzz:grammar:00115", steps: 237, final: 0xf614ac1a04afb60e, chain: 0x480cc169bbee5862},
	{name: "fuzz:grammar:00116", steps: 241, final: 0xd18cbebde1af32ad, chain: 0xa20ef76d0c7614a8},
	{name: "fuzz:grammar:00117", steps: 197, final: 0xb1de6b9095a035a4, chain: 0x29e21602df68caca},
	{name: "fuzz:grammar:00118", steps: 232, final: 0xcd2c755232551159, chain: 0xe9f575dd4b97cceb},
	{name: "fuzz:grammar:00119", steps: 341, final: 0xaa9d2975ea76707f, chain: 0x4ef4fd8aa907ba67},
	{name: "fuzz:grammar:00120", steps: 299, final: 0xb885fa4f622eeb65, chain: 0x27c82801f03fdba5},
	{name: "fuzz:grammar:00121", steps: 264, final: 0xe182982fbc6ea2f8, chain: 0xf5ba1fcb4d932f12},
	{name: "fuzz:grammar:00122", steps: 344, final: 0x502134dfba13b8c4, chain: 0x263fb69df92e3ddd},
	{name: "fuzz:grammar:00123", steps: 224, final: 0x3381eb315aed52ca, chain: 0x6882ced3deb8892e},
	{name: "fuzz:grammar:00124", steps: 56, final: 0x7291e5558bd8f285, chain: 0x33168ed86dff3230},
	{name: "fuzz:grammar:00125", steps: 101, final: 0xc15da095b8069109, chain: 0xf19109740cfbb90b},
	{name: "fuzz:grammar:00126", steps: 261, final: 0x4120c160bb50b867, chain: 0x7b7e44e21714feea},
	{name: "fuzz:grammar:00127", steps: 98, final: 0xe8561122ccaa5685, chain: 0xf6d1ff5b122f1976},
	{name: "fuzz:grammar:00128", steps: 129, final: 0x0cedfec24f75d94f, chain: 0x124d516acaa783bf},
	{name: "fuzz:grammar:00129", steps: 237, final: 0x544ff17e4caee742, chain: 0x75f4f4c54bf73770},
	{name: "fuzz:grammar:00130", steps: 150, final: 0x8982e6b7fb5d7e5d, chain: 0x9c6de6edbd0bd94f},
	{name: "fuzz:grammar:00131", steps: 331, final: 0xb7915133a53b0021, chain: 0x7e7f9e294172c459},
	{name: "fuzz:grammar:00132", steps: 164, final: 0x0890f226886c3609, chain: 0xbdf47a293a6daf5f},
	{name: "fuzz:grammar:00133", steps: 305, final: 0xaf07bd97270257e2, chain: 0xea3ece4576d2154b},
	{name: "fuzz:grammar:00134", steps: 158, final: 0x3effcafcd6e7e156, chain: 0x8f4cbf8bd28a1ff5},
	{name: "fuzz:grammar:00135", steps: 211, final: 0x500f704442b5c432, chain: 0x92a5cea001da3191},
	{name: "fuzz:grammar:00136", steps: 330, final: 0x04b6f92eb6906e6c, chain: 0xd5368eaa0c73e3d5},
	{name: "fuzz:grammar:00137", steps: 102, final: 0x2037266c89f3796e, chain: 0xf4b9add0d8135fb4},
	{name: "fuzz:grammar:00138", steps: 66, final: 0x474dec5e0ad72e95, chain: 0xe4cd18af4a850b85},
	{name: "fuzz:grammar:00139", steps: 113, final: 0xfa2d4391c5587f24, chain: 0x59bfe813aa64105f},
	{name: "fuzz:grammar:00140", steps: 284, final: 0x8625bbf4a54da0e1, chain: 0x88a654cd6beaea2c},
	{name: "fuzz:grammar:00141", steps: 354, final: 0x3ed78837f1671955, chain: 0xe433955a7927ceca},
	{name: "fuzz:grammar:00142", steps: 70, final: 0x189abfb0531b8bd5, chain: 0xe8252579633f2efc},
	{name: "fuzz:grammar:00143", steps: 255, final: 0x8c68c49332572119, chain: 0xde0589f6a2968ee9},
	{name: "fuzz:grammar:00144", steps: 243, final: 0xcd90c9fe8d338ef9, chain: 0xb82c0144f1b61514},
	{name: "fuzz:grammar:00145", steps: 237, final: 0xf316c02ba4d166ba, chain: 0xd83941ab2dd85bcf},
	{name: "fuzz:grammar:00146", steps: 231, final: 0xeb0e8d34aeb5591a, chain: 0xa18cc55cf370e775},
	{name: "fuzz:grammar:00147", steps: 247, final: 0xd2de39be76800a34, chain: 0x5a382c0d033d89ad},
	{name: "fuzz:grammar:00148", steps: 135, final: 0xaa35fe7c56da3de4, chain: 0x260f804e79212bf4},
	{name: "fuzz:grammar:00149", steps: 221, final: 0xd9d5dea8ce9ac6bd, chain: 0x5095604e76c68e81},
	{name: "fuzz:grammar:00150", steps: 69, final: 0x3c82fd52cbd10ead, chain: 0x96b274d10a6bf066},
	{name: "fuzz:grammar:00151", steps: 95, final: 0x8f24dff2095d9550, chain: 0x4cdd9ca7fe228bdf},
	{name: "fuzz:grammar:00152", steps: 180, final: 0xab24f8b3de97cfa8, chain: 0x446ea6e021554120},
	{name: "fuzz:grammar:00153", steps: 131, final: 0x452e812412eee1c8, chain: 0x3b8f8a1705c5c4e8},
	{name: "fuzz:grammar:00154", steps: 123, final: 0xb44ed7b123d9c7ae, chain: 0xe462b78a0433ac8f},
	{name: "fuzz:grammar:00155", steps: 313, final: 0x3a03f4246047e5cc, chain: 0x174eb39c47412064},
	{name: "fuzz:grammar:00156", steps: 67, final: 0xcff5c9b90638b4e2, chain: 0xdee9e67851012df7},
	{name: "fuzz:grammar:00157", steps: 389, final: 0xcd4d88eba7f21791, chain: 0x1304439184095293},
	{name: "fuzz:grammar:00158", steps: 124, final: 0xd047b2e5da36fff8, chain: 0xb4db30708bdf2ad1},
	{name: "fuzz:grammar:00159", steps: 276, final: 0x10442a5850585bda, chain: 0xa9f63bdb4e14ec11},
	{name: "fuzz:grammar:00160", steps: 55, final: 0x413776687bebddaa, chain: 0x30b1c495b51438d4},
	{name: "fuzz:grammar:00161", steps: 356, final: 0xeabcd108aceb7425, chain: 0xeb428e2448d575dc},
	{name: "fuzz:grammar:00162", steps: 220, final: 0x62f4ab42647cc466, chain: 0xf73c7c32aca1ff31},
	{name: "fuzz:grammar:00163", steps: 271, final: 0x24a0c637bcd0b5a8, chain: 0xdbae9766110776e4},
	{name: "fuzz:grammar:00164", steps: 54, final: 0x9c1017807f3d2e58, chain: 0xfe90d959adb8fad1},
	{name: "fuzz:grammar:00165", steps: 256, final: 0x29b50f12435f7bfa, chain: 0xcbbe7d6b84082831},
	{name: "fuzz:grammar:00166", steps: 206, final: 0x36556d7015f69497, chain: 0x19326c35cf9b3500},
	{name: "fuzz:grammar:00167", steps: 118, final: 0x03a6a6157c222032, chain: 0xa1e1248459e88dad},
	{name: "fuzz:grammar:00168", steps: 105, final: 0xc1a3b300855ce3cc, chain: 0x6a547e5211acd67a},
	{name: "fuzz:grammar:00169", steps: 335, final: 0xddc3c9f723fb6e15, chain: 0xdbf7237a8fe4f570},
	{name: "fuzz:grammar:00170", steps: 176, final: 0x168476ab718c6733, chain: 0xdc9a4cf4a90bd96e},
	{name: "fuzz:grammar:00171", steps: 220, final: 0x0eda6b7365613334, chain: 0x1fe905bb8cdf9f17},
	{name: "fuzz:grammar:00172", steps: 58, final: 0xf7d076d1f466aad6, chain: 0xc0fdd016416430a7},
	{name: "fuzz:grammar:00173", steps: 288, final: 0x523592810d2b9fc6, chain: 0xec63030c894595e5},
	{name: "fuzz:grammar:00174", steps: 340, final: 0x019b48e9b53003fc, chain: 0xfd924a35572f43a0},
	{name: "fuzz:grammar:00175", steps: 163, final: 0x738292a8cd78afe0, chain: 0xf5b303616320e96a},
	{name: "fuzz:grammar:00176", steps: 117, final: 0x7e8e7f92a76c1d33, chain: 0x61ac4b865fdcab04},
	{name: "fuzz:grammar:00177", steps: 330, final: 0xc1c30c88e5ba4738, chain: 0xa3929675d9cbef5c},
	{name: "fuzz:grammar:00178", steps: 104, final: 0x52e38ff0762234b3, chain: 0x30240d234df74b85},
	{name: "fuzz:grammar:00179", steps: 275, final: 0x848d8bea2179a620, chain: 0x5bbdf75f87e02c25},
	{name: "fuzz:grammar:00180", steps: 252, final: 0x49d7c56da81c76b0, chain: 0x54eb109b3af37400},
	{name: "fuzz:grammar:00181", steps: 119, final: 0xe276d539b1061adb, chain: 0x6f5f21c5ee801e1c},
	{name: "fuzz:grammar:00182", steps: 201, final: 0x08f18899a3d02a72, chain: 0x3d7ce43719c791b1},
	{name: "fuzz:grammar:00183", steps: 116, final: 0x7e894a140e43722b, chain: 0xa1e888d23f4d5825},
	{name: "fuzz:grammar:00184", steps: 375, final: 0x25f514b0f94f90a5, chain: 0x08acee6d6cfa7cf2},
	{name: "fuzz:grammar:00185", steps: 167, final: 0x16afbc1c97c32214, chain: 0xa00088db43e62731},
	{name: "fuzz:grammar:00186", steps: 316, final: 0x4a89b4ff9a61ceca, chain: 0x889af0f01ce2212c},
	{name: "fuzz:grammar:00187", steps: 82, final: 0xb21a4ee793945604, chain: 0xc83ac631eae208b2},
	{name: "fuzz:grammar:00188", steps: 244, final: 0xb24906cb0efff0bc, chain: 0x443fa5c76943ed36},
	{name: "fuzz:grammar:00189", steps: 345, final: 0xd358fdfa17a83639, chain: 0x458d8a4476ff1d26},
	{name: "fuzz:grammar:00190", steps: 359, final: 0x01cdf654fadbd3cc, chain: 0x7f509ba2e80bda0b},
	{name: "fuzz:grammar:00191", steps: 58, final: 0x7c43b65a573e6b66, chain: 0x7efb522171d7737e},
	{name: "fuzz:grammar:00192", steps: 305, final: 0x1af37bafdd6fa641, chain: 0xc1ff58df314d5a96},
	{name: "fuzz:grammar:00193", steps: 259, final: 0x2f7e09901a8165b0, chain: 0x3daf8c210326570e},
	{name: "fuzz:grammar:00194", steps: 373, final: 0x1c6a27385fe20619, chain: 0x03516d4130430391},
	{name: "fuzz:grammar:00195", steps: 379, final: 0x982502a6cba80f23, chain: 0x61f41f9f42cbac31},
	{name: "fuzz:grammar:00196", steps: 179, final: 0xd283c5424be2947a, chain: 0x120f097d1689c4e8},
	{name: "fuzz:grammar:00197", steps: 238, final: 0xd898549b8ef21c58, chain: 0x89f554c785f99c33},
	{name: "fuzz:grammar:00198", steps: 365, final: 0xcd2c9aefce6a83a4, chain: 0x23f69266a21078d2},
	{name: "fuzz:grammar:00199", steps: 59, final: 0xb9cc781733c56569, chain: 0xa17ae5ea6df0861b},
	{name: "fuzz:grammar:00200", steps: 147, final: 0x5c1352d5deb4133f, chain: 0x55b1b611d9c01e92},
	{name: "fuzz:grammar:00201", steps: 375, final: 0xea9a437267eac64a, chain: 0x0c50cc5ee50b4bac},
	{name: "fuzz:grammar:00202", steps: 106, final: 0x67a5ee86224337bd, chain: 0xe31c0853ff512b0f},
	{name: "fuzz:grammar:00203", steps: 75, final: 0x0fd74c70658aff49, chain: 0x01ad7db8a51fd237},
	{name: "fuzz:grammar:00204", steps: 285, final: 0x71034740bdf3d895, chain: 0x6df8f54bed427c78},
	{name: "fuzz:grammar:00205", steps: 409, final: 0x87a9f373270e7bc7, chain: 0xc5ae42073b0dbb70},
	{name: "fuzz:grammar:00206", steps: 284, final: 0x673e07897be77126, chain: 0x637c5010daa55f1f},
	{name: "fuzz:grammar:00207", steps: 122, final: 0xaeaee711559e8529, chain: 0x46a96a1bc627fbfc},
	{name: "fuzz:grammar:00208", steps: 67, final: 0x4c1eede0b2dce31e, chain: 0x8cc6849063339411},
	{name: "fuzz:grammar:00209", steps: 325, final: 0x3d21f4581181f0cc, chain: 0xd65a5d933071de0b},
	{name: "fuzz:grammar:00210", steps: 51, final: 0xe2b6515fa85962ac, chain: 0x8d6983198d17497c},
	{name: "fuzz:grammar:00211", steps: 293, final: 0x5ca3c6b3bb235a5d, chain: 0x44f4688a962e788f},
	{name: "fuzz:grammar:00212", steps: 379, final: 0x98c5fec315de9741, chain: 0x255d841ec33d8431},
	{name: "fuzz:grammar:00213", steps: 82, final: 0x89fb8ccf646ebe74, chain: 0xdd9cb9eb150a1fdf},
	{name: "fuzz:grammar:00214", steps: 197, final: 0x0e0012294f5acd01, chain: 0x9011cb6501b736b2},
	{name: "fuzz:grammar:00215", steps: 83, final: 0x41ad4f1c6cfd68dd, chain: 0x749f9aff80136660},
	{name: "fuzz:grammar:00216", steps: 165, final: 0x603a941878da1fea, chain: 0xb0134de2abf71227},
	{name: "fuzz:grammar:00217", steps: 50, final: 0x778ef23a61bb0bfc, chain: 0xd489e90f3555806a},
	{name: "fuzz:grammar:00218", steps: 354, final: 0xc2174b638c320204, chain: 0x7a22a4ccb40cf675},
	{name: "fuzz:grammar:00219", steps: 326, final: 0xa993cc877126e909, chain: 0x4de14deefc20ffaa},
	{name: "fuzz:grammar:00220", steps: 177, final: 0x2699d087bb14322d, chain: 0x269b28904eac716e},
	{name: "fuzz:grammar:00221", steps: 268, final: 0x33e507768e5b72b5, chain: 0x611b97f8cd1b692f},
	{name: "fuzz:grammar:00222", steps: 224, final: 0x1621584548927c66, chain: 0xb63bc7aeda70f2c4},
	{name: "fuzz:grammar:00223", steps: 109, final: 0x7dc9da3595b76d83, chain: 0x9dba253e841821d2},
	{name: "fuzz:grammar:00224", steps: 264, final: 0xedcbe152d36f95f7, chain: 0x7906a740f8a07337},
	{name: "fuzz:grammar:00225", steps: 133, final: 0x7821df5808ed8dc2, chain: 0xc7c01b8d987eb68c},
	{name: "fuzz:grammar:00226", steps: 305, final: 0xd4fefd6b5c133969, chain: 0x591e04417403a33d},
	{name: "fuzz:grammar:00227", steps: 259, final: 0xc7664ec4e6830168, chain: 0xac941516aab7e27e},
	{name: "fuzz:grammar:00228", steps: 268, final: 0xde9cea6bfa20a995, chain: 0xeeca12b1a55b9ac5},
	{name: "fuzz:grammar:00229", steps: 56, final: 0xe7aedd2d9a8ed921, chain: 0x4e6a0f6d88ca5222},
	{name: "fuzz:grammar:00230", steps: 66, final: 0x40872fea413dc985, chain: 0x97de4ceb05b039a1},
	{name: "fuzz:grammar:00231", steps: 211, final: 0x09beefd2c81e34d9, chain: 0x061224db49338321},
	{name: "fuzz:grammar:00232", steps: 81, final: 0x379f9601784628c4, chain: 0x3fc40812239594da},
	{name: "fuzz:grammar:00233", steps: 59, final: 0xcfa91f79dd15b29d, chain: 0x226062b9a690cdf9},
	{name: "fuzz:grammar:00234", steps: 165, final: 0x479fd17b3e4d79d3, chain: 0x8b99b19aa10e4b8f},
	{name: "fuzz:grammar:00235", steps: 294, final: 0x9d191b2656db20b8, chain: 0xf3045331d088f1ab},
	{name: "fuzz:grammar:00236", steps: 104, final: 0xa2442bc193b33e54, chain: 0xbeab9ce253c309b7},
	{name: "fuzz:grammar:00237", steps: 295, final: 0xb0b843ac48155e68, chain: 0x1f489b1b48dcc01d},
	{name: "fuzz:grammar:00238", steps: 272, final: 0x6d5fc95af7909745, chain: 0x8743f0322aa4d56f},
	{name: "fuzz:grammar:00239", steps: 66, final: 0x76c2016c3e9b74f3, chain: 0xa4616a1f6b501887},
	{name: "fuzz:grammar:00240", steps: 226, final: 0xf0791c5071c08273, chain: 0xe13504591a7829e0},
	{name: "fuzz:grammar:00241", steps: 322, final: 0x313a0d82ba68d04a, chain: 0x2fcdae6d0340df76},
	{name: "fuzz:grammar:00242", steps: 313, final: 0x7e27f2ac845f760d, chain: 0xdab72b13343ddc94},
	{name: "fuzz:grammar:00243", steps: 55, final: 0xfeee9bc3a36af6e9, chain: 0xad84167b9115901f},
	{name: "fuzz:grammar:00244", steps: 61, final: 0xfd38f48ff199090a, chain: 0xc4e7af337560ea69},
	{name: "fuzz:grammar:00245", steps: 107, final: 0x353f33ae7fe2d1fc, chain: 0xbc54b2dfff5a8395},
	{name: "fuzz:grammar:00246", steps: 138, final: 0xbbefafd8c07dd385, chain: 0x1004bf12e86f8509},
	{name: "fuzz:grammar:00247", steps: 406, final: 0xc3c6b54a10b8290f, chain: 0x15e3fa1094acbe91},
	{name: "fuzz:grammar:00248", steps: 143, final: 0xef97ba4b30412a9e, chain: 0xd55cd9ac3e04ce86},
	{name: "fuzz:grammar:00249", steps: 190, final: 0xfec22850a911601f, chain: 0x5003da9cb237981f},
	{name: "fuzz:grammar:00250", steps: 67, final: 0xaf6d990d919b95fd, chain: 0x283e80f390142425},
	{name: "fuzz:grammar:00251", steps: 44, final: 0xa69216c824cb9d59, chain: 0x767e7f3eb3c23e37},
	{name: "fuzz:grammar:00252", steps: 70, final: 0x7e6536f53932fcf7, chain: 0x4eb588031abcd696},
	{name: "fuzz:grammar:00253", steps: 299, final: 0x9b1b45ebdf82aaf5, chain: 0xd7dce36feceafe44},
	{name: "fuzz:grammar:00254", steps: 361, final: 0x012a44be2fe047de, chain: 0x18b45116d2dfc222},
	{name: "fuzz:grammar:00255", steps: 72, final: 0x3e4d047cc1eef79a, chain: 0x3d4231e8653a261d},
	{name: "fuzz:grammar:00256", steps: 41, final: 0xd70fe728823ec9c4, chain: 0xe4e1c59810fdc0ef},
	{name: "fuzz:grammar:00257", steps: 244, final: 0x2968386f7a35e74b, chain: 0x07c59d796d6e4ebb},
	{name: "fuzz:grammar:00258", steps: 98, final: 0xc38e9ff1f5d76c90, chain: 0x23df8efdb79fdddd},
	{name: "fuzz:grammar:00259", steps: 198, final: 0x69c051833f25ac72, chain: 0x73030226542b92d5},
	{name: "fuzz:grammar:00260", steps: 292, final: 0xb19a25ef409dfa40, chain: 0xf1585daf4b644ec8},
	{name: "fuzz:grammar:00261", steps: 266, final: 0xb1fa6e5187d6656e, chain: 0xaa1761e10a78484a},
	{name: "fuzz:grammar:00262", steps: 354, final: 0x971d0286f6129b4a, chain: 0xe4535f2fc6153380},
	{name: "fuzz:grammar:00263", steps: 315, final: 0x3a339a8bb07b7dde, chain: 0xcf7272eee2edcce5},
	{name: "fuzz:grammar:00264", steps: 203, final: 0x9bc67b99d1e70390, chain: 0xb6693df6a28bbfe1},
	{name: "fuzz:grammar:00265", steps: 274, final: 0x9d60865118c06c05, chain: 0xfb918e34eeb372ed},
	{name: "fuzz:grammar:00266", steps: 229, final: 0xb1abc73ed77b37fc, chain: 0x598aa9d1cac8a8c7},
	{name: "fuzz:grammar:00267", steps: 184, final: 0xf6ed5ea1488ed4f0, chain: 0x53a260941ad779db},
	{name: "fuzz:grammar:00268", steps: 330, final: 0x9d4d771f9fceb439, chain: 0x7df8deef8a939b4b},
	{name: "fuzz:grammar:00269", steps: 271, final: 0xc258314719da3a6f, chain: 0x758a0878a4324a3e},
	{name: "fuzz:grammar:00270", steps: 109, final: 0x06444e2ee3dc5281, chain: 0x06e20e092202b2c7},
	{name: "fuzz:grammar:00271", steps: 46, final: 0xb5bdf82239e742e8, chain: 0xc05a88b7412cf31d},
	{name: "fuzz:grammar:00272", steps: 86, final: 0x8ad2075554ef6201, chain: 0x72cdda39a1431b3b},
	{name: "fuzz:grammar:00273", steps: 262, final: 0x2254f08a8ae5bdd0, chain: 0x88243b392c46b15d},
	{name: "fuzz:grammar:00274", steps: 271, final: 0x90cdea5dffc3daf5, chain: 0x2f4de8b1dcee5831},
	{name: "fuzz:grammar:00275", steps: 38, final: 0xf15df2f676e8406c, chain: 0xe0a809df99688216},
	{name: "fuzz:grammar:00276", steps: 210, final: 0xba160045a5de78fd, chain: 0xc0f53736f7c00e6b},
	{name: "fuzz:grammar:00277", steps: 173, final: 0xaf56d887d96b8861, chain: 0xbdd4be7a48f5350e},
	{name: "fuzz:grammar:00278", steps: 279, final: 0x47291b15259da45c, chain: 0x0df7182f1b38200a},
	{name: "fuzz:grammar:00279", steps: 329, final: 0x27d73f0cdfaedfda, chain: 0xc826700998e3558b},
	{name: "fuzz:grammar:00280", steps: 250, final: 0xf98a9b7842d5a839, chain: 0x71a6f5ce912b6493},
	{name: "fuzz:grammar:00281", steps: 188, final: 0x918f69e9819ad0ed, chain: 0x614439a0f65bf2e2},
	{name: "fuzz:grammar:00282", steps: 115, final: 0xaa7c4402ec38802f, chain: 0xe9c958785b0bd07d},
	{name: "fuzz:grammar:00283", steps: 302, final: 0x2efee1f65bd767e1, chain: 0x89f81063b989856e},
	{name: "fuzz:grammar:00284", steps: 235, final: 0x5a7d4e0bf195441b, chain: 0x990f1aaad492edbe},
	{name: "fuzz:grammar:00285", steps: 252, final: 0x95a12a89510aa4bb, chain: 0x7b4ceb2888030b44},
	{name: "fuzz:grammar:00286", steps: 116, final: 0x51592a1bc4418b17, chain: 0x70bd95d955fb0ce6},
	{name: "fuzz:grammar:00287", steps: 371, final: 0xa17af86f1236591b, chain: 0x107042400e8217d6},
	{name: "fuzz:grammar:00288", steps: 200, final: 0xee36ee586a5660d3, chain: 0xe95f2fadc1c1edca},
	{name: "fuzz:grammar:00289", steps: 258, final: 0xb11701a8d73894d4, chain: 0xf477edfa8a63379c},
	{name: "fuzz:grammar:00290", steps: 298, final: 0x85cdcbff69a5c0e4, chain: 0xd0ca1800463c22d2},
	{name: "fuzz:grammar:00291", steps: 280, final: 0x55300e76248f0c46, chain: 0xa3791eb5f4b6291f},
	{name: "fuzz:grammar:00292", steps: 211, final: 0x82fc2424f2b33a3a, chain: 0xa5b7cbefc28410b7},
	{name: "fuzz:grammar:00293", steps: 297, final: 0x89d8dbc0c640ed45, chain: 0x7318067f17275a0e},
	{name: "fuzz:grammar:00294", steps: 388, final: 0x79690ec201ca37dc, chain: 0xe4b0b4611be571ec},
	{name: "fuzz:grammar:00295", steps: 158, final: 0xf657edb6023f44c5, chain: 0xd969c7e9b88508b4},
	{name: "fuzz:grammar:00296", steps: 288, final: 0x0a941d8669eda966, chain: 0x3e8fabfa4185278c},
	{name: "fuzz:grammar:00297", steps: 117, final: 0x0dcc5da2bd8678a6, chain: 0x012f5ff9e91a21c5},
	{name: "fuzz:grammar:00298", steps: 65, final: 0xb9ee2b940dab126a, chain: 0x1a028d31848d40c8},
	{name: "fuzz:grammar:00299", steps: 149, final: 0x4b5977f0df1a4430, chain: 0xa4d1c18f5a777a5c},
	{name: "fuzz:grammar:00300", steps: 140, final: 0x5d1686bcbe71443c, chain: 0xae5151ca226ded71},
	{name: "fuzz:grammar:00301", steps: 241, final: 0x03dd4c257565b9fa, chain: 0x44e10029456b9697},
	{name: "fuzz:grammar:00302", steps: 80, final: 0x7795676cc9bd6f04, chain: 0x914811abf2304947},
	{name: "fuzz:grammar:00303", steps: 279, final: 0xb3dd78f0515d559d, chain: 0x87c4e62dba0187d4},
	{name: "fuzz:grammar:00304", steps: 318, final: 0x95397a95c5e14558, chain: 0x89774f9f453cad97},
	{name: "fuzz:grammar:00305", steps: 258, final: 0x26e5220c86f22391, chain: 0x488c3e333f878e30},
	{name: "fuzz:grammar:00306", steps: 174, final: 0xc7b3a5f91ea215fe, chain: 0x8641b3656ba01979},
	{name: "fuzz:grammar:00307", steps: 101, final: 0xa9aa15b33a27f191, chain: 0x1ca54a93bf96e75b},
	{name: "fuzz:grammar:00308", steps: 191, final: 0xfa80b8b520e52bc3, chain: 0xae4508bd1ee6167e},
	{name: "fuzz:grammar:00309", steps: 325, final: 0xe68cf2f6e6f17bfd, chain: 0x6af42e80d3987368},
	{name: "fuzz:grammar:00310", steps: 122, final: 0x945abbcfa73d5dc4, chain: 0x48c7aba5e8519ab8},
	{name: "fuzz:grammar:00311", steps: 351, final: 0x49d60fc810ac1d9a, chain: 0xf57c9dea9503eeb4},
	{name: "fuzz:grammar:00312", steps: 108, final: 0x18409cad7de58f3d, chain: 0x93ed733c1047e5cf},
	{name: "fuzz:grammar:00313", steps: 139, final: 0x9aa84c0d30a5f0f7, chain: 0x6a0f6d778f2deb58},
	{name: "fuzz:grammar:00314", steps: 251, final: 0x53bab1bf0b8e7667, chain: 0x2d519ca31fde7b9b},
	{name: "fuzz:grammar:00315", steps: 206, final: 0x8410457014c8e547, chain: 0x446a59f51a682ec0},
	{name: "fuzz:grammar:00316", steps: 351, final: 0xd29f174e6cfc3554, chain: 0x77ea526519cd2051},
	{name: "fuzz:grammar:00317", steps: 328, final: 0xeae424f6dca6157a, chain: 0x22c51cbf503b7b5b},
	{name: "fuzz:grammar:00318", steps: 295, final: 0x2e17276e9630042b, chain: 0x61826e73c0d9ebe6},
	{name: "fuzz:grammar:00319", steps: 347, final: 0x6864f13022283f04, chain: 0x55d4f1638b7332aa},
	{name: "fuzz:grammar:00320", steps: 161, final: 0x1d60a69ce10add78, chain: 0xefb47ff93f3bd8d9},
	{name: "fuzz:grammar:00321", steps: 317, final: 0xa500b6e0ca3dbd82, chain: 0xe3cad648b11c65c8},
	{name: "fuzz:grammar:00322", steps: 327, final: 0x010ae4a82f467341, chain: 0xfd35d51dd33c0262},
	{name: "fuzz:grammar:00323", steps: 324, final: 0xba97857176f71a90, chain: 0x1a057f33bbc286af},
	{name: "fuzz:grammar:00324", steps: 68, final: 0xc06c570a4fe83d3e, chain: 0x8af200b7682d95f7},
	{name: "fuzz:grammar:00325", steps: 182, final: 0xb21a4ee793945604, chain: 0x03cd2ff188f1f2c4},
	{name: "fuzz:grammar:00326", steps: 184, final: 0xcbd99b98c4721a37, chain: 0x05a9471e5df669c6},
	{name: "fuzz:grammar:00327", steps: 266, final: 0x0b5d571c71cd6b79, chain: 0x047038572f325c10},
	{name: "fuzz:grammar:00328", steps: 229, final: 0x0c54d2497b1423cd, chain: 0xd0eaa3c96b989b31},
	{name: "fuzz:grammar:00329", steps: 366, final: 0x4c99fba7c76c57dc, chain: 0x9f8f51f90a7da812},
	{name: "fuzz:grammar:00330", steps: 76, final: 0x62686818af9f7ae8, chain: 0x215ee38a364353d9},
	{name: "fuzz:grammar:00331", steps: 97, final: 0x3dc3f68751df048e, chain: 0xc7f4fdaa5936e907},
	{name: "fuzz:grammar:00332", steps: 221, final: 0x02aa953049ee9623, chain: 0xf03e7928387fa285},
	{name: "fuzz:grammar:00333", steps: 229, final: 0x1554864aead66b8f, chain: 0xb0aac26ee404bbe3},
	{name: "fuzz:grammar:00334", steps: 371, final: 0x74e6eabcf7282487, chain: 0xe1c68e2ef4314b02},
	{name: "fuzz:grammar:00335", steps: 144, final: 0xc522f9c2943f1922, chain: 0xd1d202d3714a12b0},
	{name: "fuzz:grammar:00336", steps: 121, final: 0x16269047dcde0515, chain: 0x0e5b5adef6df5f75},
	{name: "fuzz:grammar:00337", steps: 261, final: 0x6a85833cbaf23d3f, chain: 0xbe89e012a1761b90},
	{name: "fuzz:grammar:00338", steps: 316, final: 0x5070fe7812af4d9c, chain: 0xf7515cc92c3ae9d7},
	{name: "fuzz:grammar:00339", steps: 286, final: 0x107cffc9f9d18f66, chain: 0x047690a195889b6f},
	{name: "fuzz:grammar:00340", steps: 81, final: 0xf65ed0cb1e857a74, chain: 0xcb60439e2126ee9f},
	{name: "fuzz:grammar:00341", steps: 348, final: 0xed76b6badedff2e8, chain: 0xe2de68954fe536f0},
	{name: "fuzz:grammar:00342", steps: 237, final: 0xa746aa20c7d6558e, chain: 0x20cc27901ee8f811},
	{name: "fuzz:grammar:00343", steps: 52, final: 0x3b78f9c37d231612, chain: 0x0b3e82417f778d35},
	{name: "fuzz:grammar:00344", steps: 357, final: 0x73572d310c7a3a8c, chain: 0x25c3fe3b649324a6},
	{name: "fuzz:grammar:00345", steps: 244, final: 0x12298576df38420d, chain: 0x20f587000572896c},
	{name: "fuzz:grammar:00346", steps: 78, final: 0x5bddae7a4042aa0e, chain: 0x772a9eecce55830a},
	{name: "fuzz:grammar:00347", steps: 322, final: 0x267826a92703cc68, chain: 0x44c920795fc92c43},
	{name: "fuzz:grammar:00348", steps: 337, final: 0x4df6b241ac68346b, chain: 0x00ebff27118388fb},
	{name: "fuzz:grammar:00349", steps: 344, final: 0x7710523def444013, chain: 0x1bd6e22c37078056},
	{name: "fuzz:grammar:00350", steps: 111, final: 0xc865ca6fce63db27, chain: 0x5feb4010fce17875},
	{name: "fuzz:grammar:00351", steps: 263, final: 0x44b41236bfaa76c7, chain: 0x650e5fa3505561aa},
	{name: "fuzz:grammar:00352", steps: 90, final: 0x815085badd6fab35, chain: 0xbd74a7b1d2741ba5},
	{name: "fuzz:grammar:00353", steps: 203, final: 0x7fcbec1bcbb195f7, chain: 0x322bbe5c13893327},
	{name: "fuzz:grammar:00354", steps: 129, final: 0x4c0bec2e7d499814, chain: 0xd55628ba63cc6101},
	{name: "fuzz:grammar:00355", steps: 121, final: 0x56ede4e445333f36, chain: 0x845ae97929579dad},
	{name: "fuzz:grammar:00356", steps: 154, final: 0x3008d57123c91c06, chain: 0xb99c5ce8b0fec010},
	{name: "fuzz:grammar:00357", steps: 271, final: 0xeb28c5c39e162a2e, chain: 0xebb4616d1237889e},
	{name: "fuzz:grammar:00358", steps: 292, final: 0x671d619a9590572d, chain: 0xf63d444bebd0fe0b},
	{name: "fuzz:grammar:00359", steps: 73, final: 0x27483bcb87bdf3f9, chain: 0x670dd59dc1f5dec1},
	{name: "fuzz:grammar:00360", steps: 358, final: 0x18f7f37e4af59312, chain: 0xe931386754f30f70},
	{name: "fuzz:grammar:00361", steps: 82, final: 0x2786bd339d7b20ed, chain: 0xb42cb1eaf9cd9b59},
	{name: "fuzz:grammar:00362", steps: 343, final: 0x5b72fe983c3156af, chain: 0x034f29dadaf779b2},
	{name: "fuzz:grammar:00363", steps: 394, final: 0xd488165c0cb1bf07, chain: 0x03aa6943ed9385fd},
	{name: "fuzz:grammar:00364", steps: 57, final: 0xa1370bfd88b318d7, chain: 0x9af30c8fde62fd26},
	{name: "fuzz:grammar:00365", steps: 156, final: 0x23470e092c02beed, chain: 0xf7c7d5ef9226e200},
	{name: "fuzz:grammar:00366", steps: 354, final: 0x031753af54c219a0, chain: 0x5078e612b036b9e6},
	{name: "fuzz:grammar:00367", steps: 324, final: 0x318e3a7950d7adf3, chain: 0xb1451e30d4594a83},
	{name: "fuzz:grammar:00368", steps: 332, final: 0x5d4d5e27df6f609f, chain: 0xcafb2dcc303d8372},
	{name: "fuzz:grammar:00369", steps: 352, final: 0xb2cfe6ccf1d79876, chain: 0x3607237e15267e5d},
	{name: "fuzz:grammar:00370", steps: 219, final: 0xb50acade5189581c, chain: 0xcea163340770443b},
	{name: "fuzz:grammar:00371", steps: 144, final: 0x40c0888b8b9fc165, chain: 0x284bd64aa4de8ea1},
	{name: "fuzz:grammar:00372", steps: 366, final: 0xb5f67e3a373fcb87, chain: 0xc3d2b86141caa2be},
	{name: "fuzz:grammar:00373", steps: 210, final: 0x3899163adde8242d, chain: 0xee97f87b9066f1f1},
	{name: "fuzz:grammar:00374", steps: 249, final: 0x7fac0c07d3868a22, chain: 0x15bf63fe4b18d44e},
	{name: "fuzz:grammar:00375", steps: 336, final: 0x6bbe4b6be09e4984, chain: 0x4032afd9c6394cc6},
	{name: "fuzz:grammar:00376", steps: 142, final: 0x0313d21c14ed71e6, chain: 0x8e7611e17a85c4f3},
	{name: "fuzz:grammar:00377", steps: 345, final: 0x6611e365c7e506b1, chain: 0x8aa04ce80aeb2c4e},
	{name: "fuzz:grammar:00378", steps: 188, final: 0xdc6fbc1bd47789fd, chain: 0x5b7ff645d1510deb},
	{name: "fuzz:grammar:00379", steps: 191, final: 0xb1abd07371390494, chain: 0x73b3dbc22114572a},
	{name: "fuzz:grammar:00380", steps: 54, final: 0x6a8b4938ccd84498, chain: 0xc95bc93b84b3f5a2},
	{name: "fuzz:grammar:00381", steps: 139, final: 0x567e6922a7fe66b6, chain: 0x70f22fce4ef3aa64},
	{name: "fuzz:grammar:00382", steps: 171, final: 0x0e1296755314373f, chain: 0x8a4925ec53579b05},
	{name: "fuzz:grammar:00383", steps: 362, final: 0x54d2b7cecdf70825, chain: 0x68ac07ef2ec09c10},
	{name: "fuzz:grammar:00384", steps: 253, final: 0xc2654aed7de2f187, chain: 0x1d2063b957bac5bf},
	{name: "fuzz:grammar:00385", steps: 278, final: 0x09a810d3ebed49f6, chain: 0x5cb358157e230db7},
	{name: "fuzz:grammar:00386", steps: 196, final: 0x13f401142f548626, chain: 0x95d55a755120fda0},
	{name: "fuzz:grammar:00387", steps: 383, final: 0x9c85eb4b77311a71, chain: 0xe2641c1772615b29},
	{name: "fuzz:grammar:00388", steps: 362, final: 0x61ed558d1109b7a3, chain: 0x9cf8b26a04e1fea0},
	{name: "fuzz:grammar:00389", steps: 210, final: 0xb99f50d76676d9ed, chain: 0x6a9ae29ecf062d1d},
	{name: "fuzz:grammar:00390", steps: 65, final: 0x3eea718ee66be55e, chain: 0x9d3757765fb765e7},
	{name: "fuzz:grammar:00391", steps: 332, final: 0x17dc6d2bde3c7e95, chain: 0xeb6ea160e808d6a8},
	{name: "fuzz:grammar:00392", steps: 257, final: 0x2ae6f9635aae5c56, chain: 0xd8e053881b727546},
	{name: "fuzz:grammar:00393", steps: 81, final: 0x860d53cc9a523b17, chain: 0x488cd7ca344f41a7},
	{name: "fuzz:grammar:00394", steps: 187, final: 0xb36d2524264127f3, chain: 0x94f7fe1a5cb14289},
	{name: "fuzz:grammar:00395", steps: 348, final: 0xb261ee1f137e92b7, chain: 0x75ee2f5f1be0a6f4},
	{name: "fuzz:grammar:00396", steps: 199, final: 0x516491f816463250, chain: 0x50fe6aa4cf658c61},
	{name: "fuzz:grammar:00397", steps: 259, final: 0xd0bf992f554a8f54, chain: 0x350bda586c71656e},
	{name: "fuzz:grammar:00398", steps: 341, final: 0x4e37c3e61e257748, chain: 0xa9f359c35e486969},
	{name: "fuzz:grammar:00399", steps: 250, final: 0xcaa9d2185306db2f, chain: 0xe621a7493141ad8e},
	{name: "fuzz:grammar:00400", steps: 234, final: 0x3c914fe985352a7f, chain: 0x98ed31b9e8b0ac64},
	{name: "fuzz:grammar:00401", steps: 148, final: 0x04b9b579af82b2dc, chain: 0x845cb5f6a7a04836},
	{name: "fuzz:grammar:00402", steps: 156, final: 0xe078abc62d757a5b, chain: 0xb8620e6267831521},
	{name: "fuzz:grammar:00403", steps: 328, final: 0x0029377aedfe9b81, chain: 0x6c129277c749eedc},
	{name: "fuzz:grammar:00404", steps: 166, final: 0x05084a69e93532f8, chain: 0xfa47b1dd1022a6f4},
	{name: "fuzz:grammar:00405", steps: 185, final: 0x2ae182ad37dab15d, chain: 0x17b6daed325140eb},
	{name: "fuzz:grammar:00406", steps: 263, final: 0xfdb2f1fd380a782e, chain: 0x097ebe3e21ebbc1c},
	{name: "fuzz:grammar:00407", steps: 324, final: 0x3e0999da4a606ec4, chain: 0x1bfd18df6a8f9edf},
	{name: "fuzz:grammar:00408", steps: 216, final: 0x99c4dc0aa6ff952a, chain: 0xac2e345666db5e9d},
	{name: "fuzz:grammar:00409", steps: 210, final: 0xf9eb86eb66f42053, chain: 0x617b690474589711},
	{name: "fuzz:grammar:00410", steps: 276, final: 0x254bf5c20d5466f2, chain: 0xa069053030485317},
	{name: "fuzz:grammar:00411", steps: 75, final: 0xa2e40aaf98ddaad7, chain: 0x18443c01784d6556},
	{name: "fuzz:grammar:00412", steps: 340, final: 0x0f5f5d6112819299, chain: 0x50aae6b5aa727e79},
	{name: "fuzz:grammar:00413", steps: 84, final: 0xbb2343fd197b7873, chain: 0x1c10d1c0ec36d1bf},
	{name: "fuzz:grammar:00414", steps: 320, final: 0x079e149a365bb735, chain: 0x27d758992f9e6ae4},
	{name: "fuzz:grammar:00415", steps: 293, final: 0x3b240e48988989c2, chain: 0x0a711340f3f9804f},
	{name: "fuzz:grammar:00416", steps: 149, final: 0xbf5f0b41bde474c4, chain: 0x2cdbb1dfc9d678b3},
	{name: "fuzz:grammar:00417", steps: 89, final: 0x62e6bc7513d71559, chain: 0xd5e91a244fc5f077},
	{name: "fuzz:grammar:00418", steps: 55, final: 0x1473fe440e0bf8be, chain: 0x5eec836ca74e8069},
	{name: "fuzz:grammar:00419", steps: 109, final: 0xf2b053d6ede8b8d6, chain: 0x669d3a81b8283e56},
	{name: "fuzz:grammar:00420", steps: 107, final: 0x1f59ee1b0bd1ab59, chain: 0x03811597876d4a25},
	{name: "fuzz:grammar:00421", steps: 57, final: 0x214dfc5ebcd67435, chain: 0x6a35cc1489534d08},
	{name: "fuzz:grammar:00422", steps: 305, final: 0x23911a683e76dae6, chain: 0xed6d6291f4ab3a24},
	{name: "fuzz:grammar:00423", steps: 246, final: 0x0f0980db9947171a, chain: 0x8f4cfa2f5422d687},
	{name: "fuzz:grammar:00424", steps: 104, final: 0xd8292511da5e34f2, chain: 0x893ec7f72f573387},
	{name: "fuzz:grammar:00425", steps: 209, final: 0x2e7a2e9d344214e1, chain: 0x68de323db7e589ed},
	{name: "fuzz:grammar:00426", steps: 249, final: 0xb764f8a1fee581dc, chain: 0x8700d3ea03d4952c},
	{name: "fuzz:grammar:00427", steps: 194, final: 0x4c0ed0e50027d373, chain: 0x0ef92fcd1d997579},
	{name: "fuzz:grammar:00428", steps: 300, final: 0x920387980ac7c790, chain: 0x90345927a6cac5fb},
	{name: "fuzz:grammar:00429", steps: 149, final: 0x8b39e351a72d44e8, chain: 0xf1f14c1026bc5162},
	{name: "fuzz:grammar:00430", steps: 200, final: 0xdbe2ebd654fa5f2d, chain: 0xcaaddd5d4fd6157b},
	{name: "fuzz:grammar:00431", steps: 80, final: 0x3f94f2a38031c26a, chain: 0xa0d98518ee6e2031},
	{name: "fuzz:grammar:00432", steps: 362, final: 0xfd432e03901d5b38, chain: 0x849ea416bd6add54},
	{name: "fuzz:grammar:00433", steps: 366, final: 0x50a6200b6be671f4, chain: 0xe03956aa9e726b89},
	{name: "fuzz:grammar:00434", steps: 363, final: 0x741fcd5300999373, chain: 0x4cee102d7e611636},
	{name: "fuzz:grammar:00435", steps: 164, final: 0x947bd9f236e28f06, chain: 0x957035873be1f897},
	{name: "fuzz:grammar:00436", steps: 62, final: 0x90ec56c018647c7f, chain: 0x0d0e976e249f619d},
	{name: "fuzz:grammar:00437", steps: 207, final: 0xe2a5c949c9a89610, chain: 0xb38c6aba0f3ee572},
	{name: "fuzz:grammar:00438", steps: 109, final: 0xb30af4098d87ea8a, chain: 0xea172dd9d4f9e42f},
	{name: "fuzz:grammar:00439", steps: 337, final: 0x161316ffdec826a3, chain: 0x375fbe6b5070a391},
	{name: "fuzz:grammar:00440", steps: 68, final: 0x65373a7235ff7a68, chain: 0x02c54c852cdef0c7},
	{name: "fuzz:grammar:00441", steps: 257, final: 0xccc050d8b3a5cdc9, chain: 0x873283cf7e53267f},
	{name: "fuzz:grammar:00442", steps: 264, final: 0x69528f2245acfa76, chain: 0x852f916cf6181f8a},
	{name: "fuzz:grammar:00443", steps: 224, final: 0xb70a4469c4df3609, chain: 0x454bcd11371b3443},
	{name: "fuzz:grammar:00444", steps: 278, final: 0x7cb165b1451ea082, chain: 0x7f424e5b46d879c6},
	{name: "fuzz:grammar:00445", steps: 176, final: 0x12e7def12e082002, chain: 0x795f05c767e0587e},
	{name: "fuzz:grammar:00446", steps: 61, final: 0x5236e575d757330f, chain: 0x8c8fe1633345ff37},
	{name: "fuzz:grammar:00447", steps: 293, final: 0x444f59c34cf64a5a, chain: 0x6728b3b4a5930174},
	{name: "fuzz:grammar:00448", steps: 288, final: 0xc6e7133f33634ed2, chain: 0x200e8cab079490c8},
	{name: "fuzz:grammar:00449", steps: 225, final: 0xb0fad59fe9ae8d1a, chain: 0xf8110fbafdaf7df0},
	{name: "fuzz:grammar:00450", steps: 311, final: 0x1cf8a076d5f9eea7, chain: 0x15c77204098f7896},
	{name: "fuzz:grammar:00451", steps: 139, final: 0xf06a167a04a62f1e, chain: 0x93a11ecfdb3d89c5},
	{name: "fuzz:grammar:00452", steps: 426, final: 0xc2f6f0a4e18ea48c, chain: 0x94041f99b833c944},
	{name: "fuzz:grammar:00453", steps: 217, final: 0x0ab7d8d843219cf7, chain: 0x28f6c8b6eeaab823},
	{name: "fuzz:grammar:00454", steps: 171, final: 0xe8a52f45b82821e6, chain: 0x4ac8767bfac4f002},
	{name: "fuzz:grammar:00455", steps: 86, final: 0xf9130d6e2ec725a1, chain: 0x5cd3f4b396e26169},
	{name: "fuzz:grammar:00456", steps: 205, final: 0x3808306cbba5edc7, chain: 0x9d062a05e6550ff0},
	{name: "fuzz:grammar:00457", steps: 91, final: 0x60a17e53c3d2f6e9, chain: 0x638b6021582538ee},
	{name: "fuzz:grammar:00458", steps: 48, final: 0xac1bd6fcbabeee49, chain: 0xd434f67c509447c0},
	{name: "fuzz:grammar:00459", steps: 312, final: 0x3efd115f24b532a1, chain: 0xe136f8bb18ac98be},
	{name: "fuzz:grammar:00460", steps: 59, final: 0xb667a37d1e73686f, chain: 0x34452de32c6f657e},
	{name: "fuzz:grammar:00461", steps: 301, final: 0x36418cb640a3b3e8, chain: 0x1e1dce7898049076},
	{name: "fuzz:grammar:00462", steps: 56, final: 0x1f567095c618c843, chain: 0x1de25bae8cf50bdc},
	{name: "fuzz:grammar:00463", steps: 312, final: 0x8972bd4d80851250, chain: 0xb04b327c82d5884c},
	{name: "fuzz:grammar:00464", steps: 101, final: 0x46d0df2b5a0c41da, chain: 0x80ea73cf2be797d1},
	{name: "fuzz:grammar:00465", steps: 170, final: 0x14612646bafa5a3d, chain: 0xedc05c29b6c1d994},
	{name: "fuzz:grammar:00466", steps: 272, final: 0x2bbc1b4ba7c5d374, chain: 0x4fc4060ae55499c1},
	{name: "fuzz:grammar:00467", steps: 274, final: 0x8ca069cc5786a46f, chain: 0xf9f4d346ae1a167d},
	{name: "fuzz:grammar:00468", steps: 131, final: 0x0efc617b312d3b47, chain: 0x2ff3da44b429c09b},
	{name: "fuzz:grammar:00469", steps: 119, final: 0xbde7edd418f9b516, chain: 0x3c44aab517c8328e},
	{name: "fuzz:grammar:00470", steps: 99, final: 0x97e78630ffc4a3e1, chain: 0xaf0e36a3e25fafd4},
	{name: "fuzz:grammar:00471", steps: 341, final: 0xdf9d6e7e33c33ddc, chain: 0x35fbd41c66521fab},
	{name: "fuzz:grammar:00472", steps: 170, final: 0xbca229a000d52ddf, chain: 0x87814578e647aed6},
	{name: "fuzz:grammar:00473", steps: 373, final: 0x58b7285645379f35, chain: 0xf2abf6206fd3ea54},
	{name: "fuzz:grammar:00474", steps: 244, final: 0x3ca298f7950e7d4b, chain: 0x78d3adc3dfe6cc85},
	{name: "fuzz:grammar:00475", steps: 298, final: 0x980c79deca3b3e10, chain: 0xc00b5f890d98655a},
	{name: "fuzz:grammar:00476", steps: 211, final: 0xcb1125f20eb49718, chain: 0x9d5c78c142939900},
	{name: "fuzz:grammar:00477", steps: 316, final: 0xfaf53a680438b976, chain: 0x8020f5c0eb671247},
	{name: "fuzz:grammar:00478", steps: 268, final: 0xb7d1991182b89037, chain: 0xfd100ac2ba5bf098},
	{name: "fuzz:grammar:00479", steps: 155, final: 0x5a5767483bb303c2, chain: 0x490973b1bcd58880},
	{name: "fuzz:grammar:00480", steps: 44, final: 0xb942e72c778f34c4, chain: 0x3e296b9c4af389bc},
	{name: "fuzz:grammar:00481", steps: 204, final: 0x5e7a2c37ed16642d, chain: 0xc22c2873d217d4db},
	{name: "fuzz:grammar:00482", steps: 378, final: 0xc554659a357b1f29, chain: 0xc6269369e62f30ef},
	{name: "fuzz:grammar:00483", steps: 82, final: 0x7d0e0880ba624baa, chain: 0x2585cb5afa67b781},
	{name: "fuzz:grammar:00484", steps: 212, final: 0x3d61029bb8d73ab6, chain: 0x16fdaff6074ad823},
	{name: "fuzz:grammar:00485", steps: 251, final: 0x95a92737f476327e, chain: 0x4635551f3dd05106},
	{name: "fuzz:grammar:00486", steps: 240, final: 0x071f4942d4389ba7, chain: 0x265b39a1d5bcad19},
	{name: "fuzz:grammar:00487", steps: 350, final: 0xdd2aec10370bd73e, chain: 0x9775f4c16ff8672c},
	{name: "fuzz:grammar:00488", steps: 89, final: 0x71f83cb0fefa3d26, chain: 0x4de598d826e58583},
	{name: "fuzz:grammar:00489", steps: 215, final: 0x555e7561fd4a0fb5, chain: 0xc911c22d0814999a},
	{name: "fuzz:grammar:00490", steps: 288, final: 0xf7c281e1acfb4726, chain: 0x809be348d34430ab},
	{name: "fuzz:grammar:00491", steps: 123, final: 0xb4dd69421270974a, chain: 0x7ec2fff7f0d64998},
	{name: "fuzz:grammar:00492", steps: 157, final: 0x71a0359a066e7bd0, chain: 0xd1982d17b8b48fbd},
	{name: "fuzz:grammar:00493", steps: 91, final: 0x37506f8b80174a58, chain: 0x9ffbbca4aee63751},
	{name: "fuzz:grammar:00494", steps: 201, final: 0xcc815402fe5d5bdc, chain: 0xd931d05d73ca4313},
	{name: "fuzz:grammar:00495", steps: 181, final: 0x4d090d65d31cbe82, chain: 0x55e576cfde6ea2e0},
	{name: "fuzz:grammar:00496", steps: 41, final: 0xa5e281f242372600, chain: 0x1870f677b9760a22},
	{name: "fuzz:grammar:00497", steps: 357, final: 0x0a600a2fe1132905, chain: 0x9f95c8000c55c495},
	{name: "fuzz:grammar:00498", steps: 340, final: 0x513b5675c0763c10, chain: 0x4582d1bf576f0406},
	{name: "fuzz:grammar:00499", steps: 130, final: 0xcddcf3f8c7e47b90, chain: 0x65f19d94b3ed2506},
	{name: "fuzz:grammar:00500", steps: 180, final: 0x4e4b64af90e69521, chain: 0x7c290448b49349cd},
	{name: "fuzz:grammar:00501", steps: 318, final: 0xe7ed9af3ee0a4188, chain: 0xbd01fe8637caab01},
	{name: "fuzz:grammar:00502", steps: 182, final: 0x317c096dccb3c032, chain: 0x3a9eb56a30a10895},
	{name: "fuzz:grammar:00503", steps: 43, final: 0xb3d424a2d738628d, chain: 0x8c530d4ff5e7bd90},
	{name: "fuzz:grammar:00504", steps: 169, final: 0x5540f6b3a349709f, chain: 0x6ec80c6ebea7941e},
	{name: "fuzz:grammar:00505", steps: 239, final: 0x7ee5039b470ee020, chain: 0x218371f201a1b1b1},
	{name: "fuzz:grammar:00506", steps: 216, final: 0x7bcdc5f7db3d6815, chain: 0xc3f71b40bcbda4df},
	{name: "fuzz:grammar:00507", steps: 196, final: 0x97019e4ab82d85eb, chain: 0x67340a8aeecc09b4},
	{name: "fuzz:grammar:00508", steps: 92, final: 0x798cb3fd925ba97f, chain: 0x42f64c5eeaceb59e},
	{name: "fuzz:grammar:00509", steps: 125, final: 0xe87e805d541f7462, chain: 0x20f4dec302d86960},
	{name: "fuzz:grammar:00510", steps: 321, final: 0xd279f005bf0f0fb7, chain: 0x25c432d901d76871},
	{name: "fuzz:grammar:00511", steps: 51, final: 0xba13a37012b2965f, chain: 0xb0ed859379677d30},
	{name: "fuzz:grammar:00512", steps: 393, final: 0x86898c9e5c0afcbc, chain: 0x379a66d43533f77b},
	{name: "fuzz:grammar:00513", steps: 211, final: 0x2e8cb30d8e993dda, chain: 0x2bb78d8f0135993e},
	{name: "fuzz:grammar:00514", steps: 191, final: 0x53db4aebed6a9948, chain: 0xe5fa692c1c0730a6},
	{name: "fuzz:grammar:00515", steps: 188, final: 0x3f362f5469364e6b, chain: 0x150bf79b3e64733d},
	{name: "fuzz:grammar:00516", steps: 42, final: 0x207b16c3d1962052, chain: 0x400669de080aa354},
	{name: "fuzz:grammar:00517", steps: 171, final: 0xf435c2804c2cd1cb, chain: 0x3987abea8ba4ab82},
	{name: "fuzz:grammar:00518", steps: 230, final: 0x741f2a441be33234, chain: 0xab25ee7311fabfd8},
	{name: "fuzz:grammar:00519", steps: 295, final: 0x03942143e7d87758, chain: 0x884eafe04add02c3},
	{name: "fuzz:grammar:00520", steps: 118, final: 0x48c7d7aa0b746a19, chain: 0x50a9352e27d1286a},
	{name: "fuzz:grammar:00521", steps: 351, final: 0x98ea45ec8c58b01d, chain: 0x4138d2a41cedfabd},
	{name: "fuzz:grammar:00522", steps: 153, final: 0x087d847ce26fb4d7, chain: 0xb7ddcf72c9e5afb2},
	{name: "fuzz:grammar:00523", steps: 270, final: 0x0c386fc73d8fff7e, chain: 0x4da649cde4dab24e},
	{name: "fuzz:grammar:00524", steps: 370, final: 0x6233fb472685ed62, chain: 0x5d5152fc4c8e0d30},
	{name: "fuzz:grammar:00525", steps: 276, final: 0xc719fe95c7b5bb11, chain: 0xad0f137265d6d215},
	{name: "fuzz:grammar:00526", steps: 198, final: 0xe7b9b6da08e08753, chain: 0x67ef69dbbfbc9edf},
	{name: "fuzz:grammar:00527", steps: 103, final: 0xfe382c56254f8020, chain: 0x6f98db1436ce1979},
	{name: "fuzz:grammar:00528", steps: 227, final: 0x048d5b890130da6b, chain: 0xb4ca666445931254},
	{name: "fuzz:grammar:00529", steps: 342, final: 0x5dab7f197d048e53, chain: 0x748805c3ad5d7865},
	{name: "fuzz:grammar:00530", steps: 334, final: 0xeb0fe058db7cbd67, chain: 0x4a2307e730a0c19d},
	{name: "fuzz:grammar:00531", steps: 265, final: 0x949e152cb38f88ac, chain: 0x6e2b0c3acd6b1fcb},
	{name: "fuzz:grammar:00532", steps: 80, final: 0x79745ce153893b83, chain: 0xf192f23d810d2779},
	{name: "fuzz:grammar:00533", steps: 72, final: 0xd18de6aae43b8cd8, chain: 0x82c13f62492952dc},
	{name: "fuzz:grammar:00534", steps: 317, final: 0x2f79ef52ee1ebc6d, chain: 0x2284218e4c57a47a},
	{name: "fuzz:grammar:00535", steps: 277, final: 0xc06379006e15eb10, chain: 0xb4c6bc4f10991c5c},
	{name: "fuzz:grammar:00536", steps: 336, final: 0x989859b08567ce02, chain: 0x64bb56ae840b0b0e},
	{name: "fuzz:grammar:00537", steps: 157, final: 0x086ecf46d3106b6f, chain: 0x9d17bd9224f49b2d},
	{name: "fuzz:grammar:00538", steps: 52, final: 0xe2b68b4ca0f513b6, chain: 0x7ee9c74a1bd6ce8d},
	{name: "fuzz:grammar:00539", steps: 306, final: 0x5ccdcc86126dbeff, chain: 0x10c10a84304ff6b3},
	{name: "fuzz:grammar:00540", steps: 353, final: 0x89abd0ccf457c342, chain: 0x9f029a97438bb483},
	{name: "fuzz:grammar:00541", steps: 340, final: 0x34c317b55debdb51, chain: 0xbce8546d65dbc9d7},
	{name: "fuzz:grammar:00542", steps: 188, final: 0xba012abb1ce9addf, chain: 0x921056593631658a},
	{name: "fuzz:grammar:00543", steps: 232, final: 0x5ab7c17136b67e1f, chain: 0x556c7059c8ede370},
	{name: "fuzz:grammar:00544", steps: 188, final: 0xa60321a31873c1d8, chain: 0x6df354411474a0d7},
	{name: "fuzz:grammar:00545", steps: 227, final: 0xb116f1455cbab921, chain: 0xae671fc83b5a2074},
	{name: "fuzz:grammar:00546", steps: 330, final: 0x83e28f3bb066c436, chain: 0xc90cad9d39a5070b},
	{name: "fuzz:grammar:00547", steps: 282, final: 0x9c16d4e6a22b3314, chain: 0x2af6feb400f383ba},
	{name: "fuzz:grammar:00548", steps: 313, final: 0x30fcbd843dfbf37d, chain: 0xe82e5f259a1f7019},
	{name: "fuzz:grammar:00549", steps: 32, final: 0x8dc1e4ff263161db, chain: 0x740effbc14b8b216},
	{name: "fuzz:grammar:00550", steps: 355, final: 0xa76494d7b0075224, chain: 0x87338b486f60239b},
	{name: "fuzz:grammar:00551", steps: 347, final: 0x57d66c20c752fc23, chain: 0x28203484623f9840},
	{name: "fuzz:grammar:00552", steps: 108, final: 0x74f7979b36301b49, chain: 0xe0d0addb33fa1982},
	{name: "fuzz:grammar:00553", steps: 135, final: 0x21ac888bdd54138d, chain: 0x4c35ff65ab0a3f02},
	{name: "fuzz:grammar:00554", steps: 138, final: 0xf37db27573dd0b3e, chain: 0xd921f27dfa65b638},
	{name: "fuzz:grammar:00555", steps: 335, final: 0x819937338e69a632, chain: 0xa3939773211be57b},
	{name: "fuzz:grammar:00556", steps: 138, final: 0x4cc7d0d26392d285, chain: 0x0df196a3a7386eb4},
	{name: "fuzz:grammar:00557", steps: 71, final: 0x671581acd9be586c, chain: 0xc1251cdf5e80c666},
	{name: "fuzz:grammar:00558", steps: 228, final: 0xd43adb7e3c8feb61, chain: 0x5d70d912b75f2326},
	{name: "fuzz:grammar:00559", steps: 142, final: 0x3051dc44cb0c295b, chain: 0xc19b526b7fe9e1b5},
	{name: "fuzz:grammar:00560", steps: 350, final: 0x9258a3c41277c4f5, chain: 0xe4b5800b22495c3d},
	{name: "fuzz:grammar:00561", steps: 199, final: 0x00f37280c80a5bca, chain: 0x9dfa766bfa058b38},
	{name: "fuzz:grammar:00562", steps: 404, final: 0xc3bf961b333c9eb4, chain: 0xf6fca323c29ff100},
	{name: "fuzz:grammar:00563", steps: 354, final: 0x3a74155a99970360, chain: 0x686877a2456d644f},
	{name: "fuzz:grammar:00564", steps: 126, final: 0x66061f22574468b7, chain: 0xa179ac2b5244ccf1},
	{name: "fuzz:grammar:00565", steps: 177, final: 0x84a95b5d17df0cbe, chain: 0x2f4bd05a83af406e},
	{name: "fuzz:grammar:00566", steps: 77, final: 0x1e87cc4e27d656af, chain: 0x5e1b36903b77597e},
	{name: "fuzz:grammar:00567", steps: 189, final: 0x85768d2d73b47063, chain: 0xa07efdadcc1fcf86},
	{name: "fuzz:grammar:00568", steps: 370, final: 0x216a107e5f77b0c9, chain: 0x05af655a677d7405},
	{name: "fuzz:grammar:00569", steps: 69, final: 0xc12cf2e390ce57cc, chain: 0x86f167cd260f3557},
	{name: "fuzz:grammar:00570", steps: 320, final: 0xf1b8d25469c83f96, chain: 0x2c3c67743cc16cd9},
	{name: "fuzz:grammar:00571", steps: 162, final: 0x67e3d3945e957821, chain: 0xe1fee9458d3daf0c},
	{name: "fuzz:grammar:00572", steps: 252, final: 0x5fa19eb563b85ce1, chain: 0x20bba35961a3266d},
	{name: "fuzz:grammar:00573", steps: 195, final: 0x193da77c2567328b, chain: 0xbaec00fee88b0859},
	{name: "fuzz:grammar:00574", steps: 299, final: 0x665ec09221522387, chain: 0x25d8a4369f191b38},
	{name: "fuzz:grammar:00575", steps: 329, final: 0x1c491fc8c935ca6a, chain: 0xe19efb59cb02ab1d},
	{name: "fuzz:grammar:00576", steps: 119, final: 0x7d224b6efacc3ee7, chain: 0x27d6c5d1b769821a},
	{name: "fuzz:grammar:00577", steps: 74, final: 0x35d007354b3454fe, chain: 0xbc3bd45f95f8a460},
	{name: "fuzz:grammar:00578", steps: 282, final: 0xc6090430e45bd937, chain: 0x6c134ec7d8a9feb3},
	{name: "fuzz:grammar:00579", steps: 339, final: 0xcaa4539cd5f1e615, chain: 0x6c10293373535a6f},
	{name: "fuzz:grammar:00580", steps: 261, final: 0x7949fb5674e152a7, chain: 0x4e9555b7c136cc91},
	{name: "fuzz:grammar:00581", steps: 252, final: 0xafe8f80b23a2c8ae, chain: 0x1a5ee9bfc4d0a5e2},
	{name: "fuzz:grammar:00582", steps: 213, final: 0x24b785c0caf90159, chain: 0x742ef06967c03140},
	{name: "fuzz:grammar:00583", steps: 363, final: 0xea424f60dd2b5921, chain: 0xecd48d66547b6e54},
	{name: "fuzz:grammar:00584", steps: 170, final: 0xb948b26a684d3e48, chain: 0xfac2b21556534c3b},
	{name: "fuzz:grammar:00585", steps: 114, final: 0x013ae1c27e572efa, chain: 0x7051184b32e1349e},
	{name: "fuzz:grammar:00586", steps: 368, final: 0xe1dc704783a7d7ef, chain: 0x129045b6842ac8b9},
	{name: "fuzz:grammar:00587", steps: 77, final: 0x2b7d2c0ccceb8cee, chain: 0x290ddcbcfe2bb4d6},
	{name: "fuzz:grammar:00588", steps: 168, final: 0xf5c16af720e6c288, chain: 0xf1ee22ce20ea6458},
	{name: "fuzz:grammar:00589", steps: 231, final: 0xd347e1f1f421fe87, chain: 0xe82ce4279c209e93},
	{name: "fuzz:grammar:00590", steps: 96, final: 0x2255eecdf35efef3, chain: 0x8f38683dad67d5dd},
	{name: "fuzz:grammar:00591", steps: 45, final: 0x38a24cd7d3e79e12, chain: 0xabac83275572a2bf},
	{name: "fuzz:grammar:00592", steps: 194, final: 0x1e8db8c82db50b5b, chain: 0xa2a2b69f93c7eb17},
	{name: "fuzz:grammar:00593", steps: 312, final: 0x8e4a1aec2363e522, chain: 0xef6e6c4ddc446968},
	{name: "fuzz:grammar:00594", steps: 378, final: 0x5bef9c8885350055, chain: 0x5ec5de2df5c856a0},
	{name: "fuzz:grammar:00595", steps: 55, final: 0x5bd4a65c03c9b4f6, chain: 0xa6b484b6bd3160ee},
	{name: "fuzz:grammar:00596", steps: 184, final: 0xea699c68749cb818, chain: 0xa2ab5cbb164084fc},
	{name: "fuzz:grammar:00597", steps: 251, final: 0x602615b08ab6c453, chain: 0x8d9c5ac534f5c929},
	{name: "fuzz:grammar:00598", steps: 96, final: 0xd8885552cb48ac36, chain: 0x1090983e0f12e46e},
	{name: "fuzz:grammar:00599", steps: 101, final: 0xd3a57e8c58dc2f60, chain: 0xa5e70865fc7a4cf2},
	{name: "fuzz:grammar:00600", steps: 160, final: 0x8d20c1bc7b1748a7, chain: 0x334ad46ffa798b7b},
	{name: "fuzz:grammar:00601", steps: 279, final: 0xdf481796b9875786, chain: 0x87db600937b7234b},
	{name: "fuzz:grammar:00602", steps: 80, final: 0xe7978922dc2f5779, chain: 0x7d05d010711dc310},
	{name: "fuzz:grammar:00603", steps: 364, final: 0x37dc70715b4e5471, chain: 0x48609d1c5569824d},
	{name: "fuzz:grammar:00604", steps: 192, final: 0xc54ce3f37c285cb0, chain: 0xa596fb0d4fdd46a9},
	{name: "fuzz:grammar:00605", steps: 318, final: 0xffebc2d6b9db856d, chain: 0xefa3de26104b1d84},
	{name: "fuzz:grammar:00606", steps: 171, final: 0x329031335efc78d0, chain: 0xa70529ebac5cca52},
	{name: "fuzz:grammar:00607", steps: 323, final: 0x2ab67da1b474d6f8, chain: 0x750e7c53a49310d6},
	{name: "fuzz:grammar:00608", steps: 342, final: 0xdc23ed8edcef3af7, chain: 0xfa5d074a3f9bf7a0},
	{name: "fuzz:grammar:00609", steps: 328, final: 0x12da4fb36f91f0c8, chain: 0x6492ffdaa7539e4a},
	{name: "fuzz:grammar:00610", steps: 44, final: 0x5030ef365183174f, chain: 0xef26551232d21b84},
	{name: "fuzz:grammar:00611", steps: 283, final: 0x4bf20766e6b2def9, chain: 0x419bd4ebd62bb694},
	{name: "fuzz:grammar:00612", steps: 150, final: 0xee7d67ae1f486260, chain: 0xa8ed2d85600727f1},
	{name: "fuzz:grammar:00613", steps: 281, final: 0xfab7a29fae6cb5dc, chain: 0x6e2c413dc43c28b7},
	{name: "fuzz:grammar:00614", steps: 287, final: 0x4434a66310c15ffe, chain: 0xf4276fc3608b00d0},
	{name: "fuzz:grammar:00615", steps: 362, final: 0x4f395a7e1227804a, chain: 0xc115455948b35e33},
	{name: "fuzz:grammar:00616", steps: 116, final: 0x781dac7a1864a50d, chain: 0x1f01951174319759},
	{name: "fuzz:grammar:00617", steps: 293, final: 0xfbeb79ae19532f38, chain: 0xb23227b696749891},
	{name: "fuzz:grammar:00618", steps: 52, final: 0x0502033f8ae5ca58, chain: 0x09c29c2599348d85},
	{name: "fuzz:grammar:00619", steps: 226, final: 0xb08cb67b504b4f30, chain: 0x0c771b1e90c3b9c2},
	{name: "fuzz:grammar:00620", steps: 352, final: 0xce41a9bffff20072, chain: 0x76cda42fe9f416cc},
	{name: "fuzz:grammar:00621", steps: 291, final: 0xf6bb66681b16e0f5, chain: 0x001f8513eb0863a3},
	{name: "fuzz:grammar:00622", steps: 165, final: 0x10394a76f46bdd3c, chain: 0x84ceb4f58ae7e2e2},
	{name: "fuzz:grammar:00623", steps: 67, final: 0xc7cffddb8462ed4b, chain: 0xcd80c2e5c395b320},
	{name: "fuzz:random:00000", steps: 598, final: 0xcbca34e4f6c61edd, chain: 0x6fb6fae078aa23dd},
	{name: "fuzz:random:00001", steps: 919, final: 0x1be989ebc8b5b2a1, chain: 0xfd4535daa2814d14},
	{name: "fuzz:random:00002", steps: 101, final: 0x8f1ad42cb63976eb, chain: 0x8ae7581701ba221f},
	{name: "fuzz:random:00003", steps: 943, final: 0x1b8e30e3f17a221e, chain: 0xf20bb010e6eae070},
	{name: "fuzz:random:00004", steps: 491, final: 0xdf2c81bd83ae0fe8, chain: 0xcc349d748cbb715d},
	{name: "fuzz:random:00005", steps: 529, final: 0x7ae94fc709648ae9, chain: 0x5553fc9853f9f8bf},
	{name: "fuzz:random:00006", steps: 517, final: 0xc34b245614a30b58, chain: 0xb6623bca7355f83e},
	{name: "fuzz:random:00007", steps: 262, final: 0x2f41d7b879522bf3, chain: 0x0557dbe30097b3b4},
	{name: "fuzz:random:00008", steps: 822, final: 0x143dee36591f5514, chain: 0x60d0153b6b655774},
	{name: "fuzz:random:00009", steps: 651, final: 0xe25214e7da254b01, chain: 0xf7d480ad80e83f19},
	{name: "fuzz:random:00010", steps: 344, final: 0x21634f10c2ae7688, chain: 0x0153bd641b412fca},
	{name: "fuzz:random:00011", steps: 118, final: 0xaf84c7a3c7b1b825, chain: 0x9646f966eb45d6eb},
	{name: "fuzz:random:00012", steps: 347, final: 0x1a3f21682627a2f3, chain: 0x014cf8ec49656248},
	{name: "fuzz:random:00013", steps: 164, final: 0x219fd1314d6cbb18, chain: 0x6badc96c363dc038},
	{name: "fuzz:random:00014", steps: 56, final: 0xdb9585975ebc21ab, chain: 0xb9aece6f73bdb86b},
	{name: "fuzz:random:00015", steps: 755, final: 0xeb8fbc9835783fb1, chain: 0x6f3ab0ef746a32f1},
	{name: "fuzz:random:00016", steps: 894, final: 0xe57d8dff44a593be, chain: 0xf372966dbcbf308b},
	{name: "fuzz:random:00017", steps: 833, final: 0x858868eae828f66d, chain: 0x6883f72353dc0fd6},
	{name: "fuzz:random:00018", steps: 770, final: 0x1a3fe08949567311, chain: 0x25bac2d682a6c49d},
	{name: "fuzz:random:00019", steps: 336, final: 0xefba0f451d75ea6c, chain: 0xa4ab3994e09a74ac},
	{name: "fuzz:random:00020", steps: 535, final: 0x0d82398bce2497b2, chain: 0xf4ecd95bcd9b76a3},
	{name: "fuzz:random:00021", steps: 844, final: 0x6b2497159e2c9a9e, chain: 0x413fda98e4735ebd},
	{name: "fuzz:random:00022", steps: 166, final: 0xcae93f94ed894d6f, chain: 0xaef4eb624376b0de},
	{name: "fuzz:random:00023", steps: 880, final: 0xacc1f7ee31f64efb, chain: 0x7a45f70ce9033044},
	{name: "fuzz:random:00024", steps: 893, final: 0x789dacc8123310df, chain: 0xbe3e2e56e930a78c},
	{name: "fuzz:random:00025", steps: 790, final: 0x1870226ad9ad5cc1, chain: 0xd605e1cb4cc3fb26},
	{name: "fuzz:random:00026", steps: 686, final: 0x1fd3352caf9988a4, chain: 0xe740bafb306a9b4f},
	{name: "fuzz:random:00027", steps: 487, final: 0xbc64354345ce8a18, chain: 0x74802a9973903255},
	{name: "fuzz:random:00028", steps: 490, final: 0x6c30367d2beb62c0, chain: 0x8245c7f325a2cf59},
	{name: "fuzz:random:00029", steps: 79, final: 0x51adf07a24343ad8, chain: 0xb9895e5426fc74bc},
	{name: "fuzz:random:00030", steps: 891, final: 0x7f17fe2c02c4eaea, chain: 0xa994d556f715bdcc},
	{name: "fuzz:random:00031", steps: 1012, final: 0x86344cc6b776d808, chain: 0xf31a6e852034abc5},
	{name: "fuzz:random:00032", steps: 913, final: 0xc2c004f0492202fc, chain: 0x7297e3a967a565a9},
	{name: "fuzz:random:00033", steps: 363, final: 0xf15bc9db83edf777, chain: 0x23b3fe5cb9abe3eb},
	{name: "fuzz:random:00034", steps: 214, final: 0x3e0c7261693f5949, chain: 0xaa2417cd603267c6},
	{name: "fuzz:random:00035", steps: 831, final: 0x7979144fa4d39ae3, chain: 0x7c5290e74ca6b697},
	{name: "fuzz:random:00036", steps: 978, final: 0x22d5213d14d49f69, chain: 0xc50ba179df52f432},
	{name: "fuzz:random:00037", steps: 635, final: 0xb957ce02ceda022a, chain: 0x64311cce32ed7d28},
	{name: "fuzz:random:00038", steps: 311, final: 0xf9f57287bf489e4d, chain: 0x26850db925565dad},
	{name: "fuzz:random:00039", steps: 787, final: 0x1c755e607a83b410, chain: 0xfeeeb79dae94fae4},
	{name: "fuzz:random:00040", steps: 354, final: 0x355c88b1d3543534, chain: 0xbbcd5b855adf096b},
	{name: "fuzz:random:00041", steps: 342, final: 0x2315a79dc26d6bcc, chain: 0x9fb222c1b5abd635},
	{name: "fuzz:random:00042", steps: 569, final: 0x7e9aa1635bc7d789, chain: 0x669e58c238a4e89a},
	{name: "fuzz:random:00043", steps: 433, final: 0xbd4d904b46c1f88f, chain: 0x4a549f816df2c06e},
	{name: "fuzz:random:00044", steps: 770, final: 0xc9636f30141354f7, chain: 0x66b36eb4f721c0fd},
	{name: "fuzz:random:00045", steps: 708, final: 0x6a296c7579aef6c2, chain: 0x75af5f74d378597d},
	{name: "fuzz:random:00046", steps: 357, final: 0xc135bf9abf3468c4, chain: 0x9c48d8cc11249858},
	{name: "fuzz:random:00047", steps: 646, final: 0xead91470c9cbcb48, chain: 0x373ca456920ae313},
	{name: "fuzz:random:00048", steps: 604, final: 0x2272a65d5a4b8c02, chain: 0x4eaeede6ad02c228},
	{name: "fuzz:random:00049", steps: 271, final: 0x291a84d3350595e9, chain: 0x30dec9b7aced043a},
	{name: "fuzz:random:00050", steps: 182, final: 0x5327858854ab6b3c, chain: 0x08c516ddb7340354},
	{name: "fuzz:random:00051", steps: 629, final: 0x022bf09457330a6d, chain: 0xc5cc23b73ff468ad},
	{name: "fuzz:random:00052", steps: 736, final: 0x261b661046fb319a, chain: 0x66fba991dfb08899},
	{name: "fuzz:random:00053", steps: 580, final: 0x21aaf70c94d56315, chain: 0xf595526305a38985},
	{name: "fuzz:random:00054", steps: 985, final: 0x0c6fdcce6361b202, chain: 0x2db02a84114a94d2},
	{name: "fuzz:random:00055", steps: 144, final: 0x79c4c78ca32dfa23, chain: 0x4f0576b766e1cf82},
	{name: "fuzz:random:00056", steps: 658, final: 0xbf43d17649718292, chain: 0xc535802fdf3b7032},
	{name: "fuzz:random:00057", steps: 495, final: 0xcc801791e3ef42f6, chain: 0x1018f3d7ff862c4b},
	{name: "fuzz:random:00058", steps: 436, final: 0x9a952486060b675b, chain: 0x0ab7cc995f6d2397},
	{name: "fuzz:random:00059", steps: 237, final: 0x7af59c168a2a5118, chain: 0x307aafc6960292a1},
	{name: "fuzz:random:00060", steps: 614, final: 0xa08f59124bf3d21f, chain: 0xea4717d33bd76373},
	{name: "fuzz:random:00061", steps: 890, final: 0xfd52716203e85979, chain: 0x8db52b85bc02c85e},
	{name: "fuzz:random:00062", steps: 804, final: 0x4d89c45e265f97dc, chain: 0x038ce1ce8d03b658},
	{name: "fuzz:random:00063", steps: 51, final: 0x16d5538d71772b92, chain: 0x2a6e2cc28018c378},
	{name: "fuzz:random:00064", steps: 635, final: 0x43c2031495f074d7, chain: 0x1ed5064d6397b6fe},
	{name: "fuzz:random:00065", steps: 251, final: 0xa6f5f9dfb2e1805a, chain: 0xf08b08a345e4b8a7},
	{name: "fuzz:random:00066", steps: 650, final: 0xf2d3ba65caa816ad, chain: 0x00197ec0b0fc12df},
	{name: "fuzz:random:00067", steps: 916, final: 0xb4b034aefb2ffb37, chain: 0x4921ec611b0e58fc},
	{name: "fuzz:random:00068", steps: 900, final: 0xcb02acf5b1d5c01d, chain: 0x056702232ac6a1a1},
	{name: "fuzz:random:00069", steps: 454, final: 0x2896c7441eae6ada, chain: 0x20b8b24ed8526401},
	{name: "fuzz:random:00070", steps: 586, final: 0x5ce8cad7dac85c4f, chain: 0x10d6f1c714fa0683},
	{name: "fuzz:random:00071", steps: 506, final: 0xcb1105301f82cd31, chain: 0x34315c87a077b21b},
	{name: "fuzz:random:00072", steps: 68, final: 0x726d12c8029deb7a, chain: 0x8d428f56a554763c},
	{name: "fuzz:random:00073", steps: 347, final: 0xf891f9c27eb75afd, chain: 0x0ba845c7e0302a02},
	{name: "fuzz:random:00074", steps: 723, final: 0xcd536abb56ff12c1, chain: 0x62b9f756de266748},
	{name: "fuzz:random:00075", steps: 940, final: 0x317d7824f72e2660, chain: 0x30dcdb43188f8ac4},
	{name: "fuzz:random:00076", steps: 289, final: 0xb2c46a45be724f96, chain: 0x1c944b2325716aeb},
	{name: "fuzz:random:00077", steps: 942, final: 0xb76978fbd0366fac, chain: 0x137abce6417c4203},
	{name: "fuzz:random:00078", steps: 966, final: 0x85c943fb28975f15, chain: 0x24884056ea11eb46},
	{name: "fuzz:random:00079", steps: 498, final: 0xcbfc7c9c464ad03a, chain: 0x2e07489c7ca21c60},
	{name: "fuzz:random:00080", steps: 618, final: 0x9adf78ec1cc87646, chain: 0x1f37a53b278d6474},
	{name: "fuzz:random:00081", steps: 180, final: 0x649a83325b7db7be, chain: 0x16df974637422545},
	{name: "fuzz:random:00082", steps: 879, final: 0x88fa6dfe71fd9730, chain: 0x6052401dea52ef4d},
	{name: "fuzz:random:00083", steps: 146, final: 0x47ef68335db782ba, chain: 0x978338be97473082},
	{name: "fuzz:random:00084", steps: 144, final: 0xc198696764b495a1, chain: 0xc1b87cae4fd1f38a},
	{name: "fuzz:random:00085", steps: 233, final: 0x22512a7021c31455, chain: 0x67af2796e7ebe8bc},
	{name: "fuzz:random:00086", steps: 910, final: 0xc16e71ce54851430, chain: 0x96382822e290c52d},
	{name: "fuzz:random:00087", steps: 365, final: 0xf44447517285e420, chain: 0x74f7f2d5a31b1312},
	{name: "fuzz:random:00088", steps: 508, final: 0x5aaba2211a5bde84, chain: 0x8a0caa3046de395c},
	{name: "fuzz:random:00089", steps: 491, final: 0xc1556dbb537f5b95, chain: 0x5ee4b457ca5b97c4},
	{name: "fuzz:random:00090", steps: 548, final: 0xb4f189ca568d6894, chain: 0xd17497148706caac},
	{name: "fuzz:random:00091", steps: 324, final: 0x4cd4a8c72064ef7b, chain: 0x24337be1fafe84d3},
	{name: "fuzz:random:00092", steps: 748, final: 0x6f2bc1a2005c7962, chain: 0x3fed19a722605df6},
	{name: "fuzz:random:00093", steps: 162, final: 0xc642a3d95d4dea1d, chain: 0x86c863519d4205ea},
	{name: "fuzz:random:00094", steps: 53, final: 0xa586b3470d817784, chain: 0x1c83706aef5e45a9},
	{name: "fuzz:random:00095", steps: 885, final: 0x07ad544eba9f3e54, chain: 0xe1e15981cf118bb6},
	{name: "fuzz:random:00096", steps: 763, final: 0x7e453181851af2b2, chain: 0xa1c68c71bace67ed},
	{name: "fuzz:random:00097", steps: 737, final: 0xbab57d798ca4d6c9, chain: 0x4fd23af746dc50e5},
	{name: "fuzz:random:00098", steps: 369, final: 0x8398211920358d50, chain: 0x344f33a43b112f7f},
	{name: "fuzz:random:00099", steps: 274, final: 0x36c2257dfa1aaa53, chain: 0x0955a24ca859a935},
	{name: "fuzz:random:00100", steps: 260, final: 0x567ce93635955326, chain: 0xd671bed92533bb80},
	{name: "fuzz:random:00101", steps: 790, final: 0x16c1194952b142a2, chain: 0xe2f158296c8b2051},
	{name: "fuzz:random:00102", steps: 167, final: 0xf3ec04774972e18b, chain: 0x3106d0ead7262f5e},
	{name: "fuzz:random:00103", steps: 388, final: 0xe8e87f76e688104a, chain: 0x757f2731439b93cd},
	{name: "fuzz:random:00104", steps: 308, final: 0x963040159949fe4f, chain: 0xa84819133ef5dc08},
	{name: "fuzz:random:00105", steps: 733, final: 0xf6402f0fb90875ab, chain: 0x2fcdcc7198cb3703},
	{name: "fuzz:random:00106", steps: 532, final: 0x2e1b8c2c034416cf, chain: 0xd7081eb43b25913c},
	{name: "fuzz:random:00107", steps: 665, final: 0x664811ea404f324b, chain: 0x8ce3f97894b5480a},
	{name: "fuzz:random:00108", steps: 167, final: 0xf8c70891056a7ee6, chain: 0x48d507ac67072593},
	{name: "fuzz:random:00109", steps: 753, final: 0x721bbf8cd824b514, chain: 0x9ad5f3d3a96c4edb},
	{name: "fuzz:random:00110", steps: 42, final: 0x1b04b5eaf5543586, chain: 0x6b7daf3baa2531b3},
	{name: "fuzz:random:00111", steps: 99, final: 0x39a7b8ac6bab7af2, chain: 0x5fcbbe589eb009b9},
	{name: "fuzz:random:00112", steps: 654, final: 0x37aeda968879b35b, chain: 0x809229fa4b570b3d},
	{name: "fuzz:random:00113", steps: 854, final: 0xa3197d093389b02d, chain: 0x07d4a52d5b117283},
	{name: "fuzz:random:00114", steps: 608, final: 0xf7577aa7b644ade6, chain: 0x851083946228d69e},
	{name: "fuzz:random:00115", steps: 321, final: 0x38ea00037f734b9a, chain: 0x2435f08c926d56c2},
	{name: "fuzz:random:00116", steps: 219, final: 0x84f12ad2c118ef6d, chain: 0x1df4400f2d83997d},
	{name: "fuzz:random:00117", steps: 640, final: 0xe468729d14323c49, chain: 0x2a64ab8cf290064b},
	{name: "fuzz:random:00118", steps: 374, final: 0x0df9bfd9fe4ef2b4, chain: 0xc522f4d3556a0c66},
	{name: "fuzz:random:00119", steps: 190, final: 0x8063b38915476133, chain: 0xeca7fe5dd69e92c0},
	{name: "fuzz:random:00120", steps: 749, final: 0x896c8b471057acc4, chain: 0x650aca0bd07f8e71},
	{name: "fuzz:random:00121", steps: 729, final: 0x8ee32f9f55e39eef, chain: 0x7a6d505154cd1fb9},
	{name: "fuzz:random:00122", steps: 801, final: 0xdf6ed5b357e8b27f, chain: 0xf30fae43a75b2693},
	{name: "fuzz:random:00123", steps: 457, final: 0x79a66af0c03ec66f, chain: 0x568fa4e390c056c4},
	{name: "fuzz:random:00124", steps: 83, final: 0x93e48fd8f5d2b3b3, chain: 0xcadb5640d0141760},
	{name: "fuzz:random:00125", steps: 59, final: 0x9bf75fab5fa9ba73, chain: 0xda37a72d7c714ef2},
	{name: "fuzz:random:00126", steps: 748, final: 0x63ce4c1cedae13f1, chain: 0xc856e97df9b2e47a},
	{name: "fuzz:random:00127", steps: 974, final: 0x94aa4b9c37a2f4a3, chain: 0xd0d6ed7ae7d9a4e4},
	{name: "fuzz:random:00128", steps: 475, final: 0xe4c61e8f3055187b, chain: 0x6c9753b95f6669d1},
	{name: "fuzz:random:00129", steps: 41, final: 0xda17b0fd3cb9c308, chain: 0x0ed26e88bc3e6070},
	{name: "fuzz:random:00130", steps: 793, final: 0x768204713bb12452, chain: 0x5fa9ae8fbfba73ff},
	{name: "fuzz:random:00131", steps: 429, final: 0x7e68a450095d1fd6, chain: 0x85e556011a19e855},
	{name: "fuzz:random:00132", steps: 921, final: 0xfa95c14b513e49a2, chain: 0x457661d092a67eab},
	{name: "fuzz:random:00133", steps: 65, final: 0x7d9e59e491022492, chain: 0xafb58e73d065619c},
	{name: "fuzz:random:00134", steps: 385, final: 0xbaf9afac8463673c, chain: 0xc8f87089e04d5f32},
	{name: "fuzz:random:00135", steps: 644, final: 0x3f42619b383a52bd, chain: 0xec6e902ce9d7b15a},
	{name: "fuzz:random:00136", steps: 177, final: 0x252a208f03d60331, chain: 0xadfe7ace10691c27},
	{name: "fuzz:random:00137", steps: 751, final: 0xdcbc62045a62462c, chain: 0x85ffb015274d695b},
	{name: "fuzz:random:00138", steps: 166, final: 0x81dc526006bd71e8, chain: 0x77e7855c73b564cb},
	{name: "fuzz:random:00139", steps: 755, final: 0x1edb7fe7cf7149c3, chain: 0xa11b8ec34cb17f94},
	{name: "fuzz:random:00140", steps: 418, final: 0x576ed03a8ad7ac90, chain: 0xba81cc72a1323bcd},
	{name: "fuzz:random:00141", steps: 563, final: 0x86200d7365533aae, chain: 0xa619630572db87f9},
	{name: "fuzz:random:00142", steps: 621, final: 0xcd4be16723cdb38d, chain: 0x5ad4da9144f7abd7},
	{name: "fuzz:random:00143", steps: 929, final: 0x923cc140fa6f886a, chain: 0x6f098a6c286c71a5},
	{name: "fuzz:random:00144", steps: 922, final: 0x9353dd3e7c5aa38d, chain: 0x31a9484cc4a57bb6},
	{name: "fuzz:random:00145", steps: 152, final: 0x065cb15e3d9a3b3a, chain: 0x514874508796b84e},
	{name: "fuzz:random:00146", steps: 583, final: 0x1cbcd8fc84455e73, chain: 0x9deea3c8d4add0bb},
	{name: "fuzz:random:00147", steps: 87, final: 0x8a700e8d87871c60, chain: 0x56d9b002fa185c74},
	{name: "fuzz:random:00148", steps: 204, final: 0x02b61cc449f305d1, chain: 0x3f9c78b3a60d806f},
	{name: "fuzz:random:00149", steps: 437, final: 0x1550e91b2f5e2a44, chain: 0xfd4ca06dd218be30},
	{name: "fuzz:random:00150", steps: 952, final: 0x0d148735e82a0570, chain: 0xf8c342a1f14a2dea},
	{name: "fuzz:random:00151", steps: 862, final: 0x99e2f4f5f173a956, chain: 0xedbed45aaf0fcaf2},
	{name: "fuzz:random:00152", steps: 228, final: 0x77c7b1f6324e1097, chain: 0x00c9267d77a38aae},
	{name: "fuzz:random:00153", steps: 351, final: 0x51466db2f8b02955, chain: 0xb6e7b3a11a157764},
	{name: "fuzz:random:00154", steps: 806, final: 0x3ba1c7e4101d9b6b, chain: 0xb7453456a4e86e51},
	{name: "fuzz:random:00155", steps: 261, final: 0x642fa806f07dd6fd, chain: 0x31bdb69f29295ad4},
	{name: "fuzz:random:00156", steps: 222, final: 0xa7b06f5fccc67c71, chain: 0x0db376ef4a05f414},
	{name: "fuzz:random:00157", steps: 510, final: 0x6bafd0f098a02296, chain: 0xce1204ad4c2aadb5},
	{name: "fuzz:random:00158", steps: 155, final: 0x96c2e75d2e82b04c, chain: 0x8f209631178011d7},
	{name: "fuzz:random:00159", steps: 407, final: 0x1b8dfa87baf45669, chain: 0x7196c6caf383a26f},
	{name: "fuzz:random:00160", steps: 144, final: 0x1dd5e3c93398d663, chain: 0x34c860b6764a34c0},
	{name: "fuzz:random:00161", steps: 695, final: 0x5c1cd242ed4fe1cb, chain: 0xee11bb227fa8ffd6},
	{name: "fuzz:random:00162", steps: 539, final: 0x519bc7b7abfe12f9, chain: 0x31d64bf0320a61e0},
	{name: "fuzz:random:00163", steps: 430, final: 0xe25edb6256dc79df, chain: 0xbed1f87ba92181b7},
	{name: "fuzz:random:00164", steps: 814, final: 0x79336c2f76d93209, chain: 0xe0c7f64988504b1f},
	{name: "fuzz:random:00165", steps: 867, final: 0xef3232d911c8d6d9, chain: 0x124ffeeb90ac0d11},
	{name: "fuzz:random:00166", steps: 366, final: 0x22b3fb902069683f, chain: 0xac9007ff46288ab4},
	{name: "fuzz:random:00167", steps: 458, final: 0xdf1facc53af56b2c, chain: 0xf7d41d0664245dbb},
	{name: "fuzz:random:00168", steps: 758, final: 0xf6276d8bf3c1bed9, chain: 0x8885295f72f5eca6},
	{name: "fuzz:random:00169", steps: 743, final: 0x28237b3fb0000d2b, chain: 0xe81c7946b87b8598},
	{name: "fuzz:random:00170", steps: 78, final: 0xf6b3a8dd8788754e, chain: 0x12f04a2eacbbf31c},
	{name: "fuzz:random:00171", steps: 316, final: 0xb88be8a794595044, chain: 0x6aac8e1ac332b829},
	{name: "fuzz:random:00172", steps: 744, final: 0x6e571080cc7ae69e, chain: 0x9b86e7919964eb72},
	{name: "fuzz:random:00173", steps: 376, final: 0x6ebe9b56dd924e2b, chain: 0x4cb64386503886bd},
	{name: "fuzz:random:00174", steps: 831, final: 0x64b7ac5af837fdf8, chain: 0x7aa2a587616b2630},
	{name: "fuzz:random:00175", steps: 255, final: 0x04f906086935453f, chain: 0x040a269cf073f71a},
	{name: "fuzz:random:00176", steps: 849, final: 0x27644c7f09cd52bf, chain: 0xeb70625f013804ac},
	{name: "fuzz:random:00177", steps: 102, final: 0xab11d7bab17a7925, chain: 0xd412f3234c5f409e},
	{name: "fuzz:random:00178", steps: 110, final: 0x2bb7b175c77d999b, chain: 0x93073fd9fd98b4af},
	{name: "fuzz:random:00179", steps: 575, final: 0x1e612aecb3d85370, chain: 0x993587039c4309e8},
	{name: "fuzz:random:00180", steps: 647, final: 0xba1103a8762709b1, chain: 0xb8ad36ed7f1ae5cb},
	{name: "fuzz:random:00181", steps: 857, final: 0x7a3a1cb8c8540d54, chain: 0xa263ec64a2ff4847},
	{name: "fuzz:random:00182", steps: 878, final: 0x35b494a99c80628c, chain: 0x1e813fce28675e2b},
	{name: "fuzz:random:00183", steps: 606, final: 0x93970af0286f1188, chain: 0xd870b612f9d32ffa},
	{name: "fuzz:random:00184", steps: 274, final: 0xa4842dba83590319, chain: 0xff3aa3635bf86452},
	{name: "fuzz:random:00185", steps: 817, final: 0xfb197aac078cd6c3, chain: 0xf2c86871d7aab835},
	{name: "fuzz:random:00186", steps: 685, final: 0x3b77d7088ed7ebe8, chain: 0xc6add94d12d598f0},
	{name: "fuzz:random:00187", steps: 48, final: 0x55cb2759fb91104c, chain: 0x205251ee51420bfa},
	{name: "fuzz:random:00188", steps: 510, final: 0x1b5fcfc8a0d66dee, chain: 0x2d7741e641d87411},
	{name: "fuzz:random:00189", steps: 943, final: 0x5a9e1ddf9f39ff1f, chain: 0xa2d30adfd4780ffc},
	{name: "fuzz:random:00190", steps: 106, final: 0x2bdcd95177445d10, chain: 0x75ed9a1f576cc76f},
	{name: "fuzz:random:00191", steps: 306, final: 0x02019bb7b490eee4, chain: 0xb159ea8cdb7460dd},
	{name: "fuzz:random:00192", steps: 333, final: 0x2f85fd66f1d6b413, chain: 0x852cbd04d74055d3},
	{name: "fuzz:random:00193", steps: 99, final: 0xc5082a9fbd3b1abc, chain: 0x4440cdb71fa132ea},
	{name: "fuzz:random:00194", steps: 598, final: 0xba6013b4d465738f, chain: 0x2462fefc3fc6f8f6},
	{name: "fuzz:random:00195", steps: 883, final: 0x2595074e112c129c, chain: 0x37911818776c51c0},
	{name: "fuzz:random:00196", steps: 80, final: 0xc3cced5e8369eccc, chain: 0x6d7710aefbfaf4d0},
	{name: "fuzz:random:00197", steps: 490, final: 0xdc6069593319ced5, chain: 0x718cf611ca1842aa},
	{name: "fuzz:random:00198", steps: 407, final: 0xfe407260dea6b773, chain: 0x31f99da009d666cb},
	{name: "fuzz:random:00199", steps: 114, final: 0x183aeea1aaf5bf2d, chain: 0x57aef720a41b1481},
	{name: "fuzz:random:00200", steps: 557, final: 0xfb586e744ee67c84, chain: 0x660a6d8a553e3fb5},
	{name: "fuzz:random:00201", steps: 21, final: 0xd7280c11cf99bd81, chain: 0xcf12376499bc9d75},
	{name: "fuzz:random:00202", steps: 316, final: 0xd7a96ca084cc17f6, chain: 0x146b8550ab63a268},
	{name: "fuzz:random:00203", steps: 98, final: 0xc1df5eb9811fac10, chain: 0x8d4f90ea3aeb00e4},
	{name: "fuzz:random:00204", steps: 718, final: 0xbca45761eff106e8, chain: 0x8a12b52db53c7089},
	{name: "fuzz:random:00205", steps: 482, final: 0x9af7df6be0362a79, chain: 0x888cec0db0713cce},
	{name: "fuzz:random:00206", steps: 939, final: 0x273e6f4246a0d706, chain: 0x44671855fa065a3d},
	{name: "fuzz:random:00207", steps: 245, final: 0xe73bc83f043e56c3, chain: 0xe9f6dccd056aab19},
	{name: "fuzz:random:00208", steps: 86, final: 0xae4e3db559a9767d, chain: 0x1924d326064f3e36},
	{name: "fuzz:random:00209", steps: 295, final: 0xa7603c5ef3449716, chain: 0xbe494c4686e64550},
	{name: "fuzz:random:00210", steps: 792, final: 0xd56ebf44a5b4f50c, chain: 0x5fa59ee6c4d26c61},
	{name: "fuzz:random:00211", steps: 618, final: 0x181cdd9a074c8b4b, chain: 0xb77c401a0a85a0ca},
	{name: "fuzz:random:00212", steps: 74, final: 0x94708c34459eb142, chain: 0x2b11c7ea8585ba7c},
	{name: "fuzz:random:00213", steps: 398, final: 0xaafe0a4a8fa224b2, chain: 0xca9a3caa5b6d35df},
	{name: "fuzz:random:00214", steps: 377, final: 0xbad5bd4dbc7a08b8, chain: 0x9b9272311a0da025},
	{name: "fuzz:random:00215", steps: 624, final: 0xb410768733b5625d, chain: 0x753569a388d2906d},
	{name: "fuzz:random:00216", steps: 889, final: 0xe648f075057fe29d, chain: 0xeca662c4c716d8d8},
	{name: "fuzz:random:00217", steps: 762, final: 0x6fc46577f8d51717, chain: 0x8debf4a02f6883bf},
	{name: "fuzz:random:00218", steps: 59, final: 0x615ec09965ad6923, chain: 0xf3123281c79b6a84},
	{name: "fuzz:random:00219", steps: 130, final: 0x9becdbc2614ee6f4, chain: 0x6cd60bcf82b3bf56},
	{name: "fuzz:random:00220", steps: 766, final: 0x5a9d8bfa64d2f81d, chain: 0xe9ab995eb9cc1dcd},
	{name: "fuzz:random:00221", steps: 803, final: 0x96510c218a8d7343, chain: 0x78fe7929707096b8},
	{name: "fuzz:random:00222", steps: 48, final: 0x0aeacbbb56fa4bd7, chain: 0xad6f1f9e2aa0a50a},
	{name: "fuzz:random:00223", steps: 523, final: 0xc2123e721665f293, chain: 0xb03532955d99de8c},
	{name: "fuzz:random:00224", steps: 666, final: 0x1ef796f95cf53dac, chain: 0x43dda44dfcecbb5c},
	{name: "fuzz:random:00225", steps: 490, final: 0xd51144641241afd5, chain: 0x1cd0cb2767f67f26},
	{name: "fuzz:random:00226", steps: 902, final: 0x89420b2a83694b28, chain: 0x93819063e70b8a95},
	{name: "fuzz:random:00227", steps: 65, final: 0x1f07c01c5e2387c0, chain: 0x556cb59cac44f0b1},
	{name: "fuzz:random:00228", steps: 601, final: 0xd03f6a52ffa036f1, chain: 0xff4fd190e0804793},
	{name: "fuzz:random:00229", steps: 421, final: 0x56ccf475d405b480, chain: 0x9b9f04227d9ba887},
	{name: "fuzz:random:00230", steps: 361, final: 0x04d533c623b151d0, chain: 0xd253c00defd620e3},
	{name: "fuzz:random:00231", steps: 88, final: 0xbe328426cee37116, chain: 0x0964b7a435120aa0},
	{name: "fuzz:random:00232", steps: 468, final: 0x37c44e993686ccac, chain: 0xdf9e39d6a0779312},
	{name: "fuzz:random:00233", steps: 477, final: 0xf6b7c6e4da0d05f7, chain: 0xec59b0d79e76a6af},
	{name: "fuzz:random:00234", steps: 648, final: 0x19219bed2a75e157, chain: 0x67d2b37dd5a94439},
	{name: "fuzz:random:00235", steps: 159, final: 0xfee0f1c877ab72c8, chain: 0xed3a417e43c7a12f},
	{name: "fuzz:random:00236", steps: 203, final: 0xc28a353508f74842, chain: 0x9188122ec12e1d8f},
	{name: "fuzz:random:00237", steps: 462, final: 0x69f0c351f4251046, chain: 0xfe9b8f4dbbdf0c2e},
	{name: "fuzz:random:00238", steps: 25, final: 0x05fc473c6fbbb5f0, chain: 0x05c6059f47c4976a},
	{name: "fuzz:random:00239", steps: 776, final: 0x2ae9724e5f4588a6, chain: 0x788ebc500dfe9d35},
	{name: "fuzz:random:00240", steps: 488, final: 0xe53a19f096d95853, chain: 0x9d33a79a669cdf6b},
	{name: "fuzz:random:00241", steps: 233, final: 0xca358596bf13d0c5, chain: 0xb85c54dc243689a3},
	{name: "fuzz:random:00242", steps: 204, final: 0xd3eab7f752097e1d, chain: 0x07e3613f546b4788},
	{name: "fuzz:random:00243", steps: 101, final: 0xf2b4b5ede9f4311b, chain: 0x6a5b2143fa5c3e88},
	{name: "fuzz:random:00244", steps: 455, final: 0xf05a2b2ea1dac77a, chain: 0x4ab38ffff63adba2},
	{name: "fuzz:random:00245", steps: 332, final: 0x384da288a2a678d7, chain: 0xf1095889525be12b},
	{name: "fuzz:random:00246", steps: 205, final: 0x07c7b61adb086256, chain: 0x1f2b652e47e9b157},
	{name: "fuzz:random:00247", steps: 544, final: 0xafabb7c6bd2d8945, chain: 0xe7f136a12cfb8975},
	{name: "fuzz:random:00248", steps: 668, final: 0x055a3ffc291486a3, chain: 0xa82f9be9977a1657},
	{name: "fuzz:random:00249", steps: 329, final: 0x1192987b088e084f, chain: 0x55723ba3b100b6a3},
	{name: "fuzz:random:00250", steps: 582, final: 0x87d812b494a66167, chain: 0xe4d35577145cecef},
	{name: "fuzz:random:00251", steps: 472, final: 0x85e5eb629500fe3b, chain: 0xf1b68652f557b988},
	{name: "fuzz:random:00252", steps: 389, final: 0xb098c4bbacf90107, chain: 0xf6d7b0e607f5588d},
	{name: "fuzz:random:00253", steps: 214, final: 0xe4c74661b40598e7, chain: 0x0be66e96ebc0a936},
	{name: "fuzz:random:00254", steps: 370, final: 0xefca8fa5f52498d7, chain: 0x53d1acc6a2e9e75b},
	{name: "fuzz:random:00255", steps: 187, final: 0x132a800060487816, chain: 0x5110143ffa5453cc},
	{name: "fuzz:random:00256", steps: 926, final: 0xfce776ed7cfbeabc, chain: 0x82db8251b78c56f6},
	{name: "fuzz:random:00257", steps: 779, final: 0x84fe658d5bca4fad, chain: 0x562aaf0c266804e7},
	{name: "fuzz:random:00258", steps: 36, final: 0x800baf4ecdad7808, chain: 0x9e73861fa3b926f0},
	{name: "fuzz:random:00259", steps: 400, final: 0x49a30efa6d95b133, chain: 0x76e76de53e077b11},
	{name: "fuzz:random:00260", steps: 757, final: 0x46740e99f0508b70, chain: 0xbc91b96e378c24ec},
	{name: "fuzz:random:00261", steps: 839, final: 0x1835db80f6dea8bc, chain: 0x1e7e908461645ea9},
	{name: "fuzz:random:00262", steps: 862, final: 0xf2df3b5f758b33a2, chain: 0x60fae020967680f0},
	{name: "fuzz:random:00263", steps: 888, final: 0x545b2e9f00421497, chain: 0xf2067ba9c41d0830},
	{name: "fuzz:random:00264", steps: 526, final: 0x51b7ec027e0b7994, chain: 0xaf0185887d0764a2},
	{name: "fuzz:random:00265", steps: 306, final: 0x4e0a8ab2b26adcbf, chain: 0x2e4e5cce67ea5f28},
	{name: "fuzz:random:00266", steps: 224, final: 0x9b95c4aa9640b76f, chain: 0x57809567cad7e2f7},
	{name: "fuzz:random:00267", steps: 83, final: 0x614c1688f838a722, chain: 0x42b084330df26a2e},
	{name: "fuzz:random:00268", steps: 77, final: 0x4e466055d19ed43a, chain: 0x4b069fd977f4f6a0},
	{name: "fuzz:random:00269", steps: 730, final: 0xccf469cf81e10b05, chain: 0xd5aceddc200e9e95},
	{name: "fuzz:random:00270", steps: 891, final: 0xb956ebfd427cba68, chain: 0xbacabf6cfe429b62},
	{name: "fuzz:random:00271", steps: 680, final: 0xacb075ac81666535, chain: 0x3b9350dd6aef8c87},
	{name: "fuzz:random:00272", steps: 673, final: 0xc59b7a2ec0aed65c, chain: 0x6c86ec849bee8639},
	{name: "fuzz:random:00273", steps: 899, final: 0xda035e3f1c34ef86, chain: 0x76abb37fc0b629a8},
	{name: "fuzz:random:00274", steps: 945, final: 0x540acb89e6fe0c75, chain: 0xbb93cbfda2369e98},
	{name: "fuzz:random:00275", steps: 771, final: 0x1a67efdab0722e93, chain: 0x23379ed771f27fbf},
	{name: "fuzz:random:00276", steps: 637, final: 0x813aadf7b28383dd, chain: 0x0faee5209dad6839},
	{name: "fuzz:random:00277", steps: 369, final: 0x453e0dd06d278e26, chain: 0xe914c77975095391},
	{name: "fuzz:random:00278", steps: 883, final: 0xd629bd2d66b0ef1a, chain: 0x25d9329501d97d78},
	{name: "fuzz:random:00279", steps: 866, final: 0xe7edac54287c2047, chain: 0xcdb8bcd512ab4ee1},
	{name: "fuzz:random:00280", steps: 866, final: 0xaf2c96ac1b579818, chain: 0x93cee2180b097adf},
	{name: "fuzz:random:00281", steps: 950, final: 0x76f835b1b277d224, chain: 0x02bbc2c0dfd1090d},
	{name: "fuzz:random:00282", steps: 708, final: 0x6b9000c5a87add01, chain: 0x0a7795630d6438c9},
	{name: "fuzz:random:00283", steps: 370, final: 0x41fc9d689535f812, chain: 0xae77046bb7e692cc},
	{name: "fuzz:random:00284", steps: 159, final: 0x0e9e9244f6da2ed4, chain: 0x11921d9a720bc9e2},
	{name: "fuzz:random:00285", steps: 147, final: 0x8de8278f45353c36, chain: 0x8e0a645c9c92dcfb},
	{name: "fuzz:random:00286", steps: 753, final: 0x5aca923ad7ff9438, chain: 0xcf09d6233dfc1b30},
	{name: "fuzz:random:00287", steps: 623, final: 0x7f00330a530e5c0d, chain: 0x15ca462d25323cfc},
	{name: "fuzz:random:00288", steps: 44, final: 0x97676a5562f655d8, chain: 0x610270df8455003a},
	{name: "fuzz:random:00289", steps: 86, final: 0x48fec6ddc263b475, chain: 0xee8a541d20d90cfb},
	{name: "fuzz:random:00290", steps: 754, final: 0x94b58236ec4bcfb9, chain: 0x696ffed96a53c999},
	{name: "fuzz:random:00291", steps: 157, final: 0x24118e89ab65901b, chain: 0xebb2bfe4ddf7987a},
	{name: "fuzz:random:00292", steps: 833, final: 0x7b69a2fcb968840b, chain: 0xf1c2a6abafc18794},
	{name: "fuzz:random:00293", steps: 591, final: 0x6305520f9d8666ad, chain: 0x1dd03dd3b8927d50},
	{name: "fuzz:random:00294", steps: 596, final: 0xb26a1d44007914da, chain: 0x755d3b61a6adccae},
	{name: "fuzz:random:00295", steps: 735, final: 0xee14393c214691fb, chain: 0xd0e39e35119241c2},
	{name: "fuzz:random:00296", steps: 663, final: 0x00e32a855db9e617, chain: 0xd374e74050b1cce2},
	{name: "fuzz:random:00297", steps: 362, final: 0x323257810a7fdb66, chain: 0xae8ad20abda3e1fa},
	{name: "fuzz:random:00298", steps: 362, final: 0x3afad6caa7b7d4f5, chain: 0x378a419487b7e0a0},
	{name: "fuzz:random:00299", steps: 709, final: 0x2bff0bfb8c680979, chain: 0x2d9c943ee6177c19},
	{name: "fuzz:random:00300", steps: 515, final: 0xc7cd95a0da458947, chain: 0xcda54fe2f7db5101},
	{name: "fuzz:random:00301", steps: 636, final: 0xb2c2acf5f86b4391, chain: 0x4d5b1a830314f1ef},
	{name: "fuzz:random:00302", steps: 344, final: 0x9ab1745238324fc1, chain: 0x5f4ea5b2bf16f7ed},
	{name: "fuzz:random:00303", steps: 540, final: 0x34eedaf95175f14c, chain: 0x08e045cd625b3a98},
	{name: "fuzz:random:00304", steps: 1020, final: 0xde6b07b6ffb21950, chain: 0xb058eefbb2dfe69f},
	{name: "fuzz:random:00305", steps: 131, final: 0x44ecf0b574cb30dd, chain: 0x9a1ca88f654c43f6},
	{name: "fuzz:random:00306", steps: 618, final: 0x9d5fb4ce70ff5c94, chain: 0xf85d5432e4aa75cc},
	{name: "fuzz:random:00307", steps: 618, final: 0xa50e6bb06af15e31, chain: 0xdec3720319f64ebf},
	{name: "fuzz:random:00308", steps: 236, final: 0x4aa9e9b3e953e3a0, chain: 0xd06ff18c97a0d2a1},
	{name: "fuzz:random:00309", steps: 514, final: 0x97cb80f5ee308d87, chain: 0x40cc0073c8161159},
	{name: "fuzz:random:00310", steps: 929, final: 0x4118a7c45894b336, chain: 0xbb7555119b070096},
	{name: "fuzz:random:00311", steps: 950, final: 0xc1518b38c3854917, chain: 0x2801e260cccaa7af},
	{name: "fuzz:random:00312", steps: 749, final: 0xa8472e06e2f3467e, chain: 0x41bef113a2cf0910},
	{name: "fuzz:random:00313", steps: 246, final: 0xbc112fd4709dfc43, chain: 0x30569f939c912929},
	{name: "fuzz:random:00314", steps: 717, final: 0xba8dfd5b8f35a52a, chain: 0x3c7cb6367038ee35},
	{name: "fuzz:random:00315", steps: 797, final: 0x26e03911a683d107, chain: 0x115ff921b1a1026b},
	{name: "fuzz:random:00316", steps: 288, final: 0xc0cd1474d45d081d, chain: 0xb4a326233e6391c9},
	{name: "fuzz:random:00317", steps: 77, final: 0xb22b1a10f38fae8a, chain: 0x953df9ed4705b981},
	{name: "fuzz:random:00318", steps: 144, final: 0xb214cce7252e5b60, chain: 0x87773d6f63bbed40},
	{name: "fuzz:random:00319", steps: 794, final: 0xb10fafc4b1a9828e, chain: 0x6dceecf69d49ea4d},
	{name: "fuzz:random:00320", steps: 798, final: 0x34b14659c17faeb3, chain: 0xb9e3c9a13727882a},
	{name: "fuzz:random:00321", steps: 912, final: 0x868180d51a868b76, chain: 0x99d2ce16715f1245},
	{name: "fuzz:random:00322", steps: 646, final: 0xf74a47be343602ef, chain: 0xfee3fa2126621ee1},
	{name: "fuzz:random:00323", steps: 501, final: 0x9267de153f667e89, chain: 0x2d3d169a7a2fb9bc},
	{name: "fuzz:random:00324", steps: 868, final: 0x08c6a6384d51a297, chain: 0x1910e0c9c679ede3},
	{name: "fuzz:random:00325", steps: 970, final: 0x32a26922a33c97ce, chain: 0x7cef2adb053ba2b9},
	{name: "fuzz:random:00326", steps: 221, final: 0x2dc0c62b490d96e7, chain: 0x7d0910c18a47aabe},
	{name: "fuzz:random:00327", steps: 135, final: 0x6d09ab97ac66dd7a, chain: 0x9a30e10df7025e94},
	{name: "fuzz:random:00328", steps: 562, final: 0x2999b6ae97ac21bc, chain: 0xdb20ec5eb2e65053},
	{name: "fuzz:random:00329", steps: 239, final: 0x9d796b2376421aa9, chain: 0x55a6bccadd70ce49},
	{name: "fuzz:random:00330", steps: 909, final: 0xa60e65fdb1c826a9, chain: 0x2b54718fd80a8f61},
	// END GENERATED
}

// fixtureIOError is a fixture I/O failure of errno kind with its own message, the Go form of
// Rust's io::Error::new(kind, message): errors.Is matches the kind, and Error is the message.
type fixtureIOError struct {
	kind    syscall.Errno
	message string
}

func (e fixtureIOError) Error() string { return e.message }

func (e fixtureIOError) Is(target error) bool { return target == e.kind }

// fakeInput is terminal input that first fails interruptions reads with EINTR, then yields its
// queued chunks one per read, then EAGAIN, or end of file once eof is set.
type fakeInput struct {
	chunks        [][]byte
	eof           bool
	consumed      []byte
	interruptions int
}

func newFakeInput(data []byte, width int) *fakeInput {
	if width <= 0 {
		panic("a fake input chunk width is positive")
	}
	input := &fakeInput{}
	for len(data) > 0 {
		count := min(width, len(data))
		input.chunks = append(input.chunks, slices.Clone(data[:count]))
		data = data[count:]
	}
	return input
}

func (f *fakeInput) push(chunk []byte) {
	f.chunks = append(f.chunks, slices.Clone(chunk))
}

func (f *fakeInput) tryRead(target []byte) (int, error) {
	if f.interruptions > 0 {
		f.interruptions--
		return 0, syscall.EINTR
	}
	if len(f.chunks) == 0 {
		if f.eof {
			return 0, nil
		}
		return 0, syscall.EAGAIN
	}
	chunk := f.chunks[0]
	count := copy(target, chunk)
	if count < len(chunk) {
		f.chunks[0] = chunk[count:]
	} else {
		f.chunks = f.chunks[1:]
	}
	f.consumed = append(f.consumed, target[:count]...)
	return count, nil
}

// fakePty is a PTY whose child side reads from output and whose master side records every
// offer. failure refuses every write, fault only the next one.
type fakePty struct {
	output   *fakeInput
	reads    int
	written  []byte
	offers   [][]byte
	blocked  bool
	failure  opt[syscall.Errno]
	fault    opt[syscall.Errno]
	maxWrite int
	sizes    []terminalSize
	closed   bool
}

func newFakePty(output []byte, width int) *fakePty {
	return &fakePty{output: newFakeInput(output, width), maxWrite: math.MaxInt}
}

func (f *fakePty) tryRead(target []byte) (int, error) {
	f.reads++
	return f.output.tryRead(target)
}

func (f *fakePty) tryWrite(data []byte) (int, error) {
	if len(data) == 0 {
		panic("a PTY write offers bytes")
	}
	if f.blocked {
		return 0, syscall.EAGAIN
	}
	f.offers = append(f.offers, slices.Clone(data))
	kind, faulted := f.fault.get()
	if faulted {
		f.fault = none[syscall.Errno]()
	} else {
		kind, faulted = f.failure.get()
	}
	if faulted {
		return 0, fixtureIOError{kind: kind, message: "fixture PTY write failure"}
	}
	count := min(len(data), f.maxWrite)
	f.written = append(f.written, data[:count]...)
	return count, nil
}

func (f *fakePty) resize(size terminalSize) error {
	f.sizes = append(f.sizes, size)
	return nil
}

func (f *fakePty) close() error {
	f.closed = true
	return nil
}

// fakeDisplay is a display that records accepted bytes; ordering logs each write as "display".
type fakeDisplay struct {
	bytes    []byte
	blocked  bool
	broken   bool
	maxWrite opt[int]
	ordering *[]string
}

func (f *fakeDisplay) tryWrite(data []byte) (int, error) {
	if f.broken {
		return 0, syscall.EPIPE
	}
	if f.blocked {
		return 0, syscall.EAGAIN
	}
	if f.ordering != nil {
		*f.ordering = append(*f.ordering, "display")
	}
	count := len(data)
	if limit, set := f.maxWrite.get(); set {
		count = min(count, limit)
	}
	f.bytes = append(f.bytes, data[:count]...)
	return count, nil
}

func (f *fakeDisplay) tryFlush() error {
	if f.blocked {
		return syscall.EAGAIN
	}
	return nil
}

var (
	_ byteInput = (*fakeInput)(nil)
	_ ptyIo     = (*fakePty)(nil)
	_ display   = (*fakeDisplay)(nil)
)
