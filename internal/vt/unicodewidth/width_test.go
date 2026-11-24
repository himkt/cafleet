// Transliterated from unicode-width 0.2.2 tests/tests.rs, non-cjk assertions only (MIT OR Apache-2.0,
// used under MIT: Copyright (c) 2015 The Rust Project Developers).

package unicodewidth

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"os"
	"testing"
	"unicode/utf8"
)

type strCase struct {
	s    string
	want uint
}

type charCase struct {
	c    rune
	want int
	ok   bool
}

func checkStrs(t *testing.T, cases []strCase) {
	t.Helper()
	for _, tc := range cases {
		if got := StrWidth([]byte(tc.s)); got != tc.want {
			t.Errorf("%+q has width %d, want %d", tc.s, got, tc.want)
		}
	}
}

func checkChars(t *testing.T, cases []charCase) {
	t.Helper()
	for _, tc := range cases {
		got, ok := CharWidth(tc.c)
		if ok != tc.ok || (ok && got != tc.want) {
			t.Errorf("U+%04X has width (%d, %v), want (%d, %v)", tc.c, got, ok, tc.want, tc.ok)
		}
	}
}

func someCase(c rune, w int) charCase { return charCase{c: c, want: w, ok: true} }

func noneCase(c rune) charCase { return charCase{c: c} }

// widthOrOne is Rust's `c.width().unwrap_or(1)`.
func widthOrOne(c rune) uint {
	w, ok := CharWidth(c)
	if !ok {
		return 1
	}
	return uint(w)
}

func scalarValues(yield func(rune) bool) {
	for c := rune(0); c <= utf8.MaxRune; c++ {
		if c >= 0xD800 && c <= 0xDFFF {
			continue
		}
		if !yield(c) {
			return
		}
	}
}

func TestWidth_Str(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"ｈｅｌｌｏ", 10}, {"\x00\x00\x00\x01\x01", 5}, {"", 0}, {"₁₂₃₄", 4}})
}

func TestWidth_Emoji(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"👩", 2}, {"🔬", 2}, {"👩\u200D🔬", 2}})
}

func TestWidth_BadDevanagari(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"क", 1}, {"ष", 1}, {"क\u094Dष", 2}})
}

func TestWidth_Char(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase('ｈ', 2), noneCase(0x00), noneCase(0x01), someCase(0x2081, 1)})
}

func TestWidth_Char2(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{noneCase(0x0A), someCase('w', 1), someCase('ｈ', 2), someCase(0xAD, 0), someCase(0x1160, 0), someCase(0xA1, 1), someCase(0x300, 0)})
}

func TestWidth_Unicode12(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x1F971, 2)})
}

func TestWidth_DefaultIgnorable(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x1160, 0), someCase(0x3164, 0), someCase(0xFFA0, 0), someCase(0xE0000, 0)})
}

func TestWidth_Ambiguous(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"·", 1}, {"·", 1}, {"¨", 1}, {"ˉ", 1}})
}

func TestWidth_Jamo(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x1100, 2), someCase(0xA97C, 2), someCase(0x115F, 2), someCase(0x1160, 0), someCase(0xD7C6, 0), someCase(0x11A8, 0), someCase(0xD7FB, 0)})
}

func TestWidth_PrependedConcatenationMarks(t *testing.T) {
	t.Parallel()
	var cases []charCase
	for _, c := range []rune{0x0600, 0x0601, 0x0602, 0x0603, 0x0604, 0x06DD, 0x110BD, 0x110CD} {
		cases = append(cases, someCase(c, 1))
	}
	for _, c := range []rune{0x0605, 0x070F, 0x0890, 0x0891, 0x08E2} {
		cases = append(cases, someCase(c, 0))
	}
	checkChars(t, cases)
}

func TestWidth_GcbPrepend(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"ൎഉ", 1}, {"\U00011A89", 0}})
}

func TestWidth_InterlinearAnnotationChars(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0xFFF9, 1), someCase(0xFFFA, 1), someCase(0xFFFB, 1)})
}

func TestWidth_HieroglyphFormatControls(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x13430, 1), someCase(0x13436, 1), someCase(0x1343C, 1)})
}

func TestWidth_Marks(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x0301, 0), someCase(0x20DD, 0), someCase(0x09CB, 1), someCase(0x09BE, 0)})
}

