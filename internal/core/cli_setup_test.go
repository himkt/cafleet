package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

func setupSettings(home string) settings {
	return settingsForDatabase(filepath.Join(home, "registry.db"))
}

func mustPrepareDatabase(t *testing.T, s settings) {
	t.Helper()
	if err := prepareDatabase(s); err != nil {
		t.Fatal(err)
	}
}

func lookupAllBut(variable, value string) envLookup {
	return func(name string) (opt[string], error) {
		if name != variable {
			return some(value), nil
		}
		return none[string](), nil
	}
}

func assertAbsent(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); err == nil {
		t.Fatalf("%s exists", path)
	}
}

func installedSkillEntries(t *testing.T, base string) []string {
	t.Helper()
	entries, err := os.ReadDir(base)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	return names
}

func runningCafleetRecord(t *testing.T) string {
	t.Helper()
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	running, err := filepath.EvalSymlinks(executable)
	if err != nil {
		t.Fatal(err)
	}
	return running + "\n"
}

func TestCliSetup_ParserUsesEnumChoicesAndSupportsRepeatedMultiValueSelection(t *testing.T) {
	t.Parallel()
	parsed, ok := mustParse(t, "setup", "setup", "--coding-agent", "opencode", "claude", "--coding-agent", "claude").(setupArgs)
	if !ok {
		t.Fatal("a setup command")
	}
	if want := []string{"opencode", "claude", "claude"}; !slices.Equal(parsed.codingAgent, want) {
		t.Fatalf("coding agents = %q, want %q", parsed.codingAgent, want)
	}
}

func TestCliSetup_SetupPreparesTheDatabaseBeforeTheFirstAssetFailure(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	s := setupSettings(home)
	mustPrepareDatabase(t, s)
	err := installSelected(nil, lookupOnly("CODEX_HOME", "relative"), home)
	if err == nil || !strings.Contains(err.Error(), "CODEX_HOME") {
		t.Fatalf("installSelected = %v, want the CODEX_HOME failure", err)
	}
	conn, err := connect(s.databaseURL)
	if err != nil {
		t.Fatal(err)
	}
	defer closeConnection(t, conn)
	if err := classifySelectedSchema(conn, "sqlite:///:memory:").requireCurrent(); err != nil {
		t.Fatal(err)
	}
	assertAbsent(t, filepath.Join(home, ".hermes"))
	assertAbsent(t, filepath.Join(home, ".config/opencode"))
}

func TestCliSetup_SelectedInstallationIgnoresUnrelatedConfigurationAndIsRepeatable(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	for range 2 {
		if err := installSelected([]codingAgent{codingAgentClaude, codingAgentClaude}, lookupAllBut("CLAUDE_CONFIG_DIR", "relative"), home); err != nil {
			t.Fatal(err)
		}
		assertAbsent(t, filepath.Join(home, ".hermes"))
	}
}

func TestCliSetup_RefusedWalDatabasesAndSidecarsRemainByteIdenticalWithAHeldReader(t *testing.T) {
	t.Parallel()
	for _, mutation := range []string{"PRAGMA application_id=42", "UPDATE cafleet_database SET cafleet_version='0.0.0'"} {
		t.Run(mutation, func(t *testing.T) {
			t.Parallel()
			s := setupSettings(t.TempDir())
			mustPrepareDatabase(t, s)
			conn, err := connect(s.databaseURL)
			if err != nil {
				t.Fatal(err)
			}
			defer closeConnection(t, conn)
			mustExecBatch(t, conn, "PRAGMA journal_mode=WAL")
			mustExecBatch(t, conn, mutation)
			mustExecBatch(t, conn, "BEGIN; SELECT * FROM cafleet_database")
			path, err := databasePath(s.databaseURL)
			if err != nil {
				t.Fatal(err)
			}
			before := walSnapshot(t, path)
			if err := prepareDatabase(s); err == nil {
				t.Fatalf("%s: setup refuses the database", mutation)
			}
			for _, file := range before {
				assertFileBytes(t, file.path, file.bytes)
			}
			mustExecBatch(t, conn, "ROLLBACK")
		})
	}
}

func TestCliSetup_SetupRejectsUnknownOptionsPositionalsAndAgentValues(t *testing.T) {
	t.Parallel()
	cases := []struct {
		args  []string
		input string
	}{
		{[]string{"setup", "extra"}, "extra"},
		{[]string{"setup", "--unknown"}, "--unknown"},
		{[]string{"setup", "--coding-agent", "python"}, "python"},
		{[]string{"setup", "--coding-agent", "claude", "extra"}, "extra"},
	}
	for _, tc := range cases {
		assertParseError(t, strings.Join(tc.args, " "), tc.args, tc.input)
	}
}

func TestCliSetup_Hermes_HermesSetupInstallsOnlyTheSkillsIntoTheResolvedHomeWithoutHermesOnPath(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		selected []codingAgent
	}{
		{"plain setup", nil},
		{"explicit hermes setup", []codingAgent{codingAgentHermes}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			home := t.TempDir()
			custom := filepath.Join(home, "hermes home")
			if err := installSelected(tc.selected, lookupOnly("HERMES_HOME", custom), home); err != nil {
				t.Fatalf("%s: %v", tc.name, err)
			}
			assertEmbeddedSkills(t, filepath.Join(custom, "skills"))
			if entries := installedSkillEntries(t, custom); !slices.Equal(entries, []string{"skills"}) {
				t.Fatalf("%s: only the skills directory: %q", tc.name, entries)
			}
		})
	}
}

func assertInstalledPiAssets(t *testing.T, base string) {
	t.Helper()
	for _, file := range embeddedExtension("pi/") {
		assertFileBytes(t, filepath.Join(base, "extensions", file.path), file.data)
	}
	assertFileBytes(t, filepath.Join(base, "extensions/cafleet.executable"), []byte(runningCafleetRecord(t)))
	assertEmbeddedSkills(t, filepath.Join(base, "skills"))
}

func TestCliSetup_Pi_PiSetupInstallsSkillsExtensionAndRecordWithoutPiOnPathAndIsRepeatable(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "pi agent")
	for range 2 {
		if err := installSelected([]codingAgent{codingAgentPi}, lookupOnly("PI_CODING_AGENT_DIR", base), home); err != nil {
			t.Fatal(err)
		}
		assertInstalledPiAssets(t, base)
	}
}

func TestCliSetup_Pi_PlainSetupInstallsPiAssetsUnderTheDefaultBase(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	if err := installSelected(nil, lookupNothing, home); err != nil {
		t.Fatal(err)
	}
	assertInstalledPiAssets(t, filepath.Join(home, ".pi/agent"))
}

func TestCliSetup_Pi_PiSetupRejectsAnInvalidBaseBeforeWritingPiAssets(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	err := installSelected([]codingAgent{codingAgentPi}, lookupOnly("PI_CODING_AGENT_DIR", "relative"), home)
	if err == nil || cafleetErrorOf(err).message() != "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')" {
		t.Fatalf("installSelected = %v", err)
	}
	assertAbsent(t, filepath.Join(home, ".pi/agent"))
}

const (
	doctorMember = int64(7)
	doctorNow    = "2026-11-21T12:05:00.000000+00:00"
)

func doctorNowTime(t *testing.T) time.Time {
	t.Helper()
	now, ok := parseLenient(doctorNow)
	if !ok {
		t.Fatal("the fixture instant parses")
	}
	return now
}

