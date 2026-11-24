package core

import (
	"errors"
	"os"
	"slices"
	"syscall"
	"testing"
	"time"
)

type sampleEncoded struct {
	Z int         `json:"z"`
	A string      `json:"a"`
	L []int       `json:"l"`
	N any         `json:"n"`
	O opt[string] `json:"o"`
	S opt[string] `json:"s"`
	B bool        `json:"b"`
}

func TestTextcodec_EncodeJsonWritesTheSpecEncoding(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		value any
		want  string
	}{
		{
			name:  "string escapes",
			value: "\"\\\b\f\n\r\t\x00\x1f/\x7f é\u2028\u2029<>&",
			want:  "\"\\\"\\\\\\b\\f\\n\\r\\t\\u0000\\u001f/\x7f é\u2028\u2029<>&\"",
		},
		{
			name:  "integers",
			value: []any{int64(-9223372036854775808), int64(9223372036854775807), uint64(18446744073709551615), -1, 0},
			want:  `[-9223372036854775808,9223372036854775807,18446744073709551615,-1,0]`,
		},
		{
			name:  "struct fields in declaration order",
			value: sampleEncoded{Z: 1, A: "x", O: none[string](), S: some("v"), B: true},
			want:  `{"z":1,"a":"x","l":[],"n":null,"o":null,"s":"v","b":true}`,
		},
		{
			name:  "ordered dynamic object",
			value: jsonObject{{"z", 1}, {"a", jsonObject{{"y", "v"}}}, {"m", []any{}}, {"e", jsonObject{}}},
			want:  `{"z":1,"a":{"y":"v"},"m":[],"e":{}}`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := encodeJSON(tc.value)
			if err != nil || string(got) != tc.want {
				t.Fatalf("%s: encodeJSON = %q %v, want %q", tc.name, got, err, tc.want)
			}
		})
	}
}

func TestTextcodec_EncodeJsonRefusesInvalidUtf8(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		value any
	}{
		{name: "string", value: "bad \xff"},
		{name: "object key", value: jsonObject{{"bad \xff", 1}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got, err := encodeJSON(tc.value); err == nil {
				t.Fatalf("%s: encodeJSON = %q", tc.name, got)
			}
		})
	}
}

type sampleStrictNested struct {
	ID string `json:"id"`
}

type sampleStrict struct {
	Name    string               `json:"name"`
	Count   uint64               `json:"count"`
	Nested  sampleStrictNested   `json:"nested"`
	Items   []sampleStrictNested `json:"items"`
	Label   opt[string]          `json:"label"`
	Limit   opt[int64]           `cafleet:"present" json:"limit"`
	Capture opt[string]          `cafleet:"nullable" json:"capture"`
	Plain   opt[opt[int64]]      `cafleet:"present" json:"plain"`
	Retries int64                `cafleet:"default" json:"retries"`
}

const strictRequired = `"name":"n","count":1,"nested":{"id":"x"},"items":[{"id":"a"}],"capture":null`

