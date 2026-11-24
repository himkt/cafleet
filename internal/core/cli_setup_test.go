package core

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
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
