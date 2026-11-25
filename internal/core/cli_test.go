package core

import (
	"bytes"
	"context"
	"encoding/json/v2"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/himkt/cafleet"
)

func parseCli(args ...string) (cliArgs, *cliParseExit) {
	return cliArgsParseFrom(append([]string{"cafleet"}, args...))
}

func mustParse(t *testing.T, name string, args ...string) cliCommand {
	t.Helper()
	parsed, exit := parseCli(args...)
	if exit != nil {
		t.Fatalf("%s: %q: %s", name, args, exit.text)
	}
	return parsed.command
}

// parseOutcome returns the parse exit of args: exit 2 for a parse error, exit 0 for displayed
// help or version.
func parseOutcome(t *testing.T, name string, args ...string) *cliParseExit {
	t.Helper()
	parsed, exit := parseCli(args...)
	if exit == nil {
		t.Fatalf("%s: %q parsed as %#v", name, args, parsed)
	}
	return exit
}

func firstLine(text string) string {
	line, _, _ := strings.Cut(text, "\n")
	return line
}

// assertParseError checks the parse-error contract of SPEC §6.3 Error Messages: exit 2, a first
// line that begins with `error: `, and a message that names the offending input.
func assertParseError(t *testing.T, name string, args []string, input string) {
	t.Helper()
	outcome := parseOutcome(t, name, args...)
	if outcome.exitCode() != 2 {
		t.Fatalf("%s: %q exits %d, want 2: %s", name, args, outcome.exitCode(), outcome.text)
	}
	if line := firstLine(outcome.text); !strings.HasPrefix(line, "error: ") {
		t.Fatalf("%s: %q: first line %q does not begin with %q", name, args, line, "error: ")
	}
	if !strings.Contains(outcome.text, input) {
		t.Fatalf("%s: %q: the error does not name %q:\n%s", name, args, input, outcome.text)
	}
}

func helpText(t *testing.T, name string, path ...string) string {
	t.Helper()
	outcome := parseOutcome(t, name, append(path, "--help")...)
	if outcome.exitCode() != 0 {
		t.Fatalf("%s: %q --help exits %d: %s", name, path, outcome.exitCode(), outcome.text)
	}
	return outcome.text
}

func flattened(text string) string {
	return strings.Join(strings.Fields(text), " ")
}

func TestCli_CommandOptionsAreScopedToTheirSubcommands(t *testing.T) {
	t.Parallel()
	rejected := []struct {
		name  string
		args  []string
		input string
	}{
		{name: "json before the subcommand", args: []string{"--json", "fleet", "list"}, input: "--json"},
		{name: "setup", args: []string{"setup", "--unknown"}, input: "--unknown"},
		{name: "fleet list", args: []string{"fleet", "list", "--unknown"}, input: "--unknown"},
		{name: "member show", args: []string{"member", "show", "1", "--unknown"}, input: "--unknown"},
		{name: "message poll", args: []string{"message", "poll", "1", "--unknown"}, input: "--unknown"},
		{name: "event show", args: []string{"event", "show", "1", "--unknown"}, input: "--unknown"},
		{name: "input show", args: []string{"input", "show", "1", "--unknown"}, input: "--unknown"},
	}
	for _, tc := range rejected {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			assertParseError(t, tc.name, tc.args, tc.input)
		})
	}
	accepted := []struct {
		name string
		args []string
		want cliCommand
	}{
		{name: "fleet list json", args: []string{"fleet", "list", "--json"}, want: fleetCommandList{json: true}},
		{name: "fleet show json", args: []string{"fleet", "show", "41", "--json"}, want: fleetCommandShow{fleetID: 41, json: true}},
		{name: "member show json", args: []string{"member", "show", "42", "--json"}, want: memberCommandShow{memberID: 42, json: true}},
		{name: "message poll json", args: []string{"message", "poll", "43", "--json"}, want: messageCommandPoll{memberID: 43, json: true}},
	}
	for _, tc := range accepted {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if parsed := mustParse(t, tc.name, tc.args...); parsed != tc.want {
				t.Fatalf("%s: %q parsed as %#v, want %#v", tc.name, tc.args, parsed, tc.want)
			}
		})
	}
}

func TestCli_MemberAndMessageBodySourcesAreExclusive(t *testing.T) {
	t.Parallel()
	bases := []struct {
		name  string
		args  []string
		check func(cliCommand) bool
	}{
		{
			name: "member create",
			args: []string{"member", "create", "--fleet-id", "17", "--name", "worker", "--description", "work"},
			check: func(parsed cliCommand) bool {
				create, ok := parsed.(memberCommandCreate)
				return ok && create.fleetID == 17 && create.name == "worker" && create.description == "work"
			},
		},
		{
			name: "message send",
			args: []string{"message", "send", "--from-member-id", "17", "--to-member-id", "18"},
			check: func(parsed cliCommand) bool {
				send, ok := parsed.(messageCommandSend)
				return ok && send.fromMemberID == 17 && send.toMemberID == 18
			},
		},
		{
			name: "member exec",
			args: []string{"member", "exec", "17", "--from-member-id", "1"},
			check: func(parsed cliCommand) bool {
				native, ok := parsed.(memberCommandExec)
				return ok && native.memberID == 17
			},
		},
	}
	sources := []struct {
		name string
		args []string
	}{
		{name: "positional", args: []string{"payload"}},
		{name: "file", args: []string{"--file", "body.txt"}},
		{name: "stdin", args: []string{"--file", "-"}},
	}
	for _, base := range bases {
		t.Run(base.name+" without a body", func(t *testing.T) {
			t.Parallel()
			assertParseError(t, base.name, base.args, "--file")
		})
		t.Run(base.name+" with both bodies", func(t *testing.T) {
			t.Parallel()
			both := append(append([]string{}, base.args...), "payload", "--file", "body.txt")
			assertParseError(t, base.name, both, "--file")
		})
		for _, source := range sources {
			t.Run(base.name+" with a "+source.name+" body", func(t *testing.T) {
				t.Parallel()
				args := append(append([]string{}, base.args...), source.args...)
				if parsed := mustParse(t, base.name, args...); !base.check(parsed) {
					t.Fatalf("%s: %q must preserve its command variant, parsed as %#v", base.name, args, parsed)
				}
			})
		}
	}
}

