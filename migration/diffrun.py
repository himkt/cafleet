"""Gate G3 comparator: the Rust vt100 oracle (migration/vtoracle) against the Go emulator.

Usage:
  diffrun.py [--go-binary PATH] emulator [--corpus DIR] [--max-detail N] [CASES.jsonl...]
  diffrun.py [--go-binary PATH] widths [--strings N]

emulator  Hashes the canonical screen dump after every observation step on both sides and reports,
          per case group, identical cases, compared steps and oracle panics. For each mismatching
          case it prints both dumps' differing lines at the first differing step. With no CASES it
          generates the fixture, synthetic and seeded fuzz cases, and with --corpus DIR also the
          live and capture streams of the local stream corpus and the mutated fuzz family.
widths    Compares the width of every Unicode scalar value and of N seeded strings rich in emoji,
          ZWJ, variation-selector, regional-indicator, tag and ligature-trigger characters.

The Go side is an environment-driven test, GO_TESTS[mode] (package and test name): the emulator
modes run in internal/vt and the width modes in internal/vt/unicodewidth. It reads
  VTPARITY_MODE   hash | dump | strwidth | charwidths
  VTPARITY_INPUT  the case or string file (unused for charwidths)
  VTPARITY_ARGS   for dump, "<case name> <step>"
  VTPARITY_OUT    the file it writes, byte-identical to `vtoracle MODE INPUT ARGS` stdout (for
                  charwidths, to the table vtoracle writes)
and skips when VTPARITY_MODE is unset. While GO_EMULATOR_DIR is absent the Go side does not exist
yet: the run prints SKIP for the comparison and checks only that the oracle replays every case.
--go-binary PATH replaces the Go test with a standalone dumper taking vtoracle's command line, such
as the exp-vt port's vtdump. Exit status 1 means at least one difference.
"""
import argparse
import collections
import os
import random
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cases  # noqa: E402
import fuzz  # noqa: E402

GO_EMULATOR_DIR = cases.REPO / "internal" / "vt"
EMULATOR_TEST = ("./internal/vt", "TestParser_OracleParity")
WIDTH_TEST = ("./internal/vt/unicodewidth", "TestWidth_OracleParity")
GO_TESTS = {"hash": EMULATOR_TEST, "dump": EMULATOR_TEST, "final": EMULATOR_TEST, "strwidth": WIDTH_TEST, "charwidths": WIDTH_TEST}
STRWIDTH_SEED = 7
STRWIDTH_POOL = (
    [0x23, 0x2A, 0x30, 0x39, 0x41, 0x0A, 0x0D, 0xA9, 0x200D, 0xFE0F, 0xFE0E, 0xFE01, 0x20E3, 0x2018, 0x201C, 0x2764, 0x263A, 0x270C, 0x1F3F4, 0x1F3F3]
    + list(range(0x1F1E6, 0x1F1EA))
    + list(range(0x1F3FB, 0x1F400))
    + [0x1F468, 0x1F469, 0x1F466, 0x1F44D, 0x1F600, 0x1F9D1]
    + list(range(0xE0061, 0xE0064))
    + [0xE007F, 0xE0030, 0xE0031]
    + [0x627, 0x644, 0x622, 0x6B5, 0x64B, 0x670, 0x5DC, 0x5D0]
    + [0x1780, 0x17D2, 0x17D8, 0x1A10, 0x1A17, 0x1A15, 0x2D31, 0x2D7F, 0x2D6F, 0xA4FC, 0xA4F8, 0x10C03, 0x10C32, 0x16D67, 0x16D68, 0x16D63, 0x16D69]
    + [0x301, 0x34F, 0x17B4, 0x180B, 0xE0100, 0x2600, 0x2B50, 0x231A, 0x3030, 0x303D, 0x3297, 0xFF9E, 0x1100, 0x1160, 0x65E5]
)


GO_BINARY = None


def go_side_present():
    return GO_BINARY is not None or GO_EMULATOR_DIR.is_dir()


def run_go(mode, input_path, *args):
    """The Go side's output for `vtoracle MODE INPUT ARGS`; for charwidths, the table it writes."""
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "out")
        if GO_BINARY is not None:
            if mode == "charwidths":
                subprocess.run([GO_BINARY, mode, out], check=True)
            else:
                return subprocess.run([GO_BINARY, mode, str(input_path), *map(str, args)], check=True, capture_output=True).stdout
        else:
            package, test = GO_TESTS[mode]
            env = {**os.environ, "VTPARITY_MODE": mode, "VTPARITY_INPUT": str(input_path), "VTPARITY_ARGS": " ".join(map(str, args)), "VTPARITY_OUT": out}
            subprocess.run(["go", "test", package, "-run", f"^{test}$", "-count=1"], check=True, cwd=cases.REPO, env=env, stdout=subprocess.DEVNULL)
        assert os.path.exists(out), f"the Go side wrote no {mode} output; is its parity test missing?"
        with open(out, "rb") as f:
            return f.read()


def hash_table(text):
    table = {}
    for line in text.splitlines():
        name, *hashes = line.split(" ")
        assert name not in table, f"duplicate case name {name}"
        table[name] = hashes
    return table


def group_of(name):
    parts = name.split(":")
    return ":".join(parts[:2]) if len(parts) > 2 else parts[0]


