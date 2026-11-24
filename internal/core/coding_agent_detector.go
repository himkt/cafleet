package core

import (
	"cmp"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"maps"
	"slices"
	"strings"
	"unicode"

	"github.com/himkt/cafleet/internal/vt"
)

// observationKind flattens Rust's ObservationKind, whose Interaction variant carries an
// interactionClass, into one comparable enum.
type observationKind uint8

const (
	observationKindPermissionRequest observationKind = iota + 1
	observationKindQuestion
	observationKindUnknownInteraction
	observationKindPermissionContinuation
	observationKindComposerEmpty
	// observationKindComposerText is a composer holding typed text, which composer.text reads
	// back.
	observationKindComposerText
	observationKindUnknown
)

func observationKindInteraction(class interactionClass) observationKind {
	switch class {
	case interactionClassPermissionRequest:
		return observationKindPermissionRequest
	case interactionClassQuestion:
		return observationKindQuestion
	case interactionClassUnknownInteraction:
		return observationKindUnknownInteraction
	default:
		panic("invalid interactionClass")
	}
}

// interaction is the interaction class of an Interaction kind.
func (k observationKind) interaction() (interactionClass, bool) {
	switch k {
	case observationKindPermissionRequest:
		return interactionClassPermissionRequest, true
	case observationKindQuestion:
		return interactionClassQuestion, true
	case observationKindUnknownInteraction:
		return interactionClassUnknownInteraction, true
	case observationKindPermissionContinuation, observationKindComposerEmpty, observationKindComposerText, observationKindUnknown:
		return 0, false
	default:
		panic("invalid observationKind")
	}
}

func observationKindParse(value string) (observationKind, error) {
	if class, err := interactionClassTryFrom(value); err == nil {
		return observationKindInteraction(class), nil
	}
	switch value {
	case "permission_continuation":
		return observationKindPermissionContinuation, nil
	case "composer_empty":
		return observationKindComposerEmpty, nil
	case "composer_text":
		return observationKindComposerText, nil
	case "unknown":
		return observationKindUnknown, nil
	default:
		return 0, usageError("invalid profile observation")
	}
}

func (k *observationKind) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	text, err := readString(decoder, "an observation")
	if err != nil {
		return err
	}
	parsed, err := observationKindParse(text)
	if err != nil {
		return err
	}
	*k = parsed
	return nil
}

func (k observationKind) name() string {
	switch k {
	case observationKindPermissionRequest, observationKindQuestion, observationKindUnknownInteraction:
		class, _ := k.interaction()
		return class.asStr()
	case observationKindPermissionContinuation:
		return "permission_continuation"
	case observationKindComposerEmpty:
		return "composer_empty"
	case observationKindComposerText:
		return "composer_text"
	case observationKindUnknown:
		return "unknown"
	default:
		panic("invalid observationKind")
	}
}

// debugString is Rust's `{:?}` of the kind, which the detector golden records.
func (k observationKind) debugString() string {
	switch k {
	case observationKindPermissionRequest:
		return "Interaction(PermissionRequest)"
	case observationKindQuestion:
		return "Interaction(Question)"
	case observationKindUnknownInteraction:
		return "Interaction(UnknownInteraction)"
	case observationKindPermissionContinuation:
		return "PermissionContinuation"
	case observationKindComposerEmpty:
		return "ComposerEmpty"
	case observationKindComposerText:
		return "ComposerText"
	case observationKindUnknown:
		return "Unknown"
	default:
		panic("invalid observationKind")
	}
}

type matchEvidence uint8

const (
	matchEvidenceComplete matchEvidence = iota + 1
	matchEvidenceConflicting
	matchEvidenceAbsent
)

func (e matchEvidence) debugString() string {
	switch e {
	case matchEvidenceComplete:
		return "Complete"
	case matchEvidenceConflicting:
		return "Conflicting"
	case matchEvidenceAbsent:
		return "Absent"
	default:
		panic("invalid matchEvidence")
	}
}

type semanticPart struct {
	kind      string
	operation opt[string]
	choices   []string
}

// compareSemanticParts orders parts as Rust orders its (&str, Option<String>, Vec<String>)
// tuples: by kind, then None before Some, then the choices lexicographically.
func compareSemanticParts(a, b semanticPart) int {
	if c := cmp.Compare(a.kind, b.kind); c != 0 {
		return c
	}
	aOp, aSet := a.operation.get()
	bOp, bSet := b.operation.get()
	switch {
	case aSet != bSet:
		if aSet {
			return 1
		}
		return -1
	case aSet:
		if c := cmp.Compare(aOp, bOp); c != 0 {
			return c
		}
	}
	return slices.Compare(a.choices, b.choices)
}

