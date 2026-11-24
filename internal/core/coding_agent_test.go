package core

import (
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

// validateSpawnPolicyArgv is the Rust test-only CodingAgent::validate_spawn_policy_argv: argv
// must be exactly what buildSpawnArgvWithPolicy emits for policy, with any display name, model,
// effort and prompt, or without its prompt tail.
func (a codingAgent) validateSpawnPolicyArgv(argv []string, policy *launchPolicySelection) error {
	invalid := func() error { return appError("capture emitted permission argv mismatch") }
	cwd := "/"
	if a == codingAgentHermes {
		index := slices.Index(argv, "--in")
		if index < 0 || index+1 >= len(argv) {
			return invalid()
		}
		cwd = argv[index+1]
	}
	flagPrompt := 0
	if a == codingAgentOpencode || a == codingAgentHermes || a == codingAgentPi {
		flagPrompt = 1
	}
	baseline, err := a.buildSpawnArgvWithPolicy("", "", cwd, none[string](), none[string](), policy)
	if err != nil {
		return err
	}
	displayName := ""
	if a == codingAgentClaude || a == codingAgentPi {
		index := slices.Index(baseline, "--name")
		if index < 0 {
			panic("Claude and pi name option")
		}
		if index+1 >= len(argv) {
			return invalid()
		}
		displayName = argv[index+1]
	}
	cursor := len(baseline) - 1 - flagPrompt
	model := none[string]()
	if len(argv) > cursor+1 && argv[cursor] == "--model" {
		model = some(argv[cursor+1])
		cursor += 2
	}
	effort := none[string]()
	if len(argv) > cursor+1 {
		switch {
		case a == codingAgentClaude && argv[cursor] == "--effort",
			a == codingAgentPi && argv[cursor] == "--thinking",
			a == codingAgentHermes && argv[cursor] == "--reasoning":
			effort = some(argv[cursor+1])
			cursor += 2
		case a == codingAgentCodex:
			if value, found := strings.CutPrefix(argv[cursor], "--config=model_reasoning_effort="); found {
				effort = some(value)
				cursor++
			}
		}
	}
	if cursor > len(argv) {
		return invalid()
	}
	tail := argv[cursor:]
	prompt := none[string]()
	switch {
	case len(tail) == 0:
	case a == codingAgentOpencode && len(tail) == 2 && tail[0] == "--prompt",
		a == codingAgentHermes && len(tail) == 2 && tail[0] == "-q",
		a == codingAgentPi && len(tail) == 2 && tail[0] == "--":
		prompt = some(tail[1])
	case (a == codingAgentClaude || a == codingAgentCodex) && len(tail) == 1:
		prompt = some(tail[0])
	default:
		return invalid()
	}
	text, _ := prompt.get()
	expected, err := a.buildSpawnArgvWithPolicy(text, displayName, cwd, model, effort, policy)
	if err != nil {
		return err
	}
	if _, set := prompt.get(); !set {
		expected = expected[:len(expected)-1-flagPrompt]
	}
	if len(argv) != len(expected) || !slices.Equal(argv[1:], expected[1:]) {
		return invalid()
	}
	return nil
}

const codingAgentTestCwd = "/fixture/member work dir"

func onlyEnv(variable, value string) envLookup {
	return func(name string) (opt[string], error) {
		if name == variable {
			return some(value), nil
		}
		return none[string](), nil
	}
}

func noEnv(string) (opt[string], error) { return none[string](), nil }

func mustResolvePolicy(t testing.TB, agent codingAgent, requested opt[string]) launchPolicySelection {
	t.Helper()
	selection, err := agent.resolvePolicy(requested)
	if err != nil {
		t.Fatalf("%s resolvePolicy(%v): %v", agent.name(), requested, err)
	}
	return selection
}

func mustSpawnArgv(t testing.TB, agent codingAgent, prompt, displayName string, model, effort opt[string], policy *launchPolicySelection) []string {
	t.Helper()
	argv, err := agent.buildSpawnArgvWithPolicy(prompt, displayName, codingAgentTestCwd, model, effort, policy)
	if err != nil {
		t.Fatalf("%s buildSpawnArgvWithPolicy: %v", agent.name(), err)
	}
	return argv
}

func assertErrorKind(t testing.TB, label string, err error, kind errorKind, message string) {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: succeeded, want %q", label, message)
	}
	failure := cafleetErrorOf(err)
	if failure.kind != kind {
		t.Fatalf("%s: error kind %v, want %v (%s)", label, failure.kind, kind, err)
	}
	if message != "" && failure.message() != message {
		t.Fatalf("%s: error %q, want %q", label, failure.message(), message)
	}
}

func TestCodingAgent_LiteralArgvPreservesPermissionAndOptionalFlagBoundaries(t *testing.T) {
	t.Parallel()
	prompt := "--model 'quoted value'\n{member_id} \"next\""
	cases := []struct {
		agent codingAgent
		base  []string
	}{
		{agent: codingAgentClaude, base: []string{"claude", "--permission-mode", "auto", "--prompt-suggestions", "false", "--allowedTools", "Bash(cafleet message *)", "--name", "display name"}},
		{agent: codingAgentCodex, base: []string{"codex", "--no-daemon", "--approve-for-me"}},
		{agent: codingAgentOpencode, base: []string{"opencode", "mini", "--standalone", "--agent", "cafleet-ask"}},
		{agent: codingAgentHermes, base: []string{"hermes", "chat", "--cli", "--source", "tool", "--in", codingAgentTestCwd}},
		{agent: codingAgentPi, base: []string{"pi", "--cafleet-gate", "cafleet-ask", "--no-approve", "--tui-mode", "fullscreen", "--name", "display name"}},
	}
	for _, tc := range cases {
		for _, configured := range []bool{false, true} {
			expected := slices.Clone(tc.base)
			model := none[string]()
			if configured {
				model = some("provider/model with spaces")
				expected = append(expected, "--model", "provider/model with spaces")
			}
			effort := none[string]()
			if configured && tc.agent != codingAgentOpencode {
				effort = some("high")
				switch tc.agent {
				case codingAgentClaude:
					expected = append(expected, "--effort", "high")
				case codingAgentCodex:
					expected = append(expected, "--config=model_reasoning_effort=high")
				case codingAgentHermes:
					expected = append(expected, "--reasoning", "high")
				case codingAgentPi:
					expected = append(expected, "--thinking", "high")
				case codingAgentOpencode:
					panic("opencode takes no effort")
				default:
					panic("invalid codingAgent")
				}
			}
			switch tc.agent {
			case codingAgentOpencode:
				expected = append(expected, "--prompt")
			case codingAgentHermes:
				expected = append(expected, "-q")
			case codingAgentPi:
				expected = append(expected, "--")
			case codingAgentClaude, codingAgentCodex:
			default:
				panic("invalid codingAgent")
			}
			expected = append(expected, prompt)
			policy := mustResolvePolicy(t, tc.agent, none[string]())
			got := mustSpawnArgv(t, tc.agent, prompt, "display name", model, effort, &policy)
			if !slices.Equal(got, expected) {
				t.Fatalf("%s configured=%v: argv %q, want %q", tc.agent.name(), configured, got, expected)
			}
		}
	}
}

