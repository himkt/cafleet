package core

import (
	"bytes"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"

	"github.com/himkt/cafleet"
	"golang.org/x/sys/unix"
)

// descriptorProbe reports whether descriptor 3 reached the shell.
const descriptorProbe = `if { true >&3; } 2>/dev/null; then echo fd3=open; else echo fd3=closed; fi`

//nolint:paralleltest // changes process-wide signal handling and descriptor flags inside its own child process
func TestMain_ProcessStartupChild(t *testing.T) {
	skipUnlessChildTest(t)
	flags, err := unix.FcntlInt(3, unix.F_GETFD, 0)
	if err != nil {
		t.Fatalf("the parent passes descriptor 3: %v", err)
	}
	if flags&unix.FD_CLOEXEC != 0 {
		t.Fatal("the inherited descriptor 3 starts without FD_CLOEXEC")
	}
	if err := processStartup(); err != nil {
		t.Fatal(err)
	}
	if flags, err = unix.FcntlInt(3, unix.F_GETFD, 0); err != nil || flags&unix.FD_CLOEXEC == 0 {
		t.Fatalf("startup marks descriptor 3 FD_CLOEXEC: flags=%#x err=%v", flags, err)
	}
	output, err := exec.Command("/bin/sh", "-c", descriptorProbe).Output()
	if err != nil {
		t.Fatal(err)
	}
	fmt.Print(string(output))
	sleeper, err := startedSleeper(plainStart)
	if err != nil {
		t.Fatal(err)
	}
	ignored, readErr := processSignalSet(sleeper.Process.Pid, "SigIgn", "sigignore")
	if err := errors.Join(readErr, endSleeper(sleeper)); err != nil {
		t.Fatal(err)
	}
	fmt.Printf("ignored=%x\n", uint64(ignored))
}

func TestMain_ProcessStartupKeepsInheritedDescriptorsFromChildrenAndLeavesSigpipeAtDefault(t *testing.T) {
	t.Parallel()
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := errors.Join(reader.Close(), writer.Close()); err != nil {
			t.Error(err)
		}
	}()
	child := childTestCommand(TestMain_ProcessStartupChild, "-test.v")
	child.ExtraFiles = []*os.File{reader}
	output, err := child.CombinedOutput()
	if err != nil {
		t.Fatalf("startup child failed: %v\n%s", err, output)
	}
	text := string(output)
	if !strings.Contains(text, "fd3=closed\n") {
		t.Fatalf("an inherited descriptor above 2 reached a child (F1):\n%s", text)
	}
	_, after, found := strings.Cut(text, "ignored=")
	if !found {
		t.Fatalf("the probe printed no ignored set:\n%s", text)
	}
	line, _, _ := strings.Cut(after, "\n")
	ignored, err := strconv.ParseUint(strings.TrimSpace(line), 16, 64)
	if err != nil {
		t.Fatalf("ignored set %q: %v", line, err)
	}
	if signalSet(ignored).contains(syscall.SIGPIPE) {
		t.Fatalf("a child starts with SIGPIPE ignored (F3): %#x", ignored)
	}
}

// TestMain_MainChild runs Main with the arguments after `--` and exits with its code.
func TestMain_MainChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	os.Exit(Main(append([]string{"cafleet"}, flag.Args()...), cafleet.Assets))
}

func TestMain_ABrokenStdoutPipeEndsOutputQuietlyWithTheCommandsOwnResult(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		args []string
	}{
		{name: "help", args: []string{"--help"}},
		{name: "version", args: []string{"--version"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			reader, writer, err := os.Pipe()
			if err != nil {
				t.Fatal(err)
			}
			if err := reader.Close(); err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := writer.Close(); err != nil {
					t.Error(err)
				}
			}()
			database := databaseURLFor(filepath.Join(t.TempDir(), "absent.db"))
			child := childTestCommand(TestMain_MainChild, append([]string{"--", "--database-url", database}, tc.args...)...)
			child.Env = append(child.Env, "HOME="+t.TempDir())
			var stderr bytes.Buffer
			child.Stdout = writer
			child.Stderr = &stderr
			if err := child.Run(); err != nil {
				t.Fatalf("%s: a closed stdout reader ends the command with %v, want its own exit 0: %s", tc.name, err, stderr.String())
			}
			if stderr.Len() != 0 {
				t.Fatalf("%s: stderr %q, want nothing", tc.name, stderr.String())
			}
		})
	}
}

