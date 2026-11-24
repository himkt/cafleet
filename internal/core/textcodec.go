package core

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"reflect"
	"strconv"
	"strings"
	"syscall"
	"time"
	"unicode"
	"unicode/utf8"

	"golang.org/x/sys/unix"
)

// encodeJSON writes v in the SPEC §6.4 JSON encoding (F6): compact, struct fields in declaration
// order, nil slices as `[]`, and strings with `"`, `\`, the short control escapes and \u00XX for
// other code points below U+0020, every other scalar raw. Invalid UTF-8 is an error.
func encodeJSON(v any) ([]byte, error) {
	encoded, err := json.Marshal(v, json.Deterministic(true))
	if err != nil {
		return nil, appError(err.Error())
	}
	return encoded, nil
}

type jsonMember struct {
	key   string
	value any
}

// jsonObject is a JSON object whose members encode in slice order.
type jsonObject []jsonMember

func (o jsonObject) MarshalJSONTo(encoder *jsontext.Encoder) error {
	if err := encoder.WriteToken(jsontext.BeginObject); err != nil {
		return err
	}
	for _, member := range o {
		if err := encoder.WriteToken(jsontext.String(member.key)); err != nil {
			return err
		}
		if err := json.MarshalEncode(encoder, member.value); err != nil {
			return err
		}
	}
	return encoder.WriteToken(jsontext.EndObject)
}

func (o opt[T]) MarshalJSONTo(encoder *jsontext.Encoder) error {
	if !o.set {
		return encoder.WriteToken(jsontext.Null)
	}
	return json.MarshalEncode(encoder, o.value)
}

// optTarget lets the strict decoder fill an opt[T] of any T through reflection.
type optTarget interface {
	optValue() reflect.Value
	markSet()
}

func (o *opt[T]) optValue() reflect.Value {
	return reflect.ValueOf(&o.value).Elem()
}

func (o *opt[T]) markSet() {
	o.set = true
}

// Struct-field presence rules of decodeStrict, selected by the `cafleet` tag. Without a tag a
// field is required and not null, and an opt field may be absent or null (none).
const (
	// An opt field that may be absent but, when present, decodes its value as the inner type.
	jsonPresenceTagPresent = "present"
	// An opt field that must be present and may be null.
	jsonPresenceTagNullable = "nullable"
	// A non-opt field whose absence leaves its zero value; when present it is not null.
	jsonPresenceTagDefault = "default"
)

// decodeStrict decodes one JSON object into the struct target points to, rejecting what the
// Rust decoder rejects (F7): invalid UTF-8, duplicate member names, members with no field
// (including case variants) at every nesting level, a missing required member, a present null
// where its field forbids one, integers outside their field's range, a mistyped value and
// trailing data.
func decodeStrict(data []byte, target any) error {
	destination := reflect.ValueOf(target)
	if destination.Kind() != reflect.Pointer || destination.Elem().Kind() != reflect.Struct {
		panic(fmt.Sprintf("decodeStrict decodes into a pointer to a struct, not %T", target))
	}
	decoder := jsontext.NewDecoder(bytes.NewReader(data))
	if err := decodeStrictValue(decoder, destination.Elem()); err != nil {
		return err
	}
	if _, err := decoder.ReadToken(); err != io.EOF {
		return appError("trailing characters after the JSON value")
	}
	return nil
}

func decodeStrictValue(decoder *jsontext.Decoder, destination reflect.Value) error {
	if target, ok := destination.Addr().Interface().(optTarget); ok {
		if decoder.PeekKind() == 'n' {
			if _, err := decoder.ReadToken(); err != nil {
				return appError(err.Error())
			}
			return nil
		}
		if err := decodeStrictValue(decoder, target.optValue()); err != nil {
			return err
		}
		target.markSet()
		return nil
	}
	if _, ok := destination.Addr().Interface().(json.UnmarshalerFrom); ok {
		return decodeLeaf(decoder, destination)
	}
	if kind := destination.Kind(); kind == reflect.Struct {
		return decodeStrictStruct(decoder, destination)
	} else if kind == reflect.Slice {
		return decodeStrictSlice(decoder, destination)
	}
	if decoder.PeekKind() == 'n' {
		return appErrorf("invalid type: null, expected %s", destination.Type())
	}
	return decodeLeaf(decoder, destination)
}

func decodeLeaf(decoder *jsontext.Decoder, destination reflect.Value) error {
	if err := json.UnmarshalDecode(decoder, destination.Addr().Interface()); err != nil {
		return appError(err.Error())
	}
	return nil
}

