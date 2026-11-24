"""Gate G4: the Rust detector's reference results and their comparison with the Go detector.

Usage:
  golden.py export OUT.jsonl [--corpus DIR]
  golden.py compare REFERENCE.jsonl [--corpus DIR]

export   Runs the ignored `capture_golden` test of src/coding_agent/detector.rs, which replays every
         bundled native screen fixture as captured and, with --corpus, every capture checkpoint
         through the production TerminalState, and writes one record per screen: size, cursor, screen mode,
         observation kind, rule_id, match evidence, verified_noninteraction, matched region,
         operation, choices and composer text.
compare  Runs the Go side on the same screens and reports every record whose fields differ from
         REFERENCE. While GO_DETECTOR is absent the comparison prints SKIP.

Checkpoints come from every capture stream of the local stream corpus at DIR (format in cases.py),
at the boundaries cases.checkpoints defines; each checkpoint is named <stream>:<NNN>:<label>.
Without --corpus the checkpoint list is empty and only the fixtures are recorded.

The composer text is recorded structurally:
  {"region": [top, bottom], "rows": n, "mode_prefix": str | null,
   "segments": [{"text": str, "breaks": [char offsets of word-wrap breaks]} | {"scalars": n} | {"lines": n}]}

The Go side is the environment-driven test GO_TEST in GO_PACKAGE. It reads
CAFLEET_GOLDEN_CHECKPOINTS and writes CAFLEET_GOLDEN_OUT in the format above, and skips when
CAFLEET_GOLDEN_CHECKPOINTS is unset.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cases  # noqa: E402

GO_DETECTOR = cases.REPO / "internal" / "core" / "coding_agent_detector.go"
GO_PACKAGE = "./internal/core"
GO_TEST = "TestCodingAgentDetector_GoldenExport"
FIELDS = ["size", "cursor", "alternate_screen", "kind", "rule_id", "match_evidence", "verified_noninteraction", "matched_region", "operation", "choices", "composer_text"]


def checkpoint_streams(streams):
    out = []
    for stream in streams:
        backend = stream["name"].split(":", 1)[1].split("-")[0]
        assert stream["native_version"], f"{stream['name']} records no native version"
        case = cases.stream_case(stream)
        points = []
        for n, (idx, label) in enumerate(cases.checkpoints(stream)):
            fed = sum(1 for e in stream["events"][:idx] if "mark" not in e)
            points.append({"name": cases.checkpoint_name(stream, n, label), "steps": fed})
        out.append({**case, "backend": backend, "native_version": stream["native_version"], "checkpoints": points})
    return out


class DebugParser:
    """Parses the Rust `{:?}` rendering of a ComposerText into Python values."""

    TOKEN = re.compile(r"""\s*(?:(?P<char>'(?:\\u\{[0-9a-f]+\}|\\.|[^\\'])')|(?P<str>"(?:\\u\{[0-9a-f]+\}|\\.|[^\\"])*")|(?P<num>\d+)|(?P<ident>[A-Za-z_][A-Za-z0-9_]*)|(?P<punct>[{}()\[\],:]))""")
    ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", "'": "'", '"': '"'}

    def __init__(self, text):
        self.tokens, pos = [], 0
        while pos < len(text):
            m = self.TOKEN.match(text, pos)
            assert m, f"unparseable Debug text at {pos}: {text[pos:pos + 40]!r}"
            kind = m.lastgroup
            self.tokens.append((kind, m.group(kind)))
            pos = m.end()
        self.i = 0

    @classmethod
    def unescape(cls, body):
        out, i = [], 0
        while i < len(body):
            c = body[i]
            if c != "\\":
                out.append(c)
                i += 1
            elif body[i + 1] == "u":
                end = body.index("}", i)
                out.append(chr(int(body[i + 3 : end], 16)))
                i = end + 1
            else:
                out.append(cls.ESCAPES[body[i + 1]])
                i += 2
        return "".join(out)

    def peek(self):
        return self.tokens[self.i] if self.i < len(self.tokens) else (None, None)

    def take(self, value=None):
        tok = self.tokens[self.i]
        assert value is None or tok[1] == value, f"expected {value!r}, got {tok!r}"
        self.i += 1
        return tok

    def items(self, close):
        items = []
        while self.peek()[1] != close:
            items.append(self.value())
            if self.peek()[1] == ",":
                self.take(",")
        self.take(close)
        return items

    def value(self):
        kind, text = self.take()
        if kind == "num":
            return int(text)
        if kind == "char":
            return ("char", self.unescape(text[1:-1]))
        if kind == "str":
            return self.unescape(text[1:-1])
        if text == "[":
            return self.items("]")
        assert kind == "ident", f"unexpected token {text!r}"
        nxt = self.peek()[1]
        if nxt == "(":
            self.take("(")
            return (text, self.items(")"))
        if nxt == "{":
            self.take("{")
            fields = {}
            while self.peek()[1] != "}":
                key = self.take()[1]
                self.take(":")
                fields[key] = self.value()
                if self.peek()[1] == ",":
                    self.take(",")
            self.take("}")
            return {"__type": text, **fields}
        return (text, None)