def compare_emulator(oracle, case_path, max_detail, with_go):
    t0 = time.monotonic()
    ref = hash_table(cases.run_oracle(oracle, "hash", case_path))
    rust_s = time.monotonic() - t0
    got, go_s = None, None
    if with_go:
        t0 = time.monotonic()
        got = hash_table(run_go("hash", case_path).decode())
        go_s = time.monotonic() - t0
    totals, exact, steps, panics = collections.Counter(), collections.Counter(), collections.Counter(), collections.Counter()
    mismatches = []
    for name, rh in ref.items():
        g = group_of(name)
        totals[g] += 1
        steps[g] += len(rh)
        if "PANIC" in rh:
            panics[g] += 1
        if got is None:
            continue
        gh = got.get(name, [])
        if gh == rh:
            exact[g] += 1
            continue
        first = next((i for i, (a, b) in enumerate(zip(rh, gh)) if a != b), min(len(rh), len(gh)))
        mismatches.append((name, first, len(rh), len(gh)))
    if got is not None:
        for name in sorted(set(got) - set(ref)):
            mismatches.append((name, 0, 0, len(got[name])))
    print(f"== {case_path}: rust {rust_s:.2f}s" + (f", go {go_s:.2f}s" if go_s is not None else ", go SKIP (no internal/vt)"))
    for g in sorted(totals):
        same = f"{exact[g]}/{totals[g]} cases identical" if got is not None else f"{totals[g]} cases replayed"
        print(f"{g}: {same}, {steps[g]} steps, oracle panics in {panics[g]} cases")
    for name, first, rn, gn in mismatches[:max_detail]:
        print(f"--- {name}: first differing step {first} (rust {rn} steps, go {gn} steps)")
        a = cases.run_oracle(oracle, "dump", case_path, name, first).splitlines() if rn else []
        b = run_go("dump", case_path, name, first).decode().splitlines() if gn else []
        shown = 0
        for i in range(max(len(a), len(b))):
            la = a[i] if i < len(a) else "<none>"
            lb = b[i] if i < len(b) else "<none>"
            if la != lb and shown < 8:
                print(f"   rust: {la[:220]}\n   go  : {lb[:220]}")
                shown += 1
    return sum(totals.values()), sum(steps.values()), len(mismatches)


def generated_case_files(tmp, corpus):
    sets = {"fixtures": cases.fixture_cases() + cases.synthetic_cases(), "fuzz": fuzz.generate(corpus)}
    if corpus is not None:
        sets["streams"] = [cases.stream_case(stream) for stream in cases.corpus_streams(corpus)]
    paths = []
    for name, case_list in sets.items():
        path = os.path.join(tmp, f"{name}.jsonl")
        cases.write_jsonl(path, case_list)
        paths.append(path)
    return paths


def emulator(args):
    oracle = cases.build_oracle()
    with_go = go_side_present()
    total_cases = total_steps = total_bad = 0
    with tempfile.TemporaryDirectory() as tmp:
        paths = args.cases or generated_case_files(tmp, args.corpus)
        for path in paths:
            n, s, bad = compare_emulator(oracle, path, args.max_detail, with_go)
            total_cases, total_steps, total_bad = total_cases + n, total_steps + s, total_bad + bad
    verdict = f"{total_bad} mismatching cases" if with_go else "Go comparison SKIP"
    print(f"TOTAL: {total_cases} cases, {total_steps} steps; {verdict}")
    return 1 if total_bad else 0


def widths(args):
    oracle = cases.build_oracle()
    with_go = go_side_present()
    with tempfile.TemporaryDirectory() as tmp:
        rng = random.Random(STRWIDTH_SEED)
        strings = os.path.join(tmp, "strings.txt")
        with open(strings, "w") as f:
            for _ in range(args.strings):
                s = "".join(chr(rng.choice(STRWIDTH_POOL)) for _ in range(rng.randint(1, 8)))
                f.write(s.encode().hex() + "\n")
        rust_str = cases.run_oracle(oracle, "strwidth", strings).split()
        assert len(rust_str) == args.strings, "the oracle reported a width for every string"
        rust_chars = os.path.join(tmp, "rust-widths.bin")
        cases.run_oracle(oracle, "charwidths", rust_chars)
        with open(rust_chars, "rb") as f:
            rust_table = f.read()
        assert len(rust_table) == 0x110000, "the oracle reported every scalar value"
        if not with_go:
            print(f"widths: {len(rust_table)} scalars and {args.strings} strings measured by the oracle; Go comparison SKIP (no internal/vt)")
            return 0
        go_str = run_go("strwidth", strings).decode().split()
        go_table = run_go("charwidths", "")
    bad_chars = [cp for cp in range(0x110000) if cp >= len(go_table) or rust_table[cp] != go_table[cp]]
    bad_strs = [i for i in range(args.strings) if i >= len(go_str) or rust_str[i] != go_str[i]]
    print(f"widths: {len(bad_chars)}/{len(rust_table)} scalars differ, {len(bad_strs)}/{args.strings} strings differ")
    for cp in bad_chars[:10]:
        print(f"   U+{cp:04X}: rust {chr(rust_table[cp])} go {chr(go_table[cp]) if cp < len(go_table) else '<none>'}")
    return 1 if bad_chars or bad_strs else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    em = sub.add_parser("emulator")
    em.add_argument("cases", nargs="*")
    em.add_argument("--corpus")
    em.add_argument("--max-detail", type=int, default=10)
    wd = sub.add_parser("widths")
    wd.add_argument("--strings", type=int, default=500_000)
    ap.add_argument("--go-binary")
    args = ap.parse_args()
    global GO_BINARY
    GO_BINARY = args.go_binary
    sys.exit(emulator(args) if args.command == "emulator" else widths(args))


if __name__ == "__main__":
    main()
