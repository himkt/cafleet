"""Replay cases for gates G3 and G4, generated at run time.

A replay case is one JSON object per line:
    {"name": str, "rows": int, "cols": int, "steps": [{"hex": str} | {"resize": [rows, cols]}]}

Case sources:
  fixture:<screen>      every native screen fixture in assets/profiles/*.json, drawn with CSI
  synthetic:<name>      the hand-written emulator edge cases in SYNTHETIC
  live:<name>           a whole stream of the local corpus's live.jsonl.gz
  capture:<name>        a whole stream of the local corpus's capture.jsonl.gz

The local stream corpus is a directory holding CORPUS_FILES, never committed (the Director keeps it
in the gitignored researches/go-rewrite/corpus/). A corpus stream is one JSON object per line of a
gzip-compressed JSONL file:
    {"name": str, "rows": int, "cols": int, "native_version": str | null,
     "events": [{"t": ms, "out": hex} | {"t": ms, "resize": [rows, cols]} | {"t": ms, "mark": label}]}
Live streams were recorded without timestamps, so their events carry no "t".

Usage:
  cases.py fixtures OUT.jsonl              fixture and synthetic cases
  cases.py streams OUT.jsonl --corpus DIR  one whole-stream case per corpus stream
"""
import gzip
import json
import pathlib
import subprocess
import sys
import unicodedata

REPO = pathlib.Path(__file__).resolve().parent.parent
PROFILES = REPO / "assets" / "profiles"
CORPUS_FILES = ("live.jsonl.gz", "capture.jsonl.gz")
ORACLE_MANIFEST = REPO / "migration" / "vtoracle" / "Cargo.toml"
ORACLE_TARGET = REPO / "target" / "vtoracle"
ESC = "\x1b"
QUIET_MS = 300


def build_oracle():
    """Build migration/vtoracle in release mode against its own Cargo.lock and return its path."""
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--quiet", "--manifest-path", str(ORACLE_MANIFEST), "--target-dir", str(ORACLE_TARGET)],
        check=True,
    )
    return ORACLE_TARGET / "release" / "vtoracle"


def run_oracle(oracle, *args):
    return subprocess.run([str(oracle), *map(str, args)], check=True, capture_output=True, text=True).stdout


def write_jsonl(path, records):
    with open(path, "w") as out:
        for record in records:
            out.write(json.dumps(record, ensure_ascii=False) + "\n")


def read_jsonl(path):
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def read_corpus(path):
    with gzip.open(path, "rt") as f:
        return [json.loads(line) for line in f if line.strip()]


def write_corpus(path, streams):
    with gzip.GzipFile(path, "wb", mtime=0) as f:
        for stream in streams:
            f.write((json.dumps(stream, ensure_ascii=False, separators=(",", ":")) + "\n").encode())


def corpus_paths(directory):
    """The corpus files of a local stream corpus directory; every one of CORPUS_FILES must exist."""
    paths = [pathlib.Path(directory) / name for name in CORPUS_FILES]
    missing = [str(p) for p in paths if not p.is_file()]
    assert not missing, f"the stream corpus lacks {missing}"
    return paths


def corpus_streams(directory, prefix=None):
    streams = [s for path in corpus_paths(directory) for s in read_corpus(path)]
    if prefix is not None:
        streams = [s for s in streams if s["name"].startswith(prefix)]
    assert streams, f"no {prefix or ''}streams in the corpus at {directory}"
    return streams


def substep_count(data):
    """Observation steps a chunk splits into, as TerminalState::feed_observed_classified splits it:
    every ASCII control byte alone, and every run of other bytes between them."""
    count, in_run = 0, False
    for b in data:
        if b < 0x20 or b == 0x7F:
            count += 1
            in_run = False
        elif not in_run:
            count += 1
            in_run = True
    return count


