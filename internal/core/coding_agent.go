package core

import (
	"encoding/json/jsontext"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

type spawnProbe interface {
	resolveBinary(name string) opt[string]
	homeDir() (string, error)
	envVar(name string) (opt[string], error)
}

type codingAgent uint8

const (
	codingAgentClaude codingAgent = iota + 1
	codingAgentCodex
	codingAgentOpencode
	codingAgentHermes
	codingAgentPi
)

var piThinkingLevels = []string{"off", "minimal", "low", "medium", "high", "xhigh", "max"}

func codingAgentAll() []codingAgent {
	return []codingAgent{codingAgentClaude, codingAgentCodex, codingAgentOpencode, codingAgentHermes, codingAgentPi}
}

func (a codingAgent) name() string {
	switch a {
	case codingAgentClaude:
		return "claude"
	case codingAgentCodex:
		return "codex"
	case codingAgentOpencode:
		return "opencode"
	case codingAgentHermes:
		return "hermes"
	case codingAgentPi:
		return "pi"
	default:
		panic("invalid codingAgent")
	}
}

// UnmarshalJSONFrom decodes the lowercase backend name a profile file records.
func (a *codingAgent) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	token, err := decoder.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '"' {
		return appErrorf("invalid type: %v, expected a backend name", token.Kind())
	}
	agent, ok := codingAgentCodingAgent(token.String())
	if !ok {
		return appErrorf("unknown variant `%s`, expected one of `claude`, `codex`, `opencode`, `hermes`, `pi`", token.String())
	}
	*a = agent
	return nil
}

func (a codingAgent) configVariable() string {
	switch a {
	case codingAgentClaude:
		return "CLAUDE_CONFIG_DIR"
	case codingAgentCodex:
		return "CODEX_HOME"
	case codingAgentOpencode:
		return "OPENCODE_CONFIG_DIR"
	case codingAgentHermes:
		return "HERMES_HOME"
	case codingAgentPi:
		return "PI_CODING_AGENT_DIR"
	default:
		panic("invalid codingAgent")
	}
}

func (a codingAgent) configDir(env envLookup, home string) (string, error) {
	var fallback string
	switch a {
	case codingAgentClaude:
		fallback = joinPath(home, ".claude")
	case codingAgentCodex:
		fallback = joinPath(home, ".codex")
	case codingAgentOpencode:
		fallback = opencodeSkillsBase(home)
	case codingAgentHermes:
		fallback = joinPath(home, ".hermes")
	case codingAgentPi:
		fallback = joinPath(home, ".pi/agent")
	default:
		panic("invalid codingAgent")
	}
	return resolve(a.configVariable(), env, fallback)
}

// forwardedConfig is the backend's configuration variable and its value, when the variable is
// set.
func (a codingAgent) forwardedConfig(env envLookup) (opt[[2]string], error) {
	found, err := env(a.configVariable())
	if err != nil {
		return none[[2]string](), err
	}
	value, set := found.get()
	if !set {
		return none[[2]string](), nil
	}
	if err := requireAbsolute(a.configVariable(), value); err != nil {
		return none[[2]string](), err
	}
	return some([2]string{a.configVariable(), value}), nil
}

func (a codingAgent) requiredPreset(config, mode string) opt[string] {
	if a != codingAgentOpencode {
		return none[string]()
	}
	return some(joinPath(config, "agents/"+mode+".md"))
}

func (a codingAgent) validateModel(model opt[string]) error {
	value, set := model.get()
	if !set {
		return nil
	}
	if a == codingAgentOpencode {
		provider, id, found := strings.Cut(value, "/")
		if !found || provider == "" || id == "" {
			return usageError("--model for the opencode backend must be '<provider-id>/<model-id>' (got '" + value + "').")
		}
	}
	if a == codingAgentPi {
		if index := strings.LastIndex(value, ":"); index >= 0 && slices.Contains(piThinkingLevels, value[index+1:]) {
			return usageError("--model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '" + value + "').")
		}
	}
	return nil
}