func TestCli_MemberRunnerInternalCommandAcceptsAnAbsoluteDescriptorAndStaysHidden(t *testing.T) {
	t.Parallel()
	const descriptor = "/private/tmp/allocated-run/descriptor.json"
	parsed := mustParse(t, "member run", "__member-run", "--descriptor", descriptor)
	if parsed != cliCommand(memberRunCommand{descriptor: descriptor}) {
		t.Fatalf("__member-run parsed as %#v", parsed)
	}
	if help := helpText(t, "root help"); strings.Contains(help, "__member-run") {
		t.Fatalf("root help lists the hidden runner command:\n%s", help)
	}
	if outcome := parseOutcome(t, "missing descriptor", "__member-run", "--descriptor"); outcome.exitCode() != 2 {
		t.Fatalf("__member-run without a descriptor value exits %d: %s", outcome.exitCode(), outcome.text)
	}
}

func TestCli_HelpDescribesEveryArgumentOfTheRuntimeReadInputAndPressCommands(t *testing.T) {
	t.Parallel()
	const json = "Output in JSON format"
	const director = "The root Director issuing this Director-only command"
	cases := []struct {
		name      string
		path      []string
		arguments map[string]string
	}{
		{
			name: "event show",
			path: []string{"event", "show"},
			arguments: map[string]string{
				"<EVENT_ID>": "The runtime event to read",
				"--json":     json,
			},
		},
		{
			name: "input show",
			path: []string{"input", "show"},
			arguments: map[string]string{
				"<INPUT_ID>": "The programmatic-input receipt to read",
				"--json":     json,
			},
		},
		{
			name: "input reconcile",
			path: []string{"input", "reconcile"},
			arguments: map[string]string{
				"<INPUT_ID>":       "The programmatic-input receipt to settle",
				"--from-member-id": director,
				"--disposition":    "The observed input disposition to record",
				"--evidence-ref":   "Reference to the Director's recorded evidence for the disposition",
				"--json":           json,
			},
		},
		{
			name: "member press",
			path: []string{"member", "press"},
			arguments: map[string]string{
				"<MEMBER_ID>":      "The target member",
				"--from-member-id": director,
				"<KEY>":            "One to 64 keys sent in order, with no implicit Escape or Enter",
				"--json":           json,
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			help := flattened(helpText(t, tc.name, tc.path...))
			for argument, description := range tc.arguments {
				if !strings.Contains(help, argument) {
					t.Fatalf("%s: help names no %s:\n%s", tc.name, argument, help)
				}
				if !strings.Contains(help, description) {
					t.Fatalf("%s: help lacks the description of %s (%q):\n%s", tc.name, argument, description, help)
				}
			}
		})
	}
}

func TestCli_HelpListsTheDispositionChoicesAndAnInvalidDispositionNamesThem(t *testing.T) {
	t.Parallel()
	choices := []string{"submitted", "not-submitted", "abandoned-uncertain"}
	help := flattened(helpText(t, "input reconcile", "input", "reconcile"))
	if want := "[possible values: " + strings.Join(choices, ", ") + "]"; !strings.Contains(help, want) {
		t.Fatalf("input reconcile help lacks %q:\n%s", want, help)
	}
	args := []string{"input", "reconcile", "1", "--from-member-id", "1", "--disposition", "maybe", "--evidence-ref", "seen"}
	assertParseError(t, "invalid disposition", args, "maybe")
	rendered := parseOutcome(t, "invalid disposition", args...).text
	for _, expected := range append([]string{"--disposition"}, choices...) {
		if !strings.Contains(rendered, expected) {
			t.Fatalf("the invalid-disposition error does not name %q:\n%s", expected, rendered)
		}
	}
}