func doctorAgo(t *testing.T, seconds int) string {
	t.Helper()
	return formatUTC(doctorNowTime(t).Add(-time.Duration(seconds) * time.Second))
}

func doctorRun(t *testing.T, status string) runRecord {
	t.Helper()
	fixture := newBrokerRuntimeFixture(t)
	if _, err := fixture.conn.execute("UPDATE member_runs SET status=?1 WHERE run_id=?2", status, fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	return mustRun(t, fixture.conn, fixture.run.runID)
}

func doctorHealth(eligibility inputEligibility, degraded bool, gapOpenSince opt[string]) observationHealth {
	health := observationHealth{
		Visibility:   visibilityNotVisible,
		Eligibility:  eligibility,
		GapOpenSince: gapOpenSince,
	}
	if degraded {
		health.Visibility = visibilityUnknown
		health.GapCount = 1
		health.Degraded = true
		health.Diagnostic = some("sink_unavailable")
	}
	return health
}

func answered(health observationHealth) observationProbe {
	return observationProbeAnswered{health: some(health)}
}

func doctorHint(databaseURL opt[string]) string {
	suffix := ""
	if url, ok := databaseURL.get(); ok {
		suffix = " --database-url " + url
	}
	return fmt.Sprintf("inspect with 'cafleet member show %d --json%s'; if it does not recover, run 'cafleet member delete %d%s' and create a replacement member", doctorMember, suffix, doctorMember, suffix)
}

func TestCliSetup_MemberRuntimeRows_DoctorGapErrorAgeIs30Seconds(t *testing.T) {
	t.Parallel()
	if doctorGapErrorAge != 30*time.Second {
		t.Fatalf("doctorGapErrorAge = %v", doctorGapErrorAge)
	}
}

func TestCliSetup_MemberRuntimeRows_MemberRuntimeCheckAppliesTheFirstMatchingSituation(t *testing.T) {
	t.Parallel()
	gap := inputEligibilityHold{reason: gateReasonGap}
	gapDetail := func(run int64) string {
		return fmt.Sprintf("run=%d observation=hold:gap diagnostic=sink_unavailable", run)
	}
	cases := []struct {
		name, status string
		probe        observationProbe
		want         checkStatus
		detail       func(int64) string
	}{
		{"a starting run is not probed", "starting", observationProbeSkipped{}, checkStatusInfo, func(run int64) string { return fmt.Sprintf("run=%d status=starting: not probed", run) }},
		{"a prepared run is not probed", "prepared", observationProbeSkipped{}, checkStatusInfo, func(run int64) string { return fmt.Sprintf("run=%d status=prepared: not probed", run) }},
		{"a recovering run is not probed", "recovering", observationProbeSkipped{}, checkStatusInfo, func(run int64) string { return fmt.Sprintf("run=%d status=recovering: not probed", run) }},
		{"a failed probe", "running", observationProbeFailed{message: "cannot connect to the runner"}, checkStatusError, func(run int64) string {
			return fmt.Sprintf("run=%d health probe failed: cannot connect to the runner; %s", run, doctorHint(none[string]()))
		}},
		{"an answer before the child started", "running", observationProbeAnswered{health: none[observationHealth]()}, checkStatusInfo, func(run int64) string { return fmt.Sprintf("run=%d observation=pending", run) }},
		{"a gap open for more than 30 s", "running", answered(doctorHealth(gap, true, some(doctorAgo(t, 31)))), checkStatusError, func(run int64) string {
			return fmt.Sprintf("%s gap_open_since=%s; %s", gapDetail(run), doctorAgo(t, 31), doctorHint(none[string]()))
		}},
		{"a gap open for exactly 30 s", "degraded", answered(doctorHealth(gap, true, some(doctorAgo(t, 30)))), checkStatusWarn, gapDetail},
		{"a younger gap", "running", answered(doctorHealth(gap, true, some(doctorAgo(t, 5)))), checkStatusWarn, gapDetail},
		{"a terminal-fenced gap with no gap start", "running", answered(doctorHealth(gap, true, none[string]())), checkStatusWarn, gapDetail},
		{"an empty composer", "running", answered(doctorHealth(inputEligibilityComposerEmpty{}, false, none[string]())), checkStatusOk, func(run int64) string { return fmt.Sprintf("run=%d observation=composer_empty", run) }},
		{"an undegraded hold", "running", answered(doctorHealth(inputEligibilityHold{reason: gateReasonInteraction}, false, none[string]())), checkStatusOk, func(run int64) string {
			return fmt.Sprintf("run=%d observation=hold:interaction", run)
		}},
	}
	for _, tc := range cases {
		run := doctorRun(t, tc.status)
		got := memberRuntimeCheck(doctorMember, run, tc.probe, doctorNowTime(t), none[string]())
		if got.name != fmt.Sprintf("member.%d.runtime", doctorMember) || got.status != tc.want || got.detail != tc.detail(run.runID) {
			t.Fatalf("%s: check = %+v, want %s %q", tc.name, got, tc.want.asStr(), tc.detail(run.runID))
		}
	}
}

func TestCliSetup_MemberRuntimeRows_MemberRuntimeCheckNamesANonDefaultDatabaseInEveryHintCommand(t *testing.T) {
	t.Parallel()
	const url = "sqlite:///tmp/cafleet-doctor/registry.db"
	for _, probe := range []observationProbe{
		observationProbeFailed{message: "cannot connect to the runner"},
		answered(doctorHealth(inputEligibilityHold{reason: gateReasonGap}, true, some(doctorAgo(t, 31)))),
	} {
		got := memberRuntimeCheck(doctorMember, doctorRun(t, "running"), probe, doctorNowTime(t), some(url))
		if got.status != checkStatusError || !strings.HasSuffix(got.detail, "; "+doctorHint(some(url))) {
			t.Fatalf("check = %+v", got)
		}
	}
}

func installEmbeddedExtension(t *testing.T, prefix, root, record string) []string {
	t.Helper()
	var installed []string
	for _, file := range embeddedExtension(prefix) {
		target := filepath.Join(root, file.path)
		mustWrite(t, target, string(file.data))
		installed = append(installed, target)
	}
	mustWrite(t, record, runningCafleetRecord(t))
	return installed
}

// installOpencodeExtension returns the plugin file and its executable record.
func installOpencodeExtension(t *testing.T, base string) (string, string) {
	t.Helper()
	plugins := filepath.Join(base, "plugins")
	record := filepath.Join(plugins, "cafleet.executable")
	plugin := filepath.Join(plugins, "cafleet.ts")
	if !slices.Contains(installEmbeddedExtension(t, opencodeExtensionPrefix, plugins, record), plugin) {
		t.Fatal("the OpenCode extension holds cafleet.ts")
	}
	return plugin, record
}

// installPiExtension returns the extension file and its executable record.
func installPiExtension(t *testing.T, base string) (string, string) {
	t.Helper()
	extensions := filepath.Join(base, "extensions")
	record := filepath.Join(extensions, "cafleet.executable")
	extension := filepath.Join(extensions, "cafleet.ts")
	if !slices.Contains(installEmbeddedExtension(t, "pi/", extensions, record), extension) {
		t.Fatal("the pi extension holds cafleet.ts")
	}
	return extension, record
}

func installSkills(t *testing.T, skillsDir string) {
	t.Helper()
	for _, file := range embeddedSkills() {
		mustWrite(t, filepath.Join(skillsDir, file.path), string(file.data))
	}
}

// doctorProbe is every backend on PATH or none, with invalid (when given) set to a relative path.
func doctorProbe(home string, executable bool, invalid opt[string]) *fakeProbe {
	probe := fakeProbeInstalled(home)
	if !executable {
		probe.binaries = map[string]string{}
	}
	if name, ok := invalid.get(); ok {
		probe.withEnv(name, "relative")
	}
	return probe
}

func findCheck(t *testing.T, checks []check, name string) check {
	t.Helper()
	index := slices.IndexFunc(checks, func(c check) bool { return c.name == name })
	if index < 0 {
		t.Fatalf("every diagnostic has its stable name: %s", name)
	}
	return checks[index]
}

func checksFor(probe *fakeProbe, home string) []check {
	return agentChecks(resolveBackends(probe, home, nil))
}

func checkNames(checks []check) []string {
	names := make([]string, len(checks))
	for index, c := range checks {
		names[index] = c.name
	}
	return names
}

func TestCliSetup_DoctorReportsAModifiedInstalledSkill(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	probe := doctorProbe(home, true, none[string]())
	skills := filepath.Join(home, ".claude/skills")
	row := func() (checkStatus, string) {
		found := findCheck(t, checksFor(probe, home), "claude.skills")
		return found.status, found.detail
	}
	assertRow := func(status checkStatus, detail string) {
		t.Helper()
		if gotStatus, gotDetail := row(); gotStatus != status || gotDetail != detail {
			t.Fatalf("claude.skills = %s %q, want %s %q", gotStatus.asStr(), gotDetail, status.asStr(), detail)
		}
	}
	assertRow(checkStatusWarn, "skills not installed at "+skills+"; run 'cafleet setup --coding-agent claude'")
	installSkills(t, skills)
	assertRow(checkStatusOk, skills)
	modified := filepath.Join(skills, embeddedSkills()[0].path)
	mustWrite(t, modified, "modified")
	assertRow(checkStatusError, modified+": differs from the embedded copy; run 'cafleet setup --coding-agent claude'")
}

func TestCliSetup_IndependentFailuresKeepExecutableDiscoveryAndMarkDependentChecksUnavailable(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	checks := checksFor(doctorProbe(home, false, some("OPENCODE_CONFIG_DIR")), home)
	for _, expected := range []struct {
		name   string
		status checkStatus
	}{
		{"opencode.configuration", checkStatusError},
		{"opencode.executable", checkStatusInfo},
		{"claude.configuration", checkStatusOk},
	} {
		if got := findCheck(t, checks, expected.name).status; got != expected.status {
			t.Fatalf("%s = %s", expected.name, got.asStr())
		}
	}
	for _, name := range []string{"opencode.preset.cafleet", "opencode.preset", "opencode.skills", "claude.skills", "codex.skills"} {
		if found := findCheck(t, checks, name); found.status != checkStatusInfo || !strings.HasPrefix(found.detail, "unavailable:") {
			t.Fatalf("%s = %+v", name, found)
		}
	}
}

func TestCliSetup_OpencodePresetChecksFollowCatalogOrderAndReportEachPresetState(t *testing.T) {
	t.Parallel()
	files := []string{"cafleet.md", "cafleet-ask.md"}
	for _, installed := range [][2]bool{{true, false}, {false, true}} {
		context := fmt.Sprintf("installed=%v", installed)
		home := t.TempDir()
		agents := filepath.Join(home, ".config/opencode/agents")
		if err := os.MkdirAll(agents, 0o755); err != nil {
			t.Fatal(err)
		}
		for index, file := range files {
			if installed[index] {
				mustWrite(t, filepath.Join(agents, file), "preset")
			}
		}
		checks := checksFor(doctorProbe(home, true, none[string]()), home)
		names := checkNames(checks)
		position := slices.Index(names, "opencode.executable")
		if position < 0 || !slices.Equal(names[position+1:position+5], []string{"opencode.preset.cafleet", "opencode.preset", "opencode.extension", "opencode.skills"}) {
			t.Fatalf("%s: names %q", context, names)
		}
		for index, file := range files {
			found := checks[position+1+index]
			path := filepath.Join(agents, file)
			if installed[index] {
				if found.status != checkStatusOk || found.detail != path {
					t.Fatalf("%s: %+v", context, found)
				}
				continue
			}
			cause, ok := strings.CutPrefix(found.detail, path+": ")
			if ok {
				cause, ok = strings.CutSuffix(cause, "; run 'cafleet setup --coding-agent opencode'")
			}
			if found.status != checkStatusError || !ok || cause == "" {
				t.Fatalf("%s: %+v", context, found)
			}
		}
	}
}

type extensionState uint8

const (
	extensionStateInstalled extensionState = iota + 1
	extensionStatePlugin
	extensionStateRecord
)

func assertExtensionCheck(t *testing.T, found check, state extensionState, pluginCause opt[string], agent, plugin, record, context string) {
	t.Helper()
	fix := "; run 'cafleet setup --coding-agent " + agent + "'"
	causeAfter := func(path string) string {
		cause, ok := strings.CutPrefix(found.detail, path+": ")
		if ok {
			cause, ok = strings.CutSuffix(cause, fix)
		}
		if !ok || found.status != checkStatusError {
			t.Fatalf("%s: %+v", context, found)
		}
		return cause
	}
	switch state {
	case extensionStateInstalled:
		if found.status != checkStatusOk || found.detail != plugin {
			t.Fatalf("%s: %+v", context, found)
		}
	case extensionStatePlugin:
		cause := causeAfter(plugin)
		if want, ok := pluginCause.get(); ok && cause != want {
			t.Fatalf("%s: cause %q, want %q", context, cause, want)
		}
		if cause == "" {
			t.Fatalf("%s: an empty cause", context)
		}
	case extensionStateRecord:
		if causeAfter(record) == "" {
			t.Fatalf("%s: an empty cause", context)
		}
	default:
		panic("invalid extensionState")
	}
}

type extensionCase struct {
	context string
	mutate  func(t *testing.T, plugin, record string)
	state   extensionState
	cause   opt[string]
}

func mustRemove(t *testing.T, path string) {
	t.Helper()
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
}

func recordCases() []extensionCase {
	return []extensionCase{
		{"missing record", func(t *testing.T, _, record string) { mustRemove(t, record) }, extensionStateRecord, none[string]()},
		{"record names a missing file", func(t *testing.T, _, record string) { mustWrite(t, record, "/nonexistent/cafleet\n") }, extensionStateRecord, none[string]()},
		{"record names a file that is not executable", func(t *testing.T, _, record string) {
			plain := filepath.Join(filepath.Dir(record), "not-executable")
			mustWrite(t, plain, "plain")
			mustWrite(t, record, plain+"\n")
		}, extensionStateRecord, none[string]()},
		{"empty record", func(t *testing.T, _, record string) { mustWrite(t, record, "") }, extensionStateRecord, none[string]()},
	}
}

func TestCliSetup_OpencodeExtensionCheckReportsEachPluginAndRecordState(t *testing.T) {
	t.Parallel()
	cases := append([]extensionCase{
		{"installed", func(*testing.T, string, string) {}, extensionStateInstalled, none[string]()},
		{"missing plugin", func(t *testing.T, plugin, _ string) { mustRemove(t, plugin) }, extensionStatePlugin, none[string]()},
		{"tampered plugin", func(t *testing.T, plugin, _ string) { mustWrite(t, plugin, "tampered") }, extensionStatePlugin, some("differs from the embedded copy")},
	}, recordCases()...)
	for _, tc := range cases {
		home := t.TempDir()
		plugin, record := installOpencodeExtension(t, filepath.Join(home, ".config/opencode"))
		tc.mutate(t, plugin, record)
		checks := checksFor(doctorProbe(home, true, none[string]()), home)
		assertExtensionCheck(t, findCheck(t, checks, "opencode.extension"), tc.state, tc.cause, "opencode", plugin, record, tc.context)
	}
}

func TestCliSetup_OpencodeExtensionCheckIsUnavailableWithoutTheExecutableOrConfiguration(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	installOpencodeExtension(t, filepath.Join(home, ".config/opencode"))
	if detail := findCheck(t, checksFor(doctorProbe(home, false, none[string]()), home), "opencode.extension").detail; detail != "unavailable: executable is absent" {
		t.Fatalf("detail = %q", detail)
	}
	found := findCheck(t, checksFor(doctorProbe(home, true, some("OPENCODE_CONFIG_DIR")), home), "opencode.extension")
	if found.status != checkStatusInfo || !strings.HasPrefix(found.detail, "unavailable: configuration failed: ") {
		t.Fatalf("found = %+v", found)
	}
}

func TestCliSetup_Hermes_DoctorReportsOnlyTheConfigurationExecutableAndSkillsRowsForHermes(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	for _, executable := range []bool{true, false} {
		var names []string
		for _, name := range checkNames(checksFor(doctorProbe(home, executable, none[string]()), home)) {
			if strings.HasPrefix(name, "hermes.") {
				names = append(names, name)
			}
		}
		if want := []string{"hermes.configuration", "hermes.executable", "hermes.skills"}; !slices.Equal(names, want) {
			t.Fatalf("executable=%v: %q", executable, names)
		}
	}
}

func piBase(home string) string {
	return filepath.Join(home, ".pi/agent")
}

func piChecks(probe *fakeProbe, home string) []check {
	var checks []check
	for _, c := range checksFor(probe, home) {
		if strings.HasPrefix(c.name, "pi.") {
			checks = append(checks, c)
		}
	}
	return checks
}

func TestCliSetup_Pi_PiRowsFollowHermesAndOrderTheExtensionBetweenExecutableAndSkills(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	names := checkNames(checksFor(doctorProbe(home, true, none[string]()), home))
	pi := slices.IndexFunc(names, func(name string) bool { return strings.HasPrefix(name, "pi.") })
	if pi < 0 || !slices.ContainsFunc(names[:pi], func(name string) bool { return strings.HasPrefix(name, "hermes.") }) {
		t.Fatalf("names = %q", names)
	}
	if want := []string{"pi.configuration", "pi.executable", "pi.extension", "pi.skills"}; !slices.Equal(names[pi:], want) {
		t.Fatalf("pi rows = %q", names[pi:])
	}
}

func TestCliSetup_Pi_PiExtensionCheckReportsEachExtensionAndRecordState(t *testing.T) {
	t.Parallel()
	cases := append([]extensionCase{
		{"installed", func(*testing.T, string, string) {}, extensionStateInstalled, none[string]()},
		{"missing extension", func(t *testing.T, extension, _ string) { mustRemove(t, extension) }, extensionStatePlugin, none[string]()},
		{"unreadable extension", func(t *testing.T, extension, _ string) {
			mustRemove(t, extension)
			if err := os.Mkdir(extension, 0o755); err != nil {
				t.Fatal(err)
			}
		}, extensionStatePlugin, none[string]()},
		{"tampered extension", func(t *testing.T, extension, _ string) { mustWrite(t, extension, "tampered") }, extensionStatePlugin, some("differs from the embedded copy")},
	}, recordCases()...)
	for _, tc := range cases {
		home := t.TempDir()
		extension, record := installPiExtension(t, piBase(home))
		tc.mutate(t, extension, record)
		checks := piChecks(doctorProbe(home, true, none[string]()), home)
		assertExtensionCheck(t, findCheck(t, checks, "pi.extension"), tc.state, tc.cause, "pi", extension, record, tc.context)
	}
}

func TestCliSetup_Pi_PiExtensionCheckIsUnavailableWithoutTheExecutableOrConfiguration(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	installPiExtension(t, piBase(home))
	if found := findCheck(t, piChecks(doctorProbe(home, false, none[string]()), home), "pi.extension"); found.status != checkStatusInfo || found.detail != "unavailable: executable is absent" {
		t.Fatalf("found = %+v", found)
	}
	invalid := piChecks(doctorProbe(home, true, some("PI_CODING_AGENT_DIR")), home)
	const message = "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')"
	if found := findCheck(t, invalid, "pi.configuration"); found.status != checkStatusError || found.detail != message {
		t.Fatalf("configuration = %+v", found)
	}
	if found := findCheck(t, invalid, "pi.extension"); found.status != checkStatusInfo || found.detail != "unavailable: configuration failed: "+message {
		t.Fatalf("extension = %+v", found)
	}
}

func TestCliSetup_Pi_PiExtensionCheckReadsARelocatedBase(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "relocated pi")
	extension, record := installPiExtension(t, base)
	probe := doctorProbe(home, true, none[string]()).withEnv("PI_CODING_AGENT_DIR", base)
	assertExtensionCheck(t, findCheck(t, piChecks(probe, home), "pi.extension"), extensionStateInstalled, none[string](), "pi", extension, record, "relocated")
}

