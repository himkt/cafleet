package core

import "strings"

func validInput(text string) bool {
	return strings.TrimSpace(text) != "" && !strings.ContainsFunc(text, func(r rune) bool {
		return r <= 0x1f || r == 0x7f
	})
}
