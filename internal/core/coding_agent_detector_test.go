package core

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"maps"
	"math"
	"os"
	"slices"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/himkt/cafleet/internal/vt/unicodewidth"
)

// The Rust detector test_support module.

// matchAnswer observes screen and matches the one plan against it; a plan of another backend or
// request rule than the observation, which production never offers, matches nothing.
func (d *detector) matchAnswer(screen *screenSnapshot, plan *answerPlan) (answerMatch, bool, error) {
	observed, err := d.observe(screen)
	if err != nil {
		return answerMatch{}, false, err
	}
	if plan.capability.backend != d.backend || observed.ruleID != plan.capability.requestRuleID {
		return answerMatch{}, false, nil
	}
	matches, err := d.matchAnswers(screen, &observed, []answerPlan{*plan})
	if err != nil {
		return answerMatch{}, false, err
	}
	if len(matches) == 0 {
		return answerMatch{}, false, nil
	}
	return matches[len(matches)-1], true, nil
}

func bundledScreen(id string) string { return bundledFixture(id) }

func observeBundled(t testing.TB, backend codingAgent, version string, screen *screenSnapshot) screenObservation {
	t.Helper()
	observed, err := newDetector(profileCatalogBundled(), backend, some(version)).observe(screen)
	if err != nil {
		t.Fatalf("observe: %v", err)
	}
	return observed
}

// screenOf is a 24-row, 80-column screen after bytes.
func screenOf(t testing.TB, bytes []byte) screenSnapshot {
	t.Helper()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, terminal, bytes)
	return terminal.snapshot()
}

func fixtureDocument(id string) any { return parseTestJSON(bundledScreen(id)) }

func fixtureLines(id string) []string {
	var lines []string
	for _, line := range jarray(jget(fixtureDocument(id), "lines")).items {
		text, ok := jstring(line)
		if !ok {
			panic("a fixture line is a string")
		}
		lines = append(lines, text)
	}
	return lines
}

func fixtureSize(id string) terminalSize { return sizeOf(fixtureDocument(id)) }

// replayAsCaptured replays a bundled screen on the screen it was captured on.
func replayAsCaptured(id string, change func(*[]string), cursor opt[[2]uint16]) screenSnapshot {
	alternate := fixtureAlternate(fixtureDocument(id))
	return replayNativeFixture(bundledScreen(id), change, alternate, cursor)
}

// fixtureAlternate is the screen mode a replay paints a fixture on. A herdr capture records no
// screen mode (null); it paints on the primary screen and its replay is observed cursor-free.
func fixtureAlternate(screen any) bool {
	switch mode := jget(screen, "alternate_screen").(type) {
	case bool:
		return mode
	case nil:
		return false
	default:
		panic("a fixture screen mode is a boolean or null: " + testJSONText(mode))
	}
}

// fixtureCursor is the fixture's recorded cursor, or none for a herdr capture, which records
// none.
func fixtureCursor(screen any) opt[[2]uint16] {
	coordinate := func(key string) opt[uint16] {
		value := jget(screen, key)
		if value == nil {
			return none[uint16]()
		}
		number, ok := ju64(value)
		if !ok || number > 0xffff {
			panic("a fixture cursor coordinate")
		}
		return some(uint16(number))
	}
	row, hasRow := coordinate("cursor_row").get()
	col, hasCol := coordinate("cursor_col").get()
	switch {
	case hasRow && hasCol:
		return some([2]uint16{row, col})
	case !hasRow && !hasCol:
		return none[[2]uint16]()
	default:
		panic("a fixture cursor records both coordinates or neither")
	}
}

// assertPrimaryMatchesAlternate: Claude rules read either screen, so a primary-screen replay is
// the same live screen.
func assertPrimaryMatchesAlternate(t testing.TB, name string, primary, alternate *screenObservation) {
	t.Helper()
	if primary.kind != alternate.kind || primary.ruleID != alternate.ruleID || primary.matchEvidence != alternate.matchEvidence ||
		primary.verifiedNoninteraction != alternate.verifiedNoninteraction {
		t.Fatalf("%s: primary %s %s %s %v, alternate %s %s %s %v", name,
			primary.kind.debugString(), primary.ruleID, primary.matchEvidence.debugString(), primary.verifiedNoninteraction,
			alternate.kind.debugString(), alternate.ruleID, alternate.matchEvidence.debugString(), alternate.verifiedNoninteraction)
	}
	if primary.rawEvidence.operation != alternate.rawEvidence.operation {
		t.Fatalf("%s: operation %v, alternate %v", name, primary.rawEvidence.operation, alternate.rawEvidence.operation)
	}
	if primary.rawEvidence.operationRows != alternate.rawEvidence.operationRows {
		t.Fatalf("%s: operation_rows %v, alternate %v", name, primary.rawEvidence.operationRows, alternate.rawEvidence.operationRows)
	}
	if primary.rawEvidence.choices.set != alternate.rawEvidence.choices.set ||
		!slices.Equal(primary.rawEvidence.choices.values, alternate.rawEvidence.choices.values) {
		t.Fatalf("%s: choices %v, alternate %v", name, primary.rawEvidence.choices, alternate.rawEvidence.choices)
	}
	if !fingerprintsEqual(primary.fingerprint, alternate.fingerprint) {
		t.Fatalf("%s: the fingerprints differ", name)
	}
}

func fingerprintsEqual(a, b *semanticFingerprint) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	return a.equal(b)
}

func replayNativeFixture(fixture string, change func(*[]string), alternate bool, cursor opt[[2]uint16]) screenSnapshot {
	screen := parseTestJSON(fixture)
	terminal, err := newTerminalState(sizeOf(screen))
	if err != nil {
		panic(err)
	}
	if err := terminal.feed(fixtureBytes(screen, change, alternate, cursor)); err != nil {
		panic(err)
	}
	snapshot := terminal.snapshot()
	_, override := cursor.get()
	_, recorded := fixtureCursor(screen).get()
	if !override && !recorded {
		snapshot.cursor = none[[2]uint16]()
		snapshot.alternateScreen = none[bool]()
	}
	return snapshot
}

// nativeFixtureBytes is the byte stream that paints a committed native screen, using its own
// screen mode and cursor.
func nativeFixtureBytes(fixture string) []byte {
	screen := parseTestJSON(fixture)
	return fixtureBytes(screen, nil, fixtureAlternate(screen), none[[2]uint16]())
}

func sizeOf(screen any) terminalSize {
	rows, rowsOK := ju64(jget(screen, "rows"))
	cols, colsOK := ju64(jget(screen, "cols"))
	if !rowsOK || !colsOK || rows > 0xffff || cols > 0xffff {
		panic("a fixture geometry")
	}
	return terminalSize{rows: uint16(rows), cols: uint16(cols)}
}

func fixtureBytes(screen any, change func(*[]string), alternate bool, cursor opt[[2]uint16]) []byte {
	var lines []string
	for _, line := range jarray(jget(screen, "lines")).items {
		text, ok := jstring(line)
		if !ok {
			panic("a fixture line is a string")
		}
		lines = append(lines, text)
	}
	if change != nil {
		change(&lines)
	}
	var bytes []byte
	if alternate {
		bytes = append(bytes, "\x1b[?1049h"...)
	}
	bytes = append(bytes, "\x1b[2J"...)
	for row, line := range lines {
		if line != "" {
			bytes = fmt.Appendf(bytes, "\x1b[%d;1H%s", row+1, line)
		}
	}
	written, err := newTerminalState(sizeOf(screen))
	if err != nil {
		panic(err)
	}
	if err := written.feed(bytes); err != nil {
		panic(err)
	}
	text := written.snapshot()
	bytes = append(bytes, attributeRewrite(screen, &text)...)
	at, set := cursor.get()
	if !set {
		at, set = fixtureCursor(screen).get()
	}
	if set {
		bytes = fmt.Appendf(bytes, "\x1b[%d;%dH", at[0]+1, at[1]+1)
	}
	return bytes
}

func colourSGR(layer int, colour any) string {
	channel := func(key string) uint64 {
		number, ok := ju64(jget(colour, key))
		if !ok {
			panic("a fixture colour channel")
		}
		return number
	}
	switch kind := jget(colour, "kind"); kind {
	case "rgb":
		return fmt.Sprintf("%d;2;%d;%d;%d", layer, channel("r"), channel("g"), channel("b"))
	case "indexed":
		return fmt.Sprintf("%d;5;%d", layer, channel("index"))
	default:
		panic(fmt.Sprintf("unknown fixture colour kind %v", kind))
	}
}

type runCell struct {
	row, col uint16
	run      any
}

func runCells(fixture any, key string) []runCell {
	runs, ok := jget(fixture, key).(*jsonArr)
	if !ok {
		return nil
	}
	var cells []runCell
	for _, run := range runs.items {
		row, rowOK := ju64(jget(run, "row"))
		start, startOK := ju64(jget(run, "start_col"))
		end, endOK := ju64(jget(run, "end_col"))
		if !rowOK || !startOK || !endOK || row > math.MaxUint16 || start > math.MaxUint16 || end > math.MaxUint16 {
			panic("a fixture run geometry")
		}
		for col := uint16(start); col < uint16(end); col++ {
			cells = append(cells, runCell{row: uint16(row), col: col, run: run})
		}
	}
	return cells
}

// attributeRewrite repaints every cell a cell_backgrounds or cell_styles run covers over the
// already written text, so the replayed cells keep their captured contents and gain the
// captured background, faint and foreground attributes.
func attributeRewrite(fixture any, text *screenSnapshot) []byte {
	cells := map[[2]uint16][]string{}
	for _, cell := range runCells(fixture, "cell_backgrounds") {
		key := [2]uint16{cell.row, cell.col}
		cells[key] = append(cells[key], colourSGR(48, jget(cell.run, "bg")))
	}
	for _, cell := range runCells(fixture, "cell_styles") {
		key := [2]uint16{cell.row, cell.col}
		if faint, _ := jget(cell.run, "faint").(bool); faint {
			cells[key] = append(cells[key], "2")
		}
		if foreground := jget(cell.run, "foreground"); foreground != nil {
			cells[key] = append(cells[key], colourSGR(38, foreground))
		}
		if _, ok := cells[key]; !ok {
			cells[key] = nil
		}
	}
	keys := make([][2]uint16, 0, len(cells))
	for key := range cells {
		keys = append(keys, key)
	}
	slices.SortFunc(keys, func(a, b [2]uint16) int {
		if a[0] != b[0] {
			return int(a[0]) - int(b[0])
		}
		return int(a[1]) - int(b[1])
	})
	var bytes []byte
	open := false
	var atRow, next uint16
	var sgr string
	for _, key := range keys {
		row, col := key[0], key[1]
		attributes := strings.Join(cells[key], ";")
		continues := open && atRow == row && next == col && sgr == attributes
		if !continues {
			if open {
				bytes = append(bytes, "\x1b[0m"...)
			}
			bytes = fmt.Appendf(bytes, "\x1b[%d;%dH\x1b[%sm", row+1, col+1, attributes)
		}
		open, atRow, next, sgr = true, row, col+1, attributes
		cell := text.screen.Cell(row, col)
		if cell == nil {
			panic("run inside geometry")
		}
		if cell.IsWideContinuation() {
			continue
		}
		if cell.HasContents() {
			bytes = append(bytes, cell.Contents()...)
		} else {
			bytes = append(bytes, ' ')
		}
	}
	if open {
		bytes = append(bytes, "\x1b[0m"...)
	}
	return bytes
}

// permissionLines is the Rust test_support::lines: the synthetic permission form.
func permissionLines(operation string) []string {
	return []string{
		"Synthetic permission request",
		"Operation: " + operation,
		"> Allow once",
		"  Deny once",
		"Request form: synthetic-only",
		"Selection: ",
	}
}

// syntheticTerminal is the Rust test_support::terminal: lines painted at the bottom of a 24-row
// screen.
func syntheticTerminal(t testing.TB, lines []string, cols uint16, alternate bool) *terminalState {
	t.Helper()
	if len(lines) > 24 {
		t.Fatal("at most 24 lines")
	}
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: cols})
	if alternate {
		mustFeed(t, terminal, []byte("\x1b[?1049h"))
	}
	mustFeed(t, terminal, []byte("\x1b[2J"))
	for index, line := range lines {
		mustFeed(t, terminal, fmt.Appendf(nil, "\x1b[%d;1H%s", 25-len(lines)+index, line))
	}
	return terminal
}

// syntheticObservation is the Rust test_support::observation.
func syntheticObservation(t testing.TB, definition any, screen *screenSnapshot) screenObservation {
	t.Helper()
	catalog := mustLoadCatalog(t, definition)
	observed, err := newDetector(catalog, codingAgentCodex, some(syntheticVersion)).observe(screen)
	if err != nil {
		t.Fatalf("observe: %v", err)
	}
	return observed
}

// permissionObservation is the Rust test_support::permission.
func permissionObservation(t testing.TB, operation string) screenObservation {
	t.Helper()
	snapshot := syntheticTerminal(t, permissionLines(operation), 80, false).snapshot()
	return syntheticObservation(t, syntheticDefinitions(), &snapshot)
}

func stateObservation(t testing.TB, kind string, verified bool) screenObservation {
	t.Helper()
	snapshot := syntheticTerminal(t, []string{"Synthetic state", "State end"}, 80, false).snapshot()
	return syntheticObservation(t, simpleDefinition(kind, verified), &snapshot)
}

// composerTextRecord is the structural composer text migration/golden.py compares: region, rows,
// mode prefix and segments, with each text segment's word-wrap breaks as rune offsets.
func composerTextRecord(text composerText) any {
	segments := ja()
	for _, s := range text.segments {
		switch segment := s.(type) {
		case segmentText:
			var chars []rune
			breaks := ja()
			for _, item := range segment.items {
				if item.isBreak {
					breaks.push(len(chars))
					continue
				}
				chars = append(chars, item.char)
			}
			segments.push(jo("text", string(chars), "breaks", breaks))
		case segmentScalars:
			segments.push(jo("scalars", segment.count))
		case segmentLines:
			segments.push(jo("lines", segment.count))
		default:
			panic("invalid segment")
		}
	}
	modePrefix := any(nil)
	if prefix, set := text.modePrefix.get(); set {
		modePrefix = prefix
	}
	return jo("region", ja(text.region.top, text.region.bottom), "rows", text.rows, "mode_prefix", modePrefix, "segments", segments)
}

// detectorRecord is one golden record: the screen's geometry and mode, the bundled detector's
// observation and its composer text.
func detectorRecord(name string, backend codingAgent, version string, snapshot *screenSnapshot) (*jsonObj, error) {
	d := newDetector(profileCatalogBundled(), backend, some(version))
	observed, err := d.observe(snapshot)
	if err != nil {
		return nil, fmt.Errorf("%s: observe: %w", name, err)
	}
	composer, found, err := d.composerText(snapshot)
	if err != nil {
		return nil, fmt.Errorf("%s: composer text: %w", name, err)
	}
	cursor := any(nil)
	if at, set := snapshot.cursor.get(); set {
		cursor = ja(at[0], at[1])
	}
	alternate := any(nil)
	if mode, set := snapshot.alternateScreen.get(); set {
		alternate = mode
	}
	region := any(nil)
	if matched, set := observed.matchedRegion.get(); set {
		region = ja(matched.top, matched.bottom)
	}
	operation := any(nil)
	if text, set := observed.rawEvidence.operation.get(); set {
		operation = text
	}
	choices := any(nil)
	if observed.rawEvidence.choices.set {
		choices = jsonNormalize(observed.rawEvidence.choices.values)
	}
	composerValue := any(nil)
	if found {
		composerValue = composerTextRecord(composer)
	}
	return jo(
		"name", name,
		"size", ja(snapshot.rows, snapshot.cols),
		"cursor", cursor,
		"alternate_screen", alternate,
		"kind", observed.kind.debugString(),
		"rule_id", observed.ruleID,
		"match_evidence", observed.matchEvidence.debugString(),
		"verified_noninteraction", observed.verifiedNoninteraction,
		"matched_region", region,
		"operation", operation,
		"choices", choices,
		"composer_text", composerValue,
	), nil
}

type bundledFixtureRecord struct {
	backend codingAgent
	id      string
	version string
}

// bundledFixtureRecords lists every bundled fixture: each profile file in registry order, each
// screen in file order.
func bundledFixtureRecords() []bundledFixtureRecord {
	var fixtures []bundledFixtureRecord
	for _, member := range parsedBundledFiles().members {
		backend, ok := codingAgentCodingAgent(member.key)
		if !ok {
			panic("a bundled backend name")
		}
		for _, screen := range jobject(jget(member.value, "screens")).members {
			version, ok := jstring(jget(screen.value, "native_version"))
			if !ok {
				panic("a fixture native version")
			}
			fixtures = append(fixtures, bundledFixtureRecord{backend: backend, id: screen.key, version: version})
		}
	}
	return fixtures
}

// TestCodingAgentDetector_GoldenExport is the Go side of migration/golden.py (gate G4): it writes
// CAFLEET_GOLDEN_OUT with one record per bundled fixture, replayed as captured, and per checkpoint
// of each stream in CAFLEET_GOLDEN_CHECKPOINTS, replayed through TerminalState.
func TestCodingAgentDetector_GoldenExport(t *testing.T) {
	t.Parallel()
	checkpoints := os.Getenv("CAFLEET_GOLDEN_CHECKPOINTS")
	if checkpoints == "" {
		t.Skip("CAFLEET_GOLDEN_CHECKPOINTS is unset; migration/golden.py drives this test")
	}
	outPath := os.Getenv("CAFLEET_GOLDEN_OUT")
	if outPath == "" {
		t.Fatal("CAFLEET_GOLDEN_OUT names the output")
	}
	var out []byte
	for _, fixture := range bundledFixtureRecords() {
		snapshot := replayAsCaptured(fixture.id, nil, none[[2]uint16]())
		record, err := detectorRecord("fixture:"+fixture.id, fixture.backend, fixture.version, &snapshot)
		if err != nil {
			t.Fatal(err)
		}
		out = append(append(out, jsonBytes(record)...), '\n')
	}
	input, err := os.Open(checkpoints)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := input.Close(); err != nil {
			t.Error(err)
		}
	}()
	scanner := bufio.NewScanner(input)
	scanner.Buffer(nil, 1<<30)
	for scanner.Scan() {
		stream := parseTestJSON(scanner.Text())
		backendName, _ := jstring(jget(stream, "backend"))
		backend, ok := codingAgentCodingAgent(backendName)
		if !ok {
			t.Fatalf("a bundled backend name: %q", backendName)
		}
		version, ok := jstring(jget(stream, "native_version"))
		if !ok {
			t.Fatal("a native version")
		}
		terminal := mustTerminalState(t, sizeOf(stream))
		steps := jarray(jget(stream, "steps")).items
		fed := 0
		for _, checkpoint := range jarray(jget(stream, "checkpoints")).items {
			count, ok := ju64(jget(checkpoint, "steps"))
			if !ok || count > math.MaxInt32 {
				panic("a checkpoint step count: " + testJSONText(checkpoint))
			}
			upto := int(count)
			if upto < fed || upto > len(steps) {
				t.Fatalf("a checkpoint step count: %s", testJSONText(checkpoint))
			}
			for _, step := range steps[fed:upto] {
				if text, isHex := jstring(jget(step, "hex")); isHex {
					data, err := hex.DecodeString(text)
					if err != nil {
						t.Fatal(err)
					}
					mustFeed(t, terminal, data)
					continue
				}
				resize := jget(step, "resize")
				mustResize(t, terminal, sizeOf(jo("rows", jget(resize, 0), "cols", jget(resize, 1))))
			}
			fed = upto
			name, _ := jstring(jget(checkpoint, "name"))
			snapshot := terminal.snapshot()
			record, err := detectorRecord(name, backend, version, &snapshot)
			if err != nil {
				t.Fatal(err)
			}
			out = append(append(out, jsonBytes(record)...), '\n')
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(outPath, out, 0o600); err != nil {
		t.Fatal(err)
	}
}

type detectorFixtureGolden struct {
	fixture string
	screen  uint64
	record  string
}

type fixtureEpisode struct {
	fixture     string
	fingerprint *semanticFingerprint
}

// TestCodingAgentDetector_FixtureGolden records, per bundled native screen fixture replayed as
// captured through TerminalState, the screen hash, the bundled detector's observation and
// composer text, and its episode: the first fixture in golden order whose observation carries an
// equal fingerprint.
func TestCodingAgentDetector_FixtureGolden(t *testing.T) {
	t.Parallel()
	var got []detectorFixtureGolden
	var episodes []fixtureEpisode
	for _, fixture := range bundledFixtureRecords() {
		snapshot := replayAsCaptured(fixture.id, nil, none[[2]uint16]())
		record, err := detectorRecord("fixture:"+fixture.id, fixture.backend, fixture.version, &snapshot)
		if err != nil {
			t.Fatal(err)
		}
		record.remove("name")
		observed := observeBundled(t, fixture.backend, fixture.version, &snapshot)
		episode := any(nil)
		if observed.fingerprint != nil {
			episode = fixture.id
			for _, earlier := range episodes {
				if fingerprintsEqual(earlier.fingerprint, observed.fingerprint) {
					episode = earlier.fixture
					break
				}
			}
			episodes = append(episodes, fixtureEpisode{fixture: fixture.id, fingerprint: observed.fingerprint})
		}
		record.set("episode", episode)
		got = append(got, detectorFixtureGolden{
			fixture: fixture.id,
			screen:  fnv1a(canonicalScreenDump(snapshot.screen, &terminalTitles{}, nil)),
			record:  testJSONText(record),
		})
	}
	if *updateGolden {
		var body strings.Builder
		for _, entry := range got {
			fmt.Fprintf(&body, "\t{fixture: %q, screen: 0x%016x, record: %s},\n", entry.fixture, entry.screen, goStringLiteral(entry.record))
		}
		updateGoldenBlock(t, "coding_agent_detector_test.go", body.String())
		return
	}
	if len(got) != len(detectorFixtureGoldens) {
		t.Fatalf("the golden block records %d fixtures, the bundled catalog has %d; regenerate with -update-golden", len(detectorFixtureGoldens), len(got))
	}
	for index, entry := range got {
		if want := detectorFixtureGoldens[index]; entry != want {
			t.Errorf("%s: got screen %016x %s\nwant %s screen %016x %s", entry.fixture, entry.screen, entry.record, want.fixture, want.screen, want.record)
		}
	}
}

// goStringLiteral is a raw string literal of text when it allows one, else a quoted literal.
func goStringLiteral(text string) string {
	if !strings.Contains(text, "`") && utf8.ValidString(text) && !strings.ContainsFunc(text, func(r rune) bool { return r < 0x20 || r == 0x7f }) {
		return "`" + text + "`"
	}
	return fmt.Sprintf("%q", text)
}

func assertPermissionObservation(t testing.TB, label string, value *screenObservation) {
	t.Helper()
	if value.kind != observationKindPermissionRequest || value.matchEvidence != matchEvidenceComplete || value.fingerprint == nil {
		t.Fatalf("%s: %s %s fingerprint %v, want a complete permission request", label, value.kind.debugString(), value.matchEvidence.debugString(), value.fingerprint != nil)
	}
}

func assertNoPermission(t testing.TB, label string, value *screenObservation) {
	t.Helper()
	if value.kind == observationKindPermissionRequest {
		t.Fatalf("%s: a permission request is observed", label)
	}
}

func syntheticObserved(t testing.TB, definition any, terminal *terminalState) screenObservation {
	t.Helper()
	snapshot := terminal.snapshot()
	return syntheticObservation(t, definition, &snapshot)
}

// wrappedPermission is the synthetic permission form written from the top of a cols-wide screen.
func wrappedPermission(t testing.TB, operation string, cols uint16) screenSnapshot {
	t.Helper()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: cols})
	mustFeed(t, terminal, []byte("\x1b[2J\x1b[H"+strings.Join(permissionLines(operation), "\r\n")))
	return terminal.snapshot()
}

func evidenceKeys(t testing.TB, evidence observationEvidence) []string {
	t.Helper()
	encoded, err := encodeJSON(evidence)
	if err != nil {
		t.Fatal(err)
	}
	keys := jobject(parseTestJSON(string(encoded))).keys()
	slices.Sort(keys)
	return keys
}

func TestCodingAgentDetector_RuntimeDetectorCompleteSyntheticFormReturnsItsLiveRegionAndSemantics(t *testing.T) {
	t.Parallel()
	screen := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false).snapshot()
	value := syntheticObservation(t, syntheticDefinitions(), &screen)
	assertPermissionObservation(t, "the synthetic form", &value)
	if value.rawEvidence.nativeVersion != some(syntheticVersion) || value.ruleID != syntheticRule || value.outputSeq != screen.screenSeq {
		t.Fatalf("version %v rule %s seq %d", value.rawEvidence.nativeVersion, value.ruleID, value.outputSeq)
	}
	if value.matchedRegion != some(screenRegion{top: 18, bottom: 24}) {
		t.Fatalf("region %v", value.matchedRegion)
	}
	if value.rawEvidence.operation != some("write disposable.txt") {
		t.Fatalf("operation %v", value.rawEvidence.operation)
	}
	if !value.rawEvidence.choices.set || !slices.Equal(value.rawEvidence.choices.values, []string{"Allow once", "Deny once"}) {
		t.Fatalf("choices %v", value.rawEvidence.choices)
	}
	if slices.Contains(evidenceKeys(t, value.rawEvidence), "cwd") || value.verifiedNoninteraction {
		t.Fatal("the request carries no cwd and verifies no noninteraction")
	}
}

func TestCodingAgentDetector_RuntimeDetectorEveryRequiredLiveComponentIsIndispensable(t *testing.T) {
	t.Parallel()
	original := permissionLines("write disposable.txt")
	complete := syntheticObserved(t, syntheticDefinitions(), syntheticTerminal(t, original, 80, false))
	assertPermissionObservation(t, "the original form", &complete)
	for index := range original {
		changed := slices.Clone(original)
		changed[index] = "unrelated output"
		observed := syntheticObserved(t, syntheticDefinitions(), syntheticTerminal(t, changed, 80, false))
		assertNoPermission(t, fmt.Sprintf("row %d replaced", index), &observed)
	}
}

func TestCodingAgentDetector_RuntimeDetectorMissingOrEmptyConfiguredCaptureKeepsTheRuleIncomplete(t *testing.T) {
	t.Parallel()
	definition := syntheticDefinitions()
	jset(definition, "^Operation: (?P<operation>.*)$", "codex", "profile", "rules", 0, "matchers", 0, "pattern")
	present := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false))
	assertPermissionObservation(t, "a captured operation", &present)
	empty := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines(""), 80, false))
	assertNoPermission(t, "an empty operation", &empty)
	jset(definition, "^Operation: (?:write disposable.txt|(?P<operation>other))$", "codex", "profile", "rules", 0, "matchers", 0, "pattern")
	missing := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false))
	assertNoPermission(t, "an unmatched capture group", &missing)
}

func TestCodingAgentDetector_RuntimeDetectorDuplicateMatchersOrAmbiguousLiveRegionsRemainIncomplete(t *testing.T) {
	t.Parallel()
	duplicate := slices.Insert(permissionLines("write disposable.txt"), 2, "Operation: another disposable.txt")
	observed := syntheticObserved(t, syntheticDefinitions(), syntheticTerminal(t, duplicate, 80, false))
	assertNoPermission(t, "a duplicate matcher row", &observed)
	definition := syntheticDefinitions()
	jset(definition, "any", "codex", "profile", "rules", 0, "region", "cursor")
	one := permissionLines("write disposable.txt")
	single := syntheticObserved(t, definition, syntheticTerminal(t, one, 80, false))
	assertPermissionObservation(t, "one form", &single)
	two := syntheticObserved(t, definition, syntheticTerminal(t, append(slices.Clone(one), one...), 80, false))
	assertNoPermission(t, "two forms", &two)
}

func TestCodingAgentDetector_RuntimeDetectorRegionLimitsScreenSelectionAndCursorAreEnforced(t *testing.T) {
	t.Parallel()
	screen := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false).snapshot()
	definition := syntheticDefinitions()
	jset(definition, 6, "codex", "profile", "rules", 0, "region", "max_rows")
	six := syntheticObservation(t, definition, &screen)
	assertPermissionObservation(t, "max_rows 6", &six)
	jset(definition, 5, "codex", "profile", "rules", 0, "region", "max_rows")
	five := syntheticObservation(t, definition, &screen)
	assertNoPermission(t, "max_rows 5", &five)
	definition = syntheticDefinitions()
	jset(definition, "alternate", "codex", "profile", "rules", 0, "region", "screen")
	primary := syntheticObservation(t, definition, &screen)
	assertNoPermission(t, "an alternate rule on the primary screen", &primary)
	alternate := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt"), 80, true))
	assertPermissionObservation(t, "an alternate rule on the alternate screen", &alternate)
	moved := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false)
	mustFeed(t, moved, []byte("\x1b[1;1H"))
	atEnd := syntheticObserved(t, syntheticDefinitions(), moved)
	assertNoPermission(t, "a cursor off the end", &atEnd)
	definition = syntheticDefinitions()
	jset(definition, "inside", "codex", "profile", "rules", 0, "region", "cursor")
	inside := syntheticObserved(t, definition, moved)
	assertNoPermission(t, "a cursor outside the region", &inside)
}

func TestCodingAgentDetector_RuntimeDetectorCursorEndpointUsesWideDisplayCells(t *testing.T) {
	t.Parallel()
	definition := syntheticDefinitions()
	jset(definition, "^Selection界: $", "codex", "profile", "rules", 0, "region", "end_pattern")
	rows := permissionLines("write disposable.txt")
	rows[len(rows)-1] = "Selection界: "
	terminal := syntheticTerminal(t, rows, 80, false)
	if cursor, _ := terminal.snapshot().cursor.get(); cursor[1] != 13 {
		t.Fatalf("cursor column %d, want 13", cursor[1])
	}
	atEnd := syntheticObserved(t, definition, terminal)
	assertPermissionObservation(t, "the cursor after the wide cell", &atEnd)
	mustFeed(t, terminal, []byte("\x1b[24;13H"))
	before := syntheticObserved(t, definition, terminal)
	assertNoPermission(t, "the cursor one cell early", &before)
}

func TestCodingAgentDetector_RuntimeDetectorSoftWrapPreservesIdentityWhileSignificantSpacesChangeIt(t *testing.T) {
	t.Parallel()
	const operation = "printf 'a significant quoted operation with repeated  internal spaces'"
	wideScreen := wrappedPermission(t, operation, 100)
	narrowScreen := wrappedPermission(t, operation, 40)
	wide := syntheticObservation(t, syntheticDefinitions(), &wideScreen)
	narrow := syntheticObservation(t, syntheticDefinitions(), &narrowScreen)
	assertPermissionObservation(t, "wide", &wide)
	assertPermissionObservation(t, "narrow", &narrow)
	if !fingerprintsEqual(wide.fingerprint, narrow.fingerprint) || wide.rawEvidence.operation != narrow.rawEvidence.operation {
		t.Fatal("a soft wrap keeps the request identity")
	}
	changed := permissionObservation(t, "printf 'a significant quoted operation with repeated internal spaces'")
	assertPermissionObservation(t, "changed", &changed)
	if fingerprintsEqual(wide.fingerprint, changed.fingerprint) {
		t.Fatal("a significant space changes the identity")
	}
}

func TestCodingAgentDetector_RuntimeDetectorRegionExtractionPreservesHardNewlines(t *testing.T) {
	t.Parallel()
	definition := syntheticDefinitions()
	jset(definition, parseTestJSON(`{"id":"operation","scope":"region","pattern":"(?m)^Operation: (?P<operation>[\\s\\S]+?)\\n> Allow once$"}`), "codex", "profile", "rules", 0, "matchers", 0)
	firstScreen := wrappedPermission(t, "printf 'ab'", 80)
	secondScreen := wrappedPermission(t, "printf 'a\r\nb'", 80)
	first := syntheticObservation(t, definition, &firstScreen)
	second := syntheticObservation(t, definition, &secondScreen)
	assertPermissionObservation(t, "first", &first)
	assertPermissionObservation(t, "second", &second)
	if second.rawEvidence.operation != some("printf 'a\nb'") || fingerprintsEqual(first.fingerprint, second.fingerprint) {
		t.Fatalf("second operation %v", second.rawEvidence.operation)
	}
}

func TestCodingAgentDetector_RuntimeDetectorDecorationAndGeometryKeepStableSemantics(t *testing.T) {
	t.Parallel()
	value := permissionObservation(t, "write disposable.txt")
	terminal := syntheticTerminal(t, permissionLines("write disposable.txt"), 100, false)
	mustFeed(t, terminal, []byte("\x1b[1;1Hspinner 123\x1b[24;12H"))
	redraw := syntheticObserved(t, syntheticDefinitions(), terminal)
	assertPermissionObservation(t, "the redraw", &redraw)
	if !fingerprintsEqual(value.fingerprint, redraw.fingerprint) {
		t.Fatal("decoration and geometry keep the identity")
	}
	changed := permissionObservation(t, "write another-disposable.txt")
	if fingerprintsEqual(value.fingerprint, changed.fingerprint) {
		t.Fatal("another operation is another request")
	}
}

func singleProfileScreen(t testing.TB) screenSnapshot {
	t.Helper()
	rows := append([]string{"Live footer", "Refused: write outside.txt", "Refusal end"}, permissionLines("write disposable.txt")...)
	return syntheticTerminal(t, rows, 80, false).snapshot()
}

func TestCodingAgentDetector_RuntimeDetectorFingerprintsCarryNoNativeVersion(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, singleProfiles([]singleProfile{{codingAgentCodex, "only"}}))
	screen := singleProfileScreen(t)
	var observed []screenObservation
	for _, run := range []opt[string]{some("1.4.2"), some("3.1.4"), none[string]()} {
		value, err := newDetector(catalog, codingAgentCodex, run).observe(&screen)
		if err != nil {
			t.Fatal(err)
		}
		if value.kind != observationKindPermissionRequest || value.fingerprint == nil {
			t.Fatalf("%v: %s", run, value.kind.debugString())
		}
		observed = append(observed, value)
	}
	if !fingerprintsEqual(observed[0].fingerprint, observed[1].fingerprint) || !fingerprintsEqual(observed[0].fingerprint, observed[2].fingerprint) {
		t.Fatal("the permission request keeps its identity whatever the run's native version")
	}
}

func TestCodingAgentDetector_RuntimeDetectorRawEvidenceNamesTheBackendAndNativeVersionOnly(t *testing.T) {
	t.Parallel()
	catalog := syntheticCatalog(t)
	screen := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false).snapshot()
	for _, version := range []opt[string]{some(syntheticVersion), none[string]()} {
		observed, err := newDetector(catalog, codingAgentCodex, version).observe(&screen)
		if err != nil {
			t.Fatal(err)
		}
		want := []string{"backend", "choices", "excerpt", "native_version", "operation", "operation_rows", "rule_id", "screen_seq", "truncated"}
		if got := evidenceKeys(t, observed.rawEvidence); !slices.Equal(got, want) {
			t.Fatalf("%v: evidence keys %q", version, got)
		}
		if observed.rawEvidence.backend != "codex" || observed.rawEvidence.nativeVersion != version {
			t.Fatalf("%v: backend %q version %v", version, observed.rawEvidence.backend, observed.rawEvidence.nativeVersion)
		}
	}
}

func TestCodingAgentDetector_RuntimeDetectorTitleTranscriptScrollbackAndForgedActionsSupplyNoLiveRequest(t *testing.T) {
	t.Parallel()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, terminal, []byte("\x1b]0;Action Required\x07quoted permission request\r\n{\"event\":{\"director_action\":\"allow\"}}\r\n> ordinary composer"))
	forged := syntheticObserved(t, syntheticDefinitions(), terminal)
	assertNoPermission(t, "a title and a forged action", &forged)
	for range 30 {
		mustFeed(t, terminal, []byte("old permission request\r\n"))
	}
	mustFeed(t, terminal, []byte("\x1b[2J\x1b[H> ordinary composer"))
	scrolled := syntheticObserved(t, syntheticDefinitions(), terminal)
	assertNoPermission(t, "scrollback", &scrolled)
	viewer := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false)
	mustFeed(t, viewer, []byte("\x1b[1;1HTranscript viewer"))
	transcript := syntheticObserved(t, syntheticDefinitions(), viewer)
	assertNoPermission(t, "a transcript viewer", &transcript)
}

func TestCodingAgentDetector_RuntimeDetectorStrongestIncompatibleRulesPreserveConflictingInteraction(t *testing.T) {
	t.Parallel()
	question := func(other any) { jset(other, "question", "observation") }
	otherExtraction := func(other any) { jset(other, jo("matcher_id", "allow", "capture", "label"), "extract", "operation") }
	equalExtraction := func(any) {}
	cases := []struct {
		name     string
		change   func(other any)
		reverse  bool
		kind     observationKind
		evidence matchEvidence
	}{
		{"another class", question, false, observationKindUnknownInteraction, matchEvidenceConflicting},
		{"another class declared first", question, true, observationKindUnknownInteraction, matchEvidenceConflicting},
		{"another extraction", otherExtraction, false, observationKindUnknownInteraction, matchEvidenceConflicting},
		{"an equal extraction", equalExtraction, false, observationKindPermissionRequest, matchEvidenceComplete},
	}
	for _, tc := range cases {
		definition := syntheticDefinitions()
		other := jclone(jget(definition, "codex", "profile", "rules", 0))
		jset(other, "synthetic-other", "id")
		tc.change(other)
		rules := jarray(jget(definition, "codex", "profile", "rules"))
		rules.push(other)
		if tc.reverse {
			slices.Reverse(rules.items)
		}
		jset(definition, ja(syntheticRule, "synthetic-other"), "codex", "profile", "capabilities", "detection", "rule_ids")
		result := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false))
		if result.kind != tc.kind || result.matchEvidence != tc.evidence {
			t.Fatalf("%s: %s %s", tc.name, result.kind.debugString(), result.matchEvidence.debugString())
		}
	}
	conflicting := composerDefinition(none[string](), none[string]())
	twin := jclone(jget(conflicting, "codex", "profile", "rules", 0))
	jset(twin, "synthetic-unknown-twin", "id")
	jset(twin, "unknown", "observation")
	jset(twin, false, "verified_noninteraction")
	jarray(jget(conflicting, "codex", "profile", "rules")).push(twin)
	jset(conflicting, jo("rule_ids", ja("synthetic-unknown-twin"), "fixture_ids", ja(syntheticFixture)), "codex", "profile", "capabilities", "detection")
	composers := syntheticObserved(t, conflicting, syntheticTerminal(t, []string{"Synthetic state", "State end"}, 80, false))
	if composers.kind != observationKindUnknown || composers.matchEvidence != matchEvidenceConflicting || composers.verifiedNoninteraction {
		t.Fatalf("conflicting non-interaction rules: %s %s %v", composers.kind.debugString(), composers.matchEvidence.debugString(), composers.verifiedNoninteraction)
	}
}