func TestCodingAgent_ModelAndEffortValidationPreservesBackendRules(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		if err := agent.validateModel(none[string]()); err != nil {
			t.Fatalf("%s: absent model: %v", agent.name(), err)
		}
		if err := agent.validateEffort(none[string]()); err != nil {
			t.Fatalf("%s: absent effort: %v", agent.name(), err)
		}
		if agent.validateEffort(some("turbo")) == nil {
			t.Fatalf("%s: effort turbo accepted", agent.name())
		}
	}
	for _, model := range []string{"provider/model", "a/b/c"} {
		if err := codingAgentOpencode.validateModel(some(model)); err != nil {
			t.Fatalf("opencode model %q: %v", model, err)
		}
	}
	for _, model := range []string{"model", "/model", "provider/", ""} {
		if codingAgentOpencode.validateModel(some(model)) == nil {
			t.Fatalf("opencode model %q accepted", model)
		}
	}
	cases := []struct {
		agent   codingAgent
		valid   []string
		invalid string
	}{
		{agent: codingAgentClaude, valid: []string{"low", "medium", "high", "xhigh", "max"}, invalid: "minimal"},
		{agent: codingAgentCodex, valid: []string{"minimal", "low", "medium", "high", "xhigh"}, invalid: "max"},
		{agent: codingAgentHermes, valid: []string{"none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"}, invalid: "HIGH"},
		{agent: codingAgentPi, valid: []string{"off", "minimal", "low", "medium", "high", "xhigh", "max"}, invalid: "none"},
	}
	for _, tc := range cases {
		for _, effort := range tc.valid {
			if err := tc.agent.validateEffort(some(effort)); err != nil {
				t.Fatalf("%s effort %q: %v", tc.agent.name(), effort, err)
			}
		}
		if tc.agent.validateEffort(some(tc.invalid)) == nil {
			t.Fatalf("%s effort %q accepted", tc.agent.name(), tc.invalid)
		}
		if err := tc.agent.validateModel(some("arbitrary model")); err != nil {
			t.Fatalf("%s arbitrary model: %v", tc.agent.name(), err)
		}
	}
	assertErrorKind(t, "hermes effort HIGH", codingAgentHermes.validateEffort(some("HIGH")), errorUsage,
		"--effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got 'HIGH').")
	if codingAgentOpencode.validateEffort(some("high")) == nil {
		t.Fatal("opencode effort accepted")
	}
}

func TestCodingAgent_CodingAgentNamesOnlyTheRegistryTokens(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		if got, ok := codingAgentCodingAgent(agent.name()); !ok || got != agent {
			t.Fatalf("codingAgentCodingAgent(%q) = %v, %v", agent.name(), got, ok)
		}
	}
	for _, name := range []string{"Claude", "unknown", "", " pi"} {
		if got, ok := codingAgentCodingAgent(name); ok {
			t.Fatalf("codingAgentCodingAgent(%q) = %v", name, got)
		}
	}
}

func everyConfigVariable(name string) (opt[string], error) {
	values := map[string]string{
		"CLAUDE_CONFIG_DIR":   "/cfg/claude dir/",
		"CODEX_HOME":          "/cfg/codex",
		"OPENCODE_CONFIG_DIR": "/cfg/opencode",
		"HERMES_HOME":         "/cfg/hermes home",
		"PI_CODING_AGENT_DIR": "/cfg/pi agent",
	}
	if value, ok := values[name]; ok {
		return some(value), nil
	}
	return none[string](), nil
}

func TestCodingAgent_ForwardedConfigReturnsOnlyTheSelectedVariableWithTheCreatorExactValue(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		expected, err := everyConfigVariable(agent.configVariable())
		if err != nil {
			t.Fatal(err)
		}
		value, _ := expected.get()
		got, err := agent.forwardedConfig(everyConfigVariable)
		if err != nil {
			t.Fatalf("%s: %v", agent.name(), err)
		}
		if got != some([2]string{agent.configVariable(), value}) {
			t.Fatalf("%s forwards only its own variable with the creator's exact value: %v", agent.name(), got)
		}
	}
}

func TestCodingAgent_ForwardedConfigIsAbsentWhenTheSelectedVariableIsUnset(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		others := func(name string) (opt[string], error) {
			if name == agent.configVariable() {
				return none[string](), nil
			}
			return everyConfigVariable(name)
		}
		got, err := agent.forwardedConfig(others)
		if err != nil {
			t.Fatalf("%s: %v", agent.name(), err)
		}
		if got != none[[2]string]() {
			t.Fatalf("%s forwards no default and no other backend's variable: %v", agent.name(), got)
		}
	}
}

func TestCodingAgent_ForwardedConfigRejectsAnInvalidSelectedValueWithTheConfigDirError(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		lookup := onlyEnv(agent.configVariable(), "relative/cfg")
		expected := agent.configVariable() + " must be an absolute path (got 'relative/cfg')"
		_, err := agent.forwardedConfig(lookup)
		assertErrorKind(t, agent.name()+" forwardedConfig", err, errorApp, expected)
		_, err = agent.configDir(lookup, codingAgentTestHome)
		assertErrorKind(t, agent.name()+" configDir", err, errorApp, expected)
	}
}

func TestCodingAgent_RequiredPresetIsTheSelectedOpencodeAgentFileAndAbsentForOtherBackends(t *testing.T) {
	t.Parallel()
	const config = "/fixture/preset-base"
	for _, mode := range codingAgentOpencode.policyChoices() {
		if got := codingAgentOpencode.requiredPreset(config, mode); got != some(config+"/agents/"+mode+".md") {
			t.Fatalf("opencode %s preset %v", mode, got)
		}
	}
	for _, agent := range []codingAgent{codingAgentClaude, codingAgentCodex, codingAgentHermes} {
		for _, mode := range agent.policyChoices() {
			if got := agent.requiredPreset(config, mode); got != none[string]() {
				t.Fatalf("%s %s preset %v, want none", agent.name(), mode, got)
			}
		}
	}
}

