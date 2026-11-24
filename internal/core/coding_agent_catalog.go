package core

import (
	"bytes"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"math"
	"reflect"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"unicode"
	"unicode/utf8"

	"github.com/himkt/cafleet"
	"github.com/himkt/cafleet/internal/vt"
)

//sumtype:decl
type answerKey interface{ isAnswerKey() }

type answerKeyDecision struct{ decision permissionDecision }

type answerKeyChoice struct{ choice string }

func (answerKeyDecision) isAnswerKey() {}
func (answerKeyChoice) isAnswerKey()   {}

func answerKeyDecisionOf(key answerKey) (permissionDecision, bool) {
	switch k := key.(type) {
	case answerKeyDecision:
		return k.decision, true
	case answerKeyChoice:
		return 0, false
	default:
		panic("invalid answerKey")
	}
}

func answerKeyChoiceOf(key answerKey) (string, bool) {
	switch k := key.(type) {
	case answerKeyDecision:
		return "", false
	case answerKeyChoice:
		return k.choice, true
	default:
		panic("invalid answerKey")
	}
}

type answerCapability struct {
	backend       codingAgent
	requestRuleID string
	answerRuleID  string
	key           answerKey
}

func invalidProfile(message string) *cafleetError {
	return usageError("invalid native profile: " + message)
}

func require(ok bool, message string) error {
	if ok {
		return nil
	}
	return invalidProfile(message)
}

func unique(ids map[string]bool, id string) error {
	if ids[id] {
		return invalidProfile("duplicate profile identity")
	}
	ids[id] = true
	return nil
}

// lowerHexBytes is the bytes of text when it is lowercase hexadecimal of at most limit bytes.
func lowerHexBytes(text string, limit int) ([]byte, bool) {
	for _, b := range []byte(text) {
		if (b < '0' || b > '9') && (b < 'a' || b > 'f') {
			return nil, false
		}
	}
	if len(text)%2 != 0 || len(text)/2 > limit {
		return nil, false
	}
	out := make([]byte, len(text)/2)
	for i := range out {
		value, err := strconv.ParseUint(text[2*i:2*i+2], 16, 8)
		if err != nil {
			panic("hex digits")
		}
		out[i] = byte(value)
	}
	return out, true
}

// maybe is an optional raw profile value of a type opt cannot hold; the strict decoder fills it
// like an opt.
type maybe[T any] struct {
	value T
	set   bool
}

func (m *maybe[T]) optValue() reflect.Value { return reflect.ValueOf(&m.value).Elem() }

func (m *maybe[T]) markSet() { m.set = true }

func (m maybe[T]) get() (T, bool) { return m.value, m.set }

func readString(decoder *jsontext.Decoder, expected string) (string, error) {
	token, err := decoder.ReadToken()
	if err != nil {
		return "", err
	}
	if token.Kind() != '"' {
		return "", appErrorf("invalid type: %v, expected %s", token.Kind(), expected)
	}
	return token.String(), nil
}

// nonBlank is text with a non-whitespace character.
type nonBlank string

func (n *nonBlank) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	text, err := readString(decoder, "a string")
	if err != nil {
		return err
	}
	if strings.TrimFunc(text, unicode.IsSpace) == "" {
		return appError("nonblank text required")
	}
	*n = nonBlank(text)
	return nil
}

// idList is nonblank, pairwise distinct strings.
type idList []string

func (l *idList) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	if kind := decoder.PeekKind(); kind != '[' {
		return appErrorf("invalid type: %v, expected a sequence", kind)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return err
	}
	values := []string{}
	seen := map[string]bool{}
	for decoder.PeekKind() != ']' {
		var value nonBlank
		if err := value.UnmarshalJSONFrom(decoder); err != nil {
			return err
		}
		values = append(values, string(value))
		seen[string(value)] = true
	}
	if _, err := decoder.ReadToken(); err != nil {
		return err
	}
	if len(seen) != len(values) {
		return appError("duplicate string identity")
	}
	*l = values
	return nil
}

// pattern is a compiled nonblank regular expression.
type pattern struct{ regex *regexp.Regexp }

func (p *pattern) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	var text nonBlank
	if err := text.UnmarshalJSONFrom(decoder); err != nil {
		return err
	}
	compiled, err := regexp.Compile(string(text))
	if err != nil {
		return appError("regex: " + err.Error())
	}
	p.regex = compiled
	return nil
}

type positiveBound uint16

func (b *positiveBound) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	token, err := decoder.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '0' {
		return appErrorf("invalid type: %v, expected u64", token.Kind())
	}
	value, err := strconv.ParseUint(token.String(), 10, 64)
	if err != nil {
		return appErrorf("invalid value: %s, expected u64", token.String())
	}
	if value == 0 || value > math.MaxUint16 {
		return appError("positive viewport bound required")
	}
	*b = positiveBound(value)
	return nil
}

// glyph is one non-whitespace character.
type glyph rune

func (g *glyph) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	text, err := readString(decoder, "a string")
	if err != nil {
		return err
	}
	separator, size := utf8.DecodeRuneInString(text)
	if size == 0 || size != len(text) || unicode.IsSpace(separator) {
		return appError("panel separator requires one non-whitespace glyph")
	}
	*g = glyph(separator)
	return nil
}

// ignoredAny accepts any JSON value unread beyond its syntax.
type ignoredAny struct{}

func (*ignoredAny) UnmarshalJSONFrom(decoder *jsontext.Decoder) error { return decoder.SkipValue() }

type nonZeroU64 uint64

func (n *nonZeroU64) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	token, err := decoder.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '0' {
		return appErrorf("invalid type: %v, expected a nonzero u64", token.Kind())
	}
	value, err := strconv.ParseUint(token.String(), 10, 64)
	if err != nil || value == 0 {
		return appErrorf("invalid value: %s, expected a nonzero u64", token.String())
	}
	*n = nonZeroU64(value)
	return nil
}

// decodeVariant reads one snake_case unit variant name of a profile enum.
func decodeVariant[T any](decoder *jsontext.Decoder, names []string, values []T) (T, error) {
	var zero T
	text, err := readString(decoder, "a variant name")
	if err != nil {
		return zero, err
	}
	if index := slices.Index(names, text); index >= 0 {
		return values[index], nil
	}
	return zero, appErrorf("unknown variant `%s`, expected one of `%s`", text, strings.Join(names, "`, `"))
}

type rawProfileFile struct {
	SchemaVersion uint64      `json:"schema_version"`
	Backend       codingAgent `json:"backend"`
	Profile       rawProfile  `json:"profile"`
	Screens       ignoredAny  `json:"screens"`
}

type rawProfile struct {
	Rules        []rawRule       `json:"rules"`
	Capabilities rawCapabilities `json:"capabilities"`
}

type rawRule struct {
	ID                     nonBlank           `json:"id"`
	Priority               int64              `json:"priority"`
	Observation            observationKind    `json:"observation"`
	Region                 rawRegion          `json:"region"`
	Matchers               []rawMatcher       `json:"matchers"`
	Extract                rawExtract         `json:"extract"`
	VerifiedNoninteraction bool               `json:"verified_noninteraction"`
	Frame                  maybe[rawFrame]    `json:"frame" cafleet:"present"`
	ContinuesRuleID        opt[nonBlank]      `json:"continues_rule_id" cafleet:"present"`
	ContinuesChoiceID      opt[nonBlank]      `json:"continues_choice_id" cafleet:"present"`
	FixtureIDs             opt[ignoredAny]    `json:"fixture_ids" cafleet:"present"`
	Suggestion             opt[rawSuggestion] `json:"suggestion" cafleet:"present"`
	TextPrefixes           maybe[rawPrefixes] `json:"text_prefixes" cafleet:"present"`
	TextWrap               opt[textWrap]      `json:"text_wrap" cafleet:"present"`
	TextFramed             opt[bool]          `json:"text_framed" cafleet:"present"`
}