const (
	doctorTmuxSocket      = "/tmp/tmux-1000/default"
	doctorNoMultiplexer   = "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session"
	doctorPaneReadFailure = "tmux command failed: server exited unexpectedly"
	doctorChildConfig     = "CAFLEET_TEST_DOCTOR"
	doctorChildMarker     = "cafleet-doctor-child-stdout-begins"
	doctorChildAncestor   = int64(4242)
)

func doctorMainPane() multiplexerContext {
	return multiplexerContext{session: "main", windowID: "@1", paneID: "%1"}
}

func verifiedCaller() callerReading {
	return callerReadingResolved{name: "tmux", server: doctorTmuxSocket, pane: paneReadingVerified{variable: "TMUX_PANE", context: doctorMainPane()}}
}

func codexDaemon() sharedServer {
	return sharedServer{label: "codex app-server", pid: 407596}
}

func unverifiedDetail(t *testing.T, server string, pane string, panePID int64) string {
	t.Helper()
	return unverifiedMessage(
		callerPane{variable: "TMUX_PANE", pane: pane},
		refusalPaneNotAncestor{panePID: panePID},
		callerMux{backend: muxBackendTmux, server: server, mux: muxBackendTmux.forServer(server, fakeRunnerWithoutBinaries(t))},
	)
}