func TestCodingAgentDetector_RuntimeDetectorLowerPriorityRuleCannotOverrideTheStrongerPermissionForm(t *testing.T) {
	t.Parallel()
	definition := syntheticDefinitions()
	other := jclone(jget(definition, "codex", "profile", "rules", 0))
	jset(other, "synthetic-lower", "id")
	jset(other, "question", "observation")
	jset(other, 99, "priority")
	rules := jarray(jget(definition, "codex", "profile", "rules"))
	rules.items = slices.Insert(rules.items, 0, other)
	jset(definition, ja(syntheticRule, "synthetic-lower"), "codex", "profile", "capabilities", "detection", "rule_ids")
	observed := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false))
	assertPermissionObservation(t, "the stronger form", &observed)
}

func TestCodingAgentDetector_RuntimeDetectorOtherInteractionClassesRemainDistinct(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		kind     string
		expected observationKind
	}{{"question", observationKindQuestion}, {"unknown_interaction", observationKindUnknownInteraction}} {
		value := stateObservation(t, tc.kind, false)
		if value.kind != tc.expected || value.matchEvidence != matchEvidenceComplete {
			t.Fatalf("%s: %s %s", tc.kind, value.kind.debugString(), value.matchEvidence.debugString())
		}
	}
}

func TestCodingAgentDetector_RuntimeDetectorComposerRequiresItsOwnPositiveEvidence(t *testing.T) {
	t.Parallel()
	if got := stateObservation(t, "composer_empty", true).kind; got != observationKindComposerEmpty {
		t.Fatalf("composer kind %s", got.debugString())
	}
	unknown := syntheticObserved(t, syntheticDefinitions(), syntheticTerminal(t, nil, 80, false))
	if unknown.kind != observationKindUnknown || unknown.matchEvidence != matchEvidenceAbsent || unknown.fingerprint != nil {
		t.Fatalf("empty screen: %s %s", unknown.kind.debugString(), unknown.matchEvidence.debugString())
	}
}

func TestCodingAgentDetector_RuntimeDetectorUnsupportedVersionReportsAbsentEvidenceWithoutNearbyProfile(t *testing.T) {
	t.Parallel()
	screen := syntheticTerminal(t, permissionLines("write disposable.txt"), 80, false).snapshot()
	value, err := newDetector(syntheticCatalog(t), codingAgentClaude, some(syntheticVersion)).observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	if value.kind != observationKindUnknown || value.matchEvidence != matchEvidenceAbsent || value.fingerprint != nil {
		t.Fatalf("claude without a profile: %s %s", value.kind.debugString(), value.matchEvidence.debugString())
	}
}

const operationRowMargin = "    "

// rowedDefinition is the synthetic form with a region-scoped operation that captures every row
// between the `Operation:` prefix and the choice rows.
func rowedDefinition(marginRows, trailer opt[string]) any {
	value := syntheticDefinitions()
	r := jget(value, "codex", "profile", "rules", 0)
	jset(r, parseTestJSON(`{
		"id":"operation","scope":"region",
		"pattern":"(?m)^Operation: (?P<operation>[^\\n]+(?:\\n[^>\\n][^\\n]*)*)\\n> Allow once$"
	}`), "matchers", 0)
	operation := jget(r, "extract", "operation")
	jset(operation, operationRowMargin, "row_margin")
	if rows, set := marginRows.get(); set {
		jset(operation, rows, "margin_rows")
	}
	if text, set := trailer.get(); set {
		jset(operation, text, "row_trailer")
	}
	return value
}

func operationForm(t testing.TB, rows []string) screenSnapshot {
	t.Helper()
	lines := []string{"Synthetic permission request", "Operation: " + rows[0]}
	lines = append(lines, rows[1:]...)
	lines = append(lines, "> Allow once", "  Deny once", "Request form: synthetic-only", "Selection: ")
	return syntheticTerminal(t, lines, 80, false).snapshot()
}

type extractedOperation struct {
	text string
	rows uint64
}

func extractedOperationOf(t testing.TB, definition any, rows []string) opt[extractedOperation] {
	t.Helper()
	screen := operationForm(t, rows)
	observed := syntheticObservation(t, definition, &screen)
	if observed.kind != observationKindPermissionRequest {
		return none[extractedOperation]()
	}
	text, hasText := observed.rawEvidence.operation.get()
	count, hasRows := observed.rawEvidence.operationRows.get()
	if !hasText || !hasRows {
		t.Fatal("a permission request records its operation and its rows")
	}
	return some(extractedOperation{text: text, rows: count})
}

func assertOperation(t testing.TB, label string, got opt[extractedOperation], text string, rows uint64) {
	t.Helper()
	if want := some(extractedOperation{text: text, rows: rows}); got != want {
		t.Fatalf("%s: operation %v, want %v", label, got, want)
	}
}

func TestCodingAgentDetector_OperationRows_ContinuationMarginRowsExemptTheFirstRowAndStripEveryLaterMargin(t *testing.T) {
	t.Parallel()
	definition := rowedDefinition(some("continuation"), none[string]())
	assertOperation(t, "two rows", extractedOperationOf(t, definition, []string{"cat /long/path/", "    to/the/file.txt"}), "cat /long/path/\nto/the/file.txt", 2)
	assertOperation(t, "three rows", extractedOperationOf(t, definition, []string{"printf one", "    two   ", "    three"}), "printf one\ntwo\nthree", 3)
	assertOperation(t, "one row", extractedOperationOf(t, definition, []string{"write disposable.txt"}), "write disposable.txt", 1)
}

func TestCodingAgentDetector_OperationRows_ASelectedRowWithoutItsMarginFailsTheMatch(t *testing.T) {
	t.Parallel()
	continuation := rowedDefinition(some("continuation"), none[string]())
	if got := extractedOperationOf(t, continuation, []string{"cat /long/path/", "  to/the/file.txt"}); got != none[extractedOperation]() {
		t.Fatalf("a short margin matches: %v", got)
	}
	for _, all := range []any{rowedDefinition(some("all"), none[string]()), rowedDefinition(none[string](), none[string]())} {
		if got := extractedOperationOf(t, all, []string{"cat /long/path/", "    to/the/file.txt"}); got != none[extractedOperation]() {
			t.Fatalf("the default margin rows include the first row: %v", got)
		}
	}
}

func TestCodingAgentDetector_OperationRows_TheDefaultMarginRowsKeepTheShippedAllRowsExtraction(t *testing.T) {
	t.Parallel()
	definition := rowedDefinition(none[string](), none[string]())
	jset(definition, `(?m)^Operation:(?P<operation> [^\n]+(?:\n[^>\n][^\n]*)*)\n> Allow once$`, "codex", "profile", "rules", 0, "matchers", 0, "pattern")
	jset(definition, " ", "codex", "profile", "rules", 0, "extract", "operation", "row_margin")
	explicit := jclone(definition)
	jset(explicit, "all", "codex", "profile", "rules", 0, "extract", "operation", "margin_rows")
	for _, value := range []any{definition, explicit} {
		assertOperation(t, "all rows", extractedOperationOf(t, value, []string{"cat /long/path/", " to/the/file.txt"}), "cat /long/path/\nto/the/file.txt", 2)
	}
}

func TestCodingAgentDetector_OperationRows_ATrailerIsRemovedWithItsPrecedingWhitespaceBeforeTheMargin(t *testing.T) {
	t.Parallel()
	definition := rowedDefinition(some("continuation"), some("█"))
	assertOperation(t, "trailers with whitespace", extractedOperationOf(t, definition, []string{"cat /long/path/      █", "    to/the/file.txt     █  "}), "cat /long/path/\nto/the/file.txt", 2)
	assertOperation(t, "a row without the trailer keeps its text", extractedOperationOf(t, definition, []string{"cat /long/path/█", "    to/the/file.txt"}), "cat /long/path/\nto/the/file.txt", 2)
	assertOperation(t, "the trailer is removed only at the row end", extractedOperationOf(t, definition, []string{"cat a█b", "    c █ d"}), "cat a█b\nc █ d", 2)
}

func TestCodingAgentDetector_OperationRows_ATrailerAppliesToASingleRowOperationWithoutAMargin(t *testing.T) {
	t.Parallel()
	definition := syntheticDefinitions()
	jset(definition, "█", "codex", "profile", "rules", 0, "extract", "operation", "row_trailer")
	observed := syntheticObserved(t, definition, syntheticTerminal(t, permissionLines("write disposable.txt   █"), 80, false))
	assertPermissionObservation(t, "a trailed operation", &observed)
	if observed.rawEvidence.operation != some("write disposable.txt") || observed.rawEvidence.operationRows != some(uint64(1)) {
		t.Fatalf("operation %v rows %v", observed.rawEvidence.operation, observed.rawEvidence.operationRows)
	}
}

func TestCodingAgentDetector_OperationRows_ABlankMarginedRowInsideAMultiLineCommandStillMatches(t *testing.T) {
	t.Parallel()
	definition := rowedDefinition(none[string](), none[string]())
	jset(definition, `(?m)^Operation:(?P<operation> [^\n]+(?:\n [^\n]*)*)\n> Allow once$`, "codex", "profile", "rules", 0, "matchers", 0, "pattern")
	jset(definition, " ", "codex", "profile", "rules", 0, "extract", "operation", "row_margin")
	assertOperation(t, "margin removal precedes trimming", extractedOperationOf(t, definition, []string{"printf one", " ", " printf two"}), "printf one\n\nprintf two", 3)
}

func mustMatchAnswer(t testing.TB, d *detector, screen screenSnapshot, plan *answerPlan, label string) answerMatch {
	t.Helper()
	matched, ok, err := d.matchAnswer(&screen, plan)
	if err != nil {
		t.Fatalf("%s: %v", label, err)
	}
	if !ok {
		t.Fatalf("%s: the plan matches", label)
	}
	return matched
}

func assertNoAnswerMatch(t testing.TB, d *detector, screen screenSnapshot, plan *answerPlan, label string) {
	t.Helper()
	if _, ok, err := d.matchAnswer(&screen, plan); err != nil || ok {
		t.Fatalf("%s: matched %v, %v", label, ok, err)
	}
}

func TestCodingAgentDetector_PermissionInspectionAnswerMatchUsesCompleteCurrentRuleChoiceAndFocus(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	d := newDetector(catalog, codingAgentCodex, some("0.0.1-synthetic"))
	plan, ok := catalog.answerPlan(codingAgentCodex, "synthetic-permission", answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		t.Fatal("the allow plan")
	}
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, terminal, inspectionFixtureForm("private operation", "synthetic-only"))
	screen := terminal.snapshot()
	observation, err := d.observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	matched := mustMatchAnswer(t, d, screen, &plan, "the synthetic-only form")
	if observation.kind != observationKindPermissionRequest || !fingerprintsEqual(&matched.semantic, observation.fingerprint) {
		t.Fatal("the match carries the observation's identity")
	}
	if !slices.Equal(matched.choices, []string{"Allow once", "Deny once"}) || matched.outputSeq != screen.screenSeq || matched.plan.choiceID != "allow-once" {
		t.Fatalf("match choices %q seq %d choice %q", matched.choices, matched.outputSeq, matched.plan.choiceID)
	}
	focus := matched.focus
	if focus == "" {
		t.Fatal("the match records its focus")
	}
	mustFeed(t, terminal, inspectionFixtureForm("private operation", "synthetic-other"))
	other := mustMatchAnswer(t, d, terminal.snapshot(), &plan, "the synthetic-other form")
	if !other.semantic.equal(&matched.semantic) || other.focus == focus {
		t.Fatal("another focus keeps the semantics and changes the focus")
	}
	for _, form := range []string{"missing-focus", "synthetic-alt"} {
		mustFeed(t, terminal, inspectionFixtureForm("private operation", form))
		assertNoAnswerMatch(t, d, terminal.snapshot(), &plan, form)
	}
	mustFeed(t, terminal, inspectionFixtureForm("private operation", "synthetic-only"))
	mustFeed(t, terminal, []byte("\x1b[21;1H  Allow once"))
	assertNoAnswerMatch(t, d, terminal.snapshot(), &plan, "a form without its focused choice")
}

func TestCodingAgentDetector_PermissionInspectionAnswerMatchesHaveNoDebugOrSerializedRepresentation(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	d := newDetector(catalog, codingAgentCodex, some(syntheticVersion))
	plan, _ := catalog.answerPlan(codingAgentCodex, "synthetic-permission", answerKeyDecision{decision: permissionDecisionAllow})
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: 80})
	mustFeed(t, terminal, inspectionFixtureForm("private operation", "synthetic-only"))
	matched := mustMatchAnswer(t, d, terminal.snapshot(), &plan, "the inspection form")
	assertRedacted(t, "answerMatch", matched)
	assertRedacted(t, "*answerMatch", &matched)
}

func TestCodingAgentDetector_RuntimeProfilePrivateFingerprintSupportsComparisonWithoutDebugOrSerialization(t *testing.T) {
	t.Parallel()
	first := permissionObservation(t, "write disposable.txt")
	second := permissionObservation(t, "write disposable.txt")
	other := permissionObservation(t, "write another-disposable.txt")
	if !first.fingerprint.equal(second.fingerprint) || first.fingerprint.equal(other.fingerprint) {
		t.Fatal("fingerprints compare by their semantics")
	}
	assertRedacted(t, "semanticFingerprint", *first.fingerprint)
	assertRedacted(t, "*semanticFingerprint", first.fingerprint)
}

func bundledDetector(backend codingAgent, version string) *detector {
	return newDetector(profileCatalogBundled(), backend, some(version))
}

const (
	sgrFaint = "\x1b[2m"
	sgrGrey  = "\x1b[38;5;8m"
	sgrReset = "\x1b[0m"
)

func suggestionDefinition(style any) any {
	definition := simpleDefinition("composer_empty", true)
	r := jget(definition, "codex", "profile", "rules", 0)
	jset(r, parseTestJSON(`{"screen":"either","start_pattern":"^> ","end_pattern":"^> ","max_rows":1,"cursor":"after_prompt"}`), "region")
	jset(r, parseTestJSON(`[{"id":"prompt","scope":"line","pattern":"^> "}]`), "matchers")
	jset(r, jo("style", style), "suggestion")
	return definition
}

// suggestionComposer is the composer row `> ` followed by painted, with the cursor at
// cursorCol.
func suggestionComposer(t testing.TB, painted string, cursorCol int) screenSnapshot {
	t.Helper()
	return screenOf(t, fmt.Appendf(nil, "\x1b[2J\x1b[24;1H> %s%s\x1b[24;%dH", painted, sgrReset, cursorCol+1))
}

func TestCodingAgentDetector_SuggestionComposer_OnlyASuggestionInTheDeclaredStyleWithTheCursorAfterThePromptIsEmpty(t *testing.T) {
	t.Parallel()
	faint := parseTestJSON(`{"faint":true}`)
	grey := parseTestJSON(`{"foreground":{"kind":"indexed","index":8}}`)
	faintSuggestion := sgrFaint + "fix the failing build"
	greySuggestion := sgrGrey + "fix the failing build"
	kind := func(style any, screen screenSnapshot) observationKind {
		return syntheticObservation(t, suggestionDefinition(jclone(style)), &screen).kind
	}
	for _, tc := range []struct {
		name   string
		style  any
		screen screenSnapshot
		empty  bool
	}{
		{"faint suggestion", faint, suggestionComposer(t, faintSuggestion, 2), true},
		{"indexed foreground suggestion", grey, suggestionComposer(t, greySuggestion, 2), true},
		{"bare prompt with the cursor after it", faint, suggestionComposer(t, "", 2), true},
		{"typed draft in the default style", faint, suggestionComposer(t, "fix the failing build", 23), false},
		{"typed text followed by a suggestion", faint, suggestionComposer(t, "fix"+sgrFaint+" the failing build", 5), false},
		{"faint text under a foreground rule", grey, suggestionComposer(t, faintSuggestion, 2), false},
		{"foreground text under a faint rule", faint, suggestionComposer(t, greySuggestion, 2), false},
	} {
		if got := kind(tc.style, tc.screen) == observationKindComposerEmpty; got != tc.empty {
			t.Fatalf("%s: empty %v", tc.name, got)
		}
	}
	for _, cursorCol := range []int{0, 1, 3, 23} {
		if kind(faint, suggestionComposer(t, faintSuggestion, cursorCol)) == observationKindComposerEmpty {
			t.Fatalf("suggestion with the cursor at column %d is empty", cursorCol)
		}
	}
}

var opencodeShellModeVersions = []string{"2.1.4", "2.2.7"}

// opencodeComposer is an OpenCode composer row `┃` painted with trailing blanks and the cursor
// after the marker, then footer two rows below.
func opencodeComposer(t testing.TB, footer string) screenSnapshot {
	t.Helper()
	terminal := mustTerminalState(t, terminalSize{rows: 36, cols: 120})
	mustFeed(t, terminal, []byte("\x1b[2J\x1b[32;1H┃    \x1b[34;1H"+footer+"\x1b[32;3H"))
	return terminal.snapshot()
}

func opencodeObserved(t testing.TB, version, footer string) screenObservation {
	t.Helper()
	screen := opencodeComposer(t, footer)
	observed, err := bundledDetector(codingAgentOpencode, version).observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	return observed
}

func TestCodingAgentDetector_OpencodeShellMode_AComposerAboveTheNormalModeFooterIsAnEmptyComposer(t *testing.T) {
	t.Parallel()
	for _, version := range opencodeShellModeVersions {
		for _, agent := range []string{"cafleet", "cafleet-ask"} {
			observation := opencodeObserved(t, version, agent+" · Big Pickle · OpenCode · ctrl+p menu")
			if observation.kind != observationKindComposerEmpty || !observation.verifiedNoninteraction {
				t.Fatalf("%s %s: %s verified %v", version, agent, observation.kind.debugString(), observation.verifiedNoninteraction)
			}
		}
	}
}

func TestCodingAgentDetector_OpencodeShellMode_AShellModeComposerWithTheNormalGlyphIsNotAnEmptyComposer(t *testing.T) {
	t.Parallel()
	for _, version := range opencodeShellModeVersions {
		observation := opencodeObserved(t, version, "Shell · esc normal")
		if observation.kind == observationKindComposerEmpty || observation.verifiedNoninteraction {
			t.Fatalf("%s: %s verified %v", version, observation.kind.debugString(), observation.verifiedNoninteraction)
		}
	}
}

const (
	panelCols      = 120
	panelSeparator = 80
	panelStyle     = "\x1b[2;48;5;235m"
)

var panelForm = []string{
	"Synthetic permission request",
	"Operation: write disposable.txt",
	"> Allow once",
	"  Deny once",
	"Request form: synthetic-only",
	"Selection: ",
}

type panelRow struct {
	line         int
	left         string
	hasSeparator bool
	separatorCol int
	style        string
	panel        string
}

func newPanelRow(line int, left string, separator opt[int]) panelRow {
	col, has := separator.get()
	return panelRow{line: line, left: left, hasSeparator: has, separatorCol: col, style: panelStyle, panel: " src/lib.rs +after"}
}

// panelScreen writes each row at its 1-based screen row; a separator row draws the glyph at its
// 0-based column in its style, then one panel cell run in the same background.
func panelScreen(t testing.TB, rows []panelRow, cursor [2]int) screenSnapshot {
	t.Helper()
	bytes := []byte("\x1b[0m\x1b[2J")
	for _, row := range rows {
		bytes = fmt.Appendf(bytes, "\x1b[%d;1H%s", row.line, row.left)
		if row.hasSeparator {
			bytes = fmt.Appendf(bytes, "\x1b[%d;%dH%s│\x1b[22m%s\x1b[0m", row.line, row.separatorCol+1, row.style, row.panel)
		}
	}
	bytes = fmt.Appendf(bytes, "\x1b[%d;%dH", cursor[0]+1, cursor[1]+1)
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: panelCols})
	mustFeed(t, terminal, bytes)
	return terminal.snapshot()
}

// panelFormScreen is the synthetic permission form at rows 19-24 with a separator at columns[i]
// on form row i (none past the slice) and, when above is set, on every row above the form.
func panelFormScreen(t testing.TB, columns []opt[int], above opt[int]) screenSnapshot {
	t.Helper()
	var rows []panelRow
	if _, set := above.get(); set {
		for line := 1; line < 19; line++ {
			rows = append(rows, newPanelRow(line, "", above))
		}
	}
	for index, left := range panelForm {
		separator := none[int]()
		if index < len(columns) {
			separator = columns[index]
		}
		rows = append(rows, newPanelRow(19+index, left, separator))
	}
	return panelScreen(t, rows, [2]int{23, 11})
}

func separatorAt(columns ...int) []opt[int] {
	out := make([]opt[int], len(columns))
	for index, col := range columns {
		out[index] = some(col)
	}
	return out
}

func declaringPanelDefinition() any {
	value := syntheticDefinitions()
	jset(value, "│", "codex", "profile", "rules", 0, "region", "panel_separator")
	return value
}

func assertSameRequest(t testing.TB, name string, clipped, full *screenObservation) {
	t.Helper()
	if clipped.kind != observationKindPermissionRequest || clipped.ruleID != syntheticRule || clipped.matchEvidence != matchEvidenceComplete {
		t.Fatalf("%s: %s %s %s", name, clipped.kind.debugString(), clipped.ruleID, clipped.matchEvidence.debugString())
	}
	if clipped.rawEvidence.operation != full.rawEvidence.operation || clipped.rawEvidence.operationRows != full.rawEvidence.operationRows ||
		clipped.rawEvidence.excerpt != full.rawEvidence.excerpt || clipped.rawEvidence.choices.set != full.rawEvidence.choices.set ||
		!slices.Equal(clipped.rawEvidence.choices.values, full.rawEvidence.choices.values) {
		t.Fatalf("%s: evidence differs from the full-width form", name)
	}
	if !fingerprintsEqual(clipped.fingerprint, full.fingerprint) {
		t.Fatalf("%s: the fingerprints differ", name)
	}
}

func TestCodingAgentDetector_PanelClipping_ASharedSeparatorColumnClipsEveryPanelRowForADeclaringRule(t *testing.T) {
	t.Parallel()
	fullScreen := panelFormScreen(t, nil, none[int]())
	full := syntheticObservation(t, declaringPanelDefinition(), &fullScreen)
	if full.ruleID != syntheticRule {
		t.Fatalf("full-width rule %s", full.ruleID)
	}
	every := separatorAt(panelSeparator, panelSeparator, panelSeparator, panelSeparator, panelSeparator, panelSeparator)
	for _, tc := range []struct {
		name   string
		screen screenSnapshot
	}{
		{"every form row", panelFormScreen(t, every, none[int]())},
		{"every row above and on the form", panelFormScreen(t, every, some(panelSeparator))},
		{"a full-width last row", panelFormScreen(t, every[:5], some(panelSeparator))},
		{"one panel row", panelFormScreen(t, []opt[int]{none[int](), some(panelSeparator)}, none[int]())},
	} {
		clipped := syntheticObservation(t, declaringPanelDefinition(), &tc.screen)
		assertSameRequest(t, tc.name, &clipped, &full)
	}
}

func TestCodingAgentDetector_PanelClipping_AClippedRequestAnswersThroughTheSamePlan(t *testing.T) {
	t.Parallel()
	value := inspectionFixtureDefinition()
	jset(value, "│", "codex", "profile", "rules", 0, "region", "panel_separator")
	catalog := mustLoadCatalog(t, value)
	d := newDetector(catalog, codingAgentCodex, some(syntheticVersion))
	plan, ok := catalog.answerPlan(codingAgentCodex, syntheticRule, answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		t.Fatal("the allow plan")
	}
	every := separatorAt(panelSeparator, panelSeparator, panelSeparator, panelSeparator, panelSeparator, panelSeparator)
	clipped := mustMatchAnswer(t, d, panelFormScreen(t, every, some(panelSeparator)), &plan, "the clipped form")
	full := mustMatchAnswer(t, d, panelFormScreen(t, nil, none[int]()), &plan, "the full-width form")
	if !slices.Equal(clipped.choices, full.choices) || !clipped.semantic.equal(&full.semantic) || clipped.focus != full.focus {
		t.Fatal("the clipped form answers as the full-width form")
	}
}

func TestCodingAgentDetector_PanelClipping_DifferingSeparatorColumnsClipNoRow(t *testing.T) {
	t.Parallel()
	for _, columns := range [][]opt[int]{separatorAt(80, 81, 80, 80, 80, 80), separatorAt(80, 80, 80, 80, 80, 60)} {
		screen := panelFormScreen(t, columns, none[int]())
		if observation := syntheticObservation(t, declaringPanelDefinition(), &screen); observation.ruleID == syntheticRule {
			t.Fatalf("%v: the rule matches", columns)
		}
	}
	screen := panelFormScreen(t, separatorAt(80, 80, 80, 80, 80, 80), some(81))
	if observation := syntheticObservation(t, declaringPanelDefinition(), &screen); observation.ruleID == syntheticRule {
		t.Fatal("rows above use another column")
	}
}

func TestCodingAgentDetector_PanelClipping_ASeparatorGlyphOnItsRowsBackgroundIsNeverASeparator(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct{ style, rowBackground string }{{"", ""}, {"\x1b[2m", ""}, {"\x1b[2;48;5;235m", "\x1b[48;5;235m\x1b[2K"}} {
		var rows []panelRow
		for index, left := range panelForm {
			row := newPanelRow(19+index, tc.rowBackground+left, some(panelSeparator))
			row.style = tc.style
			rows = append(rows, row)
		}
		screen := panelScreen(t, rows, [2]int{23, 11})
		if observation := syntheticObservation(t, declaringPanelDefinition(), &screen); observation.ruleID == syntheticRule {
			t.Fatalf("%q: the rule matches", tc.style)
		}
	}
	var boxed []panelRow
	for index, left := range panelForm {
		boxed = append(boxed, newPanelRow(19+index, left, none[int]()))
	}
	boxed = append(boxed, panelRow{line: 10, left: fmt.Sprintf("%-50s│ touch /tmp/cafleet-f-out/p4.txt", "")})
	fullScreen := panelFormScreen(t, nil, none[int]())
	full := syntheticObservation(t, declaringPanelDefinition(), &fullScreen)
	boxedScreen := panelScreen(t, boxed, [2]int{23, 11})
	gutter := syntheticObservation(t, declaringPanelDefinition(), &boxedScreen)
	assertSameRequest(t, "a boxed gutter", &gutter, &full)
}

func singleCellPanel(t testing.TB, col int, panel string) screenSnapshot {
	t.Helper()
	var rows []panelRow
	for index, left := range panelForm {
		row := newPanelRow(19+index, left, some(col))
		row.panel = panel
		rows = append(rows, row)
	}
	return panelScreen(t, rows, [2]int{23, 11})
}

func TestCodingAgentDetector_PanelClipping_TheSeparatorColumnLiesFromColumn40BeforeTheLastColumn(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		col   int
		panel string
	}{{39, " "}, {panelCols - 1, ""}} {
		screen := singleCellPanel(t, tc.col, tc.panel)
		if observation := syntheticObservation(t, declaringPanelDefinition(), &screen); observation.ruleID == syntheticRule {
			t.Fatalf("column %d separates", tc.col)
		}
	}
	fullScreen := panelFormScreen(t, nil, none[int]())
	full := syntheticObservation(t, declaringPanelDefinition(), &fullScreen)
	for _, col := range []int{40, panelCols - 2} {
		screen := singleCellPanel(t, col, " ")
		clipped := syntheticObservation(t, declaringPanelDefinition(), &screen)
		assertSameRequest(t, fmt.Sprintf("column %d", col), &clipped, &full)
	}
}

func TestCodingAgentDetector_PanelClipping_AClippedRowIsNeverSoftJoinedToTheNextRow(t *testing.T) {
	t.Parallel()
	var rows []panelRow
	for index, left := range panelForm {
		row := newPanelRow(19+index, left, some(panelSeparator))
		if index == 1 {
			row.panel = " " + strings.Repeat("x", 45)
		}
		rows = append(rows, row)
	}
	screen := panelScreen(t, rows, [2]int{23, 11})
	if !screen.screen.RowWrapped(19) {
		t.Fatal("the overflowing panel text wraps the operation row")
	}
	if unclipped := syntheticObservation(t, syntheticDefinitions(), &screen); unclipped.ruleID == syntheticRule {
		t.Fatal("unclipped, the wrapped operation row joins the next row")
	}
	fullScreen := panelFormScreen(t, nil, none[int]())
	full := syntheticObservation(t, declaringPanelDefinition(), &fullScreen)
	clipped := syntheticObservation(t, declaringPanelDefinition(), &screen)
	assertSameRequest(t, "a wrapped panel row", &clipped, &full)
}

func panelStateDefinition(separator bool) any {
	value := declaringPanelDefinition()
	r := parseTestJSON(`{
		"id":"synthetic-panel-state","priority":100,"observation":"question",
		"region":{"screen":"either","start_pattern":"^Status idle +│ ready$","end_pattern":"^Status idle +│ ready$","max_rows":1,"cursor":"any"},
		"matchers":[{"id":"status","scope":"line","pattern":"^Status idle +│ ready$"}],
		"extract":{"operation":null,"choices":[]},
		"verified_noninteraction":false
	}`)
	if separator {
		jset(r, "│", "region", "panel_separator")
	}
	jarray(jget(value, "codex", "profile", "rules")).push(r)
	jset(value, ja(syntheticRule, "synthetic-panel-state"), "codex", "profile", "capabilities", "detection", "rule_ids")
	return value
}

func TestCodingAgentDetector_PanelClipping_RulesWithoutAPanelSeparatorSeeUnclippedRows(t *testing.T) {
	t.Parallel()
	status := newPanelRow(5, "Status idle", some(panelSeparator))
	status.panel = " ready"
	screen := panelScreen(t, []panelRow{status}, [2]int{0, 0})
	unclipped := syntheticObservation(t, panelStateDefinition(false), &screen)
	if unclipped.ruleID != "synthetic-panel-state" || unclipped.kind != observationKindQuestion {
		t.Fatalf("unclipped: %s %s", unclipped.ruleID, unclipped.kind.debugString())
	}
	if clipped := syntheticObservation(t, panelStateDefinition(true), &screen); clipped.ruleID == "synthetic-panel-state" {
		t.Fatal("a declaring rule sees the clipped row")
	}
}

func questionDetector(t testing.TB) (*profileCatalog, *detector) {
	t.Helper()
	catalog := mustLoadCatalog(t, questionFixtureDefinition())
	return catalog, newDetector(catalog, codingAgentCodex, some(syntheticVersion))
}

func questionPlan(t testing.TB, catalog *profileCatalog, request string, key answerKey) answerPlan {
	t.Helper()
	plan, ok := catalog.answerPlan(codingAgentCodex, request, key)
	if !ok {
		t.Fatal("synthetic question profile plan")
	}
	return plan
}

func questionMatched(t testing.TB, focused []string, id string) (answerMatch, bool) {
	t.Helper()
	catalog, d := questionDetector(t)
	plan := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: id})
	screen := screenOf(t, questionFixtureForm(focused))
	matched, ok, err := d.matchAnswer(&screen, &plan)
	if err != nil {
		t.Fatal(err)
	}
	return matched, ok
}

func TestCodingAgentDetector_QuestionAnswers_AQuestionFormIsACompleteQuestionWithItsChoiceLabels(t *testing.T) {
	t.Parallel()
	_, d := questionDetector(t)
	screen := screenOf(t, questionFixtureForm([]string{"accept"}))
	observation, err := d.observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	if observation.kind != observationKindQuestion || observation.ruleID != questionRule || observation.matchEvidence != matchEvidenceComplete || observation.fingerprint == nil {
		t.Fatalf("question: %s %s %s", observation.kind.debugString(), observation.ruleID, observation.matchEvidence.debugString())
	}
	if !slices.Equal(observation.rawEvidence.choices.values, []string{"Accept the change", "Keep current", "Ask later"}) {
		t.Fatalf("choices %q", observation.rawEvidence.choices.values)
	}
}

func TestCodingAgentDetector_QuestionAnswers_MatchAnswerMatchesAQuestionForEachOfItsChoicePlans(t *testing.T) {
	t.Parallel()
	catalog, _ := questionDetector(t)
	for _, id := range []string{"decline", "later"} {
		matched, ok := questionMatched(t, []string{"accept"}, id)
		if !ok {
			t.Fatalf("%s: a choice plan matches its question", id)
		}
		want := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: id})
		if !matched.plan.equal(&want) || !slices.Equal(matched.choices, []string{"Accept the change", "Keep current", "Ask later"}) {
			t.Fatalf("%s: match plan or choices differ", id)
		}
	}
}

func TestCodingAgentDetector_QuestionAnswers_MatchAnswerRefusesAChoicePlanOnAPermissionFormAndADecisionPlanOnAQuestion(t *testing.T) {
	t.Parallel()
	catalog, d := questionDetector(t)
	permission := screenOf(t, inspectionFixtureForm("write disposable.txt", "synthetic-only"))
	observed, err := d.observe(&permission)
	if err != nil || observed.kind != observationKindPermissionRequest {
		t.Fatalf("permission form: %s %v", observed.kind.debugString(), err)
	}
	allow := questionPlan(t, catalog, syntheticRule, answerKeyDecision{decision: permissionDecisionAllow})
	mustMatchAnswer(t, d, permission, &allow, "the decision plan answers its permission form")
	for _, id := range []string{"decline", "later"} {
		plan := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: id})
		assertNoAnswerMatch(t, d, permission, &plan, id+" on a permission form")
	}
	assertNoAnswerMatch(t, d, screenOf(t, questionFixtureForm([]string{"accept"})), &allow, "a decision plan on a question")
	decline := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: "decline"})
	assertNoAnswerMatch(t, d, screenOf(t, questionNoticeForm()), &decline, "a choice plan on another question rule")
}

func TestCodingAgentDetector_QuestionAnswers_MatchAnswerBindsAChoicePlanToItsBackendWhateverTheRunVersion(t *testing.T) {
	t.Parallel()
	catalog, _ := questionDetector(t)
	screen := screenOf(t, questionFixtureForm([]string{"accept"}))
	decline := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: "decline"})
	for _, version := range []opt[string]{some("0.0.2-synthetic"), some("99.0.0"), none[string]()} {
		mustMatchAnswer(t, newDetector(catalog, codingAgentCodex, version), screen, &decline, fmt.Sprint(version))
	}
	assertNoAnswerMatch(t, newDetector(catalog, codingAgentClaude, some(syntheticVersion)), screen, &decline, "claude")
}

func TestCodingAgentDetector_QuestionAnswers_MarkerFocusResolvesTheOneChoiceOnTheMarkerRow(t *testing.T) {
	t.Parallel()
	for _, focused := range []string{"accept", "decline", "later"} {
		for _, id := range []string{"decline", "later"} {
			matched, ok := questionMatched(t, []string{focused}, id)
			if !ok || matched.focusedChoice != some(focused) {
				t.Fatalf("%s %s: focused %v", focused, id, matched.focusedChoice)
			}
		}
	}
}

func TestCodingAgentDetector_QuestionAnswers_MarkerFocusResolvesNoChoiceWhenTheMarkerRowHoldsNoChoiceLabel(t *testing.T) {
	t.Parallel()
	catalog, d := questionDetector(t)
	unlisted := screenOf(t, []byte("\x1b[0m\x1b[2J\x1b[18;1HSynthetic question?\r\n  Accept the change\r\n  Keep current\r\n> Other option\r\n  Ask later\r\nQuestion end"))
	decline := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: "decline"})
	if matched := mustMatchAnswer(t, d, unlisted, &decline, "an unlisted focused option"); matched.focusedChoice != none[string]() {
		t.Fatalf("focused %v", matched.focusedChoice)
	}
}

