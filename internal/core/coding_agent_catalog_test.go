package core

import (
	"bytes"
	stdjson "encoding/json"
	"encoding/json/jsontext"
	"fmt"
	"io"
	"math"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"testing"
	"unicode/utf8"

	"github.com/himkt/cafleet/internal/vt"
)

// The catalog tests build profile files as JSON values the way the Rust tests build
// serde_json::Value with preserve_order: objects keep member order, numbers keep their literal,
// and indexing a missing member reads null while assigning through one creates it.

type jsonNumber string

type jsonObj struct{ members []jsonMember }

type jsonArr struct{ items []any }

func jsonNormalize(value any) any {
	switch v := value.(type) {
	case nil, bool, string, jsonNumber, *jsonObj, *jsonArr:
		return v
	case int:
		return jsonNumber(strconv.Itoa(v))
	case int64:
		return jsonNumber(strconv.FormatInt(v, 10))
	case uint16:
		return jsonNumber(strconv.FormatUint(uint64(v), 10))
	case uint64:
		return jsonNumber(strconv.FormatUint(v, 10))
	case float64:
		return jsonNumber(strconv.FormatFloat(v, 'g', -1, 64))
	case []string:
		array := &jsonArr{}
		for _, item := range v {
			array.items = append(array.items, item)
		}
		return array
	case []any:
		array := &jsonArr{}
		for _, item := range v {
			array.items = append(array.items, jsonNormalize(item))
		}
		return array
	default:
		panic(fmt.Sprintf("%T is not a JSON test value", value))
	}
}

// jo builds an object from alternating keys and values.
func jo(pairs ...any) *jsonObj {
	if len(pairs)%2 != 0 {
		panic("jo takes alternating keys and values")
	}
	object := &jsonObj{}
	for index := 0; index < len(pairs); index += 2 {
		key, ok := pairs[index].(string)
		if !ok {
			panic(fmt.Sprintf("object key %v is not a string", pairs[index]))
		}
		object.set(key, pairs[index+1])
	}
	return object
}

func ja(items ...any) *jsonArr {
	array := &jsonArr{}
	for _, item := range items {
		array.items = append(array.items, jsonNormalize(item))
	}
	return array
}

func (o *jsonObj) index(key string) int {
	return slices.IndexFunc(o.members, func(member jsonMember) bool { return member.key == key })
}

func (o *jsonObj) get(key string) (any, bool) {
	if at := o.index(key); at >= 0 {
		return o.members[at].value, true
	}
	return nil, false
}

// set replaces the member's value in place, or appends a new member, as IndexMap::insert does.
func (o *jsonObj) set(key string, value any) {
	value = jsonNormalize(value)
	if at := o.index(key); at >= 0 {
		o.members[at].value = value
		return
	}
	o.members = append(o.members, jsonMember{key: key, value: value})
}

// remove deletes the member and keeps the order of the rest, as IndexMap::shift_remove does.
func (o *jsonObj) remove(key string) (any, bool) {
	at := o.index(key)
	if at < 0 {
		return nil, false
	}
	value := o.members[at].value
	o.members = slices.Delete(o.members, at, at+1)
	return value, true
}

func (o *jsonObj) keys() []string {
	keys := make([]string, len(o.members))
	for index, member := range o.members {
		keys[index] = member.key
	}
	return keys
}

// jget reads value[path[0]][path[1]]…; a missing member, an index out of range or a step into a
// scalar reads null.
func jget(value any, path ...any) any {
	for _, step := range path {
		switch key := step.(type) {
		case string:
			object, ok := value.(*jsonObj)
			if !ok {
				return nil
			}
			value, _ = object.get(key)
		case int:
			array, ok := value.(*jsonArr)
			if !ok || key < 0 || key >= len(array.items) {
				return nil
			}
			value = array.items[key]
		default:
			panic(fmt.Sprintf("path step %v is neither a key nor an index", step))
		}
	}
	return value
}

// jset assigns value at path below root, creating an object for each missing or null member on
// the way, as serde_json's IndexMut does; an index out of range panics.
func jset(root any, value any, path ...any) {
	if len(path) == 0 {
		panic("jset needs a path")
	}
	parent := root
	for depth, step := range path {
		last := depth == len(path)-1
		switch key := step.(type) {
		case string:
			object, ok := parent.(*jsonObj)
			if !ok {
				panic(fmt.Sprintf("cannot index %T with %q", parent, key))
			}
			if last {
				object.set(key, value)
				return
			}
			child, _ := object.get(key)
			if child == nil {
				child = &jsonObj{}
				object.set(key, child)
			}
			parent = child
		case int:
			array, ok := parent.(*jsonArr)
			if !ok || key < 0 || key >= len(array.items) {
				panic(fmt.Sprintf("cannot index %T with %d", parent, key))
			}
			if last {
				array.items[key] = jsonNormalize(value)
				return
			}
			parent = array.items[key]
		default:
			panic(fmt.Sprintf("path step %v is neither a key nor an index", step))
		}
	}
}

func jpointerPath(pointer string) []any {
	if pointer == "" {
		return nil
	}
	if !strings.HasPrefix(pointer, "/") {
		panic("a JSON pointer starts with /")
	}
	var path []any
	for _, segment := range strings.Split(pointer[1:], "/") {
		path = append(path, strings.ReplaceAll(strings.ReplaceAll(segment, "~1", "/"), "~0", "~"))
	}
	return path
}

// jpointer resolves an RFC 6901 pointer, reading array segments as indexes.
func jpointer(value any, pointer string) (any, bool) {
	for _, segment := range jpointerPath(pointer) {
		key, _ := segment.(string)
		switch container := value.(type) {
		case *jsonObj:
			child, ok := container.get(key)
			if !ok {
				return nil, false
			}
			value = child
		case *jsonArr:
			index, err := strconv.Atoi(key)
			if err != nil || index < 0 || index >= len(container.items) {
				return nil, false
			}
			value = container.items[index]
		default:
			return nil, false
		}
	}
	return value, true
}

// jpointerSet replaces the existing value at pointer, as `*value.pointer_mut(p).unwrap() = v`.
func jpointerSet(root any, pointer string, value any) {
	path := jpointerPath(pointer)
	if len(path) == 0 {
		panic("jpointerSet cannot replace the root")
	}
	parentPointer := pointer[:strings.LastIndex(pointer, "/")]
	parent, ok := jpointer(root, parentPointer)
	if !ok {
		panic("the pointer names an existing value: " + pointer)
	}
	key, _ := path[len(path)-1].(string)
	switch container := parent.(type) {
	case *jsonObj:
		if _, exists := container.get(key); !exists {
			panic("the pointer names an existing value: " + pointer)
		}
		container.set(key, value)
	case *jsonArr:
		index, err := strconv.Atoi(key)
		if err != nil || index < 0 || index >= len(container.items) {
			panic("the pointer names an existing value: " + pointer)
		}
		container.items[index] = jsonNormalize(value)
	default:
		panic("the pointer names an existing value: " + pointer)
	}
}

func jobject(value any) *jsonObj {
	object, ok := value.(*jsonObj)
	if !ok {
		panic(fmt.Sprintf("%s is not an object", testJSONText(value)))
	}
	return object
}

func jarray(value any) *jsonArr {
	array, ok := value.(*jsonArr)
	if !ok {
		panic(fmt.Sprintf("%s is not an array", testJSONText(value)))
	}
	return array
}

func (a *jsonArr) push(value any) { a.items = append(a.items, jsonNormalize(value)) }

func jclone(value any) any {
	switch v := value.(type) {
	case *jsonObj:
		object := &jsonObj{members: make([]jsonMember, len(v.members))}
		for index, member := range v.members {
			object.members[index] = jsonMember{key: member.key, value: jclone(member.value)}
		}
		return object
	case *jsonArr:
		array := &jsonArr{items: make([]any, len(v.items))}
		for index, item := range v.items {
			array.items[index] = jclone(item)
		}
		return array
	default:
		return jsonNormalize(v)
	}
}

func jstring(value any) (string, bool) {
	text, ok := value.(string)
	return text, ok
}

func ju64(value any) (uint64, bool) {
	number, ok := value.(jsonNumber)
	if !ok {
		return 0, false
	}
	parsed, err := strconv.ParseUint(string(number), 10, 64)
	return parsed, err == nil
}

func jnumberEqual(a, b jsonNumber) bool {
	if a == b {
		return true
	}
	if ia, errA := strconv.ParseInt(string(a), 10, 64); errA == nil {
		ib, errB := strconv.ParseInt(string(b), 10, 64)
		return errB == nil && ia == ib
	}
	if ua, errA := strconv.ParseUint(string(a), 10, 64); errA == nil {
		ub, errB := strconv.ParseUint(string(b), 10, 64)
		return errB == nil && ua == ub
	}
	fa, errA := strconv.ParseFloat(string(a), 64)
	fb, errB := strconv.ParseFloat(string(b), 64)
	return errA == nil && errB == nil && fa == fb && strings.ContainsAny(string(b), ".eE")
}

// jequal is serde_json Value equality: objects compare as maps, regardless of member order.
func jequal(a, b any) bool {
	switch x := jsonNormalize(a).(type) {
	case nil:
		return jsonNormalize(b) == nil
	case bool, string:
		return x == jsonNormalize(b)
	case jsonNumber:
		y, ok := jsonNormalize(b).(jsonNumber)
		return ok && jnumberEqual(x, y)
	case *jsonArr:
		y, ok := jsonNormalize(b).(*jsonArr)
		return ok && slices.EqualFunc(x.items, y.items, jequal)
	case *jsonObj:
		y, ok := jsonNormalize(b).(*jsonObj)
		if !ok || len(x.members) != len(y.members) {
			return false
		}
		for _, member := range x.members {
			other, found := y.get(member.key)
			if !found || !jequal(member.value, other) {
				return false
			}
		}
		return true
	default:
		panic("invalid JSON test value")
	}
}

func jsonWrite(encoder *jsontext.Encoder, value any) error {
	switch v := jsonNormalize(value).(type) {
	case nil:
		return encoder.WriteToken(jsontext.Null)
	case bool:
		return encoder.WriteToken(jsontext.Bool(v))
	case string:
		return encoder.WriteToken(jsontext.String(v))
	case jsonNumber:
		return encoder.WriteValue(jsontext.Value(v))
	case *jsonArr:
		if err := encoder.WriteToken(jsontext.BeginArray); err != nil {
			return err
		}
		for _, item := range v.items {
			if err := jsonWrite(encoder, item); err != nil {
				return err
			}
		}
		return encoder.WriteToken(jsontext.EndArray)
	case *jsonObj:
		if err := encoder.WriteToken(jsontext.BeginObject); err != nil {
			return err
		}
		for _, member := range v.members {
			if err := encoder.WriteToken(jsontext.String(member.key)); err != nil {
				return err
			}
			if err := jsonWrite(encoder, member.value); err != nil {
				return err
			}
		}
		return encoder.WriteToken(jsontext.EndObject)
	default:
		panic("invalid JSON test value")
	}
}

// jsonBytes is the compact encoding of value in member order.
func jsonBytes(value any) []byte {
	var out bytes.Buffer
	encoder := jsontext.NewEncoder(&out)
	if err := jsonWrite(encoder, value); err != nil {
		panic(fmt.Sprintf("encoding a JSON test value: %v", err))
	}
	return bytes.TrimSuffix(out.Bytes(), []byte("\n"))
}

func testJSONText(value any) string { return string(jsonBytes(value)) }

// parseJSONValue parses one JSON text in member order. A repeated key keeps its first position
// and its last value, as serde_json's preserve_order map does; duplicate reports the first key
// repeated at any level.
func parseJSONValue(data []byte) (value any, duplicate opt[string], err error) {
	decoder := jsontext.NewDecoder(bytes.NewReader(data), jsontext.AllowDuplicateNames(true))
	value, duplicate, err = parseJSONFrom(decoder)
	if err != nil {
		return nil, duplicate, err
	}
	if _, err := decoder.ReadToken(); err != io.EOF {
		return nil, duplicate, fmt.Errorf("trailing characters after the JSON value")
	}
	return value, duplicate, nil
}

func parseJSONFrom(decoder *jsontext.Decoder) (any, opt[string], error) {
	duplicate := none[string]()
	token, err := decoder.ReadToken()
	if err != nil {
		return nil, duplicate, err
	}
	switch token.Kind() {
	case 'n':
		return nil, duplicate, nil
	case 't', 'f':
		return token.Bool(), duplicate, nil
	case '"':
		return token.String(), duplicate, nil
	case '0':
		return jsonNumber(token.String()), duplicate, nil
	case '[':
		array := &jsonArr{}
		for decoder.PeekKind() != ']' {
			item, inner, err := parseJSONFrom(decoder)
			if err != nil {
				return nil, duplicate, err
			}
			if _, set := duplicate.get(); !set {
				duplicate = inner
			}
			array.items = append(array.items, item)
		}
		_, err := decoder.ReadToken()
		return array, duplicate, err
	case '{':
		object := &jsonObj{}
		for decoder.PeekKind() != '}' {
			token, err := decoder.ReadToken()
			if err != nil {
				return nil, duplicate, err
			}
			key := token.String()
			item, inner, err := parseJSONFrom(decoder)
			if err != nil {
				return nil, duplicate, err
			}
			if _, set := duplicate.get(); !set {
				duplicate = inner
			}
			if _, exists := object.get(key); exists {
				if _, set := duplicate.get(); !set {
					duplicate = some(key)
				}
			}
			object.set(key, item)
		}
		_, err := decoder.ReadToken()
		return object, duplicate, err
	case jsontext.KindInvalid, jsontext.KindEndObject, jsontext.KindEndArray:
		return nil, duplicate, fmt.Errorf("unexpected JSON token %v", token.Kind())
	default:
		panic("invalid jsontext.Kind")
	}
}

func parseTestJSON(text string) any {
	value, _, err := parseJSONValue([]byte(text))
	if err != nil {
		panic(fmt.Sprintf("test JSON %s: %v", text, err))
	}
	return value
}

// jsonf is json!: each %s in format takes the JSON text of the matching argument.
func jsonf(format string, args ...any) any {
	texts := make([]any, len(args))
	for index, arg := range args {
		texts[index] = testJSONText(arg)
	}
	return parseTestJSON(fmt.Sprintf(format, texts...))
}

func TestCodingAgentCatalog_JSONTestValuesKeepMemberOrderAndSerdeIndexing(t *testing.T) {
	t.Parallel()
	value := parseTestJSON(`{"b":1,"a":{"x":[true,null,"s"]},"n":1.5,"big":18446744073709551615}`)
	if got := testJSONText(value); got != `{"b":1,"a":{"x":[true,null,"s"]},"n":1.5,"big":18446744073709551615}` {
		t.Fatalf("round trip %s", got)
	}
	if got := jget(value, "a", "x", 2); got != "s" {
		t.Fatalf("jget %v", got)
	}
	if got := jget(value, "missing", 0, "deeper"); got != nil {
		t.Fatalf("a missing path reads %v", got)
	}
	jset(value, "new", "c", "d")
	jset(value, 2, "b")
	if got := testJSONText(value); got != `{"b":2,"a":{"x":[true,null,"s"]},"n":1.5,"big":18446744073709551615,"c":{"d":"new"}}` {
		t.Fatalf("after jset %s", got)
	}
	jpointerSet(value, "/a/x/1", false)
	if got, ok := jpointer(value, "/a/x/1"); !ok || got != false {
		t.Fatalf("jpointer %v %v", got, ok)
	}
	if !jequal(parseTestJSON(`{"p":1,"q":[2]}`), parseTestJSON(`{"q":[2],"p":1}`)) || jequal(parseTestJSON(`[1,2]`), parseTestJSON(`[2,1]`)) {
		t.Fatal("jequal compares objects as maps and arrays in order")
	}
	_, duplicate, err := parseJSONValue([]byte(`{"a":{"k":1,"k":2},"a":3}`))
	if err != nil || duplicate != some("k") {
		t.Fatalf("duplicate %v, %v", duplicate, err)
	}
	if got := testJSONText(jsonf(`{"id":%s,"rows":%s}`, "a\x1b\"b", math.MaxUint16)); got != `{"id":"a\u001b\"b","rows":65535}` {
		t.Fatalf("jsonf %s", got)
	}
}

// The Rust catalog test_support module: the bundled-catalog guard and the synthetic profile
// builders shared by the catalog, detector and runner tests.

func guardObject(value any, keys ...string) (*jsonObj, error) {
	object, ok := value.(*jsonObj)
	if !ok {
		return nil, invalidProfile("object required")
	}
	exact := len(object.members) == len(keys)
	for _, key := range keys {
		if _, found := object.get(key); !found {
			exact = false
		}
	}
	if err := require(exact, "exact required object fields expected"); err != nil {
		return nil, err
	}
	return object, nil
}

func guardText(value any) (string, error) {
	text, ok := value.(string)
	if !ok {
		return "", invalidProfile("text required")
	}
	if err := require(strings.TrimSpace(text) != "", "nonblank text required"); err != nil {
		return "", err
	}
	return text, nil
}

func guardArray(value any) ([]any, error) {
	array, ok := value.(*jsonArr)
	if !ok {
		return nil, invalidProfile("array required")
	}
	return array.items, nil
}

func guardBoolean(value any) (bool, error) {
	flag, ok := value.(bool)
	if !ok {
		return false, invalidProfile("boolean required")
	}
	return flag, nil
}

func guardPositive(value any) (uint16, error) {
	number, ok := ju64(value)
	if !ok || number == 0 || number > math.MaxUint16 {
		return 0, invalidProfile("positive viewport bound required")
	}
	return uint16(number), nil
}

// parseBundledText parses a profile file's text, rejecting an object that repeats a key, which
// a plain parse would collapse to its last value.
func parseBundledText(text []byte) (any, error) {
	value, duplicate, err := parseJSONValue(text)
	if err != nil {
		return nil, invalidProfile(err.Error())
	}
	if key, repeated := duplicate.get(); repeated {
		return nil, invalidProfile("duplicate object key " + key)
	}
	return value, nil
}

// bundledFiles is every embedded profile file, keyed by its backend name; the caller owns the
// returned copy.
func bundledFiles() *jsonObj {
	return jobject(jclone(parsedBundledFiles()))
}

// parsedBundledFiles is the shared parse of the embedded profile files; callers only read it.
var parsedBundledFiles = sync.OnceValue(func() *jsonObj {
	files := &jsonObj{}
	for _, backend := range codingAgentAll() {
		text, ok := bundledProfileText(backend)
		if !ok {
			continue
		}
		file, err := parseBundledText(text)
		if err != nil {
			panic("the bundled-catalog guard test reports a malformed file: " + err.Error())
		}
		files.set(backend.name(), file)
	}
	return files
})

const syntaxLetterEscapes = "tnrfvxpAz"

func isASCIIPunctuation(r rune) bool {
	return r < 0x80 && strings.ContainsRune("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~", r)
}

// escapesOutsideSyntax is each escape of pattern outside the pattern syntax, spelled as written.
func escapesOutsideSyntax(pattern string) []string {
	var found []string
	rest := pattern
	for {
		at := strings.IndexByte(rest, '\\')
		if at < 0 {
			return found
		}
		tail := rest[at+1:]
		if tail == "" {
			return append(found, "\\")
		}
		letter, size := utf8.DecodeRuneInString(tail)
		after := tail[size:]
		if !isASCIIPunctuation(letter) && !strings.ContainsRune(syntaxLetterEscapes, letter) {
			braced := 0
			if strings.HasPrefix(after, "{") {
				if closing := strings.IndexByte(after, '}'); closing >= 0 {
					braced = closing + 1
				}
			}
			found = append(found, "\\"+string(letter)+after[:braced])
		}
		rest = after
	}
}

type profileString struct {
	pointer string
	text    string
}

func profileStrings(value any, pointer string, found *[]profileString) {
	switch v := value.(type) {
	case string:
		*found = append(*found, profileString{pointer: pointer, text: v})
	case *jsonArr:
		for index, item := range v.items {
			profileStrings(item, pointer+"/"+strconv.Itoa(index), found)
		}
	case *jsonObj:
		for _, member := range v.members {
			key := strings.ReplaceAll(strings.ReplaceAll(member.key, "~", "~0"), "/", "~1")
			profileStrings(member.value, pointer+"/"+key, found)
		}
	}
}

// decodeRawProfile is the Rust RawProfile::deserialize: the strict decode alone, before any
// profile validation.
func decodeRawProfile(profile any) error {
	var raw rawProfile
	return decodeStrict(jsonBytes(profile), &raw)
}

// decodesAsPattern is whether the catalog decoder compiles the profile string at pointer as a
// pattern.
func decodesAsPattern(profile any, pointer string) bool {
	probe := jclone(profile)
	jpointerSet(probe, pointer, "(")
	err := decodeRawProfile(probe)
	return err != nil && strings.Contains(err.Error(), "regex: ")
}

func patternLocation(profile any, pointer string) string {
	segments := strings.Split(strings.TrimPrefix(pointer, "/"), "/")
	if segments[0] == "rules" && len(segments) > 1 {
		index, err := strconv.Atoi(segments[1])
		if err != nil {
			panic("a rule index")
		}
		id, ok := jstring(jget(profile, "rules", index, "id"))
		if !ok {
			panic("a rule id")
		}
		return "rule " + id
	}
	return segments[0]
}

// implicitPatternClasses is one "<backend> <location>: `<construct>`" line per Perl class,
// boundary or other escape outside the pattern syntax in a profile string the decoder compiles.
func implicitPatternClasses(files any) []string {
	var found []string
	for _, member := range jobject(files).members {
		file := member.value
		profile := jget(file, "profile")
		if err := decodeRawProfile(profile); err != nil {
			panic("the profile decodes before probing: " + err.Error())
		}
		name, err := guardText(jget(file, "backend"))
		if err != nil {
			panic("profile file backend: " + err.Error())
		}
		var strs []profileString
		profileStrings(profile, "", &strs)
		for _, s := range strs {
			constructs := escapesOutsideSyntax(s.text)
			if len(constructs) == 0 || !decodesAsPattern(profile, s.pointer) {
				continue
			}
			location := patternLocation(profile, s.pointer)
			for _, construct := range constructs {
				found = append(found, name+" "+location+": `"+construct+"`")
			}
		}
	}
	return found
}

// verifyCursorFreeScreen: a herdr capture records neither the cursor nor the screen mode, so all
// three fields are null together and the screen names `multiplexer: herdr`.
func verifyCursorFreeScreen(screen *jsonObj) error {
	nulls := true
	for _, field := range []string{"alternate_screen", "cursor_row", "cursor_col"} {
		if value, _ := screen.get(field); value != nil {
			nulls = false
		}
	}
	if err := require(nulls, "a cursor-free fixture records null cursor and screen mode together"); err != nil {
		return err
	}
	multiplexer, _ := screen.get("multiplexer")
	return require(multiplexer == "herdr", "a cursor-free fixture is a herdr capture")
}

var screenFields = []string{"rows", "cols", "alternate_screen", "screen_seq", "cursor_row", "cursor_col", "lines", "masked_rows"}

func objectField(object *jsonObj, key string) any {
	value, found := object.get(key)
	if !found {
		panic("the exact-field check admitted " + key)
	}
	return value
}

func verifyBundledScreen(screen any) error {
	fields := append([]string{"native_version"}, screenFields...)
	for _, optional := range []string{"composer_evidence", "cell_backgrounds", "cell_styles", "multiplexer"} {
		if object, ok := screen.(*jsonObj); ok {
			if _, present := object.get(optional); present {
				fields = append(fields, optional)
			}
		}
	}
	m, err := guardObject(screen, fields...)
	if err != nil {
		return err
	}
	_, backgrounds := m.get("cell_backgrounds")
	_, styles := m.get("cell_styles")
	rows, err := guardPositive(objectField(m, "rows"))
	if err != nil {
		return err
	}
	cols, err := guardPositive(objectField(m, "cols"))
	if err != nil {
		return err
	}
	if _, ok := ju64(objectField(m, "screen_seq")); !ok {
		return invalidProfile("fixture screen sequence required")
	}
	cursorFree := false
	for _, field := range []string{"alternate_screen", "cursor_row", "cursor_col"} {
		if objectField(m, field) == nil {
			cursorFree = true
		}
	}
	if cursorFree {
		if err := verifyCursorFreeScreen(m); err != nil {
			return err
		}
	} else {
		_, named := m.get("multiplexer")
		if err := require(!named, "only a cursor-free fixture names its multiplexer"); err != nil {
			return err
		}
		if _, err := guardBoolean(objectField(m, "alternate_screen")); err != nil {
			return err
		}
		row, rowOK := ju64(objectField(m, "cursor_row"))
		col, colOK := ju64(objectField(m, "cursor_col"))
		if err := require(rowOK && row < uint64(rows) && colOK && col <= uint64(cols), "fixture cursor outside its geometry"); err != nil {
			return err
		}
	}
	lines, err := guardArray(objectField(m, "lines"))
	if err != nil {
		return err
	}
	allStrings := true
	for _, line := range lines {
		if _, ok := line.(string); !ok {
			allStrings = false
		}
	}
	if err := require(len(lines) == int(rows) && allStrings, "fixture lines must cover every row"); err != nil {
		return err
	}
	masks, err := guardArray(objectField(m, "masked_rows"))
	if err != nil {
		return err
	}
	masked := map[uint64]bool{}
	for _, raw := range masks {
		mask, err := guardObject(raw, "row", "reason")
		if err != nil {
			return err
		}
		row, ok := ju64(objectField(mask, "row"))
		if !ok || row >= uint64(len(lines)) {
			return invalidProfile("masked row outside fixture geometry")
		}
		if err := require(!masked[row], "duplicate masked row"); err != nil {
			return err
		}
		masked[row] = true
		if err := require(objectField(mask, "reason") == "reviewed_privacy", "masked rows require reviewed privacy"); err != nil {
			return err
		}
		if err := require(lines[row] == "", "masked row must be empty"); err != nil {
			return err
		}
	}
	if backgrounds {
		if err := verifyCellBackgrounds(objectField(m, "cell_backgrounds"), rows, cols, masked); err != nil {
			return err
		}
	}
	if styles {
		if err := verifyCellStyles(objectField(m, "cell_styles"), rows, cols, masked); err != nil {
			return err
		}
	}
	return nil
}

func verifyColour(value any) error {
	channel := func(value any) error {
		number, ok := ju64(value)
		if !ok || number > 255 {
			return invalidProfile("fixture colour channel outside 0..=255")
		}
		return nil
	}
	switch jget(value, "kind") {
	case "indexed":
		m, err := guardObject(value, "kind", "index")
		if err != nil {
			return err
		}
		return channel(objectField(m, "index"))
	case "rgb":
		m, err := guardObject(value, "kind", "r", "g", "b")
		if err != nil {
			return err
		}
		for _, name := range []string{"r", "g", "b"} {
			if err := channel(objectField(m, name)); err != nil {
				return err
			}
		}
		return nil
	default:
		return invalidProfile("fixture colour must be indexed or rgb")
	}
}

type columnSpan struct{ start, end uint64 }

func spansOverlap(spans map[uint64][]columnSpan) bool {
	for _, runs := range spans {
		slices.SortFunc(runs, func(a, b columnSpan) int {
			if a.start != b.start {
				return cmpUint64(a.start, b.start)
			}
			return cmpUint64(a.end, b.end)
		})
		for index := 1; index < len(runs); index++ {
			if runs[index-1].end > runs[index].start {
				return true
			}
		}
	}
	return false
}

func cmpUint64(a, b uint64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	default:
		return 0
	}
}

func verifyCellStyles(value any, rows, cols uint16, masked map[uint64]bool) error {
	index := func(value any, bound uint64) (uint64, error) {
		number, ok := ju64(value)
		if !ok || number > bound {
			return 0, invalidProfile("style run coordinate outside its geometry")
		}
		return number, nil
	}
	runs, err := guardArray(value)
	if err != nil {
		return err
	}
	spans := map[uint64][]columnSpan{}
	for _, raw := range runs {
		run, err := guardObject(raw, "row", "start_col", "end_col", "faint", "foreground")
		if err != nil {
			return err
		}
		row, err := index(objectField(run, "row"), uint64(rows)-1)
		if err != nil {
			return err
		}
		start, err := index(objectField(run, "start_col"), uint64(cols))
		if err != nil {
			return err
		}
		end, err := index(objectField(run, "end_col"), uint64(cols))
		if err != nil {
			return err
		}
		if err := require(start < end, "style run requires start below end"); err != nil {
			return err
		}
		if err := require(!masked[row], "style run inside a masked row"); err != nil {
			return err
		}
		faint, err := guardBoolean(objectField(run, "faint"))
		if err != nil {
			return err
		}
		foreground := objectField(run, "foreground")
		if foreground != nil {
			if err := verifyColour(foreground); err != nil {
				return err
			}
		}
		if err := require(faint || foreground != nil, "style run requires a non-default attribute"); err != nil {
			return err
		}
		spans[row] = append(spans[row], columnSpan{start: start, end: end})
	}
	return require(!spansOverlap(spans), "style runs overlap")
}

func verifyCellBackgrounds(value any, rows, cols uint16, masked map[uint64]bool) error {
	index := func(value any, bound uint64) (uint64, error) {
		number, ok := ju64(value)
		if !ok || number > bound {
			return 0, invalidProfile("background run coordinate outside its geometry")
		}
		return number, nil
	}
	runs, err := guardArray(value)
	if err != nil {
		return err
	}
	spans := map[uint64][]columnSpan{}
	for _, raw := range runs {
		run, err := guardObject(raw, "row", "start_col", "end_col", "bg")
		if err != nil {
			return err
		}
		row, err := index(objectField(run, "row"), uint64(rows)-1)
		if err != nil {
			return err
		}
		start, err := index(objectField(run, "start_col"), uint64(cols))
		if err != nil {
			return err
		}
		end, err := index(objectField(run, "end_col"), uint64(cols))
		if err != nil {
			return err
		}
		if err := require(start < end, "background run requires start below end"); err != nil {
			return err
		}
		if err := require(!masked[row], "background run inside a masked row"); err != nil {
			return err
		}
		bg := objectField(run, "bg")
		switch jget(bg, "kind") {
		case "indexed":
			m, err := guardObject(bg, "kind", "index")
			if err != nil {
				return err
			}
			if _, err := index(objectField(m, "index"), 255); err != nil {
				return err
			}
		case "rgb":
			m, err := guardObject(bg, "kind", "r", "g", "b")
			if err != nil {
				return err
			}
			for _, channel := range []string{"r", "g", "b"} {
				if _, err := index(objectField(m, channel), 255); err != nil {
					return err
				}
			}
		default:
			return invalidProfile("background run requires a non-default colour")
		}
		spans[row] = append(spans[row], columnSpan{start: start, end: end})
	}
	return require(!spansOverlap(spans), "background runs overlap")
}

// The provenance view of a profile file, decoded with the serde rules of the Rust guard's
// derived types: unknown members are ignored, a plain optional member may be absent or null,
// and a nullable member must be present.

type evidenceMultiplexer uint8

const (
	evidenceMultiplexerTmux evidenceMultiplexer = iota + 1
	evidenceMultiplexerHerdr
)

func (m evidenceMultiplexer) debug() string {
	switch m {
	case evidenceMultiplexerTmux:
		return "Tmux"
	case evidenceMultiplexerHerdr:
		return "Herdr"
	default:
		panic("invalid evidenceMultiplexer")
	}
}

func (m evidenceMultiplexer) backend() muxBackend {
	switch m {
	case evidenceMultiplexerTmux:
		return muxBackendTmux
	case evidenceMultiplexerHerdr:
		return muxBackendHerdr
	default:
		panic("invalid evidenceMultiplexer")
	}
}

type composerEvidenceKind uint8

const (
	composerEvidenceKindFraming composerEvidenceKind = iota + 1
	composerEvidenceKindPlainLimit
	composerEvidenceKindPostSubmit
	composerEvidenceKindUnsubmittedLiteral
)

func (k composerEvidenceKind) debug() string {
	switch k {
	case composerEvidenceKindFraming:
		return "Framing"
	case composerEvidenceKindPlainLimit:
		return "PlainLimit"
	case composerEvidenceKindPostSubmit:
		return "PostSubmit"
	case composerEvidenceKindUnsubmittedLiteral:
		return "UnsubmittedLiteral"
	default:
		panic("invalid composerEvidenceKind")
	}
}

func composerOperationDebug(operation composerOperation) string {
	switch operation {
	case composerOperationPrompt:
		return "Prompt"
	case composerOperationExec:
		return "Exec"
	default:
		panic("invalid composerOperation")
	}
}

type evidenceIdentity struct {
	operation   composerOperation
	multiplexer evidenceMultiplexer
	kind        composerEvidenceKind
}

type composerEvidence struct {
	identity          evidenceIdentity
	literalFraming    literalFraming
	plainLiteralLimit opt[plainLiteralLimit]
	expectedLiteral   opt[string]
}

func (e composerEvidence) matchesPlan(plan *provenanceSubmission) bool {
	switch e.identity.kind {
	case composerEvidenceKindFraming:
		return e.literalFraming == plan.literalFraming
	case composerEvidenceKindPlainLimit:
		return e.plainLiteralLimit == plan.plainLiteralLimit
	case composerEvidenceKindPostSubmit, composerEvidenceKindUnsubmittedLiteral:
		return true
	default:
		panic("invalid composerEvidenceKind")
	}
}

func serdeMissing(field string) error {
	return invalidProfile("missing field `" + field + "`")
}

// serdeField reads a required member.
func serdeField(object *jsonObj, field string) (any, error) {
	value, found := object.get(field)
	if !found {
		return nil, serdeMissing(field)
	}
	return value, nil
}

// serdeOptional reads a plain Option member: absent and null are both none.
func serdeOptional(object *jsonObj, field string) (any, bool) {
	value, _ := object.get(field)
	return value, value != nil
}

func serdeObject(value any) (*jsonObj, error) {
	object, ok := value.(*jsonObj)
	if !ok {
		return nil, invalidProfile("invalid type: " + testJSONText(value) + ", expected a map")
	}
	return object, nil
}

func serdeDenyUnknown(object *jsonObj, fields ...string) error {
	for _, key := range object.keys() {
		if !slices.Contains(fields, key) {
			return invalidProfile("unknown field `" + key + "`")
		}
	}
	return nil
}

func serdeVariant[T any](value any, names []string, values []T) (T, error) {
	var zero T
	text, ok := value.(string)
	if !ok {
		return zero, invalidProfile("invalid type: " + testJSONText(value) + ", expected a variant")
	}
	if index := slices.Index(names, text); index >= 0 {
		return values[index], nil
	}
	return zero, invalidProfile("unknown variant `" + text + "`")
}

func serdeOperation(value any) (composerOperation, error) {
	return serdeVariant(value, []string{"prompt", "exec"}, []composerOperation{composerOperationPrompt, composerOperationExec})
}

func serdeMultiplexer(value any) (evidenceMultiplexer, error) {
	return serdeVariant(value, []string{"tmux", "herdr"}, []evidenceMultiplexer{evidenceMultiplexerTmux, evidenceMultiplexerHerdr})
}

func serdeFraming(value any) (literalFraming, error) {
	return serdeVariant(value, []string{"plain", "bracketed_paste"}, []literalFraming{literalFramingPlain, literalFramingBracketedPaste})
}

func serdeNonZero(value any) (uint64, error) {
	number, ok := ju64(value)
	if !ok || number == 0 {
		return 0, invalidProfile("invalid value: " + testJSONText(value) + ", expected a nonzero u64")
	}
	return number, nil
}

func serdeNonBlank(value any) (string, error) {
	text, ok := value.(string)
	if !ok {
		return "", invalidProfile("invalid type: " + testJSONText(value) + ", expected a string")
	}
	if strings.TrimSpace(text) == "" {
		return "", invalidProfile("nonblank text required")
	}
	return text, nil
}

// serdeIDs decodes nonblank, pairwise distinct strings.
func serdeIDs(value any) ([]string, error) {
	items, ok := value.(*jsonArr)
	if !ok {
		return nil, invalidProfile("invalid type: " + testJSONText(value) + ", expected a sequence")
	}
	var ids []string
	for _, item := range items.items {
		id, err := serdeNonBlank(item)
		if err != nil {
			return nil, err
		}
		if slices.Contains(ids, id) {
			return nil, invalidProfile("duplicate string identity")
		}
		ids = append(ids, id)
	}
	return ids, nil
}

func serdeLimit(value any) (opt[plainLiteralLimit], error) {
	if value == nil {
		return none[plainLiteralLimit](), nil
	}
	object, err := serdeObject(value)
	if err != nil {
		return none[plainLiteralLimit](), err
	}
	if err := serdeDenyUnknown(object, "unit", "max"); err != nil {
		return none[plainLiteralLimit](), err
	}
	rawUnit, err := serdeField(object, "unit")
	if err != nil {
		return none[plainLiteralLimit](), err
	}
	unit, err := serdeVariant(rawUnit, []string{"bytes", "scalars"}, []literalUnit{literalUnitBytes, literalUnitScalars})
	if err != nil {
		return none[plainLiteralLimit](), err
	}
	rawMax, err := serdeField(object, "max")
	if err != nil {
		return none[plainLiteralLimit](), err
	}
	limit, err := serdeNonZero(rawMax)
	if err != nil {
		return none[plainLiteralLimit](), err
	}
	return some(plainLiteralLimit{Unit: unit, Max: nonZeroU64(limit)}), nil
}