func rowName(t *testing.T, line string) string {
	t.Helper()
	_, rest, ok := strings.Cut(line, " ")
	if ok {
		var name string
		if name, _, ok = strings.Cut(rest, ": "); ok {
			return name
		}
	}
	t.Fatalf("STATUS NAME: DETAIL row: %s", line)
	return ""
}

func namedRow(t *testing.T, rows []string, name string) string {
	t.Helper()
	var matching []string
	for _, line := range rows {
		if rowName(t, line) == name {
			matching = append(matching, line)
		}
	}
	if len(matching) != 1 {
		t.Fatalf("%s: %q", name, rows)
	}
	return matching[0]
}

func decodedString(t *testing.T, value jsontext.Value) string {
	t.Helper()
	var text string
	if err := json.Unmarshal(value, &text); err != nil {
		t.Fatalf("%s: %v", value, err)
	}
	return text
}

func decodedArray(t *testing.T, value jsontext.Value) []jsontext.Value {
	t.Helper()
	var entries []jsontext.Value
	if err := json.Unmarshal(value, &entries); err != nil {
		t.Fatalf("%s: %v", value, err)
	}
	return entries
}

func statusLines(t *testing.T, entries []jsontext.Value) []string {
	t.Helper()
	lines := make([]string, len(entries))
	for index, entry := range entries {
		detail := strings.NewReplacer("\n", `\n`, "\r", `\r`).Replace(decodedString(t, jsonAt(t, entry, "detail")))
		lines[index] = decodedString(t, jsonAt(t, entry, "status")) + " " + decodedString(t, jsonAt(t, entry, "name")) + ": " + detail
	}
	return lines
}

