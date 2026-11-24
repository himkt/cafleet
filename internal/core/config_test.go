package core

import (
	"errors"
	"os"
	"strings"
	"testing"
)

func settingsForDatabase(path string) settings {
	selected, err := newSettings(some("sqlite:///" + path))
	if err != nil {
		panic(err)
	}
	return selected
}

func TestConfig_SettingsSelectTheSuppliedDatabaseUrlVerbatimOrTheRunningVersionsDefault(t *testing.T) {
	t.Parallel()
	fallback := "sqlite:///" + os.Getenv("HOME") + "/.local/share/cafleet/cafleet_" + Version + ".db"
	if strings.Contains(fallback, "~") {
		t.Fatalf("the default URL expands the home directory at startup: %s", fallback)
	}
	selected := func(url opt[string]) settings {
		chosen, err := newSettings(url)
		if err != nil {
			t.Fatal(err)
		}
		return chosen
	}
	defaultURL, err := defaultDatabaseURL()
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name     string
		settings settings
		url      string
		explicit bool
	}{
		{name: "omitted URL", settings: selected(none[string]()), url: fallback, explicit: false},
		{name: "supplied URL without ~ expansion", settings: selected(some("sqlite:///~/custom/path.db")), url: "sqlite:///~/custom/path.db", explicit: true},
		{name: "supplied absolute URL", settings: selected(some("sqlite:///tmp/fleet.db")), url: "sqlite:///tmp/fleet.db", explicit: true},
		{name: "supplied URL naming the default database", settings: selected(some(defaultURL)), url: fallback, explicit: true},
		{name: "database path supplied through settingsForDatabase", settings: settingsForDatabase("/tmp/fleet.db"), url: "sqlite:////tmp/fleet.db", explicit: true},
	}
	for _, tc := range cases {
		if tc.settings.databaseURL != tc.url {
			t.Fatalf("%s: databaseURL = %q, want %q", tc.name, tc.settings.databaseURL, tc.url)
		}
		if tc.settings.databaseURLExplicit != tc.explicit {
			t.Fatalf("%s: databaseURLExplicit = %v, want %v", tc.name, tc.settings.databaseURLExplicit, tc.explicit)
		}
	}
}

func TestConfig_OpencodeSkillsBaseIsPinnedToTheFixedDiscoveryPath(t *testing.T) {
	t.Parallel()
	if got := opencodeSkillsBase("/home/tester"); got != "/home/tester/.config/opencode" {
		t.Fatalf("opencodeSkillsBase = %q", got)
	}
}

func TestConfig_RequireAbsoluteRejectsNonAbsoluteValues(t *testing.T) {
	t.Parallel()
	if err := requireAbsolute("HERMES_HOME", "/srv/hermes"); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{"relative", "", "./hermes", "~/hermes"} {
		err := requireAbsolute("HERMES_HOME", value)
		want := "HERMES_HOME must be an absolute path (got '" + value + "')"
		var failure *cafleetError
		if !errors.As(err, &failure) || failure.kind != errorApp || failure.message() != want {
			t.Fatalf("%q: requireAbsolute = %v, want %q", value, err, want)
		}
	}
}

