// Transliterated from unicode-width 0.2.2 src/tables.rs, non-cjk path only (MIT OR Apache-2.0,
// Copyright 2012-2025 The Rust Project Developers).

// Package unicodewidth computes UAX #11 display widths exactly as unicode-width 0.2.2 does.
package unicodewidth

import (
	"sort"
	"unicode/utf8"
)

type widthInfo uint16

const ligatureTransparentMask widthInfo = 0b0010_0000_0000_0000

const (
	wiDefault                              widthInfo = 0
	wiLineFeed                             widthInfo = 0b0000000000000001
	wiEmojiModifier                        widthInfo = 0b0000000000000010
	wiRegionalIndicator                    widthInfo = 0b0000000000000011
	wiSeveralRegionalIndicator             widthInfo = 0b0000000000000100
	wiEmojiPresentation                    widthInfo = 0b0000000000000101
	wiZwjEmojiPresentation                 widthInfo = 0b0001000000000110
	wiVs16ZwjEmojiPresentation             widthInfo = 0b1001000000000110
	wiKeycapZwjEmojiPresentation           widthInfo = 0b0001000000000111
	wiVs16KeycapZwjEmojiPresentation       widthInfo = 0b1001000000000111
	wiRegionalIndicatorZwjPresentation     widthInfo = 0b0000000000001001
	wiEvenRegionalIndicatorZwjPresentation widthInfo = 0b0000000000001010
	wiOddRegionalIndicatorZwjPresentation  widthInfo = 0b0000000000001011
	wiTagEndZwjEmojiPresentation           widthInfo = 0b0000000000010000
	wiTagD1EndZwjEmojiPresentation         widthInfo = 0b0000000000010001
	wiTagD2EndZwjEmojiPresentation         widthInfo = 0b0000000000010010
	wiTagD3EndZwjEmojiPresentation         widthInfo = 0b0000000000010011
	wiTagA1EndZwjEmojiPresentation         widthInfo = 0b0000000000011001
	wiTagA2EndZwjEmojiPresentation         widthInfo = 0b0000000000011010
	wiTagA3EndZwjEmojiPresentation         widthInfo = 0b0000000000011011
	wiTagA4EndZwjEmojiPresentation         widthInfo = 0b0000000000011100
	wiTagA5EndZwjEmojiPresentation         widthInfo = 0b0000000000011101
	wiTagA6EndZwjEmojiPresentation         widthInfo = 0b0000000000011110
	wiKiratRaiVowelSignE                   widthInfo = 0b0000000000100000
	wiKiratRaiVowelSignAi                  widthInfo = 0b0000000000100001
	wiVariationSelector123                 widthInfo = 0b0000001000000000
	wiVariationSelector15                  widthInfo = 0b0100000000000000
	wiVariationSelector16                  widthInfo = 0b1000000000000000
	wiJoiningGroupAlef                     widthInfo = 0b0011000011111111
	wiHebrewLetterLamed                    widthInfo = 0b0011100000000000
	wiZwjHebrewLetterLamed                 widthInfo = 0b0011110000000000
	wiBugineseLetterYa                     widthInfo = 0b0011100000000001
	wiZwjBugineseLetterYa                  widthInfo = 0b0011110000000001
	wiBugineseVowelSignIZwjLetterYa        widthInfo = 0b0011110000000010
	wiTifinaghConsonant                    widthInfo = 0b0011100000000011
	wiZwjTifinaghConsonant                 widthInfo = 0b0011110000000011
	wiTifinaghJoinerConsonant              widthInfo = 0b0011110000000100
	wiLisuToneLetterMyaNaJeu               widthInfo = 0b0011110000000101
	wiOldTurkicLetterOrkhonI               widthInfo = 0b0011100000000110
	wiZwjOldTurkicLetterOrkhonI            widthInfo = 0b0011110000000110
	wiKhmerCoengEligibleLetter             widthInfo = 0b0011110000000111
)

func (w widthInfo) isLigatureTransparent() bool {
	return w&0b0000_1000_0000_0000 == 0b0000_1000_0000_0000
}