func TestCli_DatabaseUrlIsAcceptedOncePerLevelAndTheDeepestOccurrenceWins(t *testing.T) {
	t.Parallel()
	selected := []struct {
		name string
		args []string
		want opt[string]
	}{
		{name: "omitted", args: []string{"fleet", "list"}, want: none[string]()},
		{name: "root level", args: []string{"--database-url", "sqlite:///a.db", "fleet", "list"}, want: some("sqlite:///a.db")},
		{name: "group level", args: []string{"fleet", "--database-url", "sqlite:///g.db", "list"}, want: some("sqlite:///g.db")},
		{name: "leaf level", args: []string{"message", "poll", "7", "--database-url", "sqlite:///l.db"}, want: some("sqlite:///l.db")},
		{name: "equals form", args: []string{"fleet", "list", "--database-url=sqlite:///e.db"}, want: some("sqlite:///e.db")},
		{name: "leaf beats root", args: []string{"--database-url", "sqlite:///a.db", "message", "poll", "7", "--database-url", "sqlite:///b.db"}, want: some("sqlite:///b.db")},
		{name: "group beats root", args: []string{"--database-url", "sqlite:///a.db", "fleet", "--database-url", "sqlite:///g.db", "list"}, want: some("sqlite:///g.db")},
		{name: "leaf beats group", args: []string{"message", "--database-url", "sqlite:///g.db", "poll", "7", "--database-url", "sqlite:///l.db"}, want: some("sqlite:///l.db")},
	}
	for _, tc := range selected {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			parsed, exit := parseCli(tc.args...)
			if exit != nil {
				t.Fatalf("%s: %q: %s", tc.name, tc.args, exit.text)
			}
			if parsed.databaseURL != tc.want {
				t.Fatalf("%s: %q selects %#v, want %#v", tc.name, tc.args, parsed.databaseURL, tc.want)
			}
		})
	}
	repeated := []struct {
		name string
		args []string
	}{
		{name: "root level", args: []string{"--database-url", "sqlite:///a.db", "--database-url", "sqlite:///b.db", "fleet", "list"}},
		{name: "group level", args: []string{"fleet", "--database-url", "sqlite:///a.db", "--database-url", "sqlite:///b.db", "list"}},
		{name: "leaf level", args: []string{"message", "poll", "7", "--database-url", "sqlite:///a.db", "--database-url", "sqlite:///b.db"}},
	}
	for _, tc := range repeated {
		t.Run("repeated at the "+tc.name, func(t *testing.T) {
			t.Parallel()
			assertParseError(t, tc.name, tc.args, "--database-url")
		})
	}
}

func TestCli_BodyFilesPreserveExactBytesAndReportInvalidSources(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	preserved := []struct {
		name string
		text string
	}{
		{name: "unicode with mixed line endings", text: "界🙂\nnext\r\n"},
		{name: "literal braces and tab", text: "{literal}\ttext"},
		{name: "surrounding spaces", text: " padded "},
	}
	for index, tc := range preserved {
		t.Run("preserves "+tc.name, func(t *testing.T) {
			t.Parallel()
			path := filepath.Join(dir, fmt.Sprintf("preserved-%d.txt", index))
			if err := os.WriteFile(path, []byte(tc.text), 0o600); err != nil {
				t.Fatal(err)
			}
			got, err := resolveBody(none[string](), some(path))
			if err != nil {
				t.Fatalf("%s: %v", tc.name, err)
			}
			if got != tc.text {
				t.Fatalf("%s: body %q, want %q", tc.name, got, tc.text)
			}
		})
	}
	blank := []struct {
		name string
		text string
	}{
		{name: "empty", text: ""},
		{name: "ascii whitespace", text: " \t\n"},
		{name: "information separators", text: "\u001c\u001f"},
		{name: "em space", text: "\u2003"},
	}
	for index, tc := range blank {
		t.Run("refuses a blank "+tc.name+" body", func(t *testing.T) {
			t.Parallel()
			_, err := resolveBody(some(tc.text), none[string]())
			assertCafleetError(t, tc.name+" positional", err, 2, "text may not be empty.")
			path := filepath.Join(dir, fmt.Sprintf("blank-%d.txt", index))
			if err := os.WriteFile(path, []byte(tc.text), 0o600); err != nil {
				t.Fatal(err)
			}
			_, err = resolveBody(none[string](), some(path))
			assertCafleetError(t, tc.name+" file", err, 1, "--file "+path+": file is empty.")
		})
	}
	t.Run("invalid utf-8", func(t *testing.T) {
		t.Parallel()
		path := filepath.Join(dir, "invalid.txt")
		if err := os.WriteFile(path, []byte{0xff}, 0o600); err != nil {
			t.Fatal(err)
		}
		_, err := resolveBody(none[string](), some(path))
		assertCafleetError(t, "invalid utf-8", err, 1, "--file "+path+": file is not valid UTF-8.")
	})
	t.Run("missing file", func(t *testing.T) {
		t.Parallel()
		path := filepath.Join(dir, "missing.txt")
		_, err := resolveBody(none[string](), some(path))
		assertCafleetError(t, "missing file", err, 1,
			"--file "+path+": file does not exist or is not a regular file. (No such file or directory (os error 2))")
	})
	t.Run("directory", func(t *testing.T) {
		t.Parallel()
		_, err := resolveBody(none[string](), some(dir))
		assertCafleetError(t, "directory", err, 1,
			"--file "+dir+": file does not exist or is not a regular file. (Is a directory (os error 21))")
	})
}

func assertCafleetError(t *testing.T, name string, err error, code int, message string) {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: resolved without an error", name)
	}
	cafleet := cafleetErrorOf(err)
	if cafleet.exitCode() != code {
		t.Fatalf("%s: exit %d, want %d: %v", name, cafleet.exitCode(), code, err)
	}
	if cafleet.message() != message {
		t.Fatalf("%s: message %q, want %q", name, cafleet.message(), message)
	}
}

const cliChildStdoutMarker = "cafleet-guard-child-stdout-begins"

// TestCli_DirectorOnlyGuard_GuardedRunChild runs Main with the arguments after `--`; the marker
// separates the test binary's own stdout from the command's.
func TestCli_DirectorOnlyGuard_GuardedRunChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	fmt.Println(cliChildStdoutMarker)
	os.Exit(Main(append([]string{"cafleet"}, flag.Args()...), cafleet.Assets))
}