func TestCodingAgentDetector_QuestionAnswers_MarkerFocusResolvesNoChoiceWhenTheMarkerRowHoldsSeveralChoiceLabels(t *testing.T) {
	t.Parallel()
	value := questionFixtureDefinition()
	jset(value, " / (?P<label>Ask later)$", "codex", "profile", "rules", 1, "matchers", 4, "pattern")
	catalog := mustLoadCatalog(t, value)
	plan := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: "decline"})
	d := newDetector(catalog, codingAgentCodex, some(syntheticVersion))
	shared := screenOf(t, []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic question?\r\n  Accept the change\r\n> Keep current / Ask later\r\nQuestion end"))
	if matched := mustMatchAnswer(t, d, shared, &plan, "two labels on the marker row"); matched.focusedChoice != none[string]() {
		t.Fatalf("focused %v", matched.focusedChoice)
	}
	separate := screenOf(t, []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic question?\r\n> Accept the change\r\n  Keep current / Ask later\r\nQuestion end"))
	if matched := mustMatchAnswer(t, d, separate, &plan, "one label on the marker row"); matched.focusedChoice != some("accept") {
		t.Fatalf("focused %v", matched.focusedChoice)
	}
}

func TestCodingAgentDetector_QuestionAnswers_MarkerFocusIdentityIsTheCaptureAndFocusedChoiceLikeBackgroundFocus(t *testing.T) {
	t.Parallel()
	accept, _ := questionMatched(t, []string{"accept"}, "decline")
	decline, _ := questionMatched(t, []string{"decline"}, "decline")
	for _, value := range []answerMatch{accept, decline} {
		encoded, err := encodeJSON([2]any{value.focusCapture, value.focusedChoice})
		if err != nil {
			t.Fatal(err)
		}
		if value.focus != string(encoded) {
			t.Fatalf("focus %q, want %s", value.focus, encoded)
		}
	}
	again, _ := questionMatched(t, []string{"accept"}, "decline")
	if accept.focus == decline.focus || !accept.semantic.equal(&decline.semantic) || accept.sameRequest(&decline) || !accept.sameRequest(&again) {
		t.Fatal("a focus move keeps the semantics and changes the request focus")
	}
	_, d := questionDetector(t)
	acceptScreen := screenOf(t, questionFixtureForm([]string{"accept"}))
	declineScreen := screenOf(t, questionFixtureForm([]string{"decline"}))
	a, errA := d.observe(&acceptScreen)
	b, errB := d.observe(&declineScreen)
	if errA != nil || errB != nil || !fingerprintsEqual(a.fingerprint, b.fingerprint) {
		t.Fatal("a focus move keeps the episode identity")
	}
}

func TestCodingAgentDetector_QuestionAnswers_AnInteractionFrameRejectPatternSuppressesTheRuleAnywhereOnTheScreen(t *testing.T) {
	t.Parallel()
	value := questionFixtureDefinition()
	jset(value, parseTestJSON(`{"require_patterns":[],"reject_patterns":["^Live composer$"]}`), "codex", "profile", "rules", 1, "frame")
	catalog := mustLoadCatalog(t, value)
	d := newDetector(catalog, codingAgentCodex, some(syntheticVersion))
	aloneScreen := screenOf(t, questionFixtureForm([]string{"accept"}))
	alone, err := d.observe(&aloneScreen)
	if err != nil || alone.kind != observationKindQuestion || alone.ruleID != questionRule {
		t.Fatalf("alone: %s %s %v", alone.kind.debugString(), alone.ruleID, err)
	}
	quotedScreen := screenOf(t, append(questionFixtureForm([]string{"accept"}), "\x1b[2;1HLive composer"...))
	quoted, err := d.observe(&quotedScreen)
	if err != nil || quoted.ruleID == questionRule || quoted.kind == observationKindQuestion {
		t.Fatalf("quoted: %s %s %v", quoted.kind.debugString(), quoted.ruleID, err)
	}
	plan := questionPlan(t, catalog, questionRule, answerKeyChoice{choice: "decline"})
	assertNoAnswerMatch(t, d, screenOf(t, append(questionFixtureForm([]string{"accept"}), "\x1b[24;1HLive composer"...)), &plan, "a live composer row on the screen")
}

func TestCodingAgentDetector_QuestionAnswers_AnInteractionFrameRequirePatternMustAppearOnTheScreen(t *testing.T) {
	t.Parallel()
	value := questionFixtureDefinition()
	jset(value, parseTestJSON(`{"require_patterns":["^Footer: question$"],"reject_patterns":["^Live composer$"]}`), "codex", "profile", "rules", 1, "frame")
	d := newDetector(mustLoadCatalog(t, value), codingAgentCodex, some(syntheticVersion))
	withoutScreen := screenOf(t, questionFixtureForm([]string{"accept"}))
	if without, err := d.observe(&withoutScreen); err != nil || without.ruleID == questionRule {
		t.Fatalf("without the footer: %s %v", without.ruleID, err)
	}
	withScreen := screenOf(t, append(questionFixtureForm([]string{"accept"}), "\x1b[2;1HFooter: question"...))
	if with, err := d.observe(&withScreen); err != nil || with.ruleID != questionRule {
		t.Fatalf("with the footer: %s %v", with.ruleID, err)
	}
}

func gatedDetector(t testing.TB) (*profileCatalog, *detector) {
	t.Helper()
	catalog := mustLoadCatalog(t, gatedFixtureDefinition())
	return catalog, newDetector(catalog, codingAgentCodex, some(syntheticVersion))
}

func gatedMatched(t testing.TB, screen screenSnapshot, decision permissionDecision) answerMatch {
	t.Helper()
	_, d := gatedDetector(t)
	plan := gatedPlan(t, decision)
	return mustMatchAnswer(t, d, screen, &plan, "the gated form matches regardless of its focus")
}

func gatedFocused(t testing.TB, screen screenSnapshot) opt[string] {
	t.Helper()
	allow := gatedMatched(t, screen, permissionDecisionAllow)
	deny := gatedMatched(t, screen, permissionDecisionDeny)
	if allow.focusedChoice != deny.focusedChoice {
		t.Fatal("both plans read the same focus matcher")
	}
	return allow.focusedChoice
}

func TestCodingAgentDetector_FocusResolution_FocusStyleBackgroundResolvesTheOneFocusedChoice(t *testing.T) {
	t.Parallel()
	_, d := gatedDetector(t)
	for _, tc := range []struct{ focus, expected string }{{"allow", "allow-once"}, {"deny", "deny-once"}} {
		screen := screenOf(t, gatedFixtureForm("write disposable.txt", tc.focus))
		if got := gatedFocused(t, screen); got != some(tc.expected) {
			t.Fatalf("%s: focused %v", tc.focus, got)
		}
		observation, err := d.observe(&screen)
		if err != nil || observation.kind != observationKindPermissionRequest || observation.ruleID != gatedRule || observation.matchEvidence != matchEvidenceComplete {
			t.Fatalf("%s: %s %s %v", tc.focus, observation.kind.debugString(), observation.ruleID, err)
		}
		if !slices.Equal(observation.rawEvidence.choices.values, []string{"Allow once", "Deny once"}) {
			t.Fatalf("%s: choices %q", tc.focus, observation.rawEvidence.choices.values)
		}
	}
}

func TestCodingAgentDetector_FocusResolution_FocusStyleBackgroundResolvesNoChoiceForNoneSeveralOrAnUnlistedOption(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name  string
		bytes []byte
	}{
		{"no styled label", gatedFixtureForm("write disposable.txt", "none")},
		{"an option without a choice entry", gatedFixtureForm("write disposable.txt", "always")},
		{"several styled choices", gatedFixtureFormStyled("write disposable.txt", true, []string{"Allow once", "Deny once"}, "", focusSGR)},
		{"a label only partly styled", []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: write disposable.txt\r\n  " + focusSGR + "Allow\x1b[0m once   Always allow   Deny once\r\nSelection: ")},
	} {
		if got := gatedFocused(t, screenOf(t, tc.bytes)); got != none[string]() {
			t.Fatalf("%s: focused %v", tc.name, got)
		}
	}
}

func TestCodingAgentDetector_FocusResolution_FocusStyleBackgroundComparesBackgroundsNotThemeColours(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name       string
		always     bool
		row, focus string
	}{
		{"indexed theme", true, "\x1b[48;5;236m", "\x1b[48;5;214m"},
		{"rgb theme", true, "\x1b[48;2;30;30;30m", "\x1b[48;2;10;200;10m"},
		{"two-option layout", false, "\x1b[48;2;30;30;30m", focusSGR},
		{"default row background", false, "", "\x1b[48;5;4m"},
	} {
		for _, label := range []struct{ label, expected string }{{"Allow once", "allow-once"}, {"Deny once", "deny-once"}} {
			screen := screenOf(t, gatedFixtureFormStyled("write disposable.txt", tc.always, []string{label.label}, tc.row, tc.focus))
			if got := gatedFocused(t, screen); got != some(label.expected) {
				t.Fatalf("%s %s: focused %v", tc.name, label.label, got)
			}
		}
		unfocused := screenOf(t, gatedFixtureFormStyled("write disposable.txt", tc.always, nil, tc.row, tc.focus))
		if got := gatedFocused(t, unfocused); got != none[string]() {
			t.Fatalf("%s on the row background: focused %v", tc.name, got)
		}
	}
}

func TestCodingAgentDetector_FocusResolution_StylingOnlyFocusChangeChangesTheRequestFocusButNotItsSemantics(t *testing.T) {
	t.Parallel()
	allow := screenOf(t, gatedFixtureForm("write disposable.txt", "allow"))
	deny := screenOf(t, gatedFixtureForm("write disposable.txt", "deny"))
	always := screenOf(t, gatedFixtureForm("write disposable.txt", "always"))
	for _, decision := range []permissionDecision{permissionDecisionAllow, permissionDecisionDeny} {
		first := gatedMatched(t, allow, decision)
		moved := gatedMatched(t, deny, decision)
		unlisted := gatedMatched(t, always, decision)
		again := gatedMatched(t, allow, decision)
		if !first.semantic.equal(&moved.semantic) || !slices.Equal(first.choices, moved.choices) ||
			first.focus == moved.focus || first.focus == unlisted.focus || first.sameRequest(&moved) || !first.sameRequest(&again) {
			t.Fatalf("%v: a styling-only focus change", decision)
		}
	}
	_, d := gatedDetector(t)
	a, errA := d.observe(&allow)
	b, errB := d.observe(&deny)
	if errA != nil || errB != nil || !fingerprintsEqual(a.fingerprint, b.fingerprint) {
		t.Fatal("a focus move keeps the episode identity")
	}
}

func TestCodingAgentDetector_FocusResolution_ContinuationRuleObservesOnlyTheEmptyReasonDialog(t *testing.T) {
	t.Parallel()
	_, d := gatedDetector(t)
	dialog := screenOf(t, gatedReasonDialog(none[string]()))
	empty, err := d.observe(&dialog)
	if err != nil || empty.kind != observationKindPermissionContinuation || empty.ruleID != gatedReasonRule ||
		empty.matchEvidence != matchEvidenceComplete || empty.verifiedNoninteraction {
		t.Fatalf("empty reason: %s %s %v", empty.kind.debugString(), empty.ruleID, err)
	}
	for _, decision := range []permissionDecision{permissionDecisionAllow, permissionDecisionDeny} {
		plan := gatedPlan(t, decision)
		assertNoAnswerMatch(t, d, dialog, &plan, "a continuation is not an answerable request form")
	}
	for _, reason := range []string{"typed reason", "(empty) and more"} {
		screen := screenOf(t, gatedReasonDialog(some(reason)))
		typed, err := d.observe(&screen)
		if err != nil || typed.kind == observationKindPermissionContinuation || typed.ruleID == gatedReasonRule {
			t.Fatalf("%s: %s %s %v", reason, typed.kind.debugString(), typed.ruleID, err)
		}
	}
}

const focusRowSGR = "\x1b[48;2;30;30;30m"

// sizedGatedForm is the gated form on a cols-wide screen whose choice row is row, written from
// column 0 after the screen is cleared to the default background.
func sizedGatedForm(t testing.TB, cols uint16, row string) screenSnapshot {
	t.Helper()
	terminal := mustTerminalState(t, terminalSize{rows: 24, cols: cols})
	mustFeed(t, terminal, []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: write disposable.txt\r\n"+row+"\x1b[0m\r\nSelection: "))
	return terminal.snapshot()
}

func gatedFocusedOn(t testing.TB, screen screenSnapshot) opt[string] {
	t.Helper()
	return gatedMatched(t, screen, permissionDecisionAllow).focusedChoice
}

func TestCodingAgentDetector_FocusResolution_FocusRowBackgroundIsTheMajorityBackgroundNotTheLeftEdge(t *testing.T) {
	t.Parallel()
	edge := sizedGatedForm(t, 80, focusRowSGR+"\x1b[K"+focusSGR+"  Allow once "+focusRowSGR+"  Always allow   Deny once")
	if got := gatedFocusedOn(t, edge); got != some("allow-once") {
		t.Fatalf("a highlight over column 0 leaves the row background to the majority: %v", got)
	}
	lastColumn := sizedGatedForm(t, 39, focusRowSGR+"\x1b[K  Allow once   Always allow   "+focusSGR+"Deny once")
	if got := gatedFocusedOn(t, lastColumn); got != some("deny-once") {
		t.Fatalf("a focused label may reach the last column: %v", got)
	}
}

func TestCodingAgentDetector_FocusResolution_FocusResolvesNoChoiceWithoutABackgroundCoveringMoreThanHalfTheRow(t *testing.T) {
	t.Parallel()
	const half = "\x1b[48;5;236m"
	splitRow := func(painted int) screenSnapshot {
		return sizedGatedForm(t, 80, fmt.Sprintf(half+"\x1b[K  "+focusSGR+"Allow once"+half+"   Always allow   Deny once\x1b[%dG\x1b[0m\x1b[K", painted+1))
	}
	if got := gatedFocusedOn(t, splitRow(40)); got != none[string]() {
		t.Fatalf("40 painted cells of 80 are not a majority: %v", got)
	}
	if got := gatedFocusedOn(t, splitRow(51)); got != some("allow-once") {
		t.Fatalf("41 cells of the painted background are a majority: %v", got)
	}
}

const syntheticEmptyRule = "synthetic-composer_empty"

// postSubmitEditors is the bundled fixtures of backend whose composer evidence records a
// post-submit screen and that the bundled detector verifies as an empty composer.
func postSubmitEditors(t testing.TB, backend codingAgent) []bundledFixtureRecord {
	t.Helper()
	var editors []bundledFixtureRecord
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend != backend || !hasPostSubmitEvidence(t, fixtureDocument(fixture.id)) {
			continue
		}
		snapshot := replayAsCaptured(fixture.id, nil, none[[2]uint16]())
		if observed := observeBundled(t, backend, fixture.version, &snapshot); observed.kind == observationKindComposerEmpty && observed.verifiedNoninteraction {
			editors = append(editors, fixture)
		}
	}
	if len(editors) == 0 {
		t.Fatalf("%s: a verified empty post-submit editor", backend.name())
	}
	return editors
}

func hasPostSubmitEvidence(t testing.TB, screen any) bool {
	t.Helper()
	records, present := jobject(screen).get("composer_evidence")
	if !present {
		return false
	}
	for _, raw := range jarray(records).items {
		record, err := serdeComposerEvidence(raw)
		if err != nil {
			t.Fatal(err)
		}
		if record.identity.kind == composerEvidenceKindPostSubmit {
			return true
		}
	}
	return false
}

// draftedEditor replays the fixture as captured with draft typed at its cursor: the cursor row
// keeps its display columns before the cursor, then holds draft in the default style, because a
// captured placeholder style on those cells would repaint the draft as the placeholder, and the
// cursor follows the draft.
func draftedEditor(id, draft string) screenSnapshot {
	screen := fixtureDocument(id)
	at, ok := fixtureCursor(screen).get()
	if !ok {
		panic(id + ": a post-submit editor records its cursor")
	}
	row, col := at[0], at[1]
	lines := jarray(jget(screen, "lines"))
	text, ok := jstring(lines.items[row])
	if !ok {
		panic("a fixture line is a string")
	}
	lines.items[row] = textBeforeColumn(id, text, col) + draft
	for _, key := range []string{"cell_backgrounds", "cell_styles"} {
		runs, present := jget(screen, key).(*jsonArr)
		if !present {
			continue
		}
		var kept []any
		for _, run := range runs.items {
			runRow, rowOK := ju64(jget(run, "row"))
			start, startOK := ju64(jget(run, "start_col"))
			end, endOK := ju64(jget(run, "end_col"))
			if !rowOK || !startOK || !endOK {
				panic("a fixture run geometry")
			}
			switch {
			case runRow != uint64(row) || end <= uint64(col):
				kept = append(kept, run)
			case start < uint64(col):
				jset(run, uint64(col), "end_col")
				kept = append(kept, run)
			}
		}
		runs.items = kept
	}
	end := uint(col) + unicodewidth.StrWidth([]byte(draft))
	if end > math.MaxUint16 {
		panic(id + ": a draft inside the screen geometry")
	}
	return replayNativeFixture(testJSONText(screen), nil, fixtureAlternate(screen), some([2]uint16{row, uint16(end)}))
}

// textBeforeColumn is the prefix of row covering its first col display columns, padded with
// spaces when row is narrower.
func textBeforeColumn(id, row string, col uint16) string {
	columns := 0
	for offset, r := range row {
		if columns == int(col) {
			return row[:offset]
		}
		width, printable := unicodewidth.CharWidth(r)
		if !printable {
			panic(id + ": a printable cursor row")
		}
		columns += width
	}
	return row + strings.Repeat(" ", int(col)-columns)
}

func TestCodingAgentDetector_ComposerReliability_PiWorkingEditorsWithTypedOrPastedInputRemainUnverified(t *testing.T) {
	t.Parallel()
	for _, editor := range postSubmitEditors(t, codingAgentPi) {
		for _, draft := range []string{"x", "! [paste #1 2047 chars]", "retained input"} {
			snapshot := draftedEditor(editor.id, draft)
			observed := observeBundled(t, codingAgentPi, editor.version, &snapshot)
			if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
				t.Fatalf("%s %q: %s verified=%v", editor.id, draft, observed.kind.debugString(), observed.verifiedNoninteraction)
			}
		}
	}
}

func TestCodingAgentDetector_ComposerReliability_PiWorkingEditorRequiresTheCursorInsideItsLiveRegion(t *testing.T) {
	t.Parallel()
	for _, editor := range postSubmitEditors(t, codingAgentPi) {
		snapshot := replayAsCaptured(editor.id, nil, some([2]uint16{0, 0}))
		observed := observeBundled(t, codingAgentPi, editor.version, &snapshot)
		if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
			t.Fatalf("%s: %s verified=%v", editor.id, observed.kind.debugString(), observed.verifiedNoninteraction)
		}
	}
}

func TestCodingAgentDetector_ComposerReliability_HermesWorkingEditorRejectsLiteralSuffixesAndNonprefixCursorPositions(t *testing.T) {
	t.Parallel()
	for _, editor := range postSubmitEditors(t, codingAgentHermes) {
		for _, draft := range []string{"x", "[Pasted 2057 chars]", "retained literal"} {
			snapshot := draftedEditor(editor.id, draft)
			observed := observeBundled(t, codingAgentHermes, editor.version, &snapshot)
			if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
				t.Fatalf("%s %q: %s verified=%v", editor.id, draft, observed.kind.debugString(), observed.verifiedNoninteraction)
			}
		}
		snapshot := replayAsCaptured(editor.id, nil, some([2]uint16{0, 0}))
		if observed := observeBundled(t, codingAgentHermes, editor.version, &snapshot); observed.kind == observationKindComposerEmpty {
			t.Fatalf("%s: a cursor off the prompt prefix is an empty composer", editor.id)
		}
	}
}

// citedPlan is a bundled answer plan with the pre-stage fixtures its answer cites.
type citedPlan struct {
	plan     answerPlan
	fixtures []bundledFixtureRecord
}

// bundledCitedPlans is every answer of backend's bundled profile file, in file order, with its
// plan from the bundled catalog.
func bundledCitedPlans(t testing.TB, backend codingAgent) []citedPlan {
	t.Helper()
	versions := map[string]string{}
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend == backend {
			versions[fixture.id] = fixture.version
		}
	}
	bundled, ok := profileCatalogBundled().profile(backend)
	if !ok {
		t.Fatalf("%s: a bundled profile", backend.name())
	}
	var plans []citedPlan
	for _, raw := range jarray(jget(parsedBundledFiles(), backend.name(), "profile", "capabilities", "answers")).items {
		var key answerKey
		if decision, isDecision := jstring(jget(raw, "decision")); isDecision {
			parsed, err := permissionDecisionTryFrom(decision)
			if err != nil {
				t.Fatal(err)
			}
			key = answerKeyDecision{decision: parsed}
		} else {
			choice, isChoice := jstring(jget(raw, "choice_id"))
			if !isChoice {
				t.Fatalf("%s: an answer keyed by a decision or a choice", backend.name())
			}
			key = answerKeyChoice{choice: choice}
		}
		request, _ := jstring(jget(raw, "request_rule_id"))
		plan, ok := bundled.answerPlan(request, key)
		if !ok {
			t.Fatalf("%s: the bundled catalog plans every answer of its file", backend.name())
		}
		cited := citedPlan{plan: plan}
		for _, id := range jarray(jget(raw, "fixture_ids")).items {
			name, _ := jstring(id)
			version, ok := versions[name]
			if !ok {
				t.Fatalf("%s: a cited fixture of the backend", name)
			}
			cited.fixtures = append(cited.fixtures, bundledFixtureRecord{backend: backend, id: name, version: version})
		}
		plans = append(plans, cited)
	}
	if len(plans) == 0 {
		t.Fatalf("%s: a bundled answer plan", backend.name())
	}
	return plans
}

// citedForms is the distinct fixtures every bundled answer plan of backend cites, in citation
// order.
func citedForms(t testing.TB, backend codingAgent) []bundledFixtureRecord {
	t.Helper()
	var forms []bundledFixtureRecord
	for _, cited := range bundledCitedPlans(t, backend) {
		for _, fixture := range cited.fixtures {
			if !slices.Contains(forms, fixture) {
				forms = append(forms, fixture)
			}
		}
	}
	return forms
}

func observeFixture(t testing.TB, fixture bundledFixtureRecord, change func(*[]string)) screenObservation {
	t.Helper()
	snapshot := replayAsCaptured(fixture.id, change, none[[2]uint16]())
	return observeBundled(t, fixture.backend, fixture.version, &snapshot)
}

func matchFixture(t testing.TB, fixture bundledFixtureRecord, change func(*[]string), plan *answerPlan) (answerMatch, bool) {
	t.Helper()
	snapshot := replayAsCaptured(fixture.id, change, none[[2]uint16]())
	matched, ok, err := bundledDetector(fixture.backend, fixture.version).matchAnswer(&snapshot, plan)
	if err != nil {
		t.Fatal(err)
	}
	return matched, ok
}

// lastRowStarting is the last row of lines starting with prefix.
func lastRowStarting(lines []string, prefix string) int {
	for row := len(lines) - 1; row >= 0; row-- {
		if strings.HasPrefix(lines[row], prefix) {
			return row
		}
	}
	panic(fmt.Sprintf("a row starts with %q", prefix))
}

// insertRows inserts rows below after and drops as many rows from the top, so the screen keeps
// its captured geometry.
func insertRows(lines *[]string, after int, rows []string) {
	grown := slices.Insert(slices.Clone(*lines), after+1, rows...)
	*lines = grown[len(rows):]
}

func assertNoRequestFrom(t testing.TB, label string, observed *screenObservation, request string) {
	t.Helper()
	if observed.kind == observationKindPermissionRequest || observed.ruleID == request {
		t.Fatalf("%s: %s %s", label, observed.kind.debugString(), observed.ruleID)
	}
}

const (
	piFocusMarker  = " → "
	piCommandRow   = " $ "
	piHintRow      = " ↑↓ navigate  enter select  escape/ctrl+c cancel"
	piHintWrapped  = " ↑↓ navigate  enter select  escape/ctrl+c"
	piHintWrapTail = "cancel"
)

func piRequest(t testing.TB) string {
	t.Helper()
	return bundledCitedPlans(t, codingAgentPi)[0].plan.capability.requestRuleID
}

func TestCodingAgentDetector_PiProfile_PiFocusFollowsTheMarkerRowForBothPlans(t *testing.T) {
	t.Parallel()
	plans := bundledCitedPlans(t, codingAgentPi)
	for _, form := range citedForms(t, codingAgentPi) {
		observed := observeFixture(t, form, nil)
		var focused []opt[string]
		for _, cited := range plans {
			matched, ok := matchFixture(t, form, nil, &cited.plan)
			if !ok {
				t.Fatalf("%s matches its %s plan", form.id, cited.plan.choiceID)
			}
			if !slices.Equal(matched.choices, observed.rawEvidence.choices.values) {
				t.Fatalf("%s: matched choices %q, observed %q", form.id, matched.choices, observed.rawEvidence.choices.values)
			}
			focused = append(focused, matched.focusedChoice)
		}
		if _, ok := focused[0].get(); !ok || slices.ContainsFunc(focused, func(focus opt[string]) bool { return focus != focused[0] }) {
			t.Fatalf("%s: every plan sees the one marker focus, got %v", form.id, focused)
		}
	}
}

func TestCodingAgentDetector_PiProfile_PiFormWithoutItsFocusMarkerIsNoRequestAndMatchesNoPlan(t *testing.T) {
	t.Parallel()
	plans := bundledCitedPlans(t, codingAgentPi)
	request := piRequest(t)
	for _, form := range citedForms(t, codingAgentPi) {
		unfocus := func(lines *[]string) {
			marked := lastRowStarting(*lines, piFocusMarker)
			(*lines)[marked] = strings.Replace((*lines)[marked], piFocusMarker, "   ", 1)
		}
		observed := observeFixture(t, form, unfocus)
		assertNoRequestFrom(t, form.id, &observed, request)
		for _, cited := range plans {
			if _, ok := matchFixture(t, form, unfocus, &cited.plan); ok {
				t.Fatalf("%s: the unfocused form matches the %s plan", form.id, cited.plan.choiceID)
			}
		}
	}
}

func TestCodingAgentDetector_PiProfile_PiStageGatesHoldOnTheirCapturedPreStageScreens(t *testing.T) {
	t.Parallel()
	for _, cited := range bundledCitedPlans(t, codingAgentPi) {
		held := make([]bool, len(cited.plan.stages))
		for _, fixture := range cited.fixtures {
			matched, ok := matchFixture(t, fixture, nil, &cited.plan)
			if !ok {
				t.Fatalf("%s matches its %s plan", fixture.id, cited.plan.choiceID)
			}
			focus, focused := matched.focusedChoice.get()
			gated := false
			for index := range cited.plan.stages {
				if gate, ok := cited.plan.stages[index].requiredFocusChoiceID(); ok && focused && gate == focus {
					held[index], gated = true, true
				}
			}
			if !gated {
				t.Fatalf("%s: focus %v holds no stage gate of the %s plan", fixture.id, matched.focusedChoice, cited.plan.choiceID)
			}
		}
		if slices.Contains(held, false) {
			t.Fatalf("%s plan: a cited pre-stage screen holds each stage gate, got %v", cited.plan.choiceID, held)
		}
	}
}

func TestCodingAgentDetector_PiProfile_PiDenyStageMovesOnlyTheFocusOfTheSameRequest(t *testing.T) {
	t.Parallel()
	compared := 0
	for _, cited := range bundledCitedPlans(t, codingAgentPi) {
		if len(cited.plan.stages) < 2 {
			continue
		}
		byCapture := map[string][]answerMatch{}
		for _, fixture := range cited.fixtures {
			matched, ok := matchFixture(t, fixture, nil, &cited.plan)
			if !ok {
				t.Fatalf("%s matches its %s plan", fixture.id, cited.plan.choiceID)
			}
			screen := fixtureDocument(fixture.id)
			capture := testJSONText(ja(jget(screen, "rows"), jget(screen, "cols"), jget(screen, "alternate_screen"), jget(screen, "cursor_row") == nil))
			byCapture[capture] = append(byCapture[capture], matched)
		}
		for capture, matches := range byCapture {
			for _, before := range matches {
				for _, after := range matches {
					if before.focusedChoice == after.focusedChoice {
						continue
					}
					compared++
					if !before.semantic.equal(&after.semantic) || before.focusCapture != after.focusCapture || before.focus == after.focus {
						t.Fatalf("%s %s: a stage moves only the focus of the same request", cited.plan.choiceID, capture)
					}
				}
			}
		}
	}
	if compared == 0 {
		t.Fatal("a multi-stage pi plan cites screens of two focuses on one capture")
	}
}

func TestCodingAgentDetector_PiProfile_PiFormWithAnyRowBelowItsSingleOperationRowYieldsNoRequest(t *testing.T) {
	t.Parallel()
	request := piRequest(t)
	extraRows := [][]string{
		{"   --second line"},
		{"   second line", "   third line"},
		{"   $ ./permission-probe allow"},
	}
	for _, form := range citedForms(t, codingAgentPi) {
		for _, rows := range extraRows {
			observed := observeFixture(t, form, func(lines *[]string) {
				insertRows(lines, lastRowStarting(*lines, piCommandRow), rows)
			})
			assertNoRequestFrom(t, fmt.Sprintf("%s: %q", form.id, rows), &observed, request)
		}
	}
}

func TestCodingAgentDetector_PiProfile_PiCommandLineEqualToAChoiceLabelYieldsNoRequest(t *testing.T) {
	t.Parallel()
	request := piRequest(t)
	for _, form := range citedForms(t, codingAgentPi) {
		for _, label := range observeFixture(t, form, nil).rawEvidence.choices.values {
			observed := observeFixture(t, form, func(lines *[]string) {
				insertRows(lines, lastRowStarting(*lines, piCommandRow), []string{"   " + label})
			})
			assertNoRequestFrom(t, fmt.Sprintf("%s: %q", form.id, label), &observed, request)
		}
	}
}

func TestCodingAgentDetector_PiProfile_PiFormsPastTheirLayoutLimitsYieldNoRequest(t *testing.T) {
	t.Parallel()
	request := piRequest(t)
	tall := make([]string, 20)
	for n := range tall {
		tall[n] = fmt.Sprintf("   line %d", n)
	}
	for _, form := range citedForms(t, codingAgentPi) {
		wrapped := observeFixture(t, form, func(lines *[]string) {
			insertRows(lines, lastRowStarting(*lines, piCommandRow), []string{" xxxxxxxx"})
		})
		assertNoRequestFrom(t, form.id+": wrapped command", &wrapped, request)
		taller := observeFixture(t, form, func(lines *[]string) {
			insertRows(lines, lastRowStarting(*lines, piCommandRow), tall)
		})
		assertNoRequestFrom(t, form.id+": taller than max_rows", &taller, request)
		hintWrapped := observeFixture(t, form, func(lines *[]string) {
			hint := lastRowStarting(*lines, piHintRow)
			(*lines)[hint] = piHintWrapped
			(*lines)[hint+1] = piHintWrapTail
		})
		assertNoRequestFrom(t, form.id+": wrapped hint row", &hintWrapped, request)
	}
}

// verifiedEmptyComposerOf is a fixture the backend's composer capability cites, of the form's
// screen mode and at least its width (its geometry when one exists), that the bundled detector
// verifies as an empty composer.
func verifiedEmptyComposerOf(t testing.TB, form bundledFixtureRecord) bundledFixtureRecord {
	t.Helper()
	versions := map[string]string{}
	for _, fixture := range bundledFixtureRecords() {
		versions[fixture.id] = fixture.version
	}
	want := fixtureDocument(form.id)
	var wider []bundledFixtureRecord
	for _, id := range jarray(jget(parsedBundledFiles(), form.backend.name(), "profile", "capabilities", "composer", "fixture_ids")).items {
		name, _ := jstring(id)
		screen := fixtureDocument(name)
		if sizeOf(screen).cols < sizeOf(want).cols || fixtureAlternate(screen) != fixtureAlternate(want) || (jget(screen, "cursor_row") == nil) != (jget(want, "cursor_row") == nil) {
			continue
		}
		composer := bundledFixtureRecord{backend: form.backend, id: name, version: versions[name]}
		if observed := observeFixture(t, composer, nil); observed.kind != observationKindComposerEmpty || !observed.verifiedNoninteraction {
			continue
		}
		if sizeOf(screen) == sizeOf(want) {
			return composer
		}
		wider = append(wider, composer)
	}
	if len(wider) == 0 {
		t.Fatalf("%s: a verified empty composer fixture of its screen mode and width", form.id)
	}
	return wider[0]
}

func TestCodingAgentDetector_PiProfile_PiGateFormTextOutsideALiveFormIsNoRequest(t *testing.T) {
	t.Parallel()
	request := piRequest(t)
	for _, form := range citedForms(t, codingAgentPi) {
		region, ok := observeFixture(t, form, nil).matchedRegion.get()
		if !ok {
			t.Fatalf("%s: the form's live region", form.id)
		}
		formRows := fixtureLines(form.id)[region.top:region.bottom]
		withoutHint := formRows[:len(formRows)-1]
		composer := verifiedEmptyComposerOf(t, form)
		idle := observeFixture(t, composer, nil)
		idleRegion, _ := idle.matchedRegion.get()
		if int(idleRegion.top) <= 1+len(withoutHint) {
			t.Fatalf("%s: the form text fits above the idle editor of %s", form.id, composer.id)
		}
		transcript := observeFixture(t, composer, func(lines *[]string) {
			copy((*lines)[1:], withoutHint)
		})
		assertNoRequestFrom(t, form.id+": form text above the idle editor", &transcript, request)
		quoted := observeFixture(t, form, func(lines *[]string) {
			for row := region.top; row < region.bottom; row++ {
				if (*lines)[row] != "" {
					(*lines)[row] = "    " + (*lines)[row]
				}
			}
		})
		assertNoRequestFrom(t, form.id+": quoted form", &quoted, request)
		primary := replayNativeFixture(bundledScreen(form.id), nil, false, none[[2]uint16]())
		onPrimary := observeBundled(t, form.backend, form.version, &primary)
		assertNoRequestFrom(t, form.id+": primary screen", &onPrimary, request)
	}
}

func TestCodingAgentDetector_PiProfile_PiRetainedDraftsAndCursorOutsideTheEditorRemainUnverified(t *testing.T) {
	t.Parallel()
	assertNotIdle := func(label string, observed *screenObservation) {
		t.Helper()
		if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
			t.Fatalf("%s: %s verified=%v", label, observed.kind.debugString(), observed.verifiedNoninteraction)
		}
	}
	drafts := 0
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend != codingAgentPi {
			continue
		}
		literals, err := preSubmitLiterals(fixtureDocument(fixture.id))
		if err != nil {
			t.Fatal(err)
		}
		_, hasCursor := fixtureCursor(fixtureDocument(fixture.id)).get()
		if len(literals) > 0 && hasCursor {
			drafts++
			observed := observeFixture(t, fixture, nil)
			assertNotIdle(fixture.id+": a retained draft", &observed)
		}
	}
	if drafts == 0 {
		t.Fatal("a pi fixture retains a typed literal")
	}
	for _, form := range citedForms(t, codingAgentPi) {
		observed := observeFixture(t, form, nil)
		assertNotIdle(form.id, &observed)
		composer := verifiedEmptyComposerOf(t, form)
		away := replayAsCaptured(composer.id, nil, some([2]uint16{0, 0}))
		awayObserved := observeBundled(t, codingAgentPi, composer.version, &away)
		assertNotIdle(composer.id+": cursor outside the editor", &awayObserved)
	}
}

// detectionForms is every fixture backend's detection capability cites.
func detectionForms(t testing.TB, backend codingAgent) []bundledFixtureRecord {
	t.Helper()
	versions := map[string]string{}
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend == backend {
			versions[fixture.id] = fixture.version
		}
	}
	var forms []bundledFixtureRecord
	for _, id := range jarray(jget(parsedBundledFiles(), backend.name(), "profile", "capabilities", "detection", "fixture_ids")).items {
		name, _ := jstring(id)
		version, ok := versions[name]
		if !ok {
			t.Fatalf("%s: a cited fixture of the backend", name)
		}
		forms = append(forms, bundledFixtureRecord{backend: backend, id: name, version: version})
	}
	if len(forms) == 0 {
		t.Fatalf("%s: a detection fixture", backend.name())
	}
	return forms
}

func TestCodingAgentDetector_Claude21312Guards_TrailingPaddingKeepsEachNewRuleIdentity(t *testing.T) {
	t.Parallel()
	const padding = 4
	for _, form := range detectionForms(t, codingAgentClaude) {
		plain := observeFixture(t, form, nil)
		if _, interaction := plain.kind.interaction(); !interaction {
			continue
		}
		cols := int(fixtureSize(form.id).cols)
		padded := observeFixture(t, form, func(lines *[]string) {
			for row, line := range *lines {
				if strings.TrimSpace(line) != "" && utf8.RuneCountInString(line)+padding < cols {
					(*lines)[row] = line + strings.Repeat(" ", padding)
				}
			}
		})
		if padded.ruleID != plain.ruleID || padded.kind != plain.kind || padded.rawEvidence.operation != plain.rawEvidence.operation ||
			padded.rawEvidence.operationRows != plain.rawEvidence.operationRows || padded.rawEvidence.choices.set != plain.rawEvidence.choices.set ||
			!slices.Equal(padded.rawEvidence.choices.values, plain.rawEvidence.choices.values) || !fingerprintsEqual(padded.fingerprint, plain.fingerprint) {
			t.Fatalf("%s +%d: %s %s, plain %s %s", form.id, padding, padded.kind.debugString(), padded.ruleID, plain.kind.debugString(), plain.ruleID)
		}
	}
}

// bundledFixtureByID is the bundled fixture record of id.
func bundledFixtureByID(id string) bundledFixtureRecord {
	for _, fixture := range bundledFixtureRecords() {
		if fixture.id == id {
			return fixture
		}
	}
	panic(id + " is a bundled fixture")
}

// assertEditedLeavesNoInteraction asserts that the unedited fixture is an interaction and every
// edited replay is neither an interaction nor the unedited screen's rule.
func assertEditedLeavesNoInteraction(t testing.TB, id string, edits map[string]func(*[]string), cursors []opt[[2]uint16]) {
	t.Helper()
	fixture := bundledFixtureByID(id)
	unedited := observeFixture(t, fixture, nil)
	if _, interaction := unedited.kind.interaction(); !interaction {
		t.Fatalf("%s: the unedited screen is an interaction, got %s", id, unedited.kind.debugString())
	}
	for _, name := range slices.Sorted(maps.Keys(edits)) {
		for _, cursor := range cursors {
			snapshot := replayAsCaptured(id, edits[name], cursor)
			observed := observeBundled(t, fixture.backend, fixture.version, &snapshot)
			if _, interaction := observed.kind.interaction(); interaction || observed.ruleID == unedited.ruleID {
				t.Fatalf("%s %s cursor %v: %s %s", id, name, cursor, observed.kind.debugString(), observed.ruleID)
			}
		}
	}
}