func (w widthInfo) setZwjBit() widthInfo { return w | 0b0000_0100_0000_0000 }

func (w widthInfo) isEmojiPresentation() bool {
	return w&wiVariationSelector16 == wiVariationSelector16
}

func (w widthInfo) isZwjEmojiPresentation() bool {
	return w&0b1011_0000_0000_0000 == 0b1001_0000_0000_0000
}

func (w widthInfo) setEmojiPresentation() widthInfo {
	if w&ligatureTransparentMask == ligatureTransparentMask || w&0b1001_0000_0000_0000 == 0b0001_0000_0000_0000 {
		return w | wiVariationSelector16&^wiVariationSelector15&^wiVariationSelector123
	}
	return wiVariationSelector16
}

func (w widthInfo) unsetEmojiPresentation() widthInfo {
	if w&ligatureTransparentMask == ligatureTransparentMask {
		return w &^ wiVariationSelector16
	}
	return wiDefault
}

func (w widthInfo) isTextPresentation() bool { return w&wiVariationSelector15 == wiVariationSelector15 }

func (w widthInfo) setTextPresentation() widthInfo {
	if w&ligatureTransparentMask == ligatureTransparentMask {
		return w | wiVariationSelector15&^wiVariationSelector16&^wiVariationSelector123
	}
	return wiVariationSelector15
}

func (w widthInfo) unsetTextPresentation() widthInfo { return w &^ wiVariationSelector15 }

func (w widthInfo) isVs123() bool { return w&wiVariationSelector123 == wiVariationSelector123 }

func (w widthInfo) setVs123() widthInfo {
	if w&ligatureTransparentMask == ligatureTransparentMask {
		return w | wiVariationSelector123&^wiVariationSelector15&^wiVariationSelector16
	}
	return wiVariationSelector123
}

func (w widthInfo) unsetVs123() widthInfo { return w &^ wiVariationSelector123 }

// scalar is Rust's `char as u32`: every rune this package receives is a Unicode scalar value.
func scalar(c rune) uint32 {
	if c < 0 {
		panic("unicodewidth: negative rune")
	}
	return uint32(c)
}

// lowByte is Rust's truncating `char as u8`.
func lowByte(c rune) uint8 { return uint8(scalar(c) & 0xFF) }

func lookupWidth(c rune) (uint8, widthInfo) {
	cp := scalar(c)
	t1 := tWidthRoot[cp>>13]
	t2 := tWidthMiddle[t1][cp>>7&0x3F]
	packed := tWidthLeaves[t2][cp>>2&0x1F]
	width := packed >> (2 * (cp & 0b11)) & 0b11
	if width < 3 {
		return width, wiDefault
	}
	switch {
	case c == 0xA:
		return 1, wiLineFeed
	case c == 0x5DC:
		return 1, wiHebrewLetterLamed
	case c >= 0x622 && c <= 0x882:
		return 1, wiJoiningGroupAlef
	case c >= 0x1780 && c <= 0x17AF:
		return 1, wiKhmerCoengEligibleLetter
	case c == 0x17D8:
		return 3, wiDefault
	case c == 0x1A10:
		return 1, wiBugineseLetterYa
	case c >= 0x2D31 && c <= 0x2D6F:
		return 1, wiTifinaghConsonant
	case c >= 0xA4FC && c <= 0xA4FD:
		return 1, wiLisuToneLetterMyaNaJeu
	case c == 0xFE01:
		return 0, wiVariationSelector123
	case c == 0xFE0E:
		return 0, wiVariationSelector15
	case c == 0xFE0F:
		return 0, wiVariationSelector16
	case c == 0x10C03:
		return 1, wiOldTurkicLetterOrkhonI
	case c == 0x16D67:
		return 1, wiKiratRaiVowelSignE
	case c == 0x16D68:
		return 1, wiKiratRaiVowelSignAi
	case c >= 0x1F1E6 && c <= 0x1F1FF:
		return 1, wiRegionalIndicator
	case c >= 0x1F3FB && c <= 0x1F3FF:
		return 2, wiEmojiModifier
	default:
		return 2, wiEmojiPresentation
	}
}