type cliChildOutcome struct {
	code   int
	stdout string
	stderr string
}

// runCliChild runs Main in a child under a fresh HOME, with an explicit --database-url to an
// absent file in a fresh directory unless argv supplies one, so no run reaches the real
// configuration homes or default database.
func runCliChild(t *testing.T, argv ...string) cliChildOutcome {
	t.Helper()
	if !slices.Contains(argv, "--database-url") {
		argv = append([]string{"--database-url", "sqlite:///" + filepath.Join(t.TempDir(), "absent.db")}, argv...)
	}
	child := childTestCommand(TestCli_DirectorOnlyGuard_GuardedRunChild, append([]string{"--"}, argv...)...)
	child.Env = append(child.Env, "HOME="+t.TempDir())
	var stdout, stderr bytes.Buffer
	child.Stdout = &stdout
	child.Stderr = &stderr
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	expired := time.AfterFunc(10*time.Second, func() {
		_ = child.Process.Kill()
	})
	err := child.Wait()
	if !expired.Stop() {
		t.Fatalf("%q exceeded its deadline: %s", argv, stderr.String())
	}
	code := 0
	var exit *exec.ExitError
	switch {
	case errors.As(err, &exit):
		code = exit.ExitCode()
	case err != nil:
		t.Fatalf("%q: %v", argv, err)
	}
	_, cliStdout, found := strings.Cut(stdout.String(), cliChildStdoutMarker+"\n")
	if !found {
		t.Fatalf("%q: the child did not reach Main: %q %q", argv, stdout.String(), stderr.String())
	}
	return cliChildOutcome{code: code, stdout: cliStdout, stderr: stderr.String()}
}

func TestCli_DirectorOnlyGuard_RunArgsReportsParseErrorsBeforeTheGuard(t *testing.T) {
	t.Parallel()
	outcome := runCliChild(t, "member", "prompt", "7")
	if outcome.code != 2 {
		t.Fatalf("exit %d, want 2: %s", outcome.code, outcome.stderr)
	}
	if strings.Contains(outcome.stderr, "Director-only") {
		t.Fatalf("the guard ran before parsing: %s", outcome.stderr)
	}
	if !strings.Contains(outcome.stderr, "TEXT") {
		t.Fatalf("the parse error does not name TEXT: %s", outcome.stderr)
	}
	if outcome.stdout != "" {
		t.Fatalf("a parse error wrote stdout %q", outcome.stdout)
	}
}

func TestCli_ParseErrorsWriteAnErrorLineAndUsageOutsideValueErrorsToStderrAndExitTwo(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		args  []string
		names []string
		usage bool
	}{
		{name: "unknown subcommand", args: []string{"nonexistent"}, names: []string{"nonexistent"}, usage: true},
		{name: "unknown option", args: []string{"fleet", "list", "--bogus"}, names: []string{"--bogus"}, usage: true},
		{name: "missing subject", args: []string{"member", "show"}, names: []string{"<MEMBER_ID>"}, usage: true},
		{name: "non-integer subject", args: []string{"fleet", "show", "abc"}, names: []string{"abc"}},
		{name: "value outside the choices", args: []string{"setup", "--coding-agent", "bogus"}, names: []string{"bogus", "claude"}},
		{name: "missing option value", args: []string{"message", "poll", "1", "--database-url"}, names: []string{"--database-url <URL>"}},
		{name: "conflicting bodies", args: []string{"message", "send", "--from-member-id", "1", "--to-member-id", "2", "hi", "--file", "body.txt"}, names: []string{"TEXT", "--file"}, usage: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			database := filepath.Join(t.TempDir(), "absent.db")
			outcome := runCliChild(t, append([]string{"--database-url", "sqlite:///" + database}, tc.args...)...)
			if outcome.code != 2 {
				t.Fatalf("%s: exit %d, want 2: %s", tc.name, outcome.code, outcome.stderr)
			}
			if outcome.stdout != "" {
				t.Fatalf("%s: stdout %q, want empty", tc.name, outcome.stdout)
			}
			lines := strings.Split(outcome.stderr, "\n")
			if !strings.HasPrefix(lines[0], "error: ") {
				t.Fatalf("%s: first stderr line %q does not begin with %q", tc.name, lines[0], "error: ")
			}
			usage := false
			for _, line := range lines[1:] {
				usage = usage || strings.HasPrefix(line, "Usage: ")
			}
			if usage != tc.usage {
				t.Fatalf("%s: a Usage: line follows the error is %t, want %t:\n%s", tc.name, usage, tc.usage, outcome.stderr)
			}
			for _, input := range tc.names {
				if !strings.Contains(outcome.stderr, input) {
					t.Fatalf("%s: the error does not name %q:\n%s", tc.name, input, outcome.stderr)
				}
			}
			if _, err := os.Stat(database); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("%s: a parse error touched the database: %v", tc.name, err)
			}
		})
	}
}