type rawPrefixes struct {
	FirstPrefix        pattern `json:"first_prefix"`
	ContinuationPrefix pattern `json:"continuation_prefix"`
}

type rawExtract struct {
	Operation opt[rawOperation] `json:"operation" cafleet:"nullable"`
	Choices   []rawChoice       `json:"choices"`
}

type rawOperation struct {
	MatcherID  nonBlank        `json:"matcher_id"`
	Capture    nonBlank        `json:"capture"`
	RowMargin  opt[string]     `json:"row_margin" cafleet:"present"`
	MarginRows opt[marginRows] `json:"margin_rows" cafleet:"present"`
	RowTrailer opt[string]     `json:"row_trailer" cafleet:"present"`
}

type rawCapabilities struct {
	Detection        maybe[rawCapability]       `json:"detection" cafleet:"nullable"`
	Composer         maybe[rawCapability]       `json:"composer" cafleet:"nullable"`
	Answers          []rawAnswer                `json:"answers"`
	DirectorDelivery maybe[rawDirectorDelivery] `json:"director_delivery" cafleet:"present"`
}

type rawDirectorDelivery struct {
	Coverage     directorCoverage            `json:"coverage"`
	Multiplexers []deliveryMultiplexer       `json:"multiplexers"`
	RuleIDs      idList                      `json:"rule_ids"`
	PlainLimit   opt[opt[plainLiteralLimit]] `json:"plain_limit" cafleet:"present"`
	FixtureIDs   ignoredAny                  `json:"fixture_ids"`
}

type directorCoverage uint8

const directorCoverageTested directorCoverage = 1

func (c *directorCoverage) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"tested"}, []directorCoverage{directorCoverageTested})
	*c = value
	return err
}

type deliveryMultiplexer uint8

const (
	deliveryMultiplexerTmux deliveryMultiplexer = iota + 1
	deliveryMultiplexerHerdr
)

func (m *deliveryMultiplexer) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"tmux", "herdr"}, []deliveryMultiplexer{deliveryMultiplexerTmux, deliveryMultiplexerHerdr})
	*m = value
	return err
}

func (m deliveryMultiplexer) backend() muxBackend {
	switch m {
	case deliveryMultiplexerTmux:
		return muxBackendTmux
	case deliveryMultiplexerHerdr:
		return muxBackendHerdr
	default:
		panic("invalid deliveryMultiplexer")
	}
}

type rawComposerText struct {
	RuleIDs            idList           `json:"rule_ids"`
	FirstPrefix        pattern          `json:"first_prefix"`
	ContinuationPrefix pattern          `json:"continuation_prefix"`
	ModePrefix         opt[nonBlank]    `json:"mode_prefix" cafleet:"present"`
	Framed             bool             `json:"framed" cafleet:"default"`
	Wrap               textWrap         `json:"wrap"`
	Placeholders       []rawPlaceholder `json:"placeholders"`
}

type rawPlaceholder struct {
	Pattern pattern `json:"pattern"`
	Unit    string  `json:"unit"`
}

// rawCapability is one capability group; only the composer group may declare submission.
type rawCapability struct {
	RuleIDs    idList                 `json:"rule_ids"`
	FixtureIDs ignoredAny             `json:"fixture_ids"`
	Submission opt[rawSubmissions]    `json:"submission" cafleet:"present"`
	Text       maybe[rawComposerText] `json:"text" cafleet:"present"`
}

type rawSubmissions struct {
	Prompt rawSubmission `json:"prompt"`
	Exec   rawSubmission `json:"exec"`
}

type rawSubmission struct {
	PrefixHex         string                 `json:"prefix_hex"`
	SubmitHex         string                 `json:"submit_hex"`
	LiteralFraming    literalFraming         `json:"literal_framing"`
	PlainLiteralLimit opt[plainLiteralLimit] `json:"plain_literal_limit" cafleet:"nullable"`
	FixtureIDs        ignoredAny             `json:"fixture_ids"`
}

type literalFraming uint8

const (
	literalFramingPlain literalFraming = iota + 1
	literalFramingBracketedPaste
)

func (f *literalFraming) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"plain", "bracketed_paste"}, []literalFraming{literalFramingPlain, literalFramingBracketedPaste})
	*f = value
	return err
}

type literalUnit uint8

const (
	literalUnitBytes literalUnit = iota + 1
	literalUnitScalars
)

func (u *literalUnit) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"bytes", "scalars"}, []literalUnit{literalUnitBytes, literalUnitScalars})
	*u = value
	return err
}

type plainLiteralLimit struct {
	Unit literalUnit `json:"unit"`
	Max  nonZeroU64  `json:"max"`
}

type rawAnswer struct {
	Decision       opt[nonBlank] `json:"decision" cafleet:"present"`
	RequestRuleID  nonBlank      `json:"request_rule_id"`
	AnswerRuleID   nonBlank      `json:"answer_rule_id"`
	ChoiceID       nonBlank      `json:"choice_id"`
	FocusMatcherID nonBlank      `json:"focus_matcher_id"`
	Stages         []rawStage    `json:"stages"`
	FixtureIDs     ignoredAny    `json:"fixture_ids"`
}

type rawStage struct {
	BytesHex              nonBlank      `json:"bytes_hex"`
	RequiredRuleID        nonBlank      `json:"required_rule_id"`
	RequiredChoiceID      nonBlank      `json:"required_choice_id"`
	RequiredFocusChoiceID opt[nonBlank] `json:"required_focus_choice_id" cafleet:"present"`
	RequiredScreenRuleID  opt[nonBlank] `json:"required_screen_rule_id" cafleet:"present"`
}

type rawCapture struct {
	MatcherID nonBlank `json:"matcher_id"`
	Capture   nonBlank `json:"capture"`
}

type capture struct {
	matcher    string
	name       string
	rowMargin  opt[string]
	marginRows marginRows
	rowTrailer opt[string]
}

type marginRows uint8

const (
	marginRowsAll marginRows = iota + 1
	marginRowsContinuation
)

func (m *marginRows) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"all", "continuation"}, []marginRows{marginRowsAll, marginRowsContinuation})
	*m = value
	return err
}

type choiceScope uint8

const (
	choiceScopeSingleRequest choiceScope = iota + 1
	choiceScopePersistent
)

func (s *choiceScope) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"single_request", "persistent"}, []choiceScope{choiceScopeSingleRequest, choiceScopePersistent})
	*s = value
	return err
}

func (s choiceScope) asStr() string {
	switch s {
	case choiceScopeSingleRequest:
		return "single_request"
	case choiceScopePersistent:
		return "persistent"
	default:
		panic("invalid choiceScope")
	}
}

func (s choiceScope) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

type rawChoice struct {
	ID    nonBlank    `json:"id"`
	Label rawCapture  `json:"label"`
	Scope choiceScope `json:"scope"`
}

type choice struct {
	id    string
	label capture
	scope choiceScope
}

type matcherScope uint8

const (
	matcherScopeLine matcherScope = iota + 1
	matcherScopeRegion
)

func (s *matcherScope) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"line", "region"}, []matcherScope{matcherScopeLine, matcherScopeRegion})
	*s = value
	return err
}

type rawMatcher struct {
	ID         nonBlank        `json:"id"`
	Scope      matcherScope    `json:"scope"`
	Pattern    pattern         `json:"pattern"`
	FocusStyle opt[focusStyle] `json:"focus_style" cafleet:"present"`
}