// CharWidth is UnicodeWidthChar::width: ok is false for control characters.
func CharWidth(c rune) (width int, ok bool) {
	if c < 0x7F {
		if c >= 0x20 {
			return 1, true
		}
		return 0, false
	}
	if c >= 0xA0 {
		w, _ := lookupWidth(c)
		return int(w), true
	}
	return 0, false
}

func isRegionalIndicator(c rune) bool { return c >= 0x1F1E6 && c <= 0x1F1FF }

func isTagLetter(c rune) bool { return c >= 0xE0061 && c <= 0xE007A }

func isTagDigit(c rune) bool { return c >= 0xE0030 && c <= 0xE0039 }

func isTifinaghLetter(c rune) bool { return (c >= 0x2D31 && c <= 0x2D65) || c == 0x2D6F }

func widthInStr(c rune, next widthInfo) (int8, widthInfo) {
	if next.isEmojiPresentation() {
		if startsEmojiPresentationSeq(c) {
			if next.isZwjEmojiPresentation() {
				return 0, wiEmojiPresentation
			}
			return 2, wiEmojiPresentation
		}
		next = next.unsetEmojiPresentation()
	}
	if c <= 0xA0 {
		switch {
		case c == '\n':
			return 1, wiLineFeed
		case c == '\r' && next == wiLineFeed:
			return 0, wiDefault
		default:
			return 1, wiDefault
		}
	}
	if next != wiDefault {
		if c == 0xFE0F {
			return 0, next.setEmojiPresentation()
		}
		if c == 0xFE01 {
			return 0, next.setVs123()
		}
		if c == 0xFE0E {
			return 0, next.setTextPresentation()
		}
		if next.isTextPresentation() {
			if startsNonIdeographicTextPresentationSeq(c) {
				return 1, wiDefault
			}
			next = next.unsetTextPresentation()
		} else if next.isVs123() {
			if c == 0x2018 || c == 0x2019 || c == 0x201C || c == 0x201D {
				return 2, wiDefault
			}
			next = next.unsetVs123()
		}
		if next.isLigatureTransparent() {
			if c == 0x200D {
				return 0, next.setZwjBit()
			} else if isLigatureTransparent(c) {
				return 0, next
			}
		}
		if w, info, ok := widthInStrPair(c, next); ok {
			return w, info
		}
	}
	w, info := lookupWidth(c)
	if w > 3 {
		panic("unicodewidth: table widths are 0 to 3")
	}
	return int8(w), info
}