func TestCli_ParseErrorsReportTheFirstLineClapReports(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		args  []string
		first string
	}{
		{name: "root without arguments", args: nil, first: cliRootAbout},
		{name: "root with only a URL", args: []string{"--database-url", "x"}, first: "error: 'cafleet' requires a subcommand but one was not provided"},
		{name: "group with its own URL", args: []string{"member", "--database-url", "x"}, first: "error: 'cafleet member' requires a subcommand but one was not provided"},
		{name: "group under a root URL", args: []string{"--database-url", "x", "member"}, first: "Member lifecycle and pane interaction"},
		{name: "group with a trailing separator", args: []string{"member", "--"}, first: "Member lifecycle and pane interaction"},
		{name: "unknown long flag in place of a value", args: []string{"--database-url", "--json", "message", "poll", "1"}, first: "error: unexpected argument '--json' found"},
		{name: "unknown short flag in place of a value", args: []string{"--database-url", "-x", "fleet"}, first: "error: unexpected argument '-x' found"},
		{name: "subgroup version flag in place of a value", args: []string{"member", "--database-url", "--version"}, first: "error: unexpected argument '--version' found"},
		{name: "known flag in place of a value", args: []string{"--database-url", "--help"}, first: "error: a value is required for '--database-url <URL>' but none was supplied"},
		{name: "known short cluster in place of a value", args: []string{"--database-url", "-hx"}, first: "error: a value is required for '--database-url <URL>' but none was supplied"},
		{name: "separator in place of a value", args: []string{"--database-url", "--", "fleet"}, first: "error: unexpected argument 'fleet' found"},
		{name: "invalid UTF-8 positional", args: []string{"message", "poll", "\xff"}, first: "error: invalid UTF-8 was detected in one or more arguments"},
		{name: "invalid UTF-8 option value", args: []string{"message", "send", "--to-member-id", "\xff", "--from-member-id", "x", "hi"}, first: "error: invalid UTF-8 was detected in one or more arguments"},
		{name: "invalid UTF-8 body before a missing option", args: []string{"message", "send", "\xff"}, first: "error: invalid UTF-8 was detected in one or more arguments"},
		{name: "invalid UTF-8 root URL", args: []string{"--database-url", "\xff", "fleet", "list"}, first: "error: invalid UTF-8 was detected in one or more arguments"},
		{name: "leaf error before an invalid root URL", args: []string{"--database-url", "\xff", "fleet", "list", "--bogus"}, first: "error: unexpected argument '--bogus' found"},
		{name: "extra positional before invalid UTF-8", args: []string{"message", "poll", "\xff", "2"}, first: "error: unexpected argument '2' found"},
		{name: "invalid UTF-8 rendered lossily", args: []string{"message", "send", "--from-member-id", "1", "--to-member-id", "2", "\xff", "\xfe"}, first: "error: unexpected argument '�' found"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			_, exit := cliArgsParseFrom(append([]string{"cafleet"}, tc.args...))
			if exit == nil {
				t.Fatalf("%s: the arguments parse", tc.name)
			}
			if exit.exitCode() != 2 {
				t.Fatalf("%s: exit %d, want 2", tc.name, exit.exitCode())
			}
			if first, _, _ := strings.Cut(exit.text, "\n"); first != tc.first {
				t.Fatalf("%s: first line %q, want %q", tc.name, first, tc.first)
			}
		})
	}
	if _, exit := cliArgsParseFrom([]string{"cafleet", "--database-url", "-", "fleet", "list"}); exit != nil {
		t.Fatalf("a lone dash is the URL value: %s", exit.text)
	}
}

func TestCli_HelpAndVersionPrintToStdoutAndExitZeroWithoutOpeningTheDatabase(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		args    []string
		version bool
	}{
		{name: "root help", args: []string{"--help"}},
		{name: "group help", args: []string{"fleet", "--help"}},
		{name: "leaf help", args: []string{"member", "press", "--help"}},
		{name: "version", args: []string{"--version"}, version: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			database := filepath.Join(t.TempDir(), "absent.db")
			outcome := runCliChild(t, append([]string{"--database-url", "sqlite:///" + database}, tc.args...)...)
			if outcome.code != 0 {
				t.Fatalf("%s: exit %d, want 0: %s", tc.name, outcome.code, outcome.stderr)
			}
			if outcome.stderr != "" {
				t.Fatalf("%s: stderr %q, want empty", tc.name, outcome.stderr)
			}
			switch {
			case tc.version && outcome.stdout != "cafleet "+Version+"\n":
				t.Fatalf("%s: stdout %q, want %q", tc.name, outcome.stdout, "cafleet "+Version+"\n")
			case !tc.version && !strings.Contains(outcome.stdout, "Usage: "):
				t.Fatalf("%s: help has no usage line:\n%s", tc.name, outcome.stdout)
			}
			if _, err := os.Stat(database); !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("%s: help or version touched the database: %v", tc.name, err)
			}
		})
	}
}

func TestCli_CommandGroupHelpStatesEachGroupsPurpose(t *testing.T) {
	t.Parallel()
	help := flattened(helpText(t, "root help"))
	purposes := []struct {
		name    string
		purpose string
	}{
		{name: "event", purpose: "Read a durable runtime event and current state"},
		{name: "input", purpose: "Read / settle a durable input receipt"},
		{name: "permission", purpose: "Inspect / answer the current native permission request"},
	}
	for _, tc := range purposes {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if !strings.Contains(help, tc.name+" "+tc.purpose) {
				t.Fatalf("%s: root help does not describe the group as %q:\n%s", tc.name, tc.purpose, help)
			}
		})
	}
}

const directorOnlyRequestToken = "0000000000000000000000000000000000000000000000000000000000000000"

type directorOnlyCase struct {
	name string
	args []string
}