type strictField struct {
	index    int
	name     string
	presence string
	isOpt    bool
}

func strictFields(structType reflect.Type) []strictField {
	optType := reflect.TypeFor[optTarget]()
	var fields []strictField
	for index := range structType.NumField() {
		field := structType.Field(index)
		tag, ok := field.Tag.Lookup("json")
		if !ok || !field.IsExported() {
			panic(fmt.Sprintf("%s.%s needs an exported field with a json tag for decodeStrict", structType, field.Name))
		}
		name, _, _ := strings.Cut(tag, ",")
		presence := field.Tag.Get("cafleet")
		isOpt := reflect.PointerTo(field.Type).Implements(optType)
		switch presence {
		case "", jsonPresenceTagDefault:
		case jsonPresenceTagPresent, jsonPresenceTagNullable:
			if !isOpt {
				panic(fmt.Sprintf("%s.%s: cafleet:%q applies only to opt fields", structType, field.Name, presence))
			}
		default:
			panic(fmt.Sprintf("%s.%s: unknown cafleet presence tag %q", structType, field.Name, presence))
		}
		fields = append(fields, strictField{index: index, name: name, presence: presence, isOpt: isOpt})
	}
	return fields
}

func decodeStrictStruct(decoder *jsontext.Decoder, destination reflect.Value) error {
	if kind := decoder.PeekKind(); kind != '{' {
		return appErrorf("invalid type: expected a JSON object for %s, found %v", destination.Type(), kind)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return appError(err.Error())
	}
	fields := strictFields(destination.Type())
	byName := make(map[string]strictField, len(fields))
	for _, field := range fields {
		byName[field.name] = field
	}
	seen := map[string]bool{}
	for decoder.PeekKind() != '}' {
		token, err := decoder.ReadToken()
		if err != nil {
			return appError(err.Error())
		}
		name := token.String()
		field, known := byName[name]
		if !known {
			return appError("unknown field `" + name + "`")
		}
		seen[name] = true
		value := destination.Field(field.index)
		if field.isOpt && field.presence == jsonPresenceTagPresent {
			target := value.Addr().Interface().(optTarget)
			if err := decodeStrictValue(decoder, target.optValue()); err != nil {
				return err
			}
			target.markSet()
			continue
		}
		if err := decodeStrictValue(decoder, value); err != nil {
			return err
		}
	}
	if _, err := decoder.ReadToken(); err != nil {
		return appError(err.Error())
	}
	for _, field := range fields {
		if seen[field.name] {
			continue
		}
		absentAllowed := field.presence == jsonPresenceTagDefault ||
			(field.isOpt && field.presence != jsonPresenceTagNullable)
		if !absentAllowed {
			return appError("missing field `" + field.name + "`")
		}
	}
	return nil
}

func decodeStrictSlice(decoder *jsontext.Decoder, destination reflect.Value) error {
	if kind := decoder.PeekKind(); kind != '[' {
		return appErrorf("invalid type: expected a JSON array for %s, found %v", destination.Type(), kind)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return appError(err.Error())
	}
	elements := reflect.MakeSlice(destination.Type(), 0, 0)
	for decoder.PeekKind() != ']' {
		element := reflect.New(destination.Type().Elem()).Elem()
		if err := decodeStrictValue(decoder, element); err != nil {
			return err
		}
		elements = reflect.Append(elements, element)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return appError(err.Error())
	}
	destination.Set(elements)
	return nil
}

// decodeTagged decodes an internally tagged JSON object: the string member named tag selects the
// variant constructor, and every other member decodes strictly into that variant.
func decodeTagged[T any](data []byte, tag string, variants map[string]func() T) (T, error) {
	var zero T
	decoder := jsontext.NewDecoder(bytes.NewReader(data))
	if kind := decoder.PeekKind(); kind != '{' {
		return zero, appErrorf("invalid type: expected a JSON object, found %v", kind)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return zero, appError(err.Error())
	}
	rest := []byte{'{'}
	variantName, tagFound := "", false
	for decoder.PeekKind() != '}' {
		token, err := decoder.ReadToken()
		if err != nil {
			return zero, appError(err.Error())
		}
		name := token.String()
		value, err := decoder.ReadValue()
		if err != nil {
			return zero, appError(err.Error())
		}
		if name == tag {
			if value.Kind() != '"' {
				return zero, appErrorf("invalid type: the `%s` tag is not a string", tag)
			}
			if err := json.Unmarshal(value, &variantName); err != nil {
				return zero, appError(err.Error())
			}
			tagFound = true
			continue
		}
		key, err := encodeJSON(name)
		if err != nil {
			return zero, err
		}
		if len(rest) > 1 {
			rest = append(rest, ',')
		}
		rest = append(append(append(rest, key...), ':'), value...)
	}
	if _, err := decoder.ReadToken(); err != nil {
		return zero, appError(err.Error())
	}
	if _, err := decoder.ReadToken(); err != io.EOF {
		return zero, appError("trailing characters after the JSON value")
	}
	if !tagFound {
		return zero, appError("missing field `" + tag + "`")
	}
	construct, known := variants[variantName]
	if !known {
		return zero, appError("unknown variant `" + variantName + "`")
	}
	variant := construct()
	if err := decodeStrict(append(rest, '}'), variant); err != nil {
		return zero, err
	}
	return variant, nil
}