func widthInStrPair(c rune, next widthInfo) (int8, widthInfo, bool) {
	switch {
	case next == wiJoiningGroupAlef && (c == 0x644 || (c >= 0x6B5 && c <= 0x6B8) || c == 0x76A || c == 0x8A6 || c == 0x8C7):
		return 0, wiDefault, true
	case next == wiJoiningGroupAlef && isTransparentZeroWidth(c):
		return 0, wiJoiningGroupAlef, true
	case next == wiZwjHebrewLetterLamed && c == 0x05D0:
		return 0, wiDefault, true
	case next == wiKhmerCoengEligibleLetter && c == 0x17D2:
		return -1, wiDefault, true
	case next == wiZwjBugineseLetterYa && c == 0x1A17:
		return 0, wiBugineseVowelSignIZwjLetterYa, true
	case next == wiBugineseVowelSignIZwjLetterYa && c == 0x1A15:
		return 0, wiDefault, true
	case (next == wiTifinaghConsonant || next == wiZwjTifinaghConsonant) && c == 0x2D7F:
		return 1, wiTifinaghJoinerConsonant, true
	case next == wiZwjTifinaghConsonant && isTifinaghLetter(c):
		return 0, wiDefault, true
	case next == wiTifinaghJoinerConsonant && isTifinaghLetter(c):
		return -1, wiDefault, true
	case next == wiLisuToneLetterMyaNaJeu && c >= 0xA4F8 && c <= 0xA4FB:
		return 0, wiDefault, true
	case next == wiZwjOldTurkicLetterOrkhonI && c == 0x10C32:
		return 0, wiDefault, true
	case next == wiEmojiModifier && isEmojiModifierBase(c):
		return 0, wiEmojiPresentation, true
	case (next == wiRegionalIndicator || next == wiSeveralRegionalIndicator) && isRegionalIndicator(c):
		return 1, wiSeveralRegionalIndicator, true
	case (next == wiEmojiPresentation || next == wiSeveralRegionalIndicator || next == wiEvenRegionalIndicatorZwjPresentation ||
		next == wiOddRegionalIndicatorZwjPresentation || next == wiEmojiModifier) && c == 0x200D:
		return 0, wiZwjEmojiPresentation, true
	case next == wiZwjEmojiPresentation && c == 0x20E3:
		return 0, wiKeycapZwjEmojiPresentation, true
	case next == wiVs16ZwjEmojiPresentation && startsEmojiPresentationSeq(c):
		return 0, wiEmojiPresentation, true
	case next == wiVs16KeycapZwjEmojiPresentation && ((c >= '0' && c <= '9') || c == '#' || c == '*'):
		return 0, wiEmojiPresentation, true
	case next == wiZwjEmojiPresentation && isRegionalIndicator(c):
		return 1, wiRegionalIndicatorZwjPresentation, true
	case (next == wiRegionalIndicatorZwjPresentation || next == wiOddRegionalIndicatorZwjPresentation) && isRegionalIndicator(c):
		return -1, wiEvenRegionalIndicatorZwjPresentation, true
	case next == wiEvenRegionalIndicatorZwjPresentation && isRegionalIndicator(c):
		return 3, wiOddRegionalIndicatorZwjPresentation, true
	case next == wiZwjEmojiPresentation && c >= 0x1F3FB && c <= 0x1F3FF:
		return 0, wiEmojiModifier, true
	case next == wiZwjEmojiPresentation && c == 0xE007F:
		return 0, wiTagEndZwjEmojiPresentation, true
	case next == wiTagEndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA1EndZwjEmojiPresentation, true
	case next == wiTagA1EndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA2EndZwjEmojiPresentation, true
	case next == wiTagA2EndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA3EndZwjEmojiPresentation, true
	case next == wiTagA3EndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA4EndZwjEmojiPresentation, true
	case next == wiTagA4EndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA5EndZwjEmojiPresentation, true
	case next == wiTagA5EndZwjEmojiPresentation && isTagLetter(c):
		return 0, wiTagA6EndZwjEmojiPresentation, true
	case (next == wiTagEndZwjEmojiPresentation || next == wiTagA1EndZwjEmojiPresentation || next == wiTagA2EndZwjEmojiPresentation ||
		next == wiTagA3EndZwjEmojiPresentation || next == wiTagA4EndZwjEmojiPresentation) && isTagDigit(c):
		return 0, wiTagD1EndZwjEmojiPresentation, true
	case next == wiTagD1EndZwjEmojiPresentation && isTagDigit(c):
		return 0, wiTagD2EndZwjEmojiPresentation, true
	case next == wiTagD2EndZwjEmojiPresentation && isTagDigit(c):
		return 0, wiTagD3EndZwjEmojiPresentation, true
	case (next == wiTagA3EndZwjEmojiPresentation || next == wiTagA4EndZwjEmojiPresentation || next == wiTagA5EndZwjEmojiPresentation ||
		next == wiTagA6EndZwjEmojiPresentation || next == wiTagD3EndZwjEmojiPresentation) && c == 0x1F3F4:
		return 0, wiEmojiPresentation, true
	case next == wiZwjEmojiPresentation:
		if _, info := lookupWidth(c); info == wiEmojiPresentation {
			return 0, wiEmojiPresentation, true
		}
	}
	switch {
	case next == wiKiratRaiVowelSignE && c == 0x16D63:
		return 0, wiDefault, true
	case next == wiKiratRaiVowelSignE && c == 0x16D67:
		return 0, wiKiratRaiVowelSignAi, true
	case next == wiKiratRaiVowelSignE && c == 0x16D68:
		return 1, wiKiratRaiVowelSignE, true
	case next == wiKiratRaiVowelSignE && c == 0x16D69:
		return 0, wiDefault, true
	case next == wiKiratRaiVowelSignAi && c == 0x16D63:
		return 0, wiDefault, true
	}
	return 0, 0, false
}