// directorOnlyCommands is one invocation of each Director-only command; the subject id is at
// index 2 of every argv.
func directorOnlyCommands(subject, caller string) []directorOnlyCase {
	return []directorOnlyCase{
		{name: "member prompt", args: []string{"member", "prompt", subject, "hello", "--from-member-id", caller}},
		{name: "member exec", args: []string{"member", "exec", subject, "printf ok", "--from-member-id", caller}},
		{name: "member press", args: []string{"member", "press", subject, "Enter", "--from-member-id", caller}},
		{name: "permission respond", args: []string{
			"permission", "respond", subject, "--event-id", "1", "--from-member-id", caller,
			"--expected-run-id", "1", "--expected-owner-epoch", "1", "--request-token", directorOnlyRequestToken,
			"--decision", "allow",
		}},
		{name: "input reconcile", args: []string{
			"input", "reconcile", subject, "--disposition", "submitted", "--evidence-ref", "seen", "--from-member-id", caller,
		}},
	}
}

var directorOnlyUnguardedCommands = [][]string{
	{"message", "send", "--from-member-id", "1", "--to-member-id", "2", "hi"},
	{"message", "poll", "1"},
	{"member", "list", "1"},
	{"member", "capture", "1"},
	{"member", "create", "--fleet-id", "1", "--name", "worker", "--description", "work", "prompt"},
	{"permission", "inspect", "7", "--event-id", "1", "--from-member-id", "1"},
	{"input", "show", "7"},
	{"event", "show", "7"},
	{"doctor"},
	{"setup"},
}

func TestCli_DirectorOnlyGuard_DirectorOnlyNamesExactlyTheGuardedCommands(t *testing.T) {
	t.Parallel()
	for _, tc := range directorOnlyCommands("7", "1") {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			guarded, ok := directorOnlyOf(mustParse(t, tc.name, tc.args...))
			if !ok || guarded.name != tc.name {
				t.Fatalf("%s: %q is guarded as %#v (%v)", tc.name, tc.args, guarded, ok)
			}
			if guarded.subject != 7 || guarded.directorMemberID != 1 {
				t.Fatalf("%s: %q guards subject %d for caller %d, want 7 and 1", tc.name, tc.args, guarded.subject, guarded.directorMemberID)
			}
		})
	}
	for _, args := range directorOnlyUnguardedCommands {
		name := strings.Join(args[:min(2, len(args))], " ")
		t.Run("unguarded "+name, func(t *testing.T) {
			t.Parallel()
			if guarded, ok := directorOnlyOf(mustParse(t, name, args...)); ok {
				t.Fatalf("%s: %q is guarded as %#v", name, args, guarded)
			}
		})
	}
}

func TestCli_DirectorOnlyGuard_DirectorOnlyCommandsRequireAPositiveFromMemberId(t *testing.T) {
	t.Parallel()
	for _, tc := range directorOnlyCommands("7", "1") {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			position := slices.Index(tc.args, "--from-member-id")
			missing := slices.Delete(slices.Clone(tc.args), position, position+2)
			if outcome := parseOutcome(t, tc.name+" without a caller", missing...); outcome.exitCode() != 2 {
				t.Fatalf("%s: %q exits %d, want 2: %s", tc.name, missing, outcome.exitCode(), outcome.text)
			}
			zero := slices.Clone(tc.args)
			zero[position+1] = "0"
			if outcome := parseOutcome(t, tc.name+" with a zero caller", zero...); outcome.exitCode() != 2 {
				t.Fatalf("%s: %q exits %d, want 2: %s", tc.name, zero, outcome.exitCode(), outcome.text)
			}
		})
	}
}

func TestCli_OrdinaryCommandsPreserveRefusedWalDatabaseAndSidecarBytes(t *testing.T) {
	t.Parallel()
	const mutation = "PRAGMA application_id=42"
	path := filepath.Join(t.TempDir(), "registry.db")
	selected := settingsForDatabase(path)
	conn := fileConn(t, selected.databaseURL)
	if err := initialize(conn, selected.databaseURL); err != nil {
		t.Fatal(err)
	}
	mustExecBatch(t, conn, "PRAGMA journal_mode=WAL")
	mustExecBatch(t, conn, mutation)
	mustExecBatch(t, conn, "BEGIN; SELECT * FROM cafleet_database")
	before := walSnapshot(t, path)
	for _, args := range [][]string{
		{"fleet", "list", "--json"},
		{"member", "press", "1", "--from-member-id", "1", "Enter"},
		{"permission", "inspect", "1", "--event-id", "1", "--from-member-id", "1"},
	} {
		if err := dispatch(selected, mustParse(t, mutation, args...)); err == nil {
			t.Fatalf("%s: %q succeeds on a refused database", mutation, args)
		}
		for _, file := range before {
			data, err := os.ReadFile(file.path)
			if err != nil || !bytes.Equal(data, file.bytes) {
				t.Fatalf("%s: %q: %s changed: %v", mutation, args, file.path, err)
			}
		}
	}
	mustExecBatch(t, conn, "ROLLBACK")
}

func TestCli_OrdinaryCommandsRequireStorageWithoutCreatingAMissingFile(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "missing.db")
	conn, err := invocationConnection(context.Background(), "sqlite:///"+path, newDeadline())
	if err == nil {
		closeConnection(t, conn)
		t.Fatal("a missing database is refused")
	}
	if err.Error() != "no cafleet database; run 'cafleet setup'" {
		t.Fatalf("invocationConnection = %v", err)
	}
	if _, err := os.Lstat(path); !os.IsNotExist(err) {
		t.Fatalf("the missing database stays absent: %v", err)
	}
}

