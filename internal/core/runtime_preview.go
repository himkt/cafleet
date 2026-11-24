package core

import (
	"fmt"
	"strings"
	"unicode/utf8"
)

const (
	inlineBodyLimit     = 4000
	resumeClause        = "[cafleet] Resume your work if something was still running."
	dismissedPromptNote = "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it."
)

// foldVisible renders text on one line: each line break (CRLF, CR or LF) as ⏎, a tab as ␉ and
// every other control character as \xHH.
func foldVisible(text string) string {
	var result strings.Builder
	runes := []rune(text)
	for i := 0; i < len(runes); i++ {
		switch r := runes[i]; {
		case r == '\r':
			if i+1 < len(runes) && runes[i+1] == '\n' {
				i++
			}
			result.WriteRune('⏎')
		case r == '\n':
			result.WriteRune('⏎')
		case r == '\t':
			result.WriteRune('␉')
		case r <= 0x1f || r == 0x7f:
			fmt.Fprintf(&result, "\\x%02x", r)
		default:
			result.WriteRune(r)
		}
	}
	return result.String()
}

func fallbackPreviewLine(canonical string) string {
	return canonical + " " + dismissedPromptNote
}

func canonicalPreviewLine(message messageRecord) string {
	body := foldVisible(message.text)
	if utf8.RuneCountInString(body) > inlineBodyLimit {
		body = bodyPointer(message)
	}
	return fmt.Sprintf("[cafleet msg %d from %d %s] %s %s", message.messageID, message.fromMemberID, message.createdAt, body, resumeClause)
}

func pointerPreviewLine(message messageRecord) string {
	return fmt.Sprintf("[cafleet msg %d from %d %s] %s %s", message.messageID, message.fromMemberID, message.createdAt, bodyPointer(message), resumeClause)
}

func bodyPointer(message messageRecord) string {
	return fmt.Sprintf("Body is %d characters; run %s to read it.", utf8.RuneCountInString(message.text), pollCommand(message.toMemberID, none[string]()))
}
