"""Seeded fuzz cases for gate G3.

Usage: fuzz.py OUT.jsonl [--corpus DIR]

Writes replay cases (format in cases.py) of three families, each from its own fixed seed:
  grammar   escape-sequence grammar: malformed and oversized CSI, SGR colour forms, DEC modes and
            queries, OSC/DCS/SOS/PM/APC with embedded 0x9C, BEL, CAN, SUB and ESC, raw C1 bytes,
            invalid and split UTF-8, wide, combining and emoji sequences at the last column,
            screens down to 1x1, resizes and random read splits
  random    weighted byte soup rich in ESC and CSI punctuation
  mutated   slices of the corpus streams with byte flips, C1/ESC/OSC/CSI insertions, deletions,
            duplications and random resizes
The grammar and random families depend on their seeds alone and together write at least
MIN_SEEDED_BYTES of child output. The mutated family also reads the streams of the local stream
corpus (format in cases.py) and runs only with --corpus. Each family stops at its byte target.
"""
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cases  # noqa: E402

SEEDS = {"grammar": 1, "random": 2, "mutated": 3}
TARGET_BYTES = {"grammar": 1_400_000, "random": 700_000, "mutated": 1_200_000}
SEEDED_FAMILIES = ("grammar", "random")
MIN_SEEDED_BYTES = 2_000_000

WIDE = ["日", "本", "語", "한", "中", "😀", "✅", "🎉", "ｱ", "Ｗ"]
ZERO = ["́", "̈", "‍", "️", "︎", "⃣", "\U0001F3FB", "្", "ﾞ", "̸"]
EMOJI_BASE = ["#", "1", "*", "❤", "☺", "✌", "👍", "🏳", "🇯", "🇵", "👨", "👩", "ل", "ا", "ל", "ក"]
NARROW = list("abcXYZ 019-=_|~") + ["é", "─", "│", "✳", "✻", "“", "Ü", "М", "…", "→"]
CONTROLS = [b"\r", b"\n", b"\b", b"\t", b"\x0b", b"\x0c", b"\x07", b"\x0e", b"\x0f", b"\x18", b"\x1a", b"\x7f", b"\x00"]
C1 = [bytes([b]) for b in range(0x80, 0xA0)]
CSI_FINALS = "@ABCDEFGHJKLMPSTXbdfmrt`hlqnsuc"
DEC_MODES = [1, 6, 9, 25, 47, 1000, 1002, 1003, 1005, 1006, 1049, 2004, 7, 12, 1004, 2026, 9999]


def num(rng):
    r = rng.random()
    if r < 0.5:
        return str(rng.randint(0, 9))
    if r < 0.8:
        return str(rng.randint(0, 200))
    if r < 0.95:
        return str(rng.randint(0, 70000))
    return "9" * rng.randint(6, 12)


def params(rng, maxn=6):
    out = ""
    for i in range(rng.randint(0, maxn)):
        if i:
            out += rng.choice([";", ";", ";", ":"])
        if rng.random() < 0.85:
            out += num(rng)
    return out


def sgr(rng):
    choice = rng.random()
    if choice < 0.3:
        p = rng.choice(["38;5;", "48;5;"]) + num(rng)
    elif choice < 0.55:
        p = rng.choice(["38;2;", "48;2;"]) + ";".join(num(rng) for _ in range(rng.randint(1, 4)))
    elif choice < 0.65:
        p = rng.choice(["38:2::", "38:5:", "48:2:", "4:3"]) + ":".join(num(rng) for _ in range(rng.randint(0, 3)))
    elif choice < 0.7:
        p = rng.choice(["38", "48", "38;", "48;2"])
    else:
        p = ";".join(str(rng.choice([0, 1, 2, 3, 4, 7, 22, 23, 24, 27, 30, 37, 39, 40, 47, 49, 90, 97, 100, 107, 5, 8, 9, 53, 256])) for _ in range(rng.randint(0, 5)))
    return "\x1b[" + p + "m"