func (a codingAgent) validatePrompt(prompt string) error {
	if a == codingAgentPi && strings.HasPrefix(prompt, "@") {
		return usageError("a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).")
	}
	return nil
}

func (a codingAgent) validateEffort(effort opt[string]) error {
	value, set := effort.get()
	if !set {
		return nil
	}
	var choices []string
	switch a {
	case codingAgentClaude:
		choices = []string{"low", "medium", "high", "xhigh", "max"}
	case codingAgentCodex:
		choices = []string{"minimal", "low", "medium", "high", "xhigh"}
	case codingAgentHermes:
		choices = []string{"none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"}
	case codingAgentPi:
		choices = piThinkingLevels
	case codingAgentOpencode:
		return usageError("opencode does not support reasoning effort.")
	default:
		panic("invalid codingAgent")
	}
	if slices.Contains(choices, value) {
		return nil
	}
	return usageError("--effort for the " + a.name() + " backend must be one of " + strings.Join(choices, ", ") + " (got '" + value + "').")
}

func (a codingAgent) ensureAvailable(probe spawnProbe, mode string) (string, error) {
	home, err := probe.homeDir()
	if err != nil {
		return "", err
	}
	config, err := a.configDir(probe.envVar, home)
	if err != nil {
		return "", err
	}
	executable, found := probe.resolveBinary(a.name()).get()
	if !found {
		return "", appError("binary " + a.name() + " not found on PATH")
	}
	if presetPath, required := a.requiredPreset(config, mode).get(); required {
		if _, err := os.ReadFile(presetPath); err != nil {
			return "", appError("cannot read opencode agent preset at " + lossyUTF8([]byte(presetPath)) + ": " + ioErrorText(err) + "; run 'cafleet setup --coding-agent opencode' first")
		}
	}
	if a == codingAgentPi {
		for _, file := range embeddedExtension(piExtensionPrefix) {
			extension := joinPath(piExtensionsDir(config), file.path)
			if _, err := os.ReadFile(extension); err != nil {
				return "", appError("cannot read the CAFleet pi extension at " + lossyUTF8([]byte(extension)) + ": " + ioErrorText(err) + "; run 'cafleet setup --coding-agent pi' first")
			}
		}
	}
	return executable, nil
}

func (a codingAgent) launchArgv(executable, prompt, displayName, cwd string, model, effort opt[string], policy *launchPolicySelection) ([]string, error) {
	if !filepath.IsAbs(executable) || !utf8.ValidString(executable) {
		return nil, appError("native executable must be an absolute UTF-8 path (got " + lossyUTF8([]byte(executable)) + ")")
	}
	argv, err := a.buildSpawnArgvWithPolicy(prompt, displayName, cwd, model, effort, policy)
	if err != nil {
		return nil, err
	}
	argv[0] = executable
	return argv, nil
}

// sharedServerSignature identifies a long-lived backend server whose tool commands carry the
// environment of the pane that started it, not of the session they serve.
type sharedServerSignature struct {
	// programs are the final path components that name the server's program token.
	programs []string
	// subcommand appears, contiguously and in order, after the program token.
	subcommand []string
}

func (s sharedServerSignature) label(agent codingAgent) string {
	return strings.Join(append([]string{agent.name()}, s.subcommand...), " ")
}

func (s sharedServerSignature) matches(argv []string) bool {
	for index, token := range argv {
		program := token[strings.LastIndex(token, "/")+1:]
		if !slices.Contains(s.programs, program) {
			continue
		}
		rest := argv[index+1:]
		for start := 0; start+len(s.subcommand) <= len(rest); start++ {
			if slices.Equal(rest[start:start+len(s.subcommand)], s.subcommand) {
				return true
			}
		}
	}
	return false
}

var codexSharedServers = []sharedServerSignature{{programs: []string{"codex"}, subcommand: []string{"app-server"}}}

var hermesSharedServers = []sharedServerSignature{{programs: []string{"hermes", "hermes_cli.main"}, subcommand: []string{"gateway", "run"}}}