type semanticFingerprint struct {
	parts []semanticPart
}

func (f *semanticFingerprint) equal(other *semanticFingerprint) bool {
	return slices.EqualFunc(f.parts, other.parts, func(a, b semanticPart) bool { return compareSemanticParts(a, b) == 0 })
}

func (semanticFingerprint) String() string             { return redacted }
func (semanticFingerprint) GoString() string           { return redacted }
func (semanticFingerprint) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (semanticFingerprint) MarshalJSON() ([]byte, error) {
	return nil, errors.New("fingerprints are not serializable")
}

// screenRegion is the full-width rows top..bottom a rule's region matched.
type screenRegion struct {
	top    uint16
	bottom uint16
}

// nullableStrings is a JSON string array or null.
type nullableStrings struct {
	values []string
	set    bool
}

// observationEvidence is the JSON object an observation records, with its members in this order.
type observationEvidence struct {
	backend       string
	nativeVersion opt[string]
	ruleID        string
	screenSeq     uint64
	operation     opt[string]
	operationRows opt[uint64]
	choices       nullableStrings
	excerpt       opt[string]
	truncated     bool
}

func (e observationEvidence) MarshalJSONTo(encoder *jsontext.Encoder) error {
	var choices any
	if e.choices.set {
		choices = e.choices.values
		if e.choices.values == nil {
			choices = []string{}
		}
	}
	return jsonObject{
		{"backend", e.backend},
		{"native_version", e.nativeVersion},
		{"rule_id", e.ruleID},
		{"screen_seq", e.screenSeq},
		{"operation", e.operation},
		{"operation_rows", e.operationRows},
		{"choices", choices},
		{"excerpt", e.excerpt},
		{"truncated", e.truncated},
	}.MarshalJSONTo(encoder)
}

type screenObservation struct {
	kind                   observationKind
	ruleID                 string
	matchEvidence          matchEvidence
	matchedRegion          opt[screenRegion]
	rawEvidence            observationEvidence
	fingerprint            *semanticFingerprint
	verifiedNoninteraction bool
	outputSeq              uint64
	continuesRuleID        opt[string]
}

type detector struct {
	backend codingAgent
	version opt[string]
	profile *profile
}

type answerMatch struct {
	plan          answerPlan
	semantic      semanticFingerprint
	choices       []string
	choiceLabel   string
	focus         string
	focusCapture  string
	focusedChoice opt[string]
	outputSeq     uint64
}

func (answerMatch) String() string             { return redacted }
func (answerMatch) GoString() string           { return redacted }
func (answerMatch) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (answerMatch) MarshalJSON() ([]byte, error) {
	return nil, errors.New("answer matches are not serializable")
}

func (m *answerMatch) sameRequest(other *answerMatch) bool {
	return m.plan.equal(&other.plan) && m.semantic.equal(&other.semantic) &&
		slices.Equal(m.choices, other.choices) && m.focus == other.focus
}

type logicalLine struct {
	text       string
	start      uint16
	end        uint16
	boundaries [][2]uint16
}

func cellAt(screen *screenSnapshot, row, col uint16) (*vt.Cell, error) {
	cell := screen.screen.Cell(row, col)
	if cell == nil {
		return nil, appError("screen cell geometry mismatch")
	}
	return cell, nil
}

// clippedLogicalLines joins wrapped rows into logical lines; clips maps a panel row to its
// separator column, and such a row keeps only the cells left of the separator and always ends
// its logical line.
func clippedLogicalLines(screen *screenSnapshot, clips map[uint16]uint16) ([]logicalLine, error) {
	var lines []logicalLine
	var current *logicalLine
	for row := range screen.rows {
		width, clipped := clips[row]
		if !clipped {
			width = screen.cols
		}
		if current == nil {
			current = &logicalLine{start: row, end: row, boundaries: [][2]uint16{{row, 0}}}
		}
		current.end = row
		end := uint16(0)
		for col := width; col > 0; col-- {
			if c := screen.screen.Cell(row, col-1); c != nil && (c.HasContents() || c.IsWideContinuation()) {
				end = col
				break
			}
		}
		for col := range end {
			cell, err := cellAt(screen, row, col)
			if err != nil {
				return nil, err
			}
			if cell.IsWideContinuation() {
				continue
			}
			text := " "
			if cell.HasContents() {
				text = cell.Contents()
			}
			cellWidth := uint16(1)
			if cell.IsWide() {
				cellWidth = 2
			}
			current.boundaries[len(current.boundaries)-1] = [2]uint16{row, col}
			current.text += text
			for range len(text) {
				current.boundaries = append(current.boundaries, [2]uint16{row, col})
			}
			current.boundaries[len(current.boundaries)-1] = [2]uint16{row, col + cellWidth}
		}
		if clipped || !screen.screen.RowWrapped(row) || row+1 == screen.rows {
			lines = append(lines, *current)
			current = nil
		}
	}
	return lines, nil
}

