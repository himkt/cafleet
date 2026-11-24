"""Scrub raw terminal recordings into corpus streams and write their privacy review bundle.

Usage:
  scrub.py --out DIR --review DIR [--name NAME]... [--literal TEXT]... [--native-version BACKEND=VERSION]...
           (--live CASE.json | --capture REC.jsonl)...

Inputs:
  --live CASE.json      a live recording: one {"name", "rows", "cols", "steps"} replay case per line
  --capture REC.jsonl   a capture recording: {"start"} then timestamped {"out"|"in"|"resize"|"mark"|"exit"} records

Every match below is replaced, in the child's output bytes only, by a placeholder with the same
byte length and display width (every match and placeholder is ASCII):
  * absolute paths under the capturing user's home, the repository root, the repository root's
    top-level directory and /tmp, and every path a status line elides with a leading U+2026;
  * the capturing host name and every fragment of it a pane edge leaves behind;
  * the capturing user name, every --name, and every --literal (experiment directory names);
  * the name in every work/<name> path segment;
  * native session identifiers: UUIDs, opencode `ses_` ids and hex runs of 32 or more digits.
The host, user and home are read from this machine, so this file names none of them.

Matching runs on the visible text of the joined output, with SGR sequences transparent and other
escape sequences as separators; OSC, DCS, SOS, PM and APC payloads are matched on their own. The
recording's input bytes, launch argv, working directory and exit record are not carried over.

Outputs:
  DIR/live.jsonl.gz, DIR/capture.jsonl.gz   corpus streams (format in cases.py)
  REVIEW/README.md                          per-stream scrub, secret-scan, layout and residual report
  REVIEW/screens/<stream>.txt               the rendered screen at every checkpoint (after every read
                                            for a live stream, which has no timestamps or marks)
  REVIEW/text/<stream>.txt                  every distinct rendered row, in order of first appearance
  REVIEW/prompts.md                         typed input and the rows each turn added

The run fails when a scrubbed stream's per-step layout differs from the raw stream's, or when a
residual scan of the scrubbed bytes or of any rendered screen still finds a scrubbed value.
"""
import argparse
import collections
import getpass
import json
import os
import pathlib
import re
import socket
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cases  # noqa: E402

SECRET_PATTERNS = {
    "sk-key": rb"sk-[A-Za-z0-9_\-]{16,}",
    "openrouter-key": rb"sk-or-[A-Za-z0-9_\-]{8,}",
    "bearer": rb"[Bb]earer [A-Za-z0-9._\-]{16,}",
    "api_key=": rb"(?i)api[_-]?key\s*[=:]\s*\S{8,}",
    "aws-key-id": rb"AKIA[0-9A-Z]{16}",
    "private-key": rb"-----BEGIN [A-Z ]*PRIVATE KEY-----",
    "jwt": rb"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.",
    "password": rb"(?i)password\s*[=:]",
}
PATH_CHARS = rb"A-Za-z0-9._~/+\-"
WORD_CHARS = rb"A-Za-z0-9_"
HOST_FRAGMENT_MIN = 6
HOST_CORE_OVERLAP_MIN = 6
SEPARATOR = 0x00