// serdeExpectedLiteral is the untagged ExpectedLiteral: the text itself, or prefix followed by
// repeat written count times.
func serdeExpectedLiteral(value any) (string, error) {
	if text, ok := value.(string); ok {
		return text, nil
	}
	failure := invalidProfile("data did not match any variant of untagged enum ExpectedLiteral")
	object, ok := value.(*jsonObj)
	if !ok || serdeDenyUnknown(object, "repeat", "count", "prefix") != nil {
		return "", failure
	}
	rawRepeat, errRepeat := serdeField(object, "repeat")
	rawCount, errCount := serdeField(object, "count")
	rawPrefix, errPrefix := serdeField(object, "prefix")
	if errRepeat != nil || errCount != nil || errPrefix != nil {
		return "", failure
	}
	repeat, err := serdeNonBlank(rawRepeat)
	if err != nil {
		return "", failure
	}
	count, err := serdeNonZero(rawCount)
	if err != nil {
		return "", failure
	}
	if count > math.MaxInt32 {
		panic("a repeat count fits in memory")
	}
	prefix, ok := rawPrefix.(string)
	if !ok {
		return "", failure
	}
	return prefix + strings.Repeat(repeat, int(count)), nil
}

func serdeComposerEvidence(value any) (composerEvidence, error) {
	object, err := serdeObject(value)
	if err != nil {
		return composerEvidence{}, err
	}
	rawKind, err := serdeField(object, "kind")
	if err != nil {
		return composerEvidence{}, err
	}
	kind, err := serdeVariant(rawKind, []string{"framing", "plain_limit", "post_submit", "unsubmitted_literal"},
		[]composerEvidenceKind{composerEvidenceKindFraming, composerEvidenceKindPlainLimit, composerEvidenceKindPostSubmit, composerEvidenceKindUnsubmittedLiteral})
	if err != nil {
		return composerEvidence{}, err
	}
	fields := []string{"kind", "operation", "multiplexer"}
	switch kind {
	case composerEvidenceKindFraming:
		fields = append(fields, "literal_framing")
	case composerEvidenceKindPlainLimit:
		fields = append(fields, "plain_literal_limit")
	case composerEvidenceKindUnsubmittedLiteral:
		fields = append(fields, "expected_literal")
	case composerEvidenceKindPostSubmit:
	}
	if err := serdeDenyUnknown(object, fields...); err != nil {
		return composerEvidence{}, err
	}
	evidence := composerEvidence{identity: evidenceIdentity{kind: kind}}
	for _, field := range fields[1:] {
		raw, err := serdeField(object, field)
		if err != nil {
			return composerEvidence{}, err
		}
		switch field {
		case "operation":
			evidence.identity.operation, err = serdeOperation(raw)
		case "multiplexer":
			evidence.identity.multiplexer, err = serdeMultiplexer(raw)
		case "literal_framing":
			evidence.literalFraming, err = serdeFraming(raw)
		case "plain_literal_limit":
			evidence.plainLiteralLimit, err = serdeLimit(raw)
		case "expected_literal":
			var literal string
			literal, err = serdeExpectedLiteral(raw)
			evidence.expectedLiteral = some(literal)
		}
		if err != nil {
			return composerEvidence{}, err
		}
	}
	return evidence, nil
}

type provenanceScreen struct {
	composerEvidence []composerEvidence
	hasEvidence      bool
}

type provenanceRule struct {
	id          string
	observation observationKind
	fixtureIDs  []string
	cited       bool
}

type provenanceSubmission struct {
	fixtureIDs        []string
	literalFraming    literalFraming
	plainLiteralLimit opt[plainLiteralLimit]
}

type provenanceCapability struct {
	ruleIDs    []string
	fixtureIDs []string
	submission bool
	prompt     *provenanceSubmission
	exec       *provenanceSubmission
}

type provenanceDirectorDelivery struct {
	fixtureIDs   []string
	multiplexers []evidenceMultiplexer
	plainLimit   opt[plainLiteralLimit]
}

type provenanceFile struct {
	backend          string
	rules            []provenanceRule
	detection        *provenanceCapability
	composer         *provenanceCapability
	answers          [][]string
	directorDelivery *provenanceDirectorDelivery
	screenIDs        []string
	screens          map[string]provenanceScreen
}

func serdeSubmission(value any) (*provenanceSubmission, error) {
	object, err := serdeObject(value)
	if err != nil {
		return nil, err
	}
	plan := &provenanceSubmission{}
	raw, err := serdeField(object, "fixture_ids")
	if err != nil {
		return nil, err
	}
	if plan.fixtureIDs, err = serdeIDs(raw); err != nil {
		return nil, err
	}
	if raw, err = serdeField(object, "literal_framing"); err != nil {
		return nil, err
	}
	if plan.literalFraming, err = serdeFraming(raw); err != nil {
		return nil, err
	}
	if raw, err = serdeField(object, "plain_literal_limit"); err != nil {
		return nil, err
	}
	if plan.plainLiteralLimit, err = serdeLimit(raw); err != nil {
		return nil, err
	}
	return plan, nil
}

func serdeCapability(value any) (*provenanceCapability, error) {
	object, err := serdeObject(value)
	if err != nil {
		return nil, err
	}
	capability := &provenanceCapability{}
	raw, err := serdeField(object, "rule_ids")
	if err != nil {
		return nil, err
	}
	if capability.ruleIDs, err = serdeIDs(raw); err != nil {
		return nil, err
	}
	if raw, err = serdeField(object, "fixture_ids"); err != nil {
		return nil, err
	}
	if capability.fixtureIDs, err = serdeIDs(raw); err != nil {
		return nil, err
	}
	submission, present := serdeOptional(object, "submission")
	if !present {
		return capability, nil
	}
	plans, err := serdeObject(submission)
	if err != nil {
		return nil, err
	}
	capability.submission = true
	if prompt, present := serdeOptional(plans, "prompt"); present {
		if capability.prompt, err = serdeSubmission(prompt); err != nil {
			return nil, err
		}
	}
	if exec, present := serdeOptional(plans, "exec"); present {
		if capability.exec, err = serdeSubmission(exec); err != nil {
			return nil, err
		}
	}
	return capability, nil
}

func serdeProvenanceProfile(file *provenanceFile, value any) error {
	profile, err := serdeObject(value)
	if err != nil {
		return err
	}
	rawRules, err := serdeField(profile, "rules")
	if err != nil {
		return err
	}
	rules, ok := rawRules.(*jsonArr)
	if !ok {
		return invalidProfile("invalid type: " + testJSONText(rawRules) + ", expected a sequence")
	}
	for _, item := range rules.items {
		object, err := serdeObject(item)
		if err != nil {
			return err
		}
		var r provenanceRule
		raw, err := serdeField(object, "id")
		if err != nil {
			return err
		}
		if r.id, err = serdeNonBlank(raw); err != nil {
			return err
		}
		if raw, err = serdeField(object, "observation"); err != nil {
			return err
		}
		text, ok := raw.(string)
		if !ok {
			return invalidProfile("invalid type: " + testJSONText(raw) + ", expected a string")
		}
		if r.observation, err = observationKindParse(text); err != nil {
			return invalidProfile(err.Error())
		}
		if cited, present := serdeOptional(object, "fixture_ids"); present {
			if r.fixtureIDs, err = serdeIDs(cited); err != nil {
				return err
			}
			r.cited = true
		}
		file.rules = append(file.rules, r)
	}
	rawCapabilities, err := serdeField(profile, "capabilities")
	if err != nil {
		return err
	}
	capabilities, err := serdeObject(rawCapabilities)
	if err != nil {
		return err
	}
	if detection, present := serdeOptional(capabilities, "detection"); present {
		if file.detection, err = serdeCapability(detection); err != nil {
			return err
		}
	}
	if composer, present := serdeOptional(capabilities, "composer"); present {
		if file.composer, err = serdeCapability(composer); err != nil {
			return err
		}
	}
	rawAnswers, err := serdeField(capabilities, "answers")
	if err != nil {
		return err
	}
	answers, ok := rawAnswers.(*jsonArr)
	if !ok {
		return invalidProfile("invalid type: " + testJSONText(rawAnswers) + ", expected a sequence")
	}
	for _, item := range answers.items {
		object, err := serdeObject(item)
		if err != nil {
			return err
		}
		raw, err := serdeField(object, "fixture_ids")
		if err != nil {
			return err
		}
		ids, err := serdeIDs(raw)
		if err != nil {
			return err
		}
		file.answers = append(file.answers, ids)
	}
	if raw, present := capabilities.get("director_delivery"); present {
		object, err := serdeObject(raw)
		if err != nil {
			return err
		}
		director := &provenanceDirectorDelivery{}
		value, err := serdeField(object, "fixture_ids")
		if err != nil {
			return err
		}
		if director.fixtureIDs, err = serdeIDs(value); err != nil {
			return err
		}
		if value, err = serdeField(object, "multiplexers"); err != nil {
			return err
		}
		multiplexers, ok := value.(*jsonArr)
		if !ok {
			return invalidProfile("invalid type: " + testJSONText(value) + ", expected a sequence")
		}
		for _, item := range multiplexers.items {
			multiplexer, err := serdeMultiplexer(item)
			if err != nil {
				return err
			}
			director.multiplexers = append(director.multiplexers, multiplexer)
		}
		if value, err = serdeField(object, "plain_limit"); err != nil {
			return err
		}
		if director.plainLimit, err = serdeLimit(value); err != nil {
			return err
		}
		file.directorDelivery = director
	}
	return nil
}

// serdeProvenanceFiles decodes the files in key order, as the Rust BTreeMap does.
func serdeProvenanceFiles(files any) ([]provenanceFile, error) {
	object, err := serdeObject(files)
	if err != nil {
		return nil, err
	}
	names := object.keys()
	slices.Sort(names)
	var decoded []provenanceFile
	for _, name := range names {
		raw, _ := object.get(name)
		fileObject, err := serdeObject(raw)
		if err != nil {
			return nil, err
		}
		file := provenanceFile{backend: name, screens: map[string]provenanceScreen{}}
		profile, err := serdeField(fileObject, "profile")
		if err != nil {
			return nil, err
		}
		if err := serdeProvenanceProfile(&file, profile); err != nil {
			return nil, err
		}
		rawScreens, err := serdeField(fileObject, "screens")
		if err != nil {
			return nil, err
		}
		screens, err := serdeObject(rawScreens)
		if err != nil {
			return nil, err
		}
		file.screenIDs = screens.keys()
		slices.Sort(file.screenIDs)
		for _, id := range file.screenIDs {
			rawScreen, _ := screens.get(id)
			screen, err := serdeObject(rawScreen)
			if err != nil {
				return nil, err
			}
			version, err := serdeField(screen, "native_version")
			if err != nil {
				return nil, err
			}
			text, ok := version.(string)
			if !ok {
				return nil, invalidProfile("invalid type: " + testJSONText(version) + ", expected a string")
			}
			if _, ok := nativeVersionParse(text); !ok {
				return nil, invalidProfile("canonical SemVer version required")
			}
			var decodedScreen provenanceScreen
			if evidence, present := screen.get("composer_evidence"); present {
				records, ok := evidence.(*jsonArr)
				if !ok {
					return nil, invalidProfile("invalid type: " + testJSONText(evidence) + ", expected a sequence")
				}
				decodedScreen.hasEvidence = true
				for _, record := range records.items {
					parsed, err := serdeComposerEvidence(record)
					if err != nil {
						return nil, err
					}
					decodedScreen.composerEvidence = append(decodedScreen.composerEvidence, parsed)
				}
			}
			file.screens[id] = decodedScreen
		}
		decoded = append(decoded, file)
	}
	return decoded, nil
}

func (p *provenanceSubmission) validateEvidence(operation composerOperation, screens map[string]provenanceScreen) error {
	coverage := map[evidenceIdentity]bool{}
	for _, id := range p.fixtureIDs {
		screen, ok := screens[id]
		if !ok {
			panic("validateProvenance cites every plan fixture before validating its evidence")
		}
		for _, record := range screen.composerEvidence {
			if record.identity.operation != operation {
				continue
			}
			if err := require(record.matchesPlan(p), fmt.Sprintf("composer evidence %s for %s/%s/%s must match its selected plan",
				id, composerOperationDebug(operation), record.identity.multiplexer.debug(), record.identity.kind.debug())); err != nil {
				return err
			}
			coverage[evidenceIdentity{operation: operation, multiplexer: record.identity.multiplexer, kind: record.identity.kind}] = true
		}
	}
	for _, multiplexer := range []evidenceMultiplexer{evidenceMultiplexerTmux, evidenceMultiplexerHerdr} {
		for _, kind := range []composerEvidenceKind{composerEvidenceKindFraming, composerEvidenceKindPlainLimit, composerEvidenceKindPostSubmit, composerEvidenceKindUnsubmittedLiteral} {
			if err := require(coverage[evidenceIdentity{operation: operation, multiplexer: multiplexer, kind: kind}],
				fmt.Sprintf("composer %s/%s/%s evidence required", composerOperationDebug(operation), multiplexer.debug(), kind.debug())); err != nil {
				return err
			}
		}
	}
	return nil
}

// inlinePreviewScalars is the 4000-scalar inline preview limit, which a null Director
// plain_limit names.
const inlinePreviewScalars = 4000

func (d *provenanceDirectorDelivery) limitLength(literal string) uint64 {
	if limit, set := d.plainLimit.get(); set && limit.Unit == literalUnitBytes {
		return uint64(len(literal))
	}
	return uint64(len([]rune(literal)))
}

func (d *provenanceDirectorDelivery) limitMax() uint64 {
	if limit, set := d.plainLimit.get(); set {
		return uint64(limit.Max)
	}
	return inlinePreviewScalars
}

// validateLimitFixtures: each listed multiplexer has a cited prompt pre-submit fixture whose
// literal is exactly plain_limit long.
func (d *provenanceDirectorDelivery) validateLimitFixtures(backend string, screens map[string]provenanceScreen) error {
	for _, multiplexer := range d.multiplexers {
		measured := false
		for _, id := range d.fixtureIDs {
			for _, record := range screens[id].composerEvidence {
				literal, isLiteral := record.expectedLiteral.get()
				if record.identity.operation == composerOperationPrompt && record.identity.multiplexer == multiplexer &&
					isLiteral && d.limitLength(literal) == d.limitMax() {
					measured = true
				}
			}
		}
		if err := require(measured, "director delivery for "+backend+" requires a measured plain_limit"); err != nil {
			return err
		}
	}
	return nil
}

func validateProvenance(files any) error {
	decoded, err := serdeProvenanceFiles(files)
	if err != nil {
		return err
	}
	ids := map[string]bool{}
	for _, file := range decoded {
		for _, id := range file.screenIDs {
			if err := require(!ids[id], "duplicate fixture id "+id); err != nil {
				return err
			}
			ids[id] = true
			screen := file.screens[id]
			if !screen.hasEvidence {
				continue
			}
			if err := require(len(screen.composerEvidence) > 0, "present composer evidence must be nonempty"); err != nil {
				return err
			}
			claims := map[evidenceIdentity]bool{}
			for _, record := range screen.composerEvidence {
				if err := require(!claims[record.identity], "duplicate operation/multiplexer/evidence kind"); err != nil {
					return err
				}
				claims[record.identity] = true
			}
		}
	}
	for _, file := range decoded {
		cite := func(cited []string) error {
			if err := require(len(cited) > 0, "capability provenance required"); err != nil {
				return err
			}
			for _, id := range cited {
				if _, ok := file.screens[id]; !ok {
					return invalidProfile(file.backend + " cites unknown fixture " + id)
				}
			}
			return nil
		}
		for _, r := range file.rules {
			if r.cited {
				if err := cite(r.fixtureIDs); err != nil {
					return err
				}
			}
		}
		if file.composer != nil && file.composer.submission {
			empty := slices.ContainsFunc(file.rules, func(r provenanceRule) bool {
				return r.observation == observationKindComposerEmpty && slices.Contains(file.composer.ruleIDs, r.id)
			})
			if err := require(empty, "executable composer plans require empty-composer observation"); err != nil {
				return err
			}
		}
		for _, capability := range []*provenanceCapability{file.detection, file.composer} {
			if capability == nil {
				continue
			}
			if err := cite(capability.fixtureIDs); err != nil {
				return err
			}
			if !capability.submission {
				continue
			}
			for _, plan := range []struct {
				operation composerOperation
				plan      *provenanceSubmission
			}{{composerOperationPrompt, capability.prompt}, {composerOperationExec, capability.exec}} {
				if plan.plan == nil {
					continue
				}
				if err := cite(plan.plan.fixtureIDs); err != nil {
					return err
				}
				if err := plan.plan.validateEvidence(plan.operation, file.screens); err != nil {
					return err
				}
			}
		}
		for _, cited := range file.answers {
			if err := cite(cited); err != nil {
				return err
			}
		}
		if file.directorDelivery != nil {
			if err := cite(file.directorDelivery.fixtureIDs); err != nil {
				return err
			}
			if err := file.directorDelivery.validateLimitFixtures(file.backend, file.screens); err != nil {
				return err
			}
		}
	}
	return nil
}

func mustCanonicalJSON(value any) string {
	text, err := canonicalJSON(jsonBytes(value))
	if err != nil {
		panic(fmt.Sprintf("canonical JSON of a test value: %v", err))
	}
	return text
}

// validatePreSubmitFixtures: every pre-submit screen that paints its rows (a lines field)
// extracts, through its backend's composer.text, a composer region overlapping no masked row
// whose text matches the screen's expected_literal.
func validatePreSubmitFixtures(files any, catalog *profileCatalog) error {
	object, ok := files.(*jsonObj)
	if !ok {
		return invalidProfile("object required")
	}
	for _, member := range object.members {
		backend, ok := codingAgentCodingAgent(member.key)
		if !ok {
			return invalidProfile("unknown backend " + member.key)
		}
		detector := newDetector(catalog, backend, none[string]())
		screens, ok := jget(member.value, "screens").(*jsonObj)
		if !ok {
			return invalidProfile("object required")
		}
		for _, entry := range screens.members {
			id, screen := entry.key, entry.value
			literals, err := preSubmitLiterals(screen)
			if err != nil {
				return err
			}
			for _, literal := range literals {
				if _, painted := jobject(screen).get("lines"); !painted {
					continue
				}
				snapshot := replayNativeFixture(mustCanonicalJSON(screen), nil, fixtureAlternate(screen), none[[2]uint16]())
				composer, found, err := detector.composerText(&snapshot)
				if err != nil {
					return err
				}
				if !found {
					return invalidProfile("pre-submit fixture " + id + " has no extractable composer text")
				}
				masks, err := guardArray(jget(screen, "masked_rows"))
				if err != nil {
					return err
				}
				for _, mask := range masks {
					row, ok := ju64(jget(mask, "row"))
					if !ok {
						return invalidProfile("masked row index required")
					}
					inside := uint64(composer.region.top) <= row && row < uint64(composer.region.bottom)
					if err := require(!inside, "pre-submit fixture "+id+" masks a composer row"); err != nil {
						return err
					}
				}
				if err := require(composer.matches([]string{literal}), "pre-submit fixture "+id+" does not render its expected_literal"); err != nil {
					return err
				}
			}
		}
	}
	return nil
}

// preSubmitRecord is one pre-submit record of a screen: its operation, multiplexer and written
// literal.
type preSubmitRecord struct {
	operation   composerOperation
	multiplexer muxBackend
	literal     string
}

func preSubmitRecords(screen any) ([]preSubmitRecord, error) {
	records, present := jobject(screen).get("composer_evidence")
	if !present {
		return nil, nil
	}
	array, ok := records.(*jsonArr)
	if !ok {
		return nil, invalidProfile("invalid type: " + testJSONText(records) + ", expected a sequence")
	}
	var out []preSubmitRecord
	for _, raw := range array.items {
		record, err := serdeComposerEvidence(raw)
		if err != nil {
			return nil, err
		}
		if literal, isLiteral := record.expectedLiteral.get(); isLiteral {
			out = append(out, preSubmitRecord{operation: record.identity.operation, multiplexer: record.identity.multiplexer.backend(), literal: literal})
		}
	}
	return out, nil
}

// preSubmitLiterals is the distinct expected_literal texts of a screen's pre-submit records, in
// byte order.
func preSubmitLiterals(screen any) ([]string, error) {
	records, err := preSubmitRecords(screen)
	if err != nil {
		return nil, err
	}
	var literals []string
	for _, record := range records {
		literals = append(literals, record.literal)
	}
	slices.Sort(literals)
	return slices.Compact(literals), nil
}

func composerEvidenceRecords(operation, multiplexer string, framing, limit any) [4]any {
	return [4]any{
		jo("operation", operation, "multiplexer", multiplexer, "kind", "framing", "literal_framing", jclone(framing)),
		jo("operation", operation, "multiplexer", multiplexer, "kind", "plain_limit", "plain_literal_limit", jclone(limit)),
		jo("operation", operation, "multiplexer", multiplexer, "kind", "post_submit"),
		jo("operation", operation, "multiplexer", multiplexer, "kind", "unsubmitted_literal", "expected_literal", syntheticLiteral),
	}
}

func withSyntheticComposerEvidence(files any) (any, error) {
	object, ok := files.(*jsonObj)
	if !ok {
		return nil, invalidProfile("object required")
	}
	decorated := jclone(files)
	for _, member := range object.members {
		backend, file := member.key, member.value
		claims := map[string][]any{}
		submission := jget(file, "profile", "capabilities", "composer", "submission")
		for _, operation := range []string{"prompt", "exec"} {
			plan := jget(submission, operation)
			if plan == nil {
				continue
			}
			cited, err := serdeIDs(jget(plan, "fixture_ids"))
			if err != nil {
				return nil, err
			}
			for _, id := range cited {
				screen, found := jobject(jget(file, "screens")).get(id)
				if !found {
					continue
				}
				if _, has := jobject(screen).get("composer_evidence"); has {
					continue
				}
				framing, ok := jobject(plan).get("literal_framing")
				if !ok {
					panic("parsed literal framing")
				}
				limit, ok := jobject(plan).get("plain_literal_limit")
				if !ok {
					panic("parsed plain limit")
				}
				for _, multiplexer := range []string{"tmux", "herdr"} {
					for _, record := range composerEvidenceRecords(operation, multiplexer, framing, limit) {
						existing := slices.IndexFunc(claims[id], func(claim any) bool {
							return jequal(jget(claim, "operation"), jget(record, "operation")) &&
								jequal(jget(claim, "multiplexer"), jget(record, "multiplexer")) &&
								jequal(jget(claim, "kind"), jget(record, "kind"))
						})
						if existing >= 0 {
							if err := require(jequal(claims[id][existing], record), "synthetic evidence conflicts"); err != nil {
								return nil, err
							}
							continue
						}
						claims[id] = append(claims[id], record)
					}
				}
			}
		}
		ids := make([]string, 0, len(claims))
		for id := range claims {
			ids = append(ids, id)
		}
		slices.Sort(ids)
		for _, id := range ids {
			jset(decorated, ja(claims[id]...), backend, "screens", id, "composer_evidence")
		}
	}
	return decorated, nil
}

const syntheticLiteral = "synthetic literal"

// profileCatalogFromFiles is the Rust test-only ProfileCatalog::from_files: it decodes a JSON
// object mapping each backend name to its profile file.
func profileCatalogFromFiles(files any) (*profileCatalog, error) {
	object, ok := files.(*jsonObj)
	if !ok {
		return nil, invalidProfile("object required")
	}
	catalog := &profileCatalog{captureOrigin: "synthetic"}
	for _, member := range object.members {
		backend, ok := codingAgentCodingAgent(member.key)
		if !ok {
			return nil, invalidProfile("unknown backend")
		}
		decoded, err := decodeProfileFileText(backend, jsonBytes(member.value))
		if err != nil {
			return nil, err
		}
		catalog.profiles = append(catalog.profiles, decoded)
	}
	return catalog, nil
}

// withoutSubmission removes operation's submission plan for backend, for missing-plan fallback
// tests.
func (c *profileCatalog) withoutSubmission(backend codingAgent, operation composerOperation) *profileCatalog {
	for index := range c.profiles {
		if c.profiles[index].backend == backend {
			c.profiles[index].submissions[operation.index()] = nil
		}
	}
	return c
}

func (c *profileCatalog) answerPlan(backend codingAgent, request string, key answerKey) (answerPlan, bool) {
	p, ok := c.profile(backend)
	if !ok {
		return answerPlan{}, false
	}
	return p.answerPlan(request, key)
}

func (s *answerStage) requiredScreenRuleID() (string, bool) {
	if screen, ok := s.gate.(stageGateScreen); ok {
		return screen.rule, true
	}
	return "", false
}

// loadTestCatalog is the Rust test_support::load: production decoding followed by the
// provenance build guard.
func loadTestCatalog(files any) (*profileCatalog, error) {
	catalog, err := profileCatalogFromFiles(files)
	if err != nil {
		return nil, err
	}
	decorated, err := withSyntheticComposerEvidence(files)
	if err != nil {
		return nil, err
	}
	if err := validateProvenance(decorated); err != nil {
		return nil, err
	}
	if err := validatePreSubmitFixtures(files, catalog); err != nil {
		return nil, err
	}
	return catalog, nil
}

func mustLoadCatalog(t testing.TB, files any) *profileCatalog {
	t.Helper()
	catalog, err := loadTestCatalog(files)
	if err != nil {
		t.Fatalf("the test catalog loads: %v", err)
	}
	return catalog
}

const syntheticTextRuleID = "synthetic-composer-text"

// syntheticInlineScalars is the scalar count above which the synthetic composer collapses its
// typed text into a `[Typed N chars]` placeholder, as native composers collapse long pastes.
const syntheticInlineScalars = 30

// addSyntheticComposerText adds the synthetic typed composer to backend's composer capability:
// a `Synthetic input: ` row above `State end`, read back through composer.text.
func addSyntheticComposerText(value any, backend string) {
	jarray(jget(value, backend, "profile", "rules")).push(syntheticTextRule())
	composer := jget(value, backend, "profile", "capabilities", "composer")
	jarray(jget(composer, "rule_ids")).push(syntheticTextRuleID)
	jset(composer, syntheticTextDeclaration(), "text")
}

func syntheticTextRule() any {
	return jsonf(`{
		"id":%s,"priority":40,"observation":"composer_text",
		"region":{"screen":"either","start_pattern":"^Synthetic input: ","end_pattern":"^State end$","max_rows":8,"cursor":"any"},
		"matchers":[{"id":"input","scope":"line","pattern":"^Synthetic input: "}],
		"extract":{"operation":null,"choices":[]},
		"verified_noninteraction":true
	}`, syntheticTextRuleID)
}

func syntheticTextDeclaration() any {
	return jsonf(`{
		"rule_ids":[%s],
		"first_prefix":"^Synthetic input: ",
		"continuation_prefix":"^  ",
		"wrap":"char",
		"placeholders":[{"pattern":"\\[Typed (?P<count>[0-9]+) chars\\]","unit":"scalars"}]
	}`, syntheticTextRuleID)
}

// syntheticComposerEcho is the child output that paints typed into the synthetic composer of
// addSyntheticComposerText over the empty synthetic composer's two bottom rows.
func syntheticComposerEcho(typed string) []byte {
	return []byte("\x1b[23;1H\x1b[2K" + syntheticComposerLine(typed) + "\r\n\x1b[2KState end")
}

// composerPainter is child output that paints a typed literal into a composer, as the native
// CLI echoes its unsubmitted input.
type composerPainter func(typed string) []byte

func syntheticComposerPainter() composerPainter { return syntheticComposerEcho }

// nativeComposerPainter paints into the composer row of the bundled backend screen: the row
// holding its cursor, rewritten as its composer.text prompt marker followed by the typed text. A
// literal beginning with the profile's mode_prefix switches the marker to that prefix where
// first_prefix accepts it, as the native shell mode does, and the rest follows.
func nativeComposerPainter(t testing.TB, backend codingAgent, size terminalSize, screen []byte) (composerPainter, bool) {
	t.Helper()
	text := jget(parsedBundledFiles(), backend.name(), "profile", "capabilities", "composer", "text")
	if text == nil {
		return nil, false
	}
	firstPattern, ok := jstring(jget(text, "first_prefix"))
	if !ok {
		t.Fatal("a composer first_prefix")
	}
	firstPrefix := regexp.MustCompile(firstPattern)
	modePrefix, hasMode := jstring(jget(text, "mode_prefix"))
	continuationPattern, ok := jstring(jget(text, "continuation_prefix"))
	if !ok {
		t.Fatal("a composer continuation_prefix")
	}
	continuation := strings.TrimLeft(continuationPattern, "^")
	cols := int(size.cols)
	terminal := mustTerminalState(t, size)
	mustFeed(t, terminal, screen)
	snapshot := terminal.snapshot()
	cursor, ok := snapshot.cursor.get()
	if !ok {
		t.Fatal("a replayed runner screen has a cursor")
	}
	row := int(cursor[0])
	lines := snapshot.screen.Rows(0, snapshot.cols)
	if row >= len(lines) {
		t.Fatal("the cursor row is on the screen")
	}
	marker := firstPrefix.FindString(lines[row])
	return func(typed string) []byte {
		shownMarker, rest := marker, typed
		switched := false
		if hasMode && marker != "" {
			_, size := utf8.DecodeRuneInString(marker)
			candidate := modePrefix + marker[size:]
			if after, found := strings.CutPrefix(typed, modePrefix); found {
				if loc := firstPrefix.FindStringIndex(candidate); loc != nil && loc[0] == 0 && loc[1] == len(candidate) {
					shownMarker, rest, switched = candidate, after, true
				}
			}
		}
		if !switched && hasMode {
			if after, found := strings.CutPrefix(typed, modePrefix); found {
				rest = after
			}
		}
		firstWidth := max(cols-utf8.RuneCountInString(shownMarker), 1)
		continuedWidth := max(cols-utf8.RuneCountInString(continuation), 1)
		chars := []rune(rest)
		split := min(firstWidth, len(chars))
		composer := []string{shownMarker + string(chars[:split])}
		remaining := chars[split:]
		for len(remaining) > 0 {
			take := min(continuedWidth, len(remaining))
			composer = append(composer, continuation+string(remaining[:take]))
			remaining = remaining[take:]
		}
		if len(composer) == 1 {
			return fmt.Appendf(nil, "\x1b[%d;1H\x1b[2K%s", row+1, composer[0])
		}
		painted := append(append(slices.Clone(lines[:row]), composer...), lines[row+1:]...)
		scrolled := max(len(painted)-len(lines), 0)
		shown := painted[scrolled:]
		out := []byte("\x1b[0m\x1b[2J")
		for index, line := range shown {
			out = fmt.Appendf(out, "\x1b[%d;1H%s", index+1, line)
		}
		last := row + len(composer) - scrolled
		return fmt.Appendf(out, "\x1b[%d;%dH", last, utf8.RuneCountInString(composer[len(composer)-1])+1)
	}, true
}

// syntheticComposerLine is the synthetic composer row holding typed, collapsed into a
// placeholder when long.
func syntheticComposerLine(typed string) string {
	shown := typed
	if count := utf8.RuneCountInString(typed); count > syntheticInlineScalars {
		shown = fmt.Sprintf("[Typed %d chars]", count)
	}
	return "Synthetic input: " + shown
}

// profileCatalogFromBundledFiles decodes the files and runs the whole bundled-catalog guard.
func profileCatalogFromBundledFiles(files any) (*profileCatalog, error) {
	catalog, err := profileCatalogFromFiles(files)
	if err != nil {
		return nil, err
	}
	for _, member := range jobject(files).members {
		screens, ok := jget(member.value, "screens").(*jsonObj)
		if !ok {
			return nil, invalidProfile("object required")
		}
		for _, screen := range screens.members {
			if err := verifyBundledScreen(screen.value); err != nil {
				return nil, err
			}
		}
	}
	if err := validateProvenance(files); err != nil {
		return nil, err
	}
	if err := validatePreSubmitFixtures(files, catalog); err != nil {
		return nil, err
	}
	catalog.captureOrigin = "bundled_unknown"
	return catalog, nil
}

// fixtureAt is a synthetic stub screen captured at version.
func fixtureAt(version string) any {
	return jo("native_version", version)
}

type singleProfile struct {
	backend codingAgent
	label   string
}

// singleProfiles is a catalog holding one profile file per entry, each citing one stub screen
// for detection, composer observation, both composer operations and an allow answer; the label
// names that screen and answer rule.
func singleProfiles(profiles []singleProfile) any {
	base := jget(syntheticDefinitions(), "codex", "profile")
	composer := jget(simpleDefinition("composer_empty", true), "codex", "profile", "rules", 0)
	value := jo()
	for _, entry := range profiles {
		id := syntheticFixture + "-" + entry.backend.name() + "-" + entry.label
		profile := jclone(base)
		rules := jarray(jget(profile, "rules"))
		rules.push(jclone(composer))
		rules.push(syntheticTextRule())
		jset(profile, ja(id), "capabilities", "detection", "fixture_ids")
		jset(profile, jsonf(`{
			"rule_ids":["synthetic-composer_empty", %s],"fixture_ids":[%s],
			"text":%s,
			"submission":{
				"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":[%s]},
				"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":[%s]}
			}
		}`, syntheticTextRuleID, id, syntheticTextDeclaration(), id, id), "capabilities", "composer")
		jset(profile, jsonf(`[{
			"decision":"allow","request_rule_id":%s,"answer_rule_id":%s,
			"choice_id":"allow-once","focus_matcher_id":"allow","fixture_ids":[%s],
			"stages":[{"bytes_hex":"78","required_rule_id":%s,"required_choice_id":"allow-once"}]
		}]`, syntheticRule, entry.label+"-allow-plan", id, syntheticRule), "capabilities", "answers")
		if _, exists := value.get(entry.backend.name()); exists {
			panic("one profile file per backend: " + entry.backend.name())
		}
		value.set(entry.backend.name(), jo(
			"schema_version", 1, "backend", entry.backend.name(), "profile", profile,
			"screens", jo(id, fixtureAt(syntheticVersion)),
		))
	}
	return value
}

// bundledFile is the backend and parsed bundled profile file whose screens holds id.
func bundledFile(id string) (codingAgent, any) {
	backend, file := sharedBundledFile(id)
	return backend, jclone(file)
}

func sharedBundledFile(id string) (codingAgent, any) {
	for _, member := range parsedBundledFiles().members {
		if _, found := jobject(jget(member.value, "screens")).get(id); found {
			backend, ok := codingAgentCodingAgent(member.key)
			if !ok {
				panic("a backend name")
			}
			return backend, member.value
		}
	}
	panic("the bundled catalog contains " + id)
}

var bundledFixtureTexts sync.Map

func bundledFixture(id string) string {
	if text, ok := bundledFixtureTexts.Load(id); ok {
		return text.(string)
	}
	_, file := sharedBundledFile(id)
	text := mustCanonicalJSON(jget(file, "screens", id))
	bundledFixtureTexts.Store(id, text)
	return text
}

func syntheticDefinitions() any {
	return parseTestJSON(`{"codex":{
		"schema_version":1,"backend":"codex",
		"profile":{
			"rules":[{"id":"synthetic-permission","priority":100,"observation":"permission_request",
				"region":{"screen":"either","start_pattern":"^Synthetic permission request$","end_pattern":"^Selection: $","max_rows":8,"cursor":"at_end"},
				"matchers":[
					{"id":"operation","scope":"line","pattern":"^Operation: (?P<operation>.+)$"},
					{"id":"allow","scope":"line","pattern":"^> (?P<label>Allow once)$"},
					{"id":"deny","scope":"line","pattern":"^  (?P<label>Deny once)$"},
					{"id":"form","scope":"line","pattern":"^Request form: synthetic-only$"}
				],
				"extract":{"operation":{"matcher_id":"operation","capture":"operation"},"choices":[
					{"id":"allow-once","label":{"matcher_id":"allow","capture":"label"},"scope":"single_request"},
					{"id":"deny-once","label":{"matcher_id":"deny","capture":"label"},"scope":"single_request"}
				]},"verified_noninteraction":false}],
			"capabilities":{"detection":{"rule_ids":["synthetic-permission"],"fixture_ids":["synthetic-permission-1"]},"composer":null,"answers":[]}},
		"screens":{"synthetic-permission-1":{"native_version":"0.0.1-synthetic"}}
	}}`)
}

func syntheticCatalog(t testing.TB) *profileCatalog {
	t.Helper()
	return mustLoadCatalog(t, syntheticDefinitions())
}