func setRow(row int, text string) func(*[]string) {
	return func(lines *[]string) { (*lines)[row] = text }
}

const (
	claudeLiveComposerRow     = "❯\U000000a0"
	claudeTranscriptViewerRow = "  Showing detailed transcript · ctrl+o to toggle"
	claudeAskSingle           = "claude-2.1.322-ask-single-tmux-120-1"
	claudeNudge               = "claude-2.1.312-auto-default-nudge-1"
	claudePendingNotice       = "claude-2.1.312-question-pending-notice-1"
	claudeNoticeText          = "Claude has a question for you — it shows once you send or clear what you're typing."
)

func TestCodingAgentDetector_ClaudeAskUserQuestion_TheLiveFormWithALiveComposerRowWrittenIntoItIsNoInteraction(t *testing.T) {
	t.Parallel()
	assertEditedLeavesNoInteraction(t, claudeAskSingle,
		map[string]func(*[]string){"a live composer row": setRow(30, claudeLiveComposerRow)},
		[]opt[[2]uint16]{none[[2]uint16](), some([2]uint16{30, 2})})
}

func TestCodingAgentDetector_ClaudeAskUserQuestion_TheLiveFormWithTheTranscriptViewerFooterWrittenIntoItIsNoInteraction(t *testing.T) {
	t.Parallel()
	assertEditedLeavesNoInteraction(t, claudeAskSingle,
		map[string]func(*[]string){"the transcript viewer footer": setRow(30, claudeTranscriptViewerRow)},
		[]opt[[2]uint16]{none[[2]uint16]()})
}

func TestCodingAgentDetector_Claude21312Guards_ADialogAQuotedAboveALiveComposerIsNoInteraction(t *testing.T) {
	t.Parallel()
	assertEditedLeavesNoInteraction(t, claudeNudge,
		map[string]func(*[]string){"a live composer below": setRow(33, claudeLiveComposerRow)},
		[]opt[[2]uint16]{none[[2]uint16](), some([2]uint16{33, 2})})
}

func TestCodingAgentDetector_Claude21312Guards_APendingNoticeOutsideItsLiveFrameOrMarginIsNoInteraction(t *testing.T) {
	t.Parallel()
	assertEditedLeavesNoInteraction(t, claudePendingNotice, map[string]func(*[]string){
		"assistant quote":    setRow(19, "● "+claudeNoticeText),
		"continuation quote": setRow(19, "  "+claudeNoticeText),
		"quoted sentence":    setRow(19, "The notice said: "+claudeNoticeText),
		"transcript viewer":  setRow(30, claudeTranscriptViewerRow),
		"no live composer":   setRow(33, "❯ Reply with the single word done."),
	}, []opt[[2]uint16]{none[[2]uint16]()})
}

func TestCodingAgentDetector_Claude21312Guards_ABatchedHeadingOutsideItsLiveFormIsNoInteraction(t *testing.T) {
	t.Parallel()
	assertEditedLeavesNoInteraction(t, "claude-2.1.312-read-file-batched-permission-1", map[string]func(*[]string){
		"assistant quote":            func(lines *[]string) { (*lines)[18] = "●" + (*lines)[18][1:] },
		"counter without a heading":  setRow(18, fmt.Sprintf("%120s", "1 of 2")),
		"counter on another heading": setRow(18, fmt.Sprintf(" Read files%109s", "1 of 2")),
		"no live footer":             setRow(27, ""),
	}, []opt[[2]uint16]{none[[2]uint16]()})
}

func TestCodingAgentDetector_Claude21312Guards_PrimaryScreenRemnantsAfterTheFormClosesProduceNoInteraction(t *testing.T) {
	t.Parallel()
	for _, id := range []string{
		"claude-2.1.312-primary-closed-form-1",
		"claude-2.1.312-primary-idle-composer-1",
		"claude-2.1.312-primary-resized-narrow-1",
		"claude-2.1.312-primary-resized-idle-composer-1",
	} {
		fixture := bundledFixtureByID(id)
		for _, cursor := range []opt[[2]uint16]{none[[2]uint16](), some([2]uint16{6, 2}), some([2]uint16{10, 2}), some([2]uint16{11, 5})} {
			snapshot := replayAsCaptured(id, nil, cursor)
			observed := observeBundled(t, fixture.backend, fixture.version, &snapshot)
			if _, interaction := observed.kind.interaction(); interaction {
				t.Fatalf("%s cursor %v: %s", id, cursor, observed.kind.debugString())
			}
		}
	}
}

const (
	claudeNativePermission  = "claude-2.1.302-default-permission-1"
	claudeWrappedPermission = "claude-2.1.302-wrapped-permission-1"
	claudeFormVersion       = "2.1.302"
	claudeReadOutside       = "claude-2.1.309-read-outside-permission-1"
	claudeAutoIdle          = "claude-2.1.302-auto-idle-composer-1"
)

func claudeObserve(t testing.TB, id string, change func(*[]string), cursor opt[[2]uint16]) screenObservation {
	t.Helper()
	snapshot := replayNativeFixture(bundledScreen(id), change, true, cursor)
	return observeBundled(t, codingAgentClaude, claudeFormVersion, &snapshot)
}

func claudeNative(t testing.TB, change func(*[]string)) screenObservation {
	t.Helper()
	return claudeObserve(t, claudeNativePermission, change, none[[2]uint16]())
}

func claudeWrapped(t testing.TB, change func(*[]string)) screenObservation {
	t.Helper()
	return claudeObserve(t, claudeWrappedPermission, change, none[[2]uint16]())
}

// claudeNativeWithOperationRows replaces the 120-column fixture's single operation row (row 18)
// with rows, taking the extra rows from the empty masked header so every later row keeps its
// position.
func claudeNativeWithOperationRows(t testing.TB, rows ...string) screenObservation {
	t.Helper()
	return claudeNative(t, func(lines *[]string) {
		at := 18 - (len(rows) - 1)
		kept := slices.Delete(slices.Clone(*lines)[len(rows)-1:], at, at+1)
		*lines = slices.Insert(kept, at, rows...)
	})
}

func claudeUnwrappedRendering(t testing.TB, operation string) screenObservation {
	t.Helper()
	return claudeNativeWithOperationRows(t, " "+operation)
}

func claudeWrappedRows(first, second string) func(*[]string) {
	return func(lines *[]string) {
		(*lines)[22] = first
		(*lines)[23] = second
	}
}

// assertClaudeRequest asserts a complete permission request of the unedited native form's rule,
// version and choices that displays operation, one line per displayed row.
func assertClaudeRequest(t testing.TB, observed *screenObservation, reference *screenObservation, operation string) {
	t.Helper()
	got, _ := observed.rawEvidence.operation.get()
	rows, _ := observed.rawEvidence.operationRows.get()
	if observed.kind != observationKindPermissionRequest || observed.matchEvidence != matchEvidenceComplete || observed.ruleID != reference.ruleID ||
		observed.rawEvidence.nativeVersion != reference.rawEvidence.nativeVersion || got != operation || rows != uint64(len(strings.Split(operation, "\n"))) ||
		!slices.Equal(observed.rawEvidence.choices.values, reference.rawEvidence.choices.values) || observed.fingerprint == nil || observed.verifiedNoninteraction {
		t.Fatalf("%q: %s %s %s operation %q rows %d", operation, observed.kind.debugString(), observed.matchEvidence.debugString(), observed.ruleID, got, rows)
	}
}

func assertNotPermission(t testing.TB, label string, observed *screenObservation) {
	t.Helper()
	if observed.kind == observationKindPermissionRequest {
		t.Fatalf("%s: a permission request of %s", label, observed.ruleID)
	}
}

func observedOperation(t testing.TB, observed *screenObservation) string {
	t.Helper()
	operation, ok := observed.rawEvidence.operation.get()
	if !ok {
		t.Fatal("an observed operation")
	}
	return operation
}

func TestCodingAgentDetector_BundledClaudeReadOutsideDialogWithTrailingRepaintPaddingKeepsItsRequestIdentity(t *testing.T) {
	t.Parallel()
	read := func(change func(*[]string)) screenObservation {
		t.Helper()
		snapshot := replayNativeFixture(bundledScreen(claudeReadOutside), change, true, none[[2]uint16]())
		return observeBundled(t, codingAgentClaude, "2.1.309", &snapshot)
	}
	clean := read(nil)
	if clean.kind != observationKindPermissionRequest || clean.matchEvidence != matchEvidenceComplete {
		t.Fatalf("the clean dialog: %s %s", clean.kind.debugString(), clean.matchEvidence.debugString())
	}
	padded := read(func(lines *[]string) {
		for _, row := range []int{16, 18, 27, 28, 29, 30, 31, 33} {
			(*lines)[row] += strings.Repeat(" ", 40)
		}
	})
	assertSameRequestAs(t, "trailing repaint padding", &padded, &clean)
}

func TestCodingAgentDetector_BundledClaudeReadOutsideDialogOutsideALiveRegionOrOnTheIdleComposerIsNoRequest(t *testing.T) {
	t.Parallel()
	observe := func(id string, change func(*[]string), cursor opt[[2]uint16]) screenObservation {
		t.Helper()
		snapshot := replayNativeFixture(bundledScreen(id), change, true, cursor)
		return observeBundled(t, codingAgentClaude, "2.1.309", &snapshot)
	}
	dialog := observe(claudeReadOutside, nil, none[[2]uint16]())
	idle := observe(claudeAutoIdle, nil, none[[2]uint16]())
	rows := fixtureLines(claudeReadOutside)[16:34]
	quoted := observe(claudeAutoIdle, func(lines *[]string) { copy((*lines)[12:], rows) }, none[[2]uint16]())
	assertNotPermission(t, "dialog quoted above the composer", &quoted)
	if quoted.ruleID == dialog.ruleID || quoted.kind != idle.kind || quoted.fingerprint != nil {
		t.Fatalf("the quoted dialog: %s %s, the idle composer %s", quoted.kind.debugString(), quoted.ruleID, idle.kind.debugString())
	}
	if outside := observe(claudeReadOutside, nil, some([2]uint16{2, 0})); outside.ruleID == dialog.ruleID {
		t.Fatal("the cursor lies outside the dialog region")
	}
}

func TestCodingAgentDetector_BundledClaudeDetectorRejectsCopiesThatAreNotTheLivePermissionForm(t *testing.T) {
	t.Parallel()
	primarySnapshot := replayNativeFixture(bundledScreen(claudeNativePermission), nil, false, none[[2]uint16]())
	primary := observeBundled(t, codingAgentClaude, claudeFormVersion, &primarySnapshot)
	alternate := claudeNative(t, nil)
	assertPrimaryMatchesAlternate(t, "permission form", &primary, &alternate)
	unfocused := claudeNative(t, func(lines *[]string) { (*lines)[23] = strings.ReplaceAll((*lines)[23], "❯", " ") })
	assertNotPermission(t, "form without a focus marker", &unfocused)
	broadened := claudeNative(t, func(lines *[]string) { (*lines)[23], (*lines)[26] = "", "" })
	assertNotPermission(t, "only broadened choices remain", &broadened)
	headless := claudeNative(t, setRow(14, ""))
	assertNotPermission(t, "form without its heading", &headless)
}

func TestCodingAgentDetector_BundledClaudeWrappedIdentityFollowsTheDisplayedRowsSoARewrapReplacesIt(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	wrapped := claudeWrapped(t, nil)
	command := strings.ReplaceAll(observedOperation(t, &wrapped), "\n", "")
	unwrapped := claudeUnwrappedRendering(t, command)
	assertClaudeRequest(t, &unwrapped, &native, command)
	if fingerprintsEqual(wrapped.fingerprint, unwrapped.fingerprint) {
		t.Fatal("the 52-column and 120-column renderings display different rows")
	}
	if again := claudeWrapped(t, nil); !fingerprintsEqual(wrapped.fingerprint, again.fingerprint) {
		t.Fatal("a redraw of the same displayed rows keeps the identity")
	}
	nativeOperation := []rune(observedOperation(t, &native))
	first, second := string(nativeOperation[:50]), string(nativeOperation[50:])
	rewrapped := claudeWrapped(t, claudeWrappedRows(" "+first, " "+second))
	assertClaudeRequest(t, &rewrapped, &native, first+"\n"+second)
	if fingerprintsEqual(rewrapped.fingerprint, native.fingerprint) {
		t.Fatal("the native 120-column request rewrapped at 52 columns is a new episode")
	}
	narrow := claudeWrapped(t, claudeWrappedRows(" /tmp/cafleet-194/claude-wrap-cells-001", " permission-probe allow"))
	assertClaudeRequest(t, &narrow, &native, "/tmp/cafleet-194/claude-wrap-cells-001\npermission-probe allow")
}

func TestCodingAgentDetector_BundledClaudePlainOperationRowsKeepTheirLineBreakWhereverARowEnds(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	full := " /tmp/cafleet-194/claude-wrap-cells-001/permission-"
	fullAtSpace := " /tmp/cafleet-194/claude-wrap-cells-001/probe-boxes"
	for _, row := range []string{full, fullAtSpace} {
		if utf8.RuneCountInString(row) != 51 {
			t.Fatalf("%q reaches the last usable column", row)
		}
	}
	cases := [][2]string{
		{" /tmp/cafleet-194/claude-wrap-cells-001", " permission-probe allow"},
		{full, " probe allow"},
		{fullAtSpace, " allow"},
	}
	for _, rows := range cases {
		observed := claudeWrapped(t, claudeWrappedRows(rows[0], rows[1]))
		assertClaudeRequest(t, &observed, &native, rows[0][1:]+"\n"+rows[1][1:])
	}
	for _, filler := range []int{28, 27} {
		row := " /tmp/cafleet-194/界界/" + strings.Repeat("a", filler)
		if utf8.RuneCountInString(row)+2 != 23+filler {
			t.Fatalf("%q: each 界 occupies two display cells", row)
		}
		observed := claudeWrapped(t, claudeWrappedRows(row, " b allow"))
		assertClaudeRequest(t, &observed, &native, row[1:]+"\nb allow")
	}
}

func TestCodingAgentDetector_BundledClaudeOperationRowsRemoveExactlyOneMarginSpace(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	cases := []struct {
		rows      []string
		operation string
	}{
		{[]string{"  indented continuation", " plain"}, " indented continuation\nplain"},
		{[]string{" │ echo plain bar", " │ next"}, "│ echo plain bar\n│ next"},
	}
	for _, c := range cases {
		observed := claudeNativeWithOperationRows(t, c.rows...)
		assertClaudeRequest(t, &observed, &native, c.operation)
	}
}

func TestCodingAgentDetector_BundledClaudeOperationIsOnlyTheRowsBetweenTheDashedRules(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	wrapped := claudeWrapped(t, nil)
	base := " Run the permission probe with the allow argument"
	described := claudeWrapped(t, setRow(20, base+strings.Repeat("x", 51-utf8.RuneCountInString(base))))
	assertClaudeRequest(t, &described, &native, observedOperation(t, &wrapped))
}

func TestCodingAgentDetector_BundledClaudeSyntheticBoxedOperationRowsKeepTheirGutterAndEachLineBreak(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	cases := [][]string{
		{" │ /tmp/cafleet-194/claude-wrap-cells-001/permission-probe", " │ allow"},
		{" │ /tmp/cafleet-194/claude-wrap-cells-001/permission-probe", " │ allow --reason", " │ boxed"},
	}
	for _, rows := range cases {
		stripped := make([]string, len(rows))
		for index, row := range rows {
			stripped[index] = strings.TrimPrefix(row, " ")
		}
		operation := strings.Join(stripped, "\n")
		boxed := claudeNativeWithOperationRows(t, rows...)
		assertClaudeRequest(t, &boxed, &native, operation)
		command := strings.Replace(strings.ReplaceAll(operation, "\n│ ", " "), "│ ", "", 1)
		if unwrapped := claudeUnwrappedRendering(t, command); fingerprintsEqual(boxed.fingerprint, unwrapped.fingerprint) {
			t.Fatalf("a boxed wrap of %d rows is width-dependent", len(rows))
		}
	}
}

func TestCodingAgentDetector_BundledClaudeBoxedRowReachingTheLastUsableColumnKeepsItsLineBreak(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	widePath := "/tmp/cafleet-194/" + strings.Repeat("w", 82) + "/permission-probe"
	wideRow := " │ " + widePath
	if utf8.RuneCountInString(wideRow) != 119 {
		t.Fatal("the wide row reaches column 119 of 120")
	}
	wide := claudeNativeWithOperationRows(t, wideRow, " │ allow")
	assertClaudeRequest(t, &wide, &native, "│ "+widePath+"\n│ allow")
	narrowRow := " │ /tmp/cafleet-194/claude-wrap-cells-001/probe-box"
	if utf8.RuneCountInString(narrowRow) != 51 {
		t.Fatal("the narrow row reaches column 51 of 52")
	}
	narrow := claudeWrapped(t, claudeWrappedRows(narrowRow, " │ allow"))
	assertClaudeRequest(t, &narrow, &native, "│ /tmp/cafleet-194/claude-wrap-cells-001/probe-box\n│ allow")
}

func TestCodingAgentDetector_BundledClaudeMultiLineOperationKeepsItsLinesInThePlainAndBoxedShapes(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	plain := claudeWrapped(t, claudeWrappedRows(" echo ok # cleanup", " rm -rf ~/x"))
	unwrappedPlain := claudeNativeWithOperationRows(t, " echo ok # cleanup", " rm -rf ~/x")
	boxed := claudeNativeWithOperationRows(t, " │ echo ok # cleanup", " │ rm -rf ~/x")
	oneLine := claudeUnwrappedRendering(t, "echo ok # cleanup rm -rf ~/x")
	cases := []struct {
		shape     string
		observed  *screenObservation
		operation string
	}{
		{"plain at 52 columns", &plain, "echo ok # cleanup\nrm -rf ~/x"},
		{"plain at 120 columns", &unwrappedPlain, "echo ok # cleanup\nrm -rf ~/x"},
		{"boxed at 120 columns", &boxed, "│ echo ok # cleanup\n│ rm -rf ~/x"},
	}
	for _, c := range cases {
		assertClaudeRequest(t, c.observed, &native, c.operation)
		if fingerprintsEqual(c.observed.fingerprint, oneLine.fingerprint) {
			t.Fatalf("%s never reads as one commented-out line", c.shape)
		}
	}
	if !fingerprintsEqual(plain.fingerprint, unwrappedPlain.fingerprint) {
		t.Fatal("the plain shapes display the same rows")
	}
	if fingerprintsEqual(plain.fingerprint, boxed.fingerprint) {
		t.Fatal("the boxed rows display their gutter")
	}
}

// unboxedCommand is a boxed operation's command: every row without its gutter, joined by spaces.
func unboxedCommand(operation string) string {
	rows := strings.Split(operation, "\n")
	for index, row := range rows {
		rows[index] = strings.TrimPrefix(row, "│ ")
	}
	return strings.Join(rows, " ")
}

func TestCodingAgentDetector_BundledClaudeDetectsTheNativeBoxedFormAndReportsItsDisplayedGutterRows(t *testing.T) {
	t.Parallel()
	const boxedForm = "claude-2.1.302-boxed-permission-1"
	observed := claudeObserve(t, boxedForm, nil, none[[2]uint16]())
	operation := observedOperation(t, &observed)
	if unwrapped := claudeUnwrappedRendering(t, unboxedCommand(operation)); fingerprintsEqual(observed.fingerprint, unwrapped.fingerprint) {
		t.Fatal("a boxed wrap is width-dependent")
	}
	cleared := claudeObserve(t, boxedForm, setRow(33, ""), none[[2]uint16]())
	assertSameRequestAs(t, "the written spaces before the closing line are no part of the request", &cleared, &observed)
}

func TestCodingAgentDetector_BundledClaudeDetectsTheNativeSingleRowBoxAndKeepsItsGutter(t *testing.T) {
	t.Parallel()
	observed := claudeObserve(t, "claude-2.1.302-boxed-single-row-permission-1", nil, none[[2]uint16]())
	operation := observedOperation(t, &observed)
	if !strings.HasPrefix(operation, "│ ") || strings.Contains(operation, "\n") {
		t.Fatalf("a single-row box keeps its gutter on one row: %q", operation)
	}
	if unwrapped := claudeUnwrappedRendering(t, unboxedCommand(operation)); fingerprintsEqual(observed.fingerprint, unwrapped.fingerprint) {
		t.Fatal("a single-row box displays a different row from a plain row of one command")
	}
}

func TestCodingAgentDetector_BundledClaudeWrappedFormKeepsTheFalsePositiveGuards(t *testing.T) {
	t.Parallel()
	native := claudeNative(t, nil)
	wrapped := claudeWrapped(t, nil)
	cases := []struct {
		name   string
		change func(*[]string)
	}{
		{"a heading copy above the form", setRow(15, " Bash command")},
		{"a second heading inside the form", setRow(26, " Bash command")},
		{"form without a focus marker", func(lines *[]string) { (*lines)[28] = strings.ReplaceAll((*lines)[28], "❯", " ") }},
		{"form without its heading", setRow(17, "")},
	}
	for _, c := range cases {
		observed := claudeWrapped(t, c.change)
		assertNotPermission(t, c.name, &observed)
	}
	outside := claudeObserve(t, claudeWrappedPermission, nil, some([2]uint16{12, 0}))
	assertNotPermission(t, "cursor outside the form", &outside)
	grown := func(extra int) screenObservation {
		return claudeWrapped(t, func(lines *[]string) {
			kept := slices.Clone(*lines)[extra:]
			*lines = slices.Insert(kept, 26-extra, make([]string, extra)...)
		})
	}
	tallest := grown(5)
	assertClaudeRequest(t, &tallest, &native, observedOperation(t, &wrapped))
	taller := grown(6)
	assertNotPermission(t, "a form taller than 24 rows", &taller)
}

const (
	codexNativeApproval = "codex-0.163.0-on-request-permission-1"
	codexFormVersion    = "0.163.0"
)

func codexForm(t testing.TB, change func(*[]string), cursor opt[[2]uint16]) screenObservation {
	t.Helper()
	snapshot := replayNativeFixture(bundledScreen(codexNativeApproval), change, true, cursor)
	return observeBundled(t, codingAgentCodex, codexFormVersion, &snapshot)
}

// replayWithPendingWrap writes the full-width cursor row last, which leaves the terminal in its
// pending-wrap state that an absolute cursor move cannot reach.
func replayWithPendingWrap(t testing.TB, id string) screenSnapshot {
	t.Helper()
	at, ok := fixtureCursor(fixtureDocument(id)).get()
	if !ok {
		t.Fatalf("%s records its cursor", id)
	}
	lines := fixtureLines(id)
	terminal := mustTerminalState(t, fixtureSize(id))
	mustFeed(t, terminal, []byte("\x1b[?1049h\x1b[2J"))
	for index, line := range lines {
		if index != int(at[0]) && line != "" {
			mustFeed(t, terminal, fmt.Appendf(nil, "\x1b[%d;1H%s", index+1, line))
		}
	}
	mustFeed(t, terminal, fmt.Appendf(nil, "\x1b[%d;1H%s", at[0]+1, lines[at[0]]))
	return terminal.snapshot()
}

func TestCodingAgentDetector_BundledCodexDetectorReplaysTheNativeApprovalFormFromUnmaskedRows(t *testing.T) {
	t.Parallel()
	observed := codexForm(t, nil, none[[2]uint16]())
	if observed.kind != observationKindPermissionRequest || observed.matchEvidence != matchEvidenceComplete || observed.fingerprint == nil {
		t.Fatalf("the native approval form: %s %s", observed.kind.debugString(), observed.matchEvidence.debugString())
	}
	replayed := codexForm(t, nil, none[[2]uint16]())
	assertSameRequestAs(t, "a replay", &replayed, &observed)
	snapshot := replayNativeFixture(bundledScreen(codexNativeApproval), nil, true, none[[2]uint16]())
	reused := observeBundled(t, codingAgentCodex, "0.164.0", &snapshot)
	if reused.kind != observed.kind || reused.ruleID != observed.ruleID || !fingerprintsEqual(reused.fingerprint, observed.fingerprint) ||
		reused.rawEvidence.nativeVersion != some("0.164.0") {
		t.Fatalf("another run version: %s %s %v", reused.kind.debugString(), reused.ruleID, reused.rawEvidence.nativeVersion)
	}
}

func TestCodingAgentDetector_BundledCodexDetectionCountsAPendingWrapCursorOnTheFullWidthClosingRowAsInside(t *testing.T) {
	t.Parallel()
	forms := []bundledFixtureRecord{bundledFixtureByID(codexNativeApproval)}
	forms = append(forms, citedForms(t, codingAgentCodex)...)
	for _, form := range forms {
		screen := replayWithPendingWrap(t, form.id)
		at, _ := fixtureCursor(fixtureDocument(form.id)).get()
		if cursor, _ := screen.cursor.get(); cursor != [2]uint16{at[0], fixtureSize(form.id).cols} {
			t.Fatalf("%s: the live terminal reports the pending-wrap column, got %v", form.id, cursor)
		}
		captured := observeFixture(t, form, nil)
		pending := observeBundled(t, form.backend, form.version, &screen)
		assertSameRequestAs(t, form.id+": a pending-wrap cursor on the closing row lies inside the form", &pending, &captured)
	}
}

func TestCodingAgentDetector_BundledCodexDetectorRejectsCopiesThatAreNotTheLiveApprovalForm(t *testing.T) {
	t.Parallel()
	primarySnapshot := replayNativeFixture(bundledScreen(codexNativeApproval), nil, false, none[[2]uint16]())
	primary := observeBundled(t, codingAgentCodex, codexFormVersion, &primarySnapshot)
	assertNotPermission(t, "primary-screen transcript copy", &primary)
	cases := []struct {
		name   string
		change func(*[]string)
		cursor opt[[2]uint16]
	}{
		{"form without a focus marker", func(lines *[]string) { (*lines)[31] = strings.Replace((*lines)[31], "›", " ", 1) }, none[[2]uint16]()},
		{"only the prefix-approval choice remains", func(lines *[]string) { (*lines)[31], (*lines)[33] = "", "" }, none[[2]uint16]()},
		{"form without its heading", setRow(22, ""), none[[2]uint16]()},
		{"form without its reason line", setRow(26, ""), none[[2]uint16]()},
		{"cursor outside the form", nil, some([2]uint16{12, 0})},
	}
	for _, c := range cases {
		observed := codexForm(t, c.change, c.cursor)
		assertNotPermission(t, c.name, &observed)
	}
}

// codexContinuedChoiceTwo moves choice 2's "(p)" shortcut onto a continuation row.
func codexContinuedChoiceTwo(lines *[]string) {
	choice, ok := strings.CutSuffix((*lines)[32], " (p)")
	if !ok {
		panic("choice 2 fits one row")
	}
	moved := slices.Delete(slices.Clone(*lines), 12, 13)
	moved[31] = choice
	*lines = slices.Insert(moved, 32, "     (p)")
}

func TestCodingAgentDetector_BundledCodexFormWithChoiceTwoOnOneRowOrContinuedDetectsTheSameRequest(t *testing.T) {
	t.Parallel()
	single := codexForm(t, nil, none[[2]uint16]())
	if single.kind != observationKindPermissionRequest || single.fingerprint == nil {
		t.Fatalf("the single-row form: %s", single.kind.debugString())
	}
	continued := codexForm(t, codexContinuedChoiceTwo, none[[2]uint16]())
	assertSameRequestAs(t, "a continued (p) row keeps the request identity", &continued, &single)
	for _, form := range citedForms(t, codingAgentCodex) {
		snapshot := replayNativeFixture(bundledScreen(form.id), nil, true, none[[2]uint16]())
		preStage := observeBundled(t, codingAgentCodex, codexFormVersion, &snapshot)
		if preStage.kind != observationKindPermissionRequest || !slices.Equal(preStage.rawEvidence.choices.values, single.rawEvidence.choices.values) {
			t.Fatalf("%s: %s choices %q", form.id, preStage.kind.debugString(), preStage.rawEvidence.choices.values)
		}
	}
}

type preStageForms struct {
	backend            codingAgent
	version            string
	fixtures           [2]string
	glyph              string
	focusRow           int
	secondChoice       [2]string
	cursorFollowsFocus bool
}

func TestCodingAgentDetector_BundledAnswerPlansMatchTheirMeasuredPreStageFormsAndBindFocus(t *testing.T) {
	t.Parallel()
	forms := []preStageForms{
		{codingAgentClaude, claudeFormVersion, [2]string{"claude-2.1.302-allow-pre-stage-1", "claude-2.1.302-deny-pre-stage-1"}, "❯", 23, [2]string{"   2.", " ❯ 2."}, true},
		{codingAgentClaude, claudeFormVersion, [2]string{"claude-2.1.302-wrapped-allow-pre-stage-1", "claude-2.1.302-wrapped-deny-pre-stage-1"}, "❯", 27, [2]string{"   2.", " ❯ 2."}, true},
		{codingAgentCodex, codexFormVersion, [2]string{"codex-0.163.0-allow-pre-stage-1", "codex-0.163.0-deny-pre-stage-1"}, "›", 30, [2]string{"  2.", "› 2."}, false},
	}
	for _, f := range forms {
		d := bundledDetector(f.backend, f.version)
		citations := 0
		for _, cited := range bundledCitedPlans(t, f.backend) {
			for _, id := range f.fixtures {
				if !slices.ContainsFunc(cited.fixtures, func(fixture bundledFixtureRecord) bool { return fixture.id == id }) {
					continue
				}
				citations++
				replay := func(change func(*[]string), cursor opt[[2]uint16]) screenSnapshot {
					return replayNativeFixture(bundledScreen(id), change, true, cursor)
				}
				screen := replay(nil, none[[2]uint16]())
				observed := observeOn(t, d, screen)
				matched := mustMatchAnswer(t, d, screen, &cited.plan, id)
				if !slices.Equal(matched.choices, observed.rawEvidence.choices.values) || matched.plan.choiceID != cited.plan.choiceID || matched.focus == "" ||
					!fingerprintsEqual(&matched.semantic, observed.fingerprint) {
					t.Fatalf("%s: the pre-stage form matches its %s plan with the observed request", id, cited.plan.choiceID)
				}
				cursor := none[[2]uint16]()
				if f.cursorFollowsFocus {
					cursor = some([2]uint16{uint16(f.focusRow + 1), 1}) //nolint:gosec // a screen row
				}
				moved := replay(func(lines *[]string) {
					(*lines)[f.focusRow] = strings.Replace((*lines)[f.focusRow], f.glyph, " ", 1)
					(*lines)[f.focusRow+1] = strings.Replace((*lines)[f.focusRow+1], f.secondChoice[0], f.secondChoice[1], 1)
				}, cursor)
				refocused := mustMatchAnswer(t, d, moved, &cited.plan, id+" with a moved focus")
				if !refocused.semantic.equal(&matched.semantic) || refocused.focus == matched.focus {
					t.Fatalf("%s: a moved focus keeps the form and changes the bound focus", id)
				}
				unfocused := replay(func(lines *[]string) {
					(*lines)[f.focusRow] = strings.Replace((*lines)[f.focusRow], f.glyph, " ", 1)
				}, none[[2]uint16]())
				assertNoAnswerMatch(t, d, unfocused, &cited.plan, id+" without its focus glyph")
			}
		}
		if citations < len(f.fixtures) {
			t.Fatalf("%q: an answer plan cites each pre-stage form", f.fixtures)
		}
	}
}

func assertVerifiedEmpty(t testing.TB, label string, observed *screenObservation) {
	t.Helper()
	if observed.kind != observationKindComposerEmpty || !observed.verifiedNoninteraction {
		t.Fatalf("%s: %s verified=%v, want a verified empty composer", label, observed.kind.debugString(), observed.verifiedNoninteraction)
	}
}

func TestCodingAgentDetector_BundledIdleRulesRejectDraftsPrimaryCopiesAndACursorOffThePrefixEnd(t *testing.T) {
	t.Parallel()
	cases := []struct {
		backend   codingAgent
		version   string
		idle      string
		promptRow uint16
		draft     string
	}{
		{codingAgentClaude, claudeFormVersion, "claude-2.1.302-idle-composer-1", 31, "❯\U000000a0hello"},
		{codingAgentCodex, "0.167.0", "codex-0.167.0-composer-idle-empty-tmux-120-1", 35, "› hello"},
	}
	for _, c := range cases {
		d := bundledDetector(c.backend, c.version)
		replay := func(change func(*[]string), cursor opt[[2]uint16]) screenObservation {
			return observeOn(t, d, replayNativeFixture(bundledScreen(c.idle), change, true, cursor))
		}
		idle := replay(nil, none[[2]uint16]())
		assertVerifiedEmpty(t, c.idle, &idle)
		draft := replay(setRow(int(c.promptRow), c.draft), some([2]uint16{c.promptRow, 7}))
		if draft.kind == observationKindComposerEmpty || draft.verifiedNoninteraction {
			t.Fatalf("%s: a typed draft is %s verified=%v", c.idle, draft.kind.debugString(), draft.verifiedNoninteraction)
		}
		away := replay(nil, some([2]uint16{c.promptRow, 0}))
		if away.kind == observationKindComposerEmpty || away.verifiedNoninteraction {
			t.Fatalf("%s: a cursor before the prefix end is %s verified=%v", c.idle, away.kind.debugString(), away.verifiedNoninteraction)
		}
	}
}

func TestCodingAgentDetector_CursorFree_BundledIdleComposersStayEmptyAndDraftsStayNotEmptyWithoutACursor(t *testing.T) {
	t.Parallel()
	cases := []struct {
		backend codingAgent
		fixture string
		draft   string
	}{
		{codingAgentClaude, "claude-2.1.302-idle-composer-1", "\x1b[32;3H\x1b[0mdraft text"},
		{codingAgentCodex, "codex-0.167.0-composer-idle-empty-tmux-120-1", "\x1b[36;3H\x1b[0m\x1b[Kdraft text"},
		{codingAgentOpencode, "opencode-2.1.4-idle-composer-1", "\x1b[12;3H\x1b[0mdraft text"},
		{codingAgentPi, "pi-1.5.0-idle-composer-tmux-120-1", "\x1b[32;1H\x1b[0mdraft text"},
	}
	for _, c := range cases {
		d := newDetector(profileCatalogBundled(), c.backend, none[string]())
		cursorFree := func(extra string) screenObservation {
			terminal := mustTerminalState(t, fixtureSize(c.fixture))
			mustFeed(t, terminal, append(nativeFixtureBytes(bundledScreen(c.fixture)), extra...))
			return observeOn(t, d, cursorFreeScreen(terminal.snapshot()))
		}
		captured := observeFixture(t, bundledFixtureByID(c.fixture), nil)
		assertVerifiedEmpty(t, c.fixture+" as captured", &captured)
		if idle := cursorFree(""); idle.kind != observationKindComposerEmpty {
			t.Fatalf("%s: the cursor-free idle composer is %s", c.fixture, idle.kind.debugString())
		}
		if drafted := cursorFree(c.draft); drafted.kind == observationKindComposerEmpty {
			t.Fatalf("%s: a drafted composer is not empty", c.fixture)
		}
	}
}

// claudeAutoTipForm is the Claude auto-tip permission form; padding written spaces follow the
// question and operation rows, as Claude leaves them when it repaints only the changed cells of
// a row that was earlier wider.
func claudeAutoTipForm(t testing.TB, padding int) screenSnapshot {
	t.Helper()
	pad := strings.Repeat(" ", padding)
	separator := strings.Repeat("╌", 120)
	rows := []string{
		" Bash command",
		` Tip: auto mode handles these prompts for you — choose "switch to auto mode" below`,
		" Create empty file wp-mark",
		separator,
		" touch wp-mark" + pad,
		separator,
		" Do you want to proceed?" + pad,
		" ❯ 1. Yes",
		"   2. Yes, and always allow access to /tmp/x from this project",
		"   3. Yes, and switch to auto mode · auto mode handles these prompts for you",
		"   4. No",
		"",
		" Esc to cancel · Tab to amend",
	}
	terminal := mustTerminalState(t, terminalSize{rows: 35, cols: 149})
	mustFeed(t, terminal, []byte("\x1b[?1049h\x1b[2J\x1b[13;1H"+strings.Join(rows, "\r\n")+"\x1b[20;2H"))
	return terminal.snapshot()
}

func TestCodingAgentDetector_BundledClaudeAutoTipFormWithWrittenPaddingRowsKeepsItsRequestIdentity(t *testing.T) {
	t.Parallel()
	d := bundledDetector(codingAgentClaude, "2.1.308")
	clean := observeOn(t, d, claudeAutoTipForm(t, 0))
	if operation, _ := clean.rawEvidence.operation.get(); clean.kind != observationKindPermissionRequest || operation != "touch wp-mark" {
		t.Fatalf("the clean form: %s %q", clean.kind.debugString(), operation)
	}
	padded := observeOn(t, d, claudeAutoTipForm(t, 40))
	assertSameRequestAs(t, "written padding", &padded, &clean)
}

type suggestionFixture struct {
	fixture         bundledFixtureRecord
	suggestionRules []string
	strictIdle      bool
}

func linePatterns(r any) []string {
	var patterns []string
	for _, m := range jarray(jget(r, "matchers")).items {
		if jget(m, "scope") == "line" {
			pattern, ok := jstring(jget(m, "pattern"))
			if !ok {
				panic("a line matcher pattern")
			}
			patterns = append(patterns, pattern)
		}
	}
	return patterns
}

