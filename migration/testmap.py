"""Gate G5: map every Rust #[test] to its Go counterpart.

Usage:
  testmap.py list [OUT.tsv]   derive the Go file and test name of every Rust test
  testmap.py check [OUT.tsv]  also require each derived test (or its omission's replacement) in Go

The Rust tests are every `#[test]` function in src/, found by reading the sources, so tests that
only one OS compiles are included on every host; `cargo test --bin cafleet -- --list` must report
exactly the ones this host compiles. BRANCH_ONLY tests are excluded. Each test maps by the design's
rules:
  file   the Rust source file owning the test's module path, through the file mapping
         (src/<a>.rs and src/<a>/mod.rs -> <a>.go, src/<a>/<b>.rs -> <a>_<b>.go, ..., src/main.rs -> main.go),
         whose test file is internal/core/<stem>_test.go; a test under #[cfg(target_os = "linux")] or
         #[cfg(not(target_os = "macos"))] goes to <stem>_linux_test.go, one under
         #[cfg(target_os = "macos")] or #[cfg(not(target_os = "linux"))] to <stem>_darwin_test.go
  name   Test<File>_<Fn> for a test directly in `mod tests`, Test<File>_<Nested1>_..._<NestedN>_<Fn>
         otherwise, with <File> the CamelCase file stem (followed by Linux or Darwin for an OS-specific
         test) and every other part the CamelCase of its Rust module or function name
OMISSIONS lists the Rust tests that assert Rust-library internals, one per line in the form
  <rust test path> -> <go test file>:<Go test name>
naming the Go test that asserts the same SPEC property instead; `#` starts a comment.

list fails on a derived-name collision or a source/listing mismatch; check also fails on any Rust
test without its Go counterpart, a duplicate Go test name, or an omission whose replacement is
missing. While internal/core is absent, check prints SKIP for the Go side. OUT.tsv receives one
row per Rust test: rust path, os, Go file, Go test, status.
"""
import collections
import pathlib
import platform
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
SRC = REPO / "src"
GO_CORE = REPO / "internal" / "core"
BRANCH_ONLY = {"coding_agent::detector::capture_golden::export"}
OMISSIONS = """
coding_agent::detector::tests::composer_reliability::measured_pi_working_editors_are_verified_empty_after_submit -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::composer_reliability::measured_hermes_working_placeholders_verify_only_the_live_editor -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::pi_profile::pi_detects_each_captured_gate_form_on_both_multiplexers_and_widths -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::pi_profile::pi_idle_editor_is_a_verified_empty_composer_after_every_captured_boundary -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::pi_profile::pi_captured_working_post_submit_editors_are_verified_empty -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::pi_profile::pi_allow_plan_is_one_enter_gated_on_the_allow_once_focus -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_PiProfile_PiStageGatesHoldOnTheirCapturedPreStageScreens
coding_agent::detector::tests::pi_profile::pi_deny_plan_moves_with_j_then_enters_without_any_escape_byte -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_PiProfile_PiStageGatesHoldOnTheirCapturedPreStageScreens
coding_agent::detector::tests::hermes_profile::hermes_detects_each_captured_form_at_every_width_on_both_multiplexers -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::hermes_profile::hermes_continuation_timeout_and_submitted_screens_are_the_idle_composer -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::claude_ask_user_question::every_form_state_of_one_call_shares_the_single_form_fingerprint -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::claude_ask_user_question::the_pending_notice_and_the_form_share_one_fingerprint -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::bundled_claude_detector_replays_the_native_permission_fixture_from_unmasked_rows -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::bundled_claude_detects_the_wrapped_native_form_and_reports_its_displayed_operation_rows -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::detector::tests::bundled_idle_composers_are_verified_non_interactions_distinct_from_the_form -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_auto::bundled_claude_auto_fixtures_produce_their_primary_observation -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_2_1_312::each_2_1_312_fixture_classifies_with_its_rule_and_evidence -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::every_ask_user_question_form_capture_opens_a_choiceless_question -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::a_native_dialog_drawn_with_the_same_component_opens_the_same_choiceless_question -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::every_review_your_answers_capture_opens_a_choiceless_question -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::a_form_narrower_than_its_w_min_is_unmatched -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::the_quoted_form_and_the_transcript_viewers_yield_no_question -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_claude_ask_user_question::the_answered_form_returns_to_a_verified_idle_composer -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_opencode::bundled_opencode_detects_every_narrow_and_compact_form_fixture -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::bundled_opencode::bundled_opencode_narrow_idle_composer_is_a_verified_empty_composer -> internal/core/coding_agent_detector_test.go:TestCodingAgentDetector_FixtureGolden
coding_agent::catalog::tests::director_screens::hermes_composer_text_reads_a_tui_composer_rule_and_members_keep_the_classic_rules -> internal/core/coding_agent_catalog_test.go:TestCodingAgentCatalog_DirectorScreens_HermesTuiComposerTextReadsAWordWrappedPreviewBelowTheTranscriptAndStatusLine
runtime::system::tests::per_stream_budget_is_64_kib_even_with_short_reads -> internal/core/runtime_system_test.go:TestRuntimeSystem_TimedRunnerChecksDeadlineWhileBothStreamsKeepProducing
runtime::system::tests::a_drained_stream_keeps_at_most_its_cap_while_still_reading_the_budget -> internal/core/runtime_system_test.go:TestRuntimeSystem_BoundedRun_EachStreamKeepsAtMostItsCapWhileTheChildRunsToExit
"""
GO_TEST_RE = re.compile(r"^func (Test\w+)\(t \*testing\.T\)", re.MULTILINE)
OS_SUFFIX = {"linux": "Linux", "darwin": "Darwin"}
HOST_OS = {"Linux": "linux", "Darwin": "darwin"}[platform.system()]


