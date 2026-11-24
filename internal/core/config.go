package core

import (
	"os"
	"path/filepath"
	"unicode/utf8"
)

type settings struct {
	databaseURL         string
	databaseURLExplicit bool
}

// newSettings selects the supplied database URL verbatim, or the running version's default
// database; the default is the only place the home directory is expanded.
func newSettings(databaseURL opt[string]) (settings, error) {
	url, supplied := databaseURL.get()
	if !supplied {
		fallback, err := defaultDatabaseURL()
		if err != nil {
			return settings{}, err
		}
		url = fallback
	}
	return settings{databaseURL: url, databaseURLExplicit: supplied}, nil
}

func defaultDatabaseURL() (string, error) {
	home, err := homeDir()
	if err != nil {
		return "", appError(cafleetErrorOf(err).message() + "; cannot derive the default database path")
	}
	return "sqlite:///" + home + "/.local/share/cafleet/cafleet_" + Version + ".db", nil
}

type envLookup func(name string) (opt[string], error)

func processEnv(name string) (opt[string], error) {
	value, set := os.LookupEnv(name)
	if !set {
		return none[string](), nil
	}
	if !utf8.ValidString(value) {
		return none[string](), appError(name + " is not valid UTF-8")
	}
	return some(value), nil
}

// opencodeSkillsBase is pinned to opencode's fixed skill discovery path, so it ignores every
// variable.
func opencodeSkillsBase(home string) string {
	return joinPath(home, ".config/opencode")
}

func homeDir() (string, error) {
	home, err := processEnv("HOME")
	if err != nil {
		return "", err
	}
	value, set := home.get()
	if !set {
		return "", appError("HOME is not set")
	}
	return value, nil
}

func requireAbsolute(variable, value string) error {
	if filepath.IsAbs(value) {
		return nil
	}
	return appError(variable + " must be an absolute path (got '" + value + "')")
}

func resolve(variable string, env envLookup, fallback string) (string, error) {
	found, err := env(variable)
	if err != nil {
		return "", err
	}
	value, set := found.get()
	if !set {
		return fallback, nil
	}
	if err := requireAbsolute(variable, value); err != nil {
		return "", err
	}
	return value, nil
}