type matcher struct {
	id         string
	scope      matcherScope
	regex      *regexp.Regexp
	focusStyle opt[focusStyle]
}

type focusStyle uint8

const (
	focusStyleBackground focusStyle = iota + 1
	focusStyleMarker
)

func (s *focusStyle) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"background", "marker"}, []focusStyle{focusStyleBackground, focusStyleMarker})
	*s = value
	return err
}

type continuation struct {
	ruleID   string
	choiceID string
}

type screenSelector uint8

const (
	screenSelectorPrimary screenSelector = iota + 1
	screenSelectorAlternate
	screenSelectorEither
)

func (s *screenSelector) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"primary", "alternate", "either"}, []screenSelector{screenSelectorPrimary, screenSelectorAlternate, screenSelectorEither})
	*s = value
	return err
}

type cursorRule uint8

const (
	cursorRuleAtEnd cursorRule = iota + 1
	// cursorRuleAfterPrompt is on the region's first row, right after the region's
	// start_pattern match.
	cursorRuleAfterPrompt
	cursorRuleInside
	cursorRuleAny
)

func (r *cursorRule) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"at_end", "after_prompt", "inside", "any"}, []cursorRule{cursorRuleAtEnd, cursorRuleAfterPrompt, cursorRuleInside, cursorRuleAny})
	*r = value
	return err
}

// suggestionStyle is the cell style a native CLI draws an inline suggestion in; such cells are
// not input.
type suggestionStyle struct {
	faint      bool
	foreground vt.Color
}

type rawSuggestion struct {
	Style rawSuggestionStyle `json:"style"`
}

type rawSuggestionStyle struct {
	Faint      opt[bool]      `json:"faint" cafleet:"present"`
	Foreground opt[rawColour] `json:"foreground" cafleet:"present"`
}

// rawColour is the fixture cell_backgrounds colour form: {"kind":"indexed","index"} or
// {"kind":"rgb","r","g","b"}.
type rawColour struct{ colour vt.Color }

type rawIndexedColour struct {
	Index uint8 `json:"index"`
}

type rawRgbColour struct {
	R uint8 `json:"r"`
	G uint8 `json:"g"`
	B uint8 `json:"b"`
}

func (c *rawColour) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decoder.ReadValue()
	if err != nil {
		return err
	}
	decoded, err := decodeTagged(value, "kind", map[string]func() any{
		"indexed": func() any { return &rawIndexedColour{} },
		"rgb":     func() any { return &rawRgbColour{} },
	})
	if err != nil {
		return err
	}
	switch colour := decoded.(type) {
	case *rawIndexedColour:
		c.colour = vt.Idx(colour.Index)
	case *rawRgbColour:
		c.colour = vt.Rgb(colour.R, colour.G, colour.B)
	default:
		panic("decodeTagged returns a registered variant")
	}
	return nil
}

func suggestionStyleOf(raw rawSuggestion) (suggestionStyle, error) {
	faint, faintSet := raw.Style.Faint.get()
	foreground, foregroundSet := raw.Style.Foreground.get()
	switch {
	case faintSet && faint && !foregroundSet:
		return suggestionStyle{faint: true}, nil
	case !faintSet && foregroundSet:
		return suggestionStyle{foreground: foreground.colour}, nil
	default:
		return suggestionStyle{}, invalidProfile(`a suggestion style is exactly {"faint": true} or one indexed or rgb foreground`)
	}
}

type rawRegion struct {
	Screen         screenSelector `json:"screen"`
	StartPattern   pattern        `json:"start_pattern"`
	EndPattern     pattern        `json:"end_pattern"`
	MaxRows        positiveBound  `json:"max_rows"`
	Cursor         cursorRule     `json:"cursor"`
	PanelSeparator opt[glyph]     `json:"panel_separator" cafleet:"present"`
}

type region struct {
	screen         screenSelector
	start          *regexp.Regexp
	end            *regexp.Regexp
	maxRows        uint16
	cursor         cursorRule
	panelSeparator opt[rune]
}

func regionOf(raw rawRegion) region {
	separator := none[rune]()
	if g, ok := raw.PanelSeparator.get(); ok {
		separator = some(rune(g))
	}
	return region{
		screen: raw.Screen, start: raw.StartPattern.regex, end: raw.EndPattern.regex,
		maxRows: uint16(raw.MaxRows), cursor: raw.Cursor, panelSeparator: separator,
	}
}

type rawFrame struct {
	RequirePatterns []pattern `json:"require_patterns"`
	RejectPatterns  []pattern `json:"reject_patterns"`
}

type frame struct {
	require []*regexp.Regexp
	reject  []*regexp.Regexp
}

func regexes(patterns []pattern) []*regexp.Regexp {
	out := make([]*regexp.Regexp, len(patterns))
	for i, p := range patterns {
		out[i] = p.regex
	}
	return out
}

// textPrefixes are the prefixes that open a composer's first and continuation rows.
type textPrefixes struct {
	firstPrefix        *regexp.Regexp
	continuationPrefix *regexp.Regexp
}

type rule struct {
	id           string
	priority     int64
	kind         observationKind
	region       region
	matchers     []matcher
	operation    opt[capture]
	choices      []choice
	verified     bool
	continuation opt[continuation]
	frame        *frame
	suggestion   opt[suggestionStyle]
	// textPrefixes replace the profile's composer.text prefixes in this rule's regions.
	textPrefixes *textPrefixes
	// textWrap replaces the profile's composer.text wrap in this rule's regions.
	textWrap opt[textWrap]
	// textFramed replaces composer.text.framed: whether the region start row frames the composer.
	textFramed opt[bool]
}

type capability struct {
	rules []string
}

type answer struct {
	key            answerKey
	request        string
	id             string
	choiceID       string
	focusMatcherID string
	stages         []answerStage
}

type answerStage struct {
	bytes []byte
	gate  stageGate
}

//sumtype:decl
type stageGate interface{ isStageGate() }

type stageGateFocus struct{ choice string }

type stageGateScreen struct{ rule string }

func (stageGateFocus) isStageGate()  {}
func (stageGateScreen) isStageGate() {}

func (s *answerStage) requiredFocusChoiceID() (string, bool) {
	if focus, ok := s.gate.(stageGateFocus); ok {
		return focus.choice, true
	}
	return "", false
}

func (s *answerStage) equal(other *answerStage) bool {
	return bytes.Equal(s.bytes, other.bytes) && s.gate == other.gate
}

const redacted = "<redacted>"

func writeRedacted(f fmt.State) {
	if _, err := f.Write([]byte(redacted)); err != nil {
		panic(err)
	}
}

func (answerStage) String() string             { return redacted }
func (answerStage) GoString() string           { return redacted }
func (answerStage) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (answerStage) MarshalJSON() ([]byte, error) {
	return nil, errors.New("answer stages are not serializable")
}

type answerPlan struct {
	capability     answerCapability
	choiceID       string
	choiceScope    choiceScope
	focusMatcherID string
	stages         []answerStage
}

func (p *answerPlan) equal(other *answerPlan) bool {
	return p.capability == other.capability && p.choiceID == other.choiceID && p.choiceScope == other.choiceScope &&
		p.focusMatcherID == other.focusMatcherID &&
		slices.EqualFunc(p.stages, other.stages, func(a, b answerStage) bool { return a.equal(&b) })
}

func (answerPlan) String() string             { return redacted }
func (answerPlan) GoString() string           { return redacted }
func (answerPlan) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (answerPlan) MarshalJSON() ([]byte, error) {
	return nil, errors.New("answer plans are not serializable")
}

type profile struct {
	backend          codingAgent
	rules            []rule
	detection        *capability
	composer         *capability
	composerText     *composerTextSpec
	submissions      [2]*composerSubmission
	answers          []answer
	directorDelivery *directorDelivery
}

