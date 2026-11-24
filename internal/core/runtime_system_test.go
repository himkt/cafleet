package core

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"
)

// executableStub writes an executable `/bin/sh` script named name holding body under dir.
func executableStub(t *testing.T, dir, name, body string) string {
	t.Helper()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"+body+"\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestRuntimeSystem_NativeExecutableResolutionUsesTheFirstAbsolutePathEntryAsFound(t *testing.T) {
	t.Parallel()
	inCwd, err := os.CreateTemp(".", ".path-test-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.Remove(inCwd.Name()); err != nil {
			t.Error(err)
		}
	})
	if err := inCwd.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(inCwd.Name(), 0o755); err != nil {
		t.Fatal(err)
	}
	name := filepath.Base(inCwd.Name())
	relative, err := os.MkdirTemp(".", ".path-test-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.RemoveAll(relative); err != nil {
			t.Error(err)
		}
	})
	executableStub(t, relative, name, "")
	relativeEntry := filepath.Base(relative)
	absolute := t.TempDir()
	dirs := map[string]string{}
	for _, entry := range []string{"unusable", "directory", "good", "linked", "dangling"} {
		dirs[entry] = filepath.Join(absolute, entry)
		if err := os.Mkdir(dirs[entry], 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(dirs["unusable"], name), []byte("#!/bin/sh\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(dirs["directory"], name), 0o755); err != nil {
		t.Fatal(err)
	}
	good := executableStub(t, dirs["good"], name, "")
	if err := os.Symlink(good, filepath.Join(dirs["linked"], name)); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(filepath.Join(absolute, "absent"), filepath.Join(dirs["dangling"], name)); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name    string
		entries []string
		want    string
	}{
		{name: "relative and empty entries are skipped even when they hold the binary", entries: []string{relativeEntry, "", dirs["good"]}, want: good},
		{name: "the first absolute match wins", entries: []string{dirs["good"], relativeEntry, ""}, want: good},
		{name: "only relative and empty entries", entries: []string{relativeEntry, ""}},
		{name: "dangling links, non-executable files and directories are skipped", entries: []string{dirs["dangling"], dirs["unusable"], dirs["directory"], dirs["good"]}, want: good},
		{name: "a symbolic link is used as found", entries: []string{dirs["linked"], dirs["good"]}, want: filepath.Join(dirs["linked"], name)},
		{name: "no entries", entries: []string{}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got, found := findExecutableIn(strings.Join(tc.entries, ":"), name).get()
			if found != (tc.want != "") || got != tc.want {
				t.Fatalf("%s: found %q (%v), want %q", tc.name, got, found, tc.want)
			}
		})
	}
}

const relativePathStub = "cafleet-relative-path-stub"

// TestRuntimeSystem_RelativePathEntryChild runs the stub by its bare name under the PATH and
// working directory its parent supplies.
func TestRuntimeSystem_RelativePathEntryChild(t *testing.T) {
	t.Parallel()
	skipUnlessChildTest(t)
	runner := systemRunner{}
	output, err := runner.run([]string{relativePathStub})
	fmt.Printf("exists=%v output=%q err=%v\n", runner.binaryExists(relativePathStub), output, err)
}

func TestRuntimeSystem_ARunnerCommandFoundOnlyThroughARelativePathEntryRuns(t *testing.T) {
	t.Parallel()
	work := t.TempDir()
	if err := os.Mkdir(filepath.Join(work, "rel"), 0o755); err != nil {
		t.Fatal(err)
	}
	executableStub(t, filepath.Join(work, "rel"), relativePathStub, "printf relative-ok")
	child := childTestCommand(TestRuntimeSystem_RelativePathEntryChild)
	child.Dir = work
	child.Env = append(child.Env, "PATH=rel", "HOME="+t.TempDir())
	output, err := child.CombinedOutput()
	if err != nil {
		t.Fatalf("child failed: %v\n%s", err, output)
	}
	want := `exists=false output="relative-ok" err=<nil>`
	if !strings.Contains(string(output), want) {
		t.Fatalf("child reported\n%s\nwant %s: the run resolves relative PATH entries while binaryExists does not", output, want)
	}
}

