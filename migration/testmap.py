"""Gate G5: map every Rust #[test] to its Go counterpart.

Usage:
  testmap.py list [OUT.tsv]   derive the Go file and test name of every Rust test
  testmap.py check [OUT.tsv]  also require each derived test (or its omission's replacement) in Go

The Rust tests are the ones `cargo test --bin cafleet -- --list` reports, minus BRANCH_ONLY. Each
maps by the design's rules:
  file   the Rust source file owning the test's module path, through the file mapping
         (src/<a>.rs and src/<a>/mod.rs -> <a>.go, src/<a>/<b>.rs -> <a>_<b>.go, ..., src/main.rs -> main.go),
         whose test file is internal/core/<stem>_test.go
  name   Test<File>_<Fn> for a test directly in `mod tests`, Test<File>_<Nested1>_..._<NestedN>_<Fn>
         otherwise, with <File> the CamelCase file stem and every other part the CamelCase of its
         Rust module or function name
OMISSIONS lists the Rust tests that assert Rust-library internals, each with the Go test asserting
the same SPEC property instead.

list fails on a derived-name collision; check also fails on any Rust test without its Go
counterpart, a duplicate Go test name, or an omission whose replacement is missing. While
internal/core is absent, check prints SKIP for the Go side. OUT.tsv receives one row per Rust test:
rust path, Go file, Go test, status.
"""
import collections
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
SRC = REPO / "src"
GO_CORE = REPO / "internal" / "core"
BRANCH_ONLY = {"coding_agent::detector::capture_golden::export"}
OMISSIONS = {}
GO_TEST_RE = re.compile(r"^func (Test\w+)\(t \*testing\.T\)", re.MULTILINE)


def camel(name):
    return "".join(part[:1].upper() + part[1:] for part in name.removeprefix("r#").split("_") if part)


def rust_tests():
    out = subprocess.run(["cargo", "test", "--bin", "cafleet", "--quiet", "--", "--list", "--format", "terse"], check=True, cwd=REPO, capture_output=True, text=True).stdout
    tests = []
    for line in out.splitlines():
        path, kind = line.rsplit(": ", 1)
        assert kind == "test", f"unexpected listing entry {line!r}"
        if path not in BRANCH_ONLY:
            tests.append(path)
    assert tests, "cargo listed no tests"
    return tests


def module_files():
    """Module path (tuple of segments) -> Go file stem, for every Rust source file."""
    files = {}
    for path in SRC.rglob("*.rs"):
        rel = path.relative_to(SRC).with_suffix("")
        parts = list(rel.parts)
        if parts == ["main"]:
            parts = []
        elif parts[-1] == "mod":
            parts = parts[:-1]
        stem = "_".join(parts) if parts else "main"
        assert tuple(parts) not in files, f"two files own module {parts}"
        files[tuple(parts)] = stem
    return files


def derive(path, files):
    segments = path.split("::")
    owner = max((k for k in files if tuple(segments[: len(k)]) == k and len(k) < len(segments)), key=len)
    stem = files[owner]
    rest = segments[len(owner) :]
    if rest[0] == "tests":
        rest = rest[1:]
    assert rest, f"{path}: no test function after the module path"
    name = "Test" + camel(stem) + "_" + "_".join(camel(s) for s in rest)
    return f"internal/core/{stem}_test.go", name


def go_tests():
    found = collections.defaultdict(list)
    for path in sorted(GO_CORE.glob("*_test.go")):
        for name in GO_TEST_RE.findall(path.read_text()):
            found[name].append(f"internal/core/{path.name}")
    return found


def main(argv):
    assert len(argv) in (2, 3) and argv[1] in ("list", "check"), __doc__
    files = module_files()
    rows, by_name, failures = [], collections.defaultdict(list), []
    for path in rust_tests():
        go_file, go_name = derive(path, files)
        by_name[go_name].append(path)
        rows.append([path, go_file, go_name, "derived"])
    for name, paths in sorted(by_name.items()):
        if len(paths) > 1:
            failures.append(f"derived Go name {name} collides: {paths}")
    for path in OMISSIONS:
        assert any(r[0] == path for r in rows), f"omission {path} names no current Rust test"

    if argv[1] == "check":
        if not GO_CORE.is_dir():
            print("G5: Go comparison SKIP (no internal/core)")
        else:
            found = go_tests()
            for name, places in sorted(found.items()):
                if len(places) > 1:
                    failures.append(f"Go test {name} defined in {places}")
            for row in rows:
                path, go_file, go_name, _ = row
                if path in OMISSIONS:
                    rep_file, rep_name = OMISSIONS[path]
                    row[1], row[2] = rep_file, rep_name
                    row[3] = "omitted" if rep_file in found.get(rep_name, []) else "omitted-replacement-missing"
                else:
                    row[3] = "mapped" if go_file in found.get(go_name, []) else "missing"
                if row[3] not in ("mapped", "omitted"):
                    failures.append(f"{path}: {row[3]} {row[1]} {row[2]}")

    if len(argv) == 3:
        with open(argv[2], "w") as out:
            out.write("rust_test\tgo_file\tgo_test\tstatus\n")
            for row in rows:
                out.write("\t".join(row) + "\n")
    status = collections.Counter(r[3] for r in rows)
    print(f"G5: {len(rows)} Rust tests in {len({r[1] for r in rows})} Go test files; " + ", ".join(f"{k} {v}" for k, v in sorted(status.items())))
    for failure in failures[:50]:
        print(f"  {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