def fill(template, n):
    return (template * (n // len(template) + 1))[:n]


def path_placeholder(n):
    return b"/" + fill(b"scrubbed/", n - 1)


class Rules:
    def __init__(self, home, repo, names, literals, host):
        top = b"/" + repo.strip(b"/").split(b"/")[0]
        self.path_prefixes = sorted({home, repo, top, b"/tmp"}, key=len, reverse=True)
        self.names = sorted(set(names), key=len, reverse=True)
        self.literals = sorted(set(literals), key=len, reverse=True)
        self.host = host
        self.host_core = next((i for i, b in enumerate(host) if chr(b).isdigit()), len(host))
        prefixes = b"|".join(re.escape(p) for p in self.path_prefixes)
        self.path_re = re.compile(rb"(?<![" + PATH_CHARS + rb"$])(?:" + prefixes + rb")(?![A-Za-z0-9_.-])[" + PATH_CHARS + rb"]*")
        self.word_res = [(re.compile(rb"(?<![" + WORD_CHARS + rb"])" + re.escape(n) + rb"(?![" + WORD_CHARS + rb"])"), fill(b"username", len(n))) for n in self.names]
        self.literal_res = [(re.compile(re.escape(lit)), fill(b"scrubbed", len(lit))) for lit in self.literals]
        self.elided_path_re = re.compile(rb"(?<=\xe2\x80\xa6)[" + PATH_CHARS + rb"]*/[" + PATH_CHARS + rb"]*")
        self.session_res = [
            re.compile(rb"(?<![0-9A-Za-z])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(?![0-9A-Za-z])"),
            re.compile(rb"(?<=ses_)[0-9A-Za-z]{16,}"),
            re.compile(rb"(?<![0-9A-Za-z])[0-9a-f]{32,}(?![0-9A-Za-z])"),
        ]
        self.work_re = re.compile(rb"(?<![A-Za-z0-9_-])work/([A-Za-z0-9._-]+)")
        self.host_placeholder = fill(b"scrubbed-host-", len(host))

    def host_fragments(self):
        """Substrings of the host name that a pane edge can leave on screen: every prefix of at least
        HOST_FRAGMENT_MIN bytes, and every substring overlapping the host's first digit onward by at
        least HOST_CORE_OVERLAP_MIN bytes. Longest first, so a full name wins over its fragments."""
        n, frags = len(self.host), set()
        for i in range(n):
            for j in range(i + 1, n + 1):
                is_prefix = i == 0 and j - i >= HOST_FRAGMENT_MIN
                core_overlap = j - max(i, self.host_core)
                if is_prefix or (core_overlap >= HOST_CORE_OVERLAP_MIN and j - i >= HOST_CORE_OVERLAP_MIN):
                    frags.add((i, j))
        return sorted(frags, key=lambda f: (f[0] - f[1], f[0]))

    def apply(self, text, counts):
        """Scrub one projected text; returns the replaced bytes (same length)."""
        buf = bytearray(text)

        def put(start, replacement, rule):
            assert len(replacement) == len(replacement.decode("ascii")), "placeholders are ASCII"
            buf[start : start + len(replacement)] = replacement
            counts[rule] += 1

        for m in self.path_re.finditer(bytes(buf)):
            put(m.start(), path_placeholder(m.end() - m.start()), "path")
        for m in self.elided_path_re.finditer(bytes(buf)):
            put(m.start(), path_placeholder(m.end() - m.start()), "elided-path")
        for i, j in self.host_fragments():
            fragment = self.host[i:j]
            for m in re.finditer(re.escape(fragment), bytes(buf)):
                put(m.start(), self.host_placeholder[i:j], "host" if (i, j) == (0, len(self.host)) else "host-fragment")
        for pattern, placeholder in self.word_res:
            for m in pattern.finditer(bytes(buf)):
                put(m.start(), placeholder, "user-name")
        for pattern, placeholder in self.literal_res:
            for m in pattern.finditer(bytes(buf)):
                put(m.start(), placeholder, "literal")
        for pattern in self.session_res:
            for m in pattern.finditer(bytes(buf)):
                put(m.start(), fill(b"0", m.end() - m.start()), "session-id")
        for m in self.work_re.finditer(bytes(buf)):
            put(m.start(1), fill(b"scrubbed", m.end(1) - m.start(1)), "work-name")
        assert len(buf) == len(text)
        return bytes(buf)

    def residual_tokens(self):
        tokens = {b"home": self.path_prefixes, b"host": [self.host[i:j] for i, j in self.host_fragments()]}
        tokens[b"name"] = self.names
        tokens[b"literal"] = self.literals
        return tokens


def segments(data):
    """Split output bytes into (kind, start, end) spans: "text" for printable bytes, "sep" for a span
    that ends a visible run, "sgr" for transparent SGR sequences and "payload" for string-sequence
    payloads. Control bytes are "text" of length 1, so they stay in the projection as delimiters."""
    out, i, n = [], 0, len(data)
    while i < n:
        b = data[i]
        if b != 0x1B:
            j = i
            while j < n and data[j] != 0x1B:
                j += 1
            out.append(("text", i, j))
            i = j
            continue
        if i + 1 >= n:
            out.append(("sep", i, n))
            break
        kind = data[i + 1]
        if kind == ord("["):
            j = i + 2
            while j < n and not 0x40 <= data[j] <= 0x7E:
                j += 1
            j = min(j + 1, n)
            out.append(("sgr" if data[j - 1] == ord("m") else "sep", i, j))
            i = j
        elif kind in b"]PX^_":
            j = i + 2
            if kind == ord("]"):
                while j < n and data[j] in b"0123456789":
                    j += 1
                if j < n and data[j] == ord(";"):
                    j += 1
            out.append(("sep", i, j))
            k = j
            while k < n and data[k] != 0x07 and not (data[k] == 0x1B and k + 1 < n and data[k + 1] == ord("\\")) and data[k] not in (0x18, 0x1A):
                k += 1
            out.append(("payload", j, k))
            end = k + (2 if k < n and data[k] == 0x1B else 1 if k < n else 0)
            out.append(("sep", k, min(end, n)))
            i = min(end, n)
        else:
            j = i + 1
            while j < n and 0x20 <= data[j] <= 0x2F:
                j += 1
            j = min(j + 1, n)
            out.append(("sep", i, j))
            i = j
    return out


def scrub_bytes(data, rules, counts):
    """Scrub joined output bytes through their visible-text projection and each string payload."""
    result = bytearray(data)
    projection, positions = bytearray(), []
    for kind, start, end in segments(data):
        if kind == "text":
            projection += data[start:end]
            positions.extend(range(start, end))
        elif kind == "sep":
            projection.append(SEPARATOR)
            positions.append(None)
        elif kind == "payload":
            result[start:end] = rules.apply(data[start:end], counts)
            projection.append(SEPARATOR)
            positions.append(None)
    scrubbed = rules.apply(bytes(projection), counts)
    for k, (before, after) in enumerate(zip(projection, scrubbed)):
        if before != after:
            assert positions[k] is not None, "a match never covers a separator"
            result[positions[k]] = after
    return bytes(result)


def load_live(path):
    streams = []
    for case in cases.read_jsonl(path):
        events = []
        for step in case["steps"]:
            if "hex" in step:
                events.append({"out": step["hex"]})
            else:
                assert "resize" in step, f"{path}: unknown step {sorted(step)}"
                events.append({"resize": step["resize"]})
        streams.append(({"name": f"live:{case['name']}", "rows": case["rows"], "cols": case["cols"], "native_version": None, "events": events}, []))
    return streams


def load_capture(path, versions):
    records = cases.read_jsonl(path)
    start = records[0]["start"]
    backend = start["name"].split("-")[0]
    assert backend in versions, f"{path}: no --native-version for backend {backend}"
    events, typed = [], []
    for r in records[1:]:
        if "out" in r:
            events.append({"t": r["t"], "out": r["out"]})
        elif "resize" in r:
            events.append({"t": r["t"], "resize": r["resize"]})
        elif "mark" in r:
            events.append({"t": r["t"], "mark": r["mark"]})
        elif "in" in r:
            if r["src"] == "ctl":
                typed.append((r["t"], bytes.fromhex(r["in"])))
            else:
                assert r["src"] == "term", f"{path}: unknown input source {r['src']}"
        else:
            assert "exit" in r, f"{path}: unknown record {sorted(r)}"
    stream = {"name": f"capture:{start['name']}", "rows": start["rows"], "cols": start["cols"], "native_version": versions[backend], "events": events}
    return [(stream, typed)]


def scrub_stream(stream, rules):
    outs = [i for i, e in enumerate(stream["events"]) if "out" in e]
    chunks = [bytes.fromhex(stream["events"][i]["out"]) for i in outs]
    counts = collections.Counter()
    joined = scrub_bytes(b"".join(chunks), rules, counts)
    scrubbed = {**stream, "events": [dict(e) for e in stream["events"]]}
    offset = 0
    for i, chunk in zip(outs, chunks):
        scrubbed["events"][i]["out"] = joined[offset : offset + len(chunk)].hex()
        offset += len(chunk)
    assert offset == len(joined)
    return scrubbed, counts, joined


def secret_scan(data):
    hits = {name: len(re.findall(p, data)) for name, p in SECRET_PATTERNS.items()}
    return {k: v for k, v in hits.items() if v}


def residual_hits(data, rules):
    hits = collections.Counter()
    for kind, tokens in rules.residual_tokens().items():
        for token in tokens:
            if kind == b"name":
                pattern = rb"(?<![" + WORD_CHARS + rb"])" + re.escape(token) + rb"(?![" + WORD_CHARS + rb"])"
            elif kind == b"home":
                pattern = re.escape(token) + rb"(?![A-Za-z0-9_.-])"
            else:
                pattern = re.escape(token)
            for m in re.finditer(pattern, data):
                hits[(kind.decode(), data[max(0, m.start() - 30) : m.end() + 30])] += 1
    return hits


def write_case_file(path, case_list):
    cases.write_jsonl(path, case_list)
    return path


def screen_records(oracle, case_path):
    out = subprocess.run([str(oracle), "screens", str(case_path)], check=True, capture_output=True, text=True).stdout
    by_name = collections.defaultdict(list)
    for line in out.splitlines():
        record = json.loads(line)
        by_name[record["name"]].append(record)
    return by_name


def layout_hashes(oracle, case_path):
    table = {}
    for line in cases.run_oracle(oracle, "layout", case_path).splitlines():
        name, *hashes = line.split(" ")
        table[name] = hashes
    return table


def screen_at(records, boundary):
    """The last distinct screen the oracle reported before observation step `boundary`."""
    current = None
    for record in records:
        if record["step"] >= boundary:
            break
        current = record
    return current


def render(record):
    if record is None:
        return ["  (no output yet)"]
    if record.get("panic"):
        return ["  (oracle panic)"]
    lines = [f"  title={record['title']!r} icon={record['icon']!r}"]
    lines += [f"  {i:2d}|{row}" for i, row in enumerate(record["rows"])]
    return lines


def stream_file_name(stream):
    return stream["name"].replace(":", "-")


def review_stream(stream, typed, records, review):
    screens = [f"# {stream['name']} ({stream['rows']}x{stream['cols']} at start, native version {stream['native_version']})", ""]
    timed = any("t" in e for e in stream["events"])
    points = cases.checkpoints(stream) if timed else [(i + 1, f"after-read-{i:03d}") for i in range(len(stream["events"]))]
    for n, (idx, label) in enumerate(points):
        boundary = cases.observation_steps_before(stream, idx)
        record = screen_at(records, boundary)
        t = next((e["t"] for e in reversed(stream["events"][:idx]) if "t" in e), None)
        when = f", t={t}ms" if timed else ""
        screens.append(f"== {cases.checkpoint_name(stream, n, label)} (event {idx}, step {boundary}{when})")
        screens += render(record)
        screens.append("")
    (review / "screens" / f"{stream_file_name(stream)}.txt").write_text("\n".join(screens))

    seen, inventory = set(), []
    for record in records:
        for row in record.get("rows", []):
            text = row.rstrip()
            if text and text not in seen:
                seen.add(text)
                inventory.append(f"{record['step']:6d}|{text}")
        for key in ("title", "icon"):
            text = record.get(key, "")
            if text and ("title:" + text) not in seen:
                seen.add("title:" + text)
                inventory.append(f"{record['step']:6d}|[{key}] {text}")
    (review / "text" / f"{stream_file_name(stream)}.txt").write_text("\n".join(inventory) + "\n")

    section = [f"## {stream['name']}", ""]
    if not typed:
        section += ["No typed input was recorded; every rendered row is in `text/" + stream_file_name(stream) + ".txt`.", ""]
        return section
    steps_before = []
    total = 0
    for e in stream["events"]:
        steps_before.append((e.get("t"), total))
        if "out" in e:
            total += cases.substep_count(bytes.fromhex(e["out"]))
        elif "resize" in e:
            total += 1

    def step_at(t):
        return next((step for et, step in steps_before if et is not None and et >= t), total)

    def rows(step):
        record = screen_at(records, step)
        return [] if record is None or record.get("panic") else [row.strip() for row in record["rows"] if row.strip()]

    starts = [step_at(t) for t, _ in typed] + [total + 1]
    section.append("Each typed input is followed by the rows the screen shows just before the next input that it did not show when the input arrived.")
    section.append("")
    for k, (t, data) in enumerate(typed):
        before = set(rows(starts[k]))
        section.append(f"- t={t}ms typed: `{data.decode('utf-8', 'backslashreplace')!r}`")
        section += [f"    - {text}" for text in rows(starts[k + 1]) if text not in before]
    section.append("")
    return section


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ap.add_argument("--review", required=True, type=pathlib.Path)
    ap.add_argument("--name", action="append", default=[])
    ap.add_argument("--literal", action="append", default=[])
    ap.add_argument("--native-version", action="append", default=[])
    ap.add_argument("--live", action="append", default=[])
    ap.add_argument("--capture", action="append", default=[])
    args = ap.parse_args()
    assert args.live or args.capture, "give at least one --live or --capture recording"

    versions = dict(v.split("=", 1) for v in args.native_version)
    repo = subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True, text=True).stdout.strip()
    home = os.path.expanduser("~")
    assert home.startswith("/") and home != "/", f"unexpected home {home!r}"
    rules = Rules(
        home=home.encode(),
        repo=repo.encode(),
        names=[getpass.getuser().encode(), *(n.encode() for n in args.name)],
        literals=[lit.encode() for lit in args.literal],
        host=socket.gethostname().encode(),
    )

    loaded = [s for p in args.live for s in load_live(p)] + [s for p in args.capture for s in load_capture(p, versions)]
    args.out.mkdir(parents=True, exist_ok=True)
    for sub in ("screens", "text", ".cases"):
        (args.review / sub).mkdir(parents=True, exist_ok=True)
    oracle = cases.build_oracle()

    report, prompts, failures = [], ["# Typed input and the rows each turn added", ""], []
    results = []
    for stream, typed in loaded:
        scrubbed, counts, joined = scrub_stream(stream, rules)
        results.append((stream, scrubbed, counts, joined, typed))

    raw_cases = write_case_file(args.review / ".cases" / "raw.jsonl", [cases.stream_case(s) for s, *_ in results])
    scrubbed_cases = write_case_file(args.review / ".cases" / "scrubbed.jsonl", [cases.stream_case(s) for _, s, *_ in results])
    raw_layout, scrubbed_layout = layout_hashes(oracle, raw_cases), layout_hashes(oracle, scrubbed_cases)
    screens = screen_records(oracle, scrubbed_cases)

    for stream, scrubbed, counts, joined, typed in results:
        name = stream["name"]
        layout_ok = raw_layout[name] == scrubbed_layout[name]
        byte_hits = residual_hits(joined, rules)
        screen_hits = collections.Counter()
        for record in screens[name]:
            text = "\n".join([record.get("title", ""), record.get("icon", ""), record.get("contents", ""), *record.get("rows", [])]).encode()
            screen_hits.update(residual_hits(text, rules))
        secrets = secret_scan(joined)
        steps = len(scrubbed_layout[name])
        report.append((name, len(joined), steps, dict(counts), secrets, layout_ok, sum(byte_hits.values()), sum(screen_hits.values())))
        if not layout_ok:
            failures.append(f"{name}: scrubbed layout differs from the raw stream")
        for (kind, context), count in sorted(byte_hits.items()) + sorted(screen_hits.items()):
            failures.append(f"{name}: residual {kind} x{count}: {context!r}")
        if secrets:
            failures.append(f"{name}: secret-shaped strings {secrets}")
        prompts += review_stream(scrubbed, typed, screens[name], args.review)

    groups = {"live": [s for _, s, *_ in results if s["name"].startswith("live:")], "capture": [s for _, s, *_ in results if s["name"].startswith("capture:")]}
    for group, streams in groups.items():
        if streams:
            cases.write_corpus(args.out / f"{group}.jsonl.gz", streams)

    lines = [
        "# Captured-stream privacy review",
        "",
        "Each stream below is scrubbed. Sign off per stream after reading its rendered checkpoints in `screens/`, its row inventory in `text/` and the typed input in `prompts.md`.",
        "",
        "| Stream | Output bytes | Observation steps | Replacements | Secret scan | Layout identical | Residual hits (bytes / screens) |",
        "|---|---|---|---|---|---|---|",
    ]
    for name, nbytes, steps, counts, secrets, layout_ok, byte_hits, screen_hits in report:
        repl = ", ".join(f"{k} {v}" for k, v in sorted(counts.items())) or "—"
        lines.append(f"| `{name}` | {nbytes} | {steps} | {repl} | {secrets or 'clean'} | {'yes' if layout_ok else 'no'} | {byte_hits} / {screen_hits} |")
    lines += ["", f"Corpus files: {', '.join(str(args.out / f'{g}.jsonl.gz') for g, s in groups.items() if s)}", ""]
    if failures:
        lines += ["## Failures", ""] + [f"- {f}" for f in failures[:200]] + [""]
    (args.review / "README.md").write_text("\n".join(lines))
    (args.review / "prompts.md").write_text("\n".join(prompts))
    print("\n".join(lines))
    if failures:
        raise SystemExit(f"{len(failures)} scrub check failures; see {args.review / 'README.md'}")


if __name__ == "__main__":
    main()