type doctorViews struct {
	code int
	text []string
	json []jsontext.Value
}

func (v doctorViews) row(t *testing.T, prefix string) string {
	t.Helper()
	var rows []string
	for _, line := range v.text {
		if strings.HasPrefix(line, prefix) {
			rows = append(rows, line)
		}
	}
	if len(rows) != 1 {
		t.Fatalf("%s: %q", prefix, v.text)
	}
	return rows[0]
}

func (v doctorViews) assertExit(t *testing.T, code int) {
	t.Helper()
	if v.code != code {
		t.Fatalf("exit %d, want %d: %q", v.code, code, v.text)
	}
}

func assertViewsAgree(t *testing.T, views doctorViews) {
	t.Helper()
	for _, line := range views.text {
		name := rowName(t, line)
		if name != "multiplexer" && name != "database" && (!strings.Contains(name, ".") || strings.HasPrefix(name, "member_runtime")) {
			t.Fatalf("doctor prints only environment check rows: %s", line)
		}
	}
	for _, entry := range views.json {
		if keys := testJSONKeys(t, entry); !slices.Equal(keys, []string{"name", "status", "detail"}) {
			t.Fatalf("keys = %q", keys)
		}
	}
	if lines := statusLines(t, views.json); !slices.Equal(lines, views.text) {
		t.Fatalf("the JSON array holds exactly the text rows in check order: %q, want %q", lines, views.text)
	}
}

type doctorScenario struct {
	home        string
	database    string
	caller      func() callerReading
	executables []string
	environment [][2]string
}

func newDoctorScenario(t *testing.T) *doctorScenario {
	t.Helper()
	home := shortTempdir(t)
	return &doctorScenario{home: home, database: filepath.Join(home, "absent.db"), caller: verifiedCaller}
}

func (s *doctorScenario) path(name string) string {
	return filepath.Join(s.home, name)
}

func (s *doctorScenario) config(agent codingAgent) string {
	return s.path(agent.name())
}

func (s *doctorScenario) connect(t *testing.T) *dbConn {
	t.Helper()
	s.database = filepath.Join(s.home, "broker.db")
	conn, _ := initializedConnAt(t, s.database)
	return conn
}

func (s *doctorScenario) opencodePresets(t *testing.T) {
	t.Helper()
	for _, preset := range []string{"cafleet.md", "cafleet-ask.md"} {
		mustWrite(t, filepath.Join(s.path("opencode"), "agents", preset), "preset")
	}
}

func (s *doctorScenario) probe() *fakeProbe {
	probe := fakeProbeInstalled(s.home)
	probe.binaries = map[string]string{}
	for _, name := range s.executables {
		probe.binaries[name] = filepath.Join(s.path("bin"), name)
	}
	for _, agent := range codingAgentAll() {
		probe.withEnv(agent.configVariable(), s.config(agent))
	}
	for _, entry := range s.environment {
		probe.withEnv(entry[0], entry[1])
	}
	return probe
}

func (s *doctorScenario) report() report {
	return gather(doctorInputs{caller: s.caller, settings: settingsForDatabase(s.database), probe: s.probe(), home: s.home})
}

func (s *doctorScenario) views(t *testing.T) doctorViews {
	t.Helper()
	r := s.report()
	views := doctorViews{text: lines(render(r, doctorModeText)), json: decodedArray(t, jsontext.Value(render(r, doctorModeJSON)))}
	if r.hasErrors() {
		views.code = 1
	}
	assertViewsAgree(t, views)
	return views
}

func (s *doctorScenario) rows(t *testing.T, code int) []string {
	t.Helper()
	views := s.views(t)
	views.assertExit(t, code)
	return views.text
}

func healthyBackends(t *testing.T, s *doctorScenario) {
	t.Helper()
	s.executables = append(s.executables, "claude", "codex", "opencode", "hermes", "pi")
	s.opencodePresets(t)
	installOpencodeExtension(t, s.path("opencode"))
	installPiExtension(t, s.path("pi"))
	for _, agent := range []codingAgent{codingAgentClaude, codingAgentHermes, codingAgentPi} {
		installSkills(t, filepath.Join(s.config(agent), "skills"))
	}
	installSkills(t, filepath.Join(opencodeSkillsBase(s.home), "skills"))
}

func healthyScenario(t *testing.T) *doctorScenario {
	t.Helper()
	s := newDoctorScenario(t)
	s.connect(t)
	healthyBackends(t, s)
	return s
}

type doctorChildPane uint8

const (
	doctorChildPaneOwn doctorChildPane = iota + 1
	doctorChildPaneUnset
	doctorChildPanePID
)

type doctorChildConfiguration struct {
	Database string   `json:"database"`
	Args     []string `json:"args"`
	Ancestor []string `json:"ancestor"`
}

func TestCliSetup_DoctorSummary_DoctorChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	var config doctorChildConfiguration
	if err := json.Unmarshal([]byte(os.Getenv(doctorChildConfig)), &config); err != nil {
		t.Fatal(err)
	}
	parsed, ok := mustParse(t, "doctor child", config.Args...).(doctorArgs)
	if !ok {
		t.Fatal("a doctor command")
	}
	own := int64(os.Getpid())
	table := newFixtureProcessTable(own)
	if config.Ancestor != nil {
		table.process(own, doctorChildAncestor, "cafleet", "doctor").process(doctorChildAncestor, 1, config.Ancestor...)
	} else {
		table.process(own, 1, "cafleet", "doctor")
	}
	fmt.Println(doctorChildMarker)
	err := doctorWith(settingsForDatabase(config.Database), parsed, table)
	if err == nil {
		os.Exit(0)
	}
	failure := cafleetErrorOf(err)
	fmt.Fprintf(os.Stderr, "Error: %s\n", failure.message())
	os.Exit(failure.exitCode())
}

type doctorOutcome struct {
	code           int
	stdout, stderr string
}