func TestTextcodec_DecodeStrictAppliesEachFieldsPresenceRule(t *testing.T) {
	t.Parallel()
	base := sampleStrict{
		Name:    "n",
		Count:   1,
		Nested:  sampleStrictNested{ID: "x"},
		Items:   []sampleStrictNested{{ID: "a"}},
		Label:   none[string](),
		Limit:   none[int64](),
		Capture: none[string](),
		Plain:   none[opt[int64]](),
	}
	with := func(change func(*sampleStrict)) sampleStrict {
		value := base
		value.Items = slices.Clone(base.Items)
		change(&value)
		return value
	}
	cases := []struct {
		name  string
		input string
		want  sampleStrict
	}{
		{name: "required members only", input: `{` + strictRequired + `}`, want: base},
		{name: "whitespace between tokens", input: " {\n" + strictRequired + " }\n", want: base},
		{name: "optional member null", input: `{` + strictRequired + `,"label":null}`, want: base},
		{name: "optional member value", input: `{` + strictRequired + `,"label":"l"}`, want: with(func(v *sampleStrict) { v.Label = some("l") })},
		{name: "present member value", input: `{` + strictRequired + `,"limit":5}`, want: with(func(v *sampleStrict) { v.Limit = some(int64(5)) })},
		{
			name:  "nullable member value",
			input: `{"name":"n","count":1,"nested":{"id":"x"},"items":[{"id":"a"}],"capture":"c"}`,
			want:  with(func(v *sampleStrict) { v.Capture = some("c") }),
		},
		{name: "tri-state member null", input: `{` + strictRequired + `,"plain":null}`, want: with(func(v *sampleStrict) { v.Plain = some(none[int64]()) })},
		{name: "tri-state member value", input: `{` + strictRequired + `,"plain":3}`, want: with(func(v *sampleStrict) { v.Plain = some(some(int64(3))) })},
		{name: "defaulted member value", input: `{` + strictRequired + `,"retries":4}`, want: with(func(v *sampleStrict) { v.Retries = 4 })},
		{
			name:  "largest unsigned integer",
			input: `{"name":"n","count":18446744073709551615,"nested":{"id":"x"},"items":[{"id":"a"}],"capture":null}`,
			want:  with(func(v *sampleStrict) { v.Count = 18446744073709551615 }),
		},
		{name: "member order is free", input: `{"capture":null,"items":[{"id":"a"}],"nested":{"id":"x"},"count":1,"name":"n"}`, want: base},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			var got sampleStrict
			if err := decodeStrict([]byte(tc.input), &got); err != nil {
				t.Fatalf("%s: decodeStrict: %v", tc.name, err)
			}
			if got.Name != tc.want.Name || got.Count != tc.want.Count || got.Nested != tc.want.Nested ||
				!slices.Equal(got.Items, tc.want.Items) || got.Label != tc.want.Label || got.Limit != tc.want.Limit ||
				got.Capture != tc.want.Capture || got.Plain != tc.want.Plain || got.Retries != tc.want.Retries {
				t.Fatalf("%s: decodeStrict = %+v, want %+v", tc.name, got, tc.want)
			}
		})
	}
}

func TestTextcodec_DecodeStrictRejectsWhatTheRustDecoderRejects(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
	}{
		{name: "missing required member", input: `{"count":1,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "required member null", input: `{"name":null,"count":1,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "missing nested required member", input: `{"name":"n","count":1,"nested":{},"items":[],"capture":null}`},
		{name: "missing required member in an array element", input: `{"name":"n","count":1,"nested":{"id":"x"},"items":[{}],"capture":null}`},
		{name: "unknown member", input: `{` + strictRequired + `,"extra":1}`},
		{name: "unknown nested member", input: `{"name":"n","count":1,"nested":{"id":"x","extra":1},"items":[],"capture":null}`},
		{name: "case variant of a member", input: `{"Name":"n","count":1,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "case variant in an array element", input: `{"name":"n","count":1,"nested":{"id":"x"},"items":[{"ID":"a"}],"capture":null}`},
		{name: "duplicate member", input: `{` + strictRequired + `,"name":"m"}`},
		{name: "invalid UTF-8", input: "{\"name\":\"\xff\",\"count\":1,\"nested\":{\"id\":\"x\"},\"items\":[],\"capture\":null}"},
		{name: "trailing data", input: `{` + strictRequired + `} {}`},
		{name: "present member null", input: `{` + strictRequired + `,"limit":null}`},
		{name: "nullable member missing", input: `{"name":"n","count":1,"nested":{"id":"x"},"items":[]}`},
		{name: "defaulted member null", input: `{` + strictRequired + `,"retries":null}`},
		{name: "negative unsigned integer", input: `{"name":"n","count":-1,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "unsigned integer overflow", input: `{"name":"n","count":18446744073709551616,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "fractional integer", input: `{"name":"n","count":1.0,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "wrong type", input: `{"name":1,"count":1,"nested":{"id":"x"},"items":[],"capture":null}`},
		{name: "not an object", input: `[]`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			var got sampleStrict
			if err := decodeStrict([]byte(tc.input), &got); err == nil {
				t.Fatalf("%s: decodeStrict accepted %+v", tc.name, got)
			}
		})
	}
}

type sampleColour interface {
	isSampleColour()
}

type sampleColourIndexed struct {
	Index uint8 `json:"index"`
}

type sampleColourRGB struct {
	R uint8 `json:"r"`
	G uint8 `json:"g"`
	B uint8 `json:"b"`
}

func (*sampleColourIndexed) isSampleColour() {}
func (*sampleColourRGB) isSampleColour()     {}

var sampleColourVariants = map[string]func() sampleColour{
	"indexed": func() sampleColour { return &sampleColourIndexed{} },
	"rgb":     func() sampleColour { return &sampleColourRGB{} },
}

func TestTextcodec_DecodeTaggedDispatchesOnTheTagMember(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  sampleColour
	}{
		{name: "tag first", input: `{"kind":"indexed","index":7}`, want: &sampleColourIndexed{Index: 7}},
		{name: "tag last", input: `{"index":7,"kind":"indexed"}`, want: &sampleColourIndexed{Index: 7}},
		{name: "other variant", input: `{"kind":"rgb","r":1,"g":2,"b":3}`, want: &sampleColourRGB{R: 1, G: 2, B: 3}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := decodeTagged([]byte(tc.input), "kind", sampleColourVariants)
			if err != nil {
				t.Fatalf("%s: decodeTagged: %v", tc.name, err)
			}
			switch want := tc.want.(type) {
			case *sampleColourIndexed:
				if value, ok := got.(*sampleColourIndexed); !ok || *value != *want {
					t.Fatalf("%s: decodeTagged = %#v, want %#v", tc.name, got, want)
				}
			case *sampleColourRGB:
				if value, ok := got.(*sampleColourRGB); !ok || *value != *want {
					t.Fatalf("%s: decodeTagged = %#v, want %#v", tc.name, got, want)
				}
			default:
				t.Fatalf("%s: unexpected case type %T", tc.name, want)
			}
		})
	}
}

