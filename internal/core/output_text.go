package core

import (
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// quoted renders s as Rust's str Debug does (F9, SPEC §6.4 Quoted text): wrapped in `"`, with
// `"`, `\`, tab, newline, carriage return and NUL escaped by name and every scalar that is not
// printable or extends a grapheme escaped as \u{H}.
func quoted(s string) string {
	if !utf8.ValidString(s) {
		panic("quoted text is valid UTF-8")
	}
	var out strings.Builder
	out.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			out.WriteString(`\"`)
		case '\\':
			out.WriteString(`\\`)
		case '\t':
			out.WriteString(`\t`)
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case 0:
			out.WriteString(`\0`)
		default:
			if quotedEscapes(r) {
				out.WriteString(`\u{` + strconv.FormatInt(int64(r), 16) + `}`)
			} else {
				out.WriteRune(r)
			}
		}
	}
	out.WriteByte('"')
	return out.String()
}

// quotedEscapes holds for a scalar of general category Cc, Cf, Cs, Co, Cn, Zl, Zp or Zs other
// than U+0020, and for one with the Grapheme_Extend property (Mn, Me and
// Other_Grapheme_Extend).
func quotedEscapes(r rune) bool {
	switch {
	case r == ' ':
		return false
	case unicode.In(r, unicode.Cc, unicode.Cf, unicode.Cs, unicode.Co, unicode.Zl, unicode.Zp, unicode.Zs):
		return true
	case unicode.In(r, unicode.Mn, unicode.Me, unicode.Other_Grapheme_Extend):
		return true
	default:
		assigned := unicode.In(r, unicode.L, unicode.M, unicode.N, unicode.P, unicode.S, unicode.Z,
			unicode.Cc, unicode.Cf, unicode.Co, unicode.Cs)
		return !assigned
	}
}