// hasStrictIdleRule reports an empty-composer rule without a suggestion style whose line matcher
// is a suggestion rule's prompt marker followed by only blanks.
func hasStrictIdleRule(profile any) bool {
	rules := jarray(jget(profile, "rules")).items
	var markers []string
	for _, r := range rules {
		if _, styled := jobject(r).get("suggestion"); styled {
			markers = append(markers, linePatterns(r)...)
		}
	}
	return slices.ContainsFunc(rules, func(r any) bool {
		_, styled := jobject(r).get("suggestion")
		return jget(r, "observation") == "composer_empty" && !styled &&
			slices.ContainsFunc(linePatterns(r), func(pattern string) bool {
				return slices.ContainsFunc(markers, func(marker string) bool { return pattern == marker+" *$" })
			})
	})
}

// suggestionFixtures is every bundled -suggestion- screen with its profile file's suggestion rule
// ids and whether that profile's idle rule is strict.
func suggestionFixtures(t testing.TB) []suggestionFixture {
	t.Helper()
	var fixtures []suggestionFixture
	for _, fixture := range bundledFixtureRecords() {
		if !strings.Contains(fixture.id, "-suggestion-") {
			continue
		}
		profile := jget(parsedBundledFiles(), fixture.backend.name(), "profile")
		var rules []string
		for _, r := range jarray(jget(profile, "rules")).items {
			if _, styled := jobject(r).get("suggestion"); styled {
				id, _ := jstring(jget(r, "id"))
				rules = append(rules, id)
			}
		}
		fixtures = append(fixtures, suggestionFixture{fixture: fixture, suggestionRules: rules, strictIdle: hasStrictIdleRule(profile)})
	}
	if len(fixtures) == 0 {
		t.Fatal("the bundled catalog carries suggestion fixtures")
	}
	return fixtures
}

func strictSuggestionFixtures(t testing.TB) []suggestionFixture {
	t.Helper()
	var strict []suggestionFixture
	for _, fixture := range suggestionFixtures(t) {
		if fixture.strictIdle {
			strict = append(strict, fixture)
		}
	}
	if len(strict) == 0 {
		t.Fatal("some suggestion fixture belongs to a backend whose profile has a strict idle rule")
	}
	return strict
}

func TestCodingAgentDetector_BundledSuggestions_AStrictIdleRuleIsTheSuggestionMarkerFollowedByOnlyBlanks(t *testing.T) {
	t.Parallel()
	profile := func(idle string) any {
		return jsonf(`{"rules":[
			{"id":"idle","observation":"composer_empty","matchers":[{"id":"prompt","scope":"line","pattern":%s}]},
			{"id":"working","observation":"composer_empty","matchers":[{"id":"prompt","scope":"line","pattern":"^> working *$"}]},
			{"id":"suggestion","observation":"composer_empty","suggestion":{"style":{"faint":true}},
				"matchers":[{"id":"prompt","scope":"line","pattern":"^> "}]}
		]}`, idle)
	}
	if !hasStrictIdleRule(profile("^>  *$")) {
		t.Fatal("the marker followed by only blanks is strict")
	}
	if hasStrictIdleRule(profile("^> ")) {
		t.Fatal("the bare marker is not strict")
	}
}

func TestCodingAgentDetector_BundledSuggestions_EveryBundledSuggestionFixtureIsAnEmptyComposer(t *testing.T) {
	t.Parallel()
	for _, s := range suggestionFixtures(t) {
		if observed := observeFixture(t, s.fixture, nil); observed.kind != observationKindComposerEmpty || !observed.verifiedNoninteraction {
			t.Fatalf("%s: %s verified=%v", s.fixture.id, observed.kind.debugString(), observed.verifiedNoninteraction)
		}
	}
}

func TestCodingAgentDetector_BundledSuggestions_UnderAStrictIdleRuleASuggestionRuleDecidesEachSuggestionFixture(t *testing.T) {
	t.Parallel()
	for _, s := range strictSuggestionFixtures(t) {
		if observed := observeFixture(t, s.fixture, nil); !slices.Contains(s.suggestionRules, observed.ruleID) {
			t.Fatalf("%s is decided by %s, not a suggestion rule", s.fixture.id, observed.ruleID)
		}
	}
}

// typedAfterTheMarker is the fixture's screen with plain typed text after the prompt marker, the
// cursor left on the marker's end as after Home.
func typedAfterTheMarker(id, typed string) screenSnapshot {
	screen := fixtureDocument(id)
	jobject(screen).remove("cell_styles")
	at, ok := fixtureCursor(screen).get()
	if !ok {
		panic(id + ": a suggestion fixture records its cursor")
	}
	lines := jarray(jget(screen, "lines"))
	text, ok := jstring(lines.items[at[0]])
	if !ok {
		panic("a fixture line is a string")
	}
	marker := []rune(text)[:at[1]]
	lines.items[at[0]] = string(marker) + typed
	return replayNativeFixture(testJSONText(screen), nil, fixtureAlternate(screen), none[[2]uint16]())
}

func TestCodingAgentDetector_BundledSuggestions_UnderAStrictIdleRuleATypedDraftWithTheCursorAtTheMarkerIsNotEmpty(t *testing.T) {
	t.Parallel()
	for _, s := range strictSuggestionFixtures(t) {
		bare := typedAfterTheMarker(s.fixture.id, "")
		if observed := observeBundled(t, s.fixture.backend, s.fixture.version, &bare); observed.kind != observationKindComposerEmpty {
			t.Fatalf("%s: the bare marker is an empty composer, got %s", s.fixture.id, observed.kind.debugString())
		}
		typed := typedAfterTheMarker(s.fixture.id, "fix the failing build")
		if observed := observeBundled(t, s.fixture.backend, s.fixture.version, &typed); observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
			t.Fatalf("%s is decided by %s", s.fixture.id, observed.ruleID)
		}
	}
}

const (
	hermesFocusMarker = "│ ❯ "
	hermesHintRow     = "↑/↓ to select"
	hermesStatusRow   = "💻 bash -c"
	hermesBoxBottom   = "╰"
)

// lastRowContaining is the last row of lines containing text.
func lastRowContaining(lines []string, text string) int {
	for row := len(lines) - 1; row >= 0; row-- {
		if strings.Contains(lines[row], text) {
			return row
		}
	}
	panic(fmt.Sprintf("a row contains %q", text))
}

// citedRequestForms is each fixture a bundled answer plan of backend cites, with the request
// rule of the plans citing it.
func citedRequestForms(t testing.TB, backend codingAgent) map[bundledFixtureRecord]string {
	t.Helper()
	forms := map[bundledFixtureRecord]string{}
	for _, cited := range bundledCitedPlans(t, backend) {
		for _, fixture := range cited.fixtures {
			if request, seen := forms[fixture]; seen && request != cited.plan.capability.requestRuleID {
				t.Fatalf("%s: plans of two request rules cite it", fixture.id)
			}
			forms[fixture] = cited.plan.capability.requestRuleID
		}
	}
	return forms
}

func sortedForms(forms map[bundledFixtureRecord]string) []bundledFixtureRecord {
	keys := make([]bundledFixtureRecord, 0, len(forms))
	for fixture := range forms {
		keys = append(keys, fixture)
	}
	slices.SortFunc(keys, func(a, b bundledFixtureRecord) int { return strings.Compare(a.id, b.id) })
	return keys
}

func assertSameRequestAs(t testing.TB, label string, observed, captured *screenObservation) {
	t.Helper()
	if observed.kind != observationKindPermissionRequest || observed.matchEvidence != matchEvidenceComplete || observed.ruleID != captured.ruleID ||
		observed.rawEvidence.operation != captured.rawEvidence.operation || observed.rawEvidence.operationRows != captured.rawEvidence.operationRows ||
		!slices.Equal(observed.rawEvidence.choices.values, captured.rawEvidence.choices.values) || observed.verifiedNoninteraction ||
		observed.fingerprint == nil || !fingerprintsEqual(observed.fingerprint, captured.fingerprint) {
		t.Fatalf("%s: %s %s %v, captured %s %s %v", label, observed.kind.debugString(), observed.ruleID, observed.rawEvidence.operation,
			captured.kind.debugString(), captured.ruleID, captured.rawEvidence.operation)
	}
}

func TestCodingAgentDetector_HermesProfile_HermesFocusFollowsTheMarkerRowAndAFormWithoutItMatchesNoPlan(t *testing.T) {
	t.Parallel()
	plans := bundledCitedPlans(t, codingAgentHermes)
	forms := citedRequestForms(t, codingAgentHermes)
	for _, form := range sortedForms(forms) {
		observed := observeFixture(t, form, nil)
		captures := map[string]bool{}
		unfocus := func(lines *[]string) {
			marked := lastRowContaining(*lines, hermesFocusMarker)
			(*lines)[marked] = strings.Replace((*lines)[marked], hermesFocusMarker, "│   ", 1)
		}
		for _, cited := range plans {
			if cited.plan.capability.requestRuleID != forms[form] {
				continue
			}
			matched, ok := matchFixture(t, form, nil, &cited.plan)
			if !ok {
				t.Fatalf("%s matches its %s plan", form.id, cited.plan.choiceID)
			}
			if !slices.Equal(matched.choices, observed.rawEvidence.choices.values) {
				t.Fatalf("%s: matched choices %q, observed %q", form.id, matched.choices, observed.rawEvidence.choices.values)
			}
			captures[matched.focusCapture] = true
			if _, ok := matchFixture(t, form, unfocus, &cited.plan); ok {
				t.Fatalf("%s without its focus marker matches its %s plan", form.id, cited.plan.choiceID)
			}
		}
		if len(captures) != 1 {
			t.Fatalf("%s: every plan shares the one focus, got %d", form.id, len(captures))
		}
	}
}

func TestCodingAgentDetector_HermesProfile_HermesFormsPastTheirLayoutLimitOrClippedYieldNoRequest(t *testing.T) {
	t.Parallel()
	forms := citedRequestForms(t, codingAgentHermes)
	for _, form := range sortedForms(forms) {
		bottomless := observeFixture(t, form, func(lines *[]string) {
			(*lines)[lastRowContaining(*lines, hermesBoxBottom)] = ""
		})
		assertNoRequestFrom(t, form.id+" without its border", &bottomless, forms[form])
		captured := observeFixture(t, form, nil)
		operation, ok := captured.rawEvidence.operation.get()
		if !ok {
			t.Fatalf("%s: a captured operation", form.id)
		}
		first, _, _ := strings.Cut(operation, "\n")
		if len(first) <= 3 {
			continue
		}
		clipped := first[:len(first)-3] + "..."
		labels := captured.rawEvidence.choices.values
		truncated := observeFixture(t, form, func(lines *[]string) {
			command := lastRowContaining(*lines, first)
			(*lines)[command] = strings.Replace((*lines)[command], first, clipped, 1)
			last := lastRowContaining(*lines, labels[len(labels)-1])
			expand := strings.Replace((*lines)[last], labels[len(labels)-1], "Show full command", 1)
			grown := slices.Insert(slices.Clone(*lines), last+1, expand)
			*lines = grown[1:]
		})
		assertNoRequestFrom(t, form.id+": a truncated command", &truncated, forms[form])
	}
}

func TestCodingAgentDetector_HermesProfile_HermesIdleComposerIsAVerifiedEmptyComposerAndADraftIsNot(t *testing.T) {
	t.Parallel()
	drafts, composers := 0, 0
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend != codingAgentHermes {
			continue
		}
		name := fixture.id
		screen := fixtureDocument(name)
		at, hasCursor := fixtureCursor(screen).get()
		if !hasCursor || fixtureAlternate(screen) {
			continue
		}
		literals, err := preSubmitLiterals(screen)
		if err != nil {
			t.Fatal(err)
		}
		observed := observeFixture(t, fixture, nil)
		if len(literals) > 0 {
			drafts++
			if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
				t.Fatalf("%s: a retained draft is %s verified=%v", name, observed.kind.debugString(), observed.verifiedNoninteraction)
			}
			continue
		}
		if observed.kind != observationKindComposerEmpty || !observed.verifiedNoninteraction || at[1] == 0 {
			continue
		}
		composers++
		away := replayAsCaptured(name, nil, some([2]uint16{at[0], 0}))
		if awayObserved := observeBundled(t, codingAgentHermes, fixture.version, &away); awayObserved.kind == observationKindComposerEmpty || awayObserved.verifiedNoninteraction {
			t.Fatalf("%s: a cursor before the prefix end is %s verified=%v", name, awayObserved.kind.debugString(), awayObserved.verifiedNoninteraction)
		}
	}
	if drafts == 0 || composers == 0 {
		t.Fatalf("a captured hermes draft (%d) and verified empty composer (%d)", drafts, composers)
	}
}

// hermesRedraw repaints the live countdown, where the width shows it, and the status-row spinner
// frame.
func hermesRedraw(lines *[]string) {
	footer := lastRowContaining(*lines, hermesHintRow)
	if open := strings.LastIndex((*lines)[footer], "("); open >= 0 {
		(*lines)[footer] = (*lines)[footer][:open] + "(9s)"
	}
	status := lastRowContaining(*lines, hermesStatusRow)
	frames := []rune((*lines)[status])
	if len(frames) < 4 {
		panic("the status row starts with its spinner frame")
	}
	(*lines)[status] = strings.Replace((*lines)[status], string(frames[3]), "⠿", 1)
}

func TestCodingAgentDetector_HermesProfile_HermesCountdownAndSpinnerRedrawsKeepTheRequestIdentity(t *testing.T) {
	t.Parallel()
	for _, form := range sortedForms(citedRequestForms(t, codingAgentHermes)) {
		captured := observeFixture(t, form, nil)
		redrawn := observeFixture(t, form, hermesRedraw)
		assertSameRequestAs(t, form.id, &redrawn, &captured)
	}
}

func TestCodingAgentDetector_HermesProfile_HermesFormOnTheAlternateScreenIsNoRequestUntilDrawnOnThePrimaryScreen(t *testing.T) {
	t.Parallel()
	forms := citedRequestForms(t, codingAgentHermes)
	for _, form := range sortedForms(forms) {
		captured := observeFixture(t, form, nil)
		alternate := replayNativeFixture(bundledScreen(form.id), nil, true, none[[2]uint16]())
		onAlternate := observeBundled(t, codingAgentHermes, form.version, &alternate)
		assertNoRequestFrom(t, form.id+": the form on the alternate screen", &onAlternate, forms[form])
		bytes := nativeFixtureBytes(bundledScreen(form.id))
		state := mustTerminalState(t, fixtureSize(form.id))
		mustFeed(t, state, []byte("\x1b[?1049h"))
		mustFeed(t, state, bytes)
		mustFeed(t, state, []byte("\x1b[?1049l"))
		restored := state.snapshot()
		onRestored := observeBundled(t, codingAgentHermes, form.version, &restored)
		assertNoRequestFrom(t, form.id+": the restored primary screen", &onRestored, forms[form])
		mustFeed(t, state, bytes)
		drawn := state.snapshot()
		onPrimary := observeBundled(t, codingAgentHermes, form.version, &drawn)
		assertSameRequestAs(t, form.id+": the form drawn on the primary screen", &onPrimary, &captured)
	}
}

func TestCodingAgentDetector_HermesProfile_HermesFormTextOutsideALiveFormIsNoRequest(t *testing.T) {
	t.Parallel()
	forms := citedRequestForms(t, codingAgentHermes)
	for _, form := range sortedForms(forms) {
		request := forms[form]
		region, ok := observeFixture(t, form, nil).matchedRegion.get()
		if !ok {
			t.Fatalf("%s: the form's live region", form.id)
		}
		formRows := fixtureLines(form.id)
		boxed := formRows[region.top : lastRowContaining(formRows, hermesBoxBottom)+1]
		composer := verifiedEmptyComposerOf(t, form)
		idleRegion, _ := observeFixture(t, composer, nil).matchedRegion.get()
		if int(idleRegion.top) <= 1+len(boxed) {
			t.Fatalf("%s: the closed form fits above the idle composer of %s", form.id, composer.id)
		}
		scrollback := observeFixture(t, composer, func(lines *[]string) { copy((*lines)[1:], boxed) })
		assertNoRequestFrom(t, form.id+": a closed form above the idle composer", &scrollback, request)
		quoted := observeFixture(t, form, func(lines *[]string) {
			for row := region.top; row < region.bottom; row++ {
				if (*lines)[row] != "" {
					(*lines)[row] = "    " + (*lines)[row]
				}
			}
		})
		assertNoRequestFrom(t, form.id+": the form quoted in a cat of a log", &quoted, request)
		footerless := observeFixture(t, form, func(lines *[]string) {
			(*lines)[lastRowContaining(*lines, hermesHintRow)] = ""
		})
		assertNoRequestFrom(t, form.id+": the form without its live footer", &footerless, request)
		unboxed := observeFixture(t, form, func(lines *[]string) {
			for row := region.top; row < region.bottom; row++ {
				(*lines)[row] = strings.NewReplacer("│", " ", "╭", " ", "╮", " ", "╰", " ", "╯", " ", "─", " ").Replace((*lines)[row])
			}
		})
		assertNoRequestFrom(t, form.id+": the form text in model output", &unboxed, request)
	}
}

const (
	opencodeLayoutVersion   = "2.1.4"
	opencodeChoicesRow      = "   Allow once   Always allow   Reject"
	opencodeControlsRow     = "⇆ select  enter confirm  esc reject"
	opencodeProbeDirectory  = "/tmp/cafleet-194/opencode-narrow-cells-057/"
	opencodeCompactChoices  = " Allow once   Always allow   Reject"
	opencodeCompactControls = "pgup/pgdn scroll"
)

type opencodeLayout uint8

const (
	opencodeLayoutWide opencodeLayout = iota + 1
	opencodeLayoutSplit
	opencodeLayoutBroken
	opencodeLayoutControlsOnly
	opencodeLayoutInterposedRow
)

type opencodeCompactLayout uint8

const (
	opencodeCompactShared opencodeCompactLayout = iota + 1
	opencodeCompactOwn
	opencodeCompactBroken
	opencodeCompactNoControls
)

// opencodeContinuationMargin is the row margin the bundled OpenCode request rule declares for
// its operation's continuation rows.
func opencodeContinuationMargin() string {
	for _, r := range jarray(jget(parsedBundledFiles(), "opencode", "profile", "rules")).items {
		if margin, ok := jstring(jget(r, "extract", "operation", "row_margin")); ok {
			return margin
		}
	}
	panic("a bundled OpenCode rule declares its continuation margin")
}

func padTo(text string, width int) string {
	count := utf8.RuneCountInString(text)
	if count > width {
		panic(fmt.Sprintf("%q fits %d columns", text, width))
	}
	return text + strings.Repeat(" ", width-count)
}

// opencodeBoxed is a row of the command box: text, then the scrollbar column four cells before
// the right edge, as the 120-column capture shows it.
func opencodeBoxed(cols int, text string) string {
	if utf8.RuneCountInString(text) >= cols-4 {
		panic(fmt.Sprintf("%q fits the box", text))
	}
	return padTo(text, cols-4) + "█   "
}

func opencodeFormRows(cols int, command []string, layout opencodeLayout) []string {
	margin := opencodeContinuationMargin()
	rows := []string{
		"  △ Permission required",
		"",
		opencodeBoxed(cols, "   # Shell command"),
		opencodeBoxed(cols, ""),
		opencodeBoxed(cols, "  $ "+command[0]),
	}
	for _, row := range command[1:] {
		rows = append(rows, opencodeBoxed(cols, margin+row))
	}
	rows = append(rows, opencodeBoxed(cols, ""), opencodeBoxed(cols, ""), "", "")
	controls := "  " + opencodeControlsRow
	switch layout {
	case opencodeLayoutWide:
		padding := cols - utf8.RuneCountInString(opencodeChoicesRow) - utf8.RuneCountInString(opencodeControlsRow) - 3
		rows = append(rows, opencodeChoicesRow+strings.Repeat(" ", padding)+opencodeControlsRow+"   ")
	case opencodeLayoutSplit:
		rows = append(rows, opencodeChoicesRow, "", controls)
	case opencodeLayoutBroken:
		rows = append(rows, "   Allow once   Always allow", "   Reject", controls)
	case opencodeLayoutControlsOnly:
		rows = append(rows, "", controls)
	case opencodeLayoutInterposedRow:
		rows = append(rows, opencodeChoicesRow, "   Press a key", controls)
	default:
		panic("invalid opencodeLayout")
	}
	return rows
}

// opencodePaint paints rows from screen row 11 of a 36-row screen.
func opencodePaint(t testing.TB, cols int, rows []string) screenSnapshot {
	t.Helper()
	if cols <= 0 || cols > math.MaxUint16 {
		t.Fatalf("%d columns", cols)
	}
	terminal := mustTerminalState(t, terminalSize{rows: 36, cols: uint16(cols)}) //nolint:gosec // bounded above
	mustFeed(t, terminal, []byte("\x1b[2J"))
	for index, row := range rows {
		if row != "" {
			mustFeed(t, terminal, fmt.Appendf(nil, "\x1b[%d;1H%s", index+11, row))
		}
	}
	return terminal.snapshot()
}

func opencodeForm(t testing.TB, cols int, command []string, layout opencodeLayout) screenSnapshot {
	t.Helper()
	return opencodePaint(t, cols, opencodeFormRows(cols, command, layout))
}

func observeOpencode(t testing.TB, screen screenSnapshot) screenObservation {
	t.Helper()
	return observeBundled(t, codingAgentOpencode, opencodeLayoutVersion, &screen)
}

// assertOpencodeRequest asserts a complete permission request with operation whose choices are
// labels the drawn choice row shows, and returns its rule.
func assertOpencodeRequest(t testing.TB, label string, observed *screenObservation, operation, choiceRow string) string {
	t.Helper()
	if observed.kind != observationKindPermissionRequest || observed.matchEvidence != matchEvidenceComplete || observed.fingerprint == nil {
		t.Fatalf("%s: %s %s", label, observed.kind.debugString(), observed.matchEvidence.debugString())
	}
	if got, _ := observed.rawEvidence.operation.get(); got != operation {
		t.Fatalf("%s: operation %q, want %q", label, got, operation)
	}
	if rows, _ := observed.rawEvidence.operationRows.get(); rows != uint64(len(strings.Split(operation, "\n"))) {
		t.Fatalf("%s: operation rows %d", label, rows)
	}
	choices := observed.rawEvidence.choices.values
	if len(choices) == 0 || slices.ContainsFunc(choices, func(choice string) bool { return !strings.Contains(choiceRow, choice) }) {
		t.Fatalf("%s: choices %q are labels of %q", label, choices, choiceRow)
	}
	return observed.ruleID
}

func assertNoOpencodeRequest(t testing.TB, label string, observed *screenObservation) {
	t.Helper()
	if observed.kind == observationKindPermissionRequest || observed.fingerprint != nil {
		t.Fatalf("%s: %s %s", label, observed.kind.debugString(), observed.ruleID)
	}
}

func opencodeAllowMatch(t testing.TB, screen screenSnapshot, rule string) answerMatch {
	t.Helper()
	bundled, ok := profileCatalogBundled().profile(codingAgentOpencode)
	if !ok {
		t.Fatal("a bundled OpenCode profile")
	}
	plan, ok := bundled.answerPlan(rule, answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		t.Fatalf("%s: an allow plan", rule)
	}
	matched, ok, err := bundledDetector(codingAgentOpencode, opencodeLayoutVersion).matchAnswer(&screen, &plan)
	if err != nil || !ok {
		t.Fatalf("the form matches the allow plan: %v", err)
	}
	return matched
}

func opencodeWrapped() []string { return []string{opencodeProbeDirectory, "permission-probe allow"} }

func TestCodingAgentDetector_OpencodeLayouts_OpencodeSplitLayoutReportsEveryDisplayedCommandRow(t *testing.T) {
	t.Parallel()
	split := observeOpencode(t, opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutSplit))
	rule := assertOpencodeRequest(t, "wrapped", &split, opencodeProbeDirectory+"\npermission-probe allow", opencodeChoicesRow)
	single := observeOpencode(t, opencodeForm(t, 57, []string{"/tmp/x/permission-probe allow"}, opencodeLayoutSplit))
	if got := assertOpencodeRequest(t, "single", &single, "/tmp/x/permission-probe allow", opencodeChoicesRow); got != rule {
		t.Fatalf("single-row split form raises %s, the wrapped one %s", got, rule)
	}
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeWideLayoutKeepsItsSingleRowRequest(t *testing.T) {
	t.Parallel()
	command := "/tmp/cafleet-194/opencode-cells-001/permission-probe allow"
	wide := observeOpencode(t, opencodeForm(t, 120, []string{command}, opencodeLayoutWide))
	split := observeOpencode(t, opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutSplit))
	if rule := assertOpencodeRequest(t, "wide", &wide, command, opencodeChoicesRow); rule != split.ruleID {
		t.Fatalf("the wide form raises %s, the split form %s", rule, split.ruleID)
	}
}

// opencodeClipped replaces the scrollbar thumb on the given rows, as a command box that scrolls
// shows a shorter thumb than its track.
func opencodeClipped(rows []string, thumbRow int, trackRows []int) []string {
	rows = slices.Clone(rows)
	rows[thumbRow] = strings.Replace(rows[thumbRow], "█", "▀", 1)
	for _, row := range trackRows {
		rows[row] = strings.Replace(rows[row], "█", " ", 1)
	}
	return rows
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeClippedCommandsYieldNoRequestForEitherRequestRule(t *testing.T) {
	t.Parallel()
	split := opencodeFormRows(57, opencodeWrapped(), opencodeLayoutSplit)
	compact := opencodeCompactRows(36, []string{"/tmp/cafleet-194/opencode-survey-", "036/permission-probe allow"}, opencodeCompactOwn)
	cases := []struct {
		name string
		cols int
		rows []string
	}{
		{"split, thumb above every command row", 57, opencodeClipped(split, 2, []int{4, 5})},
		{"split, a later command row off the thumb", 57, opencodeClipped(split, 5, nil)},
		{"compact, thumb above every command row", 36, opencodeClipped(compact, 1, []int{2, 3})},
		{"compact, a later command row off the thumb", 36, opencodeClipped(compact, 3, nil)},
	}
	for _, c := range cases {
		observed := observeOpencode(t, opencodePaint(t, c.cols, c.rows))
		assertNoOpencodeRequest(t, c.name, &observed)
	}
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeFormsOutsideTheSupportedLayoutsYieldNoRequest(t *testing.T) {
	t.Parallel()
	headless := opencodeFormRows(57, opencodeWrapped(), opencodeLayoutSplit)
	headless[0] = ""
	stray := slices.Insert(opencodeFormRows(57, opencodeWrapped(), opencodeLayoutSplit), 6, opencodeBoxed(57, "stray text"))
	cases := []struct {
		name   string
		screen screenSnapshot
	}{
		{"broken layout", opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutBroken)},
		{"controls without a choice row", opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutControlsOnly)},
		{"a row between the choices and the controls", opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutInterposedRow)},
		{"a continuation row indented past the margin", opencodeForm(t, 57, []string{opencodeProbeDirectory, " permission-probe allow"}, opencodeLayoutSplit)},
		{"a row without the margin before the box end", opencodePaint(t, 57, stray)},
		{"form text outside a live region", opencodePaint(t, 57, headless)},
	}
	for _, c := range cases {
		observed := observeOpencode(t, c.screen)
		assertNoOpencodeRequest(t, c.name, &observed)
	}
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeLayoutsDifferInFocusCaptureAndRewrapsDifferInIdentity(t *testing.T) {
	t.Parallel()
	wide := opencodeForm(t, 120, []string{opencodeProbeDirectory + "permission-probe allow"}, opencodeLayoutWide)
	split := opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutSplit)
	rule := observeOpencode(t, split).ruleID
	if opencodeAllowMatch(t, wide, rule).focus == opencodeAllowMatch(t, split, rule).focus {
		t.Fatal("the focus capture is part of the request identity")
	}
	wideObserved, splitObserved := observeOpencode(t, wide), observeOpencode(t, split)
	if fingerprintsEqual(wideObserved.fingerprint, splitObserved.fingerprint) {
		t.Fatal("the wide and split layouts are different requests")
	}
	rewrapped := observeOpencode(t, opencodeForm(t, 57, []string{"/tmp/cafleet-194/opencode-narrow-", "cells-057/permission-probe allow"}, opencodeLayoutSplit))
	if fingerprintsEqual(splitObserved.fingerprint, rewrapped.fingerprint) {
		t.Fatal("a moved row break is a different request")
	}
	again := opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutSplit)
	againObserved := observeOpencode(t, again)
	if !fingerprintsEqual(splitObserved.fingerprint, againObserved.fingerprint) {
		t.Fatal("an unchanged form keeps its identity")
	}
	if opencodeAllowMatch(t, split, rule).focus != opencodeAllowMatch(t, again, rule).focus {
		t.Fatal("an unchanged form keeps its focus")
	}
}