// doctorProcess runs doctorWith in a child whose PATH holds only tmux and the given stubs, with
// every backend's configuration under home and HOME set to home. The child's process table
// records the child itself with parent 1, or with ancestor's argv between the child and pid 1.
func doctorProcess(t *testing.T, home, database string, stubs [][2]string, args []string, ancestor []string, pane doctorChildPane, panePID int64) doctorOutcome {
	t.Helper()
	bin := t.TempDir()
	reported := `"$PPID"`
	if pane == doctorChildPanePID {
		reported = strconv.FormatInt(panePID, 10)
	}
	executableStub(t, bin, "tmux", "case \" $* \" in *'#{pane_pid}'*) printf '%%1|%s\\n' "+reported+" ;; *' display-message '*) printf 'main|@1|%%1\\n' ;; esac")
	for _, stub := range stubs {
		executableStub(t, bin, stub[0], stub[1])
	}
	configuration, err := json.Marshal(doctorChildConfiguration{Database: database, Args: args, Ancestor: ancestor})
	if err != nil {
		t.Fatal(err)
	}
	child := childTestCommand(TestCliSetup_DoctorSummary_DoctorChild)
	var env []string
	for _, entry := range child.Env {
		name, _, _ := strings.Cut(entry, "=")
		if !slices.Contains([]string{"PATH", "HOME", "TMUX", "TMUX_PANE", "HERDR_ENV"}, name) {
			env = append(env, entry)
		}
	}
	env = append(env, doctorChildConfig+"="+string(configuration), "PATH="+bin, "HOME="+home, "TMUX=cafleet-test,1,0")
	if pane != doctorChildPaneUnset {
		env = append(env, "TMUX_PANE=%1")
	}
	for _, agent := range codingAgentAll() {
		env = append(env, agent.configVariable()+"="+filepath.Join(home, agent.name()))
	}
	child.Env = env
	var stdout, stderr bytes.Buffer
	child.Stdout = &stdout
	child.Stderr = &stderr
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	expired := time.AfterFunc(20*time.Second, func() { _ = child.Process.Kill() })
	err = child.Wait()
	if !expired.Stop() {
		t.Fatalf("%q: doctor exceeded its deadline: %s", args, stderr.String())
	}
	code := 0
	var exit *exec.ExitError
	switch {
	case errors.As(err, &exit):
		code = exit.ExitCode()
	case err != nil:
		t.Fatalf("%q: %v", args, err)
	}
	_, output, found := strings.Cut(stdout.String(), doctorChildMarker+"\n")
	if !found {
		t.Fatalf("%q: the child reached doctor: %q %s", args, stdout.String(), stderr.String())
	}
	return doctorOutcome{code: code, stdout: output, stderr: stderr.String()}
}

// healthyHome is a home where every backend is ready, with its database and the backend stubs.
func healthyHome(t *testing.T) (string, string, [][2]string) {
	t.Helper()
	home := shortTempdir(t)
	database := filepath.Join(home, "broker.db")
	initializedConnAt(t, database)
	opencode := filepath.Join(home, "opencode")
	for _, preset := range []string{"cafleet.md", "cafleet-ask.md"} {
		mustWrite(t, filepath.Join(opencode, "agents", preset), "preset")
	}
	installOpencodeExtension(t, opencode)
	installPiExtension(t, filepath.Join(home, "pi"))
	for _, agent := range codingAgentAll() {
		installSkills(t, filepath.Join(home, agent.name(), "skills"))
	}
	installSkills(t, filepath.Join(opencodeSkillsBase(home), "skills"))
	var stubs [][2]string
	for _, name := range []string{"claude", "codex", "opencode", "hermes", "pi"} {
		stubs = append(stubs, [2]string{name, "exit 0"})
	}
	return home, database, stubs
}

func checkLines(t *testing.T, stdout string) []string {
	t.Helper()
	return statusLines(t, decodedArray(t, jsontext.Value(strings.TrimRight(stdout, "\n"))))
}

func TestCliSetup_DoctorSummary_DoctorProcessPrintsJsonForAHealthyHome(t *testing.T) {
	t.Parallel()
	home, database, stubs := healthyHome(t)
	outcome := doctorProcess(t, home, database, stubs, []string{"doctor", "--json"}, nil, doctorChildPaneOwn, 0)
	if outcome.code != 0 || outcome.stderr != "" {
		t.Fatalf("exit %d: %s", outcome.code, outcome.stderr)
	}
	checks := checkLines(t, outcome.stdout)
	want := []string{"ok multiplexer: tmux server=cafleet-test", "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1"}
	if len(checks) < 2 || !slices.Equal(checks[:2], want) {
		t.Fatalf("checks = %q", checks)
	}
	for _, line := range checks {
		if !strings.HasPrefix(line, "ok ") {
			t.Fatalf("checks = %q", checks)
		}
	}
}

func TestCliSetup_DoctorSummary_DoctorProcessPrintsTheJsonArrayRowsAsTextInCheckOrder(t *testing.T) {
	t.Parallel()
	home, database, stubs := healthyHome(t)
	run := func(args ...string) string {
		outcome := doctorProcess(t, home, database, stubs, args, nil, doctorChildPaneOwn, 0)
		if outcome.code != 0 {
			t.Fatalf("%q: exit %d: %s", args, outcome.code, outcome.stderr)
		}
		return outcome.stdout
	}
	payload := run("doctor", "--json")
	text := run("doctor")
	for _, entry := range decodedArray(t, jsontext.Value(strings.TrimRight(payload, "\n"))) {
		if keys := testJSONKeys(t, entry); !slices.Equal(keys, []string{"name", "status", "detail"}) {
			t.Fatalf("keys = %q", keys)
		}
	}
	if got, want := lines(text), checkLines(t, payload); !slices.Equal(got, want) {
		t.Fatalf("text %q, want %q", got, want)
	}
}

func TestCliSetup_DoctorSummary_DoctorProcessUnderACodexAppServerWarnsOnTheCallerPaneAndExitsZero(t *testing.T) {
	t.Parallel()
	home, database, stubs := healthyHome(t)
	outcome := doctorProcess(t, home, database, stubs, []string{"doctor", "--json"},
		[]string{"/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex", "app-server", "--list"}, doctorChildPaneOwn, 0)
	if outcome.code != 0 || outcome.stderr != "" {
		t.Fatalf("exit %d: %s", outcome.code, outcome.stderr)
	}
	detail := sharedServerMessage(sharedServer{label: "codex app-server", pid: doctorChildAncestor})
	want := []string{
		fmt.Sprintf("info multiplexer: not checked: a shared codex app-server server (pid %d) supplies the caller environment", doctorChildAncestor),
		"warn multiplexer.caller_pane: " + detail,
	}
	if checks := checkLines(t, outcome.stdout); len(checks) < 2 || !slices.Equal(checks[:2], want) {
		t.Fatalf("checks = %q", checks)
	}
}

func assertCallerPaneWarning(t *testing.T, outcome doctorOutcome, detail string) {
	t.Helper()
	if outcome.code != 0 || outcome.stderr != "" {
		t.Fatalf("exit %d: %s", outcome.code, outcome.stderr)
	}
	want := []string{"ok multiplexer: tmux server=cafleet-test", "warn multiplexer.caller_pane: " + detail}
	if checks := checkLines(t, outcome.stdout); len(checks) < 2 || !slices.Equal(checks[:2], want) {
		t.Fatalf("checks = %q", checks)
	}
}

func TestCliSetup_DoctorSummary_DoctorProcessWithTmuxPaneUnsetWarnsOnTheCallerPaneAndExitsZero(t *testing.T) {
	t.Parallel()
	home, database, stubs := healthyHome(t)
	outcome := doctorProcess(t, home, database, stubs, []string{"doctor", "--json"}, nil, doctorChildPaneUnset, 0)
	assertCallerPaneWarning(t, outcome, unsetMessage(muxBackendTmux))
}