// panelClips is the separator column of every panel row, when every panel row shares one column.
func panelClips(screen *screenSnapshot, separator rune) map[uint16]uint16 {
	glyphText := string(separator)
	clips := map[uint16]uint16{}
	last := uint16(0)
	if screen.cols > 0 {
		last = screen.cols - 1
	}
	for row := range screen.rows {
		for col := uint16(40); col < last; col++ {
			before, at, after := screen.screen.Cell(row, col-1), screen.screen.Cell(row, col), screen.screen.Cell(row, col+1)
			if before != nil && at != nil && after != nil && at.Contents() == glyphText &&
				at.BgColor() != before.BgColor() && at.BgColor() == after.BgColor() {
				clips[row] = col
				break
			}
		}
	}
	var first opt[uint16]
	for _, column := range clips {
		if value, ok := first.get(); ok && value != column {
			return map[uint16]uint16{}
		}
		first = some(column)
	}
	return clips
}

// screenLines are the unclipped logical lines and, per declared separator glyph, the clipped
// ones.
type screenLines struct {
	full    []logicalLine
	clipped map[rune][]logicalLine
}

func newScreenLines(screen *screenSnapshot, p *profile) (*screenLines, error) {
	clipped := map[rune][]logicalLine{}
	for _, r := range p.rules {
		separator, ok := r.region.panelSeparator.get()
		if !ok {
			continue
		}
		if _, done := clipped[separator]; done {
			continue
		}
		lines, err := clippedLogicalLines(screen, panelClips(screen, separator))
		if err != nil {
			return nil, err
		}
		clipped[separator] = lines
	}
	full, err := clippedLogicalLines(screen, map[uint16]uint16{})
	if err != nil {
		return nil, err
	}
	return &screenLines{full: full, clipped: clipped}, nil
}

func (l *screenLines) of(r *rule) []logicalLine {
	separator, ok := r.region.panelSeparator.get()
	if !ok {
		return l.full
	}
	lines, ok := l.clipped[separator]
	if !ok {
		panic("screen lines exist for every declared separator")
	}
	return lines
}

type labelCells struct {
	choice string
	cells  [][2]uint16
}

type matched struct {
	rule         *rule
	region       screenRegion
	operation    opt[string]
	choices      []string
	excerpt      string
	focuses      map[string]string
	matcherLines map[string]int
	labelCells   []labelCells
}

func captured(reference *capture, values map[string]map[string]string) opt[string] {
	named, ok := values[reference.matcher]
	if !ok {
		return none[string]()
	}
	value, ok := named[reference.name]
	if !ok || value == "" {
		return none[string]()
	}
	return some(value)
}

type candidate struct {
	start, end int
	region     screenRegion
}

// composerBottom is the row after a composer region's last composer row: a composer_text
// region's end-pattern row is not composer text.
func composerBottom(r *rule, region screenRegion, screen *screenSnapshot) (uint16, error) {
	lastRow, err := typedRow(screen, region.bottom-1, nil)
	if err != nil {
		return 0, err
	}
	if r.kind == observationKindComposerText && r.region.end.MatchString(lastRow) {
		return region.bottom - 1, nil
	}
	return region.bottom, nil
}

// retainLiveComposer keeps, without a cursor, only the composer region whose composer rows end
// lowest on the screen; a composer-shaped region above it belongs to the transcript.
func retainLiveComposer(all []matched, screen *screenSnapshot) ([]matched, error) {
	bottoms := make([]opt[uint16], len(all))
	lowest := none[uint16]()
	for i, m := range all {
		if !isComposer(m.rule.kind) {
			continue
		}
		bottom, err := composerBottom(m.rule, m.region, screen)
		if err != nil {
			return nil, err
		}
		bottoms[i] = some(bottom)
		if value, ok := lowest.get(); !ok || bottom > value {
			lowest = some(bottom)
		}
	}
	low, ok := lowest.get()
	if !ok {
		return all, nil
	}
	kept := all[:0:0]
	for i, m := range all {
		if bottom, isSet := bottoms[i].get(); !isSet || bottom == low {
			kept = append(kept, m)
		}
	}
	return kept, nil
}

