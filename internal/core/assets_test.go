package core

import (
	"bytes"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"testing/fstest"

	"github.com/himkt/cafleet"
)

func embeddedPaths(files []embeddedFile) []string {
	paths := make([]string, len(files))
	for index, file := range files {
		paths[index] = file.path
	}
	return paths
}

func TestAssets_EmbeddedTableListsOneTreeInBytewisePathOrderAndSkipsDotNames(t *testing.T) {
	t.Parallel()
	tree := fstest.MapFS{
		"assets/skills/b/SKILL.md":       {Data: []byte("b")},
		"assets/skills/a/x.md":           {Data: []byte("x")},
		"assets/skills/a/B.md":           {Data: []byte("B")},
		"assets/skills/a-b/c":            {Data: []byte("c")},
		"assets/skills/Z.md":             {Data: []byte("Z")},
		"assets/skills/a/.gitkeep":       {Data: []byte("")},
		"assets/skills/.hidden/SKILL.md": {Data: []byte("hidden")},
		"assets/skills/a/.cache/c.md":    {Data: []byte("cached")},
		"assets/presets/p.md":            {Data: []byte("other tree")},
	}
	files := embeddedTable(tree, "assets/skills")
	want := []string{"Z.md", "a-b/c", "a/B.md", "a/x.md", "b/SKILL.md"}
	if got := embeddedPaths(files); !slices.Equal(got, want) {
		t.Fatalf("paths = %q, want %q", got, want)
	}
	for _, file := range files {
		if source := tree["assets/skills/"+file.path]; string(file.data) != string(source.Data) {
			t.Fatalf("%s holds %q, want %q", file.path, file.data, source.Data)
		}
	}
}

func TestAssets_EmbeddedTablePanicsOnAnEmptyOrMissingTree(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		tree fstest.MapFS
	}{
		{name: "only dot names", tree: fstest.MapFS{"assets/skills/.hidden": {Data: []byte("x")}, "assets/skills/.dir/f": {Data: []byte("y")}}},
		{name: "missing root", tree: fstest.MapFS{"assets/presets/p.md": {Data: []byte("p")}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			defer func() {
				if recover() == nil {
					t.Errorf("%s: embeddedTable panics", tc.name)
				}
			}()
			files := embeddedTable(tc.tree, "assets/skills")
			t.Errorf("%s: embeddedTable returned %q", tc.name, embeddedPaths(files))
		})
	}
}

func TestAssets_EmbeddedTablesCarryTheBundledFilesSortedWithoutDotNames(t *testing.T) {
	t.Parallel()
	tables := []struct {
		root  string
		files []embeddedFile
	}{
		{root: "assets/skills", files: embeddedSkills()},
		{root: "assets/presets", files: embeddedPresets()},
		{root: "assets/extensions", files: embeddedExtensions()},
	}
	for _, table := range tables {
		paths := embeddedPaths(table.files)
		if len(paths) == 0 || !slices.IsSorted(paths) {
			t.Fatalf("%s: paths %q are nonempty and sorted bytewise", table.root, paths)
		}
		for _, file := range table.files {
			for _, segment := range strings.Split(file.path, "/") {
				if strings.HasPrefix(segment, ".") {
					t.Fatalf("%s: %s names a dot entry", table.root, file.path)
				}
			}
			data, err := fs.ReadFile(cafleet.Assets, table.root+"/"+file.path)
			if err != nil || string(data) != string(file.data) {
				t.Fatalf("%s: %s differs from the embedded file: %v", table.root, file.path, err)
			}
		}
	}
}

func TestAssets_PresetReturnsTheEmbeddedPresetAtItsPath(t *testing.T) {
	t.Parallel()
	for _, file := range embeddedPresets() {
		if got := preset(file.path); got != some(string(file.data)) {
			t.Fatalf("preset(%q) differs from the embedded bytes", file.path)
		}
	}
	if got := preset("absent/preset.md"); got != none[string]() {
		t.Fatalf("preset(absent) = %v", got)
	}
}