type textWrap uint8

const (
	// textWrapChar joins rows with nothing.
	textWrapChar textWrap = iota + 1
	// textWrapWord lets a row break match nothing or exactly one literal space.
	textWrapWord
)

func (w *textWrap) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"char", "word"}, []textWrap{textWrapChar, textWrapWord})
	*w = value
	return err
}

type placeholderUnit uint8

const (
	placeholderUnitScalars placeholderUnit = iota + 1
	placeholderUnitLines
)

type placeholder struct {
	regex *regexp.Regexp
	unit  placeholderUnit
}

// composerTextSpec is how a profile reads the typed text back out of its composer rows.
type composerTextSpec struct {
	ruleIDs    []string
	prefixes   textPrefixes
	modePrefix opt[string]
	// framed is whether the region's start row frames the composer, whose text begins on the
	// next row.
	framed       bool
	wrap         textWrap
	placeholders []placeholder
}

// composerText is the composer text of one screen: its rule region, the rows the text spans
// and its segments.
type composerText struct {
	region   screenRegion
	rows     int
	segments []segment
	// modePrefix is leading literal text the native CLI consumes as a mode switch instead of
	// drawing it.
	modePrefix opt[string]
}

//sumtype:decl
type segment interface{ isSegment() }

type segmentText struct{ items []textItem }

type segmentScalars struct{ count int }

type segmentLines struct{ count int }

func (segmentText) isSegment()    {}
func (segmentScalars) isSegment() {}
func (segmentLines) isSegment()   {}

// textItem is one character, or a word-wrap row break (isBreak), which matches nothing or one
// literal space.
type textItem struct {
	char    rune
	isBreak bool
}

// anchoredEnd is the offset where regex's leftmost match ends when that match starts at 0.
func anchoredEnd(regex *regexp.Regexp, text string) (int, bool) {
	found := regex.FindStringIndex(text)
	if found == nil || found[0] != 0 {
		return 0, false
	}
	return found[1], true
}

// read is the composer text of the trimmed rows of the region rule bounds; false when the first
// row does not open with the first-row prefix.
func (s *composerTextSpec) read(r *rule, region screenRegion, rows []string) (composerText, bool) {
	prefixes := s.prefixes
	if r.textPrefixes != nil {
		prefixes = *r.textPrefixes
	}
	wrap := s.wrap
	if value, ok := r.textWrap.get(); ok {
		wrap = value
	}
	framed := s.framed
	if value, ok := r.textFramed.get(); ok {
		framed = value
	}
	if framed {
		if len(rows) < 1 {
			return composerText{}, false
		}
		rows = rows[1:]
	}
	if len(rows) == 0 {
		return composerText{}, false
	}
	first, rest := rows[0], rows[1:]
	end, ok := anchoredEnd(prefixes.firstPrefix, first)
	if !ok {
		return composerText{}, false
	}
	joined := first[end:]
	breaks := map[int]bool{}
	count := 1
	for _, row := range rest {
		end, ok := anchoredEnd(prefixes.continuationPrefix, row)
		if !ok {
			break
		}
		breaks[len(joined)] = true
		joined += row[end:]
		count++
	}
	segments, ok := s.segments(joined, breaks, wrap)
	if !ok {
		return composerText{}, false
	}
	return composerText{region: region, rows: count, segments: segments, modePrefix: s.modePrefix}, true
}

func (s *composerTextSpec) segments(joined string, breaks map[int]bool, wrap textWrap) ([]segment, bool) {
	var segments []segment
	var text []textItem
	offset := 0
	pushText := func(from, to int) {
		for index, character := range joined[from:to] {
			if wrap == textWrapWord && breaks[from+index] {
				text = append(text, textItem{isBreak: true})
			}
			text = append(text, textItem{char: character})
		}
	}
	for {
		var best []int
		var bestUnit placeholderUnit
		var bestRegex *regexp.Regexp
		for _, p := range s.placeholders {
			found := p.regex.FindStringSubmatchIndex(joined[offset:])
			if found == nil {
				continue
			}
			if best == nil || found[0]+offset < best[0] {
				for i := range found {
					if found[i] >= 0 {
						found[i] += offset
					}
				}
				best, bestUnit, bestRegex = found, p.unit, p.regex
			}
		}
		if best == nil {
			pushText(offset, len(joined))
			break
		}
		start, end := best[0], best[1]
		pushText(offset, start)
		if wrap == textWrapWord && breaks[start] {
			text = append(text, textItem{isBreak: true})
		}
		if len(text) > 0 {
			segments = append(segments, segmentText{items: text})
			text = nil
		}
		group := bestRegex.SubexpIndex("count")
		count, err := strconv.ParseUint(joined[best[2*group]:best[2*group+1]], 10, strconv.IntSize-1)
		if err != nil {
			return nil, false
		}
		switch bestUnit {
		case placeholderUnitScalars:
			segments = append(segments, segmentScalars{count: int(count)})
		case placeholderUnitLines:
			segments = append(segments, segmentLines{count: int(count)})
		default:
			panic("invalid placeholderUnit")
		}
		if end == start {
			return nil, false
		}
		offset = end
	}
	if len(text) > 0 {
		segments = append(segments, segmentText{items: text})
	}
	return segments, true
}

// matches reports whether the segments consume exactly the literal written as chunks, in order:
// text consumes its own characters, a scalars placeholder its count of scalars and a lines
// placeholder one whole chunk of its line count. A literal beginning with the mode prefix
// matches with or without it, because the native CLI may draw or consume the prefix.
func (c *composerText) matches(chunks []string) bool {
	characters := []rune(strings.Join(chunks, ""))
	if c.matchesAfter(chunks, characters, 0) {
		return true
	}
	prefix, ok := c.modePrefix.get()
	if !ok {
		return false
	}
	runes := []rune(prefix)
	return len(characters) >= len(runes) && slices.Equal(characters[:len(runes)], runes) &&
		c.matchesAfter(chunks, characters, len(runes))
}

type chunkEnd struct{ end, lines int }

func (c *composerText) matchesAfter(chunks []string, characters []rune, skipped int) bool {
	literal := characters[skipped:]
	wholeChunks := map[int]chunkEnd{}
	start := 0
	for _, chunk := range chunks {
		end := start + utf8.RuneCountInString(chunk)
		if start >= skipped {
			wholeChunks[start-skipped] = chunkEnd{end: end - skipped, lines: strings.Count(chunk, "\n") + 1}
		}
		start = end
	}
	positions := map[int]bool{0: true}
	for _, seg := range c.segments {
		next := map[int]bool{}
		switch s := seg.(type) {
		case segmentText:
			for _, item := range s.items {
				next = map[int]bool{}
				for at := range positions {
					if item.isBreak {
						next[at] = true
					}
					wanted := item.char
					if item.isBreak {
						wanted = ' '
					}
					if at < len(literal) && literal[at] == wanted {
						next[at+1] = true
					}
				}
				positions = next
			}
			next = positions
		case segmentScalars:
			for at := range positions {
				if at+s.count <= len(literal) {
					next[at+s.count] = true
				}
			}
		case segmentLines:
			for at := range positions {
				if whole, ok := wholeChunks[at]; ok && whole.lines == s.count {
					next[whole.end] = true
				}
			}
		default:
			panic("invalid segment")
		}
		positions = next
		if len(positions) == 0 {
			return false
		}
	}
	return positions[len(literal)]
}

// directorDelivery is where the Director delivery sweep may type into a backend's Director pane.
type directorDelivery struct {
	multiplexers []muxBackend
	ruleIDs      []string
	plainLimit   opt[plainLiteralLimit]
}