func assertPolicy(t testing.TB, label string, selection launchPolicySelection, requested opt[string], resolved string, source modeSource, argv []string) {
	t.Helper()
	if selection.requestedMode != requested {
		t.Fatalf("%s: requested mode %v, want %v", label, selection.requestedMode, requested)
	}
	if selection.resolvedMode != resolved {
		t.Fatalf("%s: resolved mode %q, want %q", label, selection.resolvedMode, resolved)
	}
	if selection.modeSource != source {
		t.Fatalf("%s: mode source %v, want %v", label, selection.modeSource, source)
	}
	if !slices.Equal(selection.permissionArgv, argv) {
		t.Fatalf("%s: permission argv %q, want %q", label, selection.permissionArgv, argv)
	}
}

func TestCodingAgent_OpencodeLaunchPolicyRecordsTheAgentForTheOmittedAndExplicitModes(t *testing.T) {
	t.Parallel()
	omitted := mustResolvePolicy(t, codingAgentOpencode, none[string]())
	assertPolicy(t, "omitted", omitted, none[string](), "cafleet-ask", modeSourceDefault, []string{"--agent", "cafleet-ask"})
	const config = "/fixture/preset-base"
	if got := codingAgentOpencode.requiredPreset(config, omitted.resolvedMode); got != some(config+"/agents/cafleet-ask.md") {
		t.Fatalf("omitted preset %v", got)
	}
	explicit := mustResolvePolicy(t, codingAgentOpencode, some("cafleet"))
	assertPolicy(t, "explicit", explicit, some("cafleet"), "cafleet", modeSourceExplicit, []string{"--agent", "cafleet"})
	if got := codingAgentOpencode.policyChoices(); !slices.Equal(got, []string{"cafleet", "cafleet-ask"}) {
		t.Fatalf("opencode choices %q", got)
	}
}

const codingAgentTestHome = "/home/tester"

func defaultConfigDir(agent codingAgent) string {
	switch agent {
	case codingAgentClaude:
		return codingAgentTestHome + "/.claude"
	case codingAgentCodex:
		return codingAgentTestHome + "/.codex"
	case codingAgentOpencode:
		return codingAgentTestHome + "/.config/opencode"
	case codingAgentHermes:
		return codingAgentTestHome + "/.hermes"
	case codingAgentPi:
		return codingAgentTestHome + "/.pi/agent"
	default:
		panic("invalid codingAgent")
	}
}

func TestCodingAgent_ConfigDirResolvesOnlyTheBackendVariableOrItsDefault(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		variable := agent.configVariable()
		cases := []struct {
			name     string
			lookup   envLookup
			expected string
		}{
			{name: "unset variable", lookup: noEnv, expected: defaultConfigDir(agent)},
			{name: "absolute value used verbatim", lookup: onlyEnv(variable, "/custom/cfg"), expected: "/custom/cfg"},
			{name: "every other variable set", lookup: func(name string) (opt[string], error) {
				if name != variable {
					return some("/other/cfg"), nil
				}
				return none[string](), nil
			}, expected: defaultConfigDir(agent)},
		}
		for _, tc := range cases {
			got, err := agent.configDir(tc.lookup, codingAgentTestHome)
			if err != nil {
				t.Fatalf("%s: %s: %v", agent.name(), tc.name, err)
			}
			if got != tc.expected {
				t.Fatalf("%s: %s: %q, want %q", agent.name(), tc.name, got, tc.expected)
			}
		}
	}
}

func TestCodingAgent_LaunchArgvReplacesOnlyTheProgramWithAnAbsoluteExecutable(t *testing.T) {
	t.Parallel()
	backend := codingAgentClaude
	policy := mustResolvePolicy(t, backend, none[string]())
	bare := mustSpawnArgv(t, backend, "assignment", "worker", none[string](), some("high"), &policy)
	if bare[0] != "claude" {
		t.Fatalf("bare argv[0] %q", bare[0])
	}
	const executable = "/fixture/versions/2.1.302/claude"
	launched, err := backend.launchArgv(executable, "assignment", "worker", codingAgentTestCwd, none[string](), some("high"), &policy)
	if err != nil {
		t.Fatal(err)
	}
	if launched[0] != executable {
		t.Fatalf("argv[0] is the probed path: %q", launched[0])
	}
	if !slices.Equal(launched[1:], bare[1:]) {
		t.Fatalf("launched tail %q, want %q", launched[1:], bare[1:])
	}
	if _, err := backend.launchArgv("claude", "assignment", "worker", codingAgentTestCwd, none[string](), none[string](), &policy); err == nil {
		t.Fatal("a launched argv[0] is always an absolute path")
	}
}

func TestCodingAgent_VersionTokenRecordsTheDocumentedFirstLinesWithoutALeadingV(t *testing.T) {
	t.Parallel()
	cases := []struct {
		line    string
		version string
		failure string
	}{
		{line: "2.1.302 (Claude Code)", version: "2.1.302"},
		{line: "codex-cli 0.163.0", version: "0.163.0"},
		{line: "opencode v2.1.4", version: "2.1.4"},
		{line: "v1.2.3-beta.1", version: "1.2.3-beta.1"},
		{line: "codex 2.1 build 2.1.302", version: "2.1.302"},
		{line: "codex 2.1", failure: "printed no version token"},
		{line: "opencode vv2.1.4", failure: "printed no version token"},
		{line: "v2.1.4 2.1.5", failure: "printed several version tokens"},
	}
	for _, tc := range cases {
		assertVersionLine(t, tc.line, tc.version, tc.failure, func(line string) (string, error) { return versionToken(line) })
	}
}

func assertVersionLine(t testing.TB, line, version, failure string, parse func(string) (string, error)) {
	t.Helper()
	got, err := parse(line)
	switch {
	case failure == "" && err != nil:
		t.Fatalf("%q: %v, want %q", line, err, version)
	case failure == "" && got != version:
		t.Fatalf("%q: %q, want %q", line, got, version)
	case failure != "" && (err == nil || err.Error() != failure):
		t.Fatalf("%q: (%q, %v), want error %q", line, got, err, failure)
	}
}

func TestCodingAgent_NativeVersionParseAcceptsOnlyCanonicalSemver(t *testing.T) {
	t.Parallel()
	cases := []struct {
		text string
		ok   bool
	}{
		{text: "0.0.1", ok: true},
		{text: "2.1.302", ok: true},
		{text: "3.0.0-beta.1", ok: true},
		{text: "99.0.0+build.1", ok: true},
		{text: "1.2.3-0.a-b.c+d.e-f", ok: true},
		{text: "18446744073709551615.0.0", ok: true},
		{text: "18446744073709551616.0.0", ok: false},
		{text: "01.2.3", ok: false},
		{text: "1.2", ok: false},
		{text: "v1.2.3", ok: false},
		{text: "1.2.3-01", ok: false},
		{text: "1.2.3-", ok: false},
		{text: "1.2.3+", ok: false},
		{text: " 1.2.3", ok: false},
		{text: "2026.09.24", ok: false},
		{text: "", ok: false},
	}
	for _, tc := range cases {
		if _, ok := nativeVersionParse(tc.text); ok != tc.ok {
			t.Fatalf("nativeVersionParse(%q) ok = %v, want %v", tc.text, ok, tc.ok)
		}
	}
}