var updateGolden = flag.Bool("update-golden", false, "rewrite each golden test's generated block from the current output")

const (
	childTestEnv  = "CAFLEET_TEST_CHILD"
	generatedEnd  = "// END GENERATED"
	generatedOpen = "// BEGIN GENERATED:"
)

// testName is the name of the package-level test function test; closures and methods panic,
// because `go test -run` cannot select them.
func testName(test func(*testing.T)) string {
	function := runtime.FuncForPC(reflect.ValueOf(test).Pointer())
	if function == nil {
		panic("the test function has no runtime symbol")
	}
	full := function.Name()
	parts := strings.Split(full[strings.LastIndexByte(full, '/')+1:], ".")
	if len(parts) != 2 || !strings.HasPrefix(parts[1], "Test") {
		panic(fmt.Sprintf("%s is not a package-level test function", full))
	}
	return parts[1]
}

// childTestCommand re-executes this test binary to run only test, with the guard that
// skipUnlessChildTest checks set to its name.
func childTestCommand(test func(*testing.T), args ...string) *exec.Cmd {
	name := testName(test)
	command := exec.Command(os.Args[0], append([]string{"-test.run", "^" + name + "$", "-test.count=1"}, args...)...)
	command.Env = append(os.Environ(), childTestEnv+"="+name)
	return command
}

func skipUnlessChildTest(t *testing.T) {
	t.Helper()
	if os.Getenv(childTestEnv) != t.Name() {
		t.Skip("runs only as a re-executed child test")
	}
}

// generatedBegin is testName's BEGIN GENERATED marker, which names the command that regenerates
// its block: the internal/core test binary alone, because the -update-golden flag exists only there.
func generatedBegin(testName string) string {
	return generatedOpen + " go test ./internal/core -run '^" + testName + "$' -update-golden"
}

// rewriteGeneratedBlock replaces the lines between testName's BEGIN GENERATED marker and the
// next END GENERATED line with body, leaving every other byte of source unchanged.
func rewriteGeneratedBlock(source []byte, testName string, body string) ([]byte, error) {
	if body != "" && !strings.HasSuffix(body, "\n") {
		return nil, fmt.Errorf("the generated block of %s must end with a newline", testName)
	}
	begin := generatedBegin(testName)
	lines := bytes.SplitAfter(source, []byte("\n"))
	start, end := -1, -1
	for index, line := range lines {
		text := strings.TrimSpace(string(line))
		open := start >= 0 && end < 0
		switch {
		case text == begin && start >= 0:
			return nil, fmt.Errorf("%s has more than one BEGIN GENERATED marker", testName)
		case text == begin:
			start = index
		case open && strings.HasPrefix(text, generatedOpen):
			return nil, fmt.Errorf("the generated block of %s contains another BEGIN GENERATED marker", testName)
		case open && text == generatedEnd:
			end = index
		}
	}
	if start < 0 {
		return nil, fmt.Errorf("%s has no BEGIN GENERATED marker", testName)
	}
	if end < 0 {
		return nil, fmt.Errorf("the generated block of %s has no END GENERATED marker", testName)
	}
	rewritten := bytes.Join(lines[:start+1], nil)
	rewritten = append(rewritten, body...)
	return append(rewritten, bytes.Join(lines[end:], nil)...), nil
}

func rewriteGeneratedFile(path string, testName string, body string) error {
	if !strings.HasSuffix(path, "_test.go") {
		return fmt.Errorf("%s is not a _test.go file", path)
	}
	info, err := os.Stat(path)
	if err != nil {
		return err
	}
	source, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	rewritten, err := rewriteGeneratedBlock(source, testName, body)
	if err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	return os.WriteFile(path, rewritten, info.Mode().Perm())
}

