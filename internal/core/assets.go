package core

import (
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"unicode/utf8"

	"github.com/himkt/cafleet"
)

const (
	opencodeExtensionPrefix  = "opencode/"
	opencodeExecutableRecord = "cafleet.executable"
	piExtensionPrefix        = "pi/"
	piExecutableRecord       = "cafleet.executable"
)

type agentPaths struct {
	skillsDir string
	presets   []presetTarget
	extension opt[fileExtension]
}

// presetTarget is an embedded preset, by its path in the presets tree, and its install target.
type presetTarget struct {
	source string
	target string
}

// fileExtension is a backend whose extension is a set of embedded files installed beside one
// executable record.
type fileExtension struct {
	prefix string
	dir    string
	record string
}

// assetsAgentPaths is the Rust agent_paths, prefixed because it collides with the agentPaths type.
func assetsAgentPaths(env envLookup, home string, agent codingAgent) (agentPaths, error) {
	base, err := agent.configDir(env, home)
	if err != nil {
		return agentPaths{}, err
	}
	var presets []presetTarget
	switch agent {
	case codingAgentClaude, codingAgentHermes, codingAgentPi:
	case codingAgentCodex:
		presets = []presetTarget{{source: "codex/cafleet.rules", target: joinPath(base, "rules/cafleet.rules")}}
	case codingAgentOpencode:
		for _, mode := range agent.policyChoices() {
			target, set := agent.requiredPreset(base, mode).get()
			if !set {
				panic("OpenCode requires a preset for every launch policy")
			}
			presets = append(presets, presetTarget{source: "opencode/" + mode + ".md", target: target})
		}
	default:
		panic("invalid codingAgent")
	}
	extension := none[fileExtension]()
	switch agent {
	case codingAgentOpencode:
		extension = some(fileExtension{prefix: opencodeExtensionPrefix, dir: opencodePluginsDir(base), record: opencodeExecutableRecord})
	case codingAgentPi:
		extension = some(fileExtension{prefix: piExtensionPrefix, dir: piExtensionsDir(base), record: piExecutableRecord})
	case codingAgentClaude, codingAgentCodex, codingAgentHermes:
	default:
		panic("invalid codingAgent")
	}
	return agentPaths{skillsDir: skillsDir(agent, base, home), presets: presets, extension: extension}, nil
}

// skillsDir is where setup installs agent's skills, given its resolved configuration base.
func skillsDir(agent codingAgent, base, home string) string {
	if agent == codingAgentOpencode {
		return joinPath(opencodeSkillsBase(home), "skills")
	}
	return joinPath(base, "skills")
}

type embeddedFile struct {
	path string
	data []byte
}

// embeddedTable is every file of the embedded tree under root, at its `/`-separated path relative
// to root, sorted bytewise by that path; entries whose name starts with `.` are skipped with
// everything beneath them (F12). An empty tree is a build defect.
func embeddedTable(tree fs.FS, root string) []embeddedFile {
	var files []embeddedFile
	err := fs.WalkDir(tree, root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if path != root && strings.HasPrefix(entry.Name(), ".") {
			if entry.IsDir() {
				return fs.SkipDir
			}
			return nil
		}
		if entry.IsDir() {
			return nil
		}
		data, err := fs.ReadFile(tree, path)
		if err != nil {
			return err
		}
		files = append(files, embeddedFile{path: strings.TrimPrefix(path, root+"/"), data: data})
		return nil
	})
	if err != nil {
		panic(fmt.Sprintf("the embedded %s tree is readable: %v", root, err))
	}
	if len(files) == 0 {
		panic(fmt.Sprintf("the %s tree is empty", root))
	}
	slices.SortFunc(files, func(a, b embeddedFile) int { return strings.Compare(a.path, b.path) })
	return files
}

var (
	embeddedSkills     = sync.OnceValue(func() []embeddedFile { return embeddedTable(cafleet.Assets, "assets/skills") })
	embeddedPresets    = sync.OnceValue(func() []embeddedFile { return embeddedTable(cafleet.Assets, "assets/presets") })
	embeddedExtensions = sync.OnceValue(func() []embeddedFile { return embeddedTable(cafleet.Assets, "assets/extensions") })
)

func opencodePluginsDir(presetBase string) string {
	return joinPath(presetBase, "plugins")
}

func piExtensionsDir(piBase string) string {
	return joinPath(piBase, "extensions")
}

// embeddedExtension is every embedded extension file under prefix, with its path relative to the
// extension root.
func embeddedExtension(prefix string) []embeddedFile {
	var files []embeddedFile
	for _, file := range embeddedExtensions() {
		if rest, ok := strings.CutPrefix(file.path, prefix); ok {
			files = append(files, embeddedFile{path: rest, data: file.data})
		}
	}
	if len(files) == 0 {
		panic(fmt.Sprintf("the %s extension is embedded", prefix))
	}
	return files
}