func TestTextcodec_DecodeTaggedRejectsAMissingUnknownOrMalformedTag(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
	}{
		{name: "missing tag", input: `{"index":7}`},
		{name: "unknown tag", input: `{"kind":"hsl","index":7}`},
		{name: "case variant of the tag value", input: `{"kind":"Indexed","index":7}`},
		{name: "null tag", input: `{"kind":null,"index":7}`},
		{name: "duplicate tag", input: `{"kind":"indexed","kind":"rgb","index":7}`},
		{name: "unknown member of the variant", input: `{"kind":"indexed","index":7,"r":1}`},
		{name: "missing member of the variant", input: `{"kind":"rgb","r":1,"g":2}`},
		{name: "integer out of the field's range", input: `{"kind":"indexed","index":256}`},
		{name: "not an object", input: `"indexed"`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got, err := decodeTagged([]byte(tc.input), "kind", sampleColourVariants); err == nil {
				t.Fatalf("%s: decodeTagged accepted %#v", tc.name, got)
			}
		})
	}
}

func TestTextcodec_LossyUtf8ReplacesEachMaximalIllFormedSubsequence(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  string
	}{
		{name: "valid ASCII", input: "abc", want: "abc"},
		{name: "valid multibyte", input: "a\xe2\x98\xbab", want: "a☺b"},
		{name: "lone invalid byte", input: "\xff", want: "\ufffd"},
		{name: "two invalid bytes", input: "\xff\xfe", want: "\ufffd\ufffd"},
		{name: "truncated three-byte sequence", input: "\xe2\x98", want: "\ufffd"},
		{name: "truncated sequence before ASCII", input: "\xe2\x98a", want: "\ufffda"},
		{name: "truncated four-byte sequence", input: "\xf0\x9f\x98a", want: "\ufffda"},
		{name: "surrogate", input: "\xed\xa0\x80", want: "\ufffd\ufffd\ufffd"},
		{name: "overlong two-byte", input: "\xc0\xaf", want: "\ufffd\ufffd"},
		{name: "above U+10FFFF", input: "\xf4\x90\x80\x80", want: "\ufffd\ufffd\ufffd\ufffd"},
		{name: "overlong three-byte", input: "\xe0\x80\x80", want: "\ufffd\ufffd\ufffd"},
		{name: "invalid second byte of a four-byte lead", input: "\xf0\x80", want: "\ufffd\ufffd"},
		{name: "truncated sequence before an invalid byte", input: "\xe2\x98\xff", want: "\ufffd\ufffd"},
		{name: "continuation bytes", input: "\x80\x80", want: "\ufffd\ufffd"},
		{name: "five-byte lead", input: "\xf8\x88\x80\x80\x80", want: "\ufffd\ufffd\ufffd\ufffd\ufffd"},
		{name: "empty", input: "", want: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := lossyUTF8([]byte(tc.input)); got != tc.want {
				t.Fatalf("%s: lossyUTF8 = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestTextcodec_LinesSplitsLikeRustStrLines(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  []string
	}{
		{name: "empty", input: "", want: []string{}},
		{name: "one line", input: "a", want: []string{"a"}},
		{name: "trailing newline", input: "a\n", want: []string{"a"}},
		{name: "trailing empty line", input: "a\n\n", want: []string{"a", ""}},
		{name: "CRLF", input: "a\r\nb", want: []string{"a", "b"}},
		{name: "only one CR stripped", input: "a\r\r\n", want: []string{"a\r"}},
		{name: "bare CR inside a line", input: "a\rb", want: []string{"a\rb"}},
		{name: "newline only", input: "\n", want: []string{""}},
		{name: "trailing CR without newline", input: "a\r", want: []string{"a\r"}},
		{name: "CRLF only", input: "\r\n", want: []string{""}},
		{name: "CR after a newline", input: "a\n\rb", want: []string{"a", "\rb"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := lines(tc.input); !slices.Equal(got, tc.want) {
				t.Fatalf("%s: lines = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestTextcodec_JoinPathAppendsWithoutCleaning(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		base string
		elem string
		want string
	}{
		{name: "plain", base: "/srv/x", elem: "y", want: "/srv/x/y"},
		{name: "base with trailing slash", base: "/srv/x/", elem: "y", want: "/srv/x/y"},
		{name: "double slash and dot kept", base: "/srv//x", elem: "./y", want: "/srv//x/./y"},
		{name: "parent kept", base: "/srv/x", elem: "../y", want: "/srv/x/../y"},
		{name: "empty base", base: "", elem: "y", want: "y"},
		{name: "absolute element replaces", base: "/srv/x", elem: "/abs", want: "/abs"},
		{name: "empty element adds a separator", base: "/srv/x", elem: "", want: "/srv/x/"},
		{name: "relative base and trailing slash", base: "relative", elem: "y/z/", want: "relative/y/z/"},
		{name: "trailing dot kept", base: "/srv/x/.", elem: "y", want: "/srv/x/./y"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := joinPath(tc.base, tc.elem); got != tc.want {
				t.Fatalf("%s: joinPath(%q, %q) = %q, want %q", tc.name, tc.base, tc.elem, got, tc.want)
			}
		})
	}
}

func TestTextcodec_ParseRfc3339AcceptsWhatChronoAccepts(t *testing.T) {
	t.Parallel()
	zone := func(offsetSeconds int) *time.Location { return time.FixedZone("", offsetSeconds) }
	cases := []struct {
		name   string
		input  string
		want   time.Time
		offset int
	}{
		{name: "numeric UTC offset", input: "2026-07-30T09:53:43+00:00", want: time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)},
		{name: "Z", input: "2026-07-30T09:53:43Z", want: time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)},
		{name: "lowercase separator and z", input: "2026-07-30t09:53:43z", want: time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)},
		{name: "space separator", input: "2026-07-30 09:53:43+00:00", want: time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)},
		{name: "one fractional digit", input: "2026-07-30T09:53:43.1+09:00", want: time.Date(2026, 7, 30, 9, 53, 43, 100_000_000, zone(9*3600)), offset: 9 * 3600},
		{name: "nine fractional digits", input: "2026-07-30T09:53:43.123456789-05:30", want: time.Date(2026, 7, 30, 9, 53, 43, 123_456_789, zone(-(5*3600 + 1800))), offset: -(5*3600 + 1800)},
		{name: "ten fractional digits truncate", input: "2026-07-30T09:53:43.1234567891Z", want: time.Date(2026, 7, 30, 9, 53, 43, 123_456_789, time.UTC)},
		{name: "negative zero offset", input: "2026-07-30T09:53:43-00:00", want: time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)},
		{name: "largest offset", input: "2026-07-30T09:53:43+23:59", want: time.Date(2026, 7, 30, 9, 53, 43, 0, zone(23*3600+59*60)), offset: 23*3600 + 59*60},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, ok := parseRFC3339(tc.input)
			if !ok || !got.Equal(tc.want) {
				t.Fatalf("%s: parseRFC3339 = %v %v, want %v", tc.name, got, ok, tc.want)
			}
			if _, offset := got.Zone(); offset != tc.offset {
				t.Fatalf("%s: offset = %d, want %d", tc.name, offset, tc.offset)
			}
		})
	}
}