// processFixture runs shell scripts that first record their own PID in dir/pid.
type processFixture struct {
	dir string
}

func newProcessFixture(t *testing.T) processFixture {
	t.Helper()
	return processFixture{dir: t.TempDir()}
}

func (f processFixture) run(script string, timeout time.Duration) (string, *runError) {
	argv := []string{"/bin/sh", "-c", "printf '%s' \"$$\" > \"$1/pid\"\n" + script, "cafleet-process-test", f.dir}
	if timeout == 0 {
		return systemRunner{}.run(argv)
	}
	return systemRunner{}.runFor(argv, timeout)
}

func (f processFixture) write(t *testing.T, name string, data []byte) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(f.dir, name), data, 0o600); err != nil {
		t.Fatal(err)
	}
}

func (f processFixture) assertDirectChildReaped(t *testing.T, name string, runErr *runError) {
	t.Helper()
	recorded, err := os.ReadFile(filepath.Join(f.dir, "pid"))
	if err != nil {
		t.Fatalf("%s: child PID was not recorded: %v; runner result: %v", name, err, runErr)
	}
	pid, err := strconv.ParseInt(string(recorded), 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	assertReaped(t, name+": the runner reaps its direct child before returning", pid)
}

func megabyte() []byte {
	payload := make([]byte, 1024*1024)
	for i := range payload {
		payload[i] = byte('a' + i%26)
	}
	return payload
}

func assertRunErrorKind(t *testing.T, name string, err *runError, kind runErrorKind) *runError {
	t.Helper()
	if err == nil || err.kind != kind {
		t.Fatalf("%s: run error %#v, want kind %d", name, err, kind)
	}
	return err
}

const megabyteStreams = "cat \"$1/payload\" & writer=$!; cat \"$1/payload\" >&2; wait \"$writer\""

// assertMegabyteStreams runs script against a megabyte payload and checks that the stream it
// reports (stdout, or stderr of a failed run) holds the whole payload and the child was reaped.
func assertMegabyteStreams(t *testing.T, name, script string, timeout time.Duration, failed bool) {
	t.Helper()
	fixture := newProcessFixture(t)
	payload := megabyte()
	fixture.write(t, "payload", payload)
	output, err := fixture.run(script, timeout)
	fixture.assertDirectChildReaped(t, name, err)
	if failed {
		output = assertRunErrorKind(t, name, err, runErrorFailed).detail
	} else if err != nil {
		t.Fatalf("%s: %#v", name, err)
	}
	if output != string(payload) {
		t.Fatalf("%s: the stream holds %d bytes, want the %d-byte payload", name, len(output), len(payload))
	}
}

func TestRuntimeSystem_TimedRunnerDrainsSimultaneousMegabyteStreamsOnSuccess(t *testing.T) {
	t.Parallel()
	assertMegabyteStreams(t, "timed success", megabyteStreams, 10*time.Second, false)
}

func TestRuntimeSystem_TimedRunnerDrainsSimultaneousMegabyteStreamsOnFailure(t *testing.T) {
	t.Parallel()
	assertMegabyteStreams(t, "timed failure", megabyteStreams+"; exit 9", 10*time.Second, true)
}

func TestRuntimeSystem_UntimedRunnerPreservesLargeOutputAndReapsChild(t *testing.T) {
	t.Parallel()
	assertMegabyteStreams(t, "untimed success", megabyteStreams, 0, false)
}

// assertTimesOutWithinFiveSeconds runs script with a two-second timeout and checks that it times
// out, reaped, well inside five seconds.
func assertTimesOutWithinFiveSeconds(t *testing.T, name, script string) {
	t.Helper()
	fixture := newProcessFixture(t)
	started := time.Now()
	_, err := fixture.run(script, 2*time.Second)
	elapsed := time.Since(started)
	fixture.assertDirectChildReaped(t, name, err)
	assertRunErrorKind(t, name, err, runErrorTimeout)
	if elapsed >= 5*time.Second {
		t.Fatalf("%s: elapsed %v", name, elapsed)
	}
}

func TestRuntimeSystem_TimedRunnerKillsAndReapsASleepingDirectChildWithinFiveSeconds(t *testing.T) {
	t.Parallel()
	assertTimesOutWithinFiveSeconds(t, "sleeping direct child", "exec sleep 30")
}

func TestRuntimeSystem_TimedRunnerChecksDeadlineWhileBothStreamsKeepProducing(t *testing.T) {
	t.Parallel()
	assertTimesOutWithinFiveSeconds(t, "flooding streams", "while :; do printf 'stdout flood\\n'; printf 'stderr flood\\n' >&2; done")
}

func TestRuntimeSystem_TimedRunnerAppliesDeadlineAfterChildExitWithInheritedPipesOpen(t *testing.T) {
	t.Parallel()
	assertTimesOutWithinFiveSeconds(t, "inherited pipes", "sleep 6 & exit 0")
}

func TestRuntimeSystem_TimedRunnerWaitsForChildExitEvenAfterBothPipesClose(t *testing.T) {
	t.Parallel()
	assertTimesOutWithinFiveSeconds(t, "closed pipes", "exec 1>&- 2>&-; exec sleep 30")
}

// assertLossyDecoding runs script over an ill-formed UTF-8 payload and checks the decoded stream.
func assertLossyDecoding(t *testing.T, name, script string, failed bool) {
	t.Helper()
	fixture := newProcessFixture(t)
	fixture.write(t, "payload", []byte("a\xffb\xc3"))
	output, err := fixture.run(script, 10*time.Second)
	fixture.assertDirectChildReaped(t, name, err)
	if failed {
		output = assertRunErrorKind(t, name, err, runErrorFailed).detail
	} else if err != nil {
		t.Fatalf("%s: %#v", name, err)
	}
	if want := "a�b�"; output != want {
		t.Fatalf("%s: decoded %q, want %q", name, output, want)
	}
}

func TestRuntimeSystem_TimedRunnerDecodesSuccessfulStdoutAsLossyUtf8(t *testing.T) {
	t.Parallel()
	assertLossyDecoding(t, "successful stdout", "cat \"$1/payload\"", false)
}

func TestRuntimeSystem_TimedRunnerDecodesFailedStderrAsLossyUtf8(t *testing.T) {
	t.Parallel()
	assertLossyDecoding(t, "failed stderr", "cat \"$1/payload\" >&2; exit 1", true)
}

func TestRuntimeSystem_RunnerPreservesBinaryNotFoundClassification(t *testing.T) {
	t.Parallel()
	missing := filepath.Join(t.TempDir(), "missing-executable")
	_, err := systemRunner{}.runFor([]string{missing}, time.Second)
	assertRunErrorKind(t, "missing executable", err, runErrorBinaryNotFound)
}

func TestRuntimeSystem_CleanupRetryRetriesInterruptedAndPreservesOtherErrors(t *testing.T) {
	t.Parallel()
	attempts := 0
	result, err := retryInterrupted(func() (string, error) {
		attempts++
		if attempts <= 2 {
			return "", syscall.EINTR
		}
		return "reaped", nil
	})
	if err != nil || result != "reaped" || attempts != 3 {
		t.Fatalf("interrupted retry = %q %v after %d attempts, want reaped after 3", result, err, attempts)
	}
	attempts = 0
	original := errors.New("original wait failure")
	_, err = retryInterrupted(func() (struct{}, error) {
		attempts++
		return struct{}{}, original
	})
	if err != original || attempts != 1 {
		t.Fatalf("other error = %v after %d attempts, want the original failure after 1", err, attempts)
	}
}

// boundedRun retries a stub start that races the stub's own write (ETXTBSY).
func boundedRun(t *testing.T, program string, environment []string, bound time.Duration, limit int) (boundedOutput, *boundedFailure) {
	t.Helper()
	for range 50 {
		output, failure := runUntilExit(program, nil, environment, time.Now().Add(bound), limit)
		if failure != nil && failure.kind == boundedFailureNotStarted && errors.Is(failure.err, syscall.ETXTBSY) {
			time.Sleep(20 * time.Millisecond)
			continue
		}
		return output, failure
	}
	t.Fatal("an executable stub stayed busy")
	return boundedOutput{}, nil
}

func TestRuntimeSystem_BoundedRun_AFinishedChildReturnsItsStatusStreamsAndEnvironmentWithoutAPollingTail(t *testing.T) {
	t.Parallel()
	program := executableStub(t, t.TempDir(), "bounded", "printf 'out=%s\\n' \"$BOUNDED_VALUE\"; printf 'err\\n' >&2; exit 3")
	started := time.Now()
	output, err := boundedRun(t, program, []string{"BOUNDED_VALUE=/srv/value"}, 5*time.Second, 4096)
	if err != nil {
		t.Fatalf("the bounded run finishes: %v", err)
	}
	if elapsed := time.Since(started); elapsed >= time.Second {
		t.Fatalf("elapsed %v", elapsed)
	}
	if !output.status.Exited() || output.status.ExitStatus() != 3 {
		t.Fatalf("status %v, want exit 3", output.status)
	}
	if !bytes.Equal(output.stdout, []byte("out=/srv/value\n")) || !bytes.Equal(output.stderr, []byte("err\n")) {
		t.Fatalf("streams %q %q", output.stdout, output.stderr)
	}
}

func TestRuntimeSystem_BoundedRun_EachStreamKeepsAtMostItsCapWhileTheChildRunsToExit(t *testing.T) {
	t.Parallel()
	program := executableStub(t, t.TempDir(), "bounded", "head -c 200000 /dev/zero; head -c 200000 /dev/zero >&2")
	output, err := boundedRun(t, program, nil, 5*time.Second, 10)
	if err != nil {
		t.Fatalf("the bounded run finishes: %v", err)
	}
	if !output.status.Exited() || output.status.ExitStatus() != 0 {
		t.Fatalf("status %v, want success", output.status)
	}
	if !bytes.Equal(output.stdout, make([]byte, 10)) || !bytes.Equal(output.stderr, make([]byte, 10)) {
		t.Fatalf("streams kept %d and %d bytes, want 10 zero bytes each", len(output.stdout), len(output.stderr))
	}
}

func TestRuntimeSystem_BoundedRun_TheDeadlineKillsARunningChildAndReportsWhetherItHadExited(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		body   string
		exited bool
	}{
		{name: "running child", body: "exec /bin/sleep 30"},
		{name: "exited child holding its pipes", body: "/bin/sleep 30 & exit 0", exited: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			program := executableStub(t, t.TempDir(), "bounded", tc.body)
			started := time.Now()
			_, failure := boundedRun(t, program, nil, 300*time.Millisecond, 4096)
			if failure == nil || failure.kind != boundedFailureDeadline || failure.exited != tc.exited {
				t.Fatalf("%s: failure %#v, want a deadline with exited=%v", tc.name, failure, tc.exited)
			}
			if elapsed := time.Since(started); elapsed >= 2*time.Second {
				t.Fatalf("%s: elapsed %v", tc.name, elapsed)
			}
		})
	}
}

func TestRuntimeSystem_BoundedRun_AMissingProgramIsNotStarted(t *testing.T) {
	t.Parallel()
	_, failure := boundedRun(t, filepath.Join(t.TempDir(), "absent"), nil, time.Second, 4096)
	if failure == nil || failure.kind != boundedFailureNotStarted {
		t.Fatalf("failure %#v, want not started", failure)
	}
}