func matchRule(r *rule, screen *screenSnapshot, lines []logicalLine) (matched, bool) {
	if r.frame != nil && !frameGuardHolds(r, lines) {
		return matched{}, false
	}
	candidates := qualifyingRegions(r, screen, lines)
	if r.kind == observationKindComposerText || (r.kind == observationKindComposerEmpty && cursorFree(screen)) {
		for i := len(candidates) - 1; i >= 0; i-- {
			if m, ok := extract(r, lines, candidates[i]); ok {
				return m, true
			}
		}
		return matched{}, false
	}
	if cursorFree(screen) {
		var found []matched
		for _, c := range candidates {
			if m, ok := extract(r, lines, c); ok {
				found = append(found, m)
				if len(found) > 1 {
					return matched{}, false
				}
			}
		}
		if len(found) == 1 {
			return found[0], true
		}
		return matched{}, false
	}
	if len(candidates) == 1 {
		return extract(r, lines, candidates[0])
	}
	return matched{}, false
}

// cursorFree is a capture without cursor or screen-mode data: cursor conditions read as any,
// the screen selector is ignored and exactly one candidate must satisfy the region matchers.
func cursorFree(screen *screenSnapshot) bool {
	return !screen.cursor.set || !screen.alternateScreen.set
}

func frameGuardHolds(r *rule, lines []logicalLine) bool {
	for _, pattern := range r.frame.require {
		if !slices.ContainsFunc(lines, func(l logicalLine) bool { return pattern.MatchString(l.text) }) {
			return false
		}
	}
	for _, pattern := range r.frame.reject {
		if slices.ContainsFunc(lines, func(l logicalLine) bool { return pattern.MatchString(l.text) }) {
			return false
		}
	}
	return true
}

func qualifyingRegions(r *rule, screen *screenSnapshot, lines []logicalLine) []candidate {
	return regionCandidates(&r.region, r.suggestion, r.kind == observationKindComposerText, screen, lines)
}

func boundaryAt(line *logicalLine, offset int) opt[[2]uint16] {
	if offset < 0 || offset >= len(line.boundaries) {
		return none[[2]uint16]()
	}
	return some(line.boundaries[offset])
}

// regionCandidates finds the regions a rule's region matches; with firstLaterEnd, each start
// row's only candidate ends at the first later row that matches end_pattern, as a composer_text
// region does.
func regionCandidates(regionRule *region, suggestion opt[suggestionStyle], firstLaterEnd bool, screen *screenSnapshot, lines []logicalLine) []candidate {
	cursor, alternate := none[[2]uint16](), none[bool]()
	if screen.cursor.set && screen.alternateScreen.set {
		cursor, alternate = screen.cursor, screen.alternateScreen
	}
	alt, altSet := alternate.get()
	if (regionRule.screen == screenSelectorPrimary && altSet && alt) || (regionRule.screen == screenSelectorAlternate && altSet && !alt) {
		return nil
	}
	var regions []candidate
	for start := range lines {
		line := &lines[start]
		starts := regionRule.start.FindAllStringIndex(line.text, 2)
		if len(starts) != 1 {
			continue
		}
		afterPrompt := boundaryAt(line, starts[0][1])
		closed := false
		first := start
		if firstLaterEnd {
			first++
		}
		for end := first; end < len(lines); end++ {
			last := &lines[end]
			if last.end-line.start+1 > regionRule.maxRows {
				closed = true
				break
			}
			found := regionRule.end.FindAllStringIndex(last.text, 2)
			if len(found) != 1 {
				continue
			}
			closed = true
			reg := screenRegion{top: line.start, bottom: last.end + 1}
			at, cursorSet := cursor.get()
			qualified := true
			if cursorSet {
				switch regionRule.cursor {
				case cursorRuleAny:
				case cursorRuleAtEnd:
					qualified = boundaryAt(last, found[0][1]) == some(at)
				case cursorRuleAfterPrompt:
					qualified = afterPrompt == some(at)
				case cursorRuleInside:
					// A pending-wrap cursor (column == cols) stands on its row's last cell, inside
					// the full-width region.
					qualified = at[0] >= reg.top && at[0] < reg.bottom && at[1] <= screen.cols
				default:
					panic("invalid cursorRule")
				}
			}
			if style, ok := suggestion.get(); qualified && ok {
				position, positionSet := afterPrompt.get()
				qualified = positionSet && suggestionOnly(screen, position[0], position[1], style)
			}
			if qualified {
				regions = append(regions, candidate{start: start, end: end, region: reg})
			}
			if firstLaterEnd {
				break
			}
		}
		if firstLaterEnd && !closed {
			if c, ok := bottomClosed(regionRule, cursor, screen, lines, start); ok {
				regions = append(regions, c)
			}
		}
	}
	return regions
}