func TestTextcodec_ParseRfc3339AcceptsALeapSecond(t *testing.T) {
	t.Parallel()
	last := time.Date(2026, 6, 30, 23, 59, 59, 0, time.UTC)
	next := time.Date(2026, 7, 1, 0, 0, 0, 0, time.UTC)
	for _, input := range []string{"2026-06-30T23:59:60Z", "2026-06-30T23:59:60.5Z"} {
		got, ok := parseRFC3339(input)
		if !ok || got.Before(last) || got.After(next) {
			t.Fatalf("%s: parseRFC3339 = %v %v, want an instant in [%v, %v]", input, got, ok, last, next)
		}
	}
}

func TestTextcodec_ParseRfc3339RejectsWhatChronoRejects(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
	}{
		{name: "no offset", input: "2026-07-30T09:53:43"},
		{name: "offset without colon", input: "2026-07-30T09:53:43+0000"},
		{name: "offset without minutes", input: "2026-07-30T09:53:43+00"},
		{name: "space before the offset", input: "2026-07-30T09:53:43 +00:00"},
		{name: "one-digit hour", input: "2026-07-30T9:53:43Z"},
		{name: "empty fraction", input: "2026-07-30T09:53:43.Z"},
		{name: "hour 24", input: "2026-07-30T24:00:00Z"},
		{name: "February 30", input: "2026-02-30T09:53:43Z"},
		{name: "offset 24 hours", input: "2026-07-30T09:53:43+24:00"},
		{name: "trailing space", input: "2026-07-30T09:53:43z "},
		{name: "leading space", input: " 2026-07-30T09:53:43Z"},
		{name: "prose", input: "yesterday"},
		{name: "empty", input: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got, ok := parseRFC3339(tc.input); ok {
				t.Fatalf("%s: parseRFC3339(%q) = %v", tc.name, tc.input, got)
			}
		})
	}
}

