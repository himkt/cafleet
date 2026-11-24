package core

import "testing"

func TestOutputText_QuotedEscapesLikeRustStrDebug(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  string
	}{
		{name: "printable text and apostrophes", input: "plain 'quote' é ☺", want: `"plain 'quote' é ☺"`},
		{name: "named escapes", input: "\"\\\t\n\r\x00", want: `"\"\\\t\n\r\0"`},
		{name: "other controls", input: "\x01\x1f\x7f\u0085", want: `"\u{1}\u{1f}\u{7f}\u{85}"`},
		{name: "separators and format characters", input: "\u00a0\u2028\u2029\u3000\u200b\ufeff\u00ad\u200d\U000e0020", want: `"\u{a0}\u{2028}\u{2029}\u{3000}\u{200b}\u{feff}\u{ad}\u{200d}\u{e0020}"`},
		{name: "space stays raw", input: "a b", want: `"a b"`},
		{name: "grapheme extend after a base", input: "e\u0301", want: `"e\u{301}"`},
		{name: "grapheme extend alone", input: "\u0301", want: `"\u{301}"`},
		{name: "variation selector", input: "😀\ufe0f", want: `"😀\u{fe0f}"`},
		{name: "private use and unassigned", input: "\ue000\u0378\U0010ffff", want: `"\u{e000}\u{378}\u{10ffff}"`},
		{name: "empty", input: "", want: `""`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := quoted(tc.input); got != tc.want {
				t.Fatalf("%s: quoted = %s, want %s", tc.name, got, tc.want)
			}
		})
	}
}