// bottomClosed closes a composer drawn on the screen's last rows, which has no later end row, at
// the screen's bottom edge; every row of it is composer text.
func bottomClosed(regionRule *region, cursor opt[[2]uint16], screen *screenSnapshot, lines []logicalLine, start int) (candidate, bool) {
	end := len(lines) - 1
	reg := screenRegion{top: lines[start].start, bottom: lines[end].end + 1}
	qualified := true
	if at, ok := cursor.get(); ok {
		switch regionRule.cursor {
		case cursorRuleAny:
		case cursorRuleInside:
			qualified = at[0] >= reg.top && at[0] < reg.bottom && at[1] <= screen.cols
		case cursorRuleAtEnd, cursorRuleAfterPrompt:
			qualified = false
		default:
			panic("invalid cursorRule")
		}
	}
	return candidate{start: start, end: end, region: reg}, qualified
}

func trimEnd(s string) string { return strings.TrimRightFunc(s, unicode.IsSpace) }

// typedRow is row's text without cells drawn in any of styles, trailing blanks trimmed.
func typedRow(screen *screenSnapshot, row uint16, styles []suggestionStyle) (string, error) {
	var text strings.Builder
	for col := range screen.cols {
		cell, err := cellAt(screen, row, col)
		if err != nil {
			return "", err
		}
		if cell.IsWideContinuation() || slices.ContainsFunc(styles, func(style suggestionStyle) bool { return suggestionCell(cell, style) }) {
			continue
		}
		if cell.HasContents() {
			text.WriteString(cell.Contents())
		} else {
			text.WriteByte(' ')
		}
	}
	return trimEnd(text.String()), nil
}

// suggestionOnly reports whether every cell from col to the row's last non-blank cell carries
// the suggestion style.
func suggestionOnly(screen *screenSnapshot, row, col uint16, style suggestionStyle) bool {
	var cells []*vt.Cell
	for column := col; column < screen.cols; column++ {
		if cell := screen.screen.Cell(row, column); cell != nil {
			cells = append(cells, cell)
		}
	}
	last := -1
	for i := len(cells) - 1; i >= 0; i-- {
		if strings.TrimFunc(cells[i].Contents(), unicode.IsSpace) != "" {
			last = i
			break
		}
	}
	for _, cell := range cells[:last+1] {
		if !cell.IsWideContinuation() && !suggestionCell(cell, style) {
			return false
		}
	}
	return true
}

func suggestionCell(cell *vt.Cell, style suggestionStyle) bool {
	if style.faint {
		return cell.Dim()
	}
	return cell.FgColor() == style.foreground
}

// displayedRows removes the trailer, then the margin from the rows margin_rows selects, then
// trailing whitespace; trimming last keeps a whitespace-only margined row a match.
func displayedRows(value string, c *capture) (string, bool) {
	rows := strings.Split(value, "\n")
	out := make([]string, len(rows))
	for index, row := range rows {
		if trailer, ok := c.rowTrailer.get(); ok {
			if stripped, found := strings.CutSuffix(trimEnd(row), trailer); found {
				row = trimEnd(stripped)
			}
		}
		margined := index > 0 || c.marginRows == marginRowsAll
		if margin, ok := c.rowMargin.get(); ok && margined {
			stripped, found := strings.CutPrefix(row, margin)
			if !found {
				return "", false
			}
			row = stripped
		}
		out[index] = trimEnd(row)
	}
	return strings.Join(out, "\n"), true
}

type located struct{ line, start, end int }

type capturedValue struct {
	named   map[string]string
	full    string
	located map[string]located
	line    int
}

func sortedObject(values map[string]string) jsonObject {
	object := jsonObject{}
	for _, key := range slices.Sorted(maps.Keys(values)) {
		object = append(object, jsonMember{key, values[key]})
	}
	return object
}

func focusJSON(v any) string {
	encoded, err := encodeJSON(v)
	if err != nil {
		panic("focus strings serialize: " + err.Error())
	}
	return string(encoded)
}