func (d *directorDelivery) covers(multiplexer muxBackend) bool {
	return slices.Contains(d.multiplexers, multiplexer)
}

// plainLiteralAllowed reports whether literal fits the Director screen's measured plain limit;
// a null limit leaves every inline preview plain.
func (d *directorDelivery) plainLiteralAllowed(literal string) bool {
	limit, ok := d.plainLimit.get()
	if !ok {
		return true
	}
	size := len(literal)
	if limit.Unit == literalUnitScalars {
		size = utf8.RuneCountInString(literal)
	}
	return uint64(size) <= uint64(limit.Max)
}

func (p *profile) ruleByID(id string) (*rule, bool) {
	for i := range p.rules {
		if p.rules[i].id == id {
			return &p.rules[i], true
		}
	}
	return nil, false
}

func (p *profile) answerPlan(request string, key answerKey) (answerPlan, bool) {
	var found *answer
	for i := range p.answers {
		if p.answers[i].request == request && p.answers[i].key == key {
			found = &p.answers[i]
			break
		}
	}
	if found == nil {
		return answerPlan{}, false
	}
	requestRule, ok := p.ruleByID(request)
	if !ok {
		panic("validated answer request rule")
	}
	scope := choiceScope(0)
	for _, c := range requestRule.choices {
		if c.id == found.choiceID {
			scope = c.scope
			break
		}
	}
	if scope == 0 {
		panic("validated answer request rule and choice")
	}
	return answerPlan{
		capability:     answerCapability{backend: p.backend, requestRuleID: request, answerRuleID: found.id, key: key},
		choiceID:       found.choiceID,
		choiceScope:    scope,
		focusMatcherID: found.focusMatcherID,
		stages:         slices.Clone(found.stages),
	}, true
}

// answerPlans is every plan of the request rule: decisions in allow, deny order, then choices in
// the rule's choice order.
func (p *profile) answerPlans(request string) []answerPlan {
	requestRule, ok := p.ruleByID(request)
	if !ok {
		return nil
	}
	keys := []answerKey{answerKeyDecision{decision: permissionDecisionAllow}, answerKeyDecision{decision: permissionDecisionDeny}}
	for _, c := range requestRule.choices {
		keys = append(keys, answerKeyChoice{choice: c.id})
	}
	var plans []answerPlan
	for _, key := range keys {
		if plan, ok := p.answerPlan(request, key); ok {
			plans = append(plans, plan)
		}
	}
	return plans
}

type profileCatalog struct {
	profiles      []profile
	captureOrigin string
}

func matcherByID(matchers []matcher, id string) (*matcher, bool) {
	for i := range matchers {
		if matchers[i].id == id {
			return &matchers[i], true
		}
	}
	return nil, false
}

func validateCapture(c *capture, matchers []matcher) error {
	m, ok := matcherByID(matchers, c.matcher)
	if !ok {
		return invalidProfile("unknown capture matcher")
	}
	return require(slices.Contains(m.regex.SubexpNames(), c.name) && c.name != "", "unknown named capture")
}

func operationCapture(raw rawOperation, matchers []matcher) (capture, error) {
	singleRow := func(text string) bool { return text != "" && !strings.Contains(text, "\n") }
	margin, marginSet := raw.RowMargin.get()
	if err := require(!marginSet || singleRow(margin), "nonempty single-row margin required"); err != nil {
		return capture{}, err
	}
	rows, rowsSet := raw.MarginRows.get()
	if err := require(!rowsSet || marginSet, "margin_rows requires row_margin"); err != nil {
		return capture{}, err
	}
	trailer, trailerSet := raw.RowTrailer.get()
	if err := require(!trailerSet || (singleRow(trailer) && strings.TrimFunc(trailer, unicode.IsSpace) == trailer), "trimmed nonempty single-row trailer required"); err != nil {
		return capture{}, err
	}
	if !rowsSet {
		rows = marginRowsAll
	}
	c := capture{matcher: string(raw.MatcherID), name: string(raw.Capture), rowMargin: raw.RowMargin, marginRows: rows, rowTrailer: raw.RowTrailer}
	if err := validateCapture(&c, matchers); err != nil {
		return capture{}, err
	}
	return c, nil
}

func captureOf(raw rawCapture) capture {
	return capture{matcher: string(raw.MatcherID), name: string(raw.Capture), marginRows: marginRowsAll}
}

func compileRule(raw rawRule) (rule, error) {
	kind := raw.Observation
	continues := kind == observationKindPermissionContinuation
	ruleID, ruleSet := raw.ContinuesRuleID.get()
	choiceID, choiceSet := raw.ContinuesChoiceID.get()
	_, fixturesSet := raw.FixtureIDs.get()
	cont := none[continuation]()
	switch {
	case continues && ruleSet && choiceSet && fixturesSet:
		cont = some(continuation{ruleID: string(ruleID), choiceID: string(choiceID)})
	case !continues && !ruleSet && !choiceSet:
	default:
		return rule{}, invalidProfile("exactly the permission continuation rules declare continues_rule_id and continues_choice_id, with fixture_ids")
	}
	verified := raw.VerifiedNoninteraction
	_, interaction := kind.interaction()
	if err := require(!verified || (!interaction && kind != observationKindUnknown && kind != observationKindPermissionContinuation),
		"interaction/unknown/continuation cannot verify noninteraction"); err != nil {
		return rule{}, err
	}
	matchers := make([]matcher, len(raw.Matchers))
	ids := map[string]bool{}
	for i, m := range raw.Matchers {
		matchers[i] = matcher{id: string(m.ID), scope: m.Scope, regex: m.Pattern.regex, focusStyle: m.FocusStyle}
		if err := unique(ids, string(m.ID)); err != nil {
			return rule{}, err
		}
	}
	if err := require(len(matchers) > 0, "required matchers cannot be empty"); err != nil {
		return rule{}, err
	}
	choices := make([]choice, len(raw.Extract.Choices))
	ids = map[string]bool{}
	for i, c := range raw.Extract.Choices {
		choices[i] = choice{id: string(c.ID), label: captureOf(c.Label), scope: c.Scope}
		if err := unique(ids, choices[i].id); err != nil {
			return rule{}, err
		}
		if err := validateCapture(&choices[i].label, matchers); err != nil {
			return rule{}, err
		}
	}
	rawOp, opSet := raw.Extract.Operation.get()
	if err := require(!continues || (!opSet && len(choices) == 0), "continuation declares no operation or choices"); err != nil {
		return rule{}, err
	}
	operation := none[capture]()
	if opSet {
		c, err := operationCapture(rawOp, matchers)
		if err != nil {
			return rule{}, err
		}
		operation = some(c)
	}
	var compiledFrame *frame
	if rawFrame, ok := raw.Frame.get(); ok {
		if !interaction {
			return rule{}, invalidProfile("only interaction rules declare a frame")
		}
		if err := require(len(rawFrame.RejectPatterns) > 0, "interaction frame requires a reject pattern"); err != nil {
			return rule{}, err
		}
		compiledFrame = &frame{require: regexes(rawFrame.RequirePatterns), reject: regexes(rawFrame.RejectPatterns)}
	}
	suggestion := none[suggestionStyle]()
	if rawSuggestion, ok := raw.Suggestion.get(); ok {
		style, err := suggestionStyleOf(rawSuggestion)
		if err != nil {
			return rule{}, err
		}
		suggestion = some(style)
	}
	if err := require(!suggestion.set || kind == observationKindComposerEmpty, "only composer_empty rules declare a suggestion style"); err != nil {
		return rule{}, err
	}
	if err := require((!raw.TextPrefixes.set && !raw.TextWrap.set && !raw.TextFramed.set) || kind == observationKindComposerText,
		"only composer_text rules declare text_prefixes, text_wrap or text_framed"); err != nil {
		return rule{}, err
	}
	var prefixes *textPrefixes
	if rawPrefixes, ok := raw.TextPrefixes.get(); ok {
		prefixes = &textPrefixes{firstPrefix: rawPrefixes.FirstPrefix.regex, continuationPrefix: rawPrefixes.ContinuationPrefix.regex}
	}
	return rule{
		id: string(raw.ID), priority: raw.Priority, kind: kind, region: regionOf(raw.Region), operation: operation,
		choices: choices, matchers: matchers, verified: verified, continuation: cont, frame: compiledFrame,
		suggestion: suggestion, textPrefixes: prefixes, textWrap: raw.TextWrap, textFramed: raw.TextFramed,
	}, nil
}