def event_steps(event):
    if "out" in event:
        return [{"hex": event["out"]}]
    if "resize" in event:
        return [{"resize": event["resize"]}]
    assert "mark" in event, f"unknown corpus event {sorted(event)}"
    return []


def stream_case(stream, upto=None, name=None):
    events = stream["events"] if upto is None else stream["events"][:upto]
    return {
        "name": name or stream["name"],
        "rows": stream["rows"],
        "cols": stream["cols"],
        "steps": [step for event in events for step in event_steps(event)],
    }


def observation_steps_before(stream, upto):
    """Number of observation steps the oracle runs for the first `upto` events."""
    total = 0
    for event in stream["events"][:upto]:
        if "out" in event:
            total += substep_count(bytes.fromhex(event["out"]))
        elif "resize" in event:
            total += 1
    return total


def checkpoints(stream):
    """(event_index, label) at every state boundary: the state each mark names, the state before
    each resize, every output gap of at least QUIET_MS and the final state. The index is the number
    of leading events that make up the checkpoint's state."""
    points = []
    last_out_i, last_out_t = None, None
    for i, event in enumerate(stream["events"]):
        if "mark" in event:
            points.append((i, f"mark={event['mark']}"))
        elif "resize" in event:
            points.append((i, f"pre-resize={event['resize'][0]}x{event['resize'][1]}"))
        elif "out" in event and "t" in event:
            if last_out_t is not None and event["t"] - last_out_t >= QUIET_MS:
                points.append((last_out_i + 1, f"quiet@{last_out_t}ms"))
            last_out_i, last_out_t = i, event["t"]
    points.append((len(stream["events"]), "final"))
    seen, unique = set(), []
    for idx, label in sorted(points, key=lambda p: p[0]):
        if idx not in seen:
            seen.add(idx)
            unique.append((idx, label))
    return unique


def checkpoint_name(stream, n, label):
    return f"{stream['name']}:{n:03d}:{label}"


def width(ch):
    if unicodedata.combining(ch) or unicodedata.category(ch) in ("Mn", "Me", "Cf"):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def colour_sgr(layer, c):
    if c["kind"] == "rgb":
        return f"{layer};2;{c['r']};{c['g']};{c['b']}"
    assert c["kind"] == "indexed", f"unknown colour kind {c['kind']}"
    return f"{layer};5;{c['index']}"


def fixture_case(name, fx):
    rows, cols = fx["rows"], fx["cols"]
    style = {}
    for run in fx.get("cell_backgrounds", []):
        for col in range(run["start_col"], run["end_col"]):
            style.setdefault((run["row"], col), []).append(colour_sgr(48, run["bg"]))
    for run in fx.get("cell_styles", []):
        for col in range(run["start_col"], run["end_col"]):
            attrs = style.setdefault((run["row"], col), [])
            if run["faint"]:
                attrs.append("2")
            if run["foreground"] is not None:
                attrs.append(colour_sgr(38, run["foreground"]))
    out = []
    if fx["alternate_screen"]:
        out.append(f"{ESC}[?1049h")
    out.append(f"{ESC}[2J")
    for row, line in enumerate(fx["lines"]):
        styled_cols = [c for (r, c) in style if r == row]
        if not line and not styled_cols:
            continue
        out.append(f"{ESC}[{row + 1};1H")
        col, current = 0, ""
        for ch in line:
            sgr = ";".join(style.get((row, col), []))
            if sgr != current:
                out.append(f"{ESC}[0m" + (f"{ESC}[{sgr}m" if sgr else ""))
                current = sgr
            out.append(ch)
            col += width(ch)
        for c in range(col, max(styled_cols, default=-1) + 1):
            sgr = ";".join(style.get((row, c), []))
            if sgr != current:
                out.append(f"{ESC}[0m" + (f"{ESC}[{sgr}m" if sgr else ""))
                current = sgr
            out.append(" ")
        out.append(f"{ESC}[0m")
    if fx["cursor_row"] is not None:
        out.append(f"{ESC}[{fx['cursor_row'] + 1};{fx['cursor_col'] + 1}H")
    return {"name": name, "rows": rows, "cols": cols, "steps": [{"hex": "".join(out).encode().hex()}]}