// StrWidth is UnicodeWidthStr::width, including its usize wrapping on negative contributions.
func StrWidth(s []byte) uint {
	var runes [32]rune
	rs := runes[:0]
	for i := 0; i < len(s); {
		c, size := utf8.DecodeRune(s[i:])
		rs = append(rs, c)
		i += size
	}
	var sum uint
	info := wiDefault
	for i := len(rs) - 1; i >= 0; i-- {
		var add int8
		add, info = widthInStr(rs[i], info)
		sum += uint(int(add))
	}
	return sum
}

func inRanges24(table [][2]uint32, cp uint32) bool {
	i := sort.Search(len(table), func(i int) bool { return table[i][1] >= cp })
	return i < len(table) && table[i][0] <= cp
}

func inRanges8(table [][2]uint8, b uint8) bool {
	i := sort.Search(len(table), func(i int) bool { return table[i][1] >= b })
	return i < len(table) && table[i][0] <= b
}

func isTransparentZeroWidth(c rune) bool {
	if w, _ := lookupWidth(c); w != 0 {
		return false
	}
	return !inRanges24(tNonTransparentZeroWidths, scalar(c))
}

func isLigatureTransparent(c rune) bool {
	return c == 0x34F || (c >= 0x17B4 && c <= 0x17B5) || (c >= 0x180B && c <= 0x180D) || c == 0x180F ||
		c == 0x200D || (c >= 0xFE00 && c <= 0xFE0F) || (c >= 0xE0100 && c <= 0xE01EF)
}

func startsEmojiPresentationSeq(c rune) bool {
	cp := scalar(c)
	var leaf int
	switch cp >> 10 {
	case 0x0:
		leaf = 0
	case 0x8:
		leaf = 1
	case 0x9:
		leaf = 2
	case 0xA:
		leaf = 3
	case 0xC:
		leaf = 4
	case 0x7C:
		leaf = 5
	case 0x7D:
		leaf = 6
	default:
		return false
	}
	b := tEmojiPresentationLeaves[leaf][(cp>>3)&0x7F]
	return (b>>(cp&7))&1 == 1
}

var textPresentationLeaves = map[uint32][][2]uint8{
	0x23: tTextPresentationLeaf0, 0x25: tTextPresentationLeaf1, 0x26: tTextPresentationLeaf2,
	0x27: tTextPresentationLeaf3, 0x2B: tTextPresentationLeaf4, 0x1F0: tTextPresentationLeaf5,
	0x1F3: tTextPresentationLeaf6, 0x1F4: tTextPresentationLeaf7, 0x1F5: tTextPresentationLeaf8,
	0x1F6: tTextPresentationLeaf9,
}

var emojiModifierLeaves = map[uint32][][2]uint8{
	0x26: tEmojiModifierLeaf0, 0x27: tEmojiModifierLeaf1, 0x1F3: tEmojiModifierLeaf2,
	0x1F4: tEmojiModifierLeaf3, 0x1F5: tEmojiModifierLeaf4, 0x1F6: tEmojiModifierLeaf5,
	0x1F9: tEmojiModifierLeaf6, 0x1FA: tEmojiModifierLeaf7,
}

func startsNonIdeographicTextPresentationSeq(c rune) bool {
	leaf, ok := textPresentationLeaves[scalar(c)>>8]
	return ok && inRanges8(leaf, lowByte(c))
}

func isEmojiModifierBase(c rune) bool {
	leaf, ok := emojiModifierLeaves[scalar(c)>>8]
	return ok && inRanges8(leaf, lowByte(c))
}
