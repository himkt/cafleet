package core

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
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