func TestCodingAgent_NativeVersionProbe_NativeLaunchIdentityLaunchesEveryParsedVersionWithTheBackendsProfile(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, singleProfiles([]singleProfile{{backend: codingAgentCodex, label: "only"}}))
	for _, version := range []string{"0.0.1", "1.0.0", "1.5.0", "3.0.0-beta.1", "99.0.0+build.1"} {
		identity := codingAgentNativeLaunchIdentity(codingAgentCodex, version, nil, catalog)
		if identity.nativeVersion != some(version) {
			t.Fatalf("%s: native version %v", version, identity.nativeVersion)
		}
		if identity.coverage != coverageTested {
			t.Fatalf("%s: coverage %v", version, identity.coverage)
		}
		if identity.warning != none[string]() {
			t.Fatalf("%s launches without a warning: %v", version, identity.warning)
		}
	}
}

func TestCodingAgent_NativeVersionProbe_NativeLaunchIdentityLaunchesTheLowestParsedVersionWithTheBundledProfile(t *testing.T) {
	t.Parallel()
	catalog := profileCatalogBundled()
	for _, backend := range codingAgentAll() {
		identity := codingAgentNativeLaunchIdentity(backend, "0.0.1", nil, catalog)
		if identity.nativeVersion != some("0.0.1") {
			t.Fatalf("%s: native version %v", backend.name(), identity.nativeVersion)
		}
		if identity.coverage != catalog.detectionCoverage(backend) {
			t.Fatalf("%s: coverage %v", backend.name(), identity.coverage)
		}
		if identity.warning != none[string]() {
			t.Fatalf("%s: warning %v", backend.name(), identity.warning)
		}
	}
	for _, backend := range []codingAgent{codingAgentClaude, codingAgentCodex, codingAgentOpencode} {
		if got := codingAgentNativeLaunchIdentity(backend, "0.0.1", nil, catalog).coverage; got != coverageTested {
			t.Fatalf("%s declares detection: coverage %v", backend.name(), got)
		}
	}
}

func TestCodingAgent_NativeVersionProbe_NativeLaunchIdentityLaunchesAFailedProbeWithTheProfileCoverageAndItsWarning(t *testing.T) {
	t.Parallel()
	catalog := profileCatalogBundled()
	probeFailure := appError("probe exited with status 1")
	for _, backend := range []codingAgent{codingAgentClaude, codingAgentCodex} {
		failed := codingAgentNativeLaunchIdentity(backend, "", probeFailure, catalog)
		if failed.nativeVersion != none[string]() {
			t.Fatalf("%s: native version %v", backend.name(), failed.nativeVersion)
		}
		if failed.coverage != coverageTested {
			t.Fatalf("%s: coverage %v", backend.name(), failed.coverage)
		}
		want := some("Warning: native version unavailable for " + backend.name() + ": probe exited with status 1.")
		if failed.warning != want {
			t.Fatalf("%s: warning %v, want %v", backend.name(), failed.warning, want)
		}
	}
	withoutProfile := mustLoadCatalog(t, inspectionFixtureDefinition())
	failed := codingAgentNativeLaunchIdentity(codingAgentClaude, "", probeFailure, withoutProfile)
	if failed.nativeVersion != none[string]() || failed.coverage != coverageUnknown {
		t.Fatalf("without a profile: %+v", failed)
	}
	if failed.warning != some("Warning: native version unavailable for claude: probe exited with status 1.") {
		t.Fatalf("without a profile: warning %v", failed.warning)
	}
}

func TestCodingAgent_NativeVersionProbe_NativeLaunchIdentityWithoutAnyProfileForTheBackendRecordsUnknownSilently(t *testing.T) {
	t.Parallel()
	catalog := mustLoadCatalog(t, inspectionFixtureDefinition())
	for _, backend := range []codingAgent{codingAgentClaude, codingAgentOpencode} {
		identity := codingAgentNativeLaunchIdentity(backend, "2.1.302", nil, catalog)
		if identity.nativeVersion != some("2.1.302") || identity.coverage != coverageUnknown || identity.warning != none[string]() {
			t.Fatalf("%s: %+v", backend.name(), identity)
		}
	}
}

func expectedPermissionArgv(backend codingAgent, selected string) []string {
	switch backend {
	case codingAgentClaude:
		return []string{"--permission-mode", selected}
	case codingAgentCodex:
		if selected == "approve-for-me" {
			return []string{"--approve-for-me"}
		}
		return []string{"--ask-for-approval", selected, "--sandbox", "workspace-write"}
	case codingAgentOpencode:
		return []string{"--agent", selected}
	case codingAgentHermes:
		return []string{}
	case codingAgentPi:
		return []string{"--cafleet-gate", selected}
	default:
		panic("invalid codingAgent")
	}
}

func TestCodingAgent_RuntimePolicyOmissionSelectsEachBackendsDefaultArgv(t *testing.T) {
	t.Parallel()
	defaults := map[codingAgent]string{
		codingAgentClaude:   "auto",
		codingAgentCodex:    "approve-for-me",
		codingAgentOpencode: "cafleet-ask",
		codingAgentHermes:   "smart",
		codingAgentPi:       "cafleet-ask",
	}
	for _, backend := range codingAgentAll() {
		mode, ok := defaults[backend]
		if !ok {
			panic("every backend has a default mode")
		}
		selection := mustResolvePolicy(t, backend, none[string]())
		assertPolicy(t, backend.name(), selection, none[string](), mode, modeSourceDefault, expectedPermissionArgv(backend, mode))
	}
}