func TestAssets_EmbeddedExtensionStripsItsPrefixAndPanicsForAnUnknownExtension(t *testing.T) {
	t.Parallel()
	for _, prefix := range []string{opencodeExtensionPrefix, piExtensionPrefix} {
		var want []embeddedFile
		for _, file := range embeddedExtensions() {
			if rest, ok := strings.CutPrefix(file.path, prefix); ok {
				want = append(want, embeddedFile{path: rest, data: file.data})
			}
		}
		got := embeddedExtension(prefix)
		if len(got) == 0 || !slices.EqualFunc(got, want, func(a, b embeddedFile) bool {
			return a.path == b.path && string(a.data) == string(b.data)
		}) {
			t.Fatalf("%s: embeddedExtension = %q, want %q", prefix, embeddedPaths(got), embeddedPaths(want))
		}
	}
	defer func() {
		if recover() == nil {
			t.Fatal("an unknown extension prefix panics")
		}
	}()
	embeddedExtension("absent/")
}

func TestAssets_SkillFilesPlacesEverySkillFileUnderTheSkillsDirectory(t *testing.T) {
	t.Parallel()
	skills := embeddedSkills()
	files := skillFiles("/srv/home/.claude/skills")
	if len(files) != len(skills) {
		t.Fatalf("skillFiles lists %d files, want %d", len(files), len(skills))
	}
	for index, file := range files {
		if file.path != "/srv/home/.claude/skills/"+skills[index].path || string(file.data) != string(skills[index].data) {
			t.Fatalf("skill file %d = %s", index, file.path)
		}
	}
}