def s(*parts):
    steps = []
    for p in parts:
        if isinstance(p, str):
            steps.append({"hex": p.encode("utf-8", "surrogateescape").hex()})
        elif isinstance(p, bytes):
            steps.append({"hex": p.hex()})
        else:
            steps.append({"resize": list(p)})
    return steps


SYNTHETIC = {
    "cjk-text": (6, 20, s("日本語のテキスト abc 한국어")),
    "wide-at-last-column": (4, 10, s("123456789", "日本")),
    "wide-overwrite-half": (4, 10, s("日本語", f"{ESC}[1;2Hx")),
    "wide-overwrite-lead": (4, 10, s(f"{ESC}[41m日本語{ESC}[0m", f"{ESC}[1;1Hy")),
    "combining-acute": (4, 10, s("éx ä")),
    "combining-split-across-writes": (4, 10, s("e", "́x")),
    "emoji-zwj-family": (4, 20, s("A\U0001F468‍\U0001F469‍\U0001F467B")),
    "emoji-flag": (4, 20, s("A\U0001F1EF\U0001F1F5B")),
    "emoji-vs16-heart": (4, 20, s("A❤️B")),
    "emoji-plain": (4, 20, s("A\U0001F600B✅C")),
    "ambiguous-width": (4, 20, s("①─═○●×→…§")),
    "hangul-jamo": (4, 20, s("각X")),
    "autowrap-row-wrapped": (4, 10, s("abcdefghijKLMNO")),
    "pending-wrap-cursor": (4, 10, s("abcdefghij")),
    "pending-wrap-then-cr": (4, 10, s("abcdefghij\r")),
    "no-autowrap-mode": (4, 10, s(f"{ESC}[?7labcdefghijKLM")),
    "sgr-basic-indexed": (2, 40, s(f"{ESC}[31mR{ESC}[91mB{ESC}[38;5;9mI{ESC}[38;5;1mJ{ESC}[41mb{ESC}[101mc{ESC}[0m")),
    "sgr-rgb-semicolon": (2, 40, s(f"{ESC}[38;2;1;2;3mR{ESC}[48;2;250;128;0mG{ESC}[0m")),
    "sgr-rgb-colon": (2, 40, s(f"{ESC}[38:2::1:2:3mR{ESC}[48:5:200mG{ESC}[0m")),
    "sgr-attrs": (2, 40, s(f"{ESC}[1mB{ESC}[2mD{ESC}[22mN{ESC}[3mI{ESC}[23m{ESC}[4mU{ESC}[24m{ESC}[7mV{ESC}[27m{ESC}[1;2mX{ESC}[0m")),
    "sgr-default-resets": (2, 40, s(f"{ESC}[31;42mA{ESC}[39mB{ESC}[49mC{ESC}[m")),
    "sgr-bright-bold": (2, 40, s(f"{ESC}[1;31mA{ESC}[0m")),
    "sgr-underline-styles": (2, 40, s(f"{ESC}[4:3mA{ESC}[4:0mB{ESC}[21mC{ESC}[0m")),
    "alt-screen-1049": (4, 20, s("primary", f"{ESC}[?1049h", "alternate", f"{ESC}[?1049l")),
    "alt-screen-1049-stay": (4, 20, s("primary", f"{ESC}[?1049h", "alternate")),
    "alt-screen-47": (4, 20, s("primary", f"{ESC}[?47h", "alt47")),
    "alt-screen-1047": (4, 20, s("primary", f"{ESC}[?1047h", "alt1047", f"{ESC}[?1047l")),
    "title-osc0-bel": (2, 20, s(f"{ESC}]0;hello title\x07x")),
    "title-osc2-st": (2, 20, s(f"{ESC}]2;st title{ESC}\\x")),
    "title-fragmented": (2, 20, s(f"{ESC}]2;frag", "mented t", "itle\x07x")),
    "title-utf8": (2, 20, s(f"{ESC}]0;✳ Claude Code\x07")),
    "title-pi": (2, 20, s(f"{ESC}]0;π - 28 - cafleet\x07")),
    "title-curly-quote": (2, 20, s(f"{ESC}]2;say “hi”\x07")),
    "title-umlaut": (2, 20, s(f"{ESC}]2;Über\x07")),
    "title-spinner-glyphs": (2, 20, s(f"{ESC}]0;✻ Working\x07")),
    "dcs-with-utf8": (2, 20, s(f"{ESC}Pq✳{ESC}\\Z")),
    "bracketed-paste-on": (2, 20, s(f"{ESC}[?2004h")),
    "bracketed-paste-off": (2, 20, s(f"{ESC}[?2004h", f"{ESC}[?2004l")),
    "scroll-region": (6, 10, s("1\r\n2\r\n3\r\n4\r\n5\r\n6", f"{ESC}[2;5r{ESC}[5;1H\nX{ESC}[2;1H{ESC}MY")),
    "insert-delete-lines": (6, 10, s("1\r\n2\r\n3\r\n4\r\n5\r\n6", f"{ESC}[2;1H{ESC}[2L{ESC}[5;1H{ESC}[1M")),
    "insert-delete-chars": (2, 10, s("abcdefgh", f"{ESC}[1;3H{ESC}[2@{ESC}[1;1H{ESC}[1P{ESC}[1;6H{ESC}[2X")),
    "erase-line-modes": (4, 10, s("aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc", f"{ESC}[1;5H{ESC}[K{ESC}[2;5H{ESC}[1K{ESC}[3;5H{ESC}[2K")),
    "erase-display-modes": (4, 10, s("aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc\r\ndddddddddd", f"{ESC}[2;5H{ESC}[J{ESC}[1;3H{ESC}[1J")),
    "erase-with-bg-bce": (3, 10, s(f"{ESC}[44m{ESC}[2J{ESC}[0mX")),
    "lf-scroll-at-bottom": (3, 10, s("1\r\n2\r\n3\r\n4\r\n5")),
    "tabs-bs-cr": (2, 30, s("a\tb\tc\x08\x08X\rY")),
    "save-restore-cursor": (3, 20, s(f"ab{ESC}7{ESC}[3;5Hcd{ESC}8ef{ESC}[s{ESC}[2;2Hz{ESC}[uq")),
    "origin-mode": (6, 10, s(f"{ESC}[2;4r{ESC}[?6h{ESC}[1;1HO{ESC}[?6l")),
    "insert-mode-irm": (2, 10, s(f"abcdef{ESC}[1;2H{ESC}[4hXY{ESC}[4l")),
    "dec-special-graphics": (2, 10, s(f"{ESC}(0lqqk{ESC}(Bx")),
    "sync-output-2026": (2, 10, s(f"{ESC}[?2026hab{ESC}[?2026l")),
    "kitty-keyboard-push": (2, 10, s(f"{ESC}[>1uab{ESC}[<u")),
    "rep-repeat": (2, 20, s(f"x{ESC}[5b")),
    "invalid-utf8": (2, 20, s(b"a\xffb\xc3c")),
    "utf8-split-across-writes": (2, 20, s(b"\xe6\x97", b"\xa5X")),
    "cursor-hide-show": (2, 10, s(f"{ESC}[?25lab{ESC}[?25h")),
    "cup-out-of-range": (4, 10, s(f"{ESC}[99;99HZ")),
    "resize-shrink": (6, 20, s("line one is long\r\nline two\r\nline three", (3, 8))),
    "resize-grow": (3, 8, s("abcdefgh\r\nxyz", (6, 20), "Q")),
    "resize-alt": (4, 20, s(f"{ESC}[?1049halt text", (2, 10))),
    "decaln": (3, 5, s(f"{ESC}#8")),
    "ris-reset": (3, 10, s(f"abc{ESC}[?2004h{ESC}]0;t\x07{ESC}c")),
    "query-da1": (2, 10, s(f"{ESC}[cA")),
    "query-dsr-cpr": (2, 10, s(f"{ESC}[6nB")),
    "query-decrqm": (2, 10, s(f"{ESC}[?2004$pC")),
    "query-osc11": (2, 10, s(f"{ESC}]11;?\x07D")),
    "query-xtversion": (2, 10, s(f"{ESC}[>0qE")),
    "query-kitty-flags": (2, 10, s(f"{ESC}[?uF")),
    "c1-8bit-csi": (2, 10, s(b"\x9b31mA")),
    "newline-mode-lnm": (3, 10, s(f"{ESC}[20ha\nb{ESC}[20l")),
    "nel-ind-ri": (4, 10, s(f"a{ESC}Eb{ESC}Dc{ESC}Md")),
    "vpa-cha-hpa": (4, 10, s(f"{ESC}[3dA{ESC}[5GB{ESC}[2`C")),
    "scroll-up-down-su-sd": (4, 10, s("1\r\n2\r\n3\r\n4", f"{ESC}[1S", f"{ESC}[2T")),
    "write-space-vs-erase": (2, 10, s(f"ab  {ESC}[1;8Hc")),
    "bce-erase-line": (3, 10, s(f"abc{ESC}[45m{ESC}[K{ESC}[0m")),
    "bce-erase-chars": (3, 10, s(f"abcdef{ESC}[1;2H{ESC}[46m{ESC}[3X{ESC}[0m")),
    "bce-insert-lines": (4, 10, s(f"1\r\n2\r\n3{ESC}[2;1H{ESC}[42m{ESC}[1L{ESC}[0m")),
    "bce-scroll-lf": (3, 10, s(f"1\r\n2\r\n3{ESC}[43m\r\n{ESC}[0mx")),
    "bce-erase-display-below": (3, 10, s(f"aaa\r\nbbb{ESC}[1;2H{ESC}[41m{ESC}[J{ESC}[0m")),
    "bce-rgb-erase-line": (2, 10, s(f"{ESC}[48;2;30;30;46m{ESC}[2K{ESC}[0m")),
    "one-column-emoji-presentation": (1, 1, s(b"\xe2\x98\xba\xef\xb8\x8f\xef\xb8\x8f")),
}