func TestCliSetup_DoctorSummary_DoctorProcessWithANonAncestorPaneWarnsUnverifiedAndExitsZero(t *testing.T) {
	t.Parallel()
	const stranger = int64(405228)
	home, database, stubs := healthyHome(t)
	outcome := doctorProcess(t, home, database, stubs, []string{"doctor", "--json"}, nil, doctorChildPanePID, stranger)
	assertCallerPaneWarning(t, outcome, unverifiedDetail(t, "cafleet-test", "%1", stranger))
}

func TestCliSetup_DoctorSummary_DoctorProcessWithoutHermesOrADatabasePrintsJsonAndFails(t *testing.T) {
	t.Parallel()
	home := shortTempdir(t)
	database := filepath.Join(home, "absent.db")
	outcome := doctorProcess(t, home, database, nil, []string{"doctor", "--json"}, nil, doctorChildPaneOwn, 0)
	if outcome.code != 1 || outcome.stderr != "Error: doctor found errors\n" {
		t.Fatalf("exit %d: %q", outcome.code, outcome.stderr)
	}
	checks := checkLines(t, outcome.stdout)
	if !strings.HasPrefix(namedRow(t, checks, "database"), "error database: ") {
		t.Fatalf("checks = %q", checks)
	}
	if got := namedRow(t, checks, "hermes.executable"); got != "info hermes.executable: hermes is optional and absent from PATH" {
		t.Fatalf("hermes.executable = %q", got)
	}
	assertAbsent(t, database)
}

func TestCliSetup_DoctorSummary_CallerPaneRows_TheCallerReadingDecidesTheMultiplexerAndCallerPaneRowsAndExit(t *testing.T) {
	t.Parallel()
	tmux := "ok multiplexer: tmux server=" + doctorTmuxSocket
	unverified := unverifiedDetail(t, doctorTmuxSocket, "%3", 405228)
	cases := []struct {
		name   string
		caller func() callerReading
		rows   []string
		code   int
	}{
		{"shared server", func() callerReading {
			return callerReadingSharedServer{label: codexDaemon().label, pid: codexDaemon().pid, message: sharedServerMessage(codexDaemon())}
		}, []string{
			"info multiplexer: not checked: a shared codex app-server server (pid 407596) supplies the caller environment",
			"warn multiplexer.caller_pane: " + sharedServerMessage(codexDaemon()),
		}, 0},
		{"unavailable multiplexer", func() callerReading { return callerReadingUnavailable{cause: appError(doctorNoMultiplexer)} }, []string{
			"error multiplexer: " + doctorNoMultiplexer,
			"info multiplexer.caller_pane: unavailable: the multiplexer is unavailable",
		}, 1},
		{"unset env pane", func() callerReading {
			return callerReadingResolved{name: "tmux", server: doctorTmuxSocket, pane: paneReadingUnverified{detail: unsetMessage(muxBackendTmux)}}
		}, []string{tmux, "warn multiplexer.caller_pane: " + unsetMessage(muxBackendTmux)}, 0},
		{"refused env pane", func() callerReading {
			return callerReadingResolved{name: "tmux", server: doctorTmuxSocket, pane: paneReadingUnverified{detail: unverified}}
		}, []string{tmux, "warn multiplexer.caller_pane: " + unverified}, 0},
		{"verified tmux pane", verifiedCaller, []string{tmux, "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1"}, 0},
		{"propagated pane error", func() callerReading {
			return callerReadingResolved{name: "tmux", server: doctorTmuxSocket, paneErr: appError(doctorPaneReadFailure)}
		}, []string{tmux, "error multiplexer.caller_pane: " + doctorPaneReadFailure}, 1},
		{"verified herdr pane", func() callerReading {
			return callerReadingResolved{name: "herdr", server: "default", pane: paneReadingVerified{
				variable: "HERDR_PANE_ID",
				context:  multiplexerContext{session: "w4", windowID: "w4:t1", paneID: "w4:p3N"},
			}}
		}, []string{"ok multiplexer: herdr server=default", "ok multiplexer.caller_pane: HERDR_PANE_ID=w4:p3N session=w4 window=w4:t1"}, 0},
	}
	for _, tc := range cases {
		s := healthyScenario(t)
		s.caller = tc.caller
		views := s.views(t)
		if len(views.text) < 2 || !slices.Equal(views.text[:2], tc.rows) {
			t.Fatalf("%s: rows %q, want %q", tc.name, views.text, tc.rows)
		}
		views.assertExit(t, tc.code)
	}
}

// memberWithRun registers name with a run in status; each run gets its own nonce and path.
func memberWithRun(t *testing.T, conn *dbConn, url string, fleet, director int64, name string, nonce byte, status string) (int64, int64) {
	t.Helper()
	member := register(t, conn, fleet, name, some("%5"))
	spec := brokerRuntimeFixtureSpec(t, conn, url, fleet, director, member)
	spec.launchNonce = strings.Repeat(string(nonce), 64)
	spec.runtimePath = strings.Replace(spec.runtimePath, "/fixture/", "/"+name+"/", 1)
	run, err := createRun(context.Background(), conn, spec)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := conn.execute("UPDATE member_runs SET status=?1 WHERE run_id=?2", status, run.runID); err != nil {
		t.Fatal(err)
	}
	return member, run.runID
}

func TestCliSetup_DoctorSummary_DoctorAddsMemberRuntimeRowsAfterTheDatabaseRowAndFailsOnAnErrorRow(t *testing.T) {
	t.Parallel()
	s := newDoctorScenario(t)
	conn := s.connect(t)
	healthyBackends(t, s)
	url := settingsForDatabase(s.database).databaseURL
	fleet, director := createFleetFixture(t, conn, "doctor")
	prepared, preparedRun := memberWithRun(t, conn, url, fleet, director, "prepared", 'a', "prepared")
	running, runningRun := memberWithRun(t, conn, url, fleet, director, "running", 'b', "running")
	exited, _ := memberWithRun(t, conn, url, fleet, director, "exited", 'c', "exited")
	views := s.views(t)
	views.assertExit(t, 1)
	database := slices.IndexFunc(views.text, func(line string) bool { return rowName(t, line) == "database" })
	if database < 0 || len(views.text) < database+3 {
		t.Fatalf("rows = %q", views.text)
	}
	if want := fmt.Sprintf("info member.%d.runtime: run=%d status=prepared: not probed", prepared, preparedRun); views.text[database+1] != want {
		t.Fatalf("row %q, want %q", views.text[database+1], want)
	}
	failed := views.text[database+2]
	prefix := fmt.Sprintf("error member.%d.runtime: run=%d health probe failed: ", running, runningRun)
	suffix := fmt.Sprintf("; inspect with 'cafleet member show %d --json --database-url %s'; if it does not recover, run 'cafleet member delete %d --database-url %s' and create a replacement member", running, url, running, url)
	if !strings.HasPrefix(failed, prefix) || !strings.HasSuffix(failed, suffix) {
		t.Fatalf("failed row = %q", failed)
	}
	for _, absent := range []int64{exited, director} {
		name := fmt.Sprintf("member.%d.runtime", absent)
		if slices.ContainsFunc(views.text, func(line string) bool { return rowName(t, line) == name }) {
			t.Fatalf("%s has no row: %q", name, views.text)
		}
	}
}