func TestWidth_DevanagariCaret(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0xA8FA, 0)})
}

func TestWidth_SolidusOverlay(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"<\u0338", 1}, {"=\u0338", 1}, {">\u0338", 1}, {"=\u0301\u0338", 1}, {"=\u0338\u0301", 1},
		{"=\uFE0F\u0338", 1}, {"#\uFE0F\u0338", 2}, {"#\u0338\uFE0F", 1},
		{"ڸ\u0338ا", 1}, {"ڸ\u0338\uFE0Eا", 1}, {"ڸ\u0338\uFE0Fا", 1},
		{"ڸ\uFE0E\u0338ا", 1}, {"ڸ\uFE0F\u0338ا", 1},
		{"=\u0338ا", 2},
	})
}

func TestWidth_EmojiPresentation(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x0023, 1), someCase(0xFE0F, 0)})
	checkStrs(t, []strCase{
		{"#\uFE0F", 2}, {"a#\uFE0Fa", 4}, {"#a\uFE0F", 2}, {"a\uFE0F", 1},
		{"##\uFE0Fa", 4}, {"*\uFE0F", 2}, {"⏹\uFE0F", 2}, {"Ⓜ\uFE0F", 2},
		{"\U0001F6F3\uFE0F", 2}, {"\U0001F700\uFE0F", 1}, {"*\u0301\uFE0F", 1},
		{"*\u200D\uFE0F", 1}, {"*\uFE0E\uFE0F", 1},
	})
}

func TestWidth_TextPresentation(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0xFE0E, 0), someCase(0x2648, 2)})
	checkStrs(t, []strCase{
		{"♈\uFE0E", 1}, {"♈\uFE0E\uFE0F", 1}, {"♈\uFE0F\uFE0E", 2}, {"\U0001F21A\uFE0E", 2},
		{"\u0301\uFE0E", 0}, {"a\uFE0E", 1}, {"𘀀\uFE0E", 2}, {"♈\u0301\uFE0E", 2}, {"♈\u200D\uFE0E", 2},
	})
}

func TestWidth_ControlLineBreak(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0x2028, 1), someCase(0x2029, 1), noneCase('\r'), noneCase('\n')})
	checkStrs(t, []strCase{
		{"\r", 1}, {"\n", 1}, {"\r\n", 1}, {"\x00", 1}, {"1\t2\r\n3\u00854", 7}, {"\r\uFE0F\n", 2}, {"\r\u200D\n", 2},
	})
}

func TestWidth_CharStrConsistent(t *testing.T) {
	t.Parallel()
	for c := range scalarValues {
		if got := StrWidth([]byte(string(c))); got != widthOrOne(c) {
			t.Fatalf("U+%04X: string width %d, char width or one %d", c, got, widthOrOne(c))
		}
	}
}

func TestWidth_LisuTones(t *testing.T) {
	t.Parallel()
	var chars []charCase
	var strs []strCase
	for c := rune(0xA4F8); c <= 0xA4FD; c++ {
		chars = append(chars, someCase(c, 1))
		strs = append(strs, strCase{string(c), 1})
	}
	for c1 := rune(0xA4F8); c1 <= 0xA4FD; c1++ {
		for c2 := rune(0xA4F8); c2 <= 0xA4FD; c2++ {
			want := uint(2)
			if c1 <= 0xA4FB && c2 >= 0xA4FC {
				want = 1
			}
			strs = append(strs, strCase{string([]rune{c1, c2}), want})
		}
	}
	strs = append(strs, []strCase{
		{"ꓪꓹ", 2}, {"ꓪꓹꓼ", 2}, {"ꓪꓹ\uFE0Fꓼ", 2}, {"ꓪꓹ\u200Dꓼ", 2}, {"ꓪꓹꓼ\uFE0F", 2}, {"ꓪꓹ\u0301ꓼ", 3}, {"ꓪꓹꓹ", 3}, {"ꓪꓼꓼ", 3},
	}...)
	checkChars(t, chars)
	checkStrs(t, strs)
}