// updateGoldenBlock rewrites the generated block of the running top-level golden test in path.
func updateGoldenBlock(t *testing.T, path string, body string) {
	t.Helper()
	if strings.Contains(t.Name(), "/") {
		t.Fatalf("%s: only a top-level test owns a generated block", t.Name())
	}
	if err := rewriteGeneratedFile(path, t.Name(), body); err != nil {
		t.Fatal(err)
	}
}

func TestMain_UpdateGoldenFlagDefaultsToComparing(t *testing.T) {
	t.Parallel()
	registered := flag.Lookup("update-golden")
	if registered == nil {
		t.Fatal("the -update-golden flag is registered")
	}
	if registered.DefValue != "false" {
		t.Fatalf("-update-golden defaults to %q", registered.DefValue)
	}
	if got := registered.Value.(flag.Getter).Get(); got != *updateGolden {
		t.Fatalf("-update-golden reads %v, updateGolden holds %v", got, *updateGolden)
	}
}

func TestMain_TestNameIsThePackageLevelTestFunctionName(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		test func(*testing.T)
		want string
	}{
		{name: "this test", test: TestMain_TestNameIsThePackageLevelTestFunctionName, want: "TestMain_TestNameIsThePackageLevelTestFunctionName"},
		{name: "another test", test: TestMain_ReexecutedChildProbe, want: "TestMain_ReexecutedChildProbe"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := testName(tc.test); got != tc.want {
				t.Fatalf("%s: testName = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestMain_TestNameRejectsFunctionsGoTestCannotSelect(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name string
		test func(*testing.T)
	}{
		{name: "closure", test: func(*testing.T) {}},
		{name: "nil function", test: nil},
		{name: "non-test helper", test: skipUnlessChildTest},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			defer func() {
				if recover() == nil {
					t.Fatalf("%s: testName panics", tc.name)
				}
			}()
			got := testName(tc.test)
			t.Fatalf("%s: testName returned %q", tc.name, got)
		})
	}
}

func TestMain_ChildTestCommandRunsOnlyTheNamedTestWithItsGuard(t *testing.T) {
	t.Parallel()
	command := childTestCommand(TestMain_ReexecutedChildProbe, "-test.v")
	wantArgs := []string{os.Args[0], "-test.run", "^TestMain_ReexecutedChildProbe$", "-test.count=1", "-test.v"}
	if !slices.Equal(command.Args, wantArgs) {
		t.Fatalf("child argv = %q, want %q", command.Args, wantArgs)
	}
	guard := childTestEnv + "=TestMain_ReexecutedChildProbe"
	if command.Env[len(command.Env)-1] != guard {
		t.Fatalf("child environment ends with %q, want %q", command.Env[len(command.Env)-1], guard)
	}
}

const reexecutedChildMarker = "re-executed child test ran"

func TestMain_ReexecutedChildProbe(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	fmt.Println(reexecutedChildMarker)
}

func TestMain_ReexecutedChildRunsOnlyUnderItsGuard(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		guard   string
		runs    bool
		skipped bool
	}{
		{name: "guard names the child", guard: "TestMain_ReexecutedChildProbe", runs: true},
		{name: "guard empty", guard: "", skipped: true},
		{name: "guard names another test", guard: "TestMain_ChildTestCommandRunsOnlyTheNamedTestWithItsGuard", skipped: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			command := childTestCommand(TestMain_ReexecutedChildProbe, "-test.v")
			command.Env = append(command.Env, childTestEnv+"="+tc.guard)
			output, err := command.CombinedOutput()
			if err != nil {
				t.Fatalf("%s: child failed: %v\n%s", tc.name, err, output)
			}
			text := string(output)
			if got := strings.Contains(text, reexecutedChildMarker); got != tc.runs {
				t.Fatalf("%s: child body ran = %v, want %v\n%s", tc.name, got, tc.runs, text)
			}
			if got := strings.Contains(text, "--- SKIP: TestMain_ReexecutedChildProbe"); got != tc.skipped {
				t.Fatalf("%s: child skipped = %v, want %v\n%s", tc.name, got, tc.skipped, text)
			}
		})
	}
}