func (a codingAgent) sharedServerSignatures() []sharedServerSignature {
	switch a {
	case codingAgentCodex:
		return codexSharedServers
	case codingAgentHermes:
		return hermesSharedServers
	case codingAgentClaude, codingAgentOpencode, codingAgentPi:
		return nil
	default:
		panic("invalid codingAgent")
	}
}

func codingAgentCodingAgent(name string) (codingAgent, bool) {
	for _, agent := range codingAgentAll() {
		if agent.name() == name {
			return agent, true
		}
	}
	return 0, false
}

type launchPolicySelection struct {
	requestedMode  opt[string]
	resolvedMode   string
	modeSource     modeSource
	permissionArgv []string
}

func (l *launchPolicySelection) equal(other *launchPolicySelection) bool {
	return l.requestedMode == other.requestedMode && l.resolvedMode == other.resolvedMode &&
		l.modeSource == other.modeSource && slices.Equal(l.permissionArgv, other.permissionArgv)
}

func (a codingAgent) defaultPolicy() string {
	switch a {
	case codingAgentClaude:
		return "auto"
	case codingAgentCodex:
		return "approve-for-me"
	case codingAgentOpencode, codingAgentPi:
		return "cafleet-ask"
	case codingAgentHermes:
		return "smart"
	default:
		panic("invalid codingAgent")
	}
}

func codingAgentPermissionModeHelp() string {
	parts := make([]string, 0, len(codingAgentAll()))
	for _, agent := range codingAgentAll() {
		parts = append(parts, agent.name()+": "+strings.Join(agent.policyChoices(), ", ")+" (default "+agent.defaultPolicy()+")")
	}
	return "Launch-only native permission policy for the resolved backend — " + strings.Join(parts, "; ")
}

func (a codingAgent) policyChoices() []string {
	switch a {
	case codingAgentClaude:
		return []string{"default", "manual", "dontAsk", "acceptEdits", "auto", "bypassPermissions", "plan"}
	case codingAgentCodex:
		return []string{"on-request", "never", "approve-for-me"}
	case codingAgentOpencode, codingAgentPi:
		return []string{"cafleet", "cafleet-ask"}
	case codingAgentHermes:
		return []string{"smart"}
	default:
		panic("invalid codingAgent")
	}
}

func (a codingAgent) permissionArguments(mode string) []string {
	switch a {
	case codingAgentClaude:
		return []string{"--permission-mode", mode}
	case codingAgentCodex:
		if mode == "approve-for-me" {
			return []string{"--approve-for-me"}
		}
		return []string{"--ask-for-approval", mode, "--sandbox", "workspace-write"}
	case codingAgentOpencode:
		return []string{"--agent", mode}
	case codingAgentHermes:
		return []string{}
	case codingAgentPi:
		return []string{"--cafleet-gate", mode}
	default:
		panic("invalid codingAgent")
	}
}

func (a codingAgent) resolvePolicy(requested opt[string]) (launchPolicySelection, error) {
	choices := a.policyChoices()
	mode := a.defaultPolicy()
	source := modeSourceDefault
	if value, set := requested.get(); set {
		if !slices.Contains(choices, value) {
			return launchPolicySelection{}, usageError("--permission-mode for " + a.name() + " must be one of " + strings.Join(choices, ", ") + " (got '" + value + "')")
		}
		mode, source = value, modeSourceExplicit
	}
	return launchPolicySelection{
		requestedMode:  requested,
		resolvedMode:   mode,
		modeSource:     source,
		permissionArgv: a.permissionArguments(mode),
	}, nil
}