// appendJSONInteger appends a JSON number literal, which SPEC §6.4 restricts to decimal
// integers.
func appendJSONInteger(out []byte, literal string) ([]byte, error) {
	digits := strings.TrimPrefix(literal, "-")
	if digits == "" || strings.Trim(digits, "0123456789") != "" || (len(digits) > 1 && digits[0] == '0') {
		return nil, appErrorf("a JSON number in a cafleet document is a decimal integer, got %q", literal)
	}
	return append(out, literal...), nil
}

// lossyUTF8 decodes data as UTF-8, replacing each maximal ill-formed subsequence with one U+FFFD
// (F8).
func lossyUTF8(data []byte) string {
	var out strings.Builder
	out.Grow(len(data))
	for len(data) > 0 {
		r, size := utf8.DecodeRune(data)
		if r != utf8.RuneError || size > 1 {
			out.Write(data[:size])
			data = data[size:]
			continue
		}
		out.WriteRune(utf8.RuneError)
		data = data[maximalIllFormedPrefix(data):]
	}
	return out.String()
}

// maximalIllFormedPrefix is the length of the maximal prefix of data that a valid UTF-8 sequence
// could start with, given that data does not start with a valid sequence; it is at least 1.
func maximalIllFormedPrefix(data []byte) int {
	lead := data[0]
	var need int
	low, high := byte(0x80), byte(0xbf)
	switch {
	case lead >= 0xc2 && lead <= 0xdf:
		need = 1
	case lead == 0xe0:
		need, low = 2, 0xa0
	case lead >= 0xe1 && lead <= 0xec, lead == 0xee, lead == 0xef:
		need = 2
	case lead == 0xed:
		need, high = 2, 0x9f
	case lead == 0xf0:
		need, low = 3, 0x90
	case lead >= 0xf1 && lead <= 0xf3:
		need = 3
	case lead == 0xf4:
		need, high = 3, 0x8f
	default:
		return 1
	}
	length := 1
	for ; length <= need && length < len(data); length++ {
		b := data[length]
		if b < low || b > high {
			break
		}
		low, high = 0x80, 0xbf
	}
	return length
}

// lines splits s as Rust's str::lines does (F10): at each "\n", stripping a "\r" that directly
// precedes it; a final line ending yields no empty last line.
func lines(s string) []string {
	out := []string{}
	for s != "" {
		end := strings.IndexByte(s, '\n')
		if end < 0 {
			out = append(out, s)
			break
		}
		out = append(out, strings.TrimSuffix(s[:end], "\r"))
		s = s[end+1:]
	}
	return out
}

// joinPath appends element to base without cleaning either (F11): an absolute element replaces
// base, and a separator is inserted only when base is nonempty and does not already end in one.
func joinPath(base, element string) string {
	switch {
	case strings.HasPrefix(element, "/"):
		return element
	case base == "" || strings.HasSuffix(base, "/"):
		return base + element
	default:
		return base + "/" + element
	}
}

// osErrorText renders an operating-system error cause as the platform's description with its
// first letter capitalized, followed by " (os error N)" (F18, SPEC §6.4 Cause texts).
func osErrorText(errno syscall.Errno) string {
	description := errno.Error()
	first, size := utf8.DecodeRuneInString(description)
	return string(unicode.ToUpper(first)) + description[size:] + " (os error " + strconv.Itoa(int(errno)) + ")"
}

// ioErrorText renders an I/O cause: an operating-system error through osErrorText, any other
// error by its own text.
func ioErrorText(err error) string {
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return osErrorText(errno)
	}
	return err.Error()
}