func inspectionFixtureDefinition() any {
	value := syntheticDefinitions()
	jset(value, "^Request form: (?P<focus>synthetic-only|synthetic-other)$", "codex", "profile", "rules", 0, "matchers", 3, "pattern")
	alternate := jclone(jget(value, "codex", "profile", "rules", 0))
	jset(alternate, "inspection-alternate-request", "id")
	jset(alternate, "^Request form: (?P<focus>synthetic-alt)$", "matchers", 3, "pattern")
	jarray(jget(value, "codex", "profile", "rules")).push(alternate)
	jset(value, ja("synthetic-permission", "inspection-alternate-request"), "codex", "profile", "capabilities", "detection", "rule_ids")
	jset(value, ja(
		syntheticAnswer("allow", "synthetic-permission", "inspection-allow-plan", "allow-once", "79"),
		syntheticAnswer("deny", "synthetic-permission", "inspection-deny-plan", "deny-once", "6e"),
		syntheticAnswer("allow", "inspection-alternate-request", "inspection-alternate-allow", "allow-once", "61"),
	), "codex", "profile", "capabilities", "answers")
	return value
}

// syntheticAnswer is the Rust test_support::answer.
func syntheticAnswer(decision, request, id, choice, byteHex string) any {
	return jsonf(`{
		"decision":%s,"request_rule_id":%s,"answer_rule_id":%s,
		"choice_id":%s,"focus_matcher_id":"form","fixture_ids":["synthetic-permission-1"],
		"stages":[
			{"bytes_hex":%s,"required_rule_id":%s,"required_choice_id":%s},
			{"bytes_hex":"0d","required_rule_id":%s,"required_choice_id":%s}
		]
	}`, decision, request, id, choice, byteHex, request, choice, request, choice)
}

func inspectionFixtureForm(operation, focus string) []byte {
	return []byte("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: " + operation + "\r\n> Allow once\r\n  Deny once\r\nRequest form: " + focus + "\r\nSelection: ")
}

const (
	gatedRule          = "synthetic-gated"
	gatedReasonRule    = "synthetic-reason"
	gatedReasonFixture = "synthetic-reason-native-1"
	focusSGR           = "\x1b[48;2;250;178;131m"
)

func gatedFixtureDefinition() any {
	value := syntheticDefinitions()
	jset(value, fixtureAt(syntheticVersion), "codex", "screens", gatedReasonFixture)
	jset(value, jsonf(`[
		{
			"id":%s,"priority":100,"observation":"permission_request",
			"region":{"screen":"either","start_pattern":"^Synthetic gated request$","end_pattern":"^Selection: $","max_rows":8,"cursor":"any"},
			"matchers":[
				{"id":"heading","scope":"line","pattern":"^Synthetic gated request$"},
				{"id":"operation","scope":"line","pattern":"^Operation: (?P<operation>.+)$"},
				{"id":"focus","scope":"line","pattern":"^  Allow once   (?:Always allow   )?Deny once$","focus_style":"background"},
				{"id":"allow","scope":"line","pattern":"^  (?P<label>Allow once)   "},
				{"id":"deny","scope":"line","pattern":"   (?P<label>Deny once)$"}
			],
			"extract":{"operation":{"matcher_id":"operation","capture":"operation"},"choices":[
				{"id":"allow-once","label":{"matcher_id":"allow","capture":"label"},"scope":"single_request"},
				{"id":"deny-once","label":{"matcher_id":"deny","capture":"label"},"scope":"single_request"}
			]},
			"verified_noninteraction":false
		},
		{
			"id":%s,"priority":100,"observation":"permission_continuation",
			"region":{"screen":"either","start_pattern":"^Synthetic reject reason$","end_pattern":"^Confirm: enter$","max_rows":4,"cursor":"any"},
			"matchers":[
				{"id":"heading","scope":"line","pattern":"^Synthetic reject reason$"},
				{"id":"empty","scope":"line","pattern":"^Reason: \\(empty\\)$"}
			],
			"extract":{"operation":null,"choices":[]},
			"verified_noninteraction":false,"continues_rule_id":%s,"continues_choice_id":"deny-once",
			"fixture_ids":[%s]
		}
	]`, gatedRule, gatedReasonRule, gatedRule, gatedReasonFixture), "codex", "profile", "rules")
	jset(value, ja(gatedRule), "codex", "profile", "capabilities", "detection", "rule_ids")
	jset(value, jsonf(`[
		{
			"decision":"allow","request_rule_id":%s,"answer_rule_id":"synthetic-gated-allow",
			"choice_id":"allow-once","focus_matcher_id":"focus","fixture_ids":["synthetic-permission-1"],
			"stages":[
				{"bytes_hex":"0d","required_rule_id":%s,"required_choice_id":"allow-once","required_focus_choice_id":"allow-once"}
			]
		},
		{
			"decision":"deny","request_rule_id":%s,"answer_rule_id":"synthetic-gated-deny",
			"choice_id":"deny-once","focus_matcher_id":"focus","fixture_ids":["synthetic-permission-1"],
			"stages":[
				{"bytes_hex":"68","required_rule_id":%s,"required_choice_id":"deny-once","required_focus_choice_id":"allow-once"},
				{"bytes_hex":"0d","required_rule_id":%s,"required_choice_id":"deny-once","required_focus_choice_id":"deny-once"},
				{"bytes_hex":"0d","required_rule_id":%s,"required_choice_id":"deny-once","required_screen_rule_id":%s}
			]
		}
	]`, gatedRule, gatedRule, gatedRule, gatedRule, gatedRule, gatedRule, gatedReasonRule), "codex", "profile", "capabilities", "answers")
	return value
}

// gatedFixtureFormStyled: focused names the labels painted with focusSGR; the rest of the choice
// row keeps rowSGR, which is the row background the detector compares against.
func gatedFixtureFormStyled(operation string, always bool, focused []string, rowSGR, focusSGR string) []byte {
	labels := []string{"Allow once", "Deny once"}
	if always {
		labels = []string{"Allow once", "Always allow", "Deny once"}
	}
	// The erase paints the whole row in its background, as the native form does.
	row := rowSGR + "\x1b[K  "
	for index, label := range labels {
		if index > 0 {
			row += "   "
		}
		if slices.Contains(focused, label) {
			row += focusSGR + label + "\x1b[0m" + rowSGR
		} else {
			row += label
		}
	}
	row += "\x1b[0m"
	return []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic gated request\r\nOperation: " + operation + "\r\n" + row + "\r\nSelection: ")
}

func gatedFixtureForm(operation, focused string) []byte {
	labels := map[string][]string{
		"allow":  {"Allow once"},
		"deny":   {"Deny once"},
		"always": {"Always allow"},
		"none":   {},
	}
	focus, ok := labels[focused]
	if !ok {
		panic("known synthetic focus")
	}
	return gatedFixtureFormStyled(operation, true, focus, "", focusSGR)
}

func gatedReasonDialog(reason opt[string]) []byte {
	text, set := reason.get()
	if !set {
		text = "(empty)"
	}
	return []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic reject reason\r\nReason: " + text + "\r\nConfirm: enter")
}

const (
	questionRule        = "synthetic-question"
	questionNoticeRule  = "synthetic-question-notice"
	questionDeclinePlan = "synthetic-question-decline"
	questionLaterPlan   = "synthetic-question-later"
)

func questionStage(byteHex, choice string, focus opt[string]) any {
	stage := jo("bytes_hex", byteHex, "required_rule_id", questionRule, "required_choice_id", choice)
	if focusChoice, set := focus.get(); set {
		stage.set("required_focus_choice_id", focusChoice)
	}
	return stage
}

func choiceAnswer(request, id, choice string, stages any) any {
	return jo("request_rule_id", request, "answer_rule_id", id, "choice_id", choice,
		"focus_matcher_id", "focus", "fixture_ids", ja(syntheticFixture), "stages", stages)
}

// questionFixtureDefinition is the permission profile plus a marker-focused question with a
// gated decline plan, an ungated session-scoped later plan and a plan-less accept choice, and a
// choice-less question notice.
func questionFixtureDefinition() any {
	value := syntheticDefinitions()
	rules := jarray(jget(value, "codex", "profile", "rules"))
	rules.push(jsonf(`{
		"id":%s,"priority":100,"observation":"question",
		"region":{"screen":"either","start_pattern":"^Synthetic question\\?$","end_pattern":"^Question end$","max_rows":8,"cursor":"any"},
		"matchers":[
			{"id":"heading","scope":"line","pattern":"^Synthetic question\\?$"},
			{"id":"focus","scope":"line","pattern":"^> \\S","focus_style":"marker"},
			{"id":"accept","scope":"line","pattern":"^(?:> |  )(?P<label>Accept the change)$"},
			{"id":"decline","scope":"line","pattern":"^(?:> |  )(?P<label>Keep \\S.*)$"},
			{"id":"later","scope":"line","pattern":"^(?:> |  )(?P<label>Ask later)$"}
		],
		"extract":{"operation":null,"choices":[
			{"id":"accept","label":{"matcher_id":"accept","capture":"label"},"scope":"persistent"},
			{"id":"decline","label":{"matcher_id":"decline","capture":"label"},"scope":"persistent"},
			{"id":"later","label":{"matcher_id":"later","capture":"label"},"scope":"persistent"}
		]},
		"verified_noninteraction":false
	}`, questionRule))
	rules.push(jsonf(`{
		"id":%s,"priority":100,"observation":"question",
		"region":{"screen":"either","start_pattern":"^Synthetic notice$","end_pattern":"^Synthetic notice$","max_rows":1,"cursor":"any"},
		"matchers":[{"id":"notice","scope":"line","pattern":"^Synthetic notice$"}],
		"extract":{"operation":null,"choices":[]},
		"verified_noninteraction":false
	}`, questionNoticeRule))
	jset(value, ja(syntheticRule, questionRule, questionNoticeRule), "codex", "profile", "capabilities", "detection", "rule_ids")
	jset(value, ja(
		syntheticAnswer("allow", syntheticRule, "synthetic-question-profile-allow", "allow-once", "79"),
		choiceAnswer(questionRule, questionDeclinePlan, "decline", ja(
			questionStage("0e", "decline", some("accept")),
			questionStage("0d", "decline", some("decline")),
		)),
		choiceAnswer(questionRule, questionLaterPlan, "later", ja(questionStage("33", "later", none[string]()))),
	), "codex", "profile", "capabilities", "answers")
	return value
}

// questionFixtureForm: focused names the choices drawn with the `> ` marker: accept, decline,
// later, several of them, or none.
func questionFixtureForm(focused []string) []byte {
	row := func(choice, label string) string {
		if slices.Contains(focused, choice) {
			return "> " + label
		}
		return "  " + label
	}
	return []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic question?\r\n" + row("accept", "Accept the change") + "\r\n" +
		row("decline", "Keep current") + "\r\n" + row("later", "Ask later") + "\r\nQuestion end")
}

func questionNoticeForm() []byte {
	return []byte("\x1b[0m\x1b[2J\x1b[19;1HSynthetic notice")
}

const (
	syntheticVersion = "0.0.1-synthetic"
	syntheticRule    = "synthetic-permission"
	syntheticFixture = "synthetic-permission-1"
)

func simpleDefinition(kind string, verified bool) any {
	value := syntheticDefinitions()
	rule := jget(value, "codex", "profile", "rules", 0)
	jset(rule, "synthetic-"+kind, "id")
	jset(rule, kind, "observation")
	jset(rule, "^Synthetic state$", "region", "start_pattern")
	jset(rule, "^State end$", "region", "end_pattern")
	jset(rule, parseTestJSON(`[{"id":"state","scope":"line","pattern":"^Synthetic state$"}]`), "matchers")
	jset(rule, parseTestJSON(`{"operation":null,"choices":[]}`), "extract")
	jset(rule, verified, "verified_noninteraction")
	declaration := jo("rule_ids", ja("synthetic-"+kind), "fixture_ids", ja(syntheticFixture))
	composerKind := strings.HasPrefix(kind, "composer_")
	if composerKind {
		declaration.set("submission", parseTestJSON(`{
			"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-permission-1"]},
			"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-permission-1"]}
		}`))
		jset(value, jo("detection", nil, "composer", declaration, "answers", ja()), "codex", "profile", "capabilities")
		addSyntheticComposerText(value, "codex")
	} else {
		jset(value, jo("detection", declaration, "composer", nil, "answers", ja()), "codex", "profile", "capabilities")
	}
	return value
}

const (
	emptyComposerRule = "synthetic-composer_empty"
	promptFixture     = "synthetic-prompt-plan"
	execFixture       = "synthetic-exec-plan"
)

// composerDefinition is the Rust test_support::definition: a composer catalog whose submission
// declares both operations; an absent prompt or exec takes a placeholder submit key, and
// composerCatalog removes its plan.
func composerDefinition(prompt, exec opt[string]) any {
	value := simpleDefinition("composer_empty", true)
	for _, id := range []string{promptFixture, execFixture} {
		jset(value, fixtureAt(syntheticVersion), "codex", "screens", id)
	}
	promptSubmit, set := prompt.get()
	if !set {
		promptSubmit = "0d"
	}
	execSubmit, set := exec.get()
	if !set {
		execSubmit = "0d"
	}
	jset(value, jsonf(`{
		"rule_ids":[%s, %s],"fixture_ids":[%s],
		"text":%s,
		"submission":{
			"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":%s,"fixture_ids":[%s]},
			"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":%s,"fixture_ids":[%s]}
		}
	}`, emptyComposerRule, syntheticTextRuleID, syntheticFixture, syntheticTextDeclaration(), promptSubmit, promptFixture, execSubmit, execFixture),
		"codex", "profile", "capabilities", "composer")
	return value
}

// composerCatalog is the composerDefinition catalog with the plan of each absent operation
// removed.
func composerCatalog(t testing.TB, prompt, exec opt[string]) *profileCatalog {
	t.Helper()
	catalog := mustLoadCatalog(t, composerDefinition(prompt, exec))
	if _, set := prompt.get(); !set {
		catalog = catalog.withoutSubmission(codingAgentCodex, composerOperationPrompt)
	}
	if _, set := exec.get(); !set {
		catalog = catalog.withoutSubmission(codingAgentCodex, composerOperationExec)
	}
	return catalog
}

func operationName(operation composerOperation) string {
	switch operation {
	case composerOperationPrompt:
		return "prompt"
	case composerOperationExec:
		return "exec"
	default:
		panic("invalid composerOperation")
	}
}

func mustComposerPlan(t testing.TB, catalog *profileCatalog, operation composerOperation) composerPlan {
	t.Helper()
	plan, ok := catalog.composerPlan(codingAgentCodex, operation)
	if !ok {
		t.Fatalf("valid selected operation %s has an executable plan", operationName(operation))
	}
	return plan
}

// declaredCapabilities is the coverage the backend's profile declares per capability.
type declaredCapabilities struct {
	detection            coverage
	composer             coverage
	promptSubmission     coverage
	nativeExecSubmission coverage
	answerDecisions      []permissionDecision
}

func (c *profileCatalog) capabilities(backend codingAgent) declaredCapabilities {
	p, found := c.profile(backend)
	tested := func(declared bool) coverage {
		if declared {
			return coverageTested
		}
		return coverageUnknown
	}
	submission := func(operation composerOperation) coverage {
		return tested(found && p.submissions[operation.index()] != nil)
	}
	var decisions []permissionDecision
	if found {
		for _, a := range p.answers {
			if decision, ok := answerKeyDecisionOf(a.key); ok && !slices.Contains(decisions, decision) {
				decisions = append(decisions, decision)
			}
		}
	}
	return declaredCapabilities{
		detection:            c.detectionCoverage(backend),
		composer:             tested(found && p.composer != nil),
		promptSubmission:     submission(composerOperationPrompt),
		nativeExecSubmission: submission(composerOperationExec),
		answerDecisions:      decisions,
	}
}

func assertCapabilities(t testing.TB, label string, catalog *profileCatalog, backend codingAgent, observation, prompt, exec coverage) {
	t.Helper()
	actual := catalog.capabilities(backend)
	if actual.composer != observation || actual.promptSubmission != prompt || actual.nativeExecSubmission != exec {
		t.Fatalf("%s: composer %v prompt %v exec %v, want %v %v %v", label, actual.composer, actual.promptSubmission, actual.nativeExecSubmission, observation, prompt, exec)
	}
}

// rejectDefinition loads good, encodes a positive control with operation's plan, and returns
// the error loading bad reports.
func rejectDefinition(t testing.TB, good, bad any, operation composerOperation, label string) error {
	t.Helper()
	catalog := mustLoadCatalog(t, good)
	plan := mustComposerPlan(t, catalog, operation)
	if _, err := plan.encode("same-operation positive control"); err != nil {
		t.Fatalf("%s: positive control: %v", label, err)
	}
	_, err := loadTestCatalog(bad)
	if err == nil {
		t.Fatalf("invalid executable definition rejected: %s", label)
	}
	return err
}

// assertDefinitionRejected is rejectDefinition whose rejection is the profile usage error.
func assertDefinitionRejected(t testing.TB, good, bad any, operation composerOperation, label string) {
	t.Helper()
	err := rejectDefinition(t, good, bad, operation, label)
	if failure := cafleetErrorOf(err); failure.kind != errorUsage || !strings.HasPrefix(failure.message(), "invalid native profile: ") {
		t.Fatalf("%s: rejected with %v", label, err)
	}
}

func expectedComposerBytes(operation composerOperation, literal string, submit []byte) []byte {
	return append(append(slices.Clone(operation.prefix()), literal...), submit...)
}

func mustValidateProvenance(t testing.TB, label string, files any) {
	t.Helper()
	if err := validateProvenance(files); err != nil {
		t.Fatalf("%s: %v", label, err)
	}
}

func assertProvenanceRejected(t testing.TB, label string, files any) {
	t.Helper()
	if validateProvenance(files) == nil {
		t.Fatalf("%s: the provenance guard accepts it", label)
	}
}

func assertLoadRejected(t testing.TB, label string, files any) {
	t.Helper()
	if _, err := loadTestCatalog(files); err == nil {
		t.Fatalf("%s: the definition loads", label)
	}
}

var composerOperations = []composerOperation{composerOperationPrompt, composerOperationExec}

var operationNames = []string{"prompt", "exec"}

var multiplexerNames = []string{"tmux", "herdr"}

var evidenceKinds = []string{"framing", "plain_limit", "post_submit", "unsubmitted_literal"}

func TestCodingAgentCatalog_VerifiedComposer_VerifiedComposerRequiresAnExecutablePlanForTheSelectedOperation(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct{ prompt, exec bool }{{true, true}, {true, false}, {false, true}, {false, false}} {
		prompt, exec := none[string](), none[string]()
		if tc.prompt {
			prompt = some("0d")
		}
		if tc.exec {
			exec = some("0d")
		}
		catalog := composerCatalog(t, prompt, exec)
		if got := catalog.verifiedComposer(codingAgentCodex, composerOperationPrompt); got != tc.prompt {
			t.Fatalf("%+v: prompt verified %v", tc, got)
		}
		if got := catalog.verifiedComposer(codingAgentCodex, composerOperationExec); got != tc.exec {
			t.Fatalf("%+v: exec verified %v", tc, got)
		}
	}
}

func TestCodingAgentCatalog_VerifiedComposer_VerifiedComposerRequiresASelectedComposerDeclaration(t *testing.T) {
	t.Parallel()
	catalog := syntheticCatalog(t)
	if _, ok := catalog.composerPlan(codingAgentCodex, composerOperationPrompt); ok {
		t.Fatal("a catalog without a composer has a prompt plan")
	}
	for _, operation := range composerOperations {
		if catalog.verifiedComposer(codingAgentCodex, operation) {
			t.Fatalf("%s verified without a composer", operationName(operation))
		}
	}
}

func TestCodingAgentCatalog_VerifiedComposer_VerifiedComposerUsesTheRecordedBackendsProfile(t *testing.T) {
	t.Parallel()
	catalog := composerCatalog(t, some("0d"), some("0d"))
	if !catalog.verifiedComposer(codingAgentCodex, composerOperationPrompt) {
		t.Fatal("codex prompt is verified")
	}
	for _, backend := range codingAgentAll() {
		if backend != codingAgentCodex && catalog.verifiedComposer(backend, composerOperationPrompt) {
			t.Fatalf("%s prompt verified by the codex profile", backend.name())
		}
	}
	absent := mustLoadCatalog(t, simpleDefinition("permission_request", false))
	if absent.verifiedComposer(codingAgentCodex, composerOperationPrompt) {
		t.Fatal("a permission-only profile verifies its prompt")
	}
}

func TestCodingAgentCatalog_VerifiedComposer_VerifiedComposerFollowsTheBackendsSingleProfilePerOperation(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, singleProfiles([]singleProfile{{backend: codingAgentCodex, label: "codex"}, {backend: codingAgentClaude, label: "claude"}}))
	claude, ok := catalog.profile(codingAgentClaude)
	if !ok {
		t.Fatal("the synthetic claude profile exists")
	}
	claude.submissions[composerOperationExec.index()] = nil
	if !catalog.verifiedComposer(codingAgentCodex, composerOperationExec) {
		t.Fatal("codex exec is verified")
	}
	if catalog.verifiedComposer(codingAgentClaude, composerOperationExec) {
		t.Fatal("claude exec is verified after its plan is removed")
	}
	if !catalog.verifiedComposer(codingAgentClaude, composerOperationPrompt) {
		t.Fatal("claude prompt is verified")
	}
}

func framedDefinition(framing string, limit any) any {
	value := composerDefinition(some("0d"), some("0d"))
	for _, operation := range operationNames {
		plan := jget(value, "codex", "profile", "capabilities", "composer", "submission", operation)
		jset(plan, framing, "literal_framing")
		jset(plan, jclone(limit), "plain_literal_limit")
	}
	return value
}