def camel(name):
    return "".join(part[:1].upper() + part[1:] for part in name.removeprefix("r#").split("_") if part)


def parse_omissions(text):
    omissions = {}
    for number, line in enumerate(text.splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        path, arrow, target = line.partition(" -> ")
        go_file, colon, go_name = target.partition(":")
        assert arrow and colon and path and go_file and go_name, f"OMISSIONS line {number}: expected '<rust path> -> <go file>:<Go test>'"
        assert path not in omissions, f"OMISSIONS line {number}: {path} is listed twice"
        omissions[path.strip()] = (go_file.strip(), go_name.strip())
    return omissions


def cfg_os(attribute):
    """The OS a #[cfg(...)] attribute restricts an item to, or None."""
    text = re.sub(r"\s+", "", attribute)
    if not text.startswith("cfg("):
        return None
    if 'not(target_os="macos")' in text:
        return "linux"
    if 'not(target_os="linux")' in text:
        return "darwin"
    if 'target_os="linux"' in text:
        return "linux"
    if 'target_os="macos"' in text:
        return "darwin"
    assert "target_os" not in text, f"unhandled target_os cfg {attribute}"
    return None


class RustScanner:
    """Finds #[test] functions and their in-file module paths and OS restriction, skipping
    comments, string, byte-string, raw-string and character literals."""

    IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")

    def __init__(self, text):
        self.text = text
        self.i = 0

    def skip_literal(self):
        t, i = self.text, self.i
        if t.startswith("//", i):
            end = t.find("\n", i)
            self.i = len(t) if end < 0 else end
            return True
        if t.startswith("/*", i):
            depth, i = 1, i + 2
            while depth:
                if t.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif t.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            self.i = i
            return True
        raw = re.match(r'b?r(#*)"', t[i : i + 300])
        if raw and (i == 0 or not (t[i - 1].isalnum() or t[i - 1] == "_")):
            closing = '"' + raw.group(1)
            self.i = t.index(closing, i + raw.end()) + len(closing)
            return True
        if t[i] == '"' or t.startswith('b"', i) and (i == 0 or not (t[i - 1].isalnum() or t[i - 1] == "_")):
            i = i + (2 if t[i] == "b" else 1)
            while t[i] != '"':
                i += 2 if t[i] == "\\" else 1
            self.i = i + 1
            return True
        if t[i] == "'" or (t.startswith("b'", i) and (i == 0 or not (t[i - 1].isalnum() or t[i - 1] == "_"))):
            start = i + (2 if t[i] == "b" else 1)
            if t[start] == "\\":
                self.i = t.index("'", start + 2) + 1
                return True
            if start + 1 < len(t) and t[start + 1] == "'":
                self.i = start + 2
                return True
            self.i = start
            return True
        return False

    def attribute(self):
        t, depth, i = self.text, 0, self.i + 1
        start = i + 1
        while True:
            self.i = i
            if self.skip_literal():
                i = self.i
                continue
            if t[i] == "[":
                depth += 1
            elif t[i] == "]":
                depth -= 1
                if depth == 0:
                    self.i = i + 1
                    return t[start:i]
            i += 1

    def tests(self):
        found = []
        scopes = []
        pending = []
        expect_mod = None
        t = self.text
        while self.i < len(t):
            if self.skip_literal():
                continue
            c = t[self.i]
            if t.startswith("#![", self.i):
                self.i += 1
                self.attribute()
                continue
            if t.startswith("#[", self.i):
                pending.append(self.attribute())
                continue
            ident = self.IDENT.match(t, self.i)
            if ident:
                word = ident.group(0)
                self.i = ident.end()
                if word in ("mod", "fn"):
                    rest = t[self.i :]
                    name = self.IDENT.match(rest.lstrip())
                    os_name = next((o for o in map(cfg_os, pending) if o), None)
                    enclosing = next((s[1] for s in reversed(scopes) if s and s[1]), None)
                    if word == "mod":
                        expect_mod = (name.group(0), os_name or enclosing)
                    elif "test" in pending:
                        path = [s[0] for s in scopes if s] + [name.group(0)]
                        found.append(("::".join(path), os_name or enclosing))
                    pending = []
                continue
            if c == "{":
                scopes.append(expect_mod)
                expect_mod, pending = None, []
            elif c == "}":
                scopes.pop()
                pending = []
            elif c == ";":
                expect_mod, pending = None, []
            self.i += 1
        assert not scopes, "unbalanced braces"
        return found


def module_files():
    """Module path (tuple of segments) -> (Go file stem, source path), for every Rust source file."""
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
        files[tuple(parts)] = (stem, path)
    return files


def source_tests(files):
    """Every #[test] in src/: full test path -> (Go file stem, file module length, OS or None)."""
    tests = {}
    for module, (stem, path) in files.items():
        for inner, os_name in RustScanner(path.read_text()).tests():
            full = "::".join([*module, inner]) if module else inner
            if full in BRANCH_ONLY:
                continue
            assert full not in tests, f"{full} found twice"
            tests[full] = (stem, len(module), os_name)
    assert tests, "no #[test] functions found in src/"
    return tests


def listed_tests():
    out = subprocess.run(["cargo", "test", "--bin", "cafleet", "--quiet", "--", "--list", "--format", "terse"], check=True, cwd=REPO, capture_output=True, text=True).stdout
    tests = set()
    for line in out.splitlines():
        path, kind = line.rsplit(": ", 1)
        assert kind == "test", f"unexpected listing entry {line!r}"
        if path not in BRANCH_ONLY:
            tests.add(path)
    assert tests, "cargo listed no tests"
    return tests


def derive(path, stem, module_length, os_name):
    rest = path.split("::")[module_length:]
    if rest[0] == "tests":
        rest = rest[1:]
    assert rest, f"{path}: no test function after the module path"
    suffix = OS_SUFFIX[os_name] if os_name else ""
    file_stem = f"{stem}_{os_name}" if os_name else stem
    return f"internal/core/{file_stem}_test.go", "Test" + camel(stem) + suffix + "_" + "_".join(camel(s) for s in rest)


def go_tests():
    found = collections.defaultdict(list)
    for path in sorted(GO_CORE.glob("*_test.go")):
        for name in GO_TEST_RE.findall(path.read_text()):
            found[name].append(f"internal/core/{path.name}")
    return found


def main(argv):
    assert len(argv) in (2, 3) and argv[1] in ("list", "check"), __doc__
    omissions = parse_omissions(OMISSIONS)
    files = module_files()
    tests = source_tests(files)
    failures = []
    host = {path for path, (_, _, os_name) in tests.items() if os_name in (None, HOST_OS)}
    listed = listed_tests()
    for path in sorted(listed - set(tests)):
        failures.append(f"{path}: cargo lists it, but no #[test] in src/ has this path")
    for path in sorted(host - listed):
        failures.append(f"{path}: found in src/ for {HOST_OS}, but cargo does not list it")
    rows, by_name = [], collections.defaultdict(list)
    for path, (stem, module_length, os_name) in sorted(tests.items()):
        go_file, go_name = derive(path, stem, module_length, os_name)
        by_name[go_name].append(path)
        rows.append([path, os_name or "any", go_file, go_name, "derived"])
    for name, paths in sorted(by_name.items()):
        if len(paths) > 1:
            failures.append(f"derived Go name {name} collides: {paths}")
    for path in omissions:
        assert path in tests, f"omission {path} names no current Rust test"

    if argv[1] == "check":
        if not GO_CORE.is_dir():
            print("G5: Go comparison SKIP (no internal/core)")
        else:
            found = go_tests()
            for name, places in sorted(found.items()):
                if len(places) > 1:
                    failures.append(f"Go test {name} defined in {places}")
            for row in rows:
                path, _, go_file, go_name, _ = row
                if path in omissions:
                    rep_file, rep_name = omissions[path]
                    row[2], row[3] = rep_file, rep_name
                    row[4] = "omitted" if rep_file in found.get(rep_name, []) else "omitted-replacement-missing"
                else:
                    row[4] = "mapped" if go_file in found.get(go_name, []) else "missing"
                if row[4] not in ("mapped", "omitted"):
                    failures.append(f"{path}: {row[4]} {row[2]} {row[3]}")

    if len(argv) == 3:
        with open(argv[2], "w") as out:
            out.write("rust_test\tos\tgo_file\tgo_test\tstatus\n")
            for row in rows:
                out.write("\t".join(row) + "\n")
    status = collections.Counter(r[4] for r in rows)
    by_os = collections.Counter(r[1] for r in rows)
    print(f"G5: {len(rows)} Rust tests ({', '.join(f'{k} {v}' for k, v in sorted(by_os.items()))}) in {len({r[2] for r in rows})} Go test files; "
          + ", ".join(f"{k} {v}" for k, v in sorted(status.items())))
    for failure in failures[:50]:
        print(f"  {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
