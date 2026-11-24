package core

import (
	"fmt"
	"strings"
	"testing"
)

const previewTestClauses = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it."

func previewRecord(text string) messageRecord {
	return messageRecordFixture(57, 15, 16, text)
}

func previewPointer(stored int) string {
	return fmt.Sprintf("Body is %d characters; run cafleet message poll 16 --json to read it.", stored)
}

func TestRuntimePreview_ThePreviewLineFoldsItsBodyInlineUpToTheLimitAndPointsAtThePollBeyondIt(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		text string
		body string
	}{
		{name: "the specified example", text: "ready (doc)\nsee the reviewer notes", body: "ready (doc)⏎see the reviewer notes"},
		{name: "line breaks, tab and C0/DEL controls fold visibly", text: "a\r\nb\nc\rd\tx\x1b\x7f", body: "a⏎b⏎c⏎d␉x\\x1b\\x7f"},
		{name: "4000 multibyte scalars stay inline", text: strings.Repeat("é", 4000), body: strings.Repeat("é", 4000)},
		{name: "4001 multibyte scalars become the pointer", text: strings.Repeat("é", 4001), body: previewPointer(4001)},
		{name: "8000 stored characters fold to 4000 and stay inline", text: strings.Repeat("\r\n", 4000), body: strings.Repeat("⏎", 4000)},
		{name: "1001 stored controls fold to 4004 and the pointer counts the stored text", text: strings.Repeat("\x01", 1001), body: previewPointer(1001)},
	}
	for _, tc := range cases {
		line := fallbackPreviewLine(canonicalPreviewLine(previewRecord(tc.text)))
		want := fmt.Sprintf("[cafleet msg 57 from 15 %s] %s %s", messageRecordFixtureStamp, tc.body, previewTestClauses)
		if line != want {
			t.Fatalf("%s: preview line\n%q\nwant\n%q", tc.name, line, want)
		}
		if !validInput(line) {
			t.Fatalf("%s: the preview line is not valid pane input: %q", tc.name, line)
		}
	}
	pointerLine := pointerPreviewLine(previewRecord("短い本文"))
	want := fmt.Sprintf("[cafleet msg 57 from 15 %s] %s [cafleet] Resume your work if something was still running.", messageRecordFixtureStamp, previewPointer(4))
	if pointerLine != want {
		t.Fatalf("the pointer line names the recipient's poll:\n%q\nwant\n%q", pointerLine, want)
	}
}