func TestCodingAgentCatalog_ComposerReliability_BytePlainLimitDiagnosticsIncludeThePrefixAndMeasuredMaximumForBothRefusals(t *testing.T) {
	t.Parallel()
	cases := []struct {
		operation     composerOperation
		literal       string
		size, maximum int
	}{
		{operation: composerOperationPrompt, literal: strings.Repeat("あ", 341) + "é", size: 1025, maximum: 1024},
		{operation: composerOperationExec, literal: strings.Repeat("あ", 333), size: 1001, maximum: 1000},
	}
	refusals := []struct{ framing, cause string }{
		{framing: "plain", cause: "the codex profile has no bracketed-paste delivery"},
		{framing: "bracketed_paste", cause: "the native CLI has not enabled bracketed paste"},
	}
	for _, tc := range cases {
		for _, refusal := range refusals {
			catalog := mustLoadCatalog(t, framedDefinition(refusal.framing, jo("unit", "bytes", "max", tc.maximum)))
			plan := mustComposerPlan(t, catalog, tc.operation)
			if plan.plainLiteralAllowed(tc.literal) {
				t.Fatalf("%s %s: the literal is allowed", operationName(tc.operation), refusal.framing)
			}
			want := fmt.Sprintf("the input is %d bytes, above the codex plain-input limit of %d bytes, and %s", tc.size, tc.maximum, refusal.cause)
			if got := plan.plainLimitRefusal(tc.literal, "codex"); got != want {
				t.Fatalf("%s %s: refusal %q, want %q", operationName(tc.operation), refusal.framing, got, want)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_ScalarPlainLimitDiagnosticsIncludeThePrefixAndUseCharactersForBothRefusals(t *testing.T) {
	t.Parallel()
	cases := []struct {
		operation composerOperation
		literal   string
	}{
		{operation: composerOperationPrompt, literal: strings.Repeat("あ", 1025)},
		{operation: composerOperationExec, literal: strings.Repeat("あ", 1023)},
	}
	refusals := []struct{ framing, cause string }{
		{framing: "plain", cause: "the hermes profile has no bracketed-paste delivery"},
		{framing: "bracketed_paste", cause: "the native CLI has not enabled bracketed paste"},
	}
	for _, tc := range cases {
		for _, refusal := range refusals {
			catalog := mustLoadCatalog(t, framedDefinition(refusal.framing, jo("unit", "scalars", "max", 1024)))
			plan := mustComposerPlan(t, catalog, tc.operation)
			if plan.plainLiteralAllowed(tc.literal) {
				t.Fatalf("%s %s: the literal is allowed", operationName(tc.operation), refusal.framing)
			}
			want := "the input is 1025 characters, above the hermes plain-input limit of 1024 characters, and " + refusal.cause
			if got := plan.plainLimitRefusal(tc.literal, "hermes"); got != want {
				t.Fatalf("%s %s: refusal %q, want %q", operationName(tc.operation), refusal.framing, got, want)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_ComposerPlansRequireExplicitFramingAndPlainLimitDeclarations(t *testing.T) {
	t.Parallel()
	for _, operation := range operationNames {
		for _, key := range []string{"literal_framing", "plain_literal_limit"} {
			value := framedDefinition("plain", nil)
			jobject(jget(value, "codex", "profile", "capabilities", "composer", "submission", operation)).remove(key)
			assertLoadRejected(t, operation+": "+key, value)
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_ComposerPlansAcceptPlainOrBracketedFramingAndPositiveByteOrScalarLimits(t *testing.T) {
	t.Parallel()
	for _, framing := range []string{"plain", "bracketed_paste"} {
		for _, limit := range []any{nil, jo("unit", "bytes", "max", 1), jo("unit", "scalars", "max", 1024)} {
			if _, err := loadTestCatalog(framedDefinition(framing, limit)); err != nil {
				t.Fatalf("%s: %s: %v", framing, testJSONText(limit), err)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_ComposerPlansRejectUnknownFramingAndMalformedPlainLimits(t *testing.T) {
	t.Parallel()
	for _, framing := range []any{nil, true, 1, "", "paste"} {
		value := framedDefinition("plain", nil)
		jset(value, framing, "codex", "profile", "capabilities", "composer", "submission", "prompt", "literal_framing")
		assertLoadRejected(t, testJSONText(framing), value)
	}
	for _, limit := range []string{
		`{}`, `{"unit":"bytes"}`, `{"max":1}`, `{"unit":"cells","max":1}`, `{"unit":"bytes","max":0}`,
		`{"unit":"scalars","max":-1}`, `{"unit":"bytes","max":1.5}`, `{"unit":"bytes","max":"1"}`,
		`{"unit":"bytes","max":true}`, `{"unit":"bytes","max":1,"extra":true}`, `false`, `[]`,
	} {
		assertLoadRejected(t, limit, framedDefinition("plain", parseTestJSON(limit)))
	}
}

func TestCodingAgentCatalog_ComposerReliability_BracketedLiteralStagesPreserveThePlainOperationPrefixAndUtf8Literal(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, framedDefinition("bracketed_paste", jo("unit", "bytes", "max", 1024)))
	for _, operation := range composerOperations {
		plan := mustComposerPlan(t, catalog, operation)
		stages, err := plan.stages("literal あ", literalFramingBracketedPaste)
		if err != nil {
			t.Fatalf("%s: %v", operationName(operation), err)
		}
		want := append(append(append(slices.Clone(operation.prefix()), "\x1b[200~"...), "literal あ"...), "\x1b[201~"...)
		if !bytes.Equal(stages[0], want) || !bytes.Equal(stages[1], []byte("\r")) {
			t.Fatalf("%s: stages %q, want %q and \\r", operationName(operation), stages, want)
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_PlainLiteralStagesOmitFramingAndKeepTheSelectedSubmitKey(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, framedDefinition("bracketed_paste", nil))
	for _, operation := range composerOperations {
		plan := mustComposerPlan(t, catalog, operation)
		stages, err := plan.stages("literal あ", literalFramingPlain)
		if err != nil {
			t.Fatalf("%s: %v", operationName(operation), err)
		}
		want := append(slices.Clone(operation.prefix()), "literal あ"...)
		if !bytes.Equal(stages[0], want) || !bytes.Equal(stages[1], []byte("\r")) {
			t.Fatalf("%s: stages %q, want %q and \\r", operationName(operation), stages, want)
		}
	}
}

func TestCodingAgentCatalog_ComposerReliability_BracketedPayloadLimitCountsBothMarkersPrefixAndSubmitBytes(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, framedDefinition("bracketed_paste", nil))
	for _, operation := range composerOperations {
		plan := mustComposerPlan(t, catalog, operation)
		maximum := 65536 - len(operation.prefix()) - 12 - 1
		stages, err := plan.stages(strings.Repeat("a", maximum), literalFramingBracketedPaste)
		if err != nil {
			t.Fatalf("%s: %v", operationName(operation), err)
		}
		if total := len(stages[0]) + len(stages[1]); total != 65536 {
			t.Fatalf("%s: %d bytes, want 65536", operationName(operation), total)
		}
		if _, err := plan.stages(strings.Repeat("a", maximum+1), literalFramingBracketedPaste); err == nil {
			t.Fatalf("%s: a payload above 65536 bytes is accepted", operationName(operation))
		}
	}
}

func bundledComposerProfile(files *jsonObj) any {
	for _, member := range files.members {
		profile := jget(member.value, "profile")
		if jget(profile, "capabilities", "composer") != nil {
			return profile
		}
	}
	panic("a bundled executable composer profile")
}

func TestCodingAgentCatalog_ComposerReliability_BundledGuardRequiresAnEmptyComposerRuleForExecutablePlans(t *testing.T) {
	t.Parallel()
	value := bundledFiles()
	profile := bundledComposerProfile(value)
	var ids []string
	for _, id := range jarray(jget(profile, "capabilities", "composer", "rule_ids")).items {
		text, _ := jstring(id)
		ids = append(ids, text)
	}
	for _, r := range jarray(jget(profile, "rules")).items {
		if id, _ := jstring(jget(r, "id")); slices.Contains(ids, id) {
			jset(r, "composer_visible", "observation")
		}
	}
	assertProvenanceRejected(t, "composer rules without an empty composer", value)
}

func TestCodingAgentCatalog_ComposerReliability_BundledGuardRequiresPlanFixtureCitationsForMeasuredDelivery(t *testing.T) {
	t.Parallel()
	value := bundledFiles()
	profile := bundledComposerProfile(value)
	jset(profile, ja(), "capabilities", "composer", "submission", "prompt", "fixture_ids")
	assertProvenanceRejected(t, "an uncited prompt plan", value)
}

func submissionPath(operation string, rest ...any) []any {
	return append([]any{"codex", "profile", "capabilities", "composer", "submission", operation}, rest...)
}

// measuredDefinition is the composer definition with one witness screen per operation,
// multiplexer and evidence kind.
func measuredDefinition() any {
	value := composerDefinition(some("0d"), some("0d"))
	sequence := 0
	for _, operation := range operationNames {
		framing := any("plain")
		limit := any(jo("unit", "scalars", "max", 1024))
		if operation == "prompt" {
			framing = "bracketed_paste"
			limit = jo("unit", "bytes", "max", 1024)
		}
		jset(value, framing, submissionPath(operation, "literal_framing")...)
		jset(value, jclone(limit), submissionPath(operation, "plain_literal_limit")...)
		ids := ja(syntheticFixture)
		for _, multiplexer := range multiplexerNames {
			for _, record := range composerEvidenceRecords(operation, multiplexer, framing, limit) {
				sequence++
				id := fmt.Sprintf("witness-%d", sequence)
				jset(value, jo("native_version", syntheticVersion, "composer_evidence", ja(record)), "codex", "screens", id)
				ids.push(id)
			}
		}
		jset(value, ids, submissionPath(operation, "fixture_ids")...)
	}
	return value
}

func witnessID(value any, operation, multiplexer, kind string) string {
	for _, member := range jobject(jget(value, "codex", "screens")).members {
		records, ok := jget(member.value, "composer_evidence").(*jsonArr)
		if !ok {
			continue
		}
		for _, record := range records.items {
			if jget(record, "operation") == operation && jget(record, "multiplexer") == multiplexer && jget(record, "kind") == kind {
				return member.key
			}
		}
	}
	panic("the explicit operation/multiplexer/category witness exists")
}

func witness(value any, operation, multiplexer, kind string) any {
	return jget(value, "codex", "screens", witnessID(value, operation, multiplexer, kind))
}

func TestCodingAgentCatalog_ComposerEvidence_CompleteOperationAndMultiplexerEvidenceAcceptsUnrelatedAdditionalCitations(t *testing.T) {
	t.Parallel()
	mustValidateProvenance(t, "measured", measuredDefinition())
}

func TestCodingAgentCatalog_ComposerEvidence_EveryWitnessCategoryIsRequiredForEachOperationOnEachMultiplexer(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, kind := range evidenceKinds {
		for _, operation := range operationNames {
			for _, multiplexer := range multiplexerNames {
				value := jclone(baseline)
				id := witnessID(value, operation, multiplexer, kind)
				ids := jarray(jget(value, submissionPath(operation, "fixture_ids")...))
				ids.items = slices.DeleteFunc(ids.items, func(cited any) bool { return cited == id })
				if !slices.Contains(ids.items, any(syntheticFixture)) || len(ids.items) <= 1 {
					t.Fatalf("%s/%s/%s: the citations keep the unrelated fixture", operation, multiplexer, kind)
				}
				assertProvenanceRejected(t, operation+"/"+multiplexer+"/"+kind, value)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerEvidence_UnrelatedValidNativeCitationsCannotEstablishAnExecutablePlan(t *testing.T) {
	t.Parallel()
	value := composerDefinition(some("0d"), some("0d"))
	for _, operation := range operationNames {
		jset(value, ja(syntheticFixture), submissionPath(operation, "fixture_ids")...)
	}
	if _, err := profileCatalogFromFiles(value); err != nil {
		t.Fatalf("production decoding: %v", err)
	}
	assertProvenanceRejected(t, "unrelated citations", value)
}

func TestCodingAgentCatalog_ComposerEvidence_AnotherOperationsClaimCannotReplaceTheSelectedOperationsWitness(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, multiplexer := range multiplexerNames {
		for _, kind := range evidenceKinds {
			value := jclone(baseline)
			jset(witness(value, "prompt", multiplexer, kind), "exec", "composer_evidence", 0, "operation")
			assertProvenanceRejected(t, multiplexer+"/"+kind, value)
		}
	}
}

func TestCodingAgentCatalog_ComposerEvidence_AnotherMultiplexersClaimCannotReplaceTheSelectedMultiplexersWitness(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, operation := range operationNames {
		for _, kind := range evidenceKinds {
			value := jclone(baseline)
			jset(witness(value, operation, "tmux", kind), "herdr", "composer_evidence", 0, "multiplexer")
			assertProvenanceRejected(t, operation+"/"+kind, value)
		}
	}
}

func TestCodingAgentCatalog_ComposerEvidence_FramingEvidenceMatchesEachSelectedPlanValue(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, operation := range operationNames {
		for _, multiplexer := range multiplexerNames {
			value := jclone(baseline)
			other := "bracketed_paste"
			if operation == "prompt" {
				other = "plain"
			}
			jset(witness(value, operation, multiplexer, "framing"), other, "composer_evidence", 0, "literal_framing")
			assertProvenanceRejected(t, operation+"/"+multiplexer, value)
		}
	}
}

func TestCodingAgentCatalog_ComposerEvidence_PlainLimitEvidenceMatchesTheUnitBoundAndNullDeclaration(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, operation := range operationNames {
		unit, otherUnit := "scalars", "bytes"
		if operation == "prompt" {
			unit, otherUnit = "bytes", "scalars"
		}
		for _, multiplexer := range multiplexerNames {
			for _, limit := range []any{jo("unit", unit, "max", 512), jo("unit", otherUnit, "max", 1024), nil} {
				value := jclone(baseline)
				jset(witness(value, operation, multiplexer, "plain_limit"), limit, "composer_evidence", 0, "plain_literal_limit")
				assertProvenanceRejected(t, operation+"/"+multiplexer+" "+testJSONText(limit), value)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerEvidence_AdditionalSelectedOperationClaimsMustMatchDespiteCompleteValidCoverage(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	for _, operation := range operationNames {
		plan := jget(baseline, submissionPath(operation)...)
		framing, _ := jstring(jget(plan, "literal_framing"))
		unit, _ := jstring(jget(plan, "plain_literal_limit", "unit"))
		otherFraming := "plain"
		if framing == "plain" {
			otherFraming = "bracketed_paste"
		}
		otherUnit := "bytes"
		if unit == "bytes" {
			otherUnit = "scalars"
		}
		for _, multiplexer := range multiplexerNames {
			matching := composerEvidenceRecords(operation, multiplexer, jget(plan, "literal_framing"), jget(plan, "plain_literal_limit"))
			validExtra := jclone(baseline)
			jset(validExtra, ja(matching[:]...), "codex", "screens", syntheticFixture, "composer_evidence")
			mustValidateProvenance(t, "matching extra claims", validExtra)
			wrongFraming := jclone(matching[0])
			jset(wrongFraming, otherFraming, "literal_framing")
			mismatched := []any{wrongFraming}
			for _, limit := range []any{jo("unit", unit, "max", 512), jo("unit", otherUnit, "max", 1024), nil} {
				record := jclone(matching[1])
				jset(record, limit, "plain_literal_limit")
				mismatched = append(mismatched, record)
			}
			for _, record := range mismatched {
				value := jclone(baseline)
				jset(value, ja(record), "codex", "screens", syntheticFixture, "composer_evidence")
				assertProvenanceRejected(t, "the additional selected claim contradicts its plan: "+testJSONText(record), value)
			}
		}
	}
}

func unlimitedDefinition() any {
	value := measuredDefinition()
	for _, operation := range operationNames {
		jset(value, nil, submissionPath(operation, "plain_literal_limit")...)
		for _, multiplexer := range multiplexerNames {
			jset(witness(value, operation, multiplexer, "plain_limit"), nil, "composer_evidence", 0, "plain_literal_limit")
		}
	}
	return value
}

func TestCodingAgentCatalog_ComposerEvidence_ExplicitlyMeasuredNullPlainLimitsAreValid(t *testing.T) {
	t.Parallel()
	mustValidateProvenance(t, "unlimited", unlimitedDefinition())
}

func TestCodingAgentCatalog_ComposerEvidence_OneFixtureCanCarryDistinctOperationMultiplexerAndCategoryClaims(t *testing.T) {
	t.Parallel()
	value := measuredDefinition()
	records := ja()
	for _, member := range jobject(jget(value, "codex", "screens")).members {
		if evidence, ok := jget(member.value, "composer_evidence").(*jsonArr); ok {
			for _, record := range evidence.items {
				records.push(jclone(record))
			}
		}
	}
	jset(value, records, "codex", "screens", syntheticFixture, "composer_evidence")
	for _, operation := range operationNames {
		jset(value, ja(syntheticFixture), submissionPath(operation, "fixture_ids")...)
	}
	mustValidateProvenance(t, "one fixture with every claim", value)
}

func TestCodingAgentCatalog_ComposerEvidence_MissingPlainLimitKeyIsRejectedEvenWhenThePlanIsUnlimited(t *testing.T) {
	t.Parallel()
	value := unlimitedDefinition()
	mustValidateProvenance(t, "unlimited", value)
	jset(value, parseTestJSON(`[{"operation":"prompt","multiplexer":"tmux","kind":"plain_limit"}]`), "codex", "screens", syntheticFixture, "composer_evidence")
	assertProvenanceRejected(t, "a plain_limit record without its limit", value)
}

func TestCodingAgentCatalog_ComposerEvidence_EvidenceRecordsHaveStrictRequiredFieldsAndValueDomains(t *testing.T) {
	t.Parallel()
	baseline := measuredDefinition()
	mustValidateProvenance(t, "baseline", baseline)
	framing := parseTestJSON(`{"operation":"prompt","multiplexer":"tmux","kind":"framing","literal_framing":"bracketed_paste"}`)
	var malformed []any
	for _, field := range []string{"operation", "multiplexer", "kind", "literal_framing"} {
		record := jclone(framing)
		if _, removed := jobject(record).remove(field); !removed {
			t.Fatalf("the framing record has %s", field)
		}
		malformed = append(malformed, record)
	}
	for _, change := range []struct {
		field string
		value any
	}{{"operation", "preview"}, {"multiplexer", "screen"}, {"kind", "idle"}, {"literal_framing", "automatic"}, {"unexpected", true}} {
		record := jclone(framing)
		jset(record, change.value, change.field)
		malformed = append(malformed, record)
	}
	for _, limit := range []string{`{"unit":"bytes","max":0}`, `{"unit":"characters","max":1}`, `{"unit":"bytes","max":1,"extra":true}`} {
		malformed = append(malformed, jo("operation", "prompt", "multiplexer", "tmux", "kind", "plain_limit", "plain_literal_limit", parseTestJSON(limit)))
	}
	for _, kind := range []string{"post_submit", "unsubmitted_literal"} {
		malformed = append(malformed, jo("operation", "prompt", "multiplexer", "tmux", "kind", kind, "literal_framing", "plain"))
	}
	malformed = append(malformed, parseTestJSON(`{"operation":"prompt","multiplexer":"tmux","kind":"unsubmitted_literal","expected_literal":"typed","literal_framing":"plain"}`))
	for _, record := range malformed {
		value := jclone(baseline)
		jset(value, ja(record), "codex", "screens", syntheticFixture, "composer_evidence")
		assertProvenanceRejected(t, testJSONText(record), value)
	}
}

func withEvidenceRecord(record any) any {
	value := measuredDefinition()
	jset(value, ja(record), "codex", "screens", syntheticFixture, "composer_evidence")
	return value
}

func TestCodingAgentCatalog_ComposerEvidence_UnsubmittedLiteralRecordsRequireAnExpectedLiteralAndOtherKindsRefuseOne(t *testing.T) {
	t.Parallel()
	unsubmitted := parseTestJSON(`{"operation":"prompt","multiplexer":"tmux","kind":"unsubmitted_literal","expected_literal":"typed"}`)
	mustValidateProvenance(t, testJSONText(unsubmitted), withEvidenceRecord(unsubmitted))
	missing := jclone(unsubmitted)
	jobject(missing).remove("expected_literal")
	assertProvenanceRejected(t, "an unsubmitted_literal record without expected_literal", withEvidenceRecord(missing))
	framing := parseTestJSON(`{"operation":"prompt","multiplexer":"tmux","kind":"framing","literal_framing":"bracketed_paste"}`)
	mustValidateProvenance(t, testJSONText(framing), withEvidenceRecord(framing))
	for _, misplaced := range []string{
		`{"operation":"prompt","multiplexer":"tmux","kind":"framing","literal_framing":"bracketed_paste","expected_literal":"! true"}`,
		`{"operation":"prompt","multiplexer":"tmux","kind":"plain_limit","plain_literal_limit":null,"expected_literal":"typed"}`,
		`{"operation":"prompt","multiplexer":"tmux","kind":"post_submit","expected_literal":"typed"}`,
	} {
		assertProvenanceRejected(t, misplaced, withEvidenceRecord(parseTestJSON(misplaced)))
	}
}

func TestCodingAgentCatalog_ComposerEvidence_AnExpectedLiteralIsAStringOrAWellFormedRepeatObject(t *testing.T) {
	t.Parallel()
	record := func(literal any) any {
		return jo("operation", "exec", "multiplexer", "herdr", "kind", "unsubmitted_literal", "expected_literal", literal)
	}
	for _, accepted := range []string{`""`, `{"repeat":"a","count":1000,"prefix":"! "}`, `{"repeat":"あ","count":4000,"prefix":""}`} {
		mustValidateProvenance(t, accepted, withEvidenceRecord(record(parseTestJSON(accepted))))
	}
	for _, malformed := range []string{
		`7`, `null`, `["a"]`, `{"repeat":"a","count":3}`, `{"repeat":"a","prefix":""}`, `{"count":3,"prefix":""}`,
		`{"repeat":"a","count":0,"prefix":""}`, `{"repeat":"a","count":-1,"prefix":""}`, `{"repeat":"a","count":"3","prefix":""}`,
		`{"repeat":"","count":3,"prefix":""}`, `{"repeat":"a","count":3,"prefix":"","extra":true}`,
	} {
		assertProvenanceRejected(t, malformed, withEvidenceRecord(record(parseTestJSON(malformed))))
	}
}

func TestCodingAgentCatalog_ComposerEvidence_DuplicateOperationMultiplexerCategoryRecordsOnOneFixtureAreRejected(t *testing.T) {
	t.Parallel()
	value := measuredDefinition()
	mustValidateProvenance(t, "measured", value)
	fixture := witness(value, "prompt", "tmux", "post_submit")
	records := jarray(jget(fixture, "composer_evidence"))
	records.push(jclone(records.items[0]))
	assertProvenanceRejected(t, "a duplicate record", value)
}

func TestCodingAgentCatalog_ComposerEvidence_APresentEmptyEvidenceArrayIsRejectedWithoutAffectingOtherValidCitations(t *testing.T) {
	t.Parallel()
	value := measuredDefinition()
	mustValidateProvenance(t, "measured", value)
	jset(value, ja(), "codex", "screens", syntheticFixture, "composer_evidence")
	assertProvenanceRejected(t, "an empty evidence array", value)
}

func TestCodingAgentCatalog_ComposerEvidence_SyntheticTestHelperPopulatesAbsentEvidenceForDynamicPlanValues(t *testing.T) {
	t.Parallel()
	value := composerDefinition(some("0d"), some("0d"))
	for _, operation := range operationNames {
		jset(value, "bracketed_paste", submissionPath(operation, "literal_framing")...)
		jset(value, jo("unit", "bytes", "max", 3), submissionPath(operation, "plain_literal_limit")...)
	}
	decorated, err := withSyntheticComposerEvidence(value)
	if err != nil {
		t.Fatal(err)
	}
	mustValidateProvenance(t, "decorated", decorated)
}

func TestCodingAgentCatalog_ComposerEvidence_SyntheticTestHelperPreservesExplicitEvidenceGaps(t *testing.T) {
	t.Parallel()
	value := measuredDefinition()
	for _, operation := range operationNames {
		ids := jarray(jget(value, submissionPath(operation, "fixture_ids")...))
		ids.items = slices.DeleteFunc(ids.items, func(id any) bool { return id == syntheticFixture })
	}
	mustLoadCatalog(t, value)
	jset(witness(value, "prompt", "tmux", "post_submit"), parseTestJSON(`[{"operation":"exec","multiplexer":"tmux","kind":"post_submit"}]`), "composer_evidence")
	assertLoadRejected(t, "a replaced witness", value)
}

func withPanelSeparator(pointer string, separator any) any {
	value := syntheticDefinitions()
	rule, ok := jpointer(value, pointer)
	if !ok {
		panic("the pointer names a rule")
	}
	jset(rule, separator, "region", "panel_separator")
	return value
}

func TestCodingAgentCatalog_PanelSeparator_ARegionLoadsWithOneNonWhitespacePanelSeparatorGlyph(t *testing.T) {
	t.Parallel()
	mustLoadCatalog(t, withPanelSeparator("/codex/profile/rules/0", "│"))
	mustLoadCatalog(t, questionFixtureDefinition())
}

const (
	questionPointer = "/codex/profile/rules/1"
	answersPointer  = "/codex/profile/capabilities/answers"
)

func loadRefusal(t testing.TB, value any) string {
	t.Helper()
	mustLoadCatalog(t, questionFixtureDefinition())
	_, err := loadTestCatalog(value)
	if err == nil {
		t.Fatal("the changed definition loads")
	}
	return err.Error()
}

func changedQuestionDefinition(change func(value any)) any {
	value := questionFixtureDefinition()
	change(value)
	return value
}

func questionRuleOf(value any) any {
	r, ok := jpointer(value, questionPointer)
	if !ok {
		panic("the question rule exists")
	}
	return r
}

func answersOf(value any) *jsonArr {
	answers, ok := jpointer(value, answersPointer)
	if !ok {
		panic("the answers exist")
	}
	return jarray(answers)
}

func codexPlan(catalog *profileCatalog, request string, key answerKey) (answerPlan, bool) {
	return catalog.answerPlan(codingAgentCodex, request, key)
}

func TestCodingAgentCatalog_QuestionAnswers_AChoiceAnswerResolvesByItsQuestionRuleAndChoiceKey(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, questionFixtureDefinition())
	decline, ok := codexPlan(catalog, questionRule, answerKeyChoice{choice: "decline"})
	if !ok {
		t.Fatal("decline plan")
	}
	capability := decline.capability
	if capability.key != (answerKeyChoice{choice: "decline"}) || capability.backend != codingAgentCodex ||
		capability.requestRuleID != questionRule || capability.answerRuleID != questionDeclinePlan {
		t.Fatalf("decline capability %+v", capability)
	}
	if decline.choiceID != "decline" || decline.focusMatcherID != "focus" {
		t.Fatalf("decline plan choice %q focus %q", decline.choiceID, decline.focusMatcherID)
	}
	type stageView struct {
		bytes string
		focus string
		gated bool
	}
	var stages []stageView
	for _, stage := range decline.stages {
		focus, gated := stage.requiredFocusChoiceID()
		stages = append(stages, stageView{bytes: string(stage.bytes), focus: focus, gated: gated})
	}
	if want := []stageView{{"\x0e", "accept", true}, {"\x0d", "decline", true}}; !slices.Equal(stages, want) {
		t.Fatalf("decline stages %+v, want %+v", stages, want)
	}
	later, ok := codexPlan(catalog, questionRule, answerKeyChoice{choice: "later"})
	if !ok || later.capability.answerRuleID != questionLaterPlan || len(later.stages) != 1 ||
		string(later.stages[0].bytes) != "3" || later.stages[0].gate != nil {
		t.Fatalf("later plan %v %+v", ok, later.capability)
	}
	for _, key := range []answerKey{answerKeyChoice{choice: "accept"}, answerKeyChoice{choice: "missing"},
		answerKeyDecision{decision: permissionDecisionAllow}, answerKeyDecision{decision: permissionDecisionDeny}} {
		if _, ok := codexPlan(catalog, questionRule, key); ok {
			t.Fatalf("the question rule has a plan for %#v", key)
		}
	}
	if _, ok := codexPlan(catalog, questionNoticeRule, answerKeyChoice{choice: "decline"}); ok {
		t.Fatal("the notice has a plan")
	}
	allow, ok := codexPlan(catalog, syntheticRule, answerKeyDecision{decision: permissionDecisionAllow})
	if !ok || allow.capability.key != (answerKeyDecision{decision: permissionDecisionAllow}) {
		t.Fatal("the decision plan beside the choice plans")
	}
	if _, ok := codexPlan(catalog, syntheticRule, answerKeyChoice{choice: "allow-once"}); ok {
		t.Fatal("the permission rule has a choice plan")
	}
	if _, ok := codexPlan(catalog, syntheticRule, answerKeyDecision{decision: permissionDecisionDeny}); ok {
		t.Fatal("the permission rule has a deny plan")
	}
}

func TestCodingAgentCatalog_QuestionAnswers_AChoiceAnswerLoadsForEveryDeclaredChoiceScope(t *testing.T) {
	t.Parallel()
	for _, scope := range []string{"single_request", "persistent"} {
		value := changedQuestionDefinition(func(value any) {
			jset(questionRuleOf(value), scope, "extract", "choices", 2, "scope")
		})
		catalog := mustLoadCatalog(t, value)
		if _, ok := codexPlan(catalog, questionRule, answerKeyChoice{choice: "later"}); !ok {
			t.Fatalf("%s: no later plan", scope)
		}
	}
}

func TestCodingAgentCatalog_QuestionAnswers_AnAnswerOutsideItsFormScopeOrChoiceSetIsRefused(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		change  func(value any)
		message string
	}{
		{
			name: "a choice answer on a permission rule",
			change: func(value any) {
				answersOf(value).push(choiceAnswer(syntheticRule, "synthetic-choice-on-permission", "deny-once",
					ja(jo("bytes_hex", "6e", "required_rule_id", syntheticRule, "required_choice_id", "deny-once"))))
			},
			message: "invalid native profile: choice answer requires question form",
		},
		{
			name: "a decision answer on a question rule",
			change: func(value any) {
				entry := jclone(answersOf(value).items[1])
				jset(entry, "allow", "decision")
				jset(entry, "synthetic-decision-on-question", "answer_rule_id")
				jset(entry, ja(questionStage("33", "decline", none[string]())), "stages")
				answersOf(value).push(entry)
			},
			message: "invalid native profile: answer requires permission form",
		},
		{
			name: "a decision answer on a persistent choice",
			change: func(value any) {
				jset(value, "persistent", "codex", "profile", "rules", 0, "extract", "choices", 0, "scope")
			},
			message: "invalid native profile: answer requires single-request scope",
		},
		{
			name: "a choice answer naming no rule choice",
			change: func(value any) {
				jset(answersOf(value).items[2], "never", "choice_id")
				jset(answersOf(value).items[2], ja(questionStage("33", "never", none[string]())), "stages")
			},
			message: "invalid native profile: unknown answer choice",
		},
		{
			name: "a duplicate question rule and choice pair",
			change: func(value any) {
				duplicate := jclone(answersOf(value).items[2])
				jset(duplicate, "synthetic-question-later-again", "answer_rule_id")
				answersOf(value).push(duplicate)
			},
			message: "invalid native profile: ",
		},
	}
	for _, tc := range cases {
		if refused := loadRefusal(t, changedQuestionDefinition(tc.change)); !strings.HasPrefix(refused, tc.message) {
			t.Fatalf("%s: %s", tc.name, refused)
		}
	}
}

func TestCodingAgentCatalog_QuestionAnswers_AChoiceIdFollowsTheLowercaseHyphenatedShapeUpTo64Bytes(t *testing.T) {
	t.Parallel()
	renamed := func(id string) any {
		return changedQuestionDefinition(func(value any) {
			jset(questionRuleOf(value), id, "extract", "choices", 2, "id")
			jset(answersOf(value).items[2], id, "choice_id")
			jset(answersOf(value).items[2], ja(questionStage("33", id, none[string]())), "stages")
		})
	}
	for _, id := range []string{"a", strings.Repeat("a", 64)} {
		catalog := mustLoadCatalog(t, renamed(id))
		if _, ok := codexPlan(catalog, questionRule, answerKeyChoice{choice: id}); !ok {
			t.Fatalf("%s: no plan", id)
		}
	}
	for _, id := range []string{"Later", "-later", "ask--later", strings.Repeat("a", 65)} {
		assertLoadRejected(t, id+" is outside the choice-id shape", renamed(id))
	}
}

func TestCodingAgentCatalog_QuestionAnswers_MarkerFocusLoadsAndGatesAChoicePlan(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, questionFixtureDefinition())
	if _, ok := codexPlan(catalog, questionRule, answerKeyChoice{choice: "decline"}); !ok {
		t.Fatal("decline plan")
	}
	unstyled := changedQuestionDefinition(func(value any) {
		jobject(jget(questionRuleOf(value), "matchers", 1)).remove("focus_style")
	})
	if got := loadRefusal(t, unstyled); got != "invalid native profile: a focus-gated plan needs a focus_style matcher" {
		t.Fatalf("unstyled refusal %q", got)
	}
	background := changedQuestionDefinition(func(value any) {
		jset(questionRuleOf(value), "background", "matchers", 1, "focus_style")
	})
	mustLoadCatalog(t, background)
}

func TestCodingAgentCatalog_QuestionAnswers_MarkerFocusGatesADecisionPlanAsBackgroundFocusDoes(t *testing.T) {
	t.Parallel()
	marker := gatedFixtureDefinition()
	jset(marker, "marker", "codex", "profile", "rules", 0, "matchers", 2, "focus_style")
	catalog := mustLoadCatalog(t, marker)
	deny, ok := codexPlan(catalog, gatedRule, answerKeyDecision{decision: permissionDecisionDeny})
	if !ok {
		t.Fatal("a marker-gated decision plan")
	}
	if focus, gated := deny.stages[0].requiredFocusChoiceID(); !gated || focus != "allow-once" {
		t.Fatalf("first stage focus %q %v", focus, gated)
	}
}

func TestCodingAgentCatalog_QuestionAnswers_AnInteractionFrameLoadsWithARejectPatternAndOptionalRequirePatterns(t *testing.T) {
	t.Parallel()
	for _, require := range []any{ja(), ja("^Synthetic question\\?$")} {
		value := changedQuestionDefinition(func(value any) {
			jset(questionRuleOf(value), jo("require_patterns", require, "reject_patterns", ja("^Live composer$")), "frame")
		})
		mustLoadCatalog(t, value)
	}
	permission := changedQuestionDefinition(func(value any) {
		jset(value, jo("require_patterns", ja(), "reject_patterns", ja("^Live composer$")), "codex", "profile", "rules", 0, "frame")
	})
	mustLoadCatalog(t, permission)
}

func TestCodingAgentCatalog_QuestionAnswers_AnInteractionFrameRequiresARejectPattern(t *testing.T) {
	t.Parallel()
	value := changedQuestionDefinition(func(value any) {
		jset(questionRuleOf(value), jo("require_patterns", ja(), "reject_patterns", ja()), "frame")
	})
	if got := loadRefusal(t, value); got != "invalid native profile: interaction frame requires a reject pattern" {
		t.Fatalf("refusal %q", got)
	}
}

const (
	nativePrimaryFixture    = "native-permission-one"
	nativeRegressionFixture = "native-regression-one"
	nativeClaudeFixture     = "native-claude-one"
)

func nativeFixtureScreen(line string) any {
	lines := make([]string, 24)
	lines[0] = line
	return jo("native_version", "1.2.3", "rows", 24, "cols", 80, "alternate_screen", true, "screen_seq", 0,
		"cursor_row", 23, "cursor_col", 0, "lines", lines, "masked_rows", ja())
}

// nativeFixtureCatalog is a codex profile file citing the primary screen for detection and
// holding the uncited regression screen beside it.
type nativeFixtureCatalog struct{ files any }

func newNativeFixtureCatalog() *nativeFixtureCatalog {
	files := syntheticDefinitions()
	jset(files, jo(nativePrimaryFixture, nativeFixtureScreen("permission screen"), nativeRegressionFixture, nativeFixtureScreen("regression screen")), "codex", "screens")
	jset(files, ja(nativePrimaryFixture), "codex", "profile", "capabilities", "detection", "fixture_ids")
	return &nativeFixtureCatalog{files: files}
}

// nativeFixtureCatalogWithClaude adds a claude profile file citing its own screen id.
func nativeFixtureCatalogWithClaude(screenID string) *nativeFixtureCatalog {
	fixture := newNativeFixtureCatalog()
	claude := jclone(jget(fixture.files, "codex"))
	jset(claude, "claude", "backend")
	jset(claude, jo(screenID, nativeFixtureScreen("claude screen")), "screens")
	jset(claude, ja(screenID), "profile", "capabilities", "detection", "fixture_ids")
	jset(fixture.files, claude, "claude")
	return fixture
}

func maskedNativeFixtureCatalog() *nativeFixtureCatalog {
	fixture := newNativeFixtureCatalog()
	fixture.changePrimary(func(s any) { pushMask(s, 5, "reviewed_privacy") })
	return fixture
}

func (f *nativeFixtureCatalog) load() (*profileCatalog, error) {
	return profileCatalogFromBundledFiles(f.files)
}

func (f *nativeFixtureCatalog) changePrimary(change func(any)) {
	change(jget(f.files, "codex", "screens", nativePrimaryFixture))
}

func usageFailure(t testing.TB, catalog *profileCatalog, err error) string {
	t.Helper()
	if err == nil {
		t.Fatal("native loader rejects the invalid bundled file")
	}
	if kind := cafleetErrorOf(err).kind; kind != errorUsage {
		t.Fatalf("native loader returns a usage error: %v", err)
	}
	return err.Error()
}

func failureMessage(_ *profileCatalog, err error) opt[string] {
	if err == nil {
		return none[string]()
	}
	return some(err.Error())
}

func embeddedProfileCatalog() (*profileCatalog, error) {
	files := jo()
	for _, backend := range codingAgentAll() {
		text, ok := bundledProfileText(backend)
		if !ok {
			continue
		}
		file, err := parseBundledText(text)
		if err != nil {
			return nil, err
		}
		files.set(backend.name(), file)
	}
	return profileCatalogFromBundledFiles(files)
}

func changedNativeFixtureCatalog(change func(files any)) (*profileCatalog, error) {
	fixture := newNativeFixtureCatalog()
	change(fixture.files)
	return fixture.load()
}

func TestCodingAgentCatalog_BundledBackendFiles_TheBundledCatalogGuardAcceptsValidProfileFilesAndNamesEachViolation(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		load     func() (*profileCatalog, error)
		expected opt[string]
	}{
		{name: "the embedded files", load: embeddedProfileCatalog, expected: none[string]()},
		{name: "an uncited screen", load: func() (*profileCatalog, error) { return newNativeFixtureCatalog().load() }, expected: none[string]()},
		{
			name: "a cross-backend citation",
			load: func() (*profileCatalog, error) {
				fixture := nativeFixtureCatalogWithClaude(nativeClaudeFixture)
				jset(fixture.files, ja(nativeClaudeFixture), "codex", "profile", "capabilities", "detection", "fixture_ids")
				return fixture.load()
			},
			expected: some("invalid native profile: codex cites unknown fixture native-claude-one"),
		},
		{
			name:     "a fixture id in two profile files",
			load:     func() (*profileCatalog, error) { return nativeFixtureCatalogWithClaude(nativePrimaryFixture).load() },
			expected: some("invalid native profile: duplicate fixture id native-permission-one"),
		},
		{
			name: "a screen id repeated within one file's text",
			load: func() (*profileCatalog, error) {
				text := strings.ReplaceAll(testJSONText(jget(newNativeFixtureCatalog().files, "codex")),
					`"`+nativeRegressionFixture+`"`, `"`+nativePrimaryFixture+`"`)
				file, err := parseBundledText([]byte(text))
				if err != nil {
					return nil, err
				}
				return profileCatalogFromBundledFiles(jo("codex", file))
			},
			expected: some("invalid native profile: duplicate object key native-permission-one"),
		},
		{
			name: "a file backend other than its file",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) { jset(files, "claude", "codex", "backend") })
			},
			expected: some("invalid native profile: profile file backend differs from its file"),
		},
		{
			name: "schema_version 2",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) { jset(files, 2, "codex", "schema_version") })
			},
			expected: some("invalid native profile: unsupported profile schema"),
		},
		{
			name: "a bundled screen without native_version",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) {
					jobject(jget(files, "codex", "screens", nativePrimaryFixture)).remove("native_version")
				})
			},
			expected: some("invalid native profile: exact required object fields expected"),
		},
		{
			name: "an uncited bundled screen with an extra key",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) { jset(files, "x", "codex", "screens", nativeRegressionFixture, "note") })
			},
			expected: some("invalid native profile: exact required object fields expected"),
		},
		{
			name: "a bundled stub screen",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) { jset(files, fixtureAt(syntheticVersion), "codex", "screens", nativePrimaryFixture) })
			},
			expected: some("invalid native profile: exact required object fields expected"),
		},
		{
			name: "a bundled screen with empty composer_evidence",
			load: func() (*profileCatalog, error) {
				return changedNativeFixtureCatalog(func(files any) { jset(files, ja(), "codex", "screens", nativePrimaryFixture, "composer_evidence") })
			},
			expected: some("invalid native profile: present composer evidence must be nonempty"),
		},
	}
	for _, tc := range cases {
		if got := failureMessage(tc.load()); got != tc.expected {
			t.Fatalf("%s: failure %v, want %v", tc.name, got, tc.expected)
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_TheBundledCatalogGuardReportsANoncanonicalScreenNativeVersion(t *testing.T) {
	t.Parallel()
	fixture := newNativeFixtureCatalog()
	fixture.changePrimary(func(s any) { jset(s, "1.2", "native_version") })
	catalog, err := fixture.load()
	if failure := usageFailure(t, catalog, err); !strings.Contains(failure, "canonical SemVer version required") {
		t.Fatalf("failure %q", failure)
	}
}

func firstMaskedOrEmptyRow(screen any) uint64 {
	if masks := jarray(jget(screen, "masked_rows")).items; len(masks) > 0 {
		row, _ := ju64(jget(masks[0], "row"))
		return row
	}
	for index, line := range jarray(jget(screen, "lines")).items {
		if line == "" {
			return uint64(index)
		}
	}
	return 0
}

func pushMask(screen any, row uint64, reason string) {
	jarray(jget(screen, "masked_rows")).push(jo("row", row, "reason", reason))
}

var nativeScreenChanges = []struct {
	name   string
	change func(s any)
}{
	{name: "extra field", change: func(s any) { jset(s, "unreviewed", "note") }},
	{name: "missing masked rows", change: func(s any) { jobject(s).remove("masked_rows") }},
	{name: "nonempty masked row", change: func(s any) {
		if masks := jarray(jget(s, "masked_rows")).items; len(masks) > 0 {
			row, _ := ju64(jget(masks[0], "row"))
			jset(s, "restored header", "lines", int(row))
			return
		}
		for index, line := range jarray(jget(s, "lines")).items {
			if line != "" {
				pushMask(s, uint64(index), "reviewed_privacy")
				return
			}
		}
		panic("a nonempty line")
	}},
	{name: "masked row out of range", change: func(s any) {
		rows, _ := ju64(jget(s, "rows"))
		pushMask(s, rows, "reviewed_privacy")
	}},
	{name: "duplicate masked row", change: func(s any) {
		row := firstMaskedOrEmptyRow(s)
		if len(jarray(jget(s, "masked_rows")).items) == 0 {
			pushMask(s, row, "reviewed_privacy")
		}
		pushMask(s, row, "reviewed_privacy")
	}},
	{name: "unreviewed mask reason", change: func(s any) {
		if len(jarray(jget(s, "masked_rows")).items) == 0 {
			pushMask(s, firstMaskedOrEmptyRow(s), "convenience")
		} else {
			jset(s, "convenience", "masked_rows", 0, "reason")
		}
	}},
	{name: "rows", change: func(s any) {
		rows, _ := ju64(jget(s, "rows"))
		jset(s, rows-1, "rows")
	}},
	{name: "line count", change: func(s any) {
		lines := jarray(jget(s, "lines"))
		lines.items = lines.items[:len(lines.items)-1]
	}},
	{name: "zero cols", change: func(s any) { jset(s, 0, "cols") }},
	{name: "cursor row outside the geometry", change: func(s any) { jset(s, jclone(jget(s, "rows")), "cursor_row") }},
	{name: "cursor column past the geometry", change: func(s any) {
		cols, _ := ju64(jget(s, "cols"))
		jset(s, cols+1, "cursor_col")
	}},
}

func TestCodingAgentCatalog_BundledBackendFiles_AScreenOutsideTheReviewedShapeIsRejected(t *testing.T) {
	t.Parallel()
	for _, base := range []struct {
		name string
		make func() *nativeFixtureCatalog
	}{{"unmasked", newNativeFixtureCatalog}, {"masked", maskedNativeFixtureCatalog}} {
		if got := failureMessage(base.make().load()); got != none[string]() {
			t.Fatalf("%s: the base fails: %v", base.name, got)
		}
		for _, tc := range nativeScreenChanges {
			fixture := base.make()
			fixture.changePrimary(tc.change)
			if _, err := fixture.load(); err == nil {
				t.Fatalf("%s: %s must be rejected", base.name, tc.name)
			}
		}
		pendingWrap := base.make()
		pendingWrap.changePrimary(func(s any) { jset(s, jclone(jget(s, "cols")), "cursor_col") })
		if got := failureMessage(pendingWrap.load()); got != none[string]() {
			t.Fatalf("%s: cursor_col equal to cols records the pending-wrap position: %v", base.name, got)
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_CellBackgroundsOutsideTheReviewedShapeAreRejected(t *testing.T) {
	t.Parallel()
	run := func(row, start, end int, bg any) any {
		return jo("row", row, "start_col", start, "end_col", end, "bg", jclone(bg))
	}
	rgb := parseTestJSON(`{"kind":"rgb","r":1,"g":2,"b":3}`)
	indexed := parseTestJSON(`{"kind":"indexed","index":255}`)
	load := func(runs any) opt[string] {
		fixture := maskedNativeFixtureCatalog()
		fixture.changePrimary(func(s any) { jset(s, runs, "cell_backgrounds") })
		return failureMessage(fixture.load())
	}
	if got := load(ja(run(0, 0, 4, rgb), run(0, 4, 80, indexed), run(23, 0, 80, rgb))); got != none[string]() {
		t.Fatalf("adjacent full-geometry runs are reviewed: %v", got)
	}
	cases := []struct {
		name string
		runs any
	}{
		{"not an array", parseTestJSON(`{"row":0}`)},
		{"run missing a field", ja(jo("row", 0, "start_col", 0, "bg", jclone(rgb)))},
		{"row below the geometry", ja(run(24, 0, 4, rgb))},
		{"start past the width", ja(run(0, 81, 82, rgb))},
		{"end past the width", ja(run(0, 0, 81, rgb))},
		{"empty run", ja(run(0, 4, 4, rgb))},
		{"masked row", ja(run(5, 0, 4, rgb))},
		{"default colour", ja(run(0, 0, 4, parseTestJSON(`{"kind":"default"}`)))},
		{"indexed colour out of range", ja(run(0, 0, 4, parseTestJSON(`{"kind":"indexed","index":256}`)))},
		{"rgb channel out of range", ja(run(0, 0, 4, parseTestJSON(`{"kind":"rgb","r":256,"g":0,"b":0}`)))},
		{"overlapping runs", ja(run(0, 0, 6, rgb), run(0, 5, 9, rgb))},
	}
	for _, tc := range cases {
		if load(tc.runs) == none[string]() {
			t.Fatalf("%s must be rejected", tc.name)
		}
	}
}

func styleRun(row, start, end int, faint bool, foreground any) any {
	return jo("row", row, "start_col", start, "end_col", end, "faint", faint, "foreground", jclone(foreground))
}

func TestCodingAgentCatalog_BundledBackendFiles_CellStylesOutsideTheReviewedShapeAreRejected(t *testing.T) {
	t.Parallel()
	rgb := parseTestJSON(`{"kind":"rgb","r":1,"g":2,"b":3}`)
	indexed := parseTestJSON(`{"kind":"indexed","index":8}`)
	load := func(runs any) opt[string] {
		fixture := maskedNativeFixtureCatalog()
		fixture.changePrimary(func(s any) { jset(s, runs, "cell_styles") })
		return failureMessage(fixture.load())
	}
	if got := load(ja(styleRun(0, 0, 4, true, nil), styleRun(0, 4, 80, false, indexed), styleRun(23, 0, 80, true, rgb))); got != none[string]() {
		t.Fatalf("adjacent full-geometry faint, foreground and combined runs are reviewed: %v", got)
	}
	layered := maskedNativeFixtureCatalog()
	layered.changePrimary(func(s any) {
		jset(s, ja(jo("row", 0, "start_col", 0, "end_col", 4, "bg", jclone(rgb))), "cell_backgrounds")
		jset(s, ja(styleRun(0, 0, 4, true, indexed)), "cell_styles")
	})
	if got := failureMessage(layered.load()); got != none[string]() {
		t.Fatalf("style runs may cover background runs: %v", got)
	}
	cases := []struct {
		name string
		runs any
	}{
		{"not an array", parseTestJSON(`{"row":0}`)},
		{"run missing a field", parseTestJSON(`[{"row":0,"start_col":0,"end_col":4,"faint":true}]`)},
		{"run with an extra field", parseTestJSON(`[{"row":0,"start_col":0,"end_col":4,"faint":true,"foreground":null,"bold":true}]`)},
		{"row below the geometry", ja(styleRun(24, 0, 4, true, nil))},
		{"start past the width", ja(styleRun(0, 81, 82, true, nil))},
		{"end past the width", ja(styleRun(0, 0, 81, true, nil))},
		{"empty run", ja(styleRun(0, 4, 4, true, nil))},
		{"masked row", ja(styleRun(5, 0, 4, true, nil))},
		{"default style", ja(styleRun(0, 0, 4, false, nil))},
		{"non-boolean faint", parseTestJSON(`[{"row":0,"start_col":0,"end_col":4,"faint":1,"foreground":null}]`)},
		{"default foreground colour", ja(styleRun(0, 0, 4, false, parseTestJSON(`{"kind":"default"}`)))},
		{"indexed foreground out of range", ja(styleRun(0, 0, 4, false, parseTestJSON(`{"kind":"indexed","index":256}`)))},
		{"rgb foreground channel out of range", ja(styleRun(0, 0, 4, false, parseTestJSON(`{"kind":"rgb","r":0,"g":256,"b":0}`)))},
		{"overlapping runs", ja(styleRun(0, 0, 6, true, nil), styleRun(0, 5, 9, false, indexed))},
	}
	for _, tc := range cases {
		if load(tc.runs) == none[string]() {
			t.Fatalf("%s must be rejected", tc.name)
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_FixtureReplayAppliesEachStyleRunAndKeepsItsBackground(t *testing.T) {
	t.Parallel()
	fixture := nativeFixtureScreen("faint grey both plain")
	jset(fixture, parseTestJSON(`[{"row":0,"start_col":11,"end_col":15,"bg":{"kind":"indexed","index":4}}]`), "cell_backgrounds")
	jset(fixture, ja(
		styleRun(0, 0, 5, true, nil),
		styleRun(0, 6, 10, false, parseTestJSON(`{"kind":"indexed","index":8}`)),
		styleRun(0, 11, 15, true, parseTestJSON(`{"kind":"rgb","r":10,"g":20,"b":30}`)),
	), "cell_styles")
	replayed := replayNativeFixture(mustCanonicalJSON(fixture), nil, true, none[[2]uint16]())
	cell := func(col uint16) *vt.Cell { return replayed.screen.Cell(0, col) }
	if got := replayed.screen.Rows(0, 21)[0]; got != "faint grey both plain" {
		t.Fatalf("row text %q", got)
	}
	defaultColor := vt.Color{Kind: vt.ColorDefault}
	for col := uint16(0); col < 5; col++ {
		if !cell(col).Dim() || cell(col).FgColor() != defaultColor {
			t.Fatalf("column %d is faint with the default foreground", col)
		}
	}
	for col := uint16(6); col < 10; col++ {
		if cell(col).Dim() || cell(col).FgColor() != vt.Idx(8) {
			t.Fatalf("column %d is indexed 8", col)
		}
	}
	for col := uint16(11); col < 15; col++ {
		if !cell(col).Dim() || cell(col).FgColor() != vt.Rgb(10, 20, 30) || cell(col).BgColor() != vt.Idx(4) {
			t.Fatalf("column %d is faint rgb on indexed 4", col)
		}
	}
	for _, col := range []uint16{5, 10, 15, 16, 20} {
		if cell(col).Dim() || cell(col).FgColor() != defaultColor || cell(col).BgColor() != defaultColor {
			t.Fatalf("column %d keeps the default style", col)
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_TheBundledCatalogSpellsEveryPatternClassExplicitly(t *testing.T) {
	t.Parallel()
	if found := implicitPatternClasses(bundledFiles()); len(found) != 0 {
		t.Fatalf("implicit pattern classes: %q", found)
	}
}

// ruleWith is the first bundled rule whose suffix holds a pattern string, as its backend name
// and rule index.
func ruleWith(files any, suffix string) (string, int) {
	for _, member := range jobject(files).members {
		for index, r := range jarray(jget(member.value, "profile", "rules")).items {
			if value, ok := jpointer(r, suffix); ok {
				if _, isString := value.(string); isString {
					return member.key, index
				}
			}
		}
	}
	panic("a bundled rule carries " + suffix)
}

func withBundledPattern(suffix, pattern string) (any, string) {
	files := bundledFiles()
	backend, index := ruleWith(files, suffix)
	r := jget(files, backend, "profile", "rules", index)
	jpointerSet(r, suffix, pattern)
	name, _ := jstring(jget(files, backend, "backend"))
	id, _ := jstring(jget(r, "id"))
	return files, name + " rule " + id
}

func TestCodingAgentCatalog_BundledBackendFiles_TheExplicitClassGuardLineReadsBackendLocationAndConstruct(t *testing.T) {
	t.Parallel()
	value := syntheticDefinitions()
	jset(value, `^Operation:\s(?P<operation>.+)$`, "codex", "profile", "rules", 0, "matchers", 0, "pattern")
	if got := implicitPatternClasses(value); !slices.Equal(got, []string{"codex rule synthetic-permission: `\\s`"}) {
		t.Fatalf("guard lines %q", got)
	}
}

var bundledPatternFields = []string{
	"/matchers/0/pattern",
	"/region/start_pattern",
	"/region/end_pattern",
	"/frame/require_patterns/0",
	"/frame/reject_patterns/0",
}

func TestCodingAgentCatalog_BundledBackendFiles_TheExplicitClassGuardNamesTheProfileRuleAndConstructInEveryPatternField(t *testing.T) {
	t.Parallel()
	// The Rust list's `\u{a0}` is outside the pattern syntax, which leaves its acceptance
	// implementation-defined; `\P{L}` is the braced construct outside the syntax that compiles.
	constructs := []string{`\s`, `\S`, `\d`, `\D`, `\w`, `\W`, `\b`, `\B`, `\P{L}`}
	for _, field := range bundledPatternFields {
		for _, construct := range constructs {
			definitions, location := withBundledPattern(field, "x"+construct+"y")
			if got := implicitPatternClasses(definitions); !slices.Equal(got, []string{location + ": `" + construct + "`"}) {
				t.Fatalf("%s %s: guard lines %q", field, construct, got)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_TheExplicitClassGuardReportsEveryConstructOfAPatternInOrder(t *testing.T) {
	t.Parallel()
	definitions, location := withBundledPattern("/matchers/0/pattern", `^\s+\d[\w,]$`)
	want := []string{location + ": `\\s`", location + ": `\\d`", location + ": `\\w`"}
	if got := implicitPatternClasses(definitions); !slices.Equal(got, want) {
		t.Fatalf("guard lines %q, want %q", got, want)
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_TheExplicitClassGuardAdmitsEveryEscapeOfThePatternSyntax(t *testing.T) {
	t.Parallel()
	admitted := []struct{ name, pattern string }{
		{"ascii punctuation", `\.\?\(\)\[\]\{\}\$\^\*\+\|\\\-\#\&\~`},
		{"control escapes", `\t\n\r\f\v`},
		{"code points", `\x41\x{a0}\x{3000}`},
		{"general category", `\p{L}\p{Nd}`},
		{"text anchors", `\Ax\z`},
		{"explicit white space class", `[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]`},
		{"explicit word boundary", `settings(?:$|[^0-9A-Za-z_])`},
	}
	for _, field := range bundledPatternFields {
		for _, tc := range admitted {
			definitions, _ := withBundledPattern(field, tc.pattern)
			if got := implicitPatternClasses(definitions); len(got) != 0 {
				t.Fatalf("%s %s: guard lines %q", field, tc.name, got)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledBackendFiles_TheExplicitClassGuardIgnoresStringsTheDecoderDoesNotCompileAsPatterns(t *testing.T) {
	t.Parallel()
	files, _ := withBundledPattern("/matchers/0/pattern", "x")
	backend, index := ruleWith(files, "/matchers/0/pattern")
	jset(files, `rule-\s-\d`, backend, "profile", "rules", index, "id")
	if got := implicitPatternClasses(files); len(got) != 0 {
		t.Fatalf("guard lines %q", got)
	}
}

func TestCodingAgentCatalog_PermissionInspectionCatalogRetainsExactExecutablePlansAndProvenance(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	for _, tc := range []struct {
		decision permissionDecision
		choice   string
		rule     string
		byteVal  byte
	}{
		{permissionDecisionAllow, "allow-once", "inspection-allow-plan", 'y'},
		{permissionDecisionDeny, "deny-once", "inspection-deny-plan", 'n'},
	} {
		key := answerKeyDecision{decision: tc.decision}
		plan, ok := catalog.answerPlan(codingAgentCodex, "synthetic-permission", key)
		if !ok {
			t.Fatalf("%s: no plan", tc.rule)
		}
		capability := plan.capability
		if capability.backend != codingAgentCodex || capability.requestRuleID != "synthetic-permission" ||
			capability.answerRuleID != tc.rule || capability.key != answerKey(key) {
			t.Fatalf("%s: capability %+v", tc.rule, capability)
		}
		if plan.choiceID != tc.choice || plan.focusMatcherID != "form" || len(plan.stages) != 2 {
			t.Fatalf("%s: plan choice %q focus %q with %d stages", tc.rule, plan.choiceID, plan.focusMatcherID, len(plan.stages))
		}
		if !bytes.Equal(plan.stages[0].bytes, []byte{tc.byteVal}) || !bytes.Equal(plan.stages[1].bytes, []byte("\r")) {
			t.Fatalf("%s: stage bytes %q %q", tc.rule, plan.stages[0].bytes, plan.stages[1].bytes)
		}
		for _, stage := range plan.stages {
			if stage.gate != nil {
				t.Fatalf("%s: a gated stage", tc.rule)
			}
		}
	}
	if _, ok := catalog.answerPlan(codingAgentClaude, "synthetic-permission", answerKeyDecision{decision: permissionDecisionAllow}); ok {
		t.Fatal("claude has the codex plan")
	}
	for _, tc := range []struct {
		rule     string
		decision permissionDecision
	}{{"unknown", permissionDecisionAllow}, {"inspection-alternate-request", permissionDecisionDeny}} {
		if _, ok := catalog.answerPlan(codingAgentCodex, tc.rule, answerKeyDecision{decision: tc.decision}); ok {
			t.Fatalf("%s %v has a plan", tc.rule, tc.decision)
		}
	}
}

func TestCodingAgentCatalog_PermissionInspectionCatalogBoundsEachStageAndCompletePlan(t *testing.T) {
	t.Parallel()
	value := inspectionFixtureDefinition()
	stage := jclone(jget(value, "codex", "profile", "capabilities", "answers", 0, "stages", 0))
	jset(stage, strings.Repeat("79", 256), "bytes_hex")
	repeated := func(count int) *jsonArr {
		stages := ja()
		for range count {
			stages.push(jclone(stage))
		}
		return stages
	}
	jset(value, repeated(16), "codex", "profile", "capabilities", "answers", 0, "stages")
	catalog := mustLoadCatalog(t, value)
	plan, ok := catalog.answerPlan(codingAgentCodex, "synthetic-permission", answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		t.Fatal("the sixteen-stage plan")
	}
	total := 0
	for _, s := range plan.stages {
		total += len(s.bytes)
	}
	if total != 4096 {
		t.Fatalf("the plan carries %d bytes, want 4096", total)
	}
	jset(value, repeated(17), "codex", "profile", "capabilities", "answers", 0, "stages")
	assertLoadRejected(t, "seventeen stages", value)
	jset(value, ja(jo("bytes_hex", strings.Repeat("79", 257), "required_rule_id", "synthetic-permission", "required_choice_id", "allow-once")),
		"codex", "profile", "capabilities", "answers", 0, "stages")
	assertLoadRejected(t, "a 257-byte stage", value)
}

// assertRedacted is the Go form of assert_not_impl!(T: Debug, Serialize): every fmt verb prints
// the redaction marker and both JSON encoders refuse the value.
func assertRedacted(t testing.TB, name string, value any) {
	t.Helper()
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%x"} {
		if got := fmt.Sprintf(verb, value); got != redacted {
			t.Fatalf("%s: %s prints %q", name, verb, got)
		}
	}
	if encoded, err := stdjson.Marshal(value); err == nil {
		t.Fatalf("%s: encoding/json encodes it as %s", name, encoded)
	}
	if encoded, err := encodeJSON(value); err == nil {
		t.Fatalf("%s: encodeJSON encodes it as %s", name, encoded)
	}
}

func TestCodingAgentCatalog_PermissionInspectionExecutablePlansHaveNoDebugOrSerializedRepresentation(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	plan, ok := catalog.answerPlan(codingAgentCodex, "synthetic-permission", answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		t.Fatal("the allow plan")
	}
	assertRedacted(t, "answerPlan", plan)
	assertRedacted(t, "*answerPlan", &plan)
	assertRedacted(t, "answerStage", plan.stages[0])
	assertRedacted(t, "*answerStage", &plan.stages[0])
}

func invalidDefinition(t testing.TB, label string, change func(value any)) {
	t.Helper()
	mustLoadCatalog(t, syntheticDefinitions())
	value := syntheticDefinitions()
	change(value)
	if _, err := loadTestCatalog(value); err == nil {
		t.Fatalf("invalid definition accepted: %s", label)
	}
}

func withAnswer(decision string) any {
	value := syntheticDefinitions()
	choice, focus := "deny-once", "deny"
	if decision == "allow" {
		choice, focus = "allow-once", "allow"
	}
	jset(value, jsonf(`[{
		"decision":%s,"request_rule_id":%s,"answer_rule_id":%s,
		"choice_id":%s,"focus_matcher_id":%s,"fixture_ids":[%s],
		"stages":[{"bytes_hex":"78","required_rule_id":%s,"required_choice_id":%s}]
	}]`, decision, syntheticRule, "synthetic-"+decision+"-plan", choice, focus, syntheticFixture, syntheticRule, choice),
		"codex", "profile", "capabilities", "answers")
	return value
}

func TestCodingAgentCatalog_RuntimeProfileCapabilitiesAreIndependentAndBackendSelected(t *testing.T) {
	t.Parallel()
	catalog := syntheticCatalog(t)
	supported := catalog.capabilities(codingAgentCodex)
	if supported.detection != coverageTested || supported.composer != coverageUnknown || len(supported.answerDecisions) != 0 {
		t.Fatalf("codex capabilities %+v", supported)
	}
	for _, backend := range codingAgentAll() {
		if backend != codingAgentCodex && catalog.capabilities(backend).detection != coverageUnknown {
			t.Fatalf("%s detection is tested", backend.name())
		}
	}
}

func TestCodingAgentCatalog_RuntimeProfileAnswerDecisionsHaveSeparateRequestAndBackendCapabilities(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name            string
		decision, other permissionDecision
	}{{"allow", permissionDecisionAllow, permissionDecisionDeny}, {"deny", permissionDecisionDeny, permissionDecisionAllow}} {
		catalog := mustLoadCatalog(t, withAnswer(tc.name))
		if got := catalog.capabilities(codingAgentCodex).answerDecisions; !slices.Equal(got, []permissionDecision{tc.decision}) {
			t.Fatalf("%s: answer decisions %v", tc.name, got)
		}
		plan, ok := catalog.answerPlan(codingAgentCodex, syntheticRule, answerKeyDecision{decision: tc.decision})
		if !ok || plan.capability.answerRuleID != "synthetic-"+tc.name+"-plan" {
			t.Fatalf("%s: plan %v %q", tc.name, ok, plan.capability.answerRuleID)
		}
		for _, absent := range []struct {
			backend codingAgent
			request string
			key     answerKey
		}{
			{codingAgentCodex, syntheticRule, answerKeyDecision{decision: tc.other}},
			{codingAgentClaude, syntheticRule, answerKeyDecision{decision: tc.decision}},
			{codingAgentCodex, "another-form", answerKeyDecision{decision: tc.decision}},
		} {
			if _, ok := catalog.answerPlan(absent.backend, absent.request, absent.key); ok {
				t.Fatalf("%s: unexpected plan for %+v", tc.name, absent)
			}
		}
	}
}

func TestCodingAgentCatalog_RuntimeProfileRootSchemaAndUnknownFieldsFailInitialization(t *testing.T) {
	t.Parallel()
	for _, value := range []string{`null`, `[]`, `{"codex":[]}`} {
		assertLoadRejected(t, value, parseTestJSON(value))
	}
	invalidDefinition(t, "schema_version 2", func(value any) { jset(value, 2, "codex", "schema_version") })
	invalidDefinition(t, "a file under another name", func(value any) {
		file, _ := jobject(value).remove("codex")
		jset(value, file, "other")
	})
	for _, path := range []string{
		"", "/codex", "/codex/profile", "/codex/profile/rules/0", "/codex/profile/rules/0/region",
		"/codex/profile/rules/0/matchers/0", "/codex/profile/rules/0/extract", "/codex/profile/rules/0/extract/choices/0",
		"/codex/profile/rules/0/extract/operation", "/codex/profile/capabilities", "/codex/profile/capabilities/detection",
	} {
		invalidDefinition(t, "unexpected member at "+path, func(value any) {
			target, ok := jpointer(value, path)
			if !ok {
				panic("the pointer names an object: " + path)
			}
			jobject(target).set("unexpected", true)
		})
	}
	for _, field := range []string{"schema_version", "backend", "profile", "screens"} {
		invalidDefinition(t, "missing "+field, func(value any) { jobject(jget(value, "codex")).remove(field) })
	}
}

func TestCodingAgentCatalog_RuntimeProfileEnumsGeometryAndRealBooleanTypesAreValidated(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		path        string
		replacement any
	}{
		{"/codex/backend", "other"},
		{"/codex/backend", "claude"},
		{"/codex/screens", ja()},
		{"/codex/screens/synthetic-permission-1/native_version", " "},
		{"/codex/screens/synthetic-permission-1/native_version", ja(syntheticVersion)},
		{"/codex/screens/synthetic-permission-1/native_version", nil},
		{"/codex/profile/rules/0/priority", 1.5},
		{"/codex/profile/rules/0/observation", "approved"},
		{"/codex/profile/rules/0/verified_noninteraction", "false"},
		{"/codex/profile/rules/0/region/screen", "scrollback"},
		{"/codex/profile/rules/0/region/max_rows", 0},
		{"/codex/profile/rules/0/region/cursor", "nearby"},
		{"/codex/profile/rules/0/matchers/0/scope", "transcript"},
		{"/codex/profile/rules/0/extract/choices/0/scope", "forever"},
	} {
		invalidDefinition(t, tc.path+" = "+testJSONText(tc.replacement), func(value any) { jpointerSet(value, tc.path, tc.replacement) })
	}
}

func TestCodingAgentCatalog_RuntimeProfileDuplicateIdentityFails(t *testing.T) {
	t.Parallel()
	invalidDefinition(t, "two files with one fixture", func(value any) {
		claude := jclone(jget(value, "codex"))
		jset(claude, "claude", "backend")
		jset(value, claude, "claude")
	})
	for _, array := range []string{"/codex/profile/rules", "/codex/profile/rules/0/matchers", "/codex/profile/rules/0/extract/choices"} {
		invalidDefinition(t, "a duplicate in "+array, func(value any) {
			target, _ := jpointer(value, array)
			items := jarray(target)
			items.push(jclone(items.items[0]))
		})
	}
}

func TestCodingAgentCatalog_RuntimeProfileRegexAndCaptureRelationshipsFailAtInitialization(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct{ path, replacement string }{
		{"/codex/profile/rules/0/region/start_pattern", "["},
		{"/codex/profile/rules/0/region/end_pattern", "["},
		{"/codex/profile/rules/0/matchers/0/pattern", "["},
		{"/codex/profile/rules/0/extract/operation/matcher_id", "missing"},
		{"/codex/profile/rules/0/extract/operation/capture", "missing"},
	} {
		invalidDefinition(t, tc.path, func(value any) { jpointerSet(value, tc.path, tc.replacement) })
	}
	invalidDefinition(t, "a verified permission request", func(value any) {
		jset(value, true, "codex", "profile", "rules", 0, "verified_noninteraction")
	})
}

func operationOf(value any) any {
	return jget(value, "codex", "profile", "rules", 0, "extract", "operation")
}

func TestCodingAgentCatalog_RuntimeProfileOperationRowMarginMustBeANonemptySingleRowString(t *testing.T) {
	t.Parallel()
	margined := syntheticDefinitions()
	jset(operationOf(margined), " ", "row_margin")
	mustLoadCatalog(t, margined)
	for _, margin := range []any{1, "", "\n"} {
		invalidDefinition(t, "row_margin "+testJSONText(margin), func(value any) { jset(operationOf(value), margin, "row_margin") })
	}
	invalidDefinition(t, "an unknown operation key", func(value any) {
		jset(operationOf(value), " ", "row_margin")
		jset(operationOf(value), "rows", "reassembly")
	})
}

func TestCodingAgentCatalog_RuntimeProfileOperationMarginRowsSelectAllOrContinuationRowsOfAMargin(t *testing.T) {
	t.Parallel()
	for _, marginRows := range []string{"all", "continuation"} {
		value := syntheticDefinitions()
		jset(operationOf(value), " ", "row_margin")
		jset(operationOf(value), marginRows, "margin_rows")
		mustLoadCatalog(t, value)
	}
	invalidDefinition(t, "margin_rows first", func(value any) {
		jset(operationOf(value), " ", "row_margin")
		jset(operationOf(value), "first", "margin_rows")
	})
	invalidDefinition(t, "margin_rows without row_margin", func(value any) { jset(operationOf(value), "all", "margin_rows") })
}

func TestCodingAgentCatalog_RuntimeProfileOperationRowTrailerIsATrimmedSingleRowLiteral(t *testing.T) {
	t.Parallel()
	for _, withMargin := range []bool{false, true} {
		value := syntheticDefinitions()
		if withMargin {
			jset(operationOf(value), " ", "row_margin")
			jset(operationOf(value), "continuation", "margin_rows")
		}
		jset(operationOf(value), "█", "row_trailer")
		mustLoadCatalog(t, value)
	}
	for _, trailer := range []any{1, "", " █", "█\n█"} {
		invalidDefinition(t, "row_trailer "+testJSONText(trailer), func(value any) { jset(operationOf(value), trailer, "row_trailer") })
	}
}

func TestCodingAgentCatalog_RuntimeProfileCapabilityProvenanceReferencesAreChecked(t *testing.T) {
	t.Parallel()
	invalidDefinition(t, "a citation into another file", func(value any) {
		claude := jclone(jget(value, "codex"))
		jset(claude, "claude", "backend")
		jset(value, claude, "claude")
		jset(value, jo(), "codex", "screens")
	})
	for _, tc := range []struct {
		path        string
		replacement any
	}{
		{"/codex/profile/capabilities/detection/rule_ids", ja("missing-rule")},
		{"/codex/profile/capabilities/detection/fixture_ids", ja("missing-fixture")},
		{"/codex/profile/capabilities/detection/fixture_ids", ja()},
	} {
		invalidDefinition(t, tc.path+" = "+testJSONText(tc.replacement), func(value any) { jpointerSet(value, tc.path, tc.replacement) })
	}
	invalidDefinition(t, "a composer capability of detection rules", func(value any) {
		jset(value, jclone(jget(value, "codex", "profile", "capabilities", "detection")), "codex", "profile", "capabilities", "composer")
	})
}

func TestCodingAgentCatalog_RuntimeProfileAnswerScopeFocusStagesAndFixtureRelationshipsAreChecked(t *testing.T) {
	t.Parallel()
	valid := withAnswer("allow")
	mustLoadCatalog(t, valid)
	for _, tc := range []struct {
		path        string
		replacement any
	}{
		{"/codex/profile/capabilities/answers/0/decision", "approve-session"},
		{"/codex/profile/capabilities/answers/0/request_rule_id", "missing"},
		{"/codex/profile/capabilities/answers/0/focus_matcher_id", "missing"},
		{"/codex/profile/capabilities/answers/0/choice_id", "missing"},
		{"/codex/profile/capabilities/answers/0/fixture_ids", ja("missing")},
		{"/codex/profile/capabilities/answers/0/stages", ja()},
		{"/codex/profile/capabilities/answers/0/stages/0/bytes_hex", "7"},
		{"/codex/profile/capabilities/answers/0/stages/0/bytes_hex", "AF"},
		{"/codex/profile/capabilities/answers/0/stages/0/bytes_hex", ""},
		{"/codex/profile/capabilities/answers/0/stages/0/required_rule_id", "missing"},
		{"/codex/profile/capabilities/answers/0/stages/0/required_choice_id", "deny-once"},
		{"/codex/profile/rules/0/extract/choices/0/scope", "persistent"},
		{"/codex/profile/rules/0/observation", "trust"},
	} {
		value := jclone(valid)
		jpointerSet(value, tc.path, tc.replacement)
		assertLoadRejected(t, "invalid answer relationship "+tc.path, value)
	}
}

func TestCodingAgentCatalog_RuntimeProfileAnswerObjectsHaveExactFieldsAndUniquePlanIdentities(t *testing.T) {
	t.Parallel()
	valid := withAnswer("allow")
	mustLoadCatalog(t, valid)
	for _, path := range []string{"/codex/profile/capabilities/answers/0", "/codex/profile/capabilities/answers/0/stages/0"} {
		value := jclone(valid)
		target, _ := jpointer(value, path)
		jobject(target).set("unexpected", true)
		assertLoadRejected(t, "unexpected member at "+path, value)
	}
	duplicate := jclone(valid)
	jarray(jget(duplicate, "codex", "profile", "capabilities", "answers")).push(jclone(jget(valid, "codex", "profile", "capabilities", "answers", 0)))
	assertLoadRejected(t, "a duplicate answer", duplicate)
}

const (
	gatedStages      = "/codex/profile/capabilities/answers/1/stages"
	gatedAllowStages = "/codex/profile/capabilities/answers/0/stages"
	gatedReason      = "/codex/profile/rules/1"
)

func gatedRejected(t testing.TB, name string, change func(definition any)) {
	t.Helper()
	mustLoadCatalog(t, gatedFixtureDefinition())
	value := gatedFixtureDefinition()
	change(value)
	assertLoadRejected(t, name+" must fail catalog loading", value)
}

func gatedStage(byteHex, choice, gateKey, gateValue string) any {
	return jo("bytes_hex", byteHex, "required_rule_id", gatedRule, "required_choice_id", choice, gateKey, gateValue)
}

func pointed(value any, pointer string) any {
	target, ok := jpointer(value, pointer)
	if !ok {
		panic("the pointer names a value: " + pointer)
	}
	return target
}

func gatedPlan(t testing.TB, decision permissionDecision) answerPlan {
	t.Helper()
	plan, ok := mustLoadCatalog(t, gatedFixtureDefinition()).answerPlan(codingAgentCodex, gatedRule, answerKeyDecision{decision: decision})
	if !ok {
		t.Fatal("gated synthetic plan")
	}
	return plan
}

type stageGates struct {
	bytes, focus, screen string
}

func gatesOf(plan answerPlan) []stageGates {
	var gates []stageGates
	for _, stage := range plan.stages {
		focus, _ := stage.requiredFocusChoiceID()
		screen, _ := stage.requiredScreenRuleID()
		gates = append(gates, stageGates{bytes: string(stage.bytes), focus: focus, screen: screen})
	}
	return gates
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRetainsEachStageGateInDeclaredOrder(t *testing.T) {
	t.Parallel()
	if got := gatesOf(gatedPlan(t, permissionDecisionAllow)); !slices.Equal(got, []stageGates{{bytes: "\r", focus: "allow-once"}}) {
		t.Fatalf("allow gates %+v", got)
	}
	want := []stageGates{{bytes: "h", focus: "allow-once"}, {bytes: "\r", focus: "deny-once"}, {bytes: "\r", screen: gatedReasonRule}}
	if got := gatesOf(gatedPlan(t, permissionDecisionDeny)); !slices.Equal(got, want) {
		t.Fatalf("deny gates %+v, want %+v", got, want)
	}
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRejectsAFocusGateOnAnUnknownChoiceOrAMalformedGate(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name  string
		value any
	}{{"unknown choice", "always-allow"}, {"non-string gate", 1}, {"null gate", nil}, {"blank gate", " "}} {
		gatedRejected(t, tc.name, func(definition any) { jpointerSet(definition, gatedStages+"/0/required_focus_choice_id", tc.value) })
	}
	gatedRejected(t, "unknown continuation rule", func(definition any) {
		jpointerSet(definition, gatedStages+"/2/required_screen_rule_id", "missing-rule")
	})
	gatedRejected(t, "both gates on one stage", func(definition any) {
		jset(pointed(definition, gatedStages+"/1"), gatedReasonRule, "required_screen_rule_id")
	})
	gatedRejected(t, "unknown stage field", func(definition any) {
		jset(pointed(definition, gatedStages+"/0"), "allow-once", "required_focus")
	})
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRejectsAPlanThatGatesOnlySomeStages(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "ungated first stage", func(definition any) {
		jobject(pointed(definition, gatedStages+"/0")).remove("required_focus_choice_id")
	})
	gatedRejected(t, "ungated middle stage", func(definition any) {
		jobject(pointed(definition, gatedStages+"/1")).remove("required_focus_choice_id")
	})
	gatedRejected(t, "ungated final stage", func(definition any) {
		jpointerSet(definition, gatedStages, ja(
			gatedStage("68", "deny-once", "required_focus_choice_id", "allow-once"),
			jo("bytes_hex", "0d", "required_rule_id", gatedRule, "required_choice_id", "deny-once"),
		))
	})
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRejectsAFinalGateOnAnotherChoice(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "allow ends on the deny focus", func(definition any) {
		jpointerSet(definition, gatedAllowStages, ja(gatedStage("0d", "allow-once", "required_focus_choice_id", "deny-once")))
	})
	gatedRejected(t, "deny ends on the allow focus", func(definition any) {
		jpointerSet(definition, gatedStages, ja(gatedStage("0d", "deny-once", "required_focus_choice_id", "allow-once")))
	})
	endsOnDenyFocus := gatedFixtureDefinition()
	jpointerSet(endsOnDenyFocus, gatedStages, ja(
		gatedStage("68", "deny-once", "required_focus_choice_id", "allow-once"),
		gatedStage("0d", "deny-once", "required_focus_choice_id", "deny-once"),
	))
	mustLoadCatalog(t, endsOnDenyFocus)
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRejectsScreenGatesWithoutTheirContinuationSuccession(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "screen gate on the request rule itself", func(definition any) {
		jpointerSet(definition, gatedStages+"/2/required_screen_rule_id", gatedRule)
	})
	gatedRejected(t, "screen gate as the first stage", func(definition any) {
		jpointerSet(definition, gatedStages, ja(gatedStage("0d", "deny-once", "required_screen_rule_id", gatedReasonRule)))
	})
	gatedRejected(t, "screen gate after a stage on another focus", func(definition any) {
		jpointerSet(definition, gatedStages, ja(
			gatedStage("68", "deny-once", "required_focus_choice_id", "allow-once"),
			gatedStage("0d", "deny-once", "required_screen_rule_id", gatedReasonRule),
		))
	})
	gatedRejected(t, "continuation of an unrelated request rule", func(definition any) {
		other := jclone(jget(definition, "codex", "profile", "rules", 0))
		jset(other, "synthetic-other-request", "id")
		jset(other, "^Synthetic other request$", "region", "start_pattern")
		jset(other, "^Synthetic other request$", "matchers", 0, "pattern")
		jarray(jget(definition, "codex", "profile", "rules")).push(other)
		jset(definition, ja(gatedRule, "synthetic-other-request"), "codex", "profile", "capabilities", "detection", "rule_ids")
		jset(pointed(definition, gatedReason), "synthetic-other-request", "continues_rule_id")
	})
	gatedRejected(t, "final continuation of another choice", func(definition any) {
		jset(pointed(definition, gatedReason), "allow-once", "continues_choice_id")
		jpointerSet(definition, gatedStages, ja(
			gatedStage("0d", "deny-once", "required_focus_choice_id", "allow-once"),
			gatedStage("0d", "deny-once", "required_screen_rule_id", gatedReasonRule),
		))
	})
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogValidatesTheContinuationRuleDeclaration(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "continuation with an operation", func(definition any) {
		r := pointed(definition, gatedReason)
		jarray(jget(r, "matchers")).push(parseTestJSON(`{"id":"operation","scope":"line","pattern":"^Reason: (?P<operation>.+)$"}`))
		jset(r, jo("matcher_id", "operation", "capture", "operation"), "extract", "operation")
	})
	gatedRejected(t, "continuation with choices", func(definition any) {
		r := pointed(definition, gatedReason)
		jarray(jget(r, "matchers")).push(parseTestJSON(`{"id":"confirm","scope":"line","pattern":"^Confirm: (?P<label>enter)$"}`))
		jset(r, parseTestJSON(`[{"id":"confirm","label":{"matcher_id":"confirm","capture":"label"},"scope":"single_request"}]`), "extract", "choices")
	})
	gatedRejected(t, "continuation as an answer request rule", func(definition any) {
		jset(definition, gatedReasonRule, "codex", "profile", "capabilities", "answers", 0, "request_rule_id")
	})
	gatedRejected(t, "continuation verifying noninteraction", func(definition any) {
		jset(pointed(definition, gatedReason), true, "verified_noninteraction")
	})
	for _, key := range []string{"continues_rule_id", "continues_choice_id", "fixture_ids"} {
		gatedRejected(t, "continuation without "+key, func(definition any) { jobject(pointed(definition, gatedReason)).remove(key) })
	}
	for _, tc := range []struct {
		name, key string
		value     any
	}{
		{"unknown continued rule", "continues_rule_id", "missing-rule"},
		{"unknown continued choice", "continues_choice_id", "missing-choice"},
		{"no cited fixture", "fixture_ids", ja()},
		{"unknown cited fixture", "fixture_ids", ja("missing-fixture")},
	} {
		gatedRejected(t, tc.name, func(definition any) { jset(pointed(definition, gatedReason), tc.value, tc.key) })
	}
	gatedRejected(t, "continuation of a non-permission rule", func(definition any) {
		jset(definition, "question", "codex", "profile", "rules", 0, "observation")
		jset(definition, ja(), "codex", "profile", "capabilities", "answers")
	})
	for _, key := range []string{"continues_rule_id", "continues_choice_id"} {
		gatedRejected(t, key+" on a permission request rule", func(definition any) {
			jset(definition, jclone(jget(pointed(definition, gatedReason), key)), "codex", "profile", "rules", 0, key)
		})
	}
}

func TestCodingAgentCatalog_GatedCatalog_GatedCatalogRequiresABackgroundFocusMatcherOverLineScopedLabels(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "focus matcher without focus_style", func(definition any) {
		jobject(jget(definition, "codex", "profile", "rules", 0, "matchers", 2)).remove("focus_style")
	})
	gatedRejected(t, "focus_style bold", func(definition any) {
		jset(definition, "bold", "codex", "profile", "rules", 0, "matchers", 2, "focus_style")
	})
	gatedRejected(t, "region-scoped choice label", func(definition any) {
		jset(definition, "region", "codex", "profile", "rules", 0, "matchers", 3, "scope")
	})
	textual := syntheticDefinitions()
	jset(textual, "background", "codex", "profile", "rules", 0, "matchers", 1, "focus_style")
	mustLoadCatalog(t, textual)
}

func TestCodingAgentCatalog_GatedCatalog_EveryBackendRejectsAnEscapeByteInAnyAnswerStage(t *testing.T) {
	t.Parallel()
	gatedRejected(t, "gated stage 1b", func(definition any) { jpointerSet(definition, gatedStages+"/0/bytes_hex", "1b") })
	value := syntheticDefinitions()
	jset(value, jsonf(`[{
		"decision":"deny","request_rule_id":%s,"answer_rule_id":"synthetic-escape",
		"choice_id":"deny-once","focus_matcher_id":"deny","fixture_ids":[%s],
		"stages":[{"bytes_hex":"6e","required_rule_id":%s,"required_choice_id":"deny-once"}]
	}]`, syntheticRule, syntheticFixture, syntheticRule), "codex", "profile", "capabilities", "answers")
	mustLoadCatalog(t, value)
	jset(value, "0d1b", "codex", "profile", "capabilities", "answers", 0, "stages", 0, "bytes_hex")
	assertLoadRejected(t, "ungated stage 0d1b", value)
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerSubmitByteLimitsAreInclusiveBeforePlanConstruction(t *testing.T) {
	t.Parallel()
	for _, operation := range composerOperations {
		for _, length := range []int{1, 32} {
			submit := some(strings.Repeat("7a", length))
			good := composerDefinition(submit, none[string]())
			if operation == composerOperationExec {
				good = composerDefinition(none[string](), submit)
			}
			catalog := mustLoadCatalog(t, good)
			plan := mustComposerPlan(t, catalog, operation)
			encoded, err := plan.encode("literal")
			if err != nil {
				t.Fatalf("%s %d: %v", operationName(operation), length, err)
			}
			if want := expectedComposerBytes(operation, "literal", bytes.Repeat([]byte("z"), length)); !bytes.Equal(encoded, want) {
				t.Fatalf("%s %d: encoded %q, want %q", operationName(operation), length, encoded, want)
			}
			for _, invalid := range []int{0, 33} {
				bad := jclone(good)
				jset(bad, strings.Repeat("7a", invalid), submissionPath(operationName(operation), "submit_hex")...)
				assertDefinitionRejected(t, good, bad, operation, "decoded submit length")
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerPrefixShapeHexAndOperationIdentityAreValidated(t *testing.T) {
	t.Parallel()
	good := composerDefinition(some("7e"), some("6b6c"))
	for _, tc := range []struct {
		operation composerOperation
		invalid   any
	}{{composerOperationPrompt, "2120"}, {composerOperationExec, ""}, {composerOperationExec, 17}} {
		bad := jclone(good)
		jset(bad, tc.invalid, submissionPath(operationName(tc.operation), "prefix_hex")...)
		assertDefinitionRejected(t, good, bad, tc.operation, "operation prefix")
	}
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerSubmitHexRequiresNonemptyEvenLowercaseHexAndStringType(t *testing.T) {
	t.Parallel()
	good := composerDefinition(some("7e"), some("6b6c"))
	for _, invalid := range []any{"", "0", "ABCDEF", nil} {
		bad := jclone(good)
		jset(bad, invalid, submissionPath("prompt", "submit_hex")...)
		assertDefinitionRejected(t, good, bad, composerOperationPrompt, "submit hex")
	}
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerOptionalSubmissionAndOperationObjectsHaveExactCompleteFields(t *testing.T) {
	t.Parallel()
	good := composerDefinition(some("7e"), some("6b6c"))
	const submission = "/codex/profile/capabilities/composer/submission"
	const prompt = submission + "/prompt"
	composer := func(value any) any { return jget(value, "codex", "profile", "capabilities", "composer") }
	cases := []struct {
		name   string
		change func(bad any)
	}{
		{"missing composer field", func(bad any) { jobject(composer(bad)).remove("rule_ids") }},
		{"unknown composer field", func(bad any) { jset(composer(bad), true, "unknown") }},
		{"null submission", func(bad any) { jpointerSet(bad, submission, nil) }},
		{"array submission", func(bad any) { jpointerSet(bad, submission, ja()) }},
		{"missing operation", func(bad any) { jobject(pointed(bad, submission)).remove("exec") }},
		{"unknown submission field", func(bad any) { jset(pointed(bad, submission), true, "unknown") }},
		{"missing plan field", func(bad any) { jobject(pointed(bad, prompt)).remove("submit_hex") }},
		{"unknown plan field", func(bad any) { jset(pointed(bad, prompt), true, "unknown") }},
		{"plan object", func(bad any) { jpointerSet(bad, prompt, 17) }},
	}
	for _, tc := range cases {
		bad := jclone(good)
		tc.change(bad)
		assertDefinitionRejected(t, good, bad, composerOperationPrompt, tc.name)
	}
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerEachOperationRequiresItsOwnValidFixtureReferenceSet(t *testing.T) {
	t.Parallel()
	good := composerDefinition(some("7e"), some("6b6c"))
	for _, invalid := range []any{ja(), ja(" "), ja("unknown-fixture"), ja(promptFixture, promptFixture), nil} {
		bad := jclone(good)
		jset(bad, invalid, submissionPath("prompt", "fixture_ids")...)
		assertDefinitionRejected(t, good, bad, composerOperationPrompt, "operation fixture set "+testJSONText(invalid))
	}
	bad := jclone(good)
	claude := jclone(jget(syntheticDefinitions(), "codex"))
	jset(claude, "claude", "backend")
	jset(claude, jo("foreign-operation-fixture", fixtureAt(syntheticVersion)), "screens")
	jset(claude, ja("foreign-operation-fixture"), "profile", "capabilities", "detection", "fixture_ids")
	jset(bad, claude, "claude")
	jset(bad, ja("foreign-operation-fixture"), submissionPath("prompt", "fixture_ids")...)
	assertDefinitionRejected(t, good, bad, composerOperationPrompt, "foreign operation provenance")
	bad = jclone(good)
	jset(bad, ja(), submissionPath("exec", "fixture_ids")...)
	assertDefinitionRejected(t, good, bad, composerOperationExec, "exec fixture set")
}

func TestCodingAgentCatalog_ComposerSubmission_ComposerRuleReferencesKeepComponentIdentityAndDeclaredOrder(t *testing.T) {
	t.Parallel()
	good := composerDefinition(some("7e"), some("6b6c"))
	jarray(jget(good, "codex", "profile", "rules")).push(jclone(jget(syntheticDefinitions(), "codex", "profile", "rules", 0)))
	jset(good, jo("rule_ids", ja(syntheticRule), "fixture_ids", ja(syntheticFixture)), "codex", "profile", "capabilities", "detection")
	catalog := mustLoadCatalog(t, good)
	if got := catalog.detectionCoverage(codingAgentCodex); got != coverageTested {
		t.Fatalf("detection coverage %v", got)
	}
	for _, invalid := range []any{ja(), ja("unknown-rule"), ja(syntheticRule), ja(emptyComposerRule, emptyComposerRule)} {
		bad := jclone(good)
		jset(bad, invalid, "codex", "profile", "capabilities", "composer", "rule_ids")
		assertDefinitionRejected(t, good, bad, composerOperationPrompt, "composer rule component "+testJSONText(invalid))
	}
}

// singleProfileLabel is the label of backend's profile, named by its allow plan.
func singleProfileLabel(catalog *profileCatalog, backend codingAgent) opt[string] {
	plan, ok := catalog.answerPlan(backend, syntheticRule, answerKeyDecision{decision: permissionDecisionAllow})
	if !ok {
		return none[string]()
	}
	label, found := strings.CutSuffix(plan.capability.answerRuleID, "-allow-plan")
	if !found {
		panic("a single-profile answer plan is named after its label")
	}
	return some(label)
}

func singleProfileRejected(t testing.TB, value any, label string) string {
	t.Helper()
	_, err := loadTestCatalog(value)
	if err == nil {
		t.Fatalf("%s accepted", label)
	}
	if code := cafleetErrorOf(err).exitCode(); code != 2 {
		t.Fatalf("%s: exit code %d: %v", label, code, err)
	}
	if !strings.HasPrefix(err.Error(), "invalid native profile: ") {
		t.Fatalf("%s: %v", label, err)
	}
	return err.Error()
}

var malformedVersions = []string{"2.1", "2.1.302.1", "v2.1.302", "01.2.3", "1.2.3-", "1.2.3-01", " 1.2.3", "synthetic-0.0.1"}

func TestCodingAgentCatalog_SingleProfile_CatalogProfileFilesHoldTheirBackendProfileAndScreens(t *testing.T) {
	t.Parallel()
	value := singleProfiles([]singleProfile{{backend: codingAgentCodex, label: "only"}})
	sortedKeys := func(v any) []string {
		keys := jobject(v).keys()
		slices.Sort(keys)
		return keys
	}
	if got := sortedKeys(jget(value, "codex")); !slices.Equal(got, []string{"backend", "profile", "schema_version", "screens"}) {
		t.Fatalf("file keys %q", got)
	}
	if got := sortedKeys(jget(value, "codex", "profile")); !slices.Equal(got, []string{"capabilities", "rules"}) {
		t.Fatalf("profile keys %q", got)
	}
	mustLoadCatalog(t, value)
}

func TestCodingAgentCatalog_SingleProfile_ValidationRejectsMalformedFixtureCaptureVersions(t *testing.T) {
	t.Parallel()
	mustLoadCatalog(t, syntheticDefinitions())
	for _, malformed := range malformedVersions {
		value := syntheticDefinitions()
		jset(value, malformed, "codex", "screens", syntheticFixture, "native_version")
		singleProfileRejected(t, value, fmt.Sprintf("fixture native_version %q", malformed))
	}
}

var citationPaths = []string{
	"detection/fixture_ids",
	"composer/fixture_ids",
	"composer/submission/prompt/fixture_ids",
	"composer/submission/exec/fixture_ids",
	"answers/0/fixture_ids",
}

// citing is the single Codex profile with citation naming one extra screen, captured on
// captured, of backend's profile file.
func citing(citation string, backend codingAgent, captured string) any {
	value := singleProfiles([]singleProfile{{backend: codingAgentCodex, label: "only"}})
	if backend == codingAgentCodex {
		jset(value, fixtureAt(captured), "codex", "screens", "citation-probe")
	} else {
		file := jclone(jget(syntheticDefinitions(), "codex"))
		jset(file, backend.name(), "backend")
		jset(file, jo("citation-probe", fixtureAt(captured)), "screens")
		jset(file, ja("citation-probe"), "profile", "capabilities", "detection", "fixture_ids")
		jset(value, file, backend.name())
	}
	jpointerSet(value, "/codex/profile/capabilities/"+citation, ja("citation-probe"))
	return value
}

func TestCodingAgentCatalog_SingleProfile_ProvenanceAcceptsACitationOfItsBackendsFixtureAtAnyCaptureVersion(t *testing.T) {
	t.Parallel()
	for _, citation := range citationPaths {
		for _, captured := range []string{"0.0.1", "1.0.0-rc.1", "2.0.0+build.1", "99.0.0"} {
			if _, err := loadTestCatalog(citing(citation, codingAgentCodex, captured)); err != nil {
				t.Fatalf("%s %s: %v", citation, captured, err)
			}
		}
	}
}

func TestCodingAgentCatalog_SingleProfile_ProvenanceRejectsACitationOfAnotherBackendsFixture(t *testing.T) {
	t.Parallel()
	for _, citation := range citationPaths {
		for _, other := range codingAgentAll() {
			if other == codingAgentCodex {
				continue
			}
			label := citation + " citing a " + other.name() + " fixture"
			if message := singleProfileRejected(t, citing(citation, other, syntheticVersion), label); message != "invalid native profile: codex cites unknown fixture citation-probe" {
				t.Fatalf("%s: %q", label, message)
			}
		}
	}
}

func TestCodingAgentCatalog_SingleProfile_EachBackendResolvesOnlyItsOwnProfile(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, singleProfiles([]singleProfile{{codingAgentCodex, "codex-only"}, {codingAgentClaude, "claude-only"}}))
	for _, tc := range []struct {
		backend  codingAgent
		expected opt[string]
	}{
		{codingAgentCodex, some("codex-only")},
		{codingAgentClaude, some("claude-only")},
		{codingAgentOpencode, none[string]()},
		{codingAgentHermes, none[string]()},
		{codingAgentPi, none[string]()},
	} {
		if got := singleProfileLabel(catalog, tc.backend); got != tc.expected {
			t.Fatalf("%s: label %v, want %v", tc.backend.name(), got, tc.expected)
		}
		tested := coverageUnknown
		if _, set := tc.expected.get(); set {
			tested = coverageTested
		}
		if got := catalog.detectionCoverage(tc.backend); got != tested {
			t.Fatalf("%s: detection %v", tc.backend.name(), got)
		}
	}
}

func TestCodingAgentCatalog_SingleProfile_EveryRunVersionResolvesEveryCapabilityThroughTheBackendsProfile(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, singleProfiles([]singleProfile{{codingAgentCodex, "only"}}))
	rows := append([]string{"Live footer", "Refused: write outside.txt", "Refusal end"}, permissionLines("write disposable.txt")...)
	screen := syntheticTerminal(t, rows, 80, false).snapshot()
	for _, run := range []opt[string]{some("0.0.1"), some("1.4.2"), some("99.0.0-beta.1"), none[string]()} {
		observed, err := newDetector(catalog, codingAgentCodex, run).observe(&screen)
		if err != nil {
			t.Fatalf("%v: %v", run, err)
		}
		if observed.kind != observationKindPermissionRequest {
			t.Fatalf("%v: kind %s", run, observed.kind.debugString())
		}
		if observed.rawEvidence.nativeVersion != run {
			t.Fatalf("%v: native_version %v", run, observed.rawEvidence.nativeVersion)
		}
	}
	for _, operation := range composerOperations {
		if !catalog.verifiedComposer(codingAgentCodex, operation) {
			t.Fatalf("%s is not verified", operationName(operation))
		}
		mustComposerPlan(t, catalog, operation)
	}
	if got := singleProfileLabel(catalog, codingAgentCodex); got != some("only") {
		t.Fatalf("label %v", got)
	}
	if got := catalog.detectionCoverage(codingAgentCodex); got != coverageTested {
		t.Fatalf("detection %v", got)
	}
}

func TestCodingAgentCatalog_ComposerNullableOperationsAndAbsentComposerKeepExecutableCoverageUnknown(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name        string
		catalog     *profileCatalog
		observation coverage
	}{
		{"no plans", composerCatalog(t, none[string](), none[string]()), coverageTested},
		{"no composer", syntheticCatalog(t), coverageUnknown},
	} {
		assertCapabilities(t, tc.name, tc.catalog, codingAgentCodex, tc.observation, coverageUnknown, coverageUnknown)
		for _, operation := range composerOperations {
			if _, ok := tc.catalog.composerPlan(codingAgentCodex, operation); ok {
				t.Fatalf("%s: %s plan", tc.name, operationName(operation))
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerPromptPlanRetainsExactSelectedMetadataAndIndependentCoverage(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		operation, other composerOperation
		catalog          *profileCatalog
		prompt, exec     coverage
	}{
		{composerOperationPrompt, composerOperationExec, composerCatalog(t, some("7e"), none[string]()), coverageTested, coverageUnknown},
		{composerOperationExec, composerOperationPrompt, composerCatalog(t, none[string](), some("6b6c")), coverageUnknown, coverageTested},
	} {
		mustComposerPlan(t, tc.catalog, tc.operation)
		assertCapabilities(t, operationName(tc.operation), tc.catalog, codingAgentCodex, coverageTested, tc.prompt, tc.exec)
		if _, ok := tc.catalog.composerPlan(codingAgentCodex, tc.other); ok {
			t.Fatalf("%s: the other operation has a plan", operationName(tc.operation))
		}
		actual := tc.catalog.capabilities(codingAgentCodex)
		if actual.detection != coverageUnknown || len(actual.answerDecisions) != 0 {
			t.Fatalf("%s: capabilities %+v", operationName(tc.operation), actual)
		}
	}
}

func TestCodingAgentCatalog_ComposerOperationProvenanceAndExactSelectionDoNotAuthorizeOtherProfiles(t *testing.T) {
	t.Parallel()
	value := composerDefinition(some("7e"), some("6b6c"))
	jarray(jget(value, "codex", "profile", "rules")).push(jclone(jget(syntheticDefinitions(), "codex", "profile", "rules", 0)))
	jset(value, jo("rule_ids", ja(syntheticRule), "fixture_ids", ja(syntheticFixture)), "codex", "profile", "capabilities", "detection")
	answers := ja()
	for _, tc := range []struct{ decision, choice, focus string }{{"allow", "allow-once", "allow"}, {"deny", "deny-once", "deny"}} {
		answers.push(jsonf(`{"decision":%s,"request_rule_id":%s,"answer_rule_id":%s,
			"choice_id":%s,"focus_matcher_id":%s,"fixture_ids":[%s],
			"stages":[{"bytes_hex":"78","required_rule_id":%s,"required_choice_id":%s}]}`,
			tc.decision, syntheticRule, "synthetic-"+tc.decision+"-plan", tc.choice, tc.focus, syntheticFixture, syntheticRule, tc.choice))
	}
	jset(value, answers, "codex", "profile", "capabilities", "answers")
	catalog := mustLoadCatalog(t, value)
	for _, tc := range []struct {
		operation composerOperation
		want      string
	}{{composerOperationPrompt, "literal~"}, {composerOperationExec, "! literalkl"}} {
		plan := mustComposerPlan(t, catalog, tc.operation)
		encoded, err := plan.encode("literal")
		if err != nil || string(encoded) != tc.want {
			t.Fatalf("%s: encoded %q, %v", operationName(tc.operation), encoded, err)
		}
	}
	assertCapabilities(t, "codex", catalog, codingAgentCodex, coverageTested, coverageTested, coverageTested)
	actual := catalog.capabilities(codingAgentCodex)
	if actual.detection != coverageTested || !slices.Equal(actual.answerDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) {
		t.Fatalf("codex capabilities %+v", actual)
	}
	for _, backend := range codingAgentAll() {
		if backend == codingAgentCodex {
			continue
		}
		for _, operation := range composerOperations {
			if _, ok := catalog.composerPlan(backend, operation); ok {
				t.Fatalf("%s has a %s plan", backend.name(), operationName(operation))
			}
		}
		assertCapabilities(t, backend.name(), catalog, backend, coverageUnknown, coverageUnknown, coverageUnknown)
	}
}

func TestCodingAgentCatalog_ComposerPromptPreservesLiteralUtf8BetweenEmptyPrefixAndTestedSubmitBytes(t *testing.T) {
	t.Parallel()
	catalog := composerCatalog(t, some("7e"), none[string]())
	literal := "  literal 世界 '$x' `backticks` \\\"quotes\\\"  "
	plan := mustComposerPlan(t, catalog, composerOperationPrompt)
	encoded, err := plan.encode(literal)
	if err != nil {
		t.Fatal(err)
	}
	if want := expectedComposerBytes(composerOperationPrompt, literal, []byte("~")); !bytes.Equal(encoded, want) || !bytes.Equal(encoded[:len(literal)], []byte(literal)) {
		t.Fatalf("encoded %q", encoded)
	}
}

func TestCodingAgentCatalog_ComposerExecOwnsExactBangSpacePrefixWithoutChangingTheCallerLiteral(t *testing.T) {
	t.Parallel()
	catalog := composerCatalog(t, none[string](), some("6b6c"))
	literal := "printf '%s' 世界 '$x' `literal`"
	plan := mustComposerPlan(t, catalog, composerOperationExec)
	encoded, err := plan.encode(literal)
	if err != nil {
		t.Fatal(err)
	}
	if want := expectedComposerBytes(composerOperationExec, literal, []byte("kl")); !bytes.Equal(encoded, want) ||
		string(encoded[:2]) != "! " || string(encoded[2:2+len(literal)]) != literal {
		t.Fatalf("encoded %q", encoded)
	}
}

func TestCodingAgentCatalog_ComposerNativePayloadLimitCountsPrefixUtf8LiteralAndSubmitBytes(t *testing.T) {
	t.Parallel()
	for _, operation := range composerOperations {
		for _, submitLength := range []int{1, 32} {
			submit := some(strings.Repeat("6b", submitLength))
			value := composerDefinition(submit, none[string]())
			if operation == composerOperationExec {
				value = composerDefinition(none[string](), submit)
			}
			selected := mustComposerPlan(t, mustLoadCatalog(t, value), operation)
			literalSize := 65536 - len(operation.prefix()) - submitLength
			literal := strings.Repeat("界", literalSize/3) + strings.Repeat("x", literalSize%3)
			if len(literal) != literalSize {
				t.Fatalf("literal size %d", len(literal))
			}
			actual, err := selected.encode(literal)
			if err != nil {
				t.Fatalf("%s %d: %v", operationName(operation), submitLength, err)
			}
			if want := expectedComposerBytes(operation, literal, bytes.Repeat([]byte("k"), submitLength)); len(actual) != 65536 || !bytes.Equal(actual, want) {
				t.Fatalf("%s %d: %d bytes", operationName(operation), submitLength, len(actual))
			}
			if _, err := selected.encode(literal + "x"); err == nil {
				t.Fatalf("%s %d: 65537 native payload bytes are accepted", operationName(operation), submitLength)
			}
		}
	}
}

func TestCodingAgentCatalog_ComposerPlansWithholdDebugSerializationAndPrivateBytesFromErrors(t *testing.T) {
	t.Parallel()
	const submit = "E16_PRIVATE_SUBMIT"
	const literal = "E16_PRIVATE_LITERAL"
	submitHex := digestHex([]byte(submit))
	for _, operation := range composerOperations {
		good := composerDefinition(some(submitHex), none[string]())
		if operation == composerOperationExec {
			good = composerDefinition(none[string](), some(submitHex))
		}
		selected := mustComposerPlan(t, mustLoadCatalog(t, good), operation)
		assertRedacted(t, "composerPlan", selected)
		assertRedacted(t, "*composerPlan", &selected)
		encoded, err := selected.encode("positive")
		if err != nil || !bytes.Equal(encoded, expectedComposerBytes(operation, "positive", []byte(submit))) {
			t.Fatalf("%s: positive control %q, %v", operationName(operation), encoded, err)
		}
		bad := jclone(good)
		invalidPrefix := "00"
		if operation == composerOperationExec {
			invalidPrefix = ""
		}
		jset(bad, invalidPrefix, submissionPath(operationName(operation), "prefix_hex")...)
		definitionError := rejectDefinition(t, good, bad, operation, "private submit with invalid prefix")
		_, encodeError := selected.encode(literal + strings.Repeat("x", 65537))
		if encodeError == nil {
			t.Fatal("oversized private literal rejects")
		}
		for _, failure := range []error{definitionError, encodeError} {
			diagnostic := fmt.Sprintf("%#v\n%v\n%s", failure, failure, cafleetErrorOf(failure).message())
			for _, private := range []string{submit, literal, submitHex} {
				if strings.Contains(diagnostic, private) {
					t.Fatalf("plan errors withhold selected bytes and literal payload: %s", diagnostic)
				}
			}
		}
	}
}

const (
	boxEmptyRule = "box-empty"
	boxTextRule  = "box-text"
	tuiTextRule  = "tui-text"
)

func boxRule(id, observation, matcher string) any {
	priority := 50
	if observation == "composer_text" {
		priority = 40
	}
	return jsonf(`{
		"id":%s,"priority":%s,"observation":%s,
		"region":{"screen":"either","start_pattern":"^> ","end_pattern":"^-- footer$","max_rows":6,"cursor":"any"},
		"matchers":[{"id":"prompt","scope":"line","pattern":%s}],
		"extract":{"operation":null,"choices":[]},
		"verified_noninteraction":true
	}`, id, priority, observation, matcher)
}

// boxDefinition is a composer box: a `> ` first row, `  ` continuation rows and a `-- footer`
// row.
func boxDefinition(wrap string) any {
	value := composerDefinition(some("0d"), some("0d"))
	jset(value, ja(boxRule(boxEmptyRule, "composer_empty", "^> *$"), boxRule(boxTextRule, "composer_text", "^> [^ ]")), "codex", "profile", "rules")
	composer := jget(value, "codex", "profile", "capabilities", "composer")
	jset(composer, ja(boxEmptyRule, boxTextRule), "rule_ids")
	jset(composer, jsonf(`{
		"rule_ids":[%s, %s],
		"first_prefix":"^> ",
		"continuation_prefix":"^  ",
		"wrap":%s,
		"placeholders":[
			{"pattern":"\\[Pasted (?P<count>[0-9]+) chars\\]","unit":"scalars"},
			{"pattern":"\\[Pasted text #[0-9]+: (?P<count>[0-9]+) lines\\]","unit":"lines"}
		]
	}`, boxEmptyRule, boxTextRule, wrap), "text")
	return value
}

func boxScreen(t testing.TB, rows []string) screenSnapshot {
	t.Helper()
	return screenOf(t, []byte("\x1b[2J\x1b[18;1H"+strings.Join(rows, "\r\n")+"\r\n-- footer"))
}

func codexDetector(t testing.TB, value any) *detector {
	t.Helper()
	return newDetector(mustLoadCatalog(t, value), codingAgentCodex, none[string]())
}

func mustComposerText(t testing.TB, d *detector, screen screenSnapshot, label string) composerText {
	t.Helper()
	composer, found, err := d.composerText(&screen)
	if err != nil {
		t.Fatalf("%s: %v", label, err)
	}
	if !found {
		t.Fatalf("%s: the composer text is extractable", label)
	}
	return composer
}

func assertNoComposerText(t testing.TB, d *detector, screen screenSnapshot, label string) {
	t.Helper()
	if _, found, err := d.composerText(&screen); err != nil || found {
		t.Fatalf("%s: composer text found %v, %v", label, found, err)
	}
}

func TestCodingAgentCatalog_ComposerText_ExtractedComposerTextMatchesTheWrittenLiteralSegmentBySegment(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		wrap    string
		rows    []string
		chunks  []string
		matches bool
	}{
		{"one row", "char", []string{"> hello"}, []string{"hello"}, true},
		{"an appended key", "char", []string{"> hello!"}, []string{"hello"}, false},
		{"a missing tail", "char", []string{"> hell"}, []string{"hello"}, false},
		{"a longer literal", "char", []string{"> hello"}, []string{"hello!"}, false},
		{"the stripped prefix keeps an operation prefix", "char", []string{"> ! true"}, []string{"! true"}, true},
		{"trailing blanks are trimmed", "char", []string{"> abc   "}, []string{"abc"}, true},
		{"char wrap joins rows with nothing after the margin", "char", []string{"> hello wor", "  ld"}, []string{"hello world"}, true},
		{"char wrap adds no space at a row break", "char", []string{"> hello", "  world"}, []string{"hello world"}, false},
		{"a row without the continuation margin ends the text", "char", []string{"> abc", "zzz"}, []string{"abc"}, true},
		{"rows after the end are not composer text", "char", []string{"> abc", "zzz"}, []string{"abczzz"}, false},
		{"text spans written chunks", "char", []string{"> hello"}, []string{"hel", "lo"}, true},
		{"word wrap matches a row break as nothing", "word", []string{"> hello", "  world"}, []string{"helloworld"}, true},
		{"word wrap matches a row break as one space", "word", []string{"> hello", "  world"}, []string{"hello world"}, true},
		{"word wrap matches no two spaces at a row break", "word", []string{"> hello", "  world"}, []string{"hello  world"}, false},
		{"a scalars placeholder consumes its count of scalars", "char", []string{"> [Pasted 5 chars]a"}, []string{"あいうえおa"}, true},
		{"a scalars placeholder counts scalars, not bytes", "char", []string{"> [Pasted 3 chars]"}, []string{"abc"}, true},
		{"a scalars placeholder with the wrong count", "char", []string{"> [Pasted 5 chars]a"}, []string{"あいうえa"}, false},
		{"a lines placeholder consumes one whole chunk of its line count", "char", []string{"> [Pasted text #3: 2 lines]"}, []string{"one\ntwo"}, true},
		{"a lines placeholder with the wrong line count", "char", []string{"> [Pasted text #3: 2 lines]"}, []string{"one"}, false},
		{"an unconsumed chunk after a lines placeholder", "char", []string{"> [Pasted text #3: 2 lines]"}, []string{"one\ntwo", "x"}, false},
		{"text after a lines placeholder reads the next chunk", "char", []string{"> [Pasted text #1: 1 lines]x"}, []string{"paste", "x"}, true},
		{"a lines placeholder never consumes part of a chunk", "char", []string{"> [Pasted text #1: 1 lines]x"}, []string{"pastex"}, false},
		{"text before a lines placeholder", "char", []string{"> ab[Pasted text #1: 1 lines]"}, []string{"ab", "paste"}, true},
	}
	for _, tc := range cases {
		composer := mustComposerText(t, codexDetector(t, boxDefinition(tc.wrap)), boxScreen(t, tc.rows), tc.name)
		if got := composer.matches(tc.chunks); got != tc.matches {
			t.Fatalf("%s: %q against %q matches %v", tc.name, tc.rows, tc.chunks, got)
		}
	}
}

func TestCodingAgentCatalog_ComposerText_TheExtractedComposerCountsItsRowsInsideItsRuleRegion(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		rows  []string
		count int
	}{{[]string{"> hello"}, 1}, {[]string{"> hello wor", "  ld"}, 2}, {[]string{"> abc", "zzz"}, 1}} {
		composer := mustComposerText(t, codexDetector(t, boxDefinition("char")), boxScreen(t, tc.rows), strings.Join(tc.rows, "|"))
		if composer.rows != tc.count {
			t.Fatalf("%q: %d rows, want %d", tc.rows, composer.rows, tc.count)
		}
		if composer.region.top != 17 || int(composer.region.bottom) < 17+len(tc.rows) {
			t.Fatalf("%q: region %+v", tc.rows, composer.region)
		}
	}
}

func TestCodingAgentCatalog_ComposerText_ComposerTextIsUnextractableWithoutExactlyOneMatchingRuleAndPrefix(t *testing.T) {
	t.Parallel()
	duplicate := boxDefinition("char")
	jarray(jget(duplicate, "codex", "profile", "rules")).push(boxRule("box-text-copy", "composer_text", "^> [^ ]"))
	jset(duplicate, ja(boxEmptyRule, boxTextRule, "box-text-copy"), "codex", "profile", "capabilities", "composer", "text", "rule_ids")
	prefix := boxDefinition("char")
	jset(prefix, "^» ", "codex", "profile", "capabilities", "composer", "text", "first_prefix")
	for _, tc := range []struct {
		name   string
		value  any
		screen screenSnapshot
	}{
		{"no matching rule", boxDefinition("char"), screenOf(t, []byte("\x1b[2J\x1b[18;1Hplain output"))},
		{"several matching rules", duplicate, boxScreen(t, []string{"> abc"})},
		{"a first row outside first_prefix", prefix, boxScreen(t, []string{"> abc"})},
	} {
		assertNoComposerText(t, codexDetector(t, tc.value), tc.screen, tc.name)
	}
}

func TestCodingAgentCatalog_ComposerText_ADraftedComposerTextRuleObservesNoEmptyComposer(t *testing.T) {
	t.Parallel()
	screen := boxScreen(t, []string{"> draft"})
	observation, err := codexDetector(t, boxDefinition("char")).observe(&screen)
	if err != nil {
		t.Fatal(err)
	}
	if observation.ruleID != boxTextRule || observation.kind == observationKindComposerEmpty {
		t.Fatalf("observation %s %s", observation.ruleID, observation.kind.debugString())
	}
}

// modeBoxDefinition is the box whose first row starts with the `>` prompt or the `!` mode prompt,
// as a native composer draws its shell mode in place of the prompt marker.
func modeBoxDefinition(modePrefix opt[string]) any {
	value := boxDefinition("char")
	for index, matcher := range []string{"^(?:>|!) *$", "^(?:>|!) +[^ ]"} {
		r := jget(value, "codex", "profile", "rules", index)
		jset(r, "^(?:>|!) ", "region", "start_pattern")
		jset(r, matcher, "matchers", 0, "pattern")
	}
	text := jget(value, "codex", "profile", "capabilities", "composer", "text")
	jset(text, "^(?:>|!) ", "first_prefix")
	if prefix, set := modePrefix.get(); set {
		jset(text, prefix, "mode_prefix")
	}
	return value
}

func TestCodingAgentCatalog_ComposerText_ALiteralBeginningWithTheModePrefixMatchesWithOrWithoutItRendered(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name       string
		modePrefix opt[string]
		row        string
		literal    string
		matches    bool
	}{
		{"a rendered mode prefix still compares the rest", some("!"), "> ! true aab", "! true aaa", false},
		{"the mode prompt replaces the prompt marker", some("!"), "!  true aaa", "! true aaa", true},
		{"the prompt marker stays before the switched text", some("!"), ">  true aaa", "! true aaa", true},
		{"a literal without the mode prefix", some("!"), "> hello", "hello", true},
		{"the mode prefix rendered as composer text", some("!"), "> ! true aaa", "! true aaa", true},
		{"without mode_prefix the switch stays part of the literal", none[string](), "!  true aaa", "! true aaa", false},
		{"without mode_prefix a typed prefix is text", none[string](), "> ! true aaa", "! true aaa", true},
	} {
		composer := mustComposerText(t, codexDetector(t, modeBoxDefinition(tc.modePrefix)), boxScreen(t, []string{tc.row}), tc.name)
		if got := composer.matches([]string{tc.literal}); got != tc.matches {
			t.Fatalf("%s: matches %v", tc.name, got)
		}
	}
}

// regionDefinition is a composer whose composer_text rule starts at start and ends before the
// first later row matching end, with `│ ` margins.
func regionDefinition(start, end, firstPrefix string, framed bool) any {
	value := boxDefinition("char")
	r := jget(value, "codex", "profile", "rules", 1)
	jset(r, start, "region", "start_pattern")
	jset(r, end, "region", "end_pattern")
	jset(r, start, "matchers", 0, "pattern")
	text := jget(value, "codex", "profile", "capabilities", "composer", "text")
	jset(text, firstPrefix, "first_prefix")
	jset(text, "^│ ", "continuation_prefix")
	if framed {
		jset(text, true, "framed")
	}
	return value
}

func rowsScreen(t testing.TB, rows []string) screenSnapshot {
	t.Helper()
	return screenOf(t, []byte("\x1b[2J\x1b[15;1H"+strings.Join(rows, "\r\n")))
}

func TestCodingAgentCatalog_ComposerText_AComposerTextRegionEndsBeforeTheFirstLaterEndRow(t *testing.T) {
	t.Parallel()
	value := regionDefinition("^> ", "^│ --$", "^> ", false)
	composer := mustComposerText(t, codexDetector(t, value), rowsScreen(t, []string{"> abc", "│ def", "│ --", "│ ghi", "│ --"}), "the region before the first end row")
	for _, tc := range []struct {
		literal string
		matches bool
	}{{"abcdef", true}, {"abcdef--", false}, {"abcdefghi", false}, {"abcdef--ghi", false}} {
		if got := composer.matches([]string{tc.literal}); got != tc.matches {
			t.Fatalf("%s: matches %v", tc.literal, got)
		}
	}
}

func TestCodingAgentCatalog_ComposerText_AFramedComposerTextStartsOnTheRowAfterItsFrameRow(t *testing.T) {
	t.Parallel()
	screen := rowsScreen(t, []string{"┌────", "│ hello", "│ world", "└────"})
	composer := mustComposerText(t, codexDetector(t, regionDefinition("^┌─+$", "^└─+$", "^│ ", true)), screen, "a framed composer")
	if !composer.matches([]string{"helloworld"}) || composer.matches([]string{"┌────helloworld"}) {
		t.Fatal("a framed composer reads the rows below its frame row")
	}
	assertNoComposerText(t, codexDetector(t, regionDefinition("^┌─+$", "^└─+$", "^│ ", false)), screen,
		"without framed the frame row is the first row, outside first_prefix")
}

func cursorFreeScreen(screen screenSnapshot) screenSnapshot {
	screen.cursor = none[[2]uint16]()
	screen.alternateScreen = none[bool]()
	return screen
}

func TestCodingAgentCatalog_ComposerText_ComposerTextReadsTheBottomMostCandidateBelowATranscriptEcho(t *testing.T) {
	t.Parallel()
	screen := screenOf(t, []byte("\x1b[2J\x1b[5;1H> echoed earlier prompt\r\n-- footer\x1b[18;1H> live draft\r\n-- footer"))
	for _, tc := range []struct {
		name   string
		screen screenSnapshot
	}{{"as captured", screen}, {"cursor-free", cursorFreeScreen(screen)}} {
		composer := mustComposerText(t, codexDetector(t, boxDefinition("char")), tc.screen, tc.name)
		if !composer.matches([]string{"live draft"}) || composer.matches([]string{"echoed earlier prompt"}) {
			t.Fatalf("%s: the live composer is read", tc.name)
		}
	}
}

func TestCodingAgentCatalog_ComposerText_AnEmptyComposerBothRulesMatchClassifiesEmpty(t *testing.T) {
	t.Parallel()
	value := boxDefinition("char")
	for _, r := range jarray(jget(value, "codex", "profile", "rules")).items {
		jset(r, "^>", "region", "start_pattern")
	}
	jset(value, "^>", "codex", "profile", "rules", 1, "matchers", 0, "pattern")
	d := codexDetector(t, value)
	empty := boxScreen(t, []string{"> "})
	observation, err := d.observe(&empty)
	if err != nil || observation.kind != observationKindComposerEmpty || observation.ruleID != boxEmptyRule {
		t.Fatalf("empty composer %s %s %v", observation.ruleID, observation.kind.debugString(), err)
	}
	draft := boxScreen(t, []string{"> draft"})
	drafted, err := d.observe(&draft)
	if err != nil || drafted.ruleID != boxTextRule || drafted.kind == observationKindComposerEmpty {
		t.Fatalf("drafted composer %s %s %v", drafted.ruleID, drafted.kind.debugString(), err)
	}
}

func TestCodingAgentCatalog_ComposerText_AComposerTextRegionWithoutALaterEndRowClosesAtTheScreenBottom(t *testing.T) {
	t.Parallel()
	composer := mustComposerText(t, codexDetector(t, boxDefinition("char")), screenOf(t, []byte("\x1b[2J\x1b[23;1H> abc\r\n  def")), "a region closed by the screen bottom")
	if !composer.matches([]string{"abcdef"}) || composer.matches([]string{"abc"}) || composer.rows != 2 {
		t.Fatalf("every row to the bottom is text: rows %d", composer.rows)
	}
}

// tuiDefinition is the box profile plus a second composer_text rule for a one-column-indented
// box, whose own text_prefixes replace the profile's `^> ` and `^  `.
func tuiDefinition() any {
	value := boxDefinition("char")
	tui := boxRule(tuiTextRule, "composer_text", "^ > [^ ]")
	jset(tui, "^ > ", "region", "start_pattern")
	jset(tui, jo("first_prefix", "^ > ", "continuation_prefix", "^   "), "text_prefixes")
	jarray(jget(value, "codex", "profile", "rules")).push(tui)
	composer := jget(value, "codex", "profile", "capabilities", "composer")
	jarray(jget(composer, "rule_ids")).push(tuiTextRule)
	jarray(jget(composer, "text", "rule_ids")).push(tuiTextRule)
	return value
}

func TestCodingAgentCatalog_ComposerText_ARuleTextPrefixesReplaceTheProfilePrefixesForItsRegions(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name    string
		rows    []string
		literal string
		matches bool
	}{
		{"the indented box strips its own prefixes", []string{" > hello", "   world"}, "helloworld", true},
		{"the profile prefixes still read the classic box", []string{"> hello", "  world"}, "helloworld", true},
		{"a classic margin ends the indented box's text", []string{" > hello", "  world"}, "helloworld", false},
		{"the indented box keeps a space the classic margin would strip", []string{" > hello", "    world"}, "hello world", true},
	} {
		composer := mustComposerText(t, codexDetector(t, tuiDefinition()), boxScreen(t, tc.rows), tc.name)
		if got := composer.matches([]string{tc.literal}); got != tc.matches {
			t.Fatalf("%s: matches %v", tc.name, got)
		}
	}
}

func TestCodingAgentCatalog_ComposerText_ARuleTextWrapOverridesTheProfileWrapForItsRegions(t *testing.T) {
	t.Parallel()
	value := tuiDefinition()
	for _, r := range jarray(jget(value, "codex", "profile", "rules")).items {
		if jget(r, "id") == tuiTextRule {
			jset(r, "word", "text_wrap")
		}
	}
	for _, tc := range []struct {
		name    string
		rows    []string
		literal string
		matches bool
	}{
		{"the indented box joins a row break as one space", []string{" > hello", "   world"}, "hello world", true},
		{"the indented box joins a row break as nothing", []string{" > hello", "   world"}, "helloworld", true},
		{"the classic box keeps the profile's char wrap", []string{"> hello", "  world"}, "hello world", false},
		{"the classic box joins rows with nothing", []string{"> hello", "  world"}, "helloworld", true},
	} {
		composer := mustComposerText(t, codexDetector(t, value), boxScreen(t, tc.rows), tc.name)
		if got := composer.matches([]string{tc.literal}); got != tc.matches {
			t.Fatalf("%s: matches %v", tc.name, got)
		}
	}
}

func assertRejectedWith(t testing.TB, value any, expected, label string) {
	t.Helper()
	_, err := loadTestCatalog(value)
	if err == nil {
		t.Fatalf("%s: the profile file is rejected", label)
	}
	if !strings.Contains(err.Error(), expected) {
		t.Fatalf("%s: %v", label, err)
	}
}

func withPermissionRule(value any) any {
	jarray(jget(value, "codex", "profile", "rules")).push(jclone(jget(syntheticDefinitions(), "codex", "profile", "rules", 0)))
	return value
}

func composerTextOf(value any) any {
	return jget(value, "codex", "profile", "capabilities", "composer", "text")
}

func TestCodingAgentCatalog_ProfileRejections_AnExecutableComposerPlanRequiresComposerTextNamingComposerRules(t *testing.T) {
	t.Parallel()
	const expected = "composer plan for codex requires composer text extraction"
	mustLoadCatalog(t, composerDefinition(some("0d"), some("0d")))
	absent := composerDefinition(some("0d"), some("0d"))
	jobject(jget(absent, "codex", "profile", "capabilities", "composer")).remove("text")
	assertRejectedWith(t, absent, expected, "no composer.text")
	missing := composerDefinition(some("0d"), some("0d"))
	jset(composerTextOf(missing), ja("no-such-rule"), "rule_ids")
	assertRejectedWith(t, missing, expected, "a missing composer.text rule")
	interaction := withPermissionRule(composerDefinition(some("0d"), some("0d")))
	jset(composerTextOf(interaction), ja(syntheticRule), "rule_ids")
	assertRejectedWith(t, interaction, expected, "a non-composer composer.text rule")
}

func TestCodingAgentCatalog_ProfileRejections_AComposerPlaceholderCapturesCountInScalarsOrLines(t *testing.T) {
	t.Parallel()
	const expected = "composer placeholder of codex must capture count in scalars or lines"
	for _, tc := range []struct {
		name        string
		placeholder string
		accepted    bool
	}{
		{"scalars", `{"pattern":"\\[(?P<count>[0-9]+)\\]","unit":"scalars"}`, true},
		{"lines", `{"pattern":"\\[(?P<count>[0-9]+) lines\\]","unit":"lines"}`, true},
		{"no count group", `{"pattern":"\\[Pasted\\]","unit":"scalars"}`, false},
		{"another group name", `{"pattern":"\\[(?P<total>[0-9]+)\\]","unit":"scalars"}`, false},
		{"a bytes unit", `{"pattern":"\\[(?P<count>[0-9]+)\\]","unit":"bytes"}`, false},
	} {
		value := composerDefinition(some("0d"), some("0d"))
		jset(composerTextOf(value), ja(parseTestJSON(tc.placeholder)), "placeholders")
		if tc.accepted {
			mustLoadCatalog(t, value)
		} else {
			assertRejectedWith(t, value, expected, tc.name)
		}
	}
}

func TestCodingAgentCatalog_ProfileRejections_ComposerTextWrapIsCharOrWord(t *testing.T) {
	t.Parallel()
	for _, wrap := range []string{"char", "word"} {
		value := composerDefinition(some("0d"), some("0d"))
		jset(composerTextOf(value), wrap, "wrap")
		mustLoadCatalog(t, value)
	}
	value := composerDefinition(some("0d"), some("0d"))
	jset(composerTextOf(value), "line", "wrap")
	assertLoadRejected(t, "wrap line", value)
}

const (
	directorTmuxLimit  = "director-tmux-limit"
	directorHerdrLimit = "director-herdr-limit"
)

func limitScreen(multiplexer string, literal any) any {
	return jo("native_version", syntheticVersion, "composer_evidence", ja(
		jo("operation", "prompt", "multiplexer", multiplexer, "kind", "unsubmitted_literal", "expected_literal", jclone(literal)),
	))
}

// directorDefinition is a composer profile declaring Director delivery on both multiplexers,
// with one Director pre-submit screen per multiplexer whose literal is literal.
func directorDefinition(plainLimit, literal any) any {
	value := composerDefinition(some("0d"), some("0d"))
	jset(value, limitScreen("tmux", literal), "codex", "screens", directorTmuxLimit)
	jset(value, limitScreen("herdr", literal), "codex", "screens", directorHerdrLimit)
	jset(value, jo(
		"coverage", "tested",
		"multiplexers", ja("tmux", "herdr"),
		"rule_ids", ja(emptyComposerRule),
		"plain_limit", jclone(plainLimit),
		"fixture_ids", ja(directorTmuxLimit, directorHerdrLimit),
	), "codex", "profile", "capabilities", "director_delivery")
	return value
}

func scalars1024() any {
	return directorDefinition(jo("unit", "scalars", "max", 1024), parseTestJSON(`{"repeat":"a","count":1024,"prefix":""}`))
}

func directorDeliveryOfCodex(t testing.TB, catalog *profileCatalog) *directorDelivery {
	t.Helper()
	director, ok := catalog.directorDelivery(codingAgentCodex)
	if !ok {
		t.Fatal("the codex profile declares Director delivery")
	}
	return director
}

func directorDeliveryField(value any) any {
	return jget(value, "codex", "profile", "capabilities", "director_delivery")
}

func TestCodingAgentCatalog_DirectorDelivery_ADeclaredDirectorDeliveryNamesItsMultiplexersRulesAndLimit(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, scalars1024())
	director := directorDeliveryOfCodex(t, catalog)
	if !director.covers(muxBackendTmux) || !director.covers(muxBackendHerdr) {
		t.Fatal("both multiplexers are covered")
	}
	if !slices.Equal(director.ruleIDs, []string{emptyComposerRule}) {
		t.Fatalf("rule ids %q", director.ruleIDs)
	}
	if !director.plainLiteralAllowed(strings.Repeat("あ", 1024)) || director.plainLiteralAllowed(strings.Repeat("あ", 1025)) {
		t.Fatal("the plain limit is 1024 scalars")
	}
	for _, backend := range codingAgentAll() {
		if _, ok := catalog.directorDelivery(backend); ok && backend != codingAgentCodex {
			t.Fatalf("%s declares Director delivery", backend.name())
		}
	}
}

func TestCodingAgentCatalog_DirectorDelivery_TheDirectorPlainLimitCountsItsUnitAndNullAllowsTheInlineLimit(t *testing.T) {
	t.Parallel()
	byteLimit := directorDeliveryOfCodex(t, mustLoadCatalog(t, directorDefinition(jo("unit", "bytes", "max", 6), "ああ")))
	if !byteLimit.plainLiteralAllowed("ああ") || byteLimit.plainLiteralAllowed("ああa") {
		t.Fatal("the plain limit is 6 bytes")
	}
	inline := directorDeliveryOfCodex(t, mustLoadCatalog(t, directorDefinition(nil, parseTestJSON(`{"repeat":"あ","count":4000,"prefix":""}`))))
	if !inline.plainLiteralAllowed(strings.Repeat("あ", 4000)) {
		t.Fatal("a null limit allows the inline preview")
	}
}

func TestCodingAgentCatalog_DirectorDelivery_AProfileWithoutDirectorDeliveryOffersNone(t *testing.T) {
	t.Parallel()
	if _, ok := mustLoadCatalog(t, composerDefinition(some("0d"), some("0d"))).directorDelivery(codingAgentCodex); ok {
		t.Fatal("a profile without director_delivery offers one")
	}
}

func TestCodingAgentCatalog_DirectorDelivery_DirectorDeliveryOnTmuxOnlyCoversNoHerdrPane(t *testing.T) {
	t.Parallel()
	value := scalars1024()
	jset(directorDeliveryField(value), ja("tmux"), "multiplexers")
	jset(directorDeliveryField(value), ja(directorTmuxLimit), "fixture_ids")
	director := directorDeliveryOfCodex(t, mustLoadCatalog(t, value))
	if !director.covers(muxBackendTmux) || director.covers(muxBackendHerdr) {
		t.Fatal("tmux only")
	}
}

func TestCodingAgentCatalog_DirectorDelivery_DirectorDeliveryRulesMustBeComposerRules(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct{ name, rule string }{{"a missing rule", "no-such-rule"}, {"an interaction rule", syntheticRule}} {
		value := withPermissionRule(scalars1024())
		jset(directorDeliveryField(value), ja(tc.rule), "rule_ids")
		assertRejectedWith(t, value, "director delivery rule "+tc.rule+" of codex is not a composer rule", tc.name)
	}
}

func TestCodingAgentCatalog_DirectorDelivery_AListedMultiplexerNeedsAKeyMappingForThePromptSubmitBytes(t *testing.T) {
	t.Parallel()
	value := scalars1024()
	jset(value, "1c", submissionPath("prompt", "submit_hex")...)
	assertRejectedWith(t, value, "director delivery for codex on herdr has no submit key mapping", "an unmapped herdr submit byte")
	jset(directorDeliveryField(value), ja("tmux"), "multiplexers")
	jset(directorDeliveryField(value), ja(directorTmuxLimit), "fixture_ids")
	mustLoadCatalog(t, value)
}

func TestCodingAgentCatalog_DirectorDelivery_DirectorDeliveryRequiresAPlainLimitMeasuredOnEveryListedMultiplexer(t *testing.T) {
	t.Parallel()
	const expected = "director delivery for codex requires a measured plain_limit"
	absent := scalars1024()
	jobject(directorDeliveryField(absent)).remove("plain_limit")
	assertRejectedWith(t, absent, expected, "no plain_limit")
	for _, tc := range []struct {
		name   string
		screen any
	}{
		{"a herdr literal below the limit", limitScreen("herdr", parseTestJSON(`{"repeat":"a","count":1023,"prefix":""}`))},
		{"a herdr literal above the limit", limitScreen("herdr", parseTestJSON(`{"repeat":"a","count":1025,"prefix":""}`))},
		{"an exec record at the limit", jsonf(`{"native_version":%s,"composer_evidence":[
			{"operation":"exec","multiplexer":"herdr","kind":"unsubmitted_literal","expected_literal":{"repeat":"a","count":1024,"prefix":""}}
		]}`, syntheticVersion)},
		{"a tmux record cited for herdr", limitScreen("tmux", parseTestJSON(`{"repeat":"a","count":1024,"prefix":""}`))},
		{"a post-submit record", jsonf(`{"native_version":%s,"composer_evidence":[
			{"operation":"prompt","multiplexer":"herdr","kind":"post_submit"}
		]}`, syntheticVersion)},
	} {
		value := scalars1024()
		jset(value, tc.screen, "codex", "screens", directorHerdrLimit)
		assertRejectedWith(t, value, expected, tc.name)
	}
	uncited := scalars1024()
	jset(directorDeliveryField(uncited), ja(directorTmuxLimit), "fixture_ids")
	assertRejectedWith(t, uncited, expected, "an uncited herdr limit fixture")
}

func TestCodingAgentCatalog_DirectorDelivery_DirectorDeliveryCitesKnownFixturesAndDeclaresTestedCoverage(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		field string
		value any
	}{{"fixture_ids", ja()}, {"fixture_ids", ja("no-such-fixture")}, {"coverage", "unknown"}, {"multiplexers", ja("screen")}} {
		bad := scalars1024()
		jset(directorDeliveryField(bad), tc.value, tc.field)
		assertLoadRejected(t, tc.field+": "+testJSONText(tc.value), bad)
	}
}

const boxPreSubmitScreen = "box-pre-submit-tmux-80-1"

// paintedPreSubmit is a painted pre-submit screen: the composer box at rows 17 through 20
// holding `abc`, then a blank row and the footer.
func paintedPreSubmit(masked []int, literal string) any {
	lines := make([]string, 24)
	lines[2] = "transcript"
	lines[17] = "> abc"
	lines[19] = "-- footer"
	masks := ja()
	for _, row := range masked {
		lines[row] = ""
		masks.push(jo("row", row, "reason", "reviewed_privacy"))
	}
	return jo("native_version", syntheticVersion, "rows", 24, "cols", 80, "alternate_screen", false,
		"screen_seq", 0, "cursor_row", 17, "cursor_col", 5, "lines", lines, "masked_rows", masks,
		"composer_evidence", ja(jo("operation", "prompt", "multiplexer", "tmux", "kind", "unsubmitted_literal", "expected_literal", literal)))
}

func withPreSubmitScreen(screen any) any {
	value := boxDefinition("char")
	jset(value, screen, "codex", "screens", boxPreSubmitScreen)
	return value
}

func TestCodingAgentCatalog_PreSubmitFixtures_APaintedPreSubmitScreenRendersItsExpectedLiteral(t *testing.T) {
	t.Parallel()
	mustLoadCatalog(t, withPreSubmitScreen(paintedPreSubmit(nil, "abc")))
	mustLoadCatalog(t, withPreSubmitScreen(paintedPreSubmit([]int{2}, "abc")))
	assertRejectedWith(t, withPreSubmitScreen(paintedPreSubmit(nil, "abd")),
		"pre-submit fixture "+boxPreSubmitScreen+" does not render its expected_literal", "a mismatching literal")
}

func TestCodingAgentCatalog_PreSubmitFixtures_APreSubmitScreenMasksNoRowOfItsComposerRegion(t *testing.T) {
	t.Parallel()
	assertRejectedWith(t, withPreSubmitScreen(paintedPreSubmit([]int{18}, "abc")),
		"pre-submit fixture "+boxPreSubmitScreen+" masks a composer row", "a masked row inside the composer region")
}

func TestCodingAgentCatalog_PreSubmitFixtures_APreSubmitScreenWithoutComposerTextIsRejected(t *testing.T) {
	t.Parallel()
	screen := paintedPreSubmit(nil, "abc")
	jset(screen, "plain output", "lines", 17)
	assertRejectedWith(t, withPreSubmitScreen(screen),
		"pre-submit fixture "+boxPreSubmitScreen+" has no extractable composer text", "no composer on the screen")
}

func hasLines(screen any) bool {
	_, painted := jobject(screen).get("lines")
	return painted
}

func TestCodingAgentCatalog_PreSubmitFixtures_EveryBundledPreSubmitFixtureExtractsToItsExpectedLiteral(t *testing.T) {
	t.Parallel()
	files := bundledFiles()
	if err := validatePreSubmitFixtures(files, profileCatalogBundled()); err != nil {
		t.Fatal(err)
	}
	for _, member := range files.members {
		executable := jget(member.value, "profile", "capabilities", "composer", "submission") != nil
		painted := 0
		for _, screen := range jobject(jget(member.value, "screens")).members {
			records, err := preSubmitRecords(screen.value)
			if err != nil {
				t.Fatal(err)
			}
			if hasLines(screen.value) && len(records) > 0 {
				painted++
			}
		}
		if executable && painted == 0 {
			t.Fatalf("%s: an executable composer plan has painted pre-submit fixtures", member.key)
		}
	}
}

type citedPreSubmit struct {
	rows    int
	literal string
}

func preSubmitComposerRows(t testing.TB, backend codingAgent, screen any) int {
	t.Helper()
	snapshot := replayNativeFixture(mustCanonicalJSON(screen), nil, fixtureAlternate(screen), none[[2]uint16]())
	return mustComposerText(t, newDetector(profileCatalogBundled(), backend, none[string]()), snapshot, "a painted pre-submit fixture").rows
}

// citedPreSubmitFixtures is the cited painted pre-submit fixtures of operation as (rows, literal)
// pairs.
func citedPreSubmitFixtures(t testing.TB, backend codingAgent, file, cited any, operation composerOperation, multiplexer opt[muxBackend]) []citedPreSubmit {
	t.Helper()
	var out []citedPreSubmit
	for _, id := range jarray(cited).items {
		name, _ := jstring(id)
		screen := jget(file, "screens", name)
		if !hasLines(screen) {
			continue
		}
		records, err := preSubmitRecords(screen)
		if err != nil {
			t.Fatal(err)
		}
		for _, record := range records {
			mux, filtered := multiplexer.get()
			if record.operation != operation || (filtered && record.multiplexer != mux) {
				continue
			}
			out = append(out, citedPreSubmit{rows: preSubmitComposerRows(t, backend, screen), literal: record.literal})
		}
	}
	return out
}

func TestCodingAgentCatalog_PreSubmitFixtures_EveryOperationPlanCitesOneRowWrappedAndMaximumLengthPreSubmitFixtures(t *testing.T) {
	t.Parallel()
	for _, member := range bundledFiles().members {
		backend, _ := codingAgentCodingAgent(member.key)
		submission := jget(member.value, "profile", "capabilities", "composer", "submission")
		if submission == nil {
			continue
		}
		for _, operation := range composerOperations {
			label := member.key + " " + operationName(operation)
			plan, ok := profileCatalogBundled().composerPlan(backend, operation)
			if !ok {
				t.Fatalf("%s: an executable plan", label)
			}
			fixtures := citedPreSubmitFixtures(t, backend, member.value, jget(submission, operationName(operation), "fixture_ids"), operation, none[muxBackend]())
			oneRow := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool { return f.rows == 1 })
			wrapped := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool { return f.rows > 1 })
			atLimit := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool {
				literal := strings.TrimPrefix(f.literal, string(operation.prefix()))
				if jget(submission, operationName(operation), "plain_literal_limit") == nil {
					return utf8.RuneCountInString(literal) == 4000
				}
				return plan.plainLiteralAllowed(literal) && !plan.plainLiteralAllowed(literal+"a")
			})
			if !oneRow || !wrapped || !atLimit {
				t.Fatalf("%s: one-row %v, wrapped %v, maximum-length %v pre-submit fixtures", label, oneRow, wrapped, atLimit)
			}
		}
	}
}

func TestCodingAgentCatalog_PreSubmitFixtures_EveryDirectorDeliveryCitesOneRowWrappedAndLimitPromptFixturesPerMultiplexer(t *testing.T) {
	t.Parallel()
	for _, member := range bundledFiles().members {
		backend, _ := codingAgentCodingAgent(member.key)
		director := jget(member.value, "profile", "capabilities", "director_delivery")
		if director == nil {
			continue
		}
		delivery, ok := profileCatalogBundled().directorDelivery(backend)
		if !ok {
			t.Fatalf("%s: the declared Director delivery loads", member.key)
		}
		for _, raw := range jarray(jget(director, "multiplexers")).items {
			name, _ := jstring(raw)
			mux, failure := muxBackendParse(name)
			if failure != nil {
				t.Fatalf("%s: %v", name, failure)
			}
			if !delivery.covers(mux) {
				t.Fatalf("%s does not cover %s", member.key, name)
			}
			label := member.key + " Director on " + name
			fixtures := citedPreSubmitFixtures(t, backend, member.value, jget(director, "fixture_ids"), composerOperationPrompt, some(mux))
			oneRow := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool { return f.rows == 1 })
			wrapped := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool { return f.rows > 1 })
			atLimit := slices.ContainsFunc(fixtures, func(f citedPreSubmit) bool {
				return delivery.plainLiteralAllowed(f.literal) && (jget(director, "plain_limit") == nil || !delivery.plainLiteralAllowed(f.literal+"a"))
			})
			if !oneRow || !wrapped || !atLimit {
				t.Fatalf("%s: one-row %v, wrapped %v, at-limit %v prompt fixtures", label, oneRow, wrapped, atLimit)
			}
		}
	}
}

// bundledRules is each rule of the bundled backend profile file that keep selects.
func bundledRules(backend codingAgent, keep func(r any) bool) []any {
	var rules []any
	for _, r := range jarray(jget(parsedBundledFiles(), backend.name(), "profile", "rules")).items {
		if keep(r) {
			rules = append(rules, r)
		}
	}
	return rules
}

func citedFixtures(t testing.TB, r any) []string {
	t.Helper()
	cited, ok := jget(r, "fixture_ids").(*jsonArr)
	if !ok || len(cited.items) == 0 {
		t.Fatalf("%s cites its native fixtures", testJSONText(jget(r, "id")))
	}
	var ids []string
	for _, id := range cited.items {
		text, _ := jstring(id)
		ids = append(ids, text)
	}
	return ids
}

func ruleObservation(t testing.TB, r any) observationKind {
	t.Helper()
	text, _ := jstring(jget(r, "observation"))
	kind, err := observationKindParse(text)
	if err != nil {
		t.Fatal(err)
	}
	return kind
}

// capturedAndCursorFree observes a bundled fixture as captured and without cursor or screen mode.
func capturedAndCursorFree(t testing.TB, observe func(*screenSnapshot) (screenObservation, error), id string, change func(*[]string)) [2]screenObservation {
	t.Helper()
	captured := replayAsCaptured(id, change, none[[2]uint16]())
	free := cursorFreeScreen(captured)
	var out [2]screenObservation
	for index, screen := range []*screenSnapshot{&captured, &free} {
		observed, err := observe(screen)
		if err != nil {
			t.Fatalf("%s: %v", id, err)
		}
		out[index] = observed
	}
	return out
}

func composerEmptyRule(r any) bool { return jget(r, "observation") == "composer_empty" }

func TestCodingAgentCatalog_DirectorScreens_OpencodeComposerRulesAcceptAnyAgentNameInTheStatusLine(t *testing.T) {
	t.Parallel()
	d := newDetector(profileCatalogBundled(), codingAgentOpencode, none[string]())
	edited := 0
	for _, member := range jobject(jget(parsedBundledFiles(), "opencode", "screens")).members {
		id := member.key
		if !hasLines(member.value) || capturedAndCursorFree(t, d.observe, id, nil)[0].kind != observationKindComposerEmpty {
			continue
		}
		row := slices.IndexFunc(fixtureLines(id), func(line string) bool { return strings.Contains(line, "cafleet-ask") })
		if row < 0 {
			continue
		}
		edited++
		for _, agent := range []string{"Build", "Plan", "cafleet", "cafleet-ask", "review bot"} {
			for _, observed := range capturedAndCursorFree(t, d.observe, id, func(lines *[]string) {
				(*lines)[row] = strings.Replace((*lines)[row], "cafleet-ask", agent, 1)
			}) {
				if observed.kind != observationKindComposerEmpty {
					t.Fatalf("%s with agent %q: %s", id, agent, observed.kind.debugString())
				}
			}
		}
	}
	if edited == 0 {
		t.Fatal("an opencode empty-composer fixture shows the default agent name")
	}
}

func TestCodingAgentCatalog_DirectorScreens_OpencodeDirectorFixturesObserveAnEmptyComposer(t *testing.T) {
	t.Parallel()
	d := newDetector(profileCatalogBundled(), codingAgentOpencode, none[string]())
	for _, r := range bundledRules(codingAgentOpencode, composerEmptyRule) {
		for _, id := range citedFixtures(t, r) {
			for _, observed := range capturedAndCursorFree(t, d.observe, id, nil) {
				if observed.kind != observationKindComposerEmpty {
					t.Fatalf("%s: %s", id, observed.kind.debugString())
				}
			}
		}
	}
}

// hermesTUIRule selects a hermes rule that reads the TUI's alternate screen.
func hermesTUIRule(r any) bool { return jget(r, "region", "screen") == "alternate" }

// hermesDirectorRules is the hermes TUI composer rules, which with every interaction rule
// classify a hermes Director screen.
func hermesDirectorRules() []string {
	var ids []string
	for _, r := range bundledRules(codingAgentHermes, func(r any) bool {
		observation := jget(r, "observation")
		return hermesTUIRule(r) && (observation == "composer_empty" || observation == "composer_text")
	}) {
		id, _ := jstring(jget(r, "id"))
		ids = append(ids, id)
	}
	return ids
}

func TestCodingAgentCatalog_DirectorScreens_HermesTuiIdleComposerObservesEmptyAndADraftStaysNotEmpty(t *testing.T) {
	t.Parallel()
	d := newDetector(profileCatalogBundled(), codingAgentHermes, none[string]())
	rules := hermesDirectorRules()
	observe := func(screen *screenSnapshot) (screenObservation, error) { return d.observeDirector(screen, rules) }
	idle := bundledRules(codingAgentHermes, func(r any) bool { return hermesTUIRule(r) && composerEmptyRule(r) })
	if len(idle) == 0 {
		t.Fatal("hermes declares a TUI empty-composer rule")
	}
	for _, r := range idle {
		ruleID, _ := jstring(jget(r, "id"))
		for _, id := range citedFixtures(t, r) {
			for _, observed := range capturedAndCursorFree(t, observe, id, nil) {
				if observed.kind != observationKindComposerEmpty || observed.ruleID != ruleID {
					t.Fatalf("%s: %s %s, want the citing rule %s", id, observed.kind.debugString(), observed.ruleID, ruleID)
				}
			}
			drafted := capturedAndCursorFree(t, observe, id, func(lines *[]string) {
				row := -1
				for index, line := range *lines {
					if strings.HasPrefix(line, " ❯") {
						row = index
					}
				}
				if row < 0 {
					panic("a Hermes TUI composer row")
				}
				(*lines)[row] = " ❯ draft text"
			})
			for _, observed := range drafted {
				if observed.kind == observationKindComposerEmpty {
					t.Fatalf("%s: a drafted TUI composer observes empty", id)
				}
			}
		}
	}
}

// wordWrapped is text split at spaces into rows of at most width characters, the space at each
// break dropped, as the hermes TUI wraps its composer.
func wordWrapped(text string, width int) []string {
	var rows []string
	for _, word := range strings.Split(text, " ") {
		if last := len(rows) - 1; last >= 0 && utf8.RuneCountInString(rows[last])+1+utf8.RuneCountInString(word) <= width {
			rows[last] += " " + word
			continue
		}
		rows = append(rows, word)
	}
	return rows
}

// hermesTUIScreen is a hermes TUI Director screen: a prior user turn and the status line above a
// composer holding composer rows, the cursor after the last one.
func hermesTUIScreen(t testing.TB, composer []string) screenSnapshot {
	t.Helper()
	lines := []string{
		" ❯ an earlier prompt from the operator",
		"",
		"   The earlier answer.",
		"",
		" ─ ready · gpt-6-luna · 12% context",
	}
	for index, text := range composer {
		if index == 0 {
			lines = append(lines, " ❯ "+text)
		} else {
			lines = append(lines, "   "+text)
		}
	}
	bytes := []byte("\x1b[?1049h\x1b[2J")
	for offset, line := range lines {
		bytes = fmt.Appendf(bytes, "\x1b[%d;1H%s", 30+offset, line)
	}
	terminal := mustTerminalState(t, terminalSize{rows: 39, cols: 120})
	mustFeed(t, terminal, bytes)
	return terminal.snapshot()
}

func TestCodingAgentCatalog_DirectorScreens_HermesTuiComposerTextReadsAWordWrappedPreviewBelowTheTranscriptAndStatusLine(t *testing.T) {
	t.Parallel()
	preview := canonicalPreviewLine(messageRecordFixture(12, 3, 1, "the results of the review are ready for the director to read"))
	rows := wordWrapped(preview, 117)
	if len(rows) < 2 {
		t.Fatalf("the preview wraps: %q", rows)
	}
	d := newDetector(profileCatalogBundled(), codingAgentHermes, none[string]())
	captured := hermesTUIScreen(t, rows)
	cases := []struct {
		name   string
		screen screenSnapshot
	}{
		{"as captured", captured},
		{"cursor-free", cursorFreeScreen(captured)},
	}
	for _, c := range cases {
		composer, found, err := d.composerText(&c.screen)
		if err != nil || !found {
			t.Fatalf("%s: the TUI composer is extractable: %v", c.name, err)
		}
		if !composer.matches([]string{preview}) {
			t.Fatalf("%s: the composer holds the preview %q", c.name, rows)
		}
		if composer.matches([]string{"an earlier prompt from the operator"}) {
			t.Fatalf("%s: the transcript turn is not the composer", c.name)
		}
		observed, err := d.observeDirector(&c.screen, hermesDirectorRules())
		if err != nil {
			t.Fatal(err)
		}
		if observed.kind == observationKindComposerEmpty {
			t.Fatalf("%s: a typed preview is not an empty composer", c.name)
		}
	}
}

func TestCodingAgentCatalog_DirectorScreens_HermesTuiApprovalFormIsDetectedWithNoAnswerPlan(t *testing.T) {
	t.Parallel()
	catalog := profileCatalogBundled()
	d := newDetector(catalog, codingAgentHermes, none[string]())
	rules := hermesDirectorRules()
	observe := func(screen *screenSnapshot) (screenObservation, error) { return d.observeDirector(screen, rules) }
	forms := bundledRules(codingAgentHermes, func(r any) bool {
		kind, err := observationKindParse(fmt.Sprint(jget(r, "observation")))
		if err != nil {
			panic(err)
		}
		_, interaction := kind.interaction()
		return hermesTUIRule(r) && interaction
	})
	if len(forms) == 0 {
		t.Fatal("hermes declares a TUI interaction rule")
	}
	for _, r := range forms {
		ruleID, _ := jstring(jget(r, "id"))
		expected := ruleObservation(t, r)
		for _, id := range citedFixtures(t, r) {
			for _, observed := range capturedAndCursorFree(t, observe, id, nil) {
				if observed.kind != expected || observed.ruleID != ruleID {
					t.Fatalf("%s: %s %s, want %s %s", id, observed.kind.debugString(), observed.ruleID, expected.debugString(), ruleID)
				}
			}
		}
		declared := 0
		for _, a := range jarray(jget(parsedBundledFiles(), "hermes", "profile", "capabilities", "answers")).items {
			if jget(a, "request_rule_id") == ruleID {
				declared++
			}
		}
		if got := len(catalog.answerPlans(codingAgentHermes, ruleID)); got != declared {
			t.Fatalf("%s: %d answer plans for %d declared answers", ruleID, got, declared)
		}
	}
}

// cursorScreen is the id of a bundled codex screen with a recorded cursor whose every composer
// record names multiplexer.
func cursorScreen(files any, multiplexer string) string {
	for _, member := range jobject(jget(files, "codex", "screens")).members {
		screen := member.value
		records, ok := jget(screen, "composer_evidence").(*jsonArr)
		if jget(screen, "cursor_row") == nil || !hasLines(screen) || !ok {
			continue
		}
		if !slices.ContainsFunc(records.items, func(record any) bool { return jget(record, "multiplexer") != multiplexer }) {
			return member.key
		}
	}
	panic("a bundled codex " + multiplexer + " screen with a cursor")
}

func guardedBundledFiles(files any) opt[string] {
	_, err := profileCatalogFromBundledFiles(files)
	if err != nil {
		return some(err.Error())
	}
	return none[string]()
}

func profileRuleIDs(p *profile) []string {
	ids := make([]string, len(p.rules))
	for index := range p.rules {
		ids[index] = p.rules[index].id
	}
	return ids
}

// TestCodingAgentCatalog_ProductionLoaderStopsReadingAProfileFileAfterItsProfile is F21: the
// production loader reads each profile file only through its profile member, so its bytes after
// that member, however malformed, change nothing, while the bundled-catalog guard still validates
// the whole file.
func TestCodingAgentCatalog_ProductionLoaderStopsReadingAProfileFileAfterItsProfile(t *testing.T) {
	t.Parallel()
	for _, backend := range codingAgentAll() {
		text, ok := bundledProfileText(backend)
		if !ok {
			continue
		}
		full, err := decodeProfileFileText(backend, text)
		if err != nil {
			t.Fatalf("%s: the bundled file decodes: %v", backend.name(), err)
		}
		file := jget(parsedBundledFiles(), backend.name())
		head := jo("schema_version", jget(file, "schema_version"), "backend", jget(file, "backend"), "profile", jget(file, "profile"))
		prefix := strings.TrimSuffix(testJSONText(head), "}")
		for _, tail := range []string{`,"screens":{"unterminated`, `,"screens":1}`, `}trailing bytes`} {
			truncated, err := decodeProfileFileText(backend, []byte(prefix+tail))
			if err != nil {
				t.Fatalf("%s %q: the loader stops after the profile member: %v", backend.name(), tail, err)
			}
			if !slices.Equal(profileRuleIDs(&truncated), profileRuleIDs(&full)) || len(truncated.answers) != len(full.answers) {
				t.Fatalf("%s %q: the profile decodes as from the whole file", backend.name(), tail)
			}
		}
		files := bundledFiles()
		jset(files, 1, backend.name(), "screens")
		if violation, found := guardedBundledFiles(files).get(); !found {
			t.Fatalf("%s: the guard validates the screens after the profile", backend.name())
		} else if violation == "" {
			t.Fatalf("%s: the guard names its violation", backend.name())
		}
	}
}

var cursorFreeFields = []string{"alternate_screen", "cursor_row", "cursor_col"}

// nulledScreen is files with the fields of screen id set to null and its screen-level
// multiplexer set to multiplexer when given.
func nulledScreen(files any, id string, fields []string, multiplexer opt[string]) any {
	out := jclone(files)
	screen := jget(out, "codex", "screens", id)
	for _, field := range fields {
		jset(screen, nil, field)
	}
	if name, set := multiplexer.get(); set {
		jset(screen, name, "multiplexer")
	}
	return out
}

func TestCodingAgentCatalog_CursorFreeFixtures_AHerdrCaptureMayRecordANullCursorAndScreenMode(t *testing.T) {
	t.Parallel()
	files := bundledFiles()
	if failure := guardedBundledFiles(files); failure != none[string]() {
		t.Fatalf("the bundled files: %v", failure)
	}
	for _, multiplexer := range multiplexerNames {
		id := cursorScreen(files, multiplexer)
		if failure := guardedBundledFiles(nulledScreen(files, id, cursorFreeFields, some("herdr"))); failure != none[string]() {
			t.Fatalf("%s: %v", id, failure)
		}
	}
}

func TestCodingAgentCatalog_CursorFreeFixtures_ANullCursorOrScreenModeIsRefusedOutsideACompleteHerdrCapture(t *testing.T) {
	t.Parallel()
	files := bundledFiles()
	herdr := cursorScreen(files, "herdr")
	for _, tc := range []struct {
		name        string
		fields      []string
		multiplexer opt[string]
		expected    string
	}{
		{"no screen-level multiplexer", cursorFreeFields, none[string](), "a cursor-free fixture is a herdr capture"},
		{"a tmux screen", cursorFreeFields, some("tmux"), "a cursor-free fixture is a herdr capture"},
		{"a null cursor without a null screen mode", cursorFreeFields[1:], some("herdr"), "a cursor-free fixture records null cursor and screen mode together"},
		{"a null row without a null column", cursorFreeFields[:2], some("herdr"), "a cursor-free fixture records null cursor and screen mode together"},
		{"a multiplexer on a screen with a cursor", nil, some("herdr"), "only a cursor-free fixture names its multiplexer"},
	} {
		failure, refused := guardedBundledFiles(nulledScreen(files, herdr, tc.fields, tc.multiplexer)).get()
		if !refused || !strings.Contains(failure, tc.expected) {
			t.Fatalf("%s: the guard reports %q, want %q", tc.name, failure, tc.expected)
		}
	}
}

func TestCodingAgentCatalog_CursorFreeFixtures_ACursorFreeFixtureReplaysWithoutCursorOrScreenMode(t *testing.T) {
	t.Parallel()
	files := bundledFiles()
	herdr := cursorScreen(files, "herdr")
	screen := jclone(jget(files, "codex", "screens", herdr))
	recorded := replayNativeFixture(mustCanonicalJSON(screen), nil, fixtureAlternate(screen), none[[2]uint16]())
	if _, ok := recorded.cursor.get(); !ok {
		t.Fatal("the recorded screen replays with its cursor")
	}
	if _, ok := recorded.alternateScreen.get(); !ok {
		t.Fatal("the recorded screen replays with its screen mode")
	}
	for _, field := range cursorFreeFields {
		jset(screen, nil, field)
	}
	replayed := replayNativeFixture(mustCanonicalJSON(screen), nil, fixtureAlternate(screen), none[[2]uint16]())
	if replayed.cursor != none[[2]uint16]() || replayed.alternateScreen != none[bool]() {
		t.Fatalf("cursor %v, screen mode %v", replayed.cursor, replayed.alternateScreen)
	}
	if replayed.screen.Contents() != recorded.screen.Contents() {
		t.Fatal("the rows replay at their positions")
	}
}

func TestCodingAgentCatalog_PanelSeparator_APanelSeparatorThatIsNotOneNonWhitespaceGlyphIsRefused(t *testing.T) {
	t.Parallel()
	for _, separator := range []any{1, "", " ", "││"} {
		assertLoadRejected(t, testJSONText(separator), withPanelSeparator("/codex/profile/rules/0", separator))
	}
}

// requestOfCitedFixture is the request rule of the bundled answer plans citing fixture.
func requestOfCitedFixture(t testing.TB, backend codingAgent, fixture string) string {
	t.Helper()
	for _, cited := range bundledCitedPlans(t, backend) {
		if slices.ContainsFunc(cited.fixtures, func(f bundledFixtureRecord) bool { return f.id == fixture }) {
			return cited.plan.capability.requestRuleID
		}
	}
	t.Fatalf("%s: an answer plan cites it", fixture)
	panic("unreachable")
}

// assertRequestPlansExclusive asserts that every plan of the request rule whose plans cite anchor
// matches each form it cites as the observed request, matches no form a plan of another request
// rule cites, and that no plan of another request rule matches its forms.
func assertRequestPlansExclusive(t *testing.T, backend codingAgent, anchor string) {
	t.Helper()
	request := requestOfCitedFixture(t, backend, anchor)
	plans := bundledCitedPlans(t, backend)
	for _, own := range plans {
		if own.plan.capability.requestRuleID != request {
			continue
		}
		for _, form := range own.fixtures {
			observed := observeFixture(t, form, nil)
			matched, ok := matchFixture(t, form, nil, &own.plan)
			if !ok || !slices.Equal(matched.choices, observed.rawEvidence.choices.values) || !fingerprintsEqual(&matched.semantic, observed.fingerprint) {
				t.Fatalf("%s: the %s plan matches its pre-stage form as the observed request", form.id, own.plan.choiceID)
			}
		}
		for _, other := range plans {
			if other.plan.capability.requestRuleID == request {
				continue
			}
			for _, form := range other.fixtures {
				if _, ok := matchFixture(t, form, nil, &own.plan); ok {
					t.Fatalf("%s: a %s plan matches another request rule's form", form.id, own.plan.choiceID)
				}
			}
			for _, form := range own.fixtures {
				if _, ok := matchFixture(t, form, nil, &other.plan); ok {
					t.Fatalf("%s: another request rule's %s plan matches the form", form.id, other.plan.choiceID)
				}
			}
		}
	}
}

func TestCodingAgentCatalog_BundledClaudeAuto_BundledClaudeAutoAskFormAnswersAllowWith1AndDenyWith2Ungated(t *testing.T) {
	t.Parallel()
	assertRequestPlansExclusive(t, codingAgentClaude, "claude-2.1.302-auto-ask-permission-1")
}

func TestCodingAgentCatalog_BundledClaudeAuto_BundledClaudeAutoTipFormAnswersAllowWith1AndDenyWith4Ungated(t *testing.T) {
	t.Parallel()
	assertRequestPlansExclusive(t, codingAgentClaude, "claude-2.1.302-access-allow-pre-stage-1")
}

func TestCodingAgentCatalog_BundledClaudeAuto_BundledClaudeReadOutsideDialogAnswersAllowWith4AndDenyWith3Ungated(t *testing.T) {
	t.Parallel()
	assertRequestPlansExclusive(t, codingAgentClaude, "claude-2.1.309-read-outside-allow-pre-stage-1")
}

func TestCodingAgentCatalog_BundledClaudeAuto_BundledClaudeReadOutsideDialogsAreCompletePermissionRequestsWithTheirOperation(t *testing.T) {
	t.Parallel()
	read := observeFixture(t, bundledFixtureByID("claude-2.1.309-read-outside-permission-1"), nil)
	edit := observeFixture(t, bundledFixtureByID("claude-2.1.309-edit-outside-permission-1"), nil)
	for _, observed := range []*screenObservation{&read, &edit} {
		if observed.kind != observationKindPermissionRequest || observed.matchEvidence != matchEvidenceComplete || observed.fingerprint == nil || observed.verifiedNoninteraction {
			t.Fatalf("a complete permission request, got %s %s", observed.kind.debugString(), observed.matchEvidence.debugString())
		}
	}
	if read.ruleID != edit.ruleID || read.rawEvidence.operation == edit.rawEvidence.operation {
		t.Fatalf("one rule displays both paths: %s %s", read.ruleID, edit.ruleID)
	}
	if fingerprintsEqual(read.fingerprint, edit.fingerprint) {
		t.Fatal("a different displayed path is a different request")
	}
}

func TestCodingAgentCatalog_BundledClaudeAuto_BundledClaudeAutoStartupOfferAndWizardAreSeparateQuestions(t *testing.T) {
	t.Parallel()
	offer := observeFixture(t, bundledFixtureByID("claude-2.1.302-auto-env-setup-offer-1"), nil)
	wizard := observeFixture(t, bundledFixtureByID("claude-2.1.302-auto-env-setup-wizard-1"), nil)
	for _, observed := range []*screenObservation{&offer, &wizard} {
		if observed.kind != observationKindQuestion || observed.matchEvidence != matchEvidenceComplete || observed.verifiedNoninteraction ||
			observed.rawEvidence.operation != none[string]() || observed.fingerprint == nil {
			t.Fatalf("a complete question without an operation, got %s %s", observed.kind.debugString(), observed.ruleID)
		}
	}
	if fingerprintsEqual(offer.fingerprint, wizard.fingerprint) {
		t.Fatal("the offer and its wizard are separate dialogs")
	}
	bundled, ok := profileCatalogBundled().profile(codingAgentClaude)
	if !ok {
		t.Fatal("a bundled Claude profile")
	}
	offerPlans := bundled.answerPlans(offer.ruleID)
	if len(offerPlans) == 0 || slices.ContainsFunc(offerPlans, func(p answerPlan) bool { _, decision := answerKeyDecisionOf(p.capability.key); return decision }) {
		t.Fatalf("the offer answers by choice only, got %d plans", len(offerPlans))
	}
	if len(bundled.answerPlans(wizard.ruleID)) != 0 {
		t.Fatal("the wizard has no answer plan")
	}
}

func observeClaudeFixture(t testing.TB, id string, change func(*[]string)) screenObservation {
	t.Helper()
	return observeFixture(t, bundledFixtureByID(id), change)
}

func observeClaudeDocument(t testing.TB, id string, document any, alternate bool) screenObservation {
	t.Helper()
	snapshot := replayNativeFixture(testJSONText(document), nil, alternate, none[[2]uint16]())
	return observeBundled(t, codingAgentClaude, bundledFixtureByID(id).version, &snapshot)
}

func assertInteraction(t testing.TB, label string, observed *screenObservation) {
	t.Helper()
	if _, interaction := observed.kind.interaction(); !interaction {
		t.Fatalf("%s: an interaction, got %s", label, observed.kind.debugString())
	}
}

func TestCodingAgentCatalog_BundledClaude21312_PrimaryScreenFixturesMatchTheirAlternateScreenRendering(t *testing.T) {
	t.Parallel()
	for _, id := range []string{"claude-2.1.312-primary-auto-default-nudge-1", "claude-2.1.312-primary-default-permission-1"} {
		primary := observeClaudeDocument(t, id, fixtureDocument(id), false)
		alternate := observeClaudeDocument(t, id, fixtureDocument(id), true)
		assertPrimaryMatchesAlternate(t, id, &primary, &alternate)
		assertInteraction(t, id, &primary)
	}
}

func TestCodingAgentCatalog_BundledClaude21312_SidebarFixturesMatchTheirFullWidthRendering(t *testing.T) {
	t.Parallel()
	panels := []struct {
		id        string
		separator int
	}{
		{"claude-2.1.312-panel-default-permission-1", 92},
		{"claude-2.1.312-panel-default-permission-144-1", 80},
		{"claude-2.1.312-panel-default-allow-pre-stage-1", 92},
	}
	for _, panel := range panels {
		beside := observeClaudeDocument(t, panel.id, fixtureDocument(panel.id), true)
		full := fixtureDocument(panel.id)
		jobject(full).remove("cell_backgrounds")
		lines := jarray(jget(full, "lines"))
		for index, line := range lines.items {
			text, _ := jstring(line)
			if runes := []rune(text); len(runes) > panel.separator && runes[panel.separator] == '│' {
				lines.items[index] = string(runes[:panel.separator])
			}
		}
		fullWidth := observeClaudeDocument(t, panel.id, full, true)
		assertPrimaryMatchesAlternate(t, panel.id, &beside, &fullWidth)
		assertInteraction(t, panel.id, &beside)
	}
}

var claudeHeadings = []string{" Read file", " Edit file", " Create file", " Bash command"}

// headingRow is the one row whose first width characters are a form heading.
func headingRow(t testing.TB, lines []string, width int) int {
	t.Helper()
	var rows []int
	for row, line := range lines {
		left := []rune(line)
		if len(left) > width {
			left = left[:width]
		}
		if slices.Contains(claudeHeadings, strings.TrimRight(string(left), " ")) {
			rows = append(rows, row)
		}
	}
	if len(rows) != 1 {
		t.Fatalf("one heading row, got %v", rows)
	}
	return rows[0]
}

// withCounter right-aligns counter at width on the heading row, keeping the row's text past width.
func withCounter(t testing.TB, width int, counter string) func(*[]string) {
	return func(lines *[]string) {
		row := headingRow(t, *lines, width)
		runes := []rune((*lines)[row])
		left, rest := runes, []rune(nil)
		if len(runes) > width {
			left, rest = runes[:width], runes[width:]
		}
		(*lines)[row] = padTo(strings.TrimRight(string(left), " "), width-utf8.RuneCountInString(counter)) + counter + string(rest)
	}
}

var claudeHeadingCounter = regexp.MustCompile(`^( (?:Read file|Edit file|Create file|Bash command)) +(\d+ of \d+)$`)

func withoutCounter(t testing.TB) func(*[]string) {
	return func(lines *[]string) {
		var rows []int
		for row, line := range *lines {
			if claudeHeadingCounter.MatchString(strings.TrimRight(line, " ")) {
				rows = append(rows, row)
			}
		}
		if len(rows) != 1 {
			t.Fatalf("one counted heading, got %v", rows)
		}
		(*lines)[rows[0]] = claudeHeadingCounter.FindStringSubmatch(strings.TrimRight((*lines)[rows[0]], " "))[1]
	}
}

var claudeBatchedReads = []string{
	"claude-2.1.312-read-file-batched-permission-1",
	"claude-2.1.312-read-file-batched-allow-post-stage-1",
	"claude-2.1.312-read-file-batched-deny-pre-stage-1",
}

func TestCodingAgentCatalog_BundledClaude21312_BatchedReadFixturesMatchTheirCounterFreeSingleForm(t *testing.T) {
	t.Parallel()
	for _, id := range claudeBatchedReads {
		batched := observeClaudeFixture(t, id, nil)
		single := observeClaudeFixture(t, id, withoutCounter(t))
		assertPrimaryMatchesAlternate(t, id, &batched, &single)
		assertInteraction(t, id, &batched)
	}
}

func TestCodingAgentCatalog_BundledClaude21312_AHeadingCounterChangesNoIdentityOnEveryHeadingAnchoredForm(t *testing.T) {
	t.Parallel()
	rules := map[string]bool{}
	forms := []string{
		"claude-2.1.312-read-file-permission-1",
		"claude-2.1.312-edit-file-permission-1",
		"claude-2.1.312-create-file-permission-1",
		"claude-2.1.312-bash-no-tip-permission-1",
		"claude-2.1.312-primary-default-permission-1",
		"claude-2.1.302-access-permission-1",
		"claude-2.1.302-auto-ask-permission-1",
	}
	for _, id := range forms {
		single := observeClaudeFixture(t, id, nil)
		assertInteraction(t, id, &single)
		counted := observeClaudeFixture(t, id, withCounter(t, int(fixtureSize(id).cols), "10 of 12"))
		assertPrimaryMatchesAlternate(t, id, &counted, &single)
		rules[single.ruleID] = true
	}
	if len(rules) != len(forms) {
		t.Fatalf("each heading-anchored form has its own rule, got %d rules for %d forms", len(rules), len(forms))
	}
}

func TestCodingAgentCatalog_BundledClaude21312_AHeadingCounterOfAnotherShapeLeavesTheFormUnmatched(t *testing.T) {
	t.Parallel()
	const id = "claude-2.1.312-read-file-permission-1"
	edits := map[string]func(*[]string){}
	for _, counter := range []string{"1 of", "of 2", "1/2", "1 of 2 queued", "one of two"} {
		edits[counter] = withCounter(t, 120, counter)
	}
	assertEditedLeavesNoInteraction(t, id, edits, []opt[[2]uint16]{none[[2]uint16]()})
}

func TestCodingAgentCatalog_BundledClaude21312_BatchedReadFormsResolveTheSingleRequestDigits(t *testing.T) {
	t.Parallel()
	bundled, ok := profileCatalogBundled().profile(codingAgentClaude)
	if !ok {
		t.Fatal("a bundled Claude profile")
	}
	for index, id := range claudeBatchedReads {
		decision := permissionDecisionAllow
		if index == 2 {
			decision = permissionDecisionDeny
		}
		form := bundledFixtureByID(id)
		plan, ok := bundled.answerPlan(observeFixture(t, form, nil).ruleID, answerKeyDecision{decision: decision})
		if !ok {
			t.Fatalf("%s: a decision plan of its rule", id)
		}
		if _, ok := matchFixture(t, form, nil, &plan); !ok {
			t.Fatalf("%s: the batched form matches its %s plan", id, plan.choiceID)
		}
	}
}

func TestCodingAgentCatalog_BundledClaude21312_DialogADeclineResolvesToTwoMarkerGatedStages(t *testing.T) {
	t.Parallel()
	const anchor = "claude-2.1.312-auto-default-decline-pre-stage-1"
	for _, cited := range bundledCitedPlans(t, codingAgentClaude) {
		if !slices.ContainsFunc(cited.fixtures, func(f bundledFixtureRecord) bool { return f.id == anchor }) {
			continue
		}
		held := make([]bool, len(cited.plan.stages))
		for index := range cited.plan.stages {
			if _, gated := cited.plan.stages[index].requiredFocusChoiceID(); !gated {
				t.Fatalf("stage %d of the %s plan is marker-gated", index, cited.plan.choiceID)
			}
		}
		forms := append(slices.Clone(cited.fixtures), bundledFixtureByID("claude-2.1.312-primary-auto-default-nudge-1"))
		for _, form := range forms {
			if observed := observeFixture(t, form, nil); observed.ruleID != cited.plan.capability.requestRuleID {
				if _, interaction := observed.kind.interaction(); interaction {
					t.Fatalf("%s: a cited screen after the final stage is no interaction, got %s", form.id, observed.ruleID)
				}
				continue
			}
			matched, ok := matchFixture(t, form, nil, &cited.plan)
			focus, focused := matched.focusedChoice.get()
			if !ok || !focused {
				t.Fatalf("%s: the %s plan matches its dialog with a focus", form.id, cited.plan.choiceID)
			}
			gated := false
			for index := range cited.plan.stages {
				if gate, _ := cited.plan.stages[index].requiredFocusChoiceID(); gate == focus {
					held[index], gated = true, true
				}
			}
			if !gated {
				t.Fatalf("%s: focus %s holds no stage gate", form.id, focus)
			}
		}
		if slices.Contains(held, false) {
			t.Fatalf("a dialog holds each stage gate, got %v", held)
		}
		return
	}
	t.Fatalf("an answer plan cites %s", anchor)
}

func TestCodingAgentCatalog_BundledClaude21312_DialogBOfferChoicesResolveToTheirSingleDigits(t *testing.T) {
	t.Parallel()
	offers := []string{"claude-2.1.312-auto-env-setup-offer-1", "claude-2.1.302-auto-env-setup-offer-1"}
	bundled, ok := profileCatalogBundled().profile(codingAgentClaude)
	if !ok {
		t.Fatal("a bundled Claude profile")
	}
	plans := bundled.answerPlans(observeClaudeFixture(t, offers[0], nil).ruleID)
	if len(plans) == 0 {
		t.Fatal("the offer has answer plans")
	}
	for _, plan := range plans {
		for _, id := range offers {
			if _, ok := matchFixture(t, bundledFixtureByID(id), nil, &plan); !ok {
				t.Fatalf("%s: the %s plan matches", id, plan.choiceID)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledClaude21312_NoPlanSelectsAYesChoiceOfEitherNudgeDialog(t *testing.T) {
	t.Parallel()
	bundled, ok := profileCatalogBundled().profile(codingAgentClaude)
	if !ok {
		t.Fatal("a bundled Claude profile")
	}
	for _, id := range []string{
		"claude-2.1.312-auto-default-nudge-1",
		"claude-2.1.312-auto-env-setup-offer-1",
		"claude-2.1.302-auto-env-setup-wizard-1",
		"claude-2.1.312-question-pending-notice-1",
	} {
		form := bundledFixtureByID(id)
		observed := observeFixture(t, form, nil)
		assertInteraction(t, id, &observed)
		for _, plan := range bundled.answerPlans(observed.ruleID) {
			if _, decision := answerKeyDecisionOf(plan.capability.key); decision {
				t.Fatalf("%s: a question answers by choice, not decision", id)
			}
			matched, ok := matchFixture(t, form, nil, &plan)
			if ok && strings.HasPrefix(matched.choiceLabel, "Yes") {
				t.Fatalf("%s: the %s plan selects %q", id, plan.choiceID, matched.choiceLabel)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledClaude21312_FileAndNoTipDecisionsResolveToTheirSingleRequestDigits(t *testing.T) {
	t.Parallel()
	for _, anchor := range []string{
		"claude-2.1.312-read-file-allow-pre-stage-1",
		"claude-2.1.312-edit-file-allow-pre-stage-1",
		"claude-2.1.312-create-file-allow-pre-stage-1",
		"claude-2.1.312-bash-no-tip-allow-pre-stage-1",
	} {
		assertRequestPlansExclusive(t, codingAgentClaude, anchor)
	}
}

func TestCodingAgentCatalog_BundledClaudeAskUserQuestion_EveryExistingClaudeFixtureKeepsItsObservationWithTheAskRulesPresent(t *testing.T) {
	t.Parallel()
	askCaptures := []string{
		"claude-2.1.322-ask-single-tmux-120-1", "claude-2.1.322-ask-tabs-tmux-120-1", "claude-2.1.322-ask-tabs-tmux-120-2",
		"claude-2.1.322-ask-multi-tmux-120-1", "claude-2.1.322-ask-multi-tmux-120-2", "claude-2.1.322-ask-multi-tmux-120-3",
		"claude-2.1.322-ask-review-tmux-120-1", "claude-2.1.322-ask-review-narrow-tmux-060-1", "claude-2.1.322-ask-typing-tmux-120-1",
		"claude-2.1.322-ask-typing-tmux-120-2", "claude-2.1.322-ask-narrow-tmux-060-1", "claude-2.1.322-ask-pending-tmux-120-1",
		"claude-2.1.322-ask-single-herdr-359-1", "claude-2.1.322-ask-tabs-herdr-359-1", "claude-2.1.322-ask-tabs-herdr-359-2",
		"claude-2.1.322-ask-multi-herdr-359-1", "claude-2.1.322-ask-multi-herdr-359-2", "claude-2.1.322-ask-multi-herdr-359-3",
		"claude-2.1.322-ask-review-herdr-359-1",
		"claude-2.1.322-ask-review-tmux-120-2", "claude-2.1.322-ask-review-narrow-tmux-060-2",
		"claude-2.1.322-ask-review-narrow-tmux-040-2", "claude-2.1.322-ask-review-herdr-359-2",
		"claude-2.1.322-ask-narrow-tmux-040-1", "claude-2.1.322-ask-review-narrow-tmux-040-1",
		"claude-2.1.322-ask-single-closed-tmux-120-1", "claude-2.1.322-ask-single-closed-herdr-359-1",
		"claude-2.1.302-auto-model-switch-question-1", "claude-2.1.322-ask-quoted-tmux-120-1",
	}
	askRules := []string{
		observeClaudeFixture(t, "claude-2.1.322-ask-single-tmux-120-1", nil).ruleID,
		observeClaudeFixture(t, "claude-2.1.322-ask-review-tmux-120-2", nil).ruleID,
	}
	files := bundledFiles()
	isAsk := func(id any) bool { text, _ := jstring(id); return slices.Contains(askRules, text) }
	rules := jarray(jget(files, "claude", "profile", "rules"))
	rules.items = slices.DeleteFunc(rules.items, func(r any) bool { return isAsk(jget(r, "id")) })
	detection := jarray(jget(files, "claude", "profile", "capabilities", "detection", "rule_ids"))
	detection.items = slices.DeleteFunc(detection.items, isAsk)
	without, err := profileCatalogFromFiles(files)
	if err != nil {
		t.Fatal(err)
	}
	compared := 0
	for _, fixture := range bundledFixtureRecords() {
		if fixture.backend != codingAgentClaude || slices.Contains(askCaptures, fixture.id) {
			continue
		}
		compared++
		screen := replayAsCaptured(fixture.id, nil, none[[2]uint16]())
		with := observeBundled(t, codingAgentClaude, fixture.version, &screen)
		before, err := newDetector(without, codingAgentClaude, some(fixture.version)).observe(&screen)
		if err != nil {
			t.Fatal(err)
		}
		if with.kind != before.kind || with.ruleID != before.ruleID || with.verifiedNoninteraction != before.verifiedNoninteraction ||
			with.rawEvidence.operation != before.rawEvidence.operation || with.rawEvidence.choices.set != before.rawEvidence.choices.set ||
			!slices.Equal(with.rawEvidence.choices.values, before.rawEvidence.choices.values) || !fingerprintsEqual(with.fingerprint, before.fingerprint) {
			t.Fatalf("%s: with the ask rules %s %s, without %s %s", fixture.id, with.kind.debugString(), with.ruleID, before.kind.debugString(), before.ruleID)
		}
	}
	if compared == 0 {
		t.Fatal("an existing Claude fixture")
	}
}

const (
	opencodePermissionFixture = "opencode-2.1.4-ask-permission-1"
	opencodeDeny1Fixture      = "opencode-2.1.4-deny-pre-stage-1"
	opencodeDeny2Fixture      = "opencode-2.1.4-deny-pre-stage-2"
	opencodeDeny3Fixture      = "opencode-2.1.4-deny-pre-stage-3"
	opencodeAlwaysFixture     = "opencode-2.1.4-focus-always-1"
	opencodeIdleFixture       = "opencode-2.1.4-idle-composer-1"
	opencodeHeading           = "  △ Permission required"
	opencodeChoiceRow         = "   Allow once   Always allow   Reject                                             ⇆ select  enter confirm  esc reject"
	opencodePlaceholder       = "Tell OpenCode what to do differently"
)

func opencodeReplay(t testing.TB, document any, change func(*[]string), cursor opt[[2]uint16]) screenObservation {
	t.Helper()
	snapshot := replayNativeFixture(testJSONText(document), change, false, cursor)
	return observeBundled(t, codingAgentOpencode, opencodeLayoutVersion, &snapshot)
}

func opencodeFixture(t testing.TB, id string, change func(*[]string), cursor opt[[2]uint16]) screenObservation {
	t.Helper()
	return opencodeReplay(t, fixtureDocument(id), change, cursor)
}

// opencodeFocus is the one focus every bundled plan of the screen's request rule resolves on it.
func opencodeFocus(t testing.TB, document any) opt[string] {
	t.Helper()
	request := opencodeReplay(t, document, nil, none[[2]uint16]()).ruleID
	snapshot := replayNativeFixture(testJSONText(document), nil, false, none[[2]uint16]())
	d := bundledDetector(codingAgentOpencode, opencodeLayoutVersion)
	var focus []opt[string]
	for _, cited := range bundledCitedPlans(t, codingAgentOpencode) {
		if cited.plan.capability.requestRuleID != request {
			continue
		}
		matched := mustMatchAnswer(t, d, snapshot, &cited.plan, "the form matches every plan of its rule whatever its focus")
		focus = append(focus, matched.focusedChoice)
	}
	if len(focus) == 0 || slices.ContainsFunc(focus, func(f opt[string]) bool { return f != focus[0] }) {
		t.Fatalf("every plan shares the focus matcher, got %v", focus)
	}
	return focus[0]
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeStageGatesHoldOnTheirPreStageFixtures(t *testing.T) {
	t.Parallel()
	plans := bundledCitedPlans(t, codingAgentOpencode)
	for _, cited := range plans {
		held := make([]bool, len(cited.plan.stages))
		for _, fixture := range cited.fixtures {
			observed := opencodeFixture(t, fixture.id, nil, none[[2]uint16]())
			if observed.ruleID == cited.plan.capability.requestRuleID {
				focus := opencodeFocus(t, fixtureDocument(fixture.id))
				for index := range cited.plan.stages {
					if gate, gated := cited.plan.stages[index].requiredFocusChoiceID(); gated && focus == some(gate) {
						held[index] = true
					}
				}
				continue
			}
			if observed.kind != observationKindPermissionContinuation || observed.matchEvidence != matchEvidenceComplete || observed.verifiedNoninteraction ||
				observed.continuesRuleID != some(cited.plan.capability.requestRuleID) {
				t.Fatalf("%s: a cited continuation of %s, got %s %s", fixture.id, cited.plan.capability.requestRuleID, observed.kind.debugString(), observed.ruleID)
			}
			for index := range cited.plan.stages {
				if rule, gated := cited.plan.stages[index].requiredScreenRuleID(); gated && rule == observed.ruleID {
					held[index] = true
				}
			}
			for _, other := range plans {
				if _, ok := matchFixture(t, fixture, nil, &other.plan); ok {
					t.Fatalf("%s: the rejection-reason dialog is not an answerable form", fixture.id)
				}
			}
		}
		if slices.Contains(held, false) {
			t.Fatalf("%s %s plan: a cited screen holds each stage gate, got %v", cited.plan.capability.requestRuleID, cited.plan.choiceID, held)
		}
	}
	always := opencodeFocus(t, fixtureDocument(opencodeAlwaysFixture))
	for _, cited := range plans {
		for index := range cited.plan.stages {
			if gate, gated := cited.plan.stages[index].requiredFocusChoiceID(); gated && always == some(gate) {
				t.Fatalf("no focus gate holds on Always allow, %s does", gate)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeNarrowStageGatesHoldOnTheirPreStageFixtures(t *testing.T) {
	t.Parallel()
	for _, cited := range bundledCitedPlans(t, codingAgentOpencode) {
		for _, fixture := range cited.fixtures {
			if fixtureSize(fixture.id).cols >= 120 {
				continue
			}
			observed := opencodeFixture(t, fixture.id, nil, none[[2]uint16]())
			if observed.ruleID != cited.plan.capability.requestRuleID && observed.continuesRuleID != some(cited.plan.capability.requestRuleID) {
				t.Fatalf("%s: a narrow pre-stage screen of %s, got %s", fixture.id, cited.plan.capability.requestRuleID, observed.ruleID)
			}
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeNarrowRejectDialogsRequireTheEmptyPlaceholder(t *testing.T) {
	t.Parallel()
	for _, id := range []string{
		"opencode-2.1.4-narrow-deny-pre-stage-3-057-1",
		"opencode-2.1.4-compact-deny-pre-stage-3-055-1",
		"opencode-2.1.4-narrow-deny-pre-stage-3-min-1",
	} {
		empty := opencodeFixture(t, id, nil, none[[2]uint16]())
		if empty.kind != observationKindPermissionContinuation {
			t.Fatalf("%s: the empty dialog is a continuation, got %s", id, empty.kind.debugString())
		}
		row := lastRowContaining(fixtureLines(id), opencodePlaceholder)
		typed := opencodeFixture(t, id, func(lines *[]string) {
			(*lines)[row] = strings.Replace((*lines)[row], opencodePlaceholder, "run it in the sandbox instead       ", 1)
		}, none[[2]uint16]())
		if typed.kind == observationKindPermissionContinuation || typed.ruleID == empty.ruleID {
			t.Fatalf("%s: a typed reason is %s %s", id, typed.kind.debugString(), typed.ruleID)
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeClippedLongCommandYieldsNoRequest(t *testing.T) {
	t.Parallel()
	const id = "opencode-2.1.4-narrow-long-command-057-1"
	observed := opencodeFixture(t, id, nil, none[[2]uint16]())
	if observed.kind == observationKindPermissionRequest || observed.fingerprint != nil {
		t.Fatalf("%s: %s %s", id, observed.kind.debugString(), observed.ruleID)
	}
	snapshot := replayNativeFixture(bundledScreen(id), nil, false, none[[2]uint16]())
	d := bundledDetector(codingAgentOpencode, opencodeLayoutVersion)
	for _, cited := range bundledCitedPlans(t, codingAgentOpencode) {
		assertNoAnswerMatch(t, d, snapshot, &cited.plan, id+" "+cited.plan.capability.requestRuleID)
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeDetectsEachNativeFormFromItsLiveRegion(t *testing.T) {
	t.Parallel()
	first := opencodeFixture(t, opencodeDeny1Fixture, nil, none[[2]uint16]())
	second := opencodeFixture(t, opencodeDeny2Fixture, nil, none[[2]uint16]())
	if first.kind != observationKindPermissionRequest || second.kind != observationKindPermissionRequest || fingerprintsEqual(first.fingerprint, second.fingerprint) {
		t.Fatal("different cases carry different operations")
	}
	elsewhere := opencodeFixture(t, opencodeDeny1Fixture, nil, some([2]uint16{5, 0}))
	assertSameRequestAs(t, "a cursor elsewhere", &elsewhere, &first)
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeFocusResolvesFromTheCapturedCellBackgrounds(t *testing.T) {
	t.Parallel()
	captured := opencodeFixture(t, opencodePermissionFixture, nil, none[[2]uint16]())
	if _, focused := opencodeFocus(t, fixtureDocument(opencodePermissionFixture)).get(); !focused {
		t.Fatal("the captured backgrounds focus a choice")
	}
	textOnly := fixtureDocument(opencodePermissionFixture)
	jobject(textOnly).remove("cell_backgrounds")
	observed := opencodeReplay(t, textOnly, nil, none[[2]uint16]())
	assertSameRequestAs(t, "the text-only form", &observed, &captured)
	if focus := opencodeFocus(t, textOnly); focus != none[string]() {
		t.Fatalf("the form marks focus by cell background only, got %v", focus)
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeFocusResolutionIsIndependentOfTheThemeColours(t *testing.T) {
	t.Parallel()
	for _, id := range []string{opencodePermissionFixture, opencodeDeny2Fixture, opencodeAlwaysFixture} {
		screen := fixtureDocument(id)
		for _, run := range jarray(jget(screen, "cell_backgrounds")).items {
			rgb := testJSONText(ja(jget(run, "bg", "r"), jget(run, "bg", "g"), jget(run, "bg", "b")))
			switch rgb {
			case "[250,178,131]":
				jset(run, jsonf(`{"kind":"indexed","index":214}`), "bg")
			case "[30,30,30]":
				jset(run, jsonf(`{"kind":"rgb","r":238,"g":238,"b":238}`), "bg")
			default:
				jset(run, jsonf(`{"kind":"indexed","index":250}`), "bg")
			}
		}
		if got, want := opencodeFocus(t, screen), opencodeFocus(t, fixtureDocument(id)); got != want {
			t.Fatalf("%s: rethemed focus %v, captured %v", id, got, want)
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeRejectReasonRequiresTheEmptyPlaceholderTextarea(t *testing.T) {
	t.Parallel()
	empty := opencodeFixture(t, opencodeDeny3Fixture, nil, none[[2]uint16]())
	if empty.kind != observationKindPermissionContinuation {
		t.Fatalf("the empty dialog is a continuation, got %s", empty.kind.debugString())
	}
	cases := []struct {
		name   string
		change func(*[]string)
	}{
		{"a typed reason", func(lines *[]string) {
			(*lines)[21] = strings.Replace((*lines)[21], opencodePlaceholder, "run it in the sandbox instead      ", 1)
		}},
		{"the span equals the placeholder exactly", func(lines *[]string) {
			(*lines)[21] = strings.Replace((*lines)[21], opencodePlaceholder+" ", opencodePlaceholder+"!", 1)
		}},
		{"without its prompt", setRow(12, "")},
	}
	for _, c := range cases {
		if observed := opencodeFixture(t, opencodeDeny3Fixture, c.change, none[[2]uint16]()); observed.ruleID == empty.ruleID {
			t.Fatalf("%s: still %s", c.name, observed.ruleID)
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeIdleComposerIsAVerifiedNonInteractionBoundary(t *testing.T) {
	t.Parallel()
	idle := opencodeFixture(t, opencodeIdleFixture, nil, none[[2]uint16]())
	assertVerifiedEmpty(t, "the idle composer", &idle)
	if idle.rawEvidence.operation != none[string]() || idle.fingerprint != nil {
		t.Fatal("an idle composer carries no request")
	}
	draft := opencodeFixture(t, opencodeIdleFixture, setRow(11, "┃ hello"), some([2]uint16{11, 7}))
	away := opencodeFixture(t, opencodeIdleFixture, nil, some([2]uint16{11, 0}))
	working := opencodeFixture(t, opencodePermissionFixture, nil, none[[2]uint16]())
	for name, observed := range map[string]*screenObservation{"typed draft": &draft, "cursor before the prefix end": &away, "a permission form": &working} {
		if observed.kind == observationKindComposerEmpty || observed.verifiedNoninteraction {
			t.Fatalf("%s: %s verified=%v", name, observed.kind.debugString(), observed.verifiedNoninteraction)
		}
	}
}

func TestCodingAgentCatalog_BundledOpencode_BundledOpencodeDetectsNothingFromFormOrDialogTextOutsideALiveRegion(t *testing.T) {
	t.Parallel()
	form := fixtureLines(opencodePermissionFixture)
	dialog := fixtureLines(opencodeDeny3Fixture)
	formRule := opencodeFixture(t, opencodePermissionFixture, nil, none[[2]uint16]()).ruleID
	reasonRule := opencodeFixture(t, opencodeDeny3Fixture, nil, none[[2]uint16]()).ruleID
	cases := []struct {
		name   string
		change func(*[]string)
	}{
		{"idle screen", nil},
		{"quoted heading and command in the transcript", func(lines *[]string) {
			(*lines)[5] = opencodeHeading
			(*lines)[6] = "  $ /tmp/cafleet-194/opencode-cells-001/permission-probe allow"
		}},
		{"quoted form without its control line", func(lines *[]string) { copy((*lines)[:11], form[10:21]) }},
		{"quoted control line without its heading", setRow(5, opencodeChoiceRow)},
		{"heading and control line too far apart", func(lines *[]string) {
			(*lines)[0] = opencodeHeading
			(*lines)[34] = opencodeChoiceRow
		}},
		{"quoted reject dialog without its control line", func(lines *[]string) {
			(*lines)[5] = dialog[10]
			(*lines)[7] = dialog[12]
		}},
	}
	for _, c := range cases {
		observed := opencodeFixture(t, opencodeIdleFixture, c.change, none[[2]uint16]())
		if _, interaction := observed.kind.interaction(); interaction || observed.kind == observationKindPermissionContinuation ||
			observed.ruleID == formRule || observed.ruleID == reasonRule {
			t.Fatalf("%s must not open a request: %s %s", c.name, observed.kind.debugString(), observed.ruleID)
		}
	}
}

// assertSeparateSubmitStages asserts that backend's bundled prompt and exec plans type each
// literal, with the exec plan's native prefix, in their first stage and submit it with one shared
// second stage.
func assertSeparateSubmitStages(t *testing.T, backend codingAgent, prompt, exec string) {
	t.Helper()
	stagesOf := func(operation composerOperation, literal string) [2][]byte {
		plan, ok := profileCatalogBundled().composerPlan(backend, operation)
		if !ok {
			t.Fatalf("%s: a %s plan", backend.name(), composerOperationDebug(operation))
		}
		stages, err := plan.stages(literal, literalFramingPlain)
		if err != nil {
			t.Fatal(err)
		}
		return stages
	}
	typed := stagesOf(composerOperationPrompt, prompt)
	native := stagesOf(composerOperationExec, exec)
	if !bytes.Equal(typed[0], []byte(prompt)) {
		t.Fatalf("%s: the prompt stage %q types the literal %q", backend.name(), typed[0], prompt)
	}
	if !bytes.HasSuffix(native[0], []byte(exec)) || len(native[0]) == len(exec) {
		t.Fatalf("%s: the exec stage %q types a native prefix and the literal %q", backend.name(), native[0], exec)
	}
	if len(typed[1]) == 0 || !bytes.Equal(typed[1], native[1]) {
		t.Fatalf("%s: one separate submit stage, got %q and %q", backend.name(), typed[1], native[1])
	}
}

func TestCodingAgentCatalog_BundledBackends_HermesComposerPlansSubmitAPromptAndANativeExecInSeparateStages(t *testing.T) {
	t.Parallel()
	assertSeparateSubmitStages(t, codingAgentHermes, "ok", "echo x")
}

func TestCodingAgentCatalog_BundledBackends_PiComposerPlansSubmitAPromptAndANativeExecInSeparateStages(t *testing.T) {
	t.Parallel()
	assertSeparateSubmitStages(t, codingAgentPi, "-leading dash", "echo cafleet-220-exec")
}

func TestCodingAgentCatalog_BundledBackends_BundledSubmissionFixturesReplayTheirComposerStagesThroughTheProductionCatalog(t *testing.T) {
	t.Parallel()
	cited := 0
	for _, backend := range codingAgentAll() {
		submission, ok := jget(parsedBundledFiles(), backend.name(), "profile", "capabilities", "composer", "submission").(*jsonObj)
		if !ok {
			continue
		}
		for _, member := range submission.members {
			operation := map[string]composerOperation{"prompt": composerOperationPrompt, "exec": composerOperationExec}[member.key]
			if operation == 0 {
				t.Fatalf("%s: a submission operation %q", backend.name(), member.key)
			}
			if _, ok := profileCatalogBundled().composerPlan(backend, operation); !ok {
				t.Fatalf("%s: a %s plan", backend.name(), member.key)
			}
			for _, id := range jarray(jget(member.value, "fixture_ids")).items {
				name, _ := jstring(id)
				cited++
				fixture := bundledFixtureByID(name)
				observed := observeFixture(t, fixture, nil)
				if _, interaction := observed.kind.interaction(); interaction {
					t.Fatalf("%s: a composer stage is never an interaction, got %s", name, observed.kind.debugString())
				}
			}
		}
	}
	if cited == 0 {
		t.Fatal("a bundled submission cites its fixtures")
	}
}