def text_unit(rng, cols):
    r = rng.random()
    if r < 0.45:
        return "".join(rng.choice(NARROW) for _ in range(rng.randint(1, cols + 3)))
    if r < 0.65:
        return "".join(rng.choice(WIDE) for _ in range(rng.randint(1, cols // 2 + 3)))
    if r < 0.85:
        return rng.choice(EMOJI_BASE + NARROW + WIDE) + "".join(rng.choice(ZERO) for _ in range(rng.randint(1, 7)))
    return f"\x1b[{rng.randint(1, 40)};{max(1, cols - rng.randint(0, 2))}H" + rng.choice(WIDE + EMOJI_BASE) + rng.choice(ZERO + [""]) + rng.choice(WIDE + NARROW)


def osc(rng):
    body = rng.choice(["0", "1", "2", "52", "8", "10", "11", "133", "", "0;a;b"]) + rng.choice([";", ""])
    body += "".join(
        rng.choice(["Claude ✳ Code", "Ü“”", "x" * rng.randint(0, 40), "c;SGVsbG8=", "?", "M—", "a;b;c;d;e;f;g;h;i;j;k;l;m;n;o;p;q;r"])
        for _ in range(rng.randint(0, 3))
    )
    data = body.encode()
    if rng.random() < 0.3:
        pos = rng.randint(0, len(data))
        data = data[:pos] + bytes([rng.choice([0x9C, 0x07, 0x18, 0x1A, 0x1B, 0x9B, 0x90, 0x00, 0x0A])]) + data[pos:]
    term = rng.choice([b"\x07", b"\x1b\\", b"\x9c", b"\x18", b"", b"\x1b", b"\x1b[", b"\xe2\x9c\xb3\x07"])
    return b"\x1b]" + data + term


def dcs_like(rng):
    intro = rng.choice([b"\x1bP", b"\x1bX", b"\x1b^", b"\x1b_", b"\x90", b"\x98", b"\x9e", b"\x9f"])
    body = rng.choice([b"1$r0m", b"+q544e", b"q#0;2;0;0;0", b"tmux;\x1b\x1b[31m", b"", b"\xe2\x9c\xb3" * 3])
    term = rng.choice([b"\x1b\\", b"\x9c", b"\x07", b"\x18", b""])
    return intro + body + term


def csi(rng):
    r = rng.random()
    if r < 0.25:
        return ("\x1b[?" + ";".join(str(rng.choice(DEC_MODES)) for _ in range(rng.randint(1, 3))) + rng.choice("hl")).encode()
    if r < 0.45:
        return sgr(rng).encode()
    if r < 0.55:
        intermediate = rng.choice(["", " ", "$", '"', "'"])
        prefix = rng.choice(["", "?", ">", "<", "=", " ", "$", "?$", "!", "  ", ">4;"])
        return ("\x1b[" + prefix + params(rng) + intermediate + rng.choice(CSI_FINALS + "pqzy~")).encode()
    if r < 0.6:
        return ("\x1b[" + ";".join(num(rng) for _ in range(rng.randint(30, 40))) + rng.choice("mHr@")).encode()
    if r < 0.65:
        s = "\x1b[" + params(rng)
        cut = rng.randint(0, len(s))
        return (s[:cut] + rng.choice(["\x1b", "\x18", "\n", "\x9b", "\x7f", "日", "\x1b["])).encode()
    if r < 0.75:
        return f"\x1b[{rng.randint(1, 4)};{rng.randint(1, 40)}r".encode()
    return ("\x1b[" + params(rng, 3) + rng.choice(CSI_FINALS)).encode()


def esc(rng):
    return ("\x1b" + rng.choice(["7", "8", "=", ">", "M", "M", "D", "E", "c", "g", "(B", "(0", ")0", "#8", " F", "%G", "H", "N", "O", "\\", "Z", "[", "]", "P", "1", "~"])).encode()


def utf8_noise(rng):
    r = rng.random()
    if r < 0.4:
        return bytes(rng.choice([[0xC3], [0xE2, 0x9C], [0xF0, 0x9F, 0x98], [0xE6, 0x97]]))
    if r < 0.7:
        return bytes(rng.choice([[0xFF], [0xC0, 0xAF], [0xED, 0xA0, 0x80], [0xF4, 0x90, 0x80, 0x80], [0x80], [0xBF], [0xF8, 0x88]]))
    return rng.choice(C1)


def grammar_case(rng, i):
    rows, cols = rng.choice([(36, 120), (24, 80), (1, 1), (1, 2), (2, 1), (3, 3), (5, 10), (10, 4), (2, 2), (6, 7), (40, 200)])
    init_rows, init_cols = rows, cols
    steps, total, chunk = [], 0, b""
    for _ in range(rng.randint(30, 300)):
        r = rng.random()
        if r < 0.3:
            piece = text_unit(rng, cols).encode()
        elif r < 0.45:
            piece = csi(rng)
        elif r < 0.55:
            piece = rng.choice(CONTROLS)
        elif r < 0.62:
            piece = osc(rng)
        elif r < 0.67:
            piece = dcs_like(rng)
        elif r < 0.75:
            piece = esc(rng)
        elif r < 0.85:
            piece = utf8_noise(rng)
        elif r < 0.9:
            piece = rng.choice(C1) + rng.choice([b"[31m", b"]0;t\x07", b"x", b"2J"])
        else:
            piece = b""
            if chunk:
                steps.append({"hex": chunk.hex()})
                total += len(chunk)
                chunk = b""
            if rng.random() < 0.5:
                rows, cols = rng.choice([(rng.randint(1, 50), rng.randint(1, 140)), (1, 1), (rows, max(1, cols - 1)), (max(1, rows - 1), cols), (rows + 3, cols + 2)])
                steps.append({"resize": [rows, cols]})
        chunk += piece
        if rng.random() < 0.15 and chunk:
            cut = rng.randint(0, len(chunk))
            steps.append({"hex": chunk[:cut].hex()})
            total += cut
            chunk = chunk[cut:]
    if chunk:
        steps.append({"hex": chunk.hex()})
        total += len(chunk)
    return {"name": f"fuzz:grammar:{i:05d}", "rows": init_rows, "cols": init_cols, "steps": steps}, total


RANDOM_ALPHABET = bytes(list(range(0x20, 0x7F)) * 3 + list(range(0x00, 0x20)) + list(range(0x80, 0x100)) + [0x1B] * 40 + list(b"[]?;:0123456789mHJK") * 4)


def random_case(rng, i):
    rows, cols = rng.randint(1, 40), rng.randint(1, 130)
    n = rng.randint(100, 4000)
    data = bytes(rng.choice(RANDOM_ALPHABET) for _ in range(n))
    steps, pos = [], 0
    while pos < len(data):
        step = rng.randint(1, 600)
        steps.append({"hex": data[pos : pos + step].hex()})
        pos += step
        if rng.random() < 0.05:
            steps.append({"resize": [rng.randint(1, 40), rng.randint(1, 130)]})
    return {"name": f"fuzz:random:{i:05d}", "rows": rows, "cols": cols, "steps": steps}, n


def mutate(rng, raw):
    data = bytearray(raw)
    for _ in range(rng.randint(1, max(2, len(data) // 300))):
        op = rng.random()
        pos = rng.randint(0, len(data))
        if op < 0.25 and data:
            data[min(pos, len(data) - 1)] = rng.randint(0, 255)
        elif op < 0.45:
            data[pos:pos] = rng.choice([b"\x9c", b"\x1b", b"\x1b[", b"\x1b]0;", b"\x18", b"\x07", bytes([rng.randint(0x80, 0x9F)]), "日".encode(), "́".encode(), "️".encode()])
        elif op < 0.6 and data:
            del data[pos : pos + rng.randint(1, 16)]
        elif op < 0.75 and data:
            a = rng.randint(0, len(data) - 1)
            data[pos:pos] = data[a : a + rng.randint(1, 64)]
        else:
            data[pos:pos] = csi(rng)
    return bytes(data)


def corpus_sources(directory):
    sources = []
    for stream in cases.corpus_streams(directory):
        raw = b"".join(bytes.fromhex(e["out"]) for e in stream["events"] if "out" in e)
        if raw:
            sources.append((stream["name"], stream["rows"], stream["cols"], raw))
    assert sources, f"no corpus output in {directory}"
    return sources


def mutated_case(rng, i, sources):
    name, rows, cols, raw = rng.choice(sources)
    start = rng.randint(0, max(0, len(raw) - 1))
    raw = raw[start : start + rng.randint(500, 20000)] if rng.random() < 0.7 else raw[: rng.randint(500, 30000)]
    data = mutate(rng, raw)
    steps, pos = [], 0
    if rng.random() < 0.3:
        rows, cols = rng.randint(1, 50), rng.randint(1, 140)
    while pos < len(data):
        step = rng.randint(1, 2000)
        steps.append({"hex": data[pos : pos + step].hex()})
        pos += step
        if rng.random() < 0.04:
            steps.append({"resize": [rng.randint(1, 50), rng.randint(1, 140)]})
    return {"name": f"fuzz:mutated:{i:05d}:{name}", "rows": rows, "cols": cols, "steps": steps}, len(data)


def family_cases(family, sources):
    rng = random.Random(SEEDS[family])
    generate = {"grammar": grammar_case, "random": random_case, "mutated": lambda r, i: mutated_case(r, i, sources)}[family]
    total, out = 0, []
    while total < TARGET_BYTES[family]:
        case, n = generate(rng, len(out))
        out.append(case)
        total += n
    return out, total


def generate(corpus=None):
    """The fuzz cases: the seeded families, plus the mutated family when a corpus directory is given."""
    families = SEEDED_FAMILIES + (("mutated",) if corpus is not None else ())
    sources = corpus_sources(corpus) if corpus is not None else None
    all_cases, seeded = [], 0
    for family in families:
        family_list, total = family_cases(family, sources)
        all_cases += family_list
        if family in SEEDED_FAMILIES:
            seeded += total
        print(f"{family}: {len(family_list)} cases, {total} bytes (seed {SEEDS[family]})")
    assert seeded >= MIN_SEEDED_BYTES, f"seeded fuzz input is {seeded} bytes, below {MIN_SEEDED_BYTES}"
    return all_cases


def main(argv):
    assert len(argv) in (2, 4) and (len(argv) == 2 or argv[2] == "--corpus"), __doc__
    out_path = argv[1]
    all_cases = generate(argv[3] if len(argv) == 4 else None)
    cases.write_jsonl(out_path, all_cases)
    print(f"{out_path}: {len(all_cases)} cases")


if __name__ == "__main__":
    main(sys.argv)