func (a codingAgent) buildSpawnArgvWithPolicy(prompt, displayName, cwd string, model, effort opt[string], policy *launchPolicySelection) ([]string, error) {
	if a.validateModel(model) != nil || a.validateEffort(effort) != nil {
		panic("member create validates model and effort before argv construction")
	}
	if a.validatePrompt(prompt) != nil {
		panic("member create validates the spawn prompt before argv construction")
	}
	argv := []string{a.name()}
	switch a {
	case codingAgentOpencode:
		argv = append(argv, "mini", "--standalone")
	case codingAgentCodex:
		argv = append(argv, "--no-daemon")
	case codingAgentHermes:
		if !utf8.ValidString(cwd) {
			return nil, appError("member working directory must be UTF-8 (got " + lossyUTF8([]byte(cwd)) + ")")
		}
		argv = append(argv, "chat", "--cli", "--source", "tool", "--in", cwd)
	case codingAgentClaude, codingAgentPi:
	default:
		panic("invalid codingAgent")
	}
	argv = append(argv, policy.permissionArgv...)
	switch a {
	case codingAgentClaude:
		argv = append(argv, "--prompt-suggestions", "false", "--allowedTools", "Bash(cafleet message *)", "--name", displayName)
	case codingAgentPi:
		argv = append(argv, "--no-approve", "--tui-mode", "fullscreen", "--name", displayName)
	case codingAgentCodex, codingAgentOpencode, codingAgentHermes:
	default:
		panic("invalid codingAgent")
	}
	if value, set := model.get(); set {
		argv = append(argv, "--model", value)
	}
	if value, set := effort.get(); set {
		switch a {
		case codingAgentClaude:
			argv = append(argv, "--effort", value)
		case codingAgentCodex:
			argv = append(argv, "--config=model_reasoning_effort="+value)
		case codingAgentHermes:
			argv = append(argv, "--reasoning", value)
		case codingAgentPi:
			argv = append(argv, "--thinking", value)
		case codingAgentOpencode:
			panic("effort validated before argv construction")
		default:
			panic("invalid codingAgent")
		}
	}
	switch a {
	case codingAgentOpencode:
		argv = append(argv, "--prompt")
	case codingAgentHermes:
		argv = append(argv, "-q")
	case codingAgentPi:
		argv = append(argv, "--")
	case codingAgentClaude, codingAgentCodex:
	default:
		panic("invalid codingAgent")
	}
	return append(argv, prompt), nil
}

// nativeVersion is a canonical SemVer 2.0.0 version, with major, minor and patch within u64 as
// the semver crate requires.
type nativeVersion struct {
	major, minor, patch uint64
	pre, build          string
}