func TestCliSetup_DoctorSummary_DoctorWithOnlyWarningsPrintsReadyBackendsAndExitsZero(t *testing.T) {
	t.Parallel()
	s := healthyScenario(t)
	views := s.views(t)
	views.assertExit(t, 0)
	expected := map[string]string{
		"opencode.extension": "ok opencode.extension: " + filepath.Join(s.path("opencode"), "plugins/cafleet.ts"),
		"hermes.skills":      "ok hermes.skills: " + filepath.Join(s.config(codingAgentHermes), "skills"),
	}
	for name, want := range expected {
		if got := namedRow(t, views.text, name); got != want {
			t.Fatalf("%s = %q, want %q", name, got, want)
		}
	}
	want := "warn codex.skills: skills not installed at " + filepath.Join(s.config(codingAgentCodex), "skills") + "; run 'cafleet setup --coding-agent codex'"
	if got := views.row(t, "warn codex.skills: "); got != want {
		t.Fatalf("codex.skills = %q, want %q", got, want)
	}
	for _, line := range views.text {
		if strings.HasPrefix(line, "warn ") && !strings.HasPrefix(line, "warn codex.") {
			t.Fatalf("rows = %q", views.text)
		}
	}
}

func TestCliSetup_DoctorSummary_DoctorReportsConfigurationAndPresetErrorsOnTheirRows(t *testing.T) {
	t.Parallel()
	s := newDoctorScenario(t)
	s.connect(t)
	s.executables = append(s.executables, "claude", "opencode")
	s.environment = append(s.environment, [2]string{"CLAUDE_CONFIG_DIR", "relative"})
	installOpencodeExtension(t, s.path("opencode"))
	views := s.views(t)
	views.assertExit(t, 1)
	if got := namedRow(t, views.text, "claude.configuration"); got != "error claude.configuration: CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')" {
		t.Fatalf("claude.configuration = %q", got)
	}
	agents := filepath.Join(s.path("opencode"), "agents")
	for _, row := range []struct{ name, preset string }{{"opencode.preset.cafleet", "cafleet.md"}, {"opencode.preset", "cafleet-ask.md"}} {
		want := fmt.Sprintf("error %s: %s: No such file or directory (os error 2); run 'cafleet setup --coding-agent opencode'", row.name, filepath.Join(agents, row.preset))
		if got := namedRow(t, views.text, row.name); got != want {
			t.Fatalf("%s = %q, want %q", row.name, got, want)
		}
	}
}

func TestCliSetup_DoctorSummary_DoctorWithoutADatabaseStillComparesTheInstalledSkills(t *testing.T) {
	t.Parallel()
	s := newDoctorScenario(t)
	s.executables = append(s.executables, "claude")
	skills := filepath.Join(s.config(codingAgentClaude), "skills")
	installSkills(t, skills)
	views := s.views(t)
	views.assertExit(t, 1)
	if !strings.HasPrefix(namedRow(t, views.text, "database"), "error database: ") {
		t.Fatalf("rows = %q", views.text)
	}
	if got := namedRow(t, views.text, "claude.skills"); got != "ok claude.skills: "+skills {
		t.Fatalf("claude.skills = %q", got)
	}
	assertAbsent(t, s.database)
}

func TestCliSetup_DoctorSummary_DoctorReportsATamperedOpencodeExtensionOnItsRow(t *testing.T) {
	t.Parallel()
	s := healthyScenario(t)
	plugin, _ := installOpencodeExtension(t, s.path("opencode"))
	mustWrite(t, plugin, "tampered")
	want := "error opencode.extension: " + plugin + ": differs from the embedded copy; run 'cafleet setup --coding-agent opencode'"
	if got := namedRow(t, s.rows(t, 1), "opencode.extension"); got != want {
		t.Fatalf("opencode.extension = %q", got)
	}
}

func TestCliSetup_DoctorSummary_DoctorReportsATamperedPiExtensionOnItsRow(t *testing.T) {
	t.Parallel()
	s := healthyScenario(t)
	extension, _ := installPiExtension(t, s.path("pi"))
	mustWrite(t, extension, "tampered")
	want := "error pi.extension: " + extension + ": differs from the embedded copy; run 'cafleet setup --coding-agent pi'"
	if got := namedRow(t, s.rows(t, 1), "pi.extension"); got != want {
		t.Fatalf("pi.extension = %q", got)
	}
}

func TestCliSetup_DoctorSummary_DoctorReportsAnUnusablePiRecordAndAMissingInstallationOnTheirRows(t *testing.T) {
	t.Parallel()
	s := newDoctorScenario(t)
	s.connect(t)
	s.executables = append(s.executables, "pi")
	_, record := installPiExtension(t, s.path("pi"))
	mustWrite(t, record, "")
	rows := s.rows(t, 1)
	if got, want := namedRow(t, rows, "pi.extension"), "error pi.extension: "+record+": the record is empty; run 'cafleet setup --coding-agent pi'"; got != want {
		t.Fatalf("pi.extension = %q, want %q", got, want)
	}
	want := "warn pi.skills: skills not installed at " + filepath.Join(s.config(codingAgentPi), "skills") + "; run 'cafleet setup --coding-agent pi'"
	if got := namedRow(t, rows, "pi.skills"); got != want {
		t.Fatalf("pi.skills = %q, want %q", got, want)
	}
}

func TestCliSetup_DoctorSummary_DoctorPrintsEachCheckAsOneEscapedRowAndOneJsonObject(t *testing.T) {
	t.Parallel()
	s := healthyScenario(t)
	s.environment = append(s.environment, [2]string{"CLAUDE_CONFIG_DIR", "rel\nat\rive"})
	views := s.views(t)
	views.assertExit(t, 1)
	cases := []struct{ name, status, detail, row string }{
		{"multiplexer", "ok", "tmux server=" + doctorTmuxSocket, "ok multiplexer: tmux server=" + doctorTmuxSocket},
		{"multiplexer.caller_pane", "ok", "TMUX_PANE=%1 session=main window=@1", "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1"},
		{"claude.configuration", "error", "CLAUDE_CONFIG_DIR must be an absolute path (got 'rel\nat\rive')", `error claude.configuration: CLAUDE_CONFIG_DIR must be an absolute path (got 'rel\nat\rive')`},
		{"hermes.executable", "ok", "hermes is on PATH", "ok hermes.executable: hermes is on PATH"},
	}
	for _, tc := range cases {
		if got := namedRow(t, views.text, tc.name); got != tc.row {
			t.Fatalf("%s = %q, want %q", tc.name, got, tc.row)
		}
		var entries []jsontext.Value
		for _, entry := range views.json {
			if decodedString(t, jsonAt(t, entry, "name")) == tc.name {
				entries = append(entries, entry)
			}
		}
		want := mustJSON(t, jsonObject{{"name", tc.name}, {"status", tc.status}, {"detail", tc.detail}})
		if len(entries) != 1 {
			t.Fatalf("%s: %d entries", tc.name, len(entries))
		}
		assertJSONEqual(t, entries[0], want, tc.name)
	}
}

func TestCliSetup_DoctorSummary_DoctorReportsAHealthyPiBackendWithItsExtensionPath(t *testing.T) {
	t.Parallel()
	s := healthyScenario(t)
	views := s.views(t)
	views.assertExit(t, 0)
	if got, want := namedRow(t, views.text, "pi.extension"), "ok pi.extension: "+filepath.Join(s.path("pi"), "extensions/cafleet.ts"); got != want {
		t.Fatalf("pi.extension = %q, want %q", got, want)
	}
}