func generatedSource(blocks ...string) []byte {
	return []byte(strings.Join(blocks, ""))
}

func TestMain_GeneratedBeginNamesTheCoreOnlyRegenerateCommand(t *testing.T) {
	t.Parallel()
	const want = "// BEGIN GENERATED: go test ./internal/core -run '^TestOwn$' -update-golden"
	if got := generatedBegin("TestOwn"); got != want {
		t.Fatalf("generatedBegin = %q, want %q", got, want)
	}
}

func TestMain_RewriteGeneratedBlockReplacesOnlyTheNamedBlock(t *testing.T) {
	t.Parallel()
	own := generatedBegin("TestOwn")
	other := generatedBegin("TestOther")
	cases := []struct {
		name   string
		source []byte
		body   string
		want   []byte
	}{
		{
			name:   "package-level block",
			source: generatedSource("package core\n\n", own, "\n", "var old = 1\n", generatedEnd, "\n\nfunc tail() {}\n"),
			body:   "var fresh = 2\nvar more = 3\n",
			want:   generatedSource("package core\n\n", own, "\n", "var fresh = 2\nvar more = 3\n", generatedEnd, "\n\nfunc tail() {}\n"),
		},
		{
			name:   "indented block keeps its marker lines",
			source: generatedSource("func f() {\n\t", own, "\n\told()\n\t", generatedEnd, "\n}\n"),
			body:   "\tfresh()\n",
			want:   generatedSource("func f() {\n\t", own, "\n\tfresh()\n\t", generatedEnd, "\n}\n"),
		},
		{
			name:   "empty body",
			source: generatedSource(own, "\nold\n", generatedEnd, "\n"),
			body:   "",
			want:   generatedSource(own, "\n", generatedEnd, "\n"),
		},
		{
			name:   "another test's block stays unchanged",
			source: generatedSource(other, "\nkept\n", generatedEnd, "\n", own, "\nold\n", generatedEnd, "\n"),
			body:   "fresh\n",
			want:   generatedSource(other, "\nkept\n", generatedEnd, "\n", own, "\nfresh\n", generatedEnd, "\n"),
		},
		{
			name:   "end marker without a trailing newline",
			source: generatedSource(own, "\nold\n", generatedEnd),
			body:   "fresh\n",
			want:   generatedSource(own, "\nfresh\n", generatedEnd),
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := rewriteGeneratedBlock(tc.source, "TestOwn", tc.body)
			if err != nil {
				t.Fatalf("%s: %v", tc.name, err)
			}
			if !bytes.Equal(got, tc.want) {
				t.Fatalf("%s: rewritten source\n%s\nwant\n%s", tc.name, got, tc.want)
			}
		})
	}
}