func TestCodingAgent_RuntimePolicyEachRegisteredExplicitChoiceMapsToOneLiteralNativeMode(t *testing.T) {
	t.Parallel()
	const prompt = "literal '$x' `text` assignment"
	for _, backend := range codingAgentAll() {
		choices := backend.policyChoices()
		if len(choices) == 0 {
			t.Fatalf("%s has no policy choices", backend.name())
		}
		unique := map[string]bool{}
		for _, mode := range choices {
			if unique[mode] {
				t.Fatalf("%s: duplicate policy %s", backend.name(), mode)
			}
			unique[mode] = true
			selection := mustResolvePolicy(t, backend, some(mode))
			assertPolicy(t, backend.name()+" "+mode, selection, some(mode), mode, modeSourceExplicit, expectedPermissionArgv(backend, mode))
			var expected []string
			switch backend {
			case codingAgentClaude:
				expected = []string{"claude", "--permission-mode", mode, "--prompt-suggestions", "false", "--allowedTools", "Bash(cafleet message *)", "--name", "fixture name"}
			case codingAgentCodex:
				expected = append([]string{"codex", "--no-daemon"}, expectedPermissionArgv(backend, mode)...)
			case codingAgentOpencode:
				expected = []string{"opencode", "mini", "--standalone", "--agent", mode}
			case codingAgentHermes:
				expected = []string{"hermes", "chat", "--cli", "--source", "tool", "--in", codingAgentTestCwd}
			case codingAgentPi:
				expected = []string{"pi", "--cafleet-gate", mode, "--no-approve", "--tui-mode", "fullscreen", "--name", "fixture name"}
			}
			expected = append(expected, "--model", "provider/model")
			effort := none[string]()
			if backend != codingAgentOpencode {
				effort = some("high")
			}
			switch backend {
			case codingAgentClaude:
				expected = append(expected, "--effort", "high")
			case codingAgentCodex:
				expected = append(expected, "--config=model_reasoning_effort=high")
			case codingAgentOpencode:
				expected = append(expected, "--prompt")
			case codingAgentHermes:
				expected = append(expected, "--reasoning", "high", "-q")
			case codingAgentPi:
				expected = append(expected, "--thinking", "high", "--")
			}
			expected = append(expected, prompt)
			got := mustSpawnArgv(t, backend, prompt, "fixture name", some("provider/model"), effort, &selection)
			if !slices.Equal(got, expected) {
				t.Fatalf("%s %s: argv %q, want %q", backend.name(), mode, got, expected)
			}
		}
	}
}

func TestCodingAgent_RuntimePolicyInvalidExplicitSelectionReportsBackendAndCatalogChoices(t *testing.T) {
	t.Parallel()
	for _, backend := range codingAgentAll() {
		for _, invalid := range []string{"", " ", "unsupported-fixture-mode"} {
			_, err := backend.resolvePolicy(some(invalid))
			assertErrorKind(t, backend.name()+" "+invalid, err, errorUsage, "")
			text := err.Error()
			if !strings.Contains(text, backend.name()) {
				t.Fatalf("%s: %q names no backend", backend.name(), text)
			}
			for _, choice := range backend.policyChoices() {
				if !strings.Contains(text, choice) {
					t.Fatalf("%s: %q lacks choice %s", backend.name(), text, choice)
				}
			}
		}
	}
}

func TestCodingAgent_RuntimePolicyHelpListsEveryCatalogWithItsDefault(t *testing.T) {
	t.Parallel()
	const want = "Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)"
	if got := codingAgentPermissionModeHelp(); got != want {
		t.Fatalf("help %q, want %q", got, want)
	}
}

func TestCodingAgent_RuntimePolicyCodexUnknownChoiceFailsWithTheExactUsageError(t *testing.T) {
	t.Parallel()
	_, err := codingAgentCodex.resolvePolicy(some("auto"))
	assertErrorKind(t, "codex auto", err, errorUsage, "--permission-mode for codex must be one of on-request, never, approve-for-me (got 'auto')")
	if code := cafleetErrorOf(err).exitCode(); code != 2 {
		t.Fatalf("exit code %d, want 2", code)
	}
}

func TestCodingAgent_Hermes_HermesCaptureArgvValidationAcceptsTheBuiltArgvAndRejectsAnAddedFlag(t *testing.T) {
	t.Parallel()
	smart := mustResolvePolicy(t, codingAgentHermes, none[string]())
	built, err := codingAgentHermes.launchArgv("/opt/hermes/bin/hermes", "assignment", "worker", codingAgentTestCwd, some("m"), some("high"), &smart)
	if err != nil {
		t.Fatal(err)
	}
	if err := codingAgentHermes.validateSpawnPolicyArgv(built, &smart); err != nil {
		t.Fatalf("the built argv %q is rejected: %v", built, err)
	}
	added := slices.Insert(slices.Clone(built), 1, "--resume")
	if codingAgentHermes.validateSpawnPolicyArgv(added, &smart) == nil {
		t.Fatalf("an added flag is accepted: %q", added)
	}
}

func TestCodingAgent_Hermes_HermesVersionRuleRecordsOnlyACanonicalParenthesizedReleaseDate(t *testing.T) {
	t.Parallel()
	const noLine = "printed no Hermes version line"
	const nonSemver = "printed a non-SemVer Hermes version"
	cases := []struct {
		line    string
		version string
		failure string
	}{
		{line: "Hermes Agent vunknown (2026.9.24)", version: "2026.9.24"},
		{line: "Hermes Agent v0.22.4 (2026.9.24)", version: "2026.9.24"},
		{line: "Hermes Agent v(1) (2026.10.1)", version: "2026.10.1"},
		{line: "Hermes Agent vunknown (2026.9.24-rc.1)", version: "2026.9.24-rc.1"},
		{line: "", failure: noLine},
		{line: "Hermes Agent v0.22.4", failure: noLine},
		{line: "Hermes Agent vunknown (2026.9.24) ", failure: noLine},
		{line: " Hermes Agent vunknown (2026.9.24)", failure: noLine},
		{line: "Hermes Agent vunknown (2026.9.24) (2026.9.25)", failure: noLine},
		{line: "Hermes Agent vunknown (2026.09.24)", failure: nonSemver},
		{line: "Hermes Agent vunknown (unknown)", failure: nonSemver},
	}
	for _, tc := range cases {
		assertVersionLine(t, tc.line, tc.version, tc.failure, codingAgentHermes.versionFromLine)
	}
}

func TestCodingAgent_Hermes_HermesVersionRuleEndsThePackageTokenOnlyAtAsciiWhitespace(t *testing.T) {
	t.Parallel()
	const noLine = "printed no Hermes version line"
	cases := []struct {
		name    string
		line    string
		version string
		failure string
	}{
		{name: "no-break space inside the package token", line: "Hermes Agent v1.0 beta (2026.9.24)", version: "2026.9.24"},
		{name: "ideographic space inside the package token", line: "Hermes Agent v1.0　beta (2026.9.24)", version: "2026.9.24"},
		{name: "tab inside the package token", line: "Hermes Agent v1.0\tbeta (2026.9.24)", failure: noLine},
		{name: "no-break space before the release date", line: "Hermes Agent vunknown (2026.9.24)", failure: noLine},
	}
	for _, tc := range cases {
		assertVersionLine(t, tc.line, tc.version, tc.failure, codingAgentHermes.versionFromLine)
	}
}