def profile_documents():
    paths = sorted(PROFILES.glob("*.json"))
    assert paths, f"no profiles under {PROFILES}"
    return [(path, json.loads(path.read_text())) for path in paths]


def fixture_cases():
    cases = []
    for _, doc in profile_documents():
        for name, fx in doc["screens"].items():
            cases.append(fixture_case(f"fixture:{name}", fx))
    assert cases, "the bundled profiles hold no native screen fixtures"
    return cases


def synthetic_cases():
    return [{"name": f"synthetic:{name}", "rows": rows, "cols": cols, "steps": steps} for name, (rows, cols, steps) in SYNTHETIC.items()]


def main(argv):
    assert len(argv) >= 3, __doc__
    mode, out_path = argv[1], argv[2]
    if mode == "fixtures":
        cases = fixture_cases() + synthetic_cases()
    elif mode == "streams":
        assert len(argv) == 5 and argv[3] == "--corpus", "usage: cases.py streams OUT.jsonl --corpus DIR"
        cases = [stream_case(stream) for stream in corpus_streams(argv[4])]
    else:
        raise SystemExit(f"unknown mode {mode}\n{__doc__}")
    write_jsonl(out_path, cases)
    print(f"{out_path}: {len(cases)} cases")


if __name__ == "__main__":
    main(sys.argv)