func TestWidth_HebrewAlefLamed(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"א", 1}, {"ל", 1}, {"אל", 2}, {"א\u200Dל", 1},
		{"א\u200D\u200D\u200D\u200D\u200D\u200D\u200Dל", 1}, {"אא\u200Dל", 2},
		{"אא\u200D\u200D\u200D\u200D\u200D\u200Dל", 2},
		{"א\uFE0F\u200D\uFE0Fל\uFE0F", 1}, {"א\uFE0E\u200D\uFE0Eל\uFE0E", 1},
	})
}

func TestWidth_ArabicLamAlef(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"ل", 1}, {"ڸ", 1}, {"أ", 1}, {"ا", 1},
		{"لأ", 1}, {"لا", 1}, {"ڸأ", 1}, {"ڸا", 1},
		{"ل\u065F\u065Eأ", 1}, {"ل\u065F\u065Eا", 1}, {"ڸ\u065F\u065Eأ", 1}, {"ڸ\u065F\u065Eا", 1},
		{"ڸ\uFE0Eا", 1}, {"ڸ\uFE0Fا", 1}, {"ڸ\u17B5ا", 1},
		{"للأ", 2}, {"للا", 2}, {"ڸڸأ", 2}, {"ڸڸا", 2},
		{"ل\u200Dأ", 2}, {"ل\u200Dا", 2}, {"ڸ\u200Dأ", 2}, {"ڸ\u200Dا", 2},
		{"ل\U0001E94Bأ", 3}, {"ل\U0001E94Bا", 3}, {"ڸ\U0001E94Bأ", 3}, {"ڸ\U0001E94Bا", 3},
	})
}

func TestWidth_BugineseAIYa(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"ᨕ", 1}, {"\u1A17", 0}, {"ᨐ", 1},
		{"ᨕ\u1A17\u200Dᨐ", 1}, {"ᨕ\u1A17\u200D\u200D\u200D\u200Dᨐ", 1},
		{"ᨕ\u1A17\u200D\u0338", 1}, {"ᨕ\uFE0E\u1A17\u200D", 1}, {"ᨕ\uFE0F\u1A17\u200D", 1},
		{"ᨕ\u1A17\uFE0E\u200D", 1}, {"ᨕ\u1A17\uFE0F\u200D", 1}, {"ᨕ\u1A17\u200D\uFE0E", 1},
		{"ᨕ\u1A17\u200D\uFE0F", 1}, {"ᨕ\u17B5\u200D\uFE0E\u1A17\u200D\uFE0F\u200D\uFE0F", 1},
		{"ᨕᨕ\u1A17\u200Dᨐ", 2}, {"ᨕᨕ\u1A17\u200D\u200D\u200D\u200Dᨐ", 2},
		{"ᨕ\u1A17ᨐ", 2}, {"ᨕ\u200Dᨐ", 2}, {"ᨕᨐ", 2},
		{"ᨕ\u1A17\u1A17\u200Dᨐ", 2}, {"ᨕ\u1A17\u0338\u200Dᨐ", 2},
	})
}

func TestWidth_TifinaghBiconsonants(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"ⵏ", 1}, {"ⴾ", 1}, {"\u2D7F", 1},
		{"ⵏ\u200Dⴾ", 1}, {"ⵏ\u2D7Fⴾ", 1}, {"ⵏ\u200Dⴾ", 1},
		{"ⵏ\uFE0F\u200D\u2D7F\uFE0E\u200D\u17B5ⴾ", 1},
		{"ⵏ\u0301\u2D7Fⴾ", 3}, {"ⵏ\u0301\u200Dⴾ", 2}, {"ⵏⴾ", 2},
		{"ⵏ\u2D7F\u2D7Fⴾ", 4}, {"\u2D7Fⴾ", 2}, {"\u2D7F\u2D7Fⵦ", 3}, {"ⵦ\u2D7Fⴾ", 3},
	})
}

func TestWidth_OldTurkicLigature(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{
		{"\U00010C32", 1}, {"\U00010C03", 1}, {"\U00010C32\U00010C03", 2},
		{"\U00010C32\u200D\U00010C03", 1}, {"\U00010C32\uFE0F\u200D\uFE0E\U00010C03", 1},
		{"\U00010C32\u2D7F\U00010C03", 3}, {"\U00010C32\u0301\u200D\U00010C03", 2},
		{"\U00010C03\u200D\U00010C32", 2}, {"\u200D\U00010C32", 1},
	})
}