func ruleIn(rules []rule, id string) (*rule, bool) {
	for i := range rules {
		if rules[i].id == id {
			return &rules[i], true
		}
	}
	return nil, false
}

func validateGates(stages []answerStage, request *rule, rules []rule, choiceID, focusMatcher string) error {
	allUngated, allGated := true, true
	for _, stage := range stages {
		if stage.gate == nil {
			allGated = false
		} else {
			allUngated = false
		}
	}
	if allUngated {
		return nil
	}
	if err := require(allGated, "a gated plan gates every stage"); err != nil {
		return err
	}
	continuationOf := func(id string) (continuation, error) {
		r, ok := ruleIn(rules, id)
		if !ok || !r.continuation.set {
			return continuation{}, invalidProfile("screen gate names no continuation rule")
		}
		return r.continuation.value, nil
	}
	focusGated := false
	for index, stage := range stages {
		switch gate := stage.gate.(type) {
		case stageGateFocus:
			focusGated = true
			if err := require(slices.ContainsFunc(request.choices, func(c choice) bool { return c.id == gate.choice }),
				"focus gate names no choice of the request rule"); err != nil {
				return err
			}
		case stageGateScreen:
			cont, err := continuationOf(gate.rule)
			if err != nil {
				return err
			}
			if err := require(cont.ruleID == request.id, "screen gate continues another request rule"); err != nil {
				return err
			}
			previous, previousOK := "", false
			if index > 0 {
				previous, previousOK = stages[index-1].requiredFocusChoiceID()
			}
			if err := require(previousOK && previous == cont.choiceID, "screen gate follows the stage selecting its continued choice"); err != nil {
				return err
			}
		default:
			panic("every stage is gated")
		}
	}
	var finalChoice string
	switch gate := stages[len(stages)-1].gate.(type) {
	case stageGateFocus:
		finalChoice = gate.choice
	case stageGateScreen:
		cont, err := continuationOf(gate.rule)
		if err != nil {
			return err
		}
		finalChoice = cont.choiceID
	default:
		panic("validated gated final stage")
	}
	if err := require(finalChoice == choiceID, "a gated plan ends on its own choice"); err != nil {
		return err
	}
	if focusGated {
		m, ok := matcherByID(request.matchers, focusMatcher)
		if !ok {
			panic("validated focus matcher")
		}
		if err := require(m.focusStyle.set, "a focus-gated plan needs a focus_style matcher"); err != nil {
			return err
		}
		for _, c := range request.choices {
			label, ok := matcherByID(request.matchers, c.label.matcher)
			if !ok {
				panic("validated choice label matcher")
			}
			if err := require(label.scope == matcherScopeLine, "focus-gated choice labels are line-scoped"); err != nil {
				return err
			}
		}
	}
	return nil
}

func validateContinuation(r *rule, rules []rule) error {
	cont, ok := r.continuation.get()
	if !ok {
		return nil
	}
	continued, ok := ruleIn(rules, cont.ruleID)
	if !ok {
		return invalidProfile("continuation names an unknown rule")
	}
	if err := require(continued.kind == observationKindPermissionRequest, "continuation continues a permission request rule"); err != nil {
		return err
	}
	return require(slices.ContainsFunc(continued.choices, func(c choice) bool { return c.id == cont.choiceID }),
		"continuation names an unknown continued choice")
}

func isComposer(kind observationKind) bool {
	return kind == observationKindComposerEmpty || kind == observationKindComposerText
}

type group uint8

const (
	groupDetection group = iota + 1
	groupComposer
)

func groupOf(kind observationKind) group {
	if isComposer(kind) {
		return groupComposer
	}
	return groupDetection
}

func capabilityOf(raw *rawCapability, g group, rules []rule) (*capability, error) {
	if err := require(g == groupComposer || !raw.Submission.set, "only the composer capability declares a submission"); err != nil {
		return nil, err
	}
	if err := require(g != groupComposer || raw.Submission.set, "the composer capability declares a submission"); err != nil {
		return nil, err
	}
	if err := require(g == groupComposer || !raw.Text.set, "only the composer capability declares composer text"); err != nil {
		return nil, err
	}
	ids := slices.Clone([]string(raw.RuleIDs))
	if err := require(len(ids) > 0, "capability rules required"); err != nil {
		return nil, err
	}
	for _, id := range ids {
		r, ok := ruleIn(rules, id)
		if !ok {
			return nil, invalidProfile("unknown capability rule")
		}
		if err := require(groupOf(r.kind) == g, "capability rule component mismatch"); err != nil {
			return nil, err
		}
	}
	return &capability{rules: ids}, nil
}

func composerSubmissionOf(raw rawSubmission, operation composerOperation) (*composerSubmission, error) {
	prefix, ok := lowerHexBytes(raw.PrefixHex, math.MaxInt)
	if err := require(ok && bytes.Equal(prefix, operation.prefix()), "composer prefix must match its operation"); err != nil {
		return nil, err
	}
	submit, ok := lowerHexBytes(raw.SubmitHex, 32)
	if !ok || len(submit) == 0 {
		return nil, invalidProfile("composer submit requires 1–32 lowercase hexadecimal bytes")
	}
	return &composerSubmission{submit: submit, literalFraming: raw.LiteralFraming, plainLiteralLimit: raw.PlainLiteralLimit}, nil
}