var semverPattern = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)` +
	`(?:-((?:0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*))*))?` +
	`(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$`)

func parseU64(digits string) (uint64, bool) {
	var value uint64
	for _, d := range []byte(digits) {
		n := uint64(d - '0')
		if value > (math.MaxUint64-n)/10 {
			return 0, false
		}
		value = value*10 + n
	}
	return value, true
}

func nativeVersionParse(text string) (nativeVersion, bool) {
	m := semverPattern.FindStringSubmatch(text)
	if m == nil {
		return nativeVersion{}, false
	}
	major, okMajor := parseU64(m[1])
	minor, okMinor := parseU64(m[2])
	patch, okPatch := parseU64(m[3])
	if !okMajor || !okMinor || !okPatch {
		return nativeVersion{}, false
	}
	return nativeVersion{major: major, minor: minor, patch: patch, pre: m[4], build: m[5]}, true
}

// UnmarshalJSONFrom decodes a profile's native_version string.
func (v *nativeVersion) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	token, err := decoder.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '"' {
		return appErrorf("invalid type: %v, expected a version string", token.Kind())
	}
	parsed, ok := nativeVersionParse(token.String())
	if !ok {
		return appError("canonical SemVer version required")
	}
	*v = parsed
	return nil
}

type nativeLaunchIdentity struct {
	nativeVersion opt[string]
	coverage      coverage
	warning       opt[string]
}

// codingAgentNativeLaunchIdentity records the probed native version, or the warning naming why it
// is unavailable, with the backend's detection coverage.
func codingAgentNativeLaunchIdentity(backend codingAgent, version string, probeErr error, catalog *profileCatalog) nativeLaunchIdentity {
	cover := catalog.detectionCoverage(backend)
	if probeErr != nil {
		return nativeLaunchIdentity{
			nativeVersion: none[string](),
			coverage:      cover,
			warning:       some("Warning: native version unavailable for " + backend.name() + ": " + cafleetErrorOf(probeErr).message() + "."),
		}
	}
	return nativeLaunchIdentity{nativeVersion: some(version), coverage: cover, warning: none[string]()}
}

const outputBytes = 4096

// runBounded runs a native command with null stdin and bounded output, returning its UTF-8 stdout
// or the failure detail that callers prefix with the command.
func runBounded(executable string, args []string, environment []string, until time.Time) (string, error) {
	output, err := runBoundedOutput(executable, args, environment, until)
	if err != nil {
		return "", err
	}
	if !output.status.Exited() || output.status.ExitStatus() != 0 {
		status := exitStatusText(output.status)
		if first := lines(lossyUTF8(output.stderr)); len(first) > 0 && strings.TrimFunc(first[0], unicode.IsSpace) != "" {
			return "", appError("exited with " + status + ": " + first[0])
		}
		return "", appError("exited with " + status)
	}
	if !utf8.Valid(output.stdout) {
		return "", appError("printed non-UTF-8 output")
	}
	return string(output.stdout), nil
}

// runBoundedOutput runs a native command like runBounded and returns its exit status and both
// output streams; only a start failure or the deadline is an error.
func runBoundedOutput(executable string, args []string, environment []string, until time.Time) (boundedOutput, error) {
	output, failure := runUntilExit(executable, args, environment, until, outputBytes)
	if failure == nil {
		return output, nil
	}
	switch failure.kind {
	case boundedFailureNotStarted:
		return boundedOutput{}, appError("could not start: " + failure.cause())
	case boundedFailureDeadline:
		if failure.exited {
			return boundedOutput{}, appError("output did not close within its deadline")
		}
		return boundedOutput{}, appError("did not finish within its deadline")
	case boundedFailureIO:
		return boundedOutput{}, appError("output could not be read: " + failure.cause())
	default:
		panic("invalid boundedFailureKind")
	}
}

func (a codingAgent) probeNativeVersion(executable string, until time.Time) (string, error) {
	fail := func(detail error) error {
		return appError(a.name() + " --version " + cafleetErrorOf(detail).message())
	}
	text, err := runBounded(executable, []string{"--version"}, nil, until)
	if err != nil {
		return "", fail(err)
	}
	all := lines(text)
	if len(all) == 0 {
		return "", fail(appError("printed no output"))
	}
	version, err := a.versionFromLine(all[0])
	if err != nil {
		return "", fail(err)
	}
	return version, nil
}

func (a codingAgent) versionFromLine(line string) (string, error) {
	switch a {
	case codingAgentHermes:
		return hermesVersion(line)
	case codingAgentClaude, codingAgentCodex, codingAgentOpencode, codingAgentPi:
		return versionToken(line)
	default:
		panic("invalid codingAgent")
	}
}

var hermesVersionRule = regexp.MustCompile(`^Hermes Agent v[^\t\n\v\f\r ]+ \((?P<version>[^)]+)\)$`)

func hermesVersion(line string) (string, error) {
	m := hermesVersionRule.FindStringSubmatch(line)
	if m == nil {
		return "", appError("printed no Hermes version line")
	}
	version := m[hermesVersionRule.SubexpIndex("version")]
	if _, ok := nativeVersionParse(version); !ok {
		return "", appError("printed a non-SemVer Hermes version")
	}
	return version, nil
}

func versionToken(line string) (string, error) {
	var tokens []string
	for _, token := range strings.FieldsFunc(line, unicode.IsSpace) {
		token = strings.TrimPrefix(token, "v")
		if _, ok := nativeVersionParse(token); ok {
			tokens = append(tokens, token)
		}
	}
	switch len(tokens) {
	case 1:
		return tokens[0], nil
	case 0:
		return "", appError("printed no version token")
	default:
		return "", appError("printed several version tokens")
	}
}
