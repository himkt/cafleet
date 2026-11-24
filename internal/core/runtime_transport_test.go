package core

import (
	"encoding/binary"
	"encoding/hex"
	"fmt"
	"math"
	"math/rand/v2"
	"runtime"
	"strconv"
	"strings"
	"sync"
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
	// END GENERATED
}