func opencodeDialog(t testing.TB, placeholder opt[string]) screenSnapshot {
	t.Helper()
	rows := []string{"  △ Reject permission", "", "  Tell OpenCode what to do differently"}
	rows = append(rows, make([]string, 7)...)
	if text, ok := placeholder.get(); ok {
		rows = append(rows, "  "+text)
	}
	rows = append(rows, "  enter confirm  esc cancel")
	return opencodePaint(t, 57, rows)
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeSplitRejectDialogRequiresTheEmptyPlaceholder(t *testing.T) {
	t.Parallel()
	empty := observeOpencode(t, opencodeDialog(t, some("Tell OpenCode what to do differently")))
	if empty.kind != observationKindPermissionContinuation || empty.matchEvidence != matchEvidenceComplete {
		t.Fatalf("the empty reason dialog: %s %s", empty.kind.debugString(), empty.matchEvidence.debugString())
	}
	cases := []struct {
		name   string
		screen screenSnapshot
	}{
		{"a prompt-only dialog", opencodeDialog(t, none[string]())},
		{"a typed reason", opencodeDialog(t, some("run it in the sandbox instead"))},
		{"a typed copy of the placeholder with more text", opencodeDialog(t, some("Tell OpenCode what to do differently!"))},
	}
	for _, c := range cases {
		if observed := observeOpencode(t, c.screen); observed.kind == observationKindPermissionContinuation || observed.ruleID == empty.ruleID {
			t.Fatalf("%s: %s %s", c.name, observed.kind.debugString(), observed.ruleID)
		}
	}
}

// opencodeCompactBoxed is a compact command-box row: text at column 0 and the scrollbar column
// at the right edge, as the 36-, 52- and 55-column captures show it.
func opencodeCompactBoxed(cols int, text string) string {
	return padTo(text, cols-1) + "█"
}

func opencodeCompactRows(cols int, command []string, layout opencodeCompactLayout) []string {
	rows := []string{
		"△ Permission required",
		opencodeCompactBoxed(cols, "# Shell command"),
		opencodeCompactBoxed(cols, "$ "+command[0]),
	}
	for _, row := range command[1:] {
		rows = append(rows, opencodeCompactBoxed(cols, row))
	}
	for range 3 {
		rows = append(rows, opencodeCompactBoxed(cols, ""))
	}
	switch layout {
	case opencodeCompactShared:
		padding := cols - utf8.RuneCountInString(opencodeCompactChoices) - len(opencodeCompactControls)
		rows = append(rows, opencodeCompactChoices+strings.Repeat(" ", padding)+opencodeCompactControls)
	case opencodeCompactOwn:
		rows = append(rows, opencodeCompactChoices, opencodeCompactControls)
	case opencodeCompactBroken:
		rows = append(rows, " Allow once   Always allow", " Reject", opencodeCompactControls)
	case opencodeCompactNoControls:
		rows = append(rows, opencodeCompactChoices)
	default:
		panic("invalid opencodeCompactLayout")
	}
	return rows
}

func opencodeCompact(t testing.TB, cols int, command []string, layout opencodeCompactLayout) screenSnapshot {
	t.Helper()
	return opencodePaint(t, cols, opencodeCompactRows(cols, command, layout))
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeCompactLayoutReportsEveryDisplayedCommandRow(t *testing.T) {
	t.Parallel()
	split := observeOpencode(t, opencodeForm(t, 57, opencodeWrapped(), opencodeLayoutSplit))
	cases := []struct {
		cols    int
		command []string
		layout  opencodeCompactLayout
	}{
		{55, []string{"/tmp/cafleet-194/opencode-survey-055/permission-", "probe allow"}, opencodeCompactShared},
		{52, []string{"/tmp/cafleet-194/opencode-survey-052/permission-", "probe allow"}, opencodeCompactOwn},
		{36, []string{"/tmp/cafleet-194/opencode-survey-", "036/permission-probe allow"}, opencodeCompactOwn},
		{55, []string{"/tmp/x/permission-probe allow"}, opencodeCompactShared},
	}
	var rules []string
	for _, c := range cases {
		label := fmt.Sprintf("%d columns %q", c.cols, c.command)
		observed := observeOpencode(t, opencodeCompact(t, c.cols, c.command, c.layout))
		rules = append(rules, assertOpencodeRequest(t, label, &observed, strings.Join(c.command, "\n"), opencodeCompactChoices))
	}
	if slices.ContainsFunc(rules, func(rule string) bool { return rule != rules[0] }) || rules[0] == split.ruleID {
		t.Fatalf("every compact form raises one rule other than the split form's %s, got %q", split.ruleID, rules)
	}
}

func TestCodingAgentDetector_OpencodeLayouts_OpencodeCompactFormsOutsideTheLayoutYieldNoRequest(t *testing.T) {
	t.Parallel()
	command := []string{"/tmp/cafleet-194/opencode-survey-", "036/permission-probe allow"}
	stray := slices.Insert(opencodeCompactRows(36, command, opencodeCompactOwn), 5, opencodeCompactBoxed(36, "stray text"))
	headless := opencodeCompactRows(36, command, opencodeCompactOwn)
	headless[0] = ""
	margined := opencodeCompactRows(36, command, opencodeCompactOwn)
	margined[0] = "  △ Permission required"
	cases := []struct {
		name   string
		screen screenSnapshot
	}{
		{"broken layout", opencodeCompact(t, 35, []string{"/tmp/x/permission-probe allow"}, opencodeCompactBroken)},
		{"no control text", opencodeCompact(t, 36, command, opencodeCompactNoControls)},
		{"a continuation row beginning with a space", opencodeCompact(t, 36, []string{"/tmp/cafleet-194/opencode-survey-", " 036/permission-probe"}, opencodeCompactOwn)},
		{"a row after the box end", opencodePaint(t, 36, stray)},
		{"form text outside a live region", opencodePaint(t, 36, headless)},
		{"a wide/split heading margin", opencodePaint(t, 36, margined)},
	}
	for _, c := range cases {
		observed := observeOpencode(t, c.screen)
		assertNoOpencodeRequest(t, c.name, &observed)
	}
}

func withoutScreenFields(screen screenSnapshot, cursor, alternate bool) screenSnapshot {
	if !cursor {
		screen.cursor = none[[2]uint16]()
	}
	if !alternate {
		screen.alternateScreen = none[bool]()
	}
	return screen
}

func emptyComposerDefinition(cursor, screen string) any {
	value := simpleDefinition("composer_empty", true)
	jset(value, cursor, "codex", "profile", "rules", 0, "region", "cursor")
	jset(value, screen, "codex", "profile", "rules", 0, "region", "screen")
	return value
}

func observeOn(t testing.TB, d *detector, screen screenSnapshot) screenObservation {
	t.Helper()
	observed, err := d.observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	return observed
}

func TestCodingAgentDetector_CursorFree_ACaptureWithoutACursorTreatsEveryCursorConditionAsAny(t *testing.T) {
	t.Parallel()
	for _, cursor := range []string{"at_end", "after_prompt", "inside"} {
		d := codexDetector(t, emptyComposerDefinition(cursor, "either"))
		screen := screenOf(t, []byte("\x1b[2J\x1b[10;1HSynthetic state\r\nState end\x1b[1;1H"))
		if observed := observeOn(t, d, screen); observed.ruleID == syntheticEmptyRule {
			t.Fatalf("%s: a cursor outside the condition keeps the region unqualified", cursor)
		}
		for _, tc := range []struct {
			name   string
			screen screenSnapshot
		}{{"no cursor or screen mode", cursorFreeScreen(screen)}, {"no cursor", withoutScreenFields(screen, false, true)}} {
			if observed := observeOn(t, d, tc.screen); observed.kind != observationKindComposerEmpty || observed.ruleID != syntheticEmptyRule {
				t.Fatalf("%s: %s: %s %s", cursor, tc.name, observed.kind.debugString(), observed.ruleID)
			}
		}
	}
}

func TestCodingAgentDetector_CursorFree_ACaptureWithoutACursorOrScreenModeIgnoresTheScreenSelector(t *testing.T) {
	t.Parallel()
	primary := screenOf(t, []byte("\x1b[2J\x1b[10;1HSynthetic state\r\nState end"))
	alternate := screenOf(t, []byte("\x1b[?1049h\x1b[2J\x1b[10;1HSynthetic state\r\nState end"))
	for _, tc := range []struct {
		selector string
		screen   screenSnapshot
	}{{"primary", alternate}, {"alternate", primary}} {
		d := codexDetector(t, emptyComposerDefinition("any", tc.selector))
		if observed := observeOn(t, d, tc.screen); observed.ruleID == syntheticEmptyRule {
			t.Fatalf("%s: a known screen mode keeps the selector", tc.selector)
		}
		for _, variant := range []struct {
			name   string
			screen screenSnapshot
		}{
			{"no cursor or screen mode", cursorFreeScreen(tc.screen)},
			{"no screen mode", withoutScreenFields(tc.screen, true, false)},
			{"no cursor", withoutScreenFields(tc.screen, false, true)},
		} {
			if observed := observeOn(t, d, variant.screen); observed.ruleID != syntheticEmptyRule {
				t.Fatalf("%s: %s: rule %s", tc.selector, variant.name, observed.ruleID)
			}
		}
	}
}

func promptRowDefinition(cursor string) any {
	value := emptyComposerDefinition(cursor, "either")
	r := jget(value, "codex", "profile", "rules", 0)
	jset(r, "^> ", "region", "start_pattern")
	jset(r, "^> ", "region", "end_pattern")
	jset(r, 1, "region", "max_rows")
	jset(r, parseTestJSON(`[{"id":"idle","scope":"line","pattern":"^> Idle placeholder$"}]`), "matchers")
	return value
}

func promptRows(t testing.TB, history string) screenSnapshot {
	t.Helper()
	return screenOf(t, []byte("\x1b[2J\x1b[4;1H> "+history+"\x1b[21;1H> Idle placeholder\x1b[1;1H"))
}

func TestCodingAgentDetector_CursorFree_WithoutACursorAComposerRuleTakesItsBottomMostCandidateSatisfyingItsMatchers(t *testing.T) {
	t.Parallel()
	d := codexDetector(t, promptRowDefinition("at_end"))
	for _, history := range []string{"history entry", "Idle placeholder"} {
		observed := observeOn(t, d, cursorFreeScreen(promptRows(t, history)))
		if observed.kind != observationKindComposerEmpty || observed.ruleID != syntheticEmptyRule {
			t.Fatalf("%s: %s %s", history, observed.kind.debugString(), observed.ruleID)
		}
		if region, ok := observed.matchedRegion.get(); !ok || region.top != 20 {
			t.Fatalf("%s: the bottom-most satisfying candidate is the region: %v", history, observed.matchedRegion)
		}
	}
}

func TestCodingAgentDetector_CursorFree_WithoutACursorAnInteractionRuleKeepsItsUniquenessCheck(t *testing.T) {
	t.Parallel()
	d := codexDetector(t, syntheticDefinitions())
	if one := observeOn(t, d, cursorFreeScreen(screenOf(t, inspectionFixtureForm("rm -rf build", "synthetic-only")))); one.ruleID != syntheticRule {
		t.Fatalf("one cursor-free form is detected: %s", one.ruleID)
	}
	form := "Synthetic permission request\r\nOperation: rm -rf build\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: "
	two := screenOf(t, []byte("\x1b[2J\x1b[3;1H"+form+"\x1b[15;1H"+form))
	if observed := observeOn(t, d, cursorFreeScreen(two)); observed.ruleID == syntheticRule {
		t.Fatal("two satisfying forms stay ambiguous")
	}
}

func TestCodingAgentDetector_CursorFree_WithoutACursorOnlyTheLowestEndingComposerMatchCompetes(t *testing.T) {
	t.Parallel()
	d := codexDetector(t, emptyComposerDefinition("at_end", "either"))
	screen := cursorFreeScreen(screenOf(t, []byte("\x1b[2J\x1b[5;1HSynthetic state\r\nState end\x1b[20;1HSynthetic input: draft\r\nState end")))
	observed := observeOn(t, d, screen)
	if observed.kind == observationKindComposerEmpty || observed.ruleID != syntheticTextRuleID {
		t.Fatalf("an empty-shaped box above a drafted composer: %s %s", observed.kind.debugString(), observed.ruleID)
	}
}

func directorDetector(t testing.TB, backend codingAgent) (*detector, []string) {
	t.Helper()
	delivery, ok := profileCatalogBundled().directorDelivery(backend)
	if !ok {
		t.Fatal("a Director backend declares Director delivery")
	}
	return newDetector(profileCatalogBundled(), backend, none[string]()), slices.Clone(delivery.ruleIDs)
}

// herdrDirectorScreens is the ids of backend's bundled cursor-free herdr screens: those with no
// composer record, and those holding a typed prompt literal.
func herdrDirectorScreens(t testing.TB, backend codingAgent) (idle, drafted []string) {
	t.Helper()
	for _, member := range jobject(jget(parsedBundledFiles(), backend.name(), "screens")).members {
		if jget(member.value, "multiplexer") != "herdr" {
			continue
		}
		records, err := preSubmitRecords(member.value)
		if err != nil {
			t.Fatal(err)
		}
		if _, present := jobject(member.value).get("composer_evidence"); !present {
			idle = append(idle, member.key)
		} else if slices.ContainsFunc(records, func(record preSubmitRecord) bool { return record.operation == composerOperationPrompt }) {
			drafted = append(drafted, member.key)
		}
	}
	return idle, drafted
}

func observeDirectorOn(t testing.TB, d *detector, rules []string, screen screenSnapshot) screenObservation {
	t.Helper()
	observed, err := d.observeDirector(&screen, rules)
	if err != nil {
		t.Fatal(err)
	}
	return observed
}

func TestCodingAgentDetector_CursorFree_EveryDirectorScreenAnEmptyComposerRuleCitesClassifiesEmptyThroughDirectorRules(t *testing.T) {
	t.Parallel()
	var misclassified []string
	for _, backend := range codingAgentAll() {
		if _, ok := profileCatalogBundled().directorDelivery(backend); !ok {
			continue
		}
		d, rules := directorDetector(t, backend)
		for _, r := range jarray(jget(parsedBundledFiles(), backend.name(), "profile", "rules")).items {
			id, _ := jstring(jget(r, "id"))
			if !composerEmptyRule(r) || !slices.Contains(rules, id) {
				continue
			}
			cited, _ := jget(r, "fixture_ids").(*jsonArr)
			if cited == nil {
				continue
			}
			for _, fixture := range cited.items {
				name, _ := jstring(fixture)
				observed := observeDirectorOn(t, d, rules, replayAsCaptured(name, nil, none[[2]uint16]()))
				if observed.kind != observationKindComposerEmpty {
					misclassified = append(misclassified, fmt.Sprintf("%s (cited by %s): %s %s %v", name, id, observed.kind.debugString(), observed.ruleID, observed.matchedRegion))
				}
			}
		}
	}
	if len(misclassified) > 0 {
		t.Fatalf("an empty Director composer classifies empty: %q", misclassified)
	}
}

func TestCodingAgentDetector_CursorFree_ADraftedDirectorComposerBelowAnEmptyShapedBoxClassifiesNotEmpty(t *testing.T) {
	t.Parallel()
	for _, backend := range codingAgentAll() {
		if _, ok := profileCatalogBundled().directorDelivery(backend); !ok {
			continue
		}
		d, rules := directorDetector(t, backend)
		idle, drafted := herdrDirectorScreens(t, backend)
		var emptyBox []string
		var observedIdle []string
		for _, id := range idle {
			observed := observeDirectorOn(t, d, rules, replayAsCaptured(id, nil, none[[2]uint16]()))
			if region, ok := observed.matchedRegion.get(); ok && observed.kind == observationKindComposerEmpty {
				emptyBox = fixtureLines(id)[region.top:region.bottom]
				break
			}
			observedIdle = append(observedIdle, fmt.Sprintf("%s: %s %s", id, observed.kind.debugString(), observed.ruleID))
		}
		if emptyBox == nil {
			t.Fatalf("%s: a cursor-free idle Director screen classifies empty: %q", backend.name(), observedIdle)
		}
		if len(drafted) == 0 {
			t.Fatalf("%s: a drafted Director screen", backend.name())
		}
		for _, id := range drafted {
			if alone := observeDirectorOn(t, d, rules, replayAsCaptured(id, nil, none[[2]uint16]())); alone.kind == observationKindComposerEmpty {
				t.Fatalf("%s: the drafted composer is empty", id)
			}
			boxed := replayAsCaptured(id, func(lines *[]string) {
				for row, line := range emptyBox {
					(*lines)[row+1] = line
				}
			}, none[[2]uint16]())
			if _, has := boxed.cursor.get(); has {
				t.Fatalf("%s: a herdr screen replays cursor-free", id)
			}
			if observed := observeDirectorOn(t, d, rules, boxed); observed.kind == observationKindComposerEmpty {
				t.Fatalf("%s: an empty-shaped box above the drafted composer classifies empty", id)
			}
		}
	}
}

func TestCodingAgentDetector_CursorFree_ACaptureWithACursorKeepsUniquenessBeforeTheMatchers(t *testing.T) {
	t.Parallel()
	d := codexDetector(t, promptRowDefinition("any"))
	if observed := observeOn(t, d, promptRows(t, "history entry")); observed.ruleID == syntheticEmptyRule {
		t.Fatal("two qualifying candidates match nothing when the cursor is known")
	}
}

func TestCodingAgentDetector_CursorFree_AnObservationWithoutANativeVersionRecordsANullVersion(t *testing.T) {
	t.Parallel()
	for _, fixture := range bundledFixtureRecords() {
		screen := cursorFreeScreen(replayAsCaptured(fixture.id, nil, none[[2]uint16]()))
		observed := observeOn(t, newDetector(profileCatalogBundled(), fixture.backend, none[string]()), screen)
		if observed.rawEvidence.nativeVersion != none[string]() {
			t.Fatalf("%s: native version %v", fixture.id, observed.rawEvidence.nativeVersion)
		}
	}
}

var detectorFixtureGoldens = []detectorFixtureGolden{
	// BEGIN GENERATED: mise //:test -- -run 'TestCodingAgentDetector_FixtureGolden' -update-golden
	{fixture: "claude-2.1.302-default-permission-1", screen: 0xd63aa2196391346a, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,29],"operation":"/private/tmp/cafleet-194/cells-002/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-default-permission-1"}`},
	{fixture: "claude-2.1.302-allow-pre-stage-1", screen: 0xdb2f1853b27806d1, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,29],"operation":"/private/tmp/cafleet-194/ans-allow-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.302-deny-pre-stage-1", screen: 0x1b873450db77c185, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,29],"operation":"/private/tmp/cafleet-194/ans-deny-001/permission-probe deny","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.302-idle-composer-1", screen: 0xf6268757403cceb2, record: `{"size":[36,120],"cursor":[31,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-wrapped-permission-1", screen: 0x5052c42faa0c1988, record: `{"size":[36,52],"cursor":[28,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,36],"operation":"/tmp/cafleet-194/claude-wrap-cells-001/permission-\nprobe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-wrapped-permission-1"}`},
	{fixture: "claude-2.1.302-wrapped-allow-pre-stage-1", screen: 0x07a0b3bdec72c0a3, record: `{"size":[36,52],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,36],"operation":"/tmp/cafleet-194/claude-wrap-ans-allow-001/permiss\nion-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-wrapped-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.302-wrapped-deny-pre-stage-1", screen: 0x4eb7879d0c8f4a1c, record: `{"size":[36,52],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,36],"operation":"/tmp/cafleet-194/claude-wrap-ans-deny-001/permissi\non-probe deny","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-wrapped-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.302-boxed-permission-1", screen: 0x1d96491b54cc70b4, record: `{"size":[36,89],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,35],"operation":"│ /tmp/cafleet-194/claude-boxed-cells-089-wide-operation-probe-01/permission-probe\n│ allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-boxed-permission-1"}`},
	{fixture: "claude-2.1.302-boxed-single-row-permission-1", screen: 0xb560fcfc6d7cd083, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,30],"operation":"│ /tmp/cafleet-194/claude-boxed-cells-120-wide-operation-probe-01/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-boxed-single-row-permission-1"}`},
	{fixture: "claude-2.1.302-auto-approved-done-1", screen: 0x977b959543755997, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-approved-running-1", screen: 0xd4cbf4346d598dba, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-ask-allow-continuation-1", screen: 0x494f7a7a68ce9b8a, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-ask-deny-continuation-1", screen: 0x9513f811d51f243b, record: `{"size":[36,90],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-ask-hiding-refusal-notice-1", screen: 0xea06da09d18130bb, record: `{"size":[36,120],"cursor":[32,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-deny.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-ask-hiding-refusal-notice-1"}`},
	{fixture: "claude-2.1.302-auto-ask-hiding-refusal-notice-resized-1", screen: 0x344a833aae21e745, record: `{"size":[36,90],"cursor":[32,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,36],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-deny.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-ask-hiding-refusal-notice-1"}`},
	{fixture: "claude-2.1.302-auto-ask-permission-1", screen: 0x5eb23029777a1215, record: `{"size":[36,120],"cursor":[32,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-allow.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-ask-permission-1"}`},
	{fixture: "claude-2.1.302-auto-fresh-first-launch-notice-1", screen: 0xc429ecfdfb7f9be0, record: `{"size":[36,120],"cursor":[19,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[19,20],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-fresh-onboarding-theme-1", screen: 0xaeae50a2def19a20, record: `{"size":[36,120],"cursor":[21,1],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-idle-composer-1", screen: 0xf376e55ec325f4a0, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-model-switch-question-1", screen: 0x60a2136a7a1d9d26, record: `{"size":[36,120],"cursor":[26,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,35],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.302-auto-quote-false-positive-1", screen: 0xb5de90f07a011c3a, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-refusal-notice-expired-1", screen: 0x4a7be271d51beb1b, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-refusal-notice-idle-1", screen: 0x93aa472765b1b098, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-refusal-notice-working-1", screen: 0x236cedc17c61ae74, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-transcript-viewer-1", screen: 0x51915e91a06945c8, record: `{"size":[36,120],"cursor":[35,0],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-two-refusals-notice-1", screen: 0x7aad77853b09f5ab, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-unavailable-manual-footer-1", screen: 0xcb5bd1eb112a329e, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-unavailable-notice-1", screen: 0x9bed05997f38b765, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-verbose-quote-false-positive-1", screen: 0xbdb2e671c9111a70, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-verbose-refusal-above-form-1", screen: 0x2d7757f838f706c7, record: `{"size":[36,120],"cursor":[32,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-deny-two.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-verbose-refusal-above-form-1"}`},
	{fixture: "claude-2.1.302-auto-verbose-refusal-hidden-by-form-1", screen: 0xf343e334efd32682, record: `{"size":[20,60],"cursor":[16,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[6,20],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-deny-two.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-verbose-refusal-above-form-1"}`},
	{fixture: "claude-2.1.302-auto-verbose-refusal-restored-idle-1", screen: 0x003a8768d2283ce7, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-verbose-refusal-rewrapped-form-1", screen: 0xd092b92a72cf9eb3, record: `{"size":[36,60],"cursor":[32,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,36],"operation":"touch /tmp/cafleet-197/ws-cl2/ask-deny-two.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-auto-verbose-refusal-above-form-1"}`},
	{fixture: "claude-2.1.302-auto-verbose-refusal-rewrapped-idle-1", screen: 0x4522ce0b985a7d23, record: `{"size":[20,60],"cursor":[17,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[17,18],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-verbose-transcript-viewer-1", screen: 0x1f992d234f4ec260, record: `{"size":[36,120],"cursor":[35,0],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-verbose-two-refusals-1", screen: 0x9c2fc28cbf33f0af, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-trust-dialog-1", screen: 0x74b137a8bc856d3a, record: `{"size":[36,120],"cursor":[13,1],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-access-allow-continuation-1", screen: 0x4e3606ac3e480f7c, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-access-allow-pre-stage-1", screen: 0xa0907132f0f5f115, record: `{"size":[36,120],"cursor":[30,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/acc/ws-clx/tip-allow.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-access-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.302-access-allow-running-1", screen: 0xfbd5e4a98abca828, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-access-deny-continuation-1", screen: 0x20a2162bee27c7b4, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-access-deny-pre-stage-1", screen: 0xa56f29e20dce1439, record: `{"size":[36,120],"cursor":[30,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/acc/ws-clx/tip-deny.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-access-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.302-access-permission-1", screen: 0xa0907132f0f5f115, record: `{"size":[36,120],"cursor":[30,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,36],"operation":"touch /tmp/cafleet-197/acc/ws-clx/tip-allow.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.302-access-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.302-auto-env-setup-cancelled-idle-1", screen: 0x4b8b624272b73292, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.302-auto-env-setup-offer-1", screen: 0x897294ec4e8d6ed4, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.302-auto-env-setup-wizard-1", screen: 0x40d80f5ceed37181, record: `{"size":[36,120],"cursor":[30,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-env-setup-wizard","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[24,36],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.302-default-manual-idle-1", screen: 0xf1cf77fa4d549e9e, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.308-auto-tip-padded-question-1", screen: 0x300a2a47dcca442d, record: `{"size":[35,200],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,35],"operation":"touch rp-mark-b","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.308-auto-tip-padded-question-1"}`},
	{fixture: "claude-2.1.309-read-outside-permission-1", screen: 0x685c4a4c38432a01, record: `{"size":[36,120],"cursor":[28,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,34],"operation":"Read(/tmp/cafleet-194/claude-read-outside-cells-001-outside/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.309-read-outside-permission-1"}`},
	{fixture: "claude-2.1.309-read-outside-long-path-1", screen: 0xf3b3e7406cb89431, record: `{"size":[36,120],"cursor":[30,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,36],"operation":"Read(/tmp/cafleet-194/claude-read-outside-long-001-outside/a-deeply-nested-directory-for-the-long-read-operation/secon\nd-level-directory-that-wraps-the-displayed-path/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.309-read-outside-long-path-1"}`},
	{fixture: "claude-2.1.309-read-outside-allow-pre-stage-1", screen: 0x0a92306cc61717a8, record: `{"size":[36,120],"cursor":[28,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,34],"operation":"Read(/tmp/cafleet-194/claude-read-outside-allow-001-outside/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.309-read-outside-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.309-read-outside-deny-pre-stage-1", screen: 0x5164c8ddf41d6bfd, record: `{"size":[36,120],"cursor":[28,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,34],"operation":"Read(/tmp/cafleet-194/claude-read-outside-deny-001-outside/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.309-read-outside-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.309-read-outside-allow-continuation-1", screen: 0x83ceb56aa8ae5d85, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.309-read-outside-deny-continuation-1", screen: 0xba821ba135569262, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.309-edit-outside-permission-1", screen: 0xbf7ea3e9ef7d3465, record: `{"size":[36,120],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,35],"operation":"Read(/tmp/cafleet-194/claude-edit-outside-cells-001-outside/edit-target.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.309-edit-outside-permission-1"}`},
	{fixture: "claude-2.1.312-auto-default-nudge-1", screen: 0xea6e6ae3435f3012, record: `{"size":[36,120],"cursor":[26,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,28],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.312-auto-default-accept-edits-nudge-1", screen: 0xc13f75e81b60f781, record: `{"size":[36,120],"cursor":[26,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,28],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep accept edits"],"composer_text":null,"episode":"claude-2.1.312-auto-default-accept-edits-nudge-1"}`},
	{fixture: "claude-2.1.312-auto-default-decline-pre-stage-1", screen: 0x4308932bdb49058a, record: `{"size":[36,120],"cursor":[26,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,28],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.312-auto-default-decline-post-stage-1-1", screen: 0x85ced09887202701, record: `{"size":[36,120],"cursor":[27,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,28],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.312-auto-default-decline-continuation-1", screen: 0x5364f831b456cc22, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-question-pending-notice-1", screen: 0x23d68b288cb9e848, record: `{"size":[36,120],"cursor":[33,34],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-question-pending-notice","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,20],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.312-question-pending-after-clear-1", screen: 0xc3503e29d6109e5d, record: `{"size":[36,120],"cursor":[26,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,28],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.312-question-pending-send-notice-1", screen: 0xd42e4627f77f2497, record: `{"size":[36,120],"cursor":[33,34],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-question-pending-notice","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,20],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.312-question-pending-after-send-1", screen: 0x29a2ad621eb0beb1, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-question-pending-after-send-idle-1", screen: 0x0ccadf0ad4b138eb, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-auto-env-setup-offer-1", screen: 0xfd247269f13e7793, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.312-auto-env-setup-not-now-pre-stage-1", screen: 0xaac39cfdbb56e7cc, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.312-auto-env-setup-not-now-continuation-1", screen: 0xe900657b23aa33f0, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-auto-env-setup-dont-show-again-pre-stage-1", screen: 0x015ed2b353ab96a4, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.312-auto-env-setup-dont-show-again-continuation-1", screen: 0xb82bee8584e55576, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-primary-auto-default-nudge-1", screen: 0x197500184ceec654, record: `{"size":[36,120],"cursor":[24,3],"alternate_screen":false,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,26],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.312-primary-default-permission-1", screen: 0xbd0c622871560bf8, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,29],"operation":"/tmp/cafleet-194/claude-primary-auto-tip-cells-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-primary-default-permission-1"}`},
	{fixture: "claude-2.1.312-primary-closed-form-1", screen: 0x3141dd4887a48984, record: `{"size":[36,120],"cursor":[16,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[16,17],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-primary-idle-composer-1", screen: 0x3141dd4887a48984, record: `{"size":[36,120],"cursor":[16,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[16,17],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-primary-resized-narrow-1", screen: 0xf478388787e06252, record: `{"size":[36,100],"cursor":[17,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[17,18],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-primary-resized-idle-composer-1", screen: 0x9da428afb4822a4f, record: `{"size":[36,120],"cursor":[17,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[17,18],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.312-read-file-permission-1", screen: 0xbb0861a8a6bc4d23, record: `{"size":[36,120],"cursor":[21,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,26],"operation":"Read(/tmp/cafleet-194/claude-read-file-cells-001-outside/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-permission-1"}`},
	{fixture: "claude-2.1.312-read-file-allow-pre-stage-1", screen: 0xca3741ba0a04a655, record: `{"size":[36,120],"cursor":[21,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,26],"operation":"Read(/tmp/cafleet-194/claude-read-file-allow-001-outside/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.312-read-file-deny-pre-stage-1", screen: 0x67c4346aef51b803, record: `{"size":[36,120],"cursor":[21,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,26],"operation":"Read(/tmp/cafleet-194/claude-read-file-deny-001-outside/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.312-edit-file-permission-1", screen: 0x51aad514a648be4e, record: `{"size":[36,120],"cursor":[26,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-edit-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,32],"operation":"../claude-edit-file-cells-001-outside/edit-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-edit-file-permission-1"}`},
	{fixture: "claude-2.1.312-edit-file-allow-pre-stage-1", screen: 0x21e36f164e8762c0, record: `{"size":[36,120],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-edit-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,33],"operation":"../claude-edit-file-allow-001-outside/edit-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-edit-file-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.312-edit-file-deny-pre-stage-1", screen: 0x8ee3503cf4b2bb23, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-edit-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,30],"operation":"../claude-edit-file-deny-001-outside/edit-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-edit-file-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.312-create-file-permission-1", screen: 0x14beed005ec30c2f, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-create-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,28],"operation":"../claude-create-file-cells-001-outside/write-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-create-file-permission-1"}`},
	{fixture: "claude-2.1.312-create-file-allow-pre-stage-1", screen: 0x995c3ce7c164d43e, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-create-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,28],"operation":"../claude-create-file-allow-001-outside/write-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-create-file-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.312-create-file-deny-pre-stage-1", screen: 0xd17332d2832bbd86, record: `{"size":[36,120],"cursor":[21,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-create-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,27],"operation":"../claude-create-file-deny-001-outside/write-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-create-file-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.312-bash-no-tip-permission-1", screen: 0xc76775b4c37cf033, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,29],"operation":"/tmp/cafleet-194/claude-bash-no-tip-cells-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-bash-no-tip-permission-1"}`},
	{fixture: "claude-2.1.312-bash-no-tip-allow-pre-stage-1", screen: 0xb183922adde33dcb, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,29],"operation":"/tmp/cafleet-194/claude-bash-no-tip-allow-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-bash-no-tip-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.312-bash-no-tip-deny-pre-stage-1", screen: 0x813982ef73fc2e1d, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,29],"operation":"/tmp/cafleet-194/claude-bash-no-tip-deny-001/permission-probe deny","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-bash-no-tip-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.312-bash-no-tip-access-permission-1", screen: 0x8b8e6a428586bccc, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,27],"operation":"│ touch /tmp/cafleet-194/claude-bash-no-tip-access-cells-001-outside/no-tip-access.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-bash-no-tip-access-permission-1"}`},
	{fixture: "claude-2.1.312-panel-default-permission-1", screen: 0xba5181d5d793e073, record: `{"size":[36,166],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,36],"operation":"/tmp/cafleet-194/claude-panel-auto-tip-cells-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-panel-default-permission-1"}`},
	{fixture: "claude-2.1.312-panel-default-permission-144-1", screen: 0x56026832ebab83df, record: `{"size":[36,144],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,36],"operation":"/tmp/cafleet-194/claude-panel-auto-tip-cells-002/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-panel-default-permission-144-1"}`},
	{fixture: "claude-2.1.312-panel-default-allow-pre-stage-1", screen: 0xc632ce1294734b20, record: `{"size":[36,166],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,36],"operation":"/tmp/cafleet-194/claude-panel-auto-tip-allow-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-panel-default-allow-pre-stage-1"}`},
	{fixture: "claude-2.1.312-read-file-batched-permission-1", screen: 0x4732766035e3126d, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,28],"operation":"Read(/tmp/cafleet-194/claude-read-file-batched-cells-001-outside/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-batched-permission-1"}`},
	{fixture: "claude-2.1.312-read-file-batched-allow-post-stage-1", screen: 0x7787dacb96dba057, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,28],"operation":"Read(/tmp/cafleet-194/claude-read-file-batched-cells-001-outside/batched-probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-batched-allow-post-stage-1"}`},
	{fixture: "claude-2.1.312-read-file-batched-deny-pre-stage-1", screen: 0x4cb326e33e13db83, record: `{"size":[36,120],"cursor":[23,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,28],"operation":"Read(/tmp/cafleet-194/claude-read-file-batched-deny-001-outside/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-read-file-batched-deny-pre-stage-1"}`},
	{fixture: "claude-2.1.312-bash-batched-permission-1", screen: 0x5e3cf1c9820379cc, record: `{"size":[36,120],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,33],"operation":"/tmp/cafleet-194/claude-bash-batched-cells-001/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.312-bash-batched-permission-1"}`},
	{fixture: "claude-2.1.319-composer-exec-plain-40-args-post-1900ms-tapped-v2-tmux-120-42", screen: 0xe97d2e7651ad9cc5, record: `{"size":[39,120],"cursor":[34,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.319-composer-prompt-idle-mode-2004-tapped-v2-herdr-119-1", screen: 0x1384178f866df965, record: `{"size":[40,119],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68", screen: 0x71cb7e0a54e63ed6, record: `{"size":[40,119],"cursor":[37,42],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68", screen: 0x18574626e7eed22b, record: `{"size":[39,120],"cursor":[36,42],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33", screen: 0x37bc20c897eba716, record: `{"size":[40,119],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33", screen: 0x0b85802efb8e070a, record: `{"size":[39,120],"cursor":[34,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-suggestion-herdr-094-1", screen: 0x78212d818ce83b65, record: `{"size":[38,94],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Try \"how does <filepath> work?\"","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-bash-deny-pre-stage-tmux-120-1", screen: 0x071f9d4d99475e04, record: `{"size":[36,120],"cursor":[26,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,32],"operation":"/tmp/cafleet-programmer-research/claude-work/permission-probe deny","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-bash-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-bash-deny-post-stage-tmux-120-1", screen: 0x3231c729bf0c2678, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-bash-allow-pre-stage-tmux-120-1", screen: 0x54cb12a13a8efbb7, record: `{"size":[36,120],"cursor":[26,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,32],"operation":"/tmp/cafleet-programmer-research/claude-work/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-bash-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-bash-allow-post-stage-tmux-120-1", screen: 0x96a97257d7f698a2, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-no-tip-deny-pre-stage-tmux-120-1", screen: 0x6f9d8451bee94cc2, record: `{"size":[36,120],"cursor":[25,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"/tmp/cafleet-programmer-research/claude-work/permission-probe deny","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-bash-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-no-tip-deny-post-stage-tmux-120-1", screen: 0xb514e40ecc535cf9, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-no-tip-allow-pre-stage-tmux-120-1", screen: 0xd0aef8e642abfa18, record: `{"size":[36,120],"cursor":[25,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-no-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"/tmp/cafleet-programmer-research/claude-work/permission-probe allow","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-bash-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-no-tip-allow-post-stage-tmux-120-1", screen: 0x0d3e8df7b57c29aa, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-tip-deny-pre-stage-tmux-120-1", screen: 0x7bd3bacf4ff4f943, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"touch /tmp/cafleet-programmer-research/cl-outside/tip-deny/tip-deny.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-tip-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-tip-deny-post-stage-tmux-120-1", screen: 0x7b6e887cef850378, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-tip-allow-pre-stage-tmux-120-1", screen: 0x10857ea0b39904c6, record: `{"size":[36,120],"cursor":[24,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-bash-permission-auto-tip","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"touch /tmp/cafleet-programmer-research/cl-outside/tip-allow/tip-allow.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-tip-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-tip-allow-post-stage-tmux-120-1", screen: 0xe0a9cf7fbb1a288b, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-deny-pre-stage-tmux-120-1", screen: 0x85c65e6afad13b21, record: `{"size":[36,120],"cursor":[26,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"touch /tmp/cafleet-programmer-research/claude-work/ask-deny.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-ask-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-ask-deny-post-stage-tmux-120-1", screen: 0xf8bdd0d0f6b56cdb, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-allow-pre-stage-tmux-120-1", screen: 0x6c6edeb2193d1635, record: `{"size":[36,120],"cursor":[26,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-auto-ask-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,30],"operation":"touch /tmp/cafleet-programmer-research/claude-work/ask-allow.marker","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-ask-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-ask-allow-post-stage-tmux-120-1", screen: 0x39273dae774346f2, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-read-deny-pre-stage-tmux-120-1", screen: 0xa690682969dc3241, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,27],"operation":"Read(/tmp/cafleet-programmer-research/cl-outside/read-deny/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-read-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-read-deny-post-stage-tmux-120-1", screen: 0xb22b8180b3cc30e2, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-read-allow-pre-stage-tmux-120-1", screen: 0xb6e2eb68372c5787, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,27],"operation":"Read(/tmp/cafleet-programmer-research/cl-outside/read-allow/probe.txt)","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-read-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-read-allow-post-stage-tmux-120-1", screen: 0x4deecd2ead3c2962, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-readout-deny-pre-stage-tmux-120-1", screen: 0xc8e86b3309647ee4, record: `{"size":[36,120],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,35],"operation":"Read(/tmp/cafleet-programmer-research/cl-outside/readout-deny/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.322-readout-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-readout-deny-post-stage-tmux-120-1", screen: 0x532604f1191b4f4d, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-readout-allow-pre-stage-tmux-120-1", screen: 0x8d81f48a8e205f59, record: `{"size":[36,120],"cursor":[29,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-read-outside-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,35],"operation":"Read(/tmp/cafleet-programmer-research/cl-outside/readout-allow/probe.txt)","choices":["Yes, but ask again next time","No, and ask again next time"],"composer_text":null,"episode":"claude-2.1.322-readout-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-readout-allow-post-stage-tmux-120-1", screen: 0xa17aa5682ec037bd, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-edit-deny-pre-stage-tmux-120-1", screen: 0x3bc0fb59465342f8, record: `{"size":[36,120],"cursor":[30,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-edit-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"../cl-outside/edit-deny/edit-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-edit-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-edit-deny-post-stage-tmux-120-1", screen: 0x01d0fcce45159f55, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-edit-allow-pre-stage-tmux-120-1", screen: 0x5851422faf3ff9f0, record: `{"size":[36,120],"cursor":[27,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-edit-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,33],"operation":"../cl-outside/edit-allow/edit-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-edit-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-edit-allow-post-stage-tmux-120-1", screen: 0xb02165f7c16c047a, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-create-deny-pre-stage-tmux-120-1", screen: 0xdc5d9cac25883743, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-create-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,28],"operation":"../cl-outside/create-deny/write-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-create-deny-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-create-deny-post-stage-tmux-120-1", screen: 0x9fc501b9759d01ca, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-create-allow-pre-stage-tmux-120-1", screen: 0x8a170e6d80c8fb3e, record: `{"size":[36,120],"cursor":[22,1],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"claude-create-file-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,28],"operation":"../cl-outside/create-allow/write-target.txt","choices":["Yes","No"],"composer_text":null,"episode":"claude-2.1.322-create-allow-pre-stage-tmux-120-1"}`},
	{fixture: "claude-2.1.322-create-allow-post-stage-tmux-120-1", screen: 0xcaa3db58c66991b1, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-nudge-decline-pre-stage-tmux-120-1", screen: 0x709245a07114c42f, record: `{"size":[36,120],"cursor":[15,3],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-default-nudge","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,17],"operation":null,"choices":["Yes, set auto mode as my default permission mode","No, keep manual mode"],"composer_text":null,"episode":"claude-2.1.312-auto-default-nudge-1"}`},
	{fixture: "claude-2.1.322-nudge-decline-post-stage-tmux-120-1", screen: 0x7c56e5c7d7f914eb, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Try \"how do I log an error?\"","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-env-not-now-pre-stage-tmux-120-1", screen: 0x991fe02196ad4616, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.322-env-dont-show-pre-stage-tmux-120-1", screen: 0x961d36444b06b019, record: `{"size":[36,120],"cursor":[27,2],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-auto-startup","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,32],"operation":null,"choices":["Not now","Don't show again"],"composer_text":null,"episode":"claude-2.1.302-auto-env-setup-offer-1"}`},
	{fixture: "claude-2.1.322-queued-hint-tmux-120-1", screen: 0x6dd06d2477271674, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Press up to edit queued messages","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-41-literal-tmux-120-1", screen: 0xa508b5d34769c164, record: `{"size":[39,120],"cursor":[36,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1", screen: 0x464cf98542f5a205, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-4000-post-submit-tmux-120-1", screen: 0x62c9f30a84e6ba82, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-41-literal-tmux-120-1", screen: 0x0f06e21519a21614, record: `{"size":[39,120],"cursor":[36,44],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-41-post-submit-tmux-120-1", screen: 0x846882ae4defc1bf, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-4000-post-submit-tmux-120-1", screen: 0x1f85c3f468d72727, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-41-literal-herdr-092-1", screen: 0xc0fd0c12679ae963, record: `{"size":[36,92],"cursor":[33,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-41-post-submit-herdr-092-1", screen: 0x31f25e03cb06e177, record: `{"size":[36,92],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-4000-post-submit-herdr-092-1", screen: 0x13a232266375ef54, record: `{"size":[36,92],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-41-literal-herdr-092-1", screen: 0xa2c89b090a8f7624, record: `{"size":[36,92],"cursor":[33,44],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,35],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-41-post-submit-herdr-092-1", screen: 0x932a039ff79f181c, record: `{"size":[36,92],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-4000-post-submit-herdr-092-1", screen: 0xaa2fc4e0ff66c67a, record: `{"size":[36,92],"cursor":[33,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-single-tmux-120-1", screen: 0xb5ed6909d7cb63f0, record: `{"size":[36,120],"cursor":[14,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-single-closed-tmux-120-1", screen: 0x18ec1956f1d32c0d, record: `{"size":[36,120],"cursor":[31,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-tabs-tmux-120-1", screen: 0x6ece7788b39083aa, record: `{"size":[36,120],"cursor":[15,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,22],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-tabs-tmux-120-2", screen: 0x5802c13eede1b7d8, record: `{"size":[36,120],"cursor":[15,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,22],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-tmux-120-1", screen: 0x70f8645cdf957fd0, record: `{"size":[36,120],"cursor":[14,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,23],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-tmux-120-2", screen: 0x2bc267b455a6fc1d, record: `{"size":[36,120],"cursor":[15,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,23],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-tmux-120-3", screen: 0xf4d7d2a689291a28, record: `{"size":[36,120],"cursor":[15,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,23],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-tmux-120-1", screen: 0x1990ca04d4b4a326, record: `{"size":[36,120],"cursor":[15,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,22],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-tmux-120-2", screen: 0xe8aad3c68edd46e2, record: `{"size":[36,120],"cursor":[23,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question-review","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[12,25],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-narrow-tmux-060-1", screen: 0xc8be59cbf9fa2c9e, record: `{"size":[36,60],"cursor":[19,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,26],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-narrow-tmux-060-2", screen: 0x966cd937c9fff138, record: `{"size":[36,60],"cursor":[27,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question-review","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[16,29],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-narrow-tmux-040-1", screen: 0x1b3885713ca7831f, record: `{"size":[36,40],"cursor":[24,0],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-review-narrow-tmux-040-2", screen: 0xd32ffdf463d724ad, record: `{"size":[36,40],"cursor":[32,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question-review","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,34],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-typing-tmux-120-1", screen: 0xb2348cdddd90e6e5, record: `{"size":[36,120],"cursor":[16,5],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-typing-tmux-120-2", screen: 0x7fde45f5a02acc1d, record: `{"size":[36,120],"cursor":[16,8],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-narrow-tmux-060-1", screen: 0xf4fd536325e1b938, record: `{"size":[36,60],"cursor":[17,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[13,24],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-narrow-tmux-040-1", screen: 0x8a79cbbe127b8626, record: `{"size":[36,40],"cursor":[21,0],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-quoted-tmux-120-1", screen: 0x8d9a14d8beb25b09, record: `{"size":[36,120],"cursor":[31,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-pending-tmux-120-1", screen: 0xb5ed6909d7cb63f0, record: `{"size":[36,120],"cursor":[14,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-single-herdr-359-1", screen: 0x836cf7a1f45200e1, record: `{"size":[84,359],"cursor":[12,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[8,19],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-single-closed-herdr-359-1", screen: 0xd4eb34a5bacf4ff3, record: `{"size":[84,359],"cursor":[79,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[79,80],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-ask-tabs-herdr-359-1", screen: 0x72ad4c1de6ab21be, record: `{"size":[84,359],"cursor":[13,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[9,20],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-tabs-herdr-359-2", screen: 0x2f14b8521c96d746, record: `{"size":[84,359],"cursor":[13,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[9,20],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-herdr-359-1", screen: 0xa9b232f288cd913b, record: `{"size":[84,359],"cursor":[12,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[8,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-herdr-359-2", screen: 0x70d8f801e9a304ac, record: `{"size":[84,359],"cursor":[13,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[8,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-multi-herdr-359-3", screen: 0xa2dccad386b5eccd, record: `{"size":[84,359],"cursor":[13,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[8,21],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-herdr-359-1", screen: 0x2e057970c32ce008, record: `{"size":[84,359],"cursor":[13,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[9,20],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.322-ask-review-herdr-359-2", screen: 0x854cb09901af2972, record: `{"size":[84,359],"cursor":[21,0],"alternate_screen":true,"kind":"Interaction(Question)","rule_id":"claude-ask-user-question-review","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,23],"operation":null,"choices":[],"composer_text":null,"episode":"claude-2.1.302-auto-model-switch-question-1"}`},
	{fixture: "claude-2.1.325-director-idle-empty-tmux-120-1", screen: 0xc4a1971b2622c51f, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Try \"fix lint errors\"","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-one-tmux-120-1", screen: 0x58873301f1b0340f, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-wrap-tmux-120-1", screen: 0x72ee6ecef722701e, record: `{"size":[39,120],"cursor":[35,96],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":2,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[116]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-busy-tmux-120-1", screen: 0xa28c957bb047adf1, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.325-director-idle-empty-herdr-120-1", screen: 0x3ca1174b8e8ca9cc, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"claude-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Try \"edit input.rs to...\"","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-one-herdr-120-1", screen: 0x7ba62b088bb7e0c9, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-wrap-herdr-120-1", screen: 0x4979e3a3c6d72e4d, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":2,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[115]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-busy-herdr-120-1", screen: 0xdad2182f43fbdb88, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"claude-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-wrap-literal-tmux-120-1", screen: 0x883a8044a27cc52b, record: `{"size":[39,120],"cursor":[35,96],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":2,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[116]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-wrap-literal-tmux-120-1", screen: 0x47b7b0bbc130acd9, record: `{"size":[39,120],"cursor":[36,87],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":2,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[116]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-plain-456-tmux-80-1", screen: 0xbcf28896a6ad05f5, record: `{"size":[24,80],"cursor":[20,78],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,22],"rows":6,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "claude-2.1.325-director-plain-456-herdr-81-1", screen: 0x86e1a8a322e09d6c, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,22],"rows":6,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-456-literal-tmux-80-1", screen: 0xbcf28896a6ad05f5, record: `{"size":[24,80],"cursor":[20,78],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,22],"rows":6,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-prompt-plain-456-literal-herdr-81-1", screen: 0x3e35d1c3c144444b, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,22],"rows":6,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-456-literal-tmux-80-1", screen: 0xcd95f39b83c80187, record: `{"size":[24,80],"cursor":[21,77],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[16,23],"rows":6,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "claude-2.1.322-composer-exec-plain-456-literal-herdr-81-1", screen: 0x9485f7e5b61de88c, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[16,23],"rows":6,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304,380]}]},"episode":null}`},
	{fixture: "codex-0.163.0-on-request-permission-1", screen: 0xfa520de0ecc679b8, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,36],"operation":"/tmp/cafleet-194/codex-cells-001/permission-probe allow","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.163.0-on-request-permission-1"}`},
	{fixture: "codex-0.163.0-allow-pre-stage-1", screen: 0x7e778475fc667960, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-194/codex-ans-allow-001/permission-probe allow","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.163.0-allow-pre-stage-1"}`},
	{fixture: "codex-0.163.0-deny-pre-stage-1", screen: 0x52a6788d8cf72cdf, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-194/codex-ans-deny-001/permission-probe deny","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.163.0-deny-pre-stage-1"}`},
	{fixture: "codex-0.167.0-composer-exec-paste-1037-args-post-1900ms-tapped-v2-herdr-119-19", screen: 0x9efd7b9cc22e2257, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-1037-args-post-1900ms-tapped-v2-tmux-120-19", screen: 0xa3a46ae9c3b460e2, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-40-args-post-1900ms-tapped-v2-herdr-119-13", screen: 0x84e7dd3e59dfa382, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-40-args-post-1900ms-tapped-v2-tmux-120-13", screen: 0x22af981bb5872b16, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-25", screen: 0x7fd86e13ffda32ea, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-25", screen: 0xb94991c9e2819dce, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1000-args-execution-proof-post-1900ms-tapped-v2-herdr-119-12", screen: 0x197170e48ccd4a51, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1000-args-execution-proof-post-1900ms-tapped-v2-tmux-120-12", screen: 0x0cd572894369b2a8, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1001-args-execution-proof-post-1900ms-tapped-v2-herdr-119-18", screen: 0x36088066515799ad, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1001-args-execution-proof-post-1900ms-tapped-v2-tmux-120-18", screen: 0xe1f7b9c8c2d315a3, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1024-args-post-1900ms-tapped-v2-herdr-119-31", screen: 0xe9b16d186af0d8dd, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1024-args-post-1900ms-tapped-v2-tmux-120-31", screen: 0xb7ac3cf8bbc28530, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1025-args-post-1900ms-tapped-v2-herdr-119-37", screen: 0x400511f9bc9c06f9, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024},{"text":"a","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-1025-args-post-1900ms-tapped-v2-tmux-120-37", screen: 0x6688c1884f9ea1b5, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024},{"text":"a","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-40-args-post-1900ms-tapped-v2-herdr-119-7", screen: 0xa4da4add18986e99, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-40-args-post-1900ms-tapped-v2-tmux-120-7", screen: 0xe0016da19f2a3096, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-unsubmitted-literal-confirmed-tapped-v2-herdr-119-40", screen: 0xce3ab23466ea1857, record: `{"size":[40,119],"cursor":[36,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-unsubmitted-literal-confirmed-tapped-v2-tmux-120-40", screen: 0x60938c6209ed68f4, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-idle-mode-2004-tapped-v2-herdr-119-1", screen: 0xaf3895db5b0ac7cb, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-idle-mode-2004-tapped-v2-tmux-120-1", screen: 0xea5da9af00f2e8a3, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-1037-post-1900ms-tapped-v2-herdr-119-19", screen: 0x218096878d3cfe64, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-1037-post-1900ms-tapped-v2-tmux-120-19", screen: 0x5313291017d4b367, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-40-post-1900ms-tapped-v2-herdr-119-13", screen: 0xd8804dbf0880b920, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-40-post-1900ms-tapped-v2-tmux-120-13", screen: 0x5826fda2dd8f5b86, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-25", screen: 0x7db177dcbcf73264, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-25", screen: 0x430583285fe51d4f, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1024-post-1900ms-tapped-v2-herdr-119-31", screen: 0x4b9edf4a564b3aaf, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1024-post-1900ms-tapped-v2-tmux-120-31", screen: 0x9565ace6a3e9c1c4, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1025-post-1900ms-tapped-v2-herdr-119-37", screen: 0x95c2b37aa683bd43, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024},{"text":"a","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1025-post-1900ms-tapped-v2-tmux-120-37", screen: 0x92c8ea90c6f63aea, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024},{"text":"a","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-40-post-1900ms-tapped-v2-herdr-119-7", screen: 0x8b480d0f9e22448a, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-40-post-1900ms-tapped-v2-tmux-120-7", screen: 0x59365119de8b4b94, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-unsubmitted-literal-confirmed-tapped-v2-herdr-119-1", screen: 0x5a1cf7a4f2372a10, record: `{"size":[40,119],"cursor":[36,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-unsubmitted-literal-confirmed-tapped-v2-tmux-120-1", screen: 0x21cf1fcd8dcac3d9, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-unsubmitted-literal-tapped-v2-herdr-119-41", screen: 0x6309bbc3adb116bd, record: `{"size":[40,119],"cursor":[36,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-unsubmitted-literal-tapped-v2-tmux-120-41", screen: 0xbbe63c149b77c39e, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-42-args-literal-tapped-v2-herdr-119-39", screen: 0xf5a9d8be41d989ab, record: `{"size":[40,119],"cursor":[36,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-42-args-literal-tapped-v2-tmux-120-39", screen: 0x60938c6209ed68f4, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-retained-authoritative-no-submit-tapped-v2-herdr-119-2", screen: 0xf7111d3ae2ec1e4c, record: `{"size":[40,119],"cursor":[36,47],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"CF228_RETAIN_prompt_herdr_1794671922561553059","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-retained-authoritative-no-submit-tapped-v2-tmux-120-2", screen: 0x9e5e53517d289181, record: `{"size":[39,120],"cursor":[35,46],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"CF228_RETAIN_prompt_tmux_1794671921279867495","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-suggestion-tmux-120-1", screen: 0xbcd253b59b5bf9c6, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.166.0-deny-pre-stage-tmux-120-1", screen: 0x054c99229b56d654, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-programmer-research/cx-outside/permission-probe deny","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.166.0-deny-pre-stage-tmux-120-1"}`},
	{fixture: "codex-0.166.0-allow-pre-stage-tmux-120-1", screen: 0x4666f2538c734a77, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-programmer-research/cx-outside/permission-probe allow","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.166.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "codex-0.167.0-deny-pre-stage-tmux-120-1", screen: 0xa6679054c52bf413, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-programmer-research/cx-outside/permission-probe deny","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.166.0-deny-pre-stage-tmux-120-1"}`},
	{fixture: "codex-0.167.0-deny-post-stage-tmux-120-1", screen: 0x27213de39f373e1d, record: `{"size":[36,120],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-allow-pre-stage-tmux-120-1", screen: 0x80dd17914302894f, record: `{"size":[36,120],"cursor":[35,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"codex-exec-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,36],"operation":"/tmp/cafleet-programmer-research/cx-outside/permission-probe allow","choices":["Yes, proceed","No, and tell Codex what to do differently"],"composer_text":null,"episode":"codex-0.166.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "codex-0.167.0-allow-post-stage-tmux-120-1", screen: 0x8bd79189f96dc619, record: `{"size":[36,120],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-auto-status-tmux-120-1", screen: 0xe510940d1c387940, record: `{"size":[36,120],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.166.0-composer-prompt-plain-41-post-submit-tmux-120-1", screen: 0x1faff3287836937c, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.166.0-composer-exec-plain-42-literal-tmux-120-1", screen: 0x8f7630dc62344bd7, record: `{"size":[39,120],"cursor":[35,45],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.166.0-composer-prompt-plain-41-post-submit-herdr-092-1", screen: 0x7c2d946e67c432b4, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.166.0-composer-exec-plain-42-literal-herdr-092-1", screen: 0xed753bbe000290da, record: `{"size":[36,92],"cursor":[32,45],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-idle-empty-tmux-120-1", screen: 0x8a6f1a4881fd2800, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-41-literal-tmux-120-1", screen: 0x768c6407fe02d12e, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-41-post-submit-tmux-120-1", screen: 0xaebd0632b93a8d93, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1024-post-submit-tmux-120-1", screen: 0xa4776e729cbfc49c, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-4000-post-submit-tmux-120-1", screen: 0xde9e78bc833d37dc, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-42-literal-tmux-120-1", screen: 0x72d089b087c3dec7, record: `{"size":[39,120],"cursor":[35,45],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-998-post-submit-tmux-120-1", screen: 0xf524ba0649b99a55, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-4000-post-submit-tmux-120-1", screen: 0x2a2dafcc7733c636, record: `{"size":[39,120],"cursor":[35,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-idle-empty-herdr-092-1", screen: 0x2a4706680031d57a, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-41-literal-herdr-092-1", screen: 0x1a6cbc48e553580e, record: `{"size":[36,92],"cursor":[32,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-41-post-submit-herdr-092-1", screen: 0x2acf9c5e632d20e8, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-plain-1024-post-submit-herdr-092-1", screen: 0x6f1ca69ad93dad0e, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-prompt-paste-4000-post-submit-herdr-092-1", screen: 0x1d7278e7273a70d2, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-42-literal-herdr-092-1", screen: 0x35ffc7e802036931, record: `{"size":[36,92],"cursor":[32,45],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-plain-998-post-submit-herdr-092-1", screen: 0x4c10e77f10b7e57c, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.167.0-composer-exec-paste-4000-post-submit-herdr-092-1", screen: 0xa5ffea970e26a22b, record: `{"size":[36,92],"cursor":[32,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-idle-empty-tmux-120-1", screen: 0x9bfec29f075eaf67, record: `{"size":[39,120],"cursor":[34,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-one-tmux-120-1", screen: 0x02fbed0ad1c5be95, record: `{"size":[39,120],"cursor":[35,43],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-wrap-tmux-120-1", screen: 0x75f3abfa2a3e0d9b, record: `{"size":[39,120],"cursor":[35,85],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,37],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,126]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-plain-1024-tmux-120-1", screen: 0xc35b700a79bf9e40, record: `{"size":[39,120],"cursor":[35,90],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[27,37],"rows":9,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[117,234,351,468,585,702,819,936]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-busy-tmux-120-1", screen: 0xb1c3267515d6a60a, record: `{"size":[39,120],"cursor":[34,2],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-idle-empty-herdr-120-1", screen: 0xf1ac9a21160f6ad2, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-one-herdr-120-1", screen: 0xff5b76f3c3062381, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-wrap-herdr-120-1", screen: 0x15aae5ecb0dd6cc4, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,38],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,125]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-plain-1024-herdr-120-1", screen: 0xc41352c7ed379703, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[28,38],"rows":9,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[116,232,348,464,580,696,812,928]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-busy-herdr-120-1", screen: 0x606884068bbea22f, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"codex-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask Codex to do anything","breaks":[]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-prompt-plain-1024-literal-tmux-80-1", screen: 0x2ecc7c6bd9246454, record: `{"size":[24,80],"cursor":[22,25],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[9,24],"rows":14,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[77,154,231,308,385,462,539,616,693,770,847,924,1001]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-prompt-paste-placeholder-1024-literal-herdr-81-1", screen: 0xe2b457a1f30215f2, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[19,21],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-exec-plain-1000-literal-tmux-80-1", screen: 0x3603b1ba72364ad2, record: `{"size":[24,80],"cursor":[22,71],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[9,24],"rows":14,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,82,159,236,313,390,467,544,621,698,775,852,929]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-exec-plain-1000-literal-herdr-81-1", screen: 0x5a2172f0377cc092, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[9,24],"rows":14,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,82,159,236,313,390,467,544,621,698,775,852,929]}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-plain-1024-tmux-80-1", screen: 0x63077564ed2ba20d, record: `{"size":[24,80],"cursor":[19,29],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[19,21],"rows":1,"mode_prefix":"!","segments":[{"scalars":1024}]},"episode":null}`},
	{fixture: "codex-0.169.0-director-plain-1024-herdr-81-1", screen: 0x49ef57ed85b99487, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[9,24],"rows":14,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[77,154,231,308,385,462,539,616,693,770,847,924,1001]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-prompt-wrap-literal-tmux-120-1", screen: 0x75f3abfa2a3e0d9b, record: `{"size":[39,120],"cursor":[35,85],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,37],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,126]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-exec-wrap-literal-tmux-120-1", screen: 0x48e929d4f1dcf149, record: `{"size":[39,120],"cursor":[34,80],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,36],"rows":3,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,122]}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-prompt-paste-2000-literal-tmux-120-1", screen: 0x8d7883acf006cd4c, record: `{"size":[39,120],"cursor":[34,29],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"scalars":2000}]},"episode":null}`},
	{fixture: "codex-0.169.0-composer-exec-paste-2000-literal-tmux-120-1", screen: 0x017f1dd864a7121e, record: `{"size":[39,120],"cursor":[34,29],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":"!","segments":[{"scalars":2002}]},"episode":null}`},
	{fixture: "opencode-2.1.4-ask-permission-1", screen: 0x1af34743b082d885, record: `{"size":[36,120],"cursor":[22,119],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":"/tmp/cafleet-194/opencode-cells-001/permission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-ask-permission-1"}`},
	{fixture: "opencode-2.1.4-allow-pre-stage-1", screen: 0x7bb4e8930708a685, record: `{"size":[36,120],"cursor":[22,119],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":"/tmp/cafleet-194/opencode-ans-allow-001/permission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-allow-pre-stage-1"}`},
	{fixture: "opencode-2.1.4-deny-pre-stage-1", screen: 0x579fab4a95c4428f, record: `{"size":[36,120],"cursor":[22,119],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":"/tmp/cafleet-194/opencode-ans-deny-001/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-1"}`},
	{fixture: "opencode-2.1.4-deny-pre-stage-2", screen: 0xca97310934a07cef, record: `{"size":[36,120],"cursor":[21,38],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":"/tmp/cafleet-194/opencode-ans-deny-002/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-2"}`},
	{fixture: "opencode-2.1.4-deny-pre-stage-3", screen: 0x33a3866214ea4f90, record: `{"size":[36,120],"cursor":[21,2],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.1.4-focus-always-1", screen: 0x058d9165f72d7e24, record: `{"size":[36,120],"cursor":[21,29],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[10,22],"operation":"/tmp/cafleet-194/opencode-focus-always-002/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-focus-always-1"}`},
	{fixture: "opencode-2.1.4-idle-composer-1", screen: 0x31029540c46a86a1, record: `{"size":[36,120],"cursor":[11,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[11,14],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.1.4-narrow-permission-057-1", screen: 0x17faabba831824dc, record: `{"size":[36,57],"cursor":[35,56],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,35],"operation":"/tmp/cafleet-194/opencode-narrow-cells-057/\npermission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-narrow-permission-057-1"}`},
	{fixture: "opencode-2.1.4-narrow-deny-pre-stage-2-057-1", screen: 0x59e61900731024de, record: `{"size":[36,57],"cursor":[22,38],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[13,25],"operation":"/tmp/cafleet-194/opencode-narrow-deny-002-057/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-narrow-deny-pre-stage-2-057-1"}`},
	{fixture: "opencode-2.1.4-narrow-deny-pre-stage-3-057-1", screen: 0x8e5c854c3faa644d, record: `{"size":[36,57],"cursor":[32,2],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,35],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.1.4-narrow-long-command-057-1", screen: 0x8b6f6bc06c63bb12, record: `{"size":[36,57],"cursor":[35,56],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.1.4-narrow-idle-composer-057-1", screen: 0x97ef56aaf0028cb3, record: `{"size":[36,57],"cursor":[11,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[11,14],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.1.4-compact-permission-055-1", screen: 0x196bc6752ab561a7, record: `{"size":[36,55],"cursor":[25,54],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[12,26],"operation":"/tmp/cafleet-194/opencode-compact-cells-055/\npermission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-compact-permission-055-1"}`},
	{fixture: "opencode-2.1.4-compact-deny-pre-stage-2-055-1", screen: 0xcd413732b9c699bb, record: `{"size":[36,55],"cursor":[25,36],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[12,26],"operation":"/tmp/cafleet-194/opencode-compact-deny-002-055/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-compact-deny-pre-stage-2-055-1"}`},
	{fixture: "opencode-2.1.4-compact-deny-pre-stage-3-055-1", screen: 0x797d61ea23b0e9fc, record: `{"size":[36,55],"cursor":[33,0],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,35],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.1.4-narrow-permission-min-1", screen: 0xc5305540d59e135b, record: `{"size":[36,36],"cursor":[35,35],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,36],"operation":"/tmp/cafleet-194/opencode-narrow-\ncells-min/permission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-narrow-permission-min-1"}`},
	{fixture: "opencode-2.1.4-narrow-deny-pre-stage-2-min-1", screen: 0xc8779581568761c8, record: `{"size":[36,36],"cursor":[27,35],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,29],"operation":"/tmp/cafleet-194/opencode-narrow-\ndeny-002-min/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.1.4-narrow-deny-pre-stage-2-min-1"}`},
	{fixture: "opencode-2.1.4-narrow-deny-pre-stage-3-min-1", screen: 0x36c2dfe1bf16ce11, record: `{"size":[36,36],"cursor":[34,0],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[22,36],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.2.5-composer-prompt-idle-mode-2004-tapped-v2-herdr-119-1", screen: 0xb6d8cea7b9252e13, record: `{"size":[40,119],"cursor":[32,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,35],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.5-composer-prompt-idle-mode-2004-tapped-v2-tmux-120-1", screen: 0x7573fd978d11efc2, record: `{"size":[39,120],"cursor":[31,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68", screen: 0x31eab8af1b3130eb, record: `{"size":[40,119],"cursor":[32,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,34],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68", screen: 0x2ed1846030255081, record: `{"size":[39,120],"cursor":[31,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33", screen: 0xf9b3c95282baa653, record: `{"size":[40,119],"cursor":[32,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,34],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33", screen: 0xdfe078e4753b7e54, record: `{"size":[39,120],"cursor":[31,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-suggestion-tmux-120-1", screen: 0x2bae3a57795f7cc0, record: `{"size":[39,120],"cursor":[5,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[5,8],"operation":null,"choices":[],"composer_text":{"region":[4,7],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask anything, / for commands, @ for context…","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-shell-mode-tmux-120-1", screen: 0xeac0e01709759df3, record: `{"size":[39,120],"cursor":[5,2],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[4,7],"rows":1,"mode_prefix":"!","segments":[{"text":"Run a command… \"git status\"","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-1-tmux-120-1", screen: 0x66e6042897e25fbe, record: `{"size":[39,120],"cursor":[23,119],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,23],"operation":"/tmp/cafleet-programmer-research/oc-perm/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-2-tmux-120-1", screen: 0xff99f7a99909db31, record: `{"size":[39,120],"cursor":[22,38],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,23],"operation":"/tmp/cafleet-programmer-research/oc-perm/permission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-3-tmux-120-1", screen: 0x436f87298b8304be, record: `{"size":[39,120],"cursor":[22,2],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[11,23],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.2.7-deny-post-idle-tmux-120-1", screen: 0xd19347a9f6c3dc9b, record: `{"size":[39,120],"cursor":[14,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[14,17],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-1-tmux-057-1", screen: 0x2d0ab5190a0dd3cf, record: `{"size":[36,57],"cursor":[26,56],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,26],"operation":"/tmp/cafleet-programmer-research/oc-perm/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-057-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-2-tmux-057-1", screen: 0xff1bd2f3d9dce1ea, record: `{"size":[36,57],"cursor":[23,38],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,26],"operation":"/tmp/cafleet-programmer-research/oc-perm/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-057-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-3-tmux-057-1", screen: 0x1154b069d1e2b8ce, record: `{"size":[36,57],"cursor":[23,2],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,26],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.2.7-deny-post-idle-tmux-057-1", screen: 0xb0b99e527748269d, record: `{"size":[36,57],"cursor":[18,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[18,21],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-1-tmux-055-1", screen: 0x72e2ec861c5b703b, record: `{"size":[36,55],"cursor":[27,54],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,28],"operation":"/tmp/cafleet-programmer-research/oc-perm/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-057-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-2-tmux-055-1", screen: 0x72216eb5f33cec57, record: `{"size":[36,55],"cursor":[27,36],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,28],"operation":"/tmp/cafleet-programmer-research/oc-perm/\npermission-probe deny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-057-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-3-tmux-055-1", screen: 0xa728fa467269ea19, record: `{"size":[36,55],"cursor":[26,0],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[14,28],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.2.7-deny-post-idle-tmux-055-1", screen: 0x00129daa6ce91e70, record: `{"size":[36,55],"cursor":[18,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[18,21],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-1-tmux-036-1", screen: 0x8eeef21f75380fa5, record: `{"size":[36,36],"cursor":[28,35],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,29],"operation":"/tmp/cafleet-programmer-\nresearch/oc-perm/permission-probe\ndeny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-036-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-2-tmux-036-1", screen: 0xe3b40edf869def86, record: `{"size":[36,36],"cursor":[27,35],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,29],"operation":"/tmp/cafleet-programmer-\nresearch/oc-perm/permission-probe\ndeny","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-deny-pre-stage-1-tmux-036-1"}`},
	{fixture: "opencode-2.2.7-deny-pre-stage-3-tmux-036-1", screen: 0x423fd4f2d31be52a, record: `{"size":[36,36],"cursor":[27,0],"alternate_screen":false,"kind":"PermissionContinuation","rule_id":"opencode-reject-reason-compact","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[15,29],"operation":null,"choices":[],"composer_text":null,"episode":"opencode-2.1.4-deny-pre-stage-3"}`},
	{fixture: "opencode-2.2.7-deny-post-idle-tmux-036-1", screen: 0xca04e272f47060ca, record: `{"size":[36,36],"cursor":[20,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[20,23],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-allow-pre-stage-tmux-120-1", screen: 0xb493c297b5dac95f, record: `{"size":[39,120],"cursor":[32,119],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"opencode-shell-permission","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,32],"operation":"/tmp/cafleet-programmer-research/oc-perm/permission-probe allow","choices":["Allow once","Reject"],"composer_text":null,"episode":"opencode-2.2.7-allow-pre-stage-tmux-120-1"}`},
	{fixture: "opencode-2.2.7-allow-post-idle-tmux-120-1", screen: 0xa01a42c001a08f68, record: `{"size":[39,120],"cursor":[27,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[27,30],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-41-literal-tmux-120-1", screen: 0x23b3b124d1462c1d, record: `{"size":[39,120],"cursor":[14,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[13,16],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-41-post-submit-tmux-120-1", screen: 0xdaf033d8bfdca401, record: `{"size":[39,120],"cursor":[16,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[16,19],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-4000-post-submit-tmux-120-1", screen: 0x5ae19b607fa3be73, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,39],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-41-literal-tmux-120-1", screen: 0x94696d911624e8a6, record: `{"size":[39,120],"cursor":[36,44],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-41-post-submit-tmux-120-1", screen: 0xe1f2cd559174cb9a, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,39],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-4000-post-submit-tmux-120-1", screen: 0x51ca687ed32d0a3e, record: `{"size":[39,120],"cursor":[31,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,34],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-41-literal-herdr-091-1", screen: 0x7689af5d9ac8bf4c, record: `{"size":[36,91],"cursor":[14,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[13,16],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-41-post-submit-herdr-091-1", screen: 0xcd40c3c6758c4bd1, record: `{"size":[36,91],"cursor":[16,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[16,19],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-prompt-plain-4000-post-submit-herdr-091-1", screen: 0x087860650e956b42, record: `{"size":[36,91],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-41-literal-herdr-091-1", screen: 0x999bcaf4a59190bb, record: `{"size":[36,91],"cursor":[33,44],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,35],"rows":1,"mode_prefix":"!","segments":[{"text":" true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-41-post-submit-herdr-091-1", screen: 0xb7dca7a53725463e, record: `{"size":[36,91],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.2.7-composer-exec-plain-4000-post-submit-herdr-091-1", screen: 0x5834bd994e30c377, record: `{"size":[36,91],"cursor":[28,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[28,31],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.3.0-director-idle-empty-tmux-120-1", screen: 0x507761e18c864f1b, record: `{"size":[39,120],"cursor":[3,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[3,6],"operation":null,"choices":[],"composer_text":{"region":[2,5],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask anything, / for commands, @ for context…","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-one-tmux-120-1", screen: 0x9fcbf61a325a27b8, record: `{"size":[39,120],"cursor":[3,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,5],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-wrap-tmux-120-1", screen: 0x382fd411ade67128, record: `{"size":[39,120],"cursor":[5,86],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,7],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,125]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-busy-tmux-120-1", screen: 0x6b86000c8205b737, record: `{"size":[39,120],"cursor":[5,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[5,8],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.3.0-director-idle-empty-herdr-120-1", screen: 0x54ba395055ac844c, record: `{"size":[7,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"opencode-suggestion-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[4,7],"operation":null,"choices":[],"composer_text":{"region":[3,6],"rows":1,"mode_prefix":"!","segments":[{"text":"Ask anything, / for commands, @ for context…","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-one-herdr-120-1", screen: 0x86a1ab490a84003a, record: `{"size":[7,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,6],"rows":1,"mode_prefix":"!","segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-wrap-herdr-120-1", screen: 0x6e7e2d83cc6028bb, record: `{"size":[9,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,8],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,124]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-busy-herdr-120-1", screen: 0x8941fcd51c4fca62, record: `{"size":[9,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"opencode-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[6,9],"operation":null,"choices":[],"composer_text":null,"episode":null}`},
	{fixture: "opencode-2.3.0-composer-prompt-wrap-literal-tmux-120-1", screen: 0x8dccec30d1e5dc75, record: `{"size":[39,120],"cursor":[5,86],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,7],"rows":3,"mode_prefix":"!","segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,125]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-composer-exec-wrap-literal-tmux-120-1", screen: 0x63e7794423243095, record: `{"size":[39,120],"cursor":[5,81],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,7],"rows":3,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,121]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-composer-prompt-plain-380-literal-tmux-80-1", screen: 0x2bd4b37e1bbaf976, record: `{"size":[39,80],"cursor":[7,78],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,9],"rows":5,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-composer-prompt-plain-380-literal-herdr-82-1", screen: 0x059d053a4cd48b51, record: `{"size":[11,82],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,10],"rows":5,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-composer-exec-plain-380-literal-tmux-80-1", screen: 0x1b8e5a385bd315b6, record: `{"size":[39,80],"cursor":[8,71],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,10],"rows":6,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,81,157,233,309]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-composer-exec-plain-380-literal-herdr-82-1", screen: 0x05052d64ed36d361, record: `{"size":[12,82],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,11],"rows":6,"mode_prefix":"!","segments":[{"text":" trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[5,81,157,233,309]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-plain-380-tmux-80-1", screen: 0x41b8a758e553fef9, record: `{"size":[39,80],"cursor":[7,78],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[2,9],"rows":5,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-plain-380-herdr-82-1", screen: 0x42d5b57a114b142a, record: `{"size":[11,82],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,10],"rows":5,"mode_prefix":"!","segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[76,152,228,304]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-preview-after-turn-tmux-120-1", screen: 0x11b4d342747e97f7, record: `{"size":[39,120],"cursor":[36,108],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,38],"rows":2,"mode_prefix":"!","segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change in src/parser.rsand reply with your findings when you are done. [cafleet] Resume your work if something was still running.","breaks":[114]}]},"episode":null}`},
	{fixture: "opencode-2.3.0-director-preview-after-turn-herdr-120-1", screen: 0x653fe2379dc7dfba, record: `{"size":[13,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[8,12],"rows":2,"mode_prefix":"!","segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change in src/parser.rsand reply with your findings when you are done. [cafleet] Resume your work if something was still running.","breaks":[114]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-tmux-120-1", screen: 0x5b1366247e2a82a1, record: `{"size":[36,120],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,32],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[15,18],"rows":2,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[118]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-allow-continuation-tmux-120-1", screen: 0xf150a635a14897de, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Turn these notes into a to-do list","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-empty-tmux-120-1", screen: 0xf150a635a14897de, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Turn these notes into a to-do list","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-literal-tmux-120-1", screen: 0x4a4deed83bfde3df, record: `{"size":[36,120],"cursor":[33,32],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply with the single word ok.","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-submitted-tmux-120-1", screen: 0xa06a9f5066737156, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Turn these notes into a to-do list","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-literal-tmux-120-1", screen: 0x74d4c812317388f8, record: `{"size":[36,120],"cursor":[33,28],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"! echo cafleet-native-exec","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-submitted-tmux-120-1", screen: 0x273456983a5d2817, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Turn these notes into a to-do list","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-tmux-080-1", screen: 0x5fec15ecc35945e6, record: `{"size":[36,80],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,32],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[14,18],"rows":3,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeescalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[77,157]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-deny-continuation-tmux-080-1", screen: 0x275dcc127e4e815e, record: `{"size":[36,80],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Research this topic and write me a brief","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-tmux-057-1", screen: 0x4d63867b95b93bb9, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,33],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[13,19],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-deny-focus-tmux-057-1", screen: 0xf5010b6d81131777, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,33],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[13,19],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-deny-enter-continuation-tmux-057-1", screen: 0x916ce3a7ec7a30e4, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":2,"mode_prefix":null,"segments":[{"text":"Set a reminder or schedule a recurring task","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-tmux-040-1", screen: 0xcf3f3d310d56798a, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,33],"operation":"bash -c\n'/tmp/chp/forms/permission-probe\nescalate'","choices":["Allow once","Deny"],"composer_text":{"region":[10,17],"rows":6,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeescalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[38,77,117,157,197]}]},"episode":"hermes-2026.9.24-escalate-tmux-040-1"}`},
	{fixture: "hermes-2026.9.24-escalate-allow-continuation-tmux-040-1", screen: 0x5d45da635a53399e, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":2,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P f","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-min-tmux-037-1", screen: 0x3115eee8477017cc, record: `{"size":[36,37],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,33],"operation":"bash -c\n'/tmp/chp/forms/permission-\nprobe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[8,16],"rows":7,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[35,72,109,146,183,220]}]},"episode":"hermes-2026.9.24-escalate-min-tmux-037-1"}`},
	{fixture: "hermes-2026.9.24-escalate-survey-tmux-036-1", screen: 0x32657559ab190ab3, record: `{"size":[36,36],"cursor":[34,2],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[7,15],"rows":7,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foregroundcommand. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[34,70,105,141,177,213]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-timeout-closure-tmux-120-1", screen: 0x0633ffdc5dfadd2e, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Summarize what's in this folder","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-override-tmux-120-1", screen: 0x3fbc26cef30496d5, record: `{"size":[36,120],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,32],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[17,20],"rows":2,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[118]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-allow-continuation-tmux-120-1", screen: 0x2146c9b22518ca81, record: `{"size":[36,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Plan a feature, then build it step by step","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-override-tmux-080-1", screen: 0x0dc63287d6400dd5, record: `{"size":[36,80],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,32],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[16,20],"rows":3,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeoverride' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[77,157]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-deny-continuation-tmux-080-1", screen: 0xd627444a4104a9a7, record: `{"size":[36,80],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Summarize what's in this folder","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-override-tmux-057-1", screen: 0xdfcc45fad92e508f, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,33],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[15,21],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-deny-focus-tmux-057-1", screen: 0xf62450f9347281d5, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,33],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[15,21],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-deny-enter-continuation-tmux-057-1", screen: 0x04eddc535c911346, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":2,"mode_prefix":null,"segments":[{"text":"Plan a feature, then build it step by step","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-override-tmux-040-1", screen: 0x6bbdb0081b6eaa3f, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,33],"operation":"bash -c\n'/tmp/chp/forms/permission-probe\noverride'","choices":["Allow once","Deny"],"composer_text":{"region":[12,19],"rows":6,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeoverride' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[38,77,117,157,197]}]},"episode":"hermes-2026.9.24-override-tmux-040-1"}`},
	{fixture: "hermes-2026.9.24-override-allow-enter-continuation-tmux-040-1", screen: 0xd6060fc50a144044, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":2,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-override-survey-tmux-028-1", screen: 0x1ae39063e3af226f, record: `{"size":[36,28],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,33],"operation":"bash -c '/tmp/chp/forms/\npermission-probe\noverride'","choices":["Allow once","Deny"],"composer_text":{"region":[7,17],"rows":9,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual resultand stop.","breaks":[26,54,82,110,138,166,194,221]}]},"episode":"hermes-2026.9.24-override-survey-tmux-028-1"}`},
	{fixture: "hermes-2026.9.24-override-survey-tmux-027-1", screen: 0x26a40d5bcfa5b957, record: `{"size":[36,27],"cursor":[34,2],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-escalate-herdr-120-1", screen: 0xbe18ed65714c4064, record: `{"size":[36,120],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,32],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[15,18],"rows":2,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[118]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-herdr-120-1", screen: 0x394be3dceb62c13d, record: `{"size":[36,120],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,32],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[17,20],"rows":2,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[118]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-herdr-080-1", screen: 0xfc3335dde1525c7d, record: `{"size":[36,80],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[18,32],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[14,18],"rows":3,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeescalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[77,157]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-herdr-080-1", screen: 0x71001378c0822c7c, record: `{"size":[36,80],"cursor":[33,4],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[20,32],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[16,20],"rows":3,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeoverride' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[77,157]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-herdr-057-1", screen: 0x234542a879d25ff7, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,33],"operation":"bash -c '/tmp/chp/forms/permission-probe escalate'","choices":["Allow once","Deny"],"composer_text":{"region":[13,19],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe escalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-escalate-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-override-herdr-057-1", screen: 0xb3da5d21c1c39b7e, record: `{"size":[36,57],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[21,33],"operation":"bash -c '/tmp/chp/forms/permission-probe override'","choices":["Allow once","Deny"],"composer_text":{"region":[15,21],"rows":5,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probe override' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result andstop.","breaks":[55,112,169,225]}]},"episode":"hermes-2026.9.24-override-tmux-120-1"}`},
	{fixture: "hermes-2026.9.24-escalate-herdr-040-1", screen: 0x53f03c62ee1ff654, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-escalate","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[17,33],"operation":"bash -c\n'/tmp/chp/forms/permission-probe\nescalate'","choices":["Allow once","Deny"],"composer_text":{"region":[10,17],"rows":6,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeescalate' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[38,77,117,157,197]}]},"episode":"hermes-2026.9.24-escalate-tmux-040-1"}`},
	{fixture: "hermes-2026.9.24-override-herdr-040-1", screen: 0xc85ee41166dfc68c, record: `{"size":[36,40],"cursor":[34,2],"alternate_screen":false,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-approval-override","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[19,33],"operation":"bash -c\n'/tmp/chp/forms/permission-probe\noverride'","choices":["Allow once","Deny"],"composer_text":{"region":[12,19],"rows":6,"mode_prefix":null,"segments":[{"text":"Use the terminal tool to run exactly bash -c '/tmp/chp/forms/permission-probeoverride' as one foreground command. Preserve any native approval request for the operator. After the native decision, report the actual result and stop.","breaks":[38,77,117,157,197]}]},"episode":"hermes-2026.9.24-override-tmux-040-1"}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-tmux-120-15", screen: 0x8ee043840e2679da, record: `{"size":[39,120],"cursor":[35,71],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,37],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-herdr-119-15", screen: 0xbfb9dfa4179174bf, record: `{"size":[40,119],"cursor":[37,71],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-tmux-120-56", screen: 0x38966185048db259, record: `{"size":[39,120],"cursor":[36,34],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-herdr-119-53", screen: 0xee62e5157c3d079d, record: `{"size":[40,119],"cursor":[37,102],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-final-mode-2004-tapped-v2-herdr-119-102", screen: 0x265b18efc74d4e31, record: `{"size":[40,119],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-final-mode-2004-tapped-v2-tmux-120-103", screen: 0x5c5be1f0a988c72c, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-2057-args-literal-tapped-v2-herdr-119-65", screen: 0x4c051fdd564174c5, record: `{"size":[40,119],"cursor":[31,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-2057-args-literal-tapped-v2-tmux-120-66", screen: 0x7a917d24b875250d, record: `{"size":[39,120],"cursor":[30,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,32],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-2057-args-post-1900ms-tapped-v2-herdr-119-69", screen: 0xc1a5b245fcc63cb1, record: `{"size":[40,119],"cursor":[32,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-2057-args-post-1900ms-tapped-v2-tmux-120-70", screen: 0x4ab34ed87f5a6bad, record: `{"size":[39,120],"cursor":[31,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":{"region":[31,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-40-args-literal-tapped-v2-herdr-119-59", screen: 0x016cf0e5b6573ab6, record: `{"size":[40,119],"cursor":[30,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-40-args-literal-tapped-v2-tmux-120-60", screen: 0x1d4edb077352979c, record: `{"size":[39,120],"cursor":[29,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-40-args-post-1900ms-tapped-v2-herdr-119-63", screen: 0x35284ea42ee9b04e, record: `{"size":[40,119],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-40-args-post-1900ms-tapped-v2-tmux-120-64", screen: 0x8464b3da35528a31, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-execution-proof-literal-tapped-v2-herdr-119-2", screen: 0x01bafd75c9427022, record: `{"size":[40,119],"cursor":[31,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-execution-proof-literal-tapped-v2-tmux-120-2", screen: 0x2f895c085f942926, record: `{"size":[39,120],"cursor":[30,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,32],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-execution-proof-post-1900ms-tapped-v2-herdr-119-6", screen: 0x5e6e61a9b556c340, record: `{"size":[40,119],"cursor":[32,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-execution-proof-post-1900ms-tapped-v2-tmux-120-6", screen: 0xf2554190169e0dac, record: `{"size":[39,120],"cursor":[31,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":{"region":[31,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-literal-tapped-v2-herdr-119-71", screen: 0x628351e09bd5a748, record: `{"size":[40,119],"cursor":[33,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,35],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-literal-tapped-v2-tmux-120-72", screen: 0xf6fe435c90c92fdb, record: `{"size":[39,120],"cursor":[32,67],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,34],"rows":2,"mode_prefix":null,"segments":[{"text":"!","breaks":[1]},{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-75", screen: 0xfef98c012fd0905e, record: `{"size":[40,119],"cursor":[34,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-76", screen: 0xfcd56813f04f799f, record: `{"size":[39,120],"cursor":[33,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,34],"operation":null,"choices":[],"composer_text":{"region":[33,35],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-execution-proof-literal-tapped-v2-herdr-119-2", screen: 0x983d2b7426db223c, record: `{"size":[40,119],"cursor":[37,104],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-execution-proof-literal-tapped-v2-tmux-120-2", screen: 0x28001f8882bd1346, record: `{"size":[39,120],"cursor":[36,74],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-execution-proof-post-1900ms-tapped-v2-herdr-119-6", screen: 0x8e35c3e217bb8ef5, record: `{"size":[40,119],"cursor":[31,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[31,32],"operation":null,"choices":[],"composer_text":{"region":[31,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-execution-proof-post-1900ms-tapped-v2-tmux-120-6", screen: 0x623db36f81e10c23, record: `{"size":[39,120],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-literal-tapped-v2-herdr-119-77", screen: 0xb1308548fc632bd0, record: `{"size":[40,119],"cursor":[37,36],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-literal-tapped-v2-tmux-120-78", screen: 0xb58654bd328aa7f0, record: `{"size":[39,120],"cursor":[36,4],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-post-1900ms-tapped-v2-herdr-119-81", screen: 0x5602d200d59797d9, record: `{"size":[40,119],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-1024-ja-args-post-1900ms-tapped-v2-tmux-120-82", screen: 0xd3de60040fa4389a, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-2048-ja-args-literal-tapped-v2-herdr-119-84", screen: 0xec347ecf6bded9d6, record: `{"size":[40,119],"cursor":[37,112],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-2048-ja-args-literal-tapped-v2-tmux-120-85", screen: 0xf57597fe7e989e05, record: `{"size":[39,120],"cursor":[36,12],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-2048-ja-args-post-1900ms-tapped-v2-herdr-119-88", screen: 0x5973d1e861994486, record: `{"size":[40,119],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-2048-ja-args-post-1900ms-tapped-v2-tmux-120-89", screen: 0x451131b4de64846c, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-40-args-literal-tapped-v2-herdr-119-53", screen: 0x473a29ea2a0ad188, record: `{"size":[40,119],"cursor":[30,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-40-args-literal-tapped-v2-tmux-120-54", screen: 0x47318b3d8b8484bd, record: `{"size":[39,120],"cursor":[29,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-40-args-post-1900ms-tapped-v2-herdr-119-57", screen: 0x2e1ebfffbbc9fb51, record: `{"size":[40,119],"cursor":[30,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,31],"operation":null,"choices":[],"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-40-args-post-1900ms-tapped-v2-tmux-120-58", screen: 0xe712a57f9d7de5c7, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-literal-tapped-v2-herdr-119-91", screen: 0x0841252b350c305e, record: `{"size":[40,119],"cursor":[37,46],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-literal-tapped-v2-tmux-120-92", screen: 0xcc459d17b399dd0c, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-95", screen: 0xf4cc19c44b523c7d, record: `{"size":[40,119],"cursor":[30,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-96", screen: 0x0838765c2fe9f767, record: `{"size":[39,120],"cursor":[29,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-99", screen: 0xabd91d10d941c269, record: `{"size":[40,119],"cursor":[30,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-100", screen: 0x8e955fdaacf21e57, record: `{"size":[39,120],"cursor":[29,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-unsubmitted-literal-tapped-v2-herdr-119-100", screen: 0xabd91d10d941c269, record: `{"size":[40,119],"cursor":[30,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-unsubmitted-literal-tapped-v2-tmux-120-101", screen: 0x8e955fdaacf21e57, record: `{"size":[39,120],"cursor":[29,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-idle-mode-2004-tapped-v2-herdr-119-1", screen: 0xda5eb7a3da4a5d75, record: `{"size":[40,119],"cursor":[37,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"Type / to browse commands, or Ctrl+P for the palette","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-idle-mode-2004-tapped-v2-tmux-120-1", screen: 0x8ef53a3e43bb1402, record: `{"size":[39,120],"cursor":[29,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[29,30],"operation":null,"choices":[],"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Explain this error and how to fix it","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2057-post-1900ms-tapped-v2-herdr-119-19", screen: 0x9dd0d2bec124a8ac, record: `{"size":[40,119],"cursor":[37,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2057-post-1900ms-tapped-v2-tmux-120-19", screen: 0xc67cca6e15a61cf3, record: `{"size":[39,120],"cursor":[36,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-40-literal-tapped-v2-herdr-119-9", screen: 0x47ef9086a6d3689c, record: `{"size":[40,119],"cursor":[37,44],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-40-literal-tapped-v2-tmux-120-9", screen: 0xedee332e9915934c, record: `{"size":[39,120],"cursor":[34,44],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,36],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-40-post-1900ms-tapped-v2-herdr-119-13", screen: 0xca73c39523be818c, record: `{"size":[40,119],"cursor":[37,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-40-post-1900ms-tapped-v2-tmux-120-13", screen: 0x9903802c10bbc677, record: `{"size":[39,120],"cursor":[35,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,36],"operation":null,"choices":[],"composer_text":{"region":[35,37],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-4000-max-preview-chunks-literal-tapped-v2-herdr-119-21", screen: 0xeb2a6801c0c4c903, record: `{"size":[40,119],"cursor":[37,71],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-4000-max-preview-chunks-literal-tapped-v2-tmux-120-21", screen: 0x67ba1e9178905951, record: `{"size":[39,120],"cursor":[36,71],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-25", screen: 0xe6738583003c98c3, record: `{"size":[40,119],"cursor":[37,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-25", screen: 0x5e60a42acafe00bd, record: `{"size":[39,120],"cursor":[36,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-1024-ja-literal-tapped-v2-herdr-119-27", screen: 0x13d9babe56f6f706, record: `{"size":[40,119],"cursor":[37,36],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[24,26],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-1024-ja-literal-tapped-v2-tmux-120-27", screen: 0x369b0102999378f4, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[23,25],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-1024-ja-post-1900ms-tapped-v2-herdr-119-31", screen: 0x59f8d32a2eb1e023, record: `{"size":[40,119],"cursor":[37,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-1024-ja-post-1900ms-tapped-v2-tmux-120-31", screen: 0x1b4c03c0bda8a73a, record: `{"size":[39,120],"cursor":[36,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-2048-ja-literal-tapped-v2-herdr-119-34", screen: 0x39dcbd66dec322e1, record: `{"size":[40,119],"cursor":[37,78],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[10,29],"rows":18,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[63,122,181,240,299,358,417,476,535,594,653,712,771,830,889,948,1007]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-2048-ja-literal-tapped-v2-tmux-120-34", screen: 0xfb6909414bde0c18, record: `{"size":[39,120],"cursor":[36,52],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[10,28],"rows":17,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[64,124,184,244,304,364,424,484,544,604,664,724,784,844,904,964]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-2048-ja-post-1900ms-tapped-v2-herdr-119-38", screen: 0xf81592cf0a5186c0, record: `{"size":[40,119],"cursor":[32,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[32,33],"operation":null,"choices":[],"composer_text":{"region":[32,34],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-2048-ja-post-1900ms-tapped-v2-tmux-120-38", screen: 0xdf0b1e9d8699ba2f, record: `{"size":[39,120],"cursor":[29,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,21],"rows":17,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[64,124,184,244,304,364,424,484,544,604,664,724,784,844,904,964]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-40-literal-tapped-v2-herdr-119-3", screen: 0x691445d1342f3bb2, record: `{"size":[40,119],"cursor":[37,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaああああああああああああああああ","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-40-literal-tapped-v2-tmux-120-3", screen: 0xf30146ac1432df42, record: `{"size":[39,120],"cursor":[29,42],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaああああああああああああああああ","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-herdr-119-7", screen: 0xc9846a8657cc5caf, record: `{"size":[40,119],"cursor":[37,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[37,38],"operation":null,"choices":[],"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-tmux-120-7", screen: 0x7f8eeafd58a6e849, record: `{"size":[39,120],"cursor":[34,4],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,35],"operation":null,"choices":[],"composer_text":{"region":[34,36],"rows":1,"mode_prefix":null,"segments":[{"text":"type to redirect this turn · Tab queues it for after","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-literal-tapped-v2-herdr-119-41", screen: 0xd922a4a31a96716e, record: `{"size":[40,119],"cursor":[37,90],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[8,27],"rows":18,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[63,122,181,240,299,358,417,476,535,594,653,712,771,830,889,948,1007]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-literal-tapped-v2-tmux-120-42", screen: 0x314051af5bcd5ccc, record: `{"size":[39,120],"cursor":[36,10],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,21],"rows":17,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[64,124,184,244,304,364,424,484,544,604,664,724,784,844,904,964]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-45", screen: 0x292693477365da15, record: `{"size":[40,119],"cursor":[30,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[1,20],"rows":18,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[63,122,181,240,299,358,417,476,535,594,653,712,771,830,889,948,1007]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-46", screen: 0x7f39142b60c74fc4, record: `{"size":[39,120],"cursor":[29,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,21],"rows":17,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[64,124,184,244,304,364,424,484,544,604,664,724,784,844,904,964]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-unsubmitted-literal-tapped-v2-herdr-119-50", screen: 0xaffc11e58da40d85, record: `{"size":[40,119],"cursor":[30,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,32],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaああああああああああああああああ","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-unsubmitted-literal-tapped-v2-tmux-120-51", screen: 0x2833caef482264d9, record: `{"size":[39,120],"cursor":[29,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaああああああああああああああああ","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-tmux-120-44", screen: 0x1dcd1cf540fdd95e, record: `{"size":[39,120],"cursor":[36,100],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,21],"rows":17,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[64,124,184,244,304,364,424,484,544,604,664,724,784,844,904,964]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-herdr-119-43", screen: 0x61d95382f9c13814, record: `{"size":[40,119],"cursor":[37,106],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[5,24],"rows":18,"mode_prefix":null,"segments":[{"text":"Reply OK. ああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああああ","breaks":[63,122,181,240,299,358,417,476,535,594,653,712,771,830,889,948,1007]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-tmux-120-94", screen: 0xcd6d997c6f712c24, record: `{"size":[39,120],"cursor":[36,92],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-herdr-119-93", screen: 0x7db235ad958ee129, record: `{"size":[40,119],"cursor":[37,112],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-suggestion-tmux-120-1", screen: 0x36390e42310993f7, record: `{"size":[39,120],"cursor":[36,2],"alternate_screen":false,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,37],"operation":null,"choices":[],"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"text":"What changed in this repo recently?","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-approval-herdr-120-1", screen: 0x44513615b7ef70a2, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-tui-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[27,36],"operation":null,"choices":[],"composer_text":null,"episode":"hermes-2026.9.24-director-approval-herdr-120-1"}`},
	{fixture: "hermes-2026.9.24-director-busy-herdr-120-1", screen: 0x81e7f8e6c24186a8, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-director-draft-herdr-120-1", screen: 0xe795fdd16eebfde0, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[38,40],"rows":1,"mode_prefix":null,"segments":[{"text":"draft text","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-idle-empty-herdr-120-1", screen: 0x7a75660c0fce4c88, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[38,40],"rows":1,"mode_prefix":null,"segments":[{"text":"Ask me anything…","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-one-herdr-120-1", screen: 0x3f847354c368f382, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[38,40],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-plain-1024-herdr-120-1", screen: 0x8ca54d51d7e72a2c, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,40],"rows":10,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[113,226,339,452,565,678,791,904,1017]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-wrap-herdr-120-1", screen: 0xc67536b34aefeaa7, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,40],"rows":2,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[113]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-approval-tmux-120-1", screen: 0x39e5fe8f06649227, record: `{"size":[39,120],"cursor":[38,0],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"hermes-tui-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[26,35],"operation":null,"choices":[],"composer_text":null,"episode":"hermes-2026.9.24-director-approval-herdr-120-1"}`},
	{fixture: "hermes-2026.9.24-director-busy-tmux-120-1", screen: 0xe011f1c00ee4bb4a, record: `{"size":[39,120],"cursor":[38,3],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "hermes-2026.9.24-director-draft-tmux-120-1", screen: 0x135f9e9dd5f1d337, record: `{"size":[39,120],"cursor":[38,13],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"draft text","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-idle-empty-tmux-120-1", screen: 0x099d4a09e2436072, record: `{"size":[39,120],"cursor":[38,0],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"Ask me anything…","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-one-tmux-120-1", screen: 0xaf0f9c5e3b93d9b2, record: `{"size":[39,120],"cursor":[38,44],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[37,39],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-plain-1024-tmux-120-1", screen: 0xd542a7be81116a64, record: `{"size":[39,120],"cursor":[38,115],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,39],"rows":9,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[114,228,342,456,570,684,798,912]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-wrap-tmux-120-1", screen: 0x4825d784f8d2d639, record: `{"size":[39,120],"cursor":[38,99],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,39],"rows":2,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[114]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-plain-1480-tmux-80-1", screen: 0xa235ac8ca4dbe476, record: `{"size":[24,80],"cursor":[23,77],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,24],"rows":20,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[74,148,222,296,370,444,518,592,666,740,814,888,962,1036,1110,1184,1258,1332,1406]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-plain-1480-herdr-81-1", screen: 0x68fb3adc9104e2c6, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[3,24],"rows":20,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[74,148,222,296,370,444,518,592,666,740,814,888,962,1036,1110,1184,1258,1332,1406]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-one-literal-tmux-120-1", screen: 0x467ddc3a2b3222e8, record: `{"size":[39,120],"cursor":[36,43],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,38],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-wrap-literal-tmux-120-1", screen: 0x6c46e7af24a9e461, record: `{"size":[39,120],"cursor":[36,92],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":2,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[118]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-narrow-wrap-literal-tmux-30-1", screen: 0x25445dcde8c72713, record: `{"size":[20,30],"cursor":[18,24],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[17,20],"rows":2,"mode_prefix":null,"segments":[{"text":"narrow draft 日本語テキストwith wide chars wrapping","breaks":[20]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-wrap-literal-tmux-120-1", screen: 0x5b5f4d1715d04491, record: `{"size":[39,120],"cursor":[30,84],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,32],"rows":2,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[118]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-558-literal-tmux-80-1", screen: 0x6a7a1a9907904ad4, record: `{"size":[24,80],"cursor":[14,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,23],"rows":7,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[78,158,238,318,398,478]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-plain-558-literal-herdr-81-1", screen: 0xecbc5f705f2c7466, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[15,16],"operation":null,"choices":[],"composer_text":{"region":[15,23],"rows":7,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[78,158,238,318,398,478]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-558-literal-tmux-80-1", screen: 0x8d23de1220683e26, record: `{"size":[24,80],"cursor":[14,0],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[15,23],"rows":7,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[78,158,238,318,398,478]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-exec-plain-558-literal-herdr-81-1", screen: 0xa7add6e02fa9b48f, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[15,16],"operation":null,"choices":[],"composer_text":{"region":[15,23],"rows":7,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[78,158,238,318,398,478]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2000-literal-tmux-120-1", screen: 0x8a57da2ae98bc926, record: `{"size":[39,120],"cursor":[29,78],"alternate_screen":false,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[29,31],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-composer-prompt-paste-2000-literal-herdr-81-1", screen: 0x64e7208338b3c9db, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"hermes-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[15,16],"operation":null,"choices":[],"composer_text":{"region":[15,17],"rows":1,"mode_prefix":null,"segments":[{"lines":1}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-preview-after-turn-tmux-120-1", screen: 0x275cfe14b05264f6, record: `{"size":[39,120],"cursor":[38,110],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,39],"rows":2,"mode_prefix":null,"segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change in src/parser.rs and reply with your findings when you are done. [cafleet] Resume your work if something was still running.","breaks":[114]}]},"episode":null}`},
	{fixture: "hermes-2026.9.24-director-preview-after-turn-herdr-120-1", screen: 0x492174a508d042d0, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,40],"rows":3,"mode_prefix":null,"segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change insrc/parser.rs and reply with your findings when you are done. [cafleet] Resume your work if something was stillrunning.","breaks":[100,211]}]},"episode":null}`},
	{fixture: "pi-1.5.0-allow-pre-stage-tmux-120-1", screen: 0xb7e3326f7e381a2f, record: `{"size":[36,120],"cursor":[34,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe allow","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "pi-1.5.0-allow-continuation-tmux-120-1", screen: 0xa7402bbef6a25177, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-deny-pre-stage-1-tmux-120-1", screen: 0xe737da28f9622087, record: `{"size":[36,120],"cursor":[34,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-pre-stage-2-tmux-120-1", screen: 0xa75465f18d313226, record: `{"size":[36,120],"cursor":[28,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":{"region":[21,33],"rows":10,"mode_prefix":null,"segments":[{"text":" CAFleet member gate: allow this bash command? $ ./permission-probe deny   Allow once → Deny ↑↓ navigate  enter select  escape/ctrl+c cancel","breaks":[0,46,72,85,92]}]},"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-continuation-tmux-120-1", screen: 0x98081818bbe6abd2, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-idle-composer-tmux-120-1", screen: 0xf8a8a26167af1db0, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-typed-literal-tmux-120-1", screen: 0x434a7b3b92789f63, record: `{"size":[36,120],"cursor":[31,32],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-post-submit-tmux-120-1", screen: 0xe6cda97e6abcec0d, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-empty-composer-tmux-120-1", screen: 0x00841f56f8b9e241, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-typed-literal-tmux-120-1", screen: 0xbb50baa5400224d3, record: `{"size":[36,120],"cursor":[31,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"! echo cafleet-220-exec","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-post-submit-tmux-120-1", screen: 0xff05f6383694cd0c, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-allow-pre-stage-tmux-049-1", screen: 0x25097e9a16ebaa05, record: `{"size":[36,49],"cursor":[34,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe allow","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "pi-1.5.0-allow-continuation-tmux-049-1", screen: 0xecf7aa07c63d64e8, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-deny-pre-stage-1-tmux-049-1", screen: 0x18ef1ca77fc653b3, record: `{"size":[36,49],"cursor":[34,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-pre-stage-2-tmux-049-1", screen: 0x8591d29f63062941, record: `{"size":[36,49],"cursor":[28,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":{"region":[21,33],"rows":10,"mode_prefix":null,"segments":[{"text":" CAFleet member gate: allow this bash command? $ ./permission-probe deny   Allow once → Deny ↑↓ navigate  enter select  escape/ctrl+c cancel","breaks":[0,46,72,85,92]}]},"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-continuation-tmux-049-1", screen: 0xfb09c27b78c7902d, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-idle-composer-tmux-049-1", screen: 0xa418d971e583ff17, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-typed-literal-tmux-049-1", screen: 0x7841ae72b6644b7b, record: `{"size":[36,49],"cursor":[31,32],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-post-submit-tmux-049-1", screen: 0xfa215bb55b912ce6, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-empty-composer-tmux-049-1", screen: 0x5ac2087dbbe8cbbe, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-typed-literal-tmux-049-1", screen: 0xcdc89d5f60d9b7b1, record: `{"size":[36,49],"cursor":[31,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"! echo cafleet-220-exec","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-post-submit-tmux-049-1", screen: 0xbe2fa1d6066dbb5c, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-allow-pre-stage-herdr-120-1", screen: 0x756cd2614785c51d, record: `{"size":[36,120],"cursor":[34,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe allow","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "pi-1.5.0-allow-continuation-herdr-120-1", screen: 0xbf6d886e1bcf174a, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-deny-pre-stage-1-herdr-120-1", screen: 0xc6d5f1e032ca2eb3, record: `{"size":[36,120],"cursor":[34,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-pre-stage-2-herdr-120-1", screen: 0xba54422581bd8bde, record: `{"size":[36,120],"cursor":[28,119],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":{"region":[21,33],"rows":10,"mode_prefix":null,"segments":[{"text":" CAFleet member gate: allow this bash command? $ ./permission-probe deny   Allow once → Deny ↑↓ navigate  enter select  escape/ctrl+c cancel","breaks":[0,46,72,85,92]}]},"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-continuation-herdr-120-1", screen: 0x20c9a519f947a1a3, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-idle-composer-herdr-120-1", screen: 0xa34a1879193ebaf8, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-typed-literal-herdr-120-1", screen: 0x7e2102a9f317b2ad, record: `{"size":[36,120],"cursor":[31,32],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-post-submit-herdr-120-1", screen: 0x6b8697dd85c15147, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-empty-composer-herdr-120-1", screen: 0xa143fe712551284b, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-typed-literal-herdr-120-1", screen: 0x68b5a17989c785d7, record: `{"size":[36,120],"cursor":[31,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"! echo cafleet-220-exec","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-post-submit-herdr-120-1", screen: 0x4df9fbc1aea8ccc2, record: `{"size":[36,120],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-allow-pre-stage-herdr-049-1", screen: 0x5e51cd0c297a8231, record: `{"size":[36,49],"cursor":[34,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe allow","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-allow-pre-stage-tmux-120-1"}`},
	{fixture: "pi-1.5.0-allow-continuation-herdr-049-1", screen: 0x3a5ad25f9d6ba2ef, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-deny-pre-stage-1-herdr-049-1", screen: 0x54b9296cfbf27515, record: `{"size":[36,49],"cursor":[34,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":null,"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-pre-stage-2-herdr-049-1", screen: 0x511355864777df07, record: `{"size":[36,49],"cursor":[28,48],"alternate_screen":true,"kind":"Interaction(PermissionRequest)","rule_id":"pi-gate-approval","match_evidence":"Complete","verified_noninteraction":false,"matched_region":[23,31],"operation":"./permission-probe deny","choices":["Allow once","Deny"],"composer_text":{"region":[21,33],"rows":10,"mode_prefix":null,"segments":[{"text":" CAFleet member gate: allow this bash command? $ ./permission-probe deny   Allow once → Deny ↑↓ navigate  enter select  escape/ctrl+c cancel","breaks":[0,46,72,85,92]}]},"episode":"pi-1.5.0-deny-pre-stage-1-tmux-120-1"}`},
	{fixture: "pi-1.5.0-deny-continuation-herdr-049-1", screen: 0x96062094ab733e95, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-idle-composer-herdr-049-1", screen: 0xabcf9b3ab2f0de11, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-typed-literal-herdr-049-1", screen: 0xd4abea817616db23, record: `{"size":[36,49],"cursor":[31,32],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply with the single word done.","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-prompt-post-submit-herdr-049-1", screen: 0x4ab4e777ef4bf45b, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-empty-composer-herdr-049-1", screen: 0x05c7e8d9fb47c633, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-typed-literal-herdr-049-1", screen: 0xf654e3b5ca7ef77d, record: `{"size":[36,49],"cursor":[31,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[{"text":"! echo cafleet-220-exec","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-exec-post-submit-herdr-049-1", screen: 0xdcf38fdfda02ea68, record: `{"size":[36,49],"cursor":[31,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[30,33],"operation":null,"choices":[],"composer_text":{"region":[30,33],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-final-mode-2004-tapped-v2-herdr-119-74", screen: 0x1269c3db91413f1c, record: `{"size":[40,119],"cursor":[37,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,39],"operation":null,"choices":[],"composer_text":{"region":[36,39],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-final-mode-2004-tapped-v2-tmux-120-74", screen: 0x75e613a1647a5ff7, record: `{"size":[39,120],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-2049-args-literal-tapped-v2-herdr-119-51", screen: 0x4b4afd5971f88bb5, record: `{"size":[40,119],"cursor":[35,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"! [paste #1 2047 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-2049-args-literal-tapped-v2-tmux-120-51", screen: 0xbb73d065167e9c6e, record: `{"size":[39,120],"cursor":[34,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"! [paste #1 2047 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-2049-args-post-1900ms-tapped-v2-herdr-119-55", screen: 0x747c53a3acdd1e16, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-2049-args-post-1900ms-tapped-v2-tmux-120-55", screen: 0x7c6a2691f61cd901, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-40-args-literal-tapped-v2-herdr-119-45", screen: 0x3d37ace29561afc2, record: `{"size":[40,119],"cursor":[35,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-40-args-literal-tapped-v2-tmux-120-45", screen: 0x419aa606b43f8b21, record: `{"size":[39,120],"cursor":[34,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-40-args-post-1900ms-tapped-v2-herdr-119-49", screen: 0x106d0ce0ae300511, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-40-args-post-1900ms-tapped-v2-tmux-120-49", screen: 0xe507cfdabf9c6c80, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-4000-args-max-preview-chunks-literal-tapped-v2-herdr-119-63", screen: 0x097ca574ea9bef46, record: `{"size":[40,119],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-4000-args-max-preview-chunks-literal-tapped-v2-tmux-120-63", screen: 0x848c507a1db671d9, record: `{"size":[39,120],"cursor":[35,23],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"! [paste #1 4083 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-67", screen: 0xa0b6a1e2e821d0b6, record: `{"size":[40,119],"cursor":[37,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[36,39],"operation":null,"choices":[],"composer_text":{"region":[36,39],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-paste-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-67", screen: 0x8a52bb1bf46a7742, record: `{"size":[39,120],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-40-args-literal-tapped-v2-herdr-119-39", screen: 0x37f89ba510954987, record: `{"size":[40,119],"cursor":[35,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-40-args-literal-tapped-v2-tmux-120-39", screen: 0xc4940ebdce58413b, record: `{"size":[39,120],"cursor":[34,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-40-args-post-1900ms-tapped-v2-herdr-119-43", screen: 0x77008bb6834c31f5, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-40-args-post-1900ms-tapped-v2-tmux-120-43", screen: 0xed105ade5966d0a7, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-execution-proof-literal-tapped-v2-herdr-119-2", screen: 0x9fe50abe07ed31ee, record: `{"size":[40,119],"cursor":[35,62],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-execution-proof-literal-tapped-v2-tmux-120-2", screen: 0x131009c8e338ba55, record: `{"size":[39,120],"cursor":[34,60],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":null,"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-execution-proof-post-1900ms-tapped-v2-herdr-119-6", screen: 0xbb1622cac9857da8, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-execution-proof-post-1900ms-tapped-v2-tmux-120-6", screen: 0x807ca850f2b7a9dc, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-literal-tapped-v2-herdr-119-57", screen: 0xa3cb681a059df306, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-literal-tapped-v2-tmux-120-57", screen: 0x23b515a10d15323d, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-61", screen: 0xe1b44b4ac753c1eb, record: `{"size":[40,119],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-61", screen: 0xd73d227fb5ccb405, record: `{"size":[39,120],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-unsubmitted-literal-tapped-v2-herdr-119-70", screen: 0x14fdfa7b13b4ae36, record: `{"size":[40,119],"cursor":[37,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,39],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-unsubmitted-literal-tapped-v2-tmux-120-70", screen: 0x7a3b2f0832ebc34d, record: `{"size":[39,120],"cursor":[36,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-idle-mode-2004-tapped-v2-herdr-119-1", screen: 0x678280b8e4358346, record: `{"size":[40,119],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-idle-mode-2004-tapped-v2-tmux-120-1", screen: 0x3a2962b10b77a164, record: `{"size":[39,120],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-2049-literal-tapped-v2-herdr-119-15", screen: 0x2a469f5be12f5c7e, record: `{"size":[40,119],"cursor":[35,21],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"[paste #1 2049 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-2049-literal-tapped-v2-tmux-120-15", screen: 0x9cd851b4f78074d8, record: `{"size":[39,120],"cursor":[34,21],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"[paste #1 2049 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-2049-post-1900ms-tapped-v2-herdr-119-19", screen: 0xf33eae6951296840, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-2049-post-1900ms-tapped-v2-tmux-120-19", screen: 0x3f6383accd7e688e, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-40-literal-tapped-v2-herdr-119-9", screen: 0xcd2c57392a4e524d, record: `{"size":[40,119],"cursor":[35,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-40-literal-tapped-v2-tmux-120-9", screen: 0x517537f27b847733, record: `{"size":[39,120],"cursor":[34,40],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-40-post-1900ms-tapped-v2-herdr-119-13", screen: 0x39b24f7dc8e53438, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-40-post-1900ms-tapped-v2-tmux-120-13", screen: 0xc4ced37cec6beed3, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-4000-max-preview-chunks-literal-tapped-v2-herdr-119-27", screen: 0x9a87875f6e902cb9, record: `{"size":[40,119],"cursor":[35,21],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"[paste #1 4085 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-4000-max-preview-chunks-literal-tapped-v2-tmux-120-27", screen: 0x6e2da1f56e13c434, record: `{"size":[39,120],"cursor":[34,21],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"[paste #1 4085 chars]","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-31", screen: 0x9c37678af95ef600, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-paste-4000-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-31", screen: 0xdec0f47b88af3290, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-40-literal-tapped-v2-herdr-119-3", screen: 0xa1da3d3f96f72118, record: `{"size":[40,119],"cursor":[36,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-40-literal-tapped-v2-tmux-120-3", screen: 0xe85c80432ca3d384, record: `{"size":[39,120],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-40-post-1900ms-tapped-v2-herdr-119-7", screen: 0x65157fc4f8be54d3, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-40-post-1900ms-tapped-v2-tmux-120-7", screen: 0x546c7af5ba6e5e2e, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-4000-max-preview-chunks-literal-tapped-v2-herdr-119-21", screen: 0xa4d2176f441e2355, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-4000-max-preview-chunks-literal-tapped-v2-tmux-120-21", screen: 0x2a2fb8a413795d02, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-4000-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-25", screen: 0xc2a6142e3573be0c, record: `{"size":[40,119],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-4000-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-25", screen: 0x48709d165eaa60fa, record: `{"size":[39,120],"cursor":[34,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[33,36],"operation":null,"choices":[],"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-unsubmitted-literal-tapped-v2-herdr-119-34", screen: 0x572d35121d90694e, record: `{"size":[40,119],"cursor":[35,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-unsubmitted-literal-tapped-v2-tmux-120-34", screen: 0x42724fb6057f71e6, record: `{"size":[39,120],"cursor":[34,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-69", screen: 0x14fdfa7b13b4ae36, record: `{"size":[40,119],"cursor":[37,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[36,39],"rows":1,"mode_prefix":null,"segments":[{"text":"! true aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33", screen: 0x572d35121d90694e, record: `{"size":[40,119],"cursor":[35,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33", screen: 0x42724fb6057f71e6, record: `{"size":[39,120],"cursor":[34,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.0-composer-exec-retained-authoritative-no-submit-tapped-v2-tmux-120-2", screen: 0x0278dc35315590d4, record: `{"size":[39,120],"cursor":[34,49],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,36],"rows":1,"mode_prefix":null,"segments":[{"text":"! true CF228_RETAIN_exec_tmux_1794671923845862112","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-idle-empty-tmux-120-1", screen: 0x3958556d485b6fb6, record: `{"size":[39,120],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.2-director-one-tmux-120-1", screen: 0xa4385f35e1487a9c, record: `{"size":[39,120],"cursor":[35,41],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-wrap-tmux-120-1", screen: 0xc9824a49245ce4a8, record: `{"size":[39,120],"cursor":[35,81],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[32,37],"rows":3,"mode_prefix":null,"segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,128]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-busy-tmux-120-1", screen: 0x011e6c367eca7ba8, record: `{"size":[39,120],"cursor":[35,0],"alternate_screen":true,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[34,37],"operation":null,"choices":[],"composer_text":{"region":[34,37],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.2-director-idle-empty-herdr-120-1", screen: 0x60041cd0afdfb9f4, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.2-director-one-herdr-120-1", screen: 0x807faefc84d96120, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[{"text":"Reply OK. aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-wrap-herdr-120-1", screen: 0x2df2f7b388930c62, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,38],"rows":3,"mode_prefix":null,"segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,127]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-busy-herdr-120-1", screen: 0x0d3b50e5fa329919, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"pi-working-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[35,38],"operation":null,"choices":[],"composer_text":{"region":[35,38],"rows":1,"mode_prefix":null,"segments":[]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-prompt-wrap-literal-tmux-120-1", screen: 0x40e89b8ae23e78b1, record: `{"size":[39,120],"cursor":[34,81],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,36],"rows":3,"mode_prefix":null,"segments":[{"text":"Reply OK.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[9,128]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-exec-wrap-literal-tmux-120-1", screen: 0xcff6ffb0470a022d, record: `{"size":[39,120],"cursor":[34,76],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,36],"rows":3,"mode_prefix":null,"segments":[{"text":"! trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[6,125]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-prompt-words-literal-tmux-120-1", screen: 0xdfdfbacbed6496bf, record: `{"size":[39,120],"cursor":[34,10],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[31,36],"rows":3,"mode_prefix":null,"segments":[{"text":"Reply OK. abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghijabcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghij abcdefghijabcdefghij","breaks":[108,217]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-prompt-plain-474-literal-tmux-80-1", screen: 0x3d5d952cb24389fd, record: `{"size":[24,80],"cursor":[19,79],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[13,21],"rows":6,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[79,158,237,316,395]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-prompt-plain-474-literal-herdr-81-1", screen: 0xa852afb362c71e1c, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[11,14],"operation":null,"choices":[],"composer_text":{"region":[13,21],"rows":6,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[79,158,237,316,395]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-exec-plain-474-literal-tmux-80-1", screen: 0x8e1bf3bb8ca4a808, record: `{"size":[24,80],"cursor":[19,72],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[12,21],"rows":7,"mode_prefix":null,"segments":[{"text":"! trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[6,85,164,243,322,401]}]},"episode":null}`},
	{fixture: "pi-1.5.2-composer-exec-plain-474-literal-herdr-81-1", screen: 0xeeb84664556dee9c, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[10,13],"operation":null,"choices":[],"composer_text":{"region":[12,21],"rows":7,"mode_prefix":null,"segments":[{"text":"! trueaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[6,85,164,243,322,401]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-plain-474-tmux-80-1", screen: 0x0f5e48c1b54073a1, record: `{"size":[24,80],"cursor":[20,79],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[14,22],"rows":6,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[79,158,237,316,395]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-plain-474-herdr-81-1", screen: 0xf740273e290c111c, record: `{"size":[24,81],"cursor":null,"alternate_screen":null,"kind":"ComposerEmpty","rule_id":"pi-idle-composer","match_evidence":"Complete","verified_noninteraction":true,"matched_region":[12,15],"operation":null,"choices":[],"composer_text":{"region":[14,22],"rows":6,"mode_prefix":null,"segments":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","breaks":[79,158,237,316,395]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-preview-after-turn-tmux-120-1", screen: 0xde5761e9ca97bd72, record: `{"size":[39,120],"cursor":[35,102],"alternate_screen":true,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[33,37],"rows":2,"mode_prefix":null,"segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change in src/parser.rs andreply with your findings when you are done. [cafleet] Resume your work if something was still running.","breaks":[118]}]},"episode":null}`},
	{fixture: "pi-1.5.2-director-preview-after-turn-herdr-120-1", screen: 0xa6d296ba37e71ac8, record: `{"size":[40,120],"cursor":null,"alternate_screen":null,"kind":"Unknown","rule_id":"unmatched-screen","match_evidence":"Absent","verified_noninteraction":false,"matched_region":null,"operation":null,"choices":null,"composer_text":{"region":[34,38],"rows":2,"mode_prefix":null,"segments":[{"text":"[cafleet msg 101 from 2 2026-11-23T12:36:00.123456+00:00] Please review the updated parser change in src/parser.rsand reply with your findings when you are done. [cafleet] Resume your work if something was still running.","breaks":[114]}]},"episode":null}`},
	// END GENERATED
}