func TestMain_RewriteGeneratedBlockRefusesAnAmbiguousOrMalformedBlock(t *testing.T) {
	t.Parallel()
	own := generatedBegin("TestOwn")
	other := generatedBegin("TestOther")
	cases := []struct {
		name   string
		source []byte
		body   string
		cause  string
	}{
		{
			name:   "no begin marker",
			source: generatedSource(other, "\nold\n", generatedEnd, "\n"),
			body:   "fresh\n",
			cause:  "TestOwn has no BEGIN GENERATED marker",
		},
		{
			name:   "marker for a longer test name only",
			source: generatedSource(generatedBegin("TestOwnLonger"), "\nold\n", generatedEnd, "\n"),
			body:   "fresh\n",
			cause:  "TestOwn has no BEGIN GENERATED marker",
		},
		{
			name:   "no end marker",
			source: generatedSource(own, "\nold\n"),
			body:   "fresh\n",
			cause:  "the generated block of TestOwn has no END GENERATED marker",
		},
		{
			name:   "two begin markers",
			source: generatedSource(own, "\nold\n", generatedEnd, "\n", own, "\nold\n", generatedEnd, "\n"),
			body:   "fresh\n",
			cause:  "TestOwn has more than one BEGIN GENERATED marker",
		},
		{
			name:   "nested begin marker",
			source: generatedSource(own, "\n", other, "\nold\n", generatedEnd, "\n"),
			body:   "fresh\n",
			cause:  "the generated block of TestOwn contains another BEGIN GENERATED marker",
		},
		{
			name:   "body without a final newline",
			source: generatedSource(own, "\nold\n", generatedEnd, "\n"),
			body:   "fresh",
			cause:  "the generated block of TestOwn must end with a newline",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, err := rewriteGeneratedBlock(tc.source, "TestOwn", tc.body)
			if err == nil {
				t.Fatalf("%s: rewrite succeeded with\n%s", tc.name, got)
			}
			if err.Error() != tc.cause {
				t.Fatalf("%s: error %q, want %q", tc.name, err, tc.cause)
			}
		})
	}
}

func TestMain_RewriteGeneratedFileRewritesATestFileInPlace(t *testing.T) {
	t.Parallel()
	own := generatedBegin("TestOwn")
	original := generatedSource("package core\n\n", own, "\nvar old = 1\n", generatedEnd, "\n")
	cases := []struct {
		name  string
		file  string
		want  []byte
		cause string
	}{
		{
			name: "test file",
			file: "owner_test.go",
			want: generatedSource("package core\n\n", own, "\nvar fresh = 2\n", generatedEnd, "\n"),
		},
		{
			name:  "production file",
			file:  "owner.go",
			want:  original,
			cause: " is not a _test.go file",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			path := filepath.Join(t.TempDir(), tc.file)
			if err := os.WriteFile(path, original, 0o640); err != nil {
				t.Fatal(err)
			}
			err := rewriteGeneratedFile(path, "TestOwn", "var fresh = 2\n")
			switch {
			case tc.cause == "" && err != nil:
				t.Fatalf("%s: %v", tc.name, err)
			case tc.cause != "" && (err == nil || err.Error() != path+tc.cause):
				t.Fatalf("%s: error %v, want %q", tc.name, err, path+tc.cause)
			}
			got, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(got, tc.want) {
				t.Fatalf("%s: file holds\n%s\nwant\n%s", tc.name, got, tc.want)
			}
			info, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			if info.Mode().Perm() != 0o640 {
				t.Fatalf("%s: mode %v, want 0640", tc.name, info.Mode().Perm())
			}
		})
	}
}

func TestMain_RewriteGeneratedFileNamesTheFileOfAMalformedBlock(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "owner_test.go")
	original := generatedSource("package core\n\n", generatedBegin("TestOwn"), "\nvar old = 1\n")
	if err := os.WriteFile(path, original, 0o600); err != nil {
		t.Fatal(err)
	}
	err := rewriteGeneratedFile(path, "TestOwn", "var fresh = 2\n")
	want := path + ": the generated block of TestOwn has no END GENERATED marker"
	if err == nil || err.Error() != want {
		t.Fatalf("error %v, want %q", err, want)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, original) {
		t.Fatalf("a refused rewrite left\n%s", got)
	}
}

func TestMain_UpdateGoldenBlockRewritesTheRunningTestsBlock(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "owner_test.go")
	own := generatedBegin(t.Name())
	other := generatedBegin("TestOther")
	source := generatedSource(other, "\nkept\n", generatedEnd, "\n", own, "\nold\n", generatedEnd, "\n")
	if err := os.WriteFile(path, source, 0o600); err != nil {
		t.Fatal(err)
	}
	updateGoldenBlock(t, path, "fresh\n")
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	want := generatedSource(other, "\nkept\n", generatedEnd, "\n", own, "\nfresh\n", generatedEnd, "\n")
	if !bytes.Equal(got, want) {
		t.Fatalf("file holds\n%s\nwant\n%s", got, want)
	}
}