func installAgent(agent codingAgent, paths agentPaths, out io.Writer) error {
	report := func(line string) error {
		_, err := io.WriteString(out, line+"\n")
		return outputResult(err)
	}
	fail := func(err error) error {
		return appErrorf("failed to install skills into %s: %s", paths.skillsDir, ioErrorText(err))
	}
	var roots []string
	for _, file := range embeddedSkills() {
		if root, ok := strings.CutSuffix(file.path, "/SKILL.md"); ok && !strings.Contains(root, "/") {
			roots = append(roots, root)
		}
	}
	if len(roots) == 0 {
		panic("embedded skills have top-level manifests")
	}
	for _, skill := range roots {
		if err := removeExistingTarget(joinPath(paths.skillsDir, skill)); err != nil {
			return fail(err)
		}
		for _, file := range embeddedSkills() {
			if !strings.HasPrefix(file.path, skill+"/") {
				continue
			}
			target := joinPath(paths.skillsDir, file.path)
			if err := createDirAll(filepath.Dir(target)); err != nil {
				return fail(err)
			}
			if err := os.WriteFile(target, file.data, 0o666); err != nil {
				return fail(err)
			}
		}
	}
	if err := report(fmt.Sprintf("%s: installed %s (v%s) -> %s", agent.name(), strings.Join(roots, ", "), Version, paths.skillsDir)); err != nil {
		return err
	}
	for _, installed := range paths.presets {
		failPreset := func(err error) error {
			return appErrorf("failed to install preset into %s: %s", installed.target, ioErrorText(err))
		}
		data, set := preset(installed.source).get()
		if !set {
			panic("embedded preset exists")
		}
		if err := createDirAll(filepath.Dir(installed.target)); err != nil {
			return failPreset(err)
		}
		if err := removeExistingTarget(installed.target); err != nil {
			return failPreset(err)
		}
		if err := os.WriteFile(installed.target, []byte(data), 0o666); err != nil {
			return failPreset(err)
		}
		if err := report(fmt.Sprintf("%s: installed preset (v%s) -> %s", agent.name(), Version, installed.target)); err != nil {
			return err
		}
	}
	if extension, set := paths.extension.get(); set {
		installed, err := installFileExtension(extension)
		if err != nil {
			return err
		}
		for _, target := range installed {
			if err := report(fmt.Sprintf("%s: installed extension (v%s) -> %s", agent.name(), Version, target)); err != nil {
				return err
			}
		}
	}
	return nil
}

func extensionFailure(target, cause string) error {
	return appErrorf("failed to install extension into %s: %s", target, cause)
}

// installFileExtension writes each embedded extension file, then the executable record, and
// returns the files.
func installFileExtension(extension fileExtension) ([]string, error) {
	var installed []string
	for _, file := range embeddedExtension(extension.prefix) {
		target := joinPath(extension.dir, file.path)
		if err := createDirAll(filepath.Dir(target)); err != nil {
			return nil, extensionFailure(target, ioErrorText(err))
		}
		if err := removeExistingTarget(target); err != nil {
			return nil, extensionFailure(target, ioErrorText(err))
		}
		if err := os.WriteFile(target, file.data, 0o666); err != nil {
			return nil, extensionFailure(target, ioErrorText(err))
		}
		installed = append(installed, target)
	}
	if err := writeExecutableRecord(joinPath(extension.dir, extension.record)); err != nil {
		return nil, err
	}
	return installed, nil
}

func writeExecutableRecord(record string) error {
	executable, err := os.Executable()
	if err == nil {
		executable, err = canonicalizePath(executable)
	}
	if err != nil {
		return extensionFailure(record, "cannot resolve the running cafleet: "+ioErrorText(err))
	}
	if err := removeExistingTarget(record); err != nil {
		return extensionFailure(record, ioErrorText(err))
	}
	if err := os.WriteFile(record, []byte(executable+"\n"), 0o666); err != nil {
		return extensionFailure(record, ioErrorText(err))
	}
	return nil
}

// executableRecordProblem is why an installed executable record is unusable, or none when it
// names an executable file.
func executableRecordProblem(record string) opt[string] {
	data, err := os.ReadFile(record)
	if err != nil {
		return some(ioErrorText(err))
	}
	if !utf8.Valid(data) {
		return some("stream did not contain valid UTF-8")
	}
	target := strings.TrimSpace(string(data))
	if target == "" {
		return some("the record is empty")
	}
	info, err := os.Stat(target)
	switch {
	case err != nil:
		return some(target + ": " + ioErrorText(err))
	case info.Mode().IsRegular() && info.Mode().Perm()&0o111 != 0:
		return none[string]()
	default:
		return some(target + " is not an executable file")
	}
}

// skillFiles is every embedded skill file at its install path under skillsDir.
func skillFiles(skillsDir string) []embeddedFile {
	skills := embeddedSkills()
	files := make([]embeddedFile, 0, len(skills))
	for _, file := range skills {
		files = append(files, embeddedFile{path: joinPath(skillsDir, file.path), data: file.data})
	}
	return files
}

// installedFileProblem is why an installed copy of an embedded file is unusable, or none when it
// equals the embedded bytes.
func installedFileProblem(target string, embedded []byte) opt[string] {
	data, err := os.ReadFile(target)
	switch {
	case err != nil:
		return some(ioErrorText(err))
	case string(data) == string(embedded):
		return none[string]()
	default:
		return some("differs from the embedded copy")
	}
}

func removeExistingTarget(target string) error {
	info, err := os.Lstat(target)
	switch {
	case errors.Is(err, fs.ErrNotExist):
		return nil
	case err != nil:
		return err
	case info.IsDir():
		return os.RemoveAll(target)
	default:
		return os.Remove(target)
	}
}

// preset is the embedded preset at path, a path relative to the presets tree.
func preset(path string) opt[string] {
	for _, file := range embeddedPresets() {
		if file.path == path {
			return some(string(file.data))
		}
	}
	return none[string]()
}