func extract(r *rule, lines []logicalLine, c candidate) (matched, bool) {
	selected := lines[c.start : c.end+1]
	texts := make([]string, len(selected))
	for i := range selected {
		texts[i] = selected[i].text
	}
	regionText := strings.Join(texts, "\n")
	values := map[string]map[string]string{}
	focuses := map[string]string{}
	matcherLines := map[string]int{}
	spans := map[string]map[string]located{}
	for _, m := range r.matchers {
		searched := texts
		if m.scope == matcherScopeRegion {
			searched = []string{regionText}
		}
		var capturedValues []capturedValue
		for lineIndex, text := range searched {
			for _, found := range m.regex.FindAllStringSubmatchIndex(text, -1) {
				named := map[string]string{}
				locations := map[string]located{}
				for group, name := range m.regex.SubexpNames() {
					if name == "" || found[2*group] < 0 {
						continue
					}
					start, end := found[2*group], found[2*group+1]
					locations[name] = located{line: lineIndex, start: start, end: end}
					named[name] = text[start:end]
				}
				capturedValues = append(capturedValues, capturedValue{named: named, full: text[found[0]:found[1]], located: locations, line: lineIndex})
			}
		}
		if len(capturedValues) != 1 {
			return matched{}, false
		}
		value := capturedValues[0]
		focuses[m.id] = focusJSON([]any{m.id, value.full, sortedObject(value.named)})
		if m.scope == matcherScopeLine {
			spans[m.id] = value.located
			matcherLines[m.id] = value.line
		}
		values[m.id] = value.named
	}
	var cells []labelCells
	for _, ch := range r.choices {
		place, ok := spans[ch.label.matcher][ch.label.name]
		if !ok {
			continue
		}
		boundaries := selected[place.line].boundaries
		cells = append(cells, labelCells{choice: ch.id, cells: slices.Compact(slices.Clone(boundaries[place.start:place.end]))})
	}
	operation := none[string]()
	if reference, ok := r.operation.get(); ok {
		value, ok := captured(&reference, values).get()
		if !ok {
			return matched{}, false
		}
		if reference.rowMargin.set || reference.rowTrailer.set {
			value, ok = displayedRows(value, &reference)
			if !ok {
				return matched{}, false
			}
		}
		operation = some(value)
	}
	choices := make([]string, 0, len(r.choices))
	for _, ch := range r.choices {
		value, ok := captured(&ch.label, values).get()
		if !ok {
			return matched{}, false
		}
		choices = append(choices, value)
	}
	return matched{
		rule: r, region: c.region, operation: operation, choices: choices, excerpt: regionText,
		focuses: focuses, matcherLines: matcherLines, labelCells: cells,
	}, true
}

type colorCount struct {
	color vt.Color
	count int
}

// majorityBackground is the background covering more than half of the row's cells; a layout
// without a left margin paints its highlight over the edge cells, so no single edge cell is the
// row background.
func majorityBackground(screen *screenSnapshot, row uint16) (vt.Color, bool) {
	var counts []colorCount
	for col := range screen.cols {
		cell := screen.screen.Cell(row, col)
		if cell == nil {
			return vt.Color{}, false
		}
		background := cell.BgColor()
		index := slices.IndexFunc(counts, func(c colorCount) bool { return c.color == background })
		if index >= 0 {
			counts[index].count++
		} else {
			counts = append(counts, colorCount{color: background, count: 1})
		}
	}
	for _, c := range counts {
		if c.count*2 > int(screen.cols) {
			return c.color, true
		}
	}
	return vt.Color{}, false
}

// backgroundFocus is the one label painted on a single background that differs from its row's
// majority background, with no other label carrying such a background.
func backgroundFocus(screen *screenSnapshot, m *matched) (string, bool) {
	var focused []string
	for _, label := range m.labelCells {
		if len(label.cells) == 0 {
			return "", false
		}
		rowBackground, ok := majorityBackground(screen, label.cells[0][0])
		if !ok {
			return "", false
		}
		backgrounds := make([]vt.Color, 0, len(label.cells))
		for _, at := range label.cells {
			cell := screen.screen.Cell(at[0], at[1])
			if cell == nil {
				return "", false
			}
			backgrounds = append(backgrounds, cell.BgColor())
		}
		if !slices.ContainsFunc(backgrounds, func(bg vt.Color) bool { return bg != rowBackground }) {
			continue
		}
		if slices.ContainsFunc(backgrounds, func(bg vt.Color) bool { return bg != backgrounds[0] }) {
			return "", false
		}
		focused = append(focused, label.choice)
	}
	if len(focused) == 1 {
		return focused[0], true
	}
	return "", false
}

// markerLayout is the line of each choice label: the marker moves between these lines, so they
// stay equal while only the focus moves and change when the choice rows' layout does.
func markerLayout(r *rule, m *matched) string {
	layout := make([]any, 0, len(r.choices))
	for _, ch := range r.choices {
		var line any
		if value, ok := m.matcherLines[ch.label.matcher]; ok {
			line = value
		}
		layout = append(layout, []any{ch.id, line})
	}
	return focusJSON(layout)
}

// markerFocus is the one choice whose label matcher matched the focus matcher's row.
func markerFocus(r *rule, m *matched, focusMatcher string) (string, bool) {
	focusLine, ok := m.matcherLines[focusMatcher]
	if !ok {
		return "", false
	}
	var focused []string
	for _, ch := range r.choices {
		if line, ok := m.matcherLines[ch.label.matcher]; ok && line == focusLine {
			focused = append(focused, ch.id)
		}
	}
	if len(focused) == 1 {
		return focused[0], true
	}
	return "", false
}