func TestConfig_ResolveUsesAnAbsoluteVariableOrTheDefault(t *testing.T) {
	t.Parallel()
	lookup := func(values map[string]string) envLookup {
		return func(name string) (opt[string], error) {
			if value, ok := values[name]; ok {
				return some(value), nil
			}
			return none[string](), nil
		}
	}
	cases := []struct {
		name   string
		values map[string]string
		want   string
		cause  string
	}{
		{name: "unset", values: map[string]string{}, want: "/home/tester/.hermes"},
		{name: "absolute", values: map[string]string{"HERMES_HOME": "/srv/hermes"}, want: "/srv/hermes"},
		{name: "relative", values: map[string]string{"HERMES_HOME": "hermes"}, cause: "HERMES_HOME must be an absolute path (got 'hermes')"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := resolve("HERMES_HOME", lookup(tc.values), "/home/tester/.hermes")
			if tc.cause != "" {
				if err == nil || err.Error() != tc.cause {
					t.Fatalf("%s: resolve = %q %v, want %q", tc.name, got, err, tc.cause)
				}
				return
			}
			if err != nil || got != tc.want {
				t.Fatalf("%s: resolve = %q %v, want %q", tc.name, got, err, tc.want)
			}
		})
	}
	failing := func(string) (opt[string], error) { return none[string](), appError("HERMES_HOME is not valid UTF-8") }
	if _, err := resolve("HERMES_HOME", failing, "/fallback"); err == nil || err.Error() != "HERMES_HOME is not valid UTF-8" {
		t.Fatalf("a failing lookup: resolve error %v", err)
	}
}

//nolint:paralleltest // sets process environment variables
func TestConfig_ProcessEnvReportsUnsetSetAndNonUtf8Values(t *testing.T) {
	const name = "CAFLEET_TEST_PROCESS_ENV"
	cases := []struct {
		name  string
		value opt[string]
		want  opt[string]
		cause string
	}{
		{name: "unset", value: none[string](), want: none[string]()},
		{name: "empty", value: some(""), want: some("")},
		{name: "value", value: some("/srv/home"), want: some("/srv/home")},
		{name: "non-UTF-8", value: some("/srv/\xff"), cause: name + " is not valid UTF-8"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if value, ok := tc.value.get(); ok {
				t.Setenv(name, value)
			} else {
				t.Setenv(name, "")
				if err := os.Unsetenv(name); err != nil {
					t.Fatal(err)
				}
			}
			got, err := processEnv(name)
			if tc.cause != "" {
				var failure *cafleetError
				if !errors.As(err, &failure) || failure.kind != errorApp || failure.message() != tc.cause {
					t.Fatalf("%s: processEnv = %v %v, want %q", tc.name, got, err, tc.cause)
				}
				return
			}
			if err != nil || got != tc.want {
				t.Fatalf("%s: processEnv = %v %v, want %v", tc.name, got, err, tc.want)
			}
		})
	}
}

//nolint:paralleltest // sets HOME for the process
func TestConfig_DefaultDatabaseUrlNeedsAUtf8Home(t *testing.T) {
	cases := []struct {
		name  string
		home  opt[string]
		want  string
		cause string
	}{
		{name: "home", home: some("/home/tester"), want: "sqlite:////home/tester/.local/share/cafleet/cafleet_" + Version + ".db"},
		{name: "unset", home: none[string](), cause: "HOME is not set; cannot derive the default database path"},
		{name: "non-UTF-8", home: some("/home/\xff"), cause: "HOME is not valid UTF-8; cannot derive the default database path"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if home, ok := tc.home.get(); ok {
				t.Setenv("HOME", home)
			} else {
				t.Setenv("HOME", "")
				if err := os.Unsetenv("HOME"); err != nil {
					t.Fatal(err)
				}
			}
			got, err := defaultDatabaseURL()
			if tc.cause != "" {
				var failure *cafleetError
				if !errors.As(err, &failure) || failure.kind != errorApp || failure.message() != tc.cause {
					t.Fatalf("%s: defaultDatabaseURL = %q %v, want %q", tc.name, got, err, tc.cause)
				}
				if _, err := newSettings(none[string]()); err == nil || err.Error() != tc.cause {
					t.Fatalf("%s: newSettings without a URL: %v, want %q", tc.name, err, tc.cause)
				}
				return
			}
			if err != nil || got != tc.want {
				t.Fatalf("%s: defaultDatabaseURL = %q %v, want %q", tc.name, got, err, tc.want)
			}
			home, err := homeDir()
			if err != nil || home != "/home/tester" {
				t.Fatalf("%s: homeDir = %q %v", tc.name, home, err)
			}
		})
	}
}
