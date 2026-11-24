package core

import (
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