func compileAnswer(raw rawAnswer, rules []rule, identities, keys map[string]bool) (answer, error) {
	request, id, choiceID := string(raw.RequestRuleID), string(raw.AnswerRuleID), string(raw.ChoiceID)
	if err := unique(identities, id); err != nil {
		return answer{}, err
	}
	r, ok := ruleIn(rules, request)
	if !ok {
		return answer{}, invalidProfile("unknown answer request rule")
	}
	decisionText, decided := raw.Decision.get()
	var key answerKey
	if decided {
		decision, err := permissionDecisionTryFrom(string(decisionText))
		if err != nil {
			return answer{}, err
		}
		if err := require(r.kind == observationKindPermissionRequest, "answer requires permission form"); err != nil {
			return answer{}, err
		}
		key = answerKeyDecision{decision: decision}
	} else {
		if err := require(r.kind == observationKindQuestion, "choice answer requires question form"); err != nil {
			return answer{}, err
		}
		if err := require(validChoiceID(choiceID), "choice id requires lowercase hyphenated words of at most 64 bytes"); err != nil {
			return answer{}, err
		}
		key = answerKeyChoice{choice: choiceID}
	}
	var keyText string
	switch k := key.(type) {
	case answerKeyDecision:
		keyText = request + ":decision:" + k.decision.asStr()
	case answerKeyChoice:
		keyText = request + ":choice:" + k.choice
	default:
		panic("invalid answerKey")
	}
	if err := unique(keys, keyText); err != nil {
		return answer{}, err
	}
	index := slices.IndexFunc(r.choices, func(c choice) bool { return c.id == choiceID })
	if index < 0 {
		return answer{}, invalidProfile("unknown answer choice")
	}
	if err := require(!decided || r.choices[index].scope == choiceScopeSingleRequest, "answer requires single-request scope"); err != nil {
		return answer{}, err
	}
	focus := string(raw.FocusMatcherID)
	if _, ok := matcherByID(r.matchers, focus); !ok {
		return answer{}, invalidProfile("unknown answer focus matcher")
	}
	if err := require(len(raw.Stages) >= 1 && len(raw.Stages) <= 16, "one to sixteen measured answer stages required"); err != nil {
		return answer{}, err
	}
	stages := make([]answerStage, 0, len(raw.Stages))
	for _, stage := range raw.Stages {
		stageBytes, ok := lowerHexBytes(string(stage.BytesHex), 256)
		if !ok || len(stageBytes) == 0 {
			return answer{}, invalidProfile("one to 256 lowercase hexadecimal stage bytes required")
		}
		if err := require(!bytes.Contains(stageBytes, []byte{0x1b}), "answer stages never contain Escape"); err != nil {
			return answer{}, err
		}
		if err := require(string(stage.RequiredRuleID) == request && string(stage.RequiredChoiceID) == choiceID, "answer stage request/decision mismatch"); err != nil {
			return answer{}, err
		}
		focusChoice, focusSet := stage.RequiredFocusChoiceID.get()
		screenRule, screenSet := stage.RequiredScreenRuleID.get()
		var gate stageGate
		switch {
		case focusSet && screenSet:
			return answer{}, invalidProfile("a stage declares at most one gate")
		case focusSet:
			gate = stageGateFocus{choice: string(focusChoice)}
		case screenSet:
			gate = stageGateScreen{rule: string(screenRule)}
		}
		stages = append(stages, answerStage{bytes: stageBytes, gate: gate})
	}
	if err := validateGates(stages, r, rules, choiceID, focus); err != nil {
		return answer{}, err
	}
	return answer{key: key, request: request, id: id, choiceID: choiceID, focusMatcherID: focus, stages: stages}, nil
}

func compileProfile(backend codingAgent, raw rawProfile) (profile, error) {
	rules := make([]rule, 0, len(raw.Rules))
	for _, rawRule := range raw.Rules {
		r, err := compileRule(rawRule)
		if err != nil {
			return profile{}, err
		}
		rules = append(rules, r)
	}
	identities := map[string]bool{}
	for _, r := range rules {
		if err := unique(identities, r.id); err != nil {
			return profile{}, err
		}
	}
	for i := range rules {
		if err := validateContinuation(&rules[i], rules); err != nil {
			return profile{}, err
		}
	}
	capabilities := raw.Capabilities
	var detection, composer *capability
	var err error
	if value, ok := capabilities.Detection.get(); ok {
		if detection, err = capabilityOf(&value, groupDetection, rules); err != nil {
			return profile{}, err
		}
	}
	if value, ok := capabilities.Composer.get(); ok {
		if composer, err = capabilityOf(&value, groupComposer, rules); err != nil {
			return profile{}, err
		}
	}
	var submission opt[rawSubmissions]
	var text maybe[rawComposerText]
	if rawComposer, ok := capabilities.Composer.get(); ok {
		submission, text = rawComposer.Submission, rawComposer.Text
	}
	var textSpec *composerTextSpec
	if submission.set && !text.set {
		return profile{}, invalidProfile("composer plan for " + backend.name() + " requires composer text extraction")
	}
	if rawText, ok := text.get(); ok {
		if textSpec, err = composerTextSpecOf(backend, rawText, rules); err != nil {
			return profile{}, err
		}
	}
	var submissions [2]*composerSubmission
	if rawSubmission, ok := submission.get(); ok {
		if submissions[0], err = composerSubmissionOf(rawSubmission.Prompt, composerOperationPrompt); err != nil {
			return profile{}, err
		}
		if submissions[1], err = composerSubmissionOf(rawSubmission.Exec, composerOperationExec); err != nil {
			return profile{}, err
		}
	}
	keys := map[string]bool{}
	answers := make([]answer, 0, len(capabilities.Answers))
	for _, rawAnswer := range capabilities.Answers {
		a, err := compileAnswer(rawAnswer, rules, identities, keys)
		if err != nil {
			return profile{}, err
		}
		answers = append(answers, a)
	}
	var delivery *directorDelivery
	if rawDelivery, ok := capabilities.DirectorDelivery.get(); ok {
		if delivery, err = directorDeliveryOf(backend, rawDelivery, rules, submissions[0]); err != nil {
			return profile{}, err
		}
	}
	return profile{
		backend: backend, rules: rules, detection: detection, composer: composer, composerText: textSpec,
		submissions: submissions, answers: answers, directorDelivery: delivery,
	}, nil
}

func composerTextSpecOf(backend codingAgent, raw rawComposerText, rules []rule) (*composerTextSpec, error) {
	for _, id := range raw.RuleIDs {
		if !slices.ContainsFunc(rules, func(r rule) bool { return r.id == id && isComposer(r.kind) }) {
			return nil, invalidProfile("composer plan for " + backend.name() + " requires composer text extraction")
		}
	}
	placeholders := make([]placeholder, 0, len(raw.Placeholders))
	for _, p := range raw.Placeholders {
		var unit placeholderUnit
		switch p.Unit {
		case "scalars":
			unit = placeholderUnitScalars
		case "lines":
			unit = placeholderUnitLines
		}
		if unit == 0 || !slices.Contains(p.Pattern.regex.SubexpNames(), "count") {
			return nil, invalidProfile("composer placeholder of " + backend.name() + " must capture count in scalars or lines")
		}
		placeholders = append(placeholders, placeholder{regex: p.Pattern.regex, unit: unit})
	}
	modePrefix := none[string]()
	if prefix, ok := raw.ModePrefix.get(); ok {
		modePrefix = some(string(prefix))
	}
	return &composerTextSpec{
		ruleIDs:      slices.Clone([]string(raw.RuleIDs)),
		prefixes:     textPrefixes{firstPrefix: raw.FirstPrefix.regex, continuationPrefix: raw.ContinuationPrefix.regex},
		modePrefix:   modePrefix,
		framed:       raw.Framed,
		wrap:         raw.Wrap,
		placeholders: placeholders,
	}, nil
}

func directorDeliveryOf(backend codingAgent, raw rawDirectorDelivery, rules []rule, prompt *composerSubmission) (*directorDelivery, error) {
	if raw.Coverage != directorCoverageTested {
		panic("invalid directorCoverage")
	}
	name := backend.name()
	if err := require(len(raw.Multiplexers) > 0, "director delivery lists a multiplexer"); err != nil {
		return nil, err
	}
	for _, id := range raw.RuleIDs {
		if !slices.ContainsFunc(rules, func(r rule) bool { return r.id == id && isComposer(r.kind) }) {
			return nil, invalidProfile("director delivery rule " + id + " of " + name + " is not a composer rule")
		}
	}
	plainLimit, ok := raw.PlainLimit.get()
	if !ok {
		return nil, invalidProfile("director delivery for " + name + " requires a measured plain_limit")
	}
	var multiplexers []muxBackend
	for _, m := range raw.Multiplexers {
		multiplexer := m.backend()
		mapped := false
		if prompt != nil {
			switch multiplexer {
			case muxBackendTmux:
				mapped = true
			case muxBackendHerdr:
				_, mapped = pressKeysFromBytes(prompt.submit)
			default:
				panic("invalid muxBackend")
			}
		}
		if !mapped {
			return nil, invalidProfile("director delivery for " + name + " on " + multiplexer.name() + " has no submit key mapping")
		}
		if !slices.Contains(multiplexers, multiplexer) {
			multiplexers = append(multiplexers, multiplexer)
		}
	}
	return &directorDelivery{multiplexers: multiplexers, ruleIDs: slices.Clone([]string(raw.RuleIDs)), plainLimit: plainLimit}, nil
}