// exitStatusText renders a child's wait status (F18, SPEC §6.4 Cause texts).
func exitStatusText(status syscall.WaitStatus) string {
	switch {
	case status.Exited():
		return "exit status: " + strconv.Itoa(status.ExitStatus())
	case status.Signaled():
		text := "signal: " + signalText(status.Signal())
		if status.CoreDump() {
			text += " (core dumped)"
		}
		return text
	case status.Stopped():
		return "stopped (not terminated) by signal: " + signalText(status.StopSignal())
	case status.Continued():
		return "continued (WIFCONTINUED)"
	default:
		return fmt.Sprintf("unrecognised wait status: %d %#x", int(status), int(status))
	}
}

// signalText is the signal number followed by its name in parentheses, or the number alone for
// an unnamed signal.
func signalText(sig syscall.Signal) string {
	text := strconv.Itoa(int(sig))
	if name := unix.SignalName(sig); name != "" {
		text += " (" + name + ")"
	}
	return text
}

// parseRFC3339 parses an RFC 3339 timestamp with chrono's grammar (F17): `T`, `t` or a space
// between date and time, an optional fraction of one or more digits, `Z`, `z` or a `±HH:MM`
// offset, and second 60 as a leap second. The result keeps the parsed offset as its zone. chrono
// orders a leap second after every instant of the second before it and before the next minute,
// so a leap second becomes the last nanosecond of that preceding second.
func parseRFC3339(value string) (time.Time, bool) {
	cursor := rfc3339Cursor{text: value}
	year := cursor.digits(4)
	cursor.expect("-")
	month := cursor.digits(2)
	cursor.expect("-")
	day := cursor.digits(2)
	cursor.expect("Tt ")
	hour := cursor.digits(2)
	cursor.expect(":")
	minute := cursor.digits(2)
	cursor.expect(":")
	second := cursor.digits(2)
	nanosecond := 0
	if cursor.accept(".") {
		nanosecond = cursor.fraction()
	}
	offsetSeconds := cursor.offset()
	if cursor.failed || cursor.position != len(value) {
		return time.Time{}, false
	}
	if month < 1 || month > 12 || day < 1 || day > daysIn(year, month) ||
		hour > 23 || minute > 59 || second > 60 {
		return time.Time{}, false
	}
	if second == 60 {
		second, nanosecond = 59, 999_999_999
	}
	zone := time.UTC
	if offsetSeconds != 0 {
		zone = time.FixedZone("", offsetSeconds)
	}
	return time.Date(year, time.Month(month), day, hour, minute, second, nanosecond, zone), true
}

func daysIn(year, month int) int {
	return time.Date(year, time.Month(month)+1, 0, 0, 0, 0, 0, time.UTC).Day()
}

// rfc3339Cursor reads a timestamp left to right; after the first mismatch failed is set and
// every later read is a no-op returning zero.
type rfc3339Cursor struct {
	text     string
	position int
	failed   bool
}

func (c *rfc3339Cursor) digits(count int) int {
	if c.failed || c.position+count > len(c.text) {
		c.failed = true
		return 0
	}
	value := 0
	for _, b := range []byte(c.text[c.position : c.position+count]) {
		if b < '0' || b > '9' {
			c.failed = true
			return 0
		}
		value = value*10 + int(b-'0')
	}
	c.position += count
	return value
}

// accept consumes one byte from set when the next byte is in it.
func (c *rfc3339Cursor) accept(set string) bool {
	if c.failed || c.position >= len(c.text) || strings.IndexByte(set, c.text[c.position]) < 0 {
		return false
	}
	c.position++
	return true
}

func (c *rfc3339Cursor) expect(set string) {
	if !c.accept(set) {
		c.failed = true
	}
}

// fraction reads one or more digits as nanoseconds, keeping the first nine.
func (c *rfc3339Cursor) fraction() int {
	start := c.position
	nanosecond, scale := 0, 100_000_000
	for !c.failed && c.position < len(c.text) && c.text[c.position] >= '0' && c.text[c.position] <= '9' {
		nanosecond += int(c.text[c.position]-'0') * scale
		scale /= 10
		c.position++
	}
	if c.position == start {
		c.failed = true
	}
	return nanosecond
}

// offset reads `Z`, `z` or `±HH:MM` as seconds east of UTC.
func (c *rfc3339Cursor) offset() int {
	if c.accept("Zz") {
		return 0
	}
	sign := 1
	switch {
	case c.accept("+"):
	case c.accept("-"):
		sign = -1
	default:
		c.failed = true
		return 0
	}
	hours := c.digits(2)
	c.expect(":")
	minutes := c.digits(2)
	if hours > 23 || minutes > 59 {
		c.failed = true
	}
	return sign * (hours*3600 + minutes*60)
}