func answerMatchOf(screen *screenSnapshot, observed *screenObservation, r *rule, m *matched, plan *answerPlan) answerMatch {
	focusMatcher, ok := matcherByID(r.matchers, plan.focusMatcherID)
	if !ok {
		panic("validated focus matcher")
	}
	style, styled := focusMatcher.focusStyle.get()
	var captureText string
	if styled && style == focusStyleMarker {
		captureText = markerLayout(r, m)
	} else {
		value, ok := m.focuses[plan.focusMatcherID]
		if !ok {
			panic("validated focus matcher")
		}
		captureText = value
	}
	focusedChoice := none[string]()
	if styled {
		var choice string
		var found bool
		switch style {
		case focusStyleBackground:
			choice, found = backgroundFocus(screen, m)
		case focusStyleMarker:
			choice, found = markerFocus(r, m, plan.focusMatcherID)
		default:
			panic("invalid focusStyle")
		}
		if found {
			focusedChoice = some(choice)
		}
	}
	focus := captureText
	if styled {
		focus = focusJSON([]any{captureText, focusedChoice})
	}
	choiceIndex := slices.IndexFunc(r.choices, func(c choice) bool { return c.id == plan.choiceID })
	if choiceIndex < 0 {
		panic("validated answer choice")
	}
	if observed.fingerprint == nil {
		panic("complete interaction fingerprint")
	}
	return answerMatch{
		choiceLabel:   m.choices[choiceIndex],
		plan:          *plan,
		semantic:      *observed.fingerprint,
		choices:       slices.Clone(m.choices),
		focus:         focus,
		focusCapture:  captureText,
		focusedChoice: focusedChoice,
		outputSeq:     screen.screenSeq,
	}
}

// withMatchedRule is base with the fields every matched rule records: its id, operation and
// excerpt.
func withMatchedRule(base observationEvidence, r *rule, operation opt[string], excerpt string) observationEvidence {
	evidence := base
	evidence.ruleID = r.id
	evidence.operation = operation
	evidence.operationRows = none[uint64]()
	if value, ok := operation.get(); ok {
		rows := strings.Count(value, "\n") + 1
		if rows < 1 {
			panic("a value has at least one row")
		}
		evidence.operationRows = some(uint64(rows))
	}
	evidence.excerpt = some(excerpt)
	return evidence
}

// matchAnswers matches every plan whose request observed (this detector's observation of
// screen) completes, building the screen's lines and the matched request rule once.
func (d *detector) matchAnswers(screen *screenSnapshot, observed *screenObservation, plans []answerPlan) ([]answerMatch, error) {
	if d.profile == nil || observed.matchEvidence != matchEvidenceComplete || len(plans) == 0 {
		return nil, nil
	}
	r, ok := d.profile.ruleByID(observed.ruleID)
	if !ok {
		panic("selected compiled request rule")
	}
	lines, err := newScreenLines(screen, d.profile)
	if err != nil {
		return nil, err
	}
	m, ok := matchRule(r, screen, lines.of(r))
	if !ok {
		return nil, nil
	}
	matches := make([]answerMatch, 0, len(plans))
	for i := range plans {
		matches = append(matches, answerMatchOf(screen, observed, r, &m, &plans[i]))
	}
	return matches, nil
}

func newDetector(catalog *profileCatalog, backend codingAgent, version opt[string]) *detector {
	p, _ := catalog.profile(backend)
	return &detector{backend: backend, version: version, profile: p}
}

// composerText is the composer text of screen: the one composer.text rule matching the screen
// bounds the rows, read with the profile's prefixes after dropping suggestion-styled cells;
// false when no rule or several rules match, or the first row lacks the first prefix.
func (d *detector) composerText(screen *screenSnapshot) (composerText, bool, error) {
	if d.profile == nil || d.profile.composerText == nil {
		return composerText{}, false, nil
	}
	spec := d.profile.composerText
	rules := make([]*rule, 0, len(spec.ruleIDs))
	for _, id := range spec.ruleIDs {
		r, ok := d.profile.ruleByID(id)
		if !ok {
			panic("validated composer text rule")
		}
		rules = append(rules, r)
	}
	lines, err := newScreenLines(screen, d.profile)
	if err != nil {
		return composerText{}, false, err
	}
	var matchedRule *rule
	var matchedRegion screenRegion
	count := 0
	for _, r := range rules {
		if m, ok := matchRule(r, screen, lines.of(r)); ok {
			count++
			if count > 1 {
				return composerText{}, false, nil
			}
			matchedRule, matchedRegion = r, m.region
		}
	}
	if count == 0 {
		return composerText{}, false, nil
	}
	textBottom, err := composerBottom(matchedRule, matchedRegion, screen)
	if err != nil {
		return composerText{}, false, err
	}
	var styles []suggestionStyle
	for _, r := range rules {
		if style, ok := r.suggestion.get(); ok {
			styles = append(styles, style)
		}
	}
	rows := make([]string, 0, int(textBottom)-int(matchedRegion.top))
	for row := matchedRegion.top; row < textBottom; row++ {
		text, err := typedRow(screen, row, styles)
		if err != nil {
			return composerText{}, false, err
		}
		rows = append(rows, text)
	}
	text, ok := spec.read(matchedRule, matchedRegion, rows)
	return text, ok, nil
}