func bundledProfileText(agent codingAgent) ([]byte, bool) {
	text, err := cafleet.Assets.ReadFile("assets/profiles/" + agent.name() + ".json")
	if err != nil {
		return nil, false
	}
	return text, true
}

func decodeFile(backend codingAgent, raw rawProfileFile) (profile, error) {
	if err := require(raw.SchemaVersion == 1, "unsupported profile schema"); err != nil {
		return profile{}, err
	}
	if err := require(raw.Backend == backend, "profile file backend differs from its file"); err != nil {
		return profile{}, err
	}
	return compileProfile(backend, raw.Profile)
}

// decodeProfileFileText strictly decodes one profile file for backend and compiles it; the
// file's screens and every fixture_ids value are provenance the bundled-catalog guard test
// checks, and the decoder skips them unread (F21).
func decodeProfileFileText(backend codingAgent, text []byte) (profile, error) {
	var raw rawProfileFile
	if err := decodeStrict(text, &raw); err != nil {
		return profile{}, invalidProfile(cafleetErrorOf(err).message())
	}
	return decodeFile(backend, raw)
}

var bundledCatalog = sync.OnceValue(func() *profileCatalog {
	var profiles []profile
	for _, agent := range codingAgentAll() {
		text, ok := bundledProfileText(agent)
		if !ok {
			continue
		}
		p, err := decodeProfileFileText(agent, text)
		if err != nil {
			panic("the bundled catalog passes its build guard: " + err.Error())
		}
		profiles = append(profiles, p)
	}
	return &profileCatalog{profiles: profiles, captureOrigin: "bundled_unknown"}
})

func profileCatalogBundled() *profileCatalog { return bundledCatalog() }

func (c *profileCatalog) profile(backend codingAgent) (*profile, bool) {
	for i := range c.profiles {
		if c.profiles[i].backend == backend {
			return &c.profiles[i], true
		}
	}
	return nil, false
}

func (c *profileCatalog) detectionCoverage(backend codingAgent) coverage {
	if p, ok := c.profile(backend); ok && p.detection != nil {
		return coverageTested
	}
	return coverageUnknown
}

func (c *profileCatalog) answerPlans(backend codingAgent, request string) []answerPlan {
	p, ok := c.profile(backend)
	if !ok {
		return nil
	}
	return p.answerPlans(request)
}

// directorDelivery is the backend's Director delivery; false when its profile declares none.
func (c *profileCatalog) directorDelivery(backend codingAgent) (*directorDelivery, bool) {
	p, ok := c.profile(backend)
	if !ok || p.directorDelivery == nil {
		return nil, false
	}
	return p.directorDelivery, true
}

type composerOperation uint8

const (
	composerOperationPrompt composerOperation = iota + 1
	composerOperationExec
)

func (o *composerOperation) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decodeVariant(decoder, []string{"prompt", "exec"}, []composerOperation{composerOperationPrompt, composerOperationExec})
	*o = value
	return err
}

func (o composerOperation) index() int {
	switch o {
	case composerOperationPrompt:
		return 0
	case composerOperationExec:
		return 1
	default:
		panic("invalid composerOperation")
	}
}

// prefix is the bytes that precede an operation's literal in the native composer.
func (o composerOperation) prefix() []byte {
	switch o {
	case composerOperationPrompt:
		return []byte{}
	case composerOperationExec:
		return []byte("! ")
	default:
		panic("invalid composerOperation")
	}
}

type composerSubmission struct {
	submit            []byte
	literalFraming    literalFraming
	plainLiteralLimit opt[plainLiteralLimit]
}

type composerPlan struct {
	operation         composerOperation
	submit            []byte
	literalFraming    literalFraming
	plainLiteralLimit opt[plainLiteralLimit]
}

func (composerPlan) String() string             { return redacted }
func (composerPlan) GoString() string           { return redacted }
func (composerPlan) Format(f fmt.State, _ rune) { writeRedacted(f) }
func (composerPlan) MarshalJSON() ([]byte, error) {
	return nil, errors.New("composer plans are not serializable")
}

func (p composerPlan) plainLiteralAllowed(literal string) bool {
	limit, ok := p.plainLiteralLimit.get()
	if !ok {
		return true
	}
	size := len(literal)
	if limit.Unit == literalUnitScalars {
		size = utf8.RuneCountInString(literal)
	}
	return uint64(len(p.operation.prefix()))+uint64(size) <= uint64(limit.Max)
}

func (p composerPlan) plainLimitRefusal(literal, backend string) string {
	limit, ok := p.plainLiteralLimit.get()
	if !ok {
		panic("a refused plain literal has its limit")
	}
	size, unit := len(literal), "bytes"
	if limit.Unit == literalUnitScalars {
		size, unit = utf8.RuneCountInString(literal), "characters"
	}
	size += len(p.operation.prefix())
	var reason string
	switch p.literalFraming {
	case literalFramingPlain:
		reason = "the " + backend + " profile has no bracketed-paste delivery"
	case literalFramingBracketedPaste:
		reason = "the native CLI has not enabled bracketed paste"
	default:
		panic("invalid literalFraming")
	}
	return fmt.Sprintf("the input is %d %s, above the %s plain-input limit of %d %s, and %s", size, unit, backend, uint64(limit.Max), unit, reason)
}

const maxComposerPayload = 65536

func (p composerPlan) encode(literal string) ([]byte, error) {
	prefix := p.operation.prefix()
	size := len(prefix) + len(literal) + len(p.submit)
	if size > maxComposerPayload {
		return nil, appError("native composer payload exceeds 65536 bytes")
	}
	out := make([]byte, 0, size)
	out = append(out, prefix...)
	out = append(out, literal...)
	return append(out, p.submit...), nil
}

// stages is the literal stage and the submit stage of literal under framing.
func (p composerPlan) stages(literal string, framing literalFraming) ([2][]byte, error) {
	encoded, err := p.encode(literal)
	if err != nil {
		return [2][]byte{}, err
	}
	if framing == literalFramingBracketedPaste {
		if p.literalFraming != literalFramingBracketedPaste {
			return [2][]byte{}, appError("native composer plan requires plain literal framing")
		}
		if len(encoded)+12 > maxComposerPayload {
			return [2][]byte{}, appError("native composer payload exceeds 65536 bytes")
		}
		submitOffset := len(encoded) - len(p.submit)
		encoded = slices.Insert(encoded, submitOffset, []byte("\x1b[201~")...)
		prefixOffset := len(p.operation.prefix())
		encoded = slices.Insert(encoded, prefixOffset, []byte("\x1b[200~")...)
	}
	split := len(encoded) - len(p.submit)
	return [2][]byte{encoded[:split:split], encoded[split:]}, nil
}

func (c *profileCatalog) verifiedComposer(backend codingAgent, operation composerOperation) bool {
	p, ok := c.profile(backend)
	return ok && p.composer != nil && p.submissions[operation.index()] != nil
}

func (c *profileCatalog) composerPlan(backend codingAgent, operation composerOperation) (composerPlan, bool) {
	p, ok := c.profile(backend)
	if !ok {
		return composerPlan{}, false
	}
	submission := p.submissions[operation.index()]
	if submission == nil {
		return composerPlan{}, false
	}
	return composerPlan{
		operation: operation, submit: slices.Clone(submission.submit),
		literalFraming: submission.literalFraming, plainLiteralLimit: submission.plainLiteralLimit,
	}, true
}