var piModes = []string{"cafleet", "cafleet-ask"}

var piThinking = []string{"off", "minimal", "low", "medium", "high", "xhigh", "max"}

const piPromptError = "a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment)."

func piPolicy(t testing.TB, mode opt[string]) launchPolicySelection {
	t.Helper()
	return mustResolvePolicy(t, codingAgentPi, mode)
}

func piArgv(t testing.TB, prompt string, model, effort, mode opt[string]) []string {
	t.Helper()
	policy := piPolicy(t, mode)
	return mustSpawnArgv(t, codingAgentPi, prompt, "pi worker", model, effort, &policy)
}

func TestCodingAgent_Pi_PiIsTheLastRegistryTokenWithItsConfigVariable(t *testing.T) {
	t.Parallel()
	all := codingAgentAll()
	if all[len(all)-1] != codingAgentPi {
		t.Fatalf("the last registry token is %s", all[len(all)-1].name())
	}
	if codingAgentPi.name() != "pi" {
		t.Fatalf("pi name %q", codingAgentPi.name())
	}
	if got, ok := codingAgentCodingAgent("pi"); !ok || got != codingAgentPi {
		t.Fatalf("codingAgentCodingAgent(pi) = %v, %v", got, ok)
	}
	if got := codingAgentPi.configVariable(); got != "PI_CODING_AGENT_DIR" {
		t.Fatalf("pi config variable %q", got)
	}
}

func TestCodingAgent_Pi_PiArgvForEveryModeCarriesTheGateTrustLayoutAndNameThenThePromptAfterASeparator(t *testing.T) {
	t.Parallel()
	prompt := "assignment {member_id}\n'quoted'"
	modes := []struct {
		requested opt[string]
		mode      string
	}{
		{requested: none[string](), mode: "cafleet-ask"},
		{requested: some("cafleet"), mode: "cafleet"},
		{requested: some("cafleet-ask"), mode: "cafleet-ask"},
	}
	options := []struct{ model, effort opt[string] }{
		{model: none[string](), effort: none[string]()},
		{model: some("openrouter/moonshotai/kimi-k2.6"), effort: none[string]()},
		{model: none[string](), effort: some("xhigh")},
		{model: some("kimi k2"), effort: some("off")},
	}
	for _, mode := range modes {
		head := []string{"pi", "--cafleet-gate", mode.mode, "--no-approve", "--tui-mode", "fullscreen", "--name", "pi worker"}
		for _, option := range options {
			expected := slices.Clone(head)
			if model, set := option.model.get(); set {
				expected = append(expected, "--model", model)
			}
			if effort, set := option.effort.get(); set {
				expected = append(expected, "--thinking", effort)
			}
			expected = append(expected, "--", prompt)
			if got := piArgv(t, prompt, option.model, option.effort, mode.requested); !slices.Equal(got, expected) {
				t.Fatalf("%v %v %v: argv %q, want %q", mode.requested, option.model, option.effort, got, expected)
			}
		}
	}
}

func TestCodingAgent_Pi_PiArgvKeepsAPromptBeginningWithADashPositionalAfterTheSeparator(t *testing.T) {
	t.Parallel()
	for _, prompt := range []string{"--model other", "-n", "--"} {
		built := piArgv(t, prompt, none[string](), none[string](), none[string]())
		if !slices.Equal(built[len(built)-2:], []string{"--", prompt}) {
			t.Fatalf("%q: tail %q", prompt, built[len(built)-2:])
		}
		if slices.Contains(built, "--model") {
			t.Fatalf("%q: the prompt adds no option token: %q", prompt, built)
		}
	}
}

func TestCodingAgent_Pi_PiArgvStartsAFreshSessionWithoutResumeOptions(t *testing.T) {
	t.Parallel()
	built := piArgv(t, "assignment", some("provider/id"), some("high"), some("cafleet-ask"))
	for _, option := range []string{"--continue", "-c", "--resume", "-r", "--session", "--session-id", "--fork"} {
		if slices.Contains(built, option) {
			t.Fatalf("%s: %q", option, built)
		}
	}
}

func TestCodingAgent_Pi_PiLaunchPolicyRecordsTheGatePairForTheOmittedAndExplicitModes(t *testing.T) {
	t.Parallel()
	assertPolicy(t, "omitted", piPolicy(t, none[string]()), none[string](), "cafleet-ask", modeSourceDefault, []string{"--cafleet-gate", "cafleet-ask"})
	assertPolicy(t, "explicit", piPolicy(t, some("cafleet")), some("cafleet"), "cafleet", modeSourceExplicit, []string{"--cafleet-gate", "cafleet"})
	if got := codingAgentPi.policyChoices(); !slices.Equal(got, piModes) {
		t.Fatalf("pi choices %q", got)
	}
}

func TestCodingAgent_Pi_PiUnknownLaunchModeFailsWithTheExactUsageError(t *testing.T) {
	t.Parallel()
	_, err := codingAgentPi.resolvePolicy(some("auto"))
	assertErrorKind(t, "pi auto", err, errorUsage, "--permission-mode for pi must be one of cafleet, cafleet-ask (got 'auto')")
	if code := cafleetErrorOf(err).exitCode(); code != 2 {
		t.Fatalf("exit code %d, want 2", code)
	}
}

func TestCodingAgent_Pi_PiModelIsRefusedOnlyWhenAThinkingLevelFollowsItsLastColon(t *testing.T) {
	t.Parallel()
	type modelCase struct {
		model    string
		accepted bool
	}
	var cases []modelCase
	for _, level := range piThinking {
		cases = append(cases, modelCase{model: "openrouter/moonshotai/kimi-k2.6:" + level})
	}
	cases = append(cases,
		modelCase{model: "kimi:high"},
		modelCase{model: "provider/model:free:max"},
		modelCase{model: "openrouter/meta-llama/llama-3.3-70b-instruct:free", accepted: true},
		modelCase{model: "openrouter/moonshotai/kimi-k2.6", accepted: true},
		modelCase{model: "claude-opus-5-5", accepted: true},
		modelCase{model: "kimi k2", accepted: true},
		modelCase{model: "provider/model:high:free", accepted: true},
		modelCase{model: "model:", accepted: true},
	)
	for _, tc := range cases {
		err := codingAgentPi.validateModel(some(tc.model))
		if tc.accepted {
			if err != nil {
				t.Fatalf("%s: %v", tc.model, err)
			}
			built := piArgv(t, "assignment", some(tc.model), none[string](), none[string]())
			index := slices.Index(built, "--model")
			if index < 0 || built[index+1] != tc.model {
				t.Fatalf("%s: argv %q", tc.model, built)
			}
			continue
		}
		assertErrorKind(t, tc.model, err, errorUsage,
			"--model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '"+tc.model+"').")
		if code := cafleetErrorOf(err).exitCode(); code != 2 {
			t.Fatalf("%s: exit code %d", tc.model, code)
		}
	}
}

