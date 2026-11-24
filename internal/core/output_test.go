package core

import "testing"

func TestOutput_CaptureCleanupRemovesCsiAndKeepsTheFinalCarriageReturnSegment(t *testing.T) {
	t.Parallel()
	stripCases := []struct {
		name string
		raw  string
		want string
	}{
		{name: "empty", raw: "", want: ""},
		{name: "plain lines", raw: "plain\ntext", want: "plain\ntext"},
		{name: "sgr colour", raw: "\x1b[31mred\x1b[0m", want: "red"},
		{name: "256-colour sgr", raw: "\x1b[38;5;196mX\x1b[0m", want: "X"},
		{name: "clear and home", raw: "\x1b[2J\x1b[Hclear", want: "clear"},
		{name: "redraw per line", raw: "one\rONE\ntwo\rTWO", want: "ONE\nTWO"},
		{name: "last redraw wins", raw: "a\rb\rc", want: "c"},
		{name: "erase line then redraw", raw: "\x1b[2Kspinner\rdone", want: "done"},
		{name: "osc stays", raw: "\x1b]0;title\x07 done", want: "\x1b]0;title\x07 done"},
		{name: "crlf lines", raw: "one\r\ntwo\r\nthree", want: "one\ntwo\nthree"},
		{name: "trailing carriage return after crlf", raw: "row1\r\nrow2\r", want: "row1\nrow2"},
		{name: "single trailing carriage return", raw: "row\r", want: "row"},
		{name: "run of carriage returns before line feed", raw: "row\r\r\n", want: "row\n"},
		{name: "redraw then crlf", raw: "spin\rDONE\r\nnext", want: "DONE\nnext"},
		{name: "csi before crlf", raw: "kept\x1b[0m\r\n", want: "kept\n"},
	}
	for _, tc := range stripCases {
		t.Run("strip "+tc.name, func(t *testing.T) {
			t.Parallel()
			if got := stripAnsi(tc.raw); got != tc.want {
				t.Fatalf("%s: stripAnsi(%q) = %q, want %q", tc.name, tc.raw, got, tc.want)
			}
		})
	}
	blankCases := []struct {
		name  string
		line  string
		blank bool
	}{
		{name: "styled whitespace", line: "\x1b[31m \t\x1b[0m", blank: true},
		{name: "styled text", line: "\x1b[31mX\x1b[0m", blank: false},
	}
	for _, tc := range blankCases {
		t.Run("blank "+tc.name, func(t *testing.T) {
			t.Parallel()
			if got := isVisuallyBlank(tc.line); got != tc.blank {
				t.Fatalf("%s: isVisuallyBlank(%q) = %v, want %v", tc.name, tc.line, got, tc.blank)
			}
		})
	}
}

func TestOutput_TextLimitCountsUnicodeCodepointsAndPreservesExactLimits(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		text  string
		limit int
		want  string
	}{
		{name: "empty at zero", text: "", limit: 0, want: ""},
		{name: "one scalar at zero", text: "a", limit: 0, want: "…"},
		{name: "exact limit", text: "abc", limit: 3, want: "abc"},
		{name: "multibyte scalars count once", text: "界🙂abc", limit: 2, want: "界🙂…"},
		{name: "one over", text: "hello", limit: 4, want: "hell…"},
		{name: "far under", text: "a", limit: 20, want: "a"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := truncateText(tc.text, tc.limit); got != tc.want {
				t.Fatalf("%s: truncateText(%q, %d) = %q, want %q", tc.name, tc.text, tc.limit, got, tc.want)
			}
		})
	}
}