func TestTextcodec_OsErrorTextCapitalizesThePlatformDescriptionAndAppendsTheNumber(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		errno syscall.Errno
		want  string
	}{
		{name: "ENOENT", errno: syscall.ENOENT, want: "No such file or directory (os error 2)"},
		{name: "EACCES", errno: syscall.EACCES, want: "Permission denied (os error 13)"},
		{name: "EEXIST", errno: syscall.EEXIST, want: "File exists (os error 17)"},
		{name: "ENOTDIR", errno: syscall.ENOTDIR, want: "Not a directory (os error 20)"},
		{name: "EPIPE", errno: syscall.EPIPE, want: "Broken pipe (os error 32)"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := osErrorText(tc.errno); got != tc.want {
				t.Fatalf("%s: osErrorText = %q, want %q", tc.name, got, tc.want)
			}
			wrapped := &os.PathError{Op: "open", Path: "/srv/x", Err: tc.errno}
			if got := ioErrorText(wrapped); got != tc.want {
				t.Fatalf("%s: ioErrorText(%v) = %q, want %q", tc.name, wrapped, got, tc.want)
			}
		})
	}
	if got := ioErrorText(errors.New("output could not be read")); got != "output could not be read" {
		t.Fatalf("a non-OS cause: ioErrorText = %q", got)
	}
}

func TestTextcodec_ExitStatusTextRendersCodesAndSignals(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		status syscall.WaitStatus
		want   string
	}{
		{name: "success", status: 0, want: "exit status: 0"},
		{name: "exit code", status: 3 << 8, want: "exit status: 3"},
		{name: "largest exit code", status: 255 << 8, want: "exit status: 255"},
		{name: "SIGTERM", status: syscall.WaitStatus(syscall.SIGTERM), want: "signal: 15 (SIGTERM)"},
		{name: "SIGKILL", status: syscall.WaitStatus(syscall.SIGKILL), want: "signal: 9 (SIGKILL)"},
		{name: "core dump", status: 0x80 | syscall.WaitStatus(syscall.SIGSEGV), want: "signal: 11 (SIGSEGV) (core dumped)"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := exitStatusText(tc.status); got != tc.want {
				t.Fatalf("%s: exitStatusText = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}