func TestCodingAgent_Pi_PiEffortForwardsOnlyAThinkingLevelAsTheThinkingOption(t *testing.T) {
	t.Parallel()
	type effortCase struct {
		value    string
		accepted bool
	}
	var cases []effortCase
	for _, level := range piThinking {
		cases = append(cases, effortCase{value: level, accepted: true})
	}
	cases = append(cases, effortCase{value: "none"}, effortCase{value: "ultra"}, effortCase{value: "HIGH"}, effortCase{value: ""})
	for _, tc := range cases {
		err := codingAgentPi.validateEffort(some(tc.value))
		if tc.accepted {
			if err != nil {
				t.Fatalf("%q: %v", tc.value, err)
			}
			built := piArgv(t, "assignment", none[string](), some(tc.value), none[string]())
			index := slices.Index(built, "--thinking")
			if index < 0 || built[index+1] != tc.value || slices.Contains(built, "--effort") {
				t.Fatalf("%q: argv %q", tc.value, built)
			}
			continue
		}
		assertErrorKind(t, fmt.Sprintf("%q", tc.value), err, errorUsage,
			"--effort for the pi backend must be one of off, minimal, low, medium, high, xhigh, max (got '"+tc.value+"').")
		if code := cafleetErrorOf(err).exitCode(); code != 2 {
			t.Fatalf("%q: exit code %d", tc.value, code)
		}
	}
}

func TestCodingAgent_Pi_OnlyPiRefusesAPromptWhoseFirstCharacterIsAnAtSign(t *testing.T) {
	t.Parallel()
	piRefused := []string{"@notes.md", "@", "@{member_id} assignment"}
	piAccepted := []string{"assignment", " @notes.md", "\n@notes.md", "mail me@example.com", "-x", "{member_id}", ""}
	for _, agent := range codingAgentAll() {
		type promptCase struct {
			prompt   string
			accepted bool
		}
		var cases []promptCase
		for _, prompt := range piRefused {
			cases = append(cases, promptCase{prompt: prompt, accepted: agent != codingAgentPi})
		}
		for _, prompt := range piAccepted {
			cases = append(cases, promptCase{prompt: prompt, accepted: true})
		}
		for _, tc := range cases {
			err := agent.validatePrompt(tc.prompt)
			if tc.accepted {
				if err != nil {
					t.Fatalf("%s %q: %v", agent.name(), tc.prompt, err)
				}
				continue
			}
			assertErrorKind(t, agent.name()+" "+tc.prompt, err, errorUsage, piPromptError)
			if code := cafleetErrorOf(err).exitCode(); code != 2 {
				t.Fatalf("%q: exit code %d", tc.prompt, code)
			}
		}
	}
}

func TestCodingAgent_Pi_PiArgvConstructionAssertsThatThePromptPassedValidation(t *testing.T) {
	t.Parallel()
	policy := piPolicy(t, none[string]())
	defer func() {
		if recover() == nil {
			t.Fatal("argv construction panics on a prompt that fails validation")
		}
	}()
	argv, err := codingAgentPi.buildSpawnArgvWithPolicy("@notes.md", "pi worker", codingAgentTestCwd, none[string](), none[string](), &policy)
	t.Fatalf("argv construction returned %q, %v", argv, err)
}

func TestCodingAgent_Pi_PiConfigDirDefaultsToThePiAgentDirectoryAndHonorsTheOverride(t *testing.T) {
	t.Parallel()
	if got, err := codingAgentPi.configDir(noEnv, codingAgentTestHome); err != nil || got != codingAgentTestHome+"/.pi/agent" {
		t.Fatalf("default config dir %q, %v", got, err)
	}
	override := onlyEnv("PI_CODING_AGENT_DIR", "/srv/pi agent")
	if got, err := codingAgentPi.configDir(override, codingAgentTestHome); err != nil || got != "/srv/pi agent" {
		t.Fatalf("override config dir %q, %v", got, err)
	}
	if got, err := codingAgentPi.forwardedConfig(override); err != nil || got != some([2]string{"PI_CODING_AGENT_DIR", "/srv/pi agent"}) {
		t.Fatalf("forwarded override %v, %v", got, err)
	}
	if got, err := codingAgentPi.forwardedConfig(noEnv); err != nil || got != none[[2]string]() {
		t.Fatalf("forwarded default %v, %v", got, err)
	}
}

func TestCodingAgent_Pi_PiVersionProbeRecordsTheSingleVersionToken(t *testing.T) {
	t.Parallel()
	assertVersionLine(t, "1.5.0", "1.5.0", "", codingAgentPi.versionFromLine)
	assertVersionLine(t, "v1.5.0", "1.5.0", "", codingAgentPi.versionFromLine)
	assertVersionLine(t, "pi", "", "printed no version token", codingAgentPi.versionFromLine)
}

func TestCodingAgent_Pi_PiLaunchArgvReplacesOnlyTheProgramWithTheResolvedExecutable(t *testing.T) {
	t.Parallel()
	policy := piPolicy(t, none[string]())
	launched, err := codingAgentPi.launchArgv("/opt/pi/bin/pi", "assignment", "pi worker", codingAgentTestCwd, none[string](), none[string](), &policy)
	if err != nil {
		t.Fatal(err)
	}
	bare := piArgv(t, "assignment", none[string](), none[string](), none[string]())
	if launched[0] != "/opt/pi/bin/pi" || !slices.Equal(launched[1:], bare[1:]) {
		t.Fatalf("launched %q, bare %q", launched, bare)
	}
}

func TestCodingAgent_Pi_PiCaptureArgvValidationAcceptsTheBuiltArgvWithOrWithoutThePromptTail(t *testing.T) {
	t.Parallel()
	for _, mode := range piModes {
		selection := piPolicy(t, some(mode))
		for _, option := range []struct{ model, effort opt[string] }{
			{model: none[string](), effort: none[string]()},
			{model: some("provider/id"), effort: some("max")},
		} {
			built, err := codingAgentPi.launchArgv("/opt/pi/bin/pi", "-assignment", "pi worker", codingAgentTestCwd, option.model, option.effort, &selection)
			if err != nil {
				t.Fatal(err)
			}
			if err := codingAgentPi.validateSpawnPolicyArgv(built, &selection); err != nil {
				t.Fatalf("%s: built argv %q rejected: %v", mode, built, err)
			}
			if built[len(built)-2] != "--" {
				t.Fatalf("%s: no separator before the prompt: %q", mode, built)
			}
			withoutPrompt := built[:len(built)-2]
			if err := codingAgentPi.validateSpawnPolicyArgv(withoutPrompt, &selection); err != nil {
				t.Fatalf("%s: argv without the prompt %q rejected: %v", mode, withoutPrompt, err)
			}
		}
	}
}

