"""List every observed stderr line of a harness run that SPEC.md does not state.

  python3 -I migration/harness/spec_inventory.py RECORDS_JSON [--spec SPEC.md] [--out FILE]

RECORDS_JSON is a `harness.py --records` file. Every code span and code-block line of SPEC.md
becomes a template: `<placeholder>`, `{placeholder}` and upper-case placeholder words (`ID`,
`MEMBER_ID`, `CAUSE`, ...) match any text, and the `Error: ` prefix is optional on both sides.
A template needs at least ten constant alphanumeric characters, so a bare column header or a
lone placeholder states nothing.

A line no template matches is unstated. When a template matches, the text each cause-like
placeholder (`CAUSE`, `<cause>`, `<error>`, `<io error>`, `<detail>`, `REASON`, ...) captured is
checked the same way, so a stated wrapper around an unstated cause is still reported. Causes
that are platform or SQLite library texts are listed separately: amendment A2 restates them as
behavior. Parser diagnostics (an exit-2 stderr that does not start with `Error: `) and help
bodies follow SPEC §6.3's generic parse-error and help rules, so they are skipped.
"""
import argparse
import collections
import json
import os
import re

PLACEHOLDER = re.compile(r"<[^<>\n]{1,60}>|\{[a-z_]+\}|\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)*\b")
CAUSE_WORDS = ("cause", "error", "detail", "reason", "message", "diagnostic")
NORMALIZED = re.compile(r"<(?:TS|PID|START|HEX|ID|NS|DEV|INO|IDLE|DUR)\d+>")
PREFIX = r"(?:Error: |error: |Warning: |warning: )?"
OS_ERROR = re.compile(r".*\(os error \d+\)")
SQLITE_TEXTS = ("database is locked", "file is not a database", "disk I/O error", "unable to open database file",
                "attempt to write a readonly database", "database or disk is full", "interrupted")


class Template:
    def __init__(self, raw):
        self.raw = raw
        pieces, last, self.causes = [], 0, []
        for k, match in enumerate(PLACEHOLDER.finditer(raw)):
            pieces.append(re.escape(raw[last:match.start()]))
            if any(word in match.group(0).lower() for word in CAUSE_WORDS):
                pieces.append(f"(?P<c{k}>.+?)")
                self.causes.append(f"c{k}")
            else:
                pieces.append(r".+?")
            last = match.end()
        pieces.append(re.escape(raw[last:]))
        self.pattern = re.compile(PREFIX + "".join(pieces) + r"\.?")
        self.constant = len(re.sub(r"[^A-Za-z0-9]", "", PLACEHOLDER.sub("", raw)))


def templates(spec_text):
    spans = re.findall(r"`([^`\n]+)`", spec_text)
    blocks = re.findall(r"```[a-z]*\n(.*?)```", spec_text, re.S)
    out, seen = [], set()
    for raw in spans + [line for block in blocks for line in block.splitlines()]:
        raw = raw.strip().replace("&#124;", "|").replace("&lt;", "<").replace("&gt;", ">")
        if raw in seen:
            continue
        seen.add(raw)
        template = Template(raw)
        if template.constant >= 10:
            out.append(template)
    out.sort(key=lambda t: -t.constant)
    return out


def library_text(text):
    text = text.strip().rstrip(".")
    return bool(OS_ERROR.fullmatch(text)) or text in SQLITE_TEXTS or text.startswith(("exit status: ", "signal: "))


def classify(text, compiled, depth=0):
    """(template raw or None, [unstated cause texts], [library cause texts]) for one text."""
    body = NORMALIZED.sub("X", text)
    for template in compiled:
        match = template.pattern.fullmatch(body) or template.pattern.fullmatch(body.removeprefix("Error: "))
        if not match:
            continue
        unstated, library = [], []
        for group in template.causes:
            cause = match.group(group)
            if library_text(cause):
                library.append(cause)
                continue
            if depth >= 3:
                continue
            inner, inner_unstated, inner_library = classify(cause, compiled, depth + 1)
            if inner is None and not short_value(cause):
                unstated.append(cause)
            unstated += inner_unstated
            library += inner_library
        return template.raw, unstated, library
    if library_text(body):
        return "<library text>", [], [body]
    return None, [], []


def short_value(text):
    """A captured value that is a name, id, path or quoted literal rather than a message."""
    return len(text) <= 24 and " " not in text.strip() or re.fullmatch(r"'[^']*'|\"[^\"]*\"|X|/\S*", text) is not None


def observed_lines(records, stream="stderr"):
    """Distinct non-blank lines of `stream`; stdout keeps only text lines (JSON is shaped by
    SPEC's ordered key lists, not by line templates)."""
    seen = collections.OrderedDict()
    for name, runs in records.items():
        run = runs[0] if isinstance(runs, list) else runs
        if run.get("error"):
            continue
        for step in run["steps"]:
            if step["kind"] in ("action", "help"):
                continue
            lines = [line for line in step[stream].split("\n") if line.strip()]
            if stream == "stdout":
                lines = [line for line in lines if not line.startswith(("{", "[{", "[]", "<"))]
            if step["exit"] == 2 and lines and not lines[0].startswith("Error: "):
                continue
            for line in lines:
                seen.setdefault(line, (name, step["label"], step["exit"]))
    return seen


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("records", nargs="+")
    parser.add_argument("--spec", default=os.path.join(os.path.dirname(__file__), os.pardir, os.pardir, "SPEC.md"))
    parser.add_argument("--out")
    args = parser.parse_args()
    with open(args.spec) as f:
        compiled = templates(f.read())
    records = {}
    for path in args.records:
        with open(path) as f:
            records.update(json.load(f))
    unstated_lines, unstated_causes, library = [], collections.OrderedDict(), collections.OrderedDict()
    for line, origin in observed_lines(records).items():
        raw, causes, libs = classify(line, compiled)
        if raw is None:
            unstated_lines.append((line, origin))
        for cause in causes:
            unstated_causes.setdefault(NORMALIZED.sub("X", cause), (line, origin))
        for text in libs:
            library.setdefault(text, (line, origin))
    out = ["## Unstated lines", ""]
    out += [f"- `{line}` — {o[0]}: {o[1]} (exit {o[2]})" for line, o in unstated_lines]
    out += ["", "## Unstated causes inside stated templates", ""]
    out += [f"- `{cause}` — in `{line}` — {o[0]}: {o[1]}" for cause, (line, o) in unstated_causes.items()]
    out += ["", "## Library-defined cause texts (A2)", ""]
    out += [f"- `{text}` — in `{line}` — {o[0]}: {o[1]}" for text, (line, o) in library.items()]
    shapes = collections.OrderedDict()
    for line, origin in observed_lines(records, "stdout").items():
        if classify(line, compiled)[0] is None:
            shapes.setdefault(re.sub(r"\d+", "N", NORMALIZED.sub("X", line)), (line, origin))
    out += ["", "## Unstated stdout text lines (digits shown as N)", ""]
    out += [f"- `{shape}` — e.g. `{line}` — {o[0]}: {o[1]}" for shape, (line, o) in shapes.items()]
    text = "\n".join(out) + "\n"
    if args.out:
        with open(args.out, "w") as f:
            f.write(text)
    print(text)
    print(f"{len(unstated_lines)} unstated lines, {len(unstated_causes)} unstated causes, {len(library)} library texts")


if __name__ == "__main__":
    main()