func TestWidth_KhmerCoeng(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"ល", 1}, {"ង", 1}, {"លង", 2}, {"ល\u17D2ង", 1}})
	ligating := func(c rune) bool {
		return (c >= 0x1780 && c <= 0x1782) || (c >= 0x1784 && c <= 0x1787) || (c >= 0x1789 && c <= 0x178C) ||
			(c >= 0x178E && c <= 0x1793) || (c >= 0x1795 && c <= 0x1798) || (c >= 0x179B && c <= 0x179D) ||
			c == 0x17A0 || c == 0x17A2 || c == 0x17A7 || (c >= 0x17AB && c <= 0x17AC) || c == 0x17AF
	}
	for c := range scalarValues {
		coeng, zwj := StrWidth([]byte("\u17D2"+string(c))), StrWidth([]byte("\u17D2\u200D\u200D"+string(c)))
		if ligating(c) {
			if coeng != 0 || zwj != 0 {
				t.Fatalf("U+%04X after coeng: widths %d and %d, want 0 and 0", c, coeng, zwj)
			}
		} else if coeng != widthOrOne(c) {
			t.Fatalf("U+%04X after coeng: width %d, want %d", c, coeng, widthOrOne(c))
		}
	}
}

func TestWidth_KhmerQaa(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"ឤ", 2}, {"អឦ", 2}})
}

func TestWidth_KhmerSignBeyyal(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"៘", 3}, {"។ល។", 3}})
}

func TestWidth_EmojiModifier(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"\U0001F46A", 2}, {"\U0001F3FB", 2}, {"\U0001F46A\U0001F3FB", 2}, {"\U0001F46A\u200D\u200D\U0001F3FB", 4}})
}