func TestCodingAgent_Pi_PiCaptureArgvValidationRejectsAnAddedFlagAMissingSeparatorOrAnotherMode(t *testing.T) {
	t.Parallel()
	selection := piPolicy(t, none[string]())
	built, err := codingAgentPi.launchArgv("/opt/pi/bin/pi", "assignment", "pi worker", codingAgentTestCwd, none[string](), none[string](), &selection)
	if err != nil {
		t.Fatal(err)
	}
	added := slices.Insert(slices.Clone(built), 1, "--continue")
	unseparated := slices.Delete(slices.Clone(built), len(built)-2, len(built)-1)
	for _, argv := range [][]string{added, unseparated} {
		if codingAgentPi.validateSpawnPolicyArgv(argv, &selection) == nil {
			t.Fatalf("%q accepted", argv)
		}
	}
	other := piPolicy(t, some("cafleet"))
	if codingAgentPi.validateSpawnPolicyArgv(built, &other) == nil {
		t.Fatal("the argv of another mode is accepted")
	}
}

var (
	codexDaemonArgv = []string{
		"/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex",
		"app-server",
		"--list",
	}
	hermesGatewayArgv = []string{
		"/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/python",
		"-m",
		"hermes_cli.main",
		"--profile",
		"default",
		"gateway",
		"run",
		"--quiet",
		"--idle-exit",
	}
)

func matchingSignatureLabels(argv []string) []string {
	var labels []string
	for _, agent := range codingAgentAll() {
		for _, signature := range agent.sharedServerSignatures() {
			if signature.matches(argv) {
				labels = append(labels, signature.label(agent))
			}
		}
	}
	return labels
}

func TestCodingAgent_SharedServerSignatures_AnArgvMatchesASignatureOnlyByItsProgramThenContiguousSubcommand(t *testing.T) {
	t.Parallel()
	codex := []string{"codex app-server"}
	hermes := []string{"hermes gateway run"}
	cases := []struct {
		name   string
		argv   []string
		labels []string
	}{
		{name: "codex daemon", argv: codexDaemonArgv, labels: codex},
		{name: "hermes gateway with options before the subcommand", argv: hermesGatewayArgv, labels: hermes},
		{name: "hermes program name", argv: []string{"/usr/local/bin/hermes", "gateway", "run"}, labels: hermes},
		{name: "codex without the daemon", argv: []string{"codex", "--no-daemon", "--dangerously-bypass-approvals-and-sandbox", "<prompt>"}},
		{name: "codex enabling daemon auto start", argv: []string{"codex", "--enable", "daemon_auto_start", "--dangerously-bypass-approvals-and-sandbox", "<prompt>"}},
		{name: "hermes chat", argv: []string{"hermes", "chat", "--tui"}},
		{name: "hermes gateway status", argv: []string{"hermes", "gateway", "status"}},
		{name: "opencode serve", argv: []string{"opencode", "serve", "--service"}},
		{name: "claude", argv: []string{"claude"}},
		{name: "empty argv", argv: []string{}},
		{name: "an option between subcommand tokens", argv: []string{"hermes", "gateway", "--quiet", "run"}},
		{name: "subcommand tokens out of order", argv: []string{"hermes", "run", "gateway"}},
		{name: "subcommand before the program", argv: []string{"app-server", "codex"}},
		{name: "program with a suffix", argv: []string{"/usr/bin/codex-linux-sandbox", "app-server"}},
		{name: "program with a prefix", argv: []string{"/opt/xcodex", "app-server"}},
		{name: "program in a directory of that name", argv: []string{"/opt/codex/bin/node", "app-server"}},
	}
	for _, tc := range cases {
		if got := matchingSignatureLabels(tc.argv); !slices.Equal(got, tc.labels) {
			t.Fatalf("%s: labels %q, want %q", tc.name, got, tc.labels)
		}
	}
}

func TestCodingAgent_SharedServerSignatures_OnlyCodexAndHermesDeclareSignatures(t *testing.T) {
	t.Parallel()
	for _, agent := range codingAgentAll() {
		expected := 0
		if agent == codingAgentCodex || agent == codingAgentHermes {
			expected = 1
		}
		if got := len(agent.sharedServerSignatures()); got != expected {
			t.Fatalf("%s declares %d signatures, want %d", agent.name(), got, expected)
		}
	}
}

func TestCodingAgent_SharedServerSignatures_LabelsNameTheBackendAndItsSubcommand(t *testing.T) {
	t.Parallel()
	label := func(agent codingAgent) string {
		signatures := agent.sharedServerSignatures()
		if len(signatures) != 1 {
			t.Fatalf("%s declares exactly one signature", agent.name())
		}
		return signatures[0].label(agent)
	}
	if got := label(codingAgentCodex); got != "codex app-server" {
		t.Fatalf("codex label %q", got)
	}
	if got := label(codingAgentHermes); got != "hermes gateway run" {
		t.Fatalf("hermes label %q", got)
	}
}

// fakeProbe resolves binaries, the home directory and variables from fixed tables.
type fakeProbe struct {
	home     string
	binaries map[string]string
	env      map[string]string
}

// fakeProbeInstalled resolves every backend binary to /fixture/bin/<name> and sets no variable.
func fakeProbeInstalled(home string) *fakeProbe {
	probe := &fakeProbe{home: home, binaries: map[string]string{}, env: map[string]string{}}
	for _, agent := range codingAgentAll() {
		probe.binaries[agent.name()] = filepath.Join("/fixture/bin", agent.name())
	}
	return probe
}

func (p *fakeProbe) withBinary(name, path string) *fakeProbe {
	p.binaries[name] = path
	return p
}

func (p *fakeProbe) withEnv(name, value string) *fakeProbe {
	p.env[name] = value
	return p
}

func (p *fakeProbe) resolveBinary(name string) opt[string] {
	if path, ok := p.binaries[name]; ok {
		return some(path)
	}
	return none[string]()
}

func (p *fakeProbe) homeDir() (string, error) {
	return p.home, nil
}

func (p *fakeProbe) envVar(name string) (opt[string], error) {
	if value, ok := p.env[name]; ok {
		return some(value), nil
	}
	return none[string](), nil
}