func TestCli_NativeCommandFilesStripOnlyOneTerminalLineEnding(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "command.txt")
	write := func(source string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(source), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	accepted := []struct {
		source string
		want   string
	}{
		{"printf '界🙂'", "printf '界🙂'"},
		{"echo literal\n", "echo literal"},
		{"echo literal\r\n", "echo literal"},
		{"  echo literal  \n", "  echo literal  "},
	}
	for _, tc := range accepted {
		write(tc.source)
		if got, err := resolveCommandBody(none[string](), some(path)); err != nil || got != tc.want {
			t.Fatalf("%q: resolveCommandBody = %q %v, want %q", tc.source, got, err, tc.want)
		}
	}
	for _, source := range []string{"", "echo x\n\n", "echo x\r"} {
		write(source)
		if got, err := resolveCommandBody(none[string](), some(path)); err == nil {
			t.Fatalf("%q: resolveCommandBody accepted %q", source, got)
		}
	}
	if got, err := resolveCommandBody(some("echo literal"), none[string]()); err != nil || got != "echo literal" {
		t.Fatalf("an inline command = %q %v", got, err)
	}
	if got, err := resolveCommandBody(some("echo literal\n"), none[string]()); err == nil {
		t.Fatalf("an inline command keeps its line ending and is refused, got %q", got)
	}
}

type directorOnlySubjects struct {
	fixture     *brokerRuntimeFixture
	input       int64
	interaction int64
}

func newDirectorOnlySubjects(t *testing.T) directorOnlySubjects {
	t.Helper()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	input := fixture.input(t, owner, "prompt:guard", inputKindPrompt).inputID
	interaction, ok := fixture.open(t, owner).interactionID.get()
	if !ok {
		t.Fatal("the opening event creates an interaction")
	}
	return directorOnlySubjects{fixture: fixture, input: input, interaction: interaction}
}

func (s directorOnlySubjects) commands(caller int64) []directorOnlyCase {
	cases := directorOnlyCommands("0", strconv.FormatInt(caller, 10))
	for index, tc := range cases {
		subject := s.fixture.member
		switch tc.name {
		case "permission respond":
			subject = s.interaction
		case "input reconcile":
			subject = s.input
		}
		args := slices.Clone(tc.args)
		args[2] = strconv.FormatInt(subject, 10)
		cases[index].args = args
	}
	return cases
}

func (s directorOnlySubjects) check(t *testing.T, tc directorOnlyCase) error {
	t.Helper()
	guarded, ok := directorOnlyOf(mustParse(t, tc.name, tc.args...))
	if !ok {
		t.Fatalf("%s: %q is guarded", tc.name, tc.args)
	}
	return requireDirector(s.fixture.conn, guarded)
}

func directorOnlyGuardMessage(name string, caller, fleet int64) string {
	return fmt.Sprintf("'cafleet %s' is a Director-only command and member %d is not the root Director of fleet %d; send the request to your Director.", name, caller, fleet)
}

func TestCli_DirectorOnlyGuard_RequireDirectorAdmitsTheRootDirectorOfEachSubjectsFleet(t *testing.T) {
	t.Parallel()
	subjects := newDirectorOnlySubjects(t)
	for _, tc := range subjects.commands(subjects.fixture.director) {
		if err := subjects.check(t, tc); err != nil {
			t.Fatalf("%s: %v", tc.name, err)
		}
	}
	for _, args := range directorOnlyUnguardedCommands {
		if guarded, ok := directorOnlyOf(mustParse(t, strings.Join(args, " "), args...)); ok {
			t.Fatalf("%q is guarded as %#v", args, guarded)
		}
	}
}

func TestCli_DirectorOnlyGuard_RequireDirectorRefusesAnyOtherCallerNamingTheSubjectsFleet(t *testing.T) {
	t.Parallel()
	subjects := newDirectorOnlySubjects(t)
	otherFleet, otherDirector := createFleetFixture(t, subjects.fixture.conn, "other-fleet")
	if otherFleet == subjects.fixture.fleet {
		t.Fatal("the other fleet is distinct")
	}
	for _, caller := range []int64{subjects.fixture.member, otherDirector, 999_999} {
		for _, tc := range subjects.commands(caller) {
			err := subjects.check(t, tc)
			var failure *cafleetError
			if !errors.As(err, &failure) || failure.exitCode() != 1 || failure.message() != directorOnlyGuardMessage(tc.name, caller, subjects.fixture.fleet) {
				t.Fatalf("%s refuses member %d: %v", tc.name, caller, err)
			}
		}
	}
}

func TestCli_DirectorOnlyGuard_RequireDirectorRefusesTheDirectorOfADeletedFleet(t *testing.T) {
	t.Parallel()
	subjects := newDirectorOnlySubjects(t)
	if _, err := subjects.fixture.conn.execute("UPDATE fleets SET deleted_at='2026-11-07T00:00:00+00:00' WHERE fleet_id=?1", subjects.fixture.fleet); err != nil {
		t.Fatal(err)
	}
	for _, tc := range subjects.commands(subjects.fixture.director) {
		if strings.HasPrefix(tc.name, "member ") {
			continue
		}
		want := directorOnlyGuardMessage(tc.name, subjects.fixture.director, subjects.fixture.fleet)
		if err := subjects.check(t, tc); err == nil || err.Error() != want {
			t.Fatalf("%s: requireDirector = %v, want %q", tc.name, err, want)
		}
	}
}