def structured_composer(debug):
    if debug is None:
        return None
    parser = DebugParser(debug)
    value = parser.value()
    assert parser.i == len(parser.tokens), f"trailing Debug text in {debug!r}"
    assert value["__type"] == "ComposerText", debug
    segments = []
    for name, args in value["segments"]:
        if name == "Text":
            text, breaks = [], []
            for item in args[0]:
                if item == ("Break", None):
                    breaks.append(len(text))
                else:
                    assert item[0] == "Char" and item[1][0][0] == "char", f"unknown text item {item}"
                    text.append(item[1][0][1])
            segments.append({"text": "".join(text), "breaks": breaks})
        else:
            assert name in ("Scalars", "Lines"), f"unknown segment {name}"
            segments.append({name.lower(): args[0]})
    mode_name, mode_args = value["mode_prefix"]
    assert mode_name in ("Some", "None"), value["mode_prefix"]
    region = value["region"]
    return {
        "region": [region["top"], region["bottom"]],
        "rows": value["rows"],
        "mode_prefix": mode_args[0] if mode_name == "Some" else None,
        "segments": segments,
    }


def write_checkpoints(path, corpus):
    streams = cases.corpus_streams(corpus, prefix="capture:") if corpus is not None else []
    cases.write_jsonl(path, checkpoint_streams(streams))


def export(out_path, corpus):
    with tempfile.TemporaryDirectory() as tmp:
        checkpoints = os.path.join(tmp, "checkpoints.jsonl")
        raw = os.path.join(tmp, "raw.jsonl")
        write_checkpoints(checkpoints, corpus)
        env = {**os.environ, "CAFLEET_GOLDEN_CHECKPOINTS": checkpoints, "CAFLEET_GOLDEN_OUT": raw}
        subprocess.run(["cargo", "test", "--bin", "cafleet", "capture_golden", "--", "--ignored"], check=True, cwd=cases.REPO, env=env)
        records = cases.read_jsonl(raw)
    for record in records:
        record["composer_text"] = structured_composer(record["composer_text"])
    names = [r["name"] for r in records]
    assert len(names) == len(set(names)), "golden record names are unique"
    cases.write_jsonl(out_path, records)
    print(f"{out_path}: {sum(n.startswith('fixture:') for n in names)} fixtures, {sum(not n.startswith('fixture:') for n in names)} capture checkpoints")


def compare(reference_path, corpus):
    reference = {r["name"]: r for r in cases.read_jsonl(reference_path)}
    if not GO_DETECTOR.exists():
        print(f"G4: {len(reference)} reference records; Go comparison SKIP (no {GO_DETECTOR.relative_to(cases.REPO)})")
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        checkpoints = os.path.join(tmp, "checkpoints.jsonl")
        out = os.path.join(tmp, "go.jsonl")
        write_checkpoints(checkpoints, corpus)
        env = {**os.environ, "CAFLEET_GOLDEN_CHECKPOINTS": checkpoints, "CAFLEET_GOLDEN_OUT": out}
        subprocess.run(["go", "test", GO_PACKAGE, "-run", f"^{GO_TEST}$", "-count=1"], check=True, cwd=cases.REPO, env=env, stdout=subprocess.DEVNULL)
        assert os.path.exists(out), f"{GO_TEST} wrote no output; is the export test missing?"
        got = {r["name"]: r for r in cases.read_jsonl(out)}
    bad = 0
    for name in sorted(set(reference) | set(got)):
        if name not in got or name not in reference:
            print(f"--- {name}: only in {'reference' if name in reference else 'Go output'}")
            bad += 1
            continue
        diffs = [f for f in FIELDS if reference[name][f] != got[name][f]]
        if diffs:
            bad += 1
            for f in diffs:
                print(f"--- {name}: {f}\n   rust: {json.dumps(reference[name][f], ensure_ascii=False)}\n   go  : {json.dumps(got[name][f], ensure_ascii=False)}")
    print(f"G4: {len(reference) - bad}/{len(reference)} records equal")
    return 1 if bad else 0


def main(argv):
    assert len(argv) in (3, 5) and argv[1] in ("export", "compare") and (len(argv) == 3 or argv[3] == "--corpus"), __doc__
    corpus = argv[4] if len(argv) == 5 else None
    if argv[1] == "export":
        export(argv[2], corpus)
        return 0
    return compare(argv[2], corpus)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