func (d *detector) observe(screen *screenSnapshot) (screenObservation, error) {
	return d.observeWhere(screen, func(p *profile, r *rule) bool {
		if r.continuation.set {
			return true
		}
		for _, c := range []*capability{p.detection, p.composer} {
			if c != nil && slices.Contains(c.rules, r.id) {
				return true
			}
		}
		return false
	})
}

// observeDirector observes a Director screen over composerRules plus every interaction rule.
func (d *detector) observeDirector(screen *screenSnapshot, composerRules []string) (screenObservation, error) {
	return d.observeWhere(screen, func(_ *profile, r *rule) bool {
		_, interaction := r.kind.interaction()
		return r.continuation.set || interaction || slices.Contains(composerRules, r.id)
	})
}

func (d *detector) observeWhere(screen *screenSnapshot, selected func(*profile, *rule) bool) (screenObservation, error) {
	observation := screenObservation{
		kind:          observationKindUnknown,
		ruleID:        "unmatched-screen",
		matchEvidence: matchEvidenceAbsent,
		rawEvidence: observationEvidence{
			backend: d.backend.name(), nativeVersion: d.version, ruleID: "unmatched-screen", screenSeq: screen.screenSeq,
		},
		outputSeq: screen.screenSeq,
	}
	if d.profile == nil {
		return observation, nil
	}
	lines, err := newScreenLines(screen, d.profile)
	if err != nil {
		return screenObservation{}, err
	}
	var all []matched
	for i := range d.profile.rules {
		r := &d.profile.rules[i]
		if !selected(d.profile, r) {
			continue
		}
		if m, ok := matchRule(r, screen, lines.of(r)); ok {
			all = append(all, m)
		}
	}
	if cursorFree(screen) {
		if all, err = retainLiveComposer(all, screen); err != nil {
			return screenObservation{}, err
		}
	}
	if len(all) == 0 {
		return observation, nil
	}
	priority := all[0].rule.priority
	for _, m := range all {
		priority = max(priority, m.rule.priority)
	}
	all = slices.DeleteFunc(all, func(m matched) bool { return m.rule.priority != priority })
	slices.SortStableFunc(all, func(a, b matched) int { return cmp.Compare(a.rule.id, b.rule.id) })
	parts := make([]semanticPart, 0, len(all))
	for _, m := range all {
		parts = append(parts, semanticPart{kind: m.rule.kind.name(), operation: m.operation, choices: m.choices})
	}
	slices.SortStableFunc(parts, compareSemanticParts)
	parts = slices.CompactFunc(parts, func(a, b semanticPart) bool { return compareSemanticParts(a, b) == 0 })
	conflict := len(parts) > 1
	best := &all[0]
	switch {
	case !conflict:
		observation.kind = best.rule.kind
	case slices.ContainsFunc(all, func(m matched) bool { _, ok := m.rule.kind.interaction(); return ok }):
		observation.kind = observationKindUnknownInteraction
	default:
		observation.kind = observationKindUnknown
	}
	observation.ruleID = best.rule.id
	observation.matchEvidence = matchEvidenceComplete
	if conflict {
		observation.matchEvidence = matchEvidenceConflicting
	}
	observation.matchedRegion = some(best.region)
	observation.verifiedNoninteraction = !conflict && best.rule.verified
	if observation.kind == observationKindPermissionContinuation {
		if cont, ok := best.rule.continuation.get(); ok {
			observation.continuesRuleID = some(cont.ruleID)
		}
	}
	operation := best.operation
	if conflict {
		operation = none[string]()
	}
	observation.rawEvidence = withMatchedRule(observation.rawEvidence, best.rule, operation, best.excerpt)
	observation.rawEvidence.choices = nullableStrings{}
	if !conflict {
		observation.rawEvidence.choices = nullableStrings{values: slices.Clone(best.choices), set: true}
	}
	if _, interaction := observation.kind.interaction(); interaction || observation.kind == observationKindPermissionContinuation {
		observation.fingerprint = &semanticFingerprint{parts: parts}
	}
	return observation, nil
}
