package core

import (
	"bytes"
	"errors"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func randomBytes(buffer *[32]byte) error {
	file, err := os.Open("/dev/urandom")
	if err != nil {
		return err
	}
	_, err = io.ReadFull(file, buffer[:])
	closeFile(file)
	return err
}

func randomHex32() (string, error) {
	var buffer [32]byte
	if err := randomBytes(&buffer); err != nil {
		return "", err
	}
	return digestHex(buffer[:]), nil
}

// findExecutableIn resolves name to the first executable regular file under an absolute entry
// of path, returning the path as found without resolving links.
func findExecutableIn(path string, name string) opt[string] {
	for _, directory := range strings.Split(path, ":") {
		if !strings.HasPrefix(directory, "/") {
			continue
		}
		if candidate := joinPath(directory, name); isExecutableFile(candidate) {
			return some(candidate)
		}
	}
	return none[string]()
}

func isExecutableFile(candidate string) bool {
	info, err := os.Stat(candidate)
	return err == nil && info.Mode().IsRegular() && info.Mode().Perm()&0o111 != 0
}

// defaultExecPath is the search path libc's execvp uses when PATH is unset.
func defaultExecPath() string {
	if runtime.GOOS == "darwin" {
		return "/usr/bin:/bin"
	}
	return "/bin:/usr/bin"
}

// resolveProgram finds program as execvp does (F19): a name with a slash is used as given;
// otherwise every PATH entry is tried in order, an empty or relative entry included, and the
// search fails with EACCES when a candidate was found but not executable, else with ENOENT.
func resolveProgram(program string) (string, error) {
	if strings.Contains(program, "/") {
		return program, nil
	}
	path, set := os.LookupEnv("PATH")
	if !set {
		path = defaultExecPath()
	}
	denied := false
	for _, directory := range strings.Split(path, ":") {
		candidate := joinPath(directory, program)
		if directory == "" {
			candidate = program
		}
		info, err := os.Stat(candidate)
		if err != nil || info.IsDir() {
			continue
		}
		if unix.Access(candidate, unix.X_OK) == nil {
			if !strings.Contains(candidate, "/") {
				candidate = "./" + candidate
			}
			return candidate, nil
		}
		denied = true
	}
	if denied {
		return "", syscall.EACCES
	}
	return "", syscall.ENOENT
}

// command builds the child of argv with argv[0] resolved by resolveProgram and kept as the
// child's own argv[0].
func command(argv []string) (*exec.Cmd, error) {
	path, err := resolveProgram(argv[0])
	if err != nil {
		return nil, err
	}
	cmd := exec.Command(path, argv[1:]...)
	cmd.Err = nil
	cmd.Args[0] = argv[0]
	return cmd, nil
}

// commandOutput runs cmd with null stdin and returns its stdout whatever its exit status.
func commandOutput(cmd *exec.Cmd) ([]byte, error) {
	var stdout bytes.Buffer
	cmd.Stdout = &stdout
	if err := startChild(cmd); err != nil {
		return nil, err
	}
	err := cmd.Wait()
	var exit *exec.ExitError
	if err != nil && !errors.As(err, &exit) {
		return nil, err
	}
	return stdout.Bytes(), nil
}

type systemRunner struct{}

func (systemRunner) binaryExists(name string) bool {
	path, set := os.LookupEnv("PATH")
	if !set {
		return false
	}
	_, found := findExecutableIn(path, name).get()
	return found
}

func (systemRunner) run(argv []string) (string, *runError) {
	return runProcess(argv, none[time.Duration]())
}

func (systemRunner) runFor(argv []string, timeout time.Duration) (string, *runError) {
	return runProcess(argv, some(timeout))
}

// spawnPiped starts argv with null stdin and piped stdout and stderr, whose read ends it
// returns.
func spawnPiped(argv []string, environment []string) (*exec.Cmd, *os.File, *os.File, error) {
	cmd, err := command(argv)
	if err != nil {
		return nil, nil, nil, err
	}
	return startPiped(cmd, environment)
}

func startPiped(cmd *exec.Cmd, environment []string) (*exec.Cmd, *os.File, *os.File, error) {
	if len(environment) > 0 {
		cmd.Env = append(os.Environ(), environment...)
	}
	stdoutRead, stdoutWrite, err := os.Pipe()
	if err != nil {
		return nil, nil, nil, err
	}
	stderrRead, stderrWrite, err := os.Pipe()
	if err != nil {
		closeAll(stdoutRead, stdoutWrite)
		return nil, nil, nil, err
	}
	cmd.Stdout, cmd.Stderr = stdoutWrite, stderrWrite
	err = startChild(cmd)
	closeAll(stdoutWrite, stderrWrite)
	if err != nil {
		closeAll(stdoutRead, stderrRead)
		return nil, nil, nil, err
	}
	return cmd, stdoutRead, stderrRead, nil
}

// retryInterrupted repeats action while it fails with EINTR.
func retryInterrupted[T any](action func() (T, error)) (T, error) {
	for {
		value, err := action()
		if !errors.Is(err, syscall.EINTR) {
			return value, err
		}
	}
}

func closeAll(files ...*os.File) {
	for _, file := range files {
		// A pipe end this process created closes without a reportable error.
		_ = file.Close()
	}
}

func spawnRunError(err error) *runError {
	if errors.Is(err, syscall.ENOENT) {
		return &runError{kind: runErrorBinaryNotFound, detail: ioErrorText(err)}
	}
	return &runError{kind: runErrorNotStarted, detail: ioErrorText(err)}
}

func decode(status syscall.WaitStatus, stdout, stderr []byte) (string, *runError) {
	if status.Exited() && status.ExitStatus() == 0 {
		return lossyUTF8(stdout), nil
	}
	return "", &runError{kind: runErrorFailed, detail: lossyUTF8(stderr)}
}

func runProcess(argv []string, timeout opt[time.Duration]) (string, *runError) {
	started := time.Now()
	limit, timed := timeout.get()
	if timed && limit == 0 {
		return "", runErrorDeadlineExpired()
	}
	cmd, stdout, stderr, err := spawnPiped(argv, nil)
	if err != nil {
		return "", spawnRunError(err)
	}
	if timed {
		return runTimed(cmd, stdout, stderr, max(limit-time.Since(started), 0))
	}
	output, failure := collectUntil(cmd, stdout, stderr, opt[time.Time]{}, unlimitedCapture)
	if failure != nil {
		return "", &runError{kind: runErrorFailed, detail: failure.cause()}
	}
	return decode(output.status, output.stdout, output.stderr)
}

const unlimitedCapture = int(^uint(0) >> 1)

func runTimed(cmd *exec.Cmd, stdout, stderr *os.File, timeout time.Duration) (string, *runError) {
	output, failure := collectUntil(cmd, stdout, stderr, some(time.Now().Add(timeout)), unlimitedCapture)
	if failure == nil {
		return decode(output.status, output.stdout, output.stderr)
	}
	primary := failure.runError()
	for _, diagnostic := range failure.cleanup {
		if primary.kind == runErrorFailed {
			primary.detail += "\n" + diagnostic
		} else {
			// The run already failed; a cleanup diagnostic has no other channel.
			_, _ = io.WriteString(os.Stderr, diagnostic+"\n")
		}
	}
	return "", primary
}

// boundedOutput is a process that exited and closed both output pipes before its deadline.
type boundedOutput struct {
	status syscall.WaitStatus
	stdout []byte
	stderr []byte
}

type boundedFailureKind uint8

const (
	boundedFailureNotStarted boundedFailureKind = iota + 1
	// The deadline passed; exited tells whether the child had already exited and only its
	// output pipes were still open.
	boundedFailureDeadline
	boundedFailureIO
)

// boundedFailure is why a bounded run returned no boundedOutput; cleanup holds the diagnostics
// of a kill or reap that failed after it.
type boundedFailure struct {
	kind    boundedFailureKind
	err     error
	exited  bool
	cleanup []string
}

func (f *boundedFailure) cause() string {
	return ioErrorText(f.err)
}

func (f *boundedFailure) runError() *runError {
	switch f.kind {
	case boundedFailureDeadline:
		return &runError{kind: runErrorTimeout}
	case boundedFailureNotStarted, boundedFailureIO:
		return &runError{kind: runErrorFailed, detail: f.cause()}
	default:
		panic("invalid boundedFailureKind")
	}
}

// runUntilExit runs program with null stdin and environment added until deadline, keeping at
// most capacity bytes of each stream; the child is killed and reaped when the run fails.
func runUntilExit(program string, args []string, environment []string, deadline time.Time, capacity int) (boundedOutput, *boundedFailure) {
	cmd := exec.Command(program, args...)
	cmd.Err = nil
	cmd, stdout, stderr, err := startPiped(cmd, environment)
	if err != nil {
		return boundedOutput{}, &boundedFailure{kind: boundedFailureNotStarted, err: err}
	}
	return collectUntil(cmd, stdout, stderr, some(deadline), capacity)
}

// capturedStream drains one pipe from spawn until it closes, keeping at most capacity bytes.
type capturedStream struct {
	data []byte
	err  error
	done chan struct{}
}

func captureStream(pipe *os.File, capacity int) *capturedStream {
	stream := &capturedStream{done: make(chan struct{})}
	go func() {
		defer close(stream.done)
		buffer := make([]byte, 8192)
		for {
			count, err := pipe.Read(buffer)
			kept := min(count, capacity-len(stream.data))
			stream.data = append(stream.data, buffer[:kept]...)
			if errors.Is(err, io.EOF) {
				return
			}
			if err != nil {
				stream.err = err
				return
			}
		}
	}()
	return stream
}

// collectUntil collects the child's status and both streams, returning as soon as the child
// has exited and both pipes have closed. Without a deadline it waits for both. On failure the
// read ends are closed and the direct child is killed and reaped.
func collectUntil(cmd *exec.Cmd, stdoutPipe, stderrPipe *os.File, deadline opt[time.Time], capacity int) (boundedOutput, *boundedFailure) {
	stdout := captureStream(stdoutPipe, capacity)
	stderr := captureStream(stderrPipe, capacity)
	var status syscall.WaitStatus
	var waitErr error
	exited := make(chan struct{})
	go func() {
		defer close(exited)
		// Wait reaps the direct child as soon as it exits; this runner pins no process group.
		state, err := retryInterrupted(cmd.Process.Wait)
		waitErr = err
		if err == nil {
			status = state.Sys().(syscall.WaitStatus)
		}
	}()
	var expired <-chan time.Time
	if until, bounded := deadline.get(); bounded {
		timer := time.NewTimer(time.Until(until))
		defer timer.Stop()
		expired = timer.C
	}
	pending := []<-chan struct{}{exited, stdout.done, stderr.done}
	var failure *boundedFailure
	for len(pending) > 0 && failure == nil {
		select {
		case <-pending[0]:
			pending = pending[1:]
		case <-expired:
			failure = &boundedFailure{kind: boundedFailureDeadline, exited: channelClosed(exited)}
		}
	}
	if failure == nil {
		failure = streamFailure(waitErr, stdout, stderr)
	}
	if failure == nil {
		return boundedOutput{status: status, stdout: stdout.data, stderr: stderr.data}, nil
	}
	closeAll(stdoutPipe, stderrPipe)
	if !channelClosed(exited) {
		_, err := retryInterrupted(func() (struct{}, error) { return struct{}{}, cmd.Process.Kill() })
		if err != nil && !errors.Is(err, os.ErrProcessDone) {
			failure.cleanup = append(failure.cleanup, "cleanup failed for child "+pidText(cmd)+" kill: "+ioErrorText(err))
		}
	}
	<-exited
	if waitErr != nil {
		failure.cleanup = append(failure.cleanup, "cleanup failed for child "+pidText(cmd)+" wait: "+ioErrorText(waitErr))
	}
	<-stdout.done
	<-stderr.done
	return boundedOutput{}, failure
}

func streamFailure(waitErr error, streams ...*capturedStream) *boundedFailure {
	if waitErr != nil {
		return &boundedFailure{kind: boundedFailureIO, err: waitErr}
	}
	for _, stream := range streams {
		if stream.err != nil {
			return &boundedFailure{kind: boundedFailureIO, err: stream.err}
		}
	}
	return nil
}

func channelClosed(done <-chan struct{}) bool {
	select {
	case <-done:
		return true
	default:
		return false
	}
}

func pidText(cmd *exec.Cmd) string {
	return strconv.Itoa(cmd.Process.Pid)
}

type systemProbe struct{}

func (systemProbe) resolveBinary(name string) opt[string] {
	path, set := os.LookupEnv("PATH")
	if !set {
		return none[string]()
	}
	return findExecutableIn(path, name)
}

func (systemProbe) homeDir() (string, error) {
	return homeDir()
}

func (systemProbe) envVar(name string) (opt[string], error) {
	return processEnv(name)
}

// readStdin reads stdin to EOF as raw bytes.
func readStdin() ([]byte, error) {
	return io.ReadAll(os.Stdin)
}