func TestAssets_InstalledFileProblemComparesInstalledAndEmbeddedBytes(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	equal := filepath.Join(dir, "equal")
	changed := filepath.Join(dir, "changed")
	for path, data := range map[string]string{equal: "embedded", changed: "edited"} {
		if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	cases := []struct {
		name   string
		target string
		want   opt[string]
	}{
		{name: "equal bytes", target: equal, want: none[string]()},
		{name: "different bytes", target: changed, want: some("differs from the embedded copy")},
		{name: "missing file", target: filepath.Join(dir, "missing"), want: some("No such file or directory (os error 2)")},
	}
	for _, tc := range cases {
		if got := installedFileProblem(tc.target, []byte("embedded")); got != tc.want {
			t.Fatalf("%s: installedFileProblem = %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestAssets_ExecutableRecordProblemNamesWhyARecordIsUnusable(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	executable := filepath.Join(dir, "cafleet")
	plain := filepath.Join(dir, "plain")
	for path, mode := range map[string]os.FileMode{executable: 0o700, plain: 0o600} {
		if err := os.WriteFile(path, []byte("#!/bin/sh\n"), mode); err != nil {
			t.Fatal(err)
		}
	}
	missing := filepath.Join(dir, "absent-cafleet")
	record := func(name, content string) string {
		path := filepath.Join(dir, name+".executable")
		if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	cases := []struct {
		name   string
		record string
		want   opt[string]
	}{
		{name: "executable target", record: record("good", executable+"\n"), want: none[string]()},
		{name: "surrounding whitespace", record: record("padded", "  "+executable+"\n\n"), want: none[string]()},
		{name: "empty record", record: record("empty", " \n"), want: some("the record is empty")},
		{name: "plain file", record: record("plain", plain+"\n"), want: some(plain + " is not an executable file")},
		{name: "directory", record: record("directory", dir+"\n"), want: some(dir + " is not an executable file")},
		{name: "missing target", record: record("missing-target", missing+"\n"), want: some(missing + ": No such file or directory (os error 2)")},
		{name: "missing record", record: filepath.Join(dir, "absent.executable"), want: some("No such file or directory (os error 2)")},
	}
	for _, tc := range cases {
		if got := executableRecordProblem(tc.record); got != tc.want {
			t.Fatalf("%s: executableRecordProblem = %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestAssets_RemoveExistingTargetRemovesFilesLinksAndTreesButNotLinkReferents(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	referent := filepath.Join(dir, "operator")
	if err := os.WriteFile(referent, []byte("operator bytes"), 0o600); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(dir, "file")
	if err := os.WriteFile(file, []byte("owned"), 0o600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(dir, "link")
	if err := os.Symlink(referent, link); err != nil {
		t.Fatal(err)
	}
	tree := filepath.Join(dir, "tree")
	if err := os.MkdirAll(filepath.Join(tree, "nested"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(tree, "nested", "f"), []byte("owned"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{file, link, tree, filepath.Join(dir, "absent")} {
		if err := removeExistingTarget(target); err != nil {
			t.Fatalf("%s: %v", target, err)
		}
		if _, err := os.Lstat(target); !os.IsNotExist(err) {
			t.Fatalf("%s remains: %v", target, err)
		}
	}
	if data, err := os.ReadFile(referent); err != nil || string(data) != "operator bytes" {
		t.Fatalf("the link referent = %q %v", data, err)
	}
}

type presetTargetFixture struct {
	source, target string
}

func lookupOnly(variable, value string) envLookup {
	return func(name string) (opt[string], error) {
		if name == variable {
			return some(value), nil
		}
		return none[string](), nil
	}
}

func lookupNothing(string) (opt[string], error) {
	return none[string](), nil
}

func expectedPresets(agent codingAgent, base string) []presetTargetFixture {
	switch agent {
	case codingAgentClaude, codingAgentHermes, codingAgentPi:
		return nil
	case codingAgentCodex:
		return []presetTargetFixture{{"codex/cafleet.rules", filepath.Join(base, "rules/cafleet.rules")}}
	case codingAgentOpencode:
		return []presetTargetFixture{
			{"opencode/cafleet.md", filepath.Join(base, "agents/cafleet.md")},
			{"opencode/cafleet-ask.md", filepath.Join(base, "agents/cafleet-ask.md")},
		}
	default:
		panic("invalid codingAgent")
	}
}

func extensionTargets(agent codingAgent, base string) []embeddedFile {
	var prefix, root string
	switch agent {
	case codingAgentClaude, codingAgentCodex, codingAgentHermes:
		return nil
	case codingAgentOpencode:
		prefix, root = opencodeExtensionPrefix, filepath.Join(base, "plugins")
	case codingAgentPi:
		prefix, root = "pi/", filepath.Join(base, "extensions")
	default:
		panic("invalid codingAgent")
	}
	var targets []embeddedFile
	for _, file := range embeddedExtension(prefix) {
		targets = append(targets, embeddedFile{path: filepath.Join(root, file.path), data: file.data})
	}
	return targets
}

func extensionRecord(agent codingAgent, base string) opt[string] {
	switch agent {
	case codingAgentClaude, codingAgentCodex, codingAgentHermes:
		return none[string]()
	case codingAgentOpencode:
		return some(filepath.Join(base, "plugins/cafleet.executable"))
	case codingAgentPi:
		return some(filepath.Join(base, "extensions/cafleet.executable"))
	default:
		panic("invalid codingAgent")
	}
}

func extensionLine(agent codingAgent, target string) string {
	return agent.name() + ": installed extension (v" + Version + ") -> " + target
}

func presetLine(agent codingAgent, target string) string {
	return agent.name() + ": installed preset (v" + Version + ") -> " + target
}

func assertExecutableRecord(t *testing.T, record string) {
	t.Helper()
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	running, err := filepath.EvalSymlinks(executable)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(running)
	if err != nil || !filepath.IsAbs(running) || info.Mode().Perm()&0o111 == 0 {
		t.Fatalf("the running binary %s is an absolute executable: %v", running, err)
	}
	assertFileBytes(t, record, []byte(running+"\n"))
}

func assertFileBytes(t *testing.T, path string, want []byte) {
	t.Helper()
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("%s holds %q, want %q", path, got, want)
	}
}

func assertEmbeddedExtensions(t *testing.T, targets []embeddedFile) {
	t.Helper()
	for _, target := range targets {
		assertFileBytes(t, target.path, target.data)
	}
}

func assertEmbeddedSkills(t *testing.T, skillsDir string) {
	t.Helper()
	for _, file := range embeddedSkills() {
		assertFileBytes(t, filepath.Join(skillsDir, file.path), file.data)
	}
}

func expectedSkillsDir(agent codingAgent, base, home string) string {
	if agent == codingAgentOpencode {
		return filepath.Join(home, ".config/opencode/skills")
	}
	return filepath.Join(base, "skills")
}

func mustAgentPaths(t *testing.T, env envLookup, home string, agent codingAgent) agentPaths {
	t.Helper()
	paths, err := assetsAgentPaths(env, home, agent)
	if err != nil {
		t.Fatal(err)
	}
	return paths
}

func install(agent codingAgent, paths agentPaths) ([]string, error) {
	var output bytes.Buffer
	err := installAgent(agent, paths, &output)
	return lines(output.String()), err
}

func mustInstall(t *testing.T, agent codingAgent, paths agentPaths) []string {
	t.Helper()
	output, err := install(agent, paths)
	if err != nil {
		t.Fatalf("%s: %v", agent.name(), err)
	}
	return output
}

func mustWrite(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatal(err)
	}
}

func assertInstallFailure(t *testing.T, err error, prefix string) {
	t.Helper()
	if err == nil {
		t.Fatal("the blocked target fails installation")
	}
	failure := cafleetErrorOf(err)
	if failure.exitCode() != 1 || !strings.HasPrefix(failure.message(), prefix) {
		t.Fatalf("failure = %q (exit %d), want prefix %q", failure.message(), failure.exitCode(), prefix)
	}
}

func TestAssets_InstallationReplacesOwnedSymlinksAndEmbedsBytesAtSelectedPaths(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	external := filepath.Join(home, "operator")
	mustWrite(t, filepath.Join(external, "keep"), "operator bytes")
	for _, agent := range codingAgentAll() {
		custom := filepath.Join(home, "custom-"+agent.name())
		paths := mustAgentPaths(t, lookupOnly(agent.configVariable(), custom), home, agent)
		skillsDir := expectedSkillsDir(agent, custom, home)
		presets := expectedPresets(agent, custom)
		extensions := extensionTargets(agent, custom)
		if err := os.MkdirAll(skillsDir, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.Symlink(external, filepath.Join(skillsDir, "cafleet")); err != nil {
			t.Fatal(err)
		}
		var owned []string
		for _, p := range presets {
			owned = append(owned, p.target)
		}
		for _, extension := range extensions {
			owned = append(owned, extension.path)
		}
		for _, target := range owned {
			if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
				t.Fatal(err)
			}
			if err := os.Symlink(filepath.Join(external, "keep"), target); err != nil {
				t.Fatal(err)
			}
		}
		for range 2 {
			mustInstall(t, agent, paths)
			assertEmbeddedSkills(t, skillsDir)
			for _, p := range presets {
				want, ok := preset(p.source).get()
				if !ok {
					t.Fatalf("%s is embedded", p.source)
				}
				assertFileBytes(t, p.target, []byte(want))
			}
			assertEmbeddedExtensions(t, extensions)
			if record, ok := extensionRecord(agent, custom).get(); ok {
				assertExecutableRecord(t, record)
			}
			assertFileBytes(t, filepath.Join(external, "keep"), []byte("operator bytes"))
			if entries, err := os.ReadDir(external); err != nil || len(entries) != 1 {
				t.Fatalf("%s: a replaced symlink's referent stays intact: %v %v", agent.name(), entries, err)
			}
		}
		if agent == codingAgentOpencode {
			if _, err := os.Stat(filepath.Join(custom, "skills")); err == nil {
				t.Fatal("OpenCode skills stay out of its preset base")
			}
		}
	}
}

func TestAssets_InstallationPrintsSkillsThenPresetsThenExtensions(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	for _, agent := range codingAgentAll() {
		custom := filepath.Join(home, "custom-"+agent.name())
		output := mustInstall(t, agent, mustAgentPaths(t, lookupOnly(agent.configVariable(), custom), home, agent))
		var expected []string
		for _, p := range expectedPresets(agent, custom) {
			expected = append(expected, presetLine(agent, p.target))
		}
		for _, extension := range extensionTargets(agent, custom) {
			expected = append(expected, extensionLine(agent, extension.path))
		}
		suffix := " (v" + Version + ") -> " + expectedSkillsDir(agent, custom, home)
		if len(output) == 0 || !strings.HasPrefix(output[0], agent.name()+": installed ") ||
			strings.Contains(output[0], "installed preset") || strings.Contains(output[0], "installed extension") ||
			!strings.HasSuffix(output[0], suffix) {
			t.Fatalf("%s: skills line first: %q", agent.name(), output)
		}
		if !slices.Equal(output[1:], expected) {
			t.Fatalf("%s: lines %q, want %q", agent.name(), output[1:], expected)
		}
	}
}

func TestAssets_OpencodePresetFailureFailsInstallation(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "preset-base")
	mustWrite(t, filepath.Join(base, "agents"), "blocked parent")
	output, err := install(codingAgentOpencode, mustAgentPaths(t, lookupOnly("OPENCODE_CONFIG_DIR", base), home, codingAgentOpencode))
	assertInstallFailure(t, err, "failed to install preset into "+filepath.Join(base, "agents/cafleet.md")+": ")
	if len(output) != 1 || !strings.HasPrefix(output[0], "opencode: installed ") ||
		!strings.HasSuffix(output[0], "-> "+filepath.Join(home, ".config/opencode/skills")) {
		t.Fatalf("only the completed skills line: %q", output)
	}
	for _, absent := range []string{filepath.Join(base, "agents/cafleet-ask.md"), filepath.Join(base, "plugins")} {
		if _, err := os.Stat(absent); err == nil {
			t.Fatalf("%s is not installed after the failure", absent)
		}
	}
}

func TestAssets_OpencodeExtensionFailureFailsInstallation(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "preset-base")
	mustWrite(t, filepath.Join(base, "plugins"), "blocked parent")
	output, err := install(codingAgentOpencode, mustAgentPaths(t, lookupOnly("OPENCODE_CONFIG_DIR", base), home, codingAgentOpencode))
	first := filepath.Join(base, "plugins", embeddedExtension(opencodeExtensionPrefix)[0].path)
	assertInstallFailure(t, err, "failed to install extension into "+first+": ")
	want := []string{
		presetLine(codingAgentOpencode, filepath.Join(base, "agents/cafleet.md")),
		presetLine(codingAgentOpencode, filepath.Join(base, "agents/cafleet-ask.md")),
	}
	if len(output) == 0 || !slices.Equal(output[1:], want) {
		t.Fatalf("no extension line for a failed extension: %q", output)
	}
	assertFileBytes(t, filepath.Join(base, "plugins"), []byte("blocked parent"))
}

func TestAssets_OpencodeDefaultInstallationUsesTheNativeConfigDirectoryForEveryAsset(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	mustInstall(t, codingAgentOpencode, mustAgentPaths(t, lookupNothing, home, codingAgentOpencode))
	base := filepath.Join(home, ".config/opencode")
	for _, p := range expectedPresets(codingAgentOpencode, base) {
		want, ok := preset(p.source).get()
		if !ok {
			t.Fatalf("%s is embedded", p.source)
		}
		assertFileBytes(t, p.target, []byte(want))
	}
	assertEmbeddedExtensions(t, extensionTargets(codingAgentOpencode, base))
	assertExecutableRecord(t, filepath.Join(base, "plugins/cafleet.executable"))
}

func installPi(t *testing.T, home string, base opt[string]) ([]string, error) {
	t.Helper()
	env := lookupNothing
	if value, ok := base.get(); ok {
		env = lookupOnly("PI_CODING_AGENT_DIR", value)
	}
	return install(codingAgentPi, mustAgentPaths(t, env, home, codingAgentPi))
}

func piExtensionTarget(base string) string {
	return filepath.Join(base, "extensions/cafleet.ts")
}

func piSkillsLineMatches(line, base string) bool {
	return strings.HasPrefix(line, "pi: installed ") && strings.HasSuffix(line, " (v"+Version+") -> "+filepath.Join(base, "skills"))
}

func TestAssets_Pi_PiDefaultInstallationWritesSkillsExtensionAndRecordUnderThePiAgentDirectory(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	if _, err := installPi(t, home, none[string]()); err != nil {
		t.Fatal(err)
	}
	base := filepath.Join(home, ".pi/agent")
	assertEmbeddedSkills(t, filepath.Join(base, "skills"))
	assertEmbeddedExtensions(t, extensionTargets(codingAgentPi, base))
	index := slices.IndexFunc(embeddedExtensions(), func(file embeddedFile) bool { return file.path == "pi/cafleet.ts" })
	if index < 0 {
		t.Fatal("pi/cafleet.ts is embedded")
	}
	assertFileBytes(t, piExtensionTarget(base), embeddedExtensions()[index].data)
	assertExecutableRecord(t, filepath.Join(base, "extensions/cafleet.executable"))
}

func TestAssets_Pi_PiInstallationPrintsTheSkillsLineThenTheExtensionLine(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "pi agent")
	output, err := installPi(t, home, some(base))
	if err != nil {
		t.Fatal(err)
	}
	if len(output) != 2 || !piSkillsLineMatches(output[0], base) || output[1] != extensionLine(codingAgentPi, piExtensionTarget(base)) {
		t.Fatalf("lines = %q", output)
	}
}

func TestAssets_Pi_PiRerunPrintsTheSameLinesAndLeavesIdenticalBytesAndOneRecord(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "pi agent")
	first, err := installPi(t, home, some(base))
	if err != nil {
		t.Fatal(err)
	}
	extension, err := os.ReadFile(piExtensionTarget(base))
	if err != nil {
		t.Fatal(err)
	}
	record, err := os.ReadFile(filepath.Join(base, "extensions/cafleet.executable"))
	if err != nil {
		t.Fatal(err)
	}
	second, err := installPi(t, home, some(base))
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(second, first) {
		t.Fatalf("rerun lines %q, want %q", second, first)
	}
	assertFileBytes(t, piExtensionTarget(base), extension)
	assertFileBytes(t, filepath.Join(base, "extensions/cafleet.executable"), record)
}

func TestAssets_Pi_PiInstallationKeepsOperatorExtensionsBesideTheCafleetExtension(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "pi agent")
	extensions := filepath.Join(base, "extensions")
	mustWrite(t, filepath.Join(extensions, "operator.ts"), "operator extension")
	mustWrite(t, filepath.Join(extensions, "tools/index.ts"), "operator package")
	if _, err := installPi(t, home, some(base)); err != nil {
		t.Fatal(err)
	}
	assertFileBytes(t, filepath.Join(extensions, "operator.ts"), []byte("operator extension"))
	assertFileBytes(t, filepath.Join(extensions, "tools/index.ts"), []byte("operator package"))
}

func TestAssets_Pi_PiExtensionFailureFailsInstallation(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	base := filepath.Join(home, "pi agent")
	mustWrite(t, filepath.Join(base, "extensions"), "blocked parent")
	output, err := installPi(t, home, some(base))
	assertInstallFailure(t, err, "failed to install extension into "+piExtensionTarget(base)+": ")
	if cafleetErrorOf(err).kind != errorApp {
		t.Fatalf("an application failure: %v", err)
	}
	if len(output) != 1 || !piSkillsLineMatches(output[0], base) {
		t.Fatalf("lines = %q", output)
	}
	assertFileBytes(t, filepath.Join(base, "extensions"), []byte("blocked parent"))
}

func TestAssets_Hermes_HermesInstallationWritesOnlyTheSkillsAndPrintsOnlyTheSkillsLine(t *testing.T) {
	t.Parallel()
	home := t.TempDir()
	paths := mustAgentPaths(t, lookupNothing, home, codingAgentHermes)
	base := filepath.Join(home, ".hermes")
	for round := range 2 {
		output := mustInstall(t, codingAgentHermes, paths)
		if len(output) != 1 || !strings.HasPrefix(output[0], "hermes: installed ") ||
			!strings.HasSuffix(output[0], " (v"+Version+") -> "+filepath.Join(base, "skills")) {
			t.Fatalf("round %d: lines %q", round, output)
		}
		assertEmbeddedSkills(t, filepath.Join(base, "skills"))
		entries, err := os.ReadDir(base)
		if err != nil || len(entries) != 1 || entries[0].Name() != "skills" {
			t.Fatalf("round %d: only the skills directory: %v %v", round, entries, err)
		}
	}
}