func TestCli_DirectorOnlyGuard_RequireDirectorLeavesAnUnknownSubjectToTheCommandsOwnNotFoundError(t *testing.T) {
	t.Parallel()
	subjects := newDirectorOnlySubjects(t)
	for _, tc := range directorOnlyCommands("999999", strconv.FormatInt(subjects.fixture.member, 10)) {
		if err := subjects.check(t, tc); err != nil {
			t.Fatalf("%s: %v", tc.name, err)
		}
	}
}

func TestCli_DirectorOnlyGuard_RunArgsRefusesAMemberCallerWithExitOneAndNothingOnStdout(t *testing.T) {
	t.Parallel()
	subjects := newDirectorOnlySubjects(t)
	for _, tc := range subjects.commands(subjects.fixture.member) {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			outcome := runCliChild(t, append([]string{"--database-url", subjects.fixture.url}, tc.args...)...)
			want := "Error: " + directorOnlyGuardMessage(tc.name, subjects.fixture.member, subjects.fixture.fleet) + "\n"
			if outcome.code != 1 || outcome.stderr != want || outcome.stdout != "" {
				t.Fatalf("%s: exit %d stdout %q stderr %q, want exit 1 and %q", tc.name, outcome.code, outcome.stdout, outcome.stderr, want)
			}
		})
	}
}

const (
	isolatedCliDatabaseEnv = "CAFLEET_TEST_ISOLATED_CLI_DATABASE"
	isolatedCliArgsEnv     = "CAFLEET_TEST_ISOLATED_CLI_ARGS"
	isolatedCliTimeout     = 10 * time.Second
)

type isolatedCliOutput struct {
	code   int
	stdout string
	stderr string
}

// isolatedCliWith runs args as the cafleet CLI against s in a re-executed child whose PATH holds
// only the executables (name, `/bin/sh` body) and whose pane calls each executable may append to
// $CAFLEET_TEST_PANE_LOG; it returns the child's outcome and that log.
func isolatedCliWith(t *testing.T, s settings, args []string, executables, environment [][2]string) (isolatedCliOutput, string) {
	t.Helper()
	dir := t.TempDir()
	paneLog := filepath.Join(dir, "pane.log")
	for _, executable := range executables {
		executableStub(t, dir, executable[0], executable[1])
	}
	encodedArgs, err := json.Marshal(args)
	if err != nil {
		t.Fatal(err)
	}
	command := childTestCommand(TestCli_IsolatedCliChild)
	overridden := map[string]bool{"PATH": true, "TMUX": true, "HERDR_ENV": true, "CAFLEET_TEST_PANE_LOG": true, "HOME": true}
	for _, pair := range environment {
		overridden[pair[0]] = true
	}
	inherited := command.Env
	command.Env = nil
	for _, entry := range inherited {
		name, _, _ := strings.Cut(entry, "=")
		if !overridden[name] {
			command.Env = append(command.Env, entry)
		}
	}
	for _, pair := range environment {
		command.Env = append(command.Env, pair[0]+"="+pair[1])
	}
	command.Env = append(command.Env,
		isolatedCliDatabaseEnv+"="+s.databaseURL,
		isolatedCliArgsEnv+"="+string(encodedArgs),
		"CAFLEET_TEST_PANE_LOG="+paneLog,
		"PATH="+dir,
		"HOME="+t.TempDir(),
		"TMUX=cafleet-test,1,0")
	var stdout, stderr bytes.Buffer
	command.Stdout, command.Stderr = &stdout, &stderr
	if err := command.Start(); err != nil {
		t.Fatal(err)
	}
	finished := make(chan error, 1)
	go func() { finished <- command.Wait() }()
	var waited error
	select {
	case waited = <-finished:
	case <-time.After(isolatedCliTimeout):
		if err := command.Process.Kill(); err != nil {
			t.Error(err)
		}
		if waitErr := <-finished; waitErr == nil {
			t.Error("the killed child reports its signal")
		}
		t.Fatalf("the isolated CLI %q did not finish within %v: %s", args, isolatedCliTimeout, stderr.String())
	}
	code := 0
	if waited != nil {
		var exit *exec.ExitError
		if !errors.As(waited, &exit) {
			t.Fatal(waited)
		}
		code = exit.ExitCode()
	}
	calls, err := os.ReadFile(paneLog)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	return isolatedCliOutput{code: code, stdout: stdout.String(), stderr: stderr.String()}, string(calls)
}

// TestCli_IsolatedCliChild is isolatedCliWith's child: it dispatches its configured arguments and
// exits with the command's own status, as the binary's main does.
func TestCli_IsolatedCliChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	path, err := databasePath(os.Getenv(isolatedCliDatabaseEnv))
	if err != nil {
		t.Fatal(err)
	}
	var args []string
	if err := json.Unmarshal([]byte(os.Getenv(isolatedCliArgsEnv)), &args); err != nil {
		t.Fatal(err)
	}
	parsed, exit := parseCli(args...)
	if exit != nil {
		t.Fatalf("the isolated arguments %q parse: %v", args, exit)
	}
	if err := dispatch(settingsForDatabase(path), parsed.command); err != nil {
		failure := cafleetErrorOf(err)
		fmt.Fprintf(os.Stderr, "Error: %s\n", failure.message())
		syscall.Exit(failure.exitCode())
	}
	syscall.Exit(0)
}