func TestWidth_EmojiZwj(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase('🏴', 2)})
	checkStrs(t, []strCase{
		{"🧑\u200D🤝\u200D🧑", 2},
		{"🇮🇱🕊\uFE0F🇵🇸", 6},
		{"🇵🇸\u200D🕊\uFE0F\u200D🇮🇱", 2},
		{"🇮🇱\u200D🕊\uFE0F\u200D\u200D🇵🇸", 4},
		{"🇵🇸\u200D\u200D🕊\uFE0F\u200D🇮🇱", 4},
		{"🇦🇦\u200D🇦🇦", 2},
		{"🇦🇦\u200D🇦🇦🇦", 3},
		{"🇦🇦\u200D🇦🇦🇦", 3},
		{"🇦🇦\u200D\u200D🇦🇦", 4},
		{"🇦🇦\u200D🇦\u200D🇦🇦", 5},
		{"🇦🇦\u200D🇦🇦\u200D🇦🇦", 2},
		{"🇦🇦\u200D🇦🇦🇦\u200D🇦🇦", 5},
		{"🇦🇦\u200D🇦🇦🇦🇦\u200D🇦🇦", 4},
		{"🇦🇦\u200D🇦🇦🇦🇦🇦\u200D🇦🇦", 7},
		{"🇦🇦\u200D🇦🇦🇦🇦🇦🇦\u200D🇦🇦", 6},
		{"🇦🇦\u200D🇦🇦🇦🇦🇦🇦🇦\u200D🇦🇦", 9},
		{"🏴\U000E0067\U000E0062\U000E0077\U000E006C\U000E0073\U000E007F", 2},
		{"🏴\U000E0067\U000E0062\U000E0065\U000E006E\U000E0067\U000E007F\u200D🏴\U000E0067\U000E0062\U000E0073\U000E0063\U000E0074\U000E007F\u200D🏴\U000E0067\U000E0062\U000E0077\U000E006C\U000E0073\U000E007F", 2},
		{"🇦👪\u200D🏿", 3},
		{"🇦🏿\u200D🏿", 3},
		{"\U000E0031", 0},
		{"\U000E0063", 0},
		{"\U000E007F", 0},
		{"🏴\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0063\u200DⓂ\uFE0F", 4},
		{"🏴\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0031\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0031\U000E0031\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0031\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0031\U000E0031\U000E0031\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 4},
		{"🏴\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 2},
		{"🏴\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E0063\U000E007F\u200DⓂ\uFE0F", 4},
		{"a\u200D🏴\U000E0067\U000E0062\U000E0077\U000E006C\U000E0073\U000E007F", 3},
		{"👪\u200Da", 3},
		{"a\u200Da", 2},
		{"*\uFE0F", 2},
		{"*\u20E3", 1},
		{"*\uFE0F\u20E3", 2},
		{"*\uFE0F", 2},
		{"*\u20E3\uFE0F", 1},
		{"*\uFE0F\u20E3\u200D👪", 2},
		{"*\u20E3\uFE0F\u200D👪", 3},
		{"*\u20E3\u200D👪", 3},
		{"*\uFE0F\u200D👪", 2},
		{"*\uFE0F\u20E3\u20E3\u200D👪", 4},
		{"*\uFE0F\uFE0F\u20E3\u200D👪", 4},
		{"🇦👪\u200D🏿\u200D👪🏻\u200DⓂ\uFE0F\u200D*\uFE0F\u200D🇦🇦\u200D🏴\U000E0067\U000E0062\U000E0077\U000E006C\U000E0073\U000E007F\u200D👪", 3},
	})
}

func TestWidth_AmbiguousLineBreak(t *testing.T) {
	t.Parallel()
	checkStrs(t, []strCase{{"⓪", 1}, {"☖", 1}, {"➀", 1}})
}

func TestWidth_Vs1Vs2Vs3(t *testing.T) {
	t.Parallel()
	checkChars(t, []charCase{someCase(0xFE00, 0), someCase(0xFE01, 0), someCase(0xFE02, 0)})
	for c := range scalarValues {
		if c == 0x2018 || c == 0x2019 || c == 0x201C || c == 0x201D {
			checkChars(t, []charCase{someCase(c, 1)})
			s := string(c)
			checkStrs(t, []strCase{
				{s + "\uFE00", 1}, {s + "\uFE00\uFE01", 1}, {s + "\uFE01", 2}, {s + "\uFE01\uFE00", 2}, {s + "\uFE02", 1}, {s + "\uFE02\uFE01", 1},
			})
			continue
		}
		for _, vs := range []string{"\uFE00", "\uFE01", "\uFE02"} {
			if got := StrWidth([]byte(string(c) + vs)); got != widthOrOne(c) {
				t.Fatalf("U+%04X followed by %+q: width %d, want %d", c, vs, got, widthOrOne(c))
			}
		}
	}
}

// TestWidth_OracleParity is the Go side of migration/diffrun.py's width comparison. With
// VTPARITY_MODE strwidth or charwidths it writes to VTPARITY_OUT exactly what migration/vtoracle
// writes for the same mode.
func TestWidth_OracleParity(t *testing.T) {
	t.Parallel()
	mode := os.Getenv("VTPARITY_MODE")
	switch mode {
	case "":
		t.Skip("VTPARITY_MODE is unset; migration/diffrun.py drives this test")
	case "strwidth":
		writeStrWidths(t, os.Getenv("VTPARITY_INPUT"), os.Getenv("VTPARITY_OUT"))
	case "charwidths":
		writeCharWidths(t, os.Getenv("VTPARITY_OUT"))
	default:
		t.Skipf("VTPARITY_MODE %s belongs to the emulator parity test", mode)
	}
}

func writeStrWidths(t *testing.T, input, output string) {
	t.Helper()
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	var out []byte
	for line := range bytes.Lines(data) {
		b, err := hex.DecodeString(string(bytes.TrimSpace(line)))
		if err != nil {
			t.Fatal(err)
		}
		if !utf8.Valid(b) {
			t.Fatalf("string %s is not valid UTF-8", line)
		}
		out = fmt.Appendln(out, StrWidth(b))
	}
	if err := os.WriteFile(output, out, 0o600); err != nil {
		t.Fatal(err)
	}
}

func writeCharWidths(t *testing.T, output string) {
	t.Helper()
	table := make([]byte, utf8.MaxRune+1)
	for cp := range table {
		c := rune(cp)
		switch w, ok := CharWidth(c); {
		case c >= 0xD800 && c <= 0xDFFF:
			table[cp] = 's'
		case !ok:
			table[cp] = 'n'
		default:
			table[cp] = "012"[min(w, 2)]
		}
	}
	if err := os.WriteFile(output, table, 0o600); err != nil {
		t.Fatal(err)
	}
}
