package core

import (
	"cmp"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"maps"
	"os"
	"os/signal"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/creack/pty"
	"golang.org/x/sys/unix"
)

// processIdentity names one process generation: its PID and a start identity that changes when
// the PID is reused.
type processIdentity struct {
	pid           int64
	startIdentity string
}

type processIdentityWire struct {
	PID           int64  `json:"pid"`
	StartIdentity string `json:"start_identity"`
}

func (*processIdentityWire) ignoresUnknownFields() {}

func (p processIdentity) MarshalJSONTo(encoder *jsontext.Encoder) error {
	return json.MarshalEncode(encoder, processIdentityWire{PID: p.pid, StartIdentity: p.startIdentity})
}

func (p *processIdentity) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decoder.ReadValue()
	if err != nil {
		return err
	}
	var wire processIdentityWire
	if err := decodeStrict(value, &wire); err != nil {
		return err
	}
	*p = processIdentity{pid: wire.PID, startIdentity: wire.StartIdentity}
	return nil
}

func processIdentityCurrent() (processIdentity, error) {
	identity, present, err := processIdentityRead(int64(syscall.Getpid()))
	if err != nil {
		return processIdentity{}, err
	}
	if !present {
		return processIdentity{}, runtimeError("current process identity unavailable")
	}
	return identity, nil
}

func (p processIdentity) matches() (bool, error) {
	current, present, err := processIdentityRead(p.pid)
	if err != nil {
		return false, err
	}
	return present && current == p, nil
}

// isAlive holds for this generation while it runs; an unreaped zombie keeps its identity but is
// not alive.
func (p processIdentity) isAlive() (bool, error) {
	matches, err := p.matches()
	if err != nil || !matches {
		return false, err
	}
	gone, err := stopped(p.pid)
	return !gone, err
}

// tryFromIntText is Rust's TryFromIntError display, the cause of a PID outside the platform's
// pid_t range.
const tryFromIntText = "out of range integral type conversion attempted"

func pidGroup(pid int64) (int32, error) {
	if pid < int64(minInt32) || pid > int64(maxInt32) {
		return 0, ioError(tryFromIntText)
	}
	return int32(pid), nil
}

const minInt32 = -maxInt32 - 1

type processObservations interface {
	members(group int32) ([]int64, error)
	identity(pid int64) (processIdentity, bool, error)
	stopped(pid int64) (bool, error)
}

type nativeProcessObservations struct{}

func (nativeProcessObservations) members(group int32) ([]int64, error) {
	return groupMembers(group)
}

func (nativeProcessObservations) identity(pid int64) (processIdentity, bool, error) {
	return processIdentityRead(pid)
}

func (nativeProcessObservations) stopped(pid int64) (bool, error) {
	return stopped(pid)
}

// signalPID sends sig to pid; a process that is already gone counts as signalled.
func signalPID(pid int64, sig syscall.Signal) error {
	target, err := pidGroup(pid)
	if err != nil {
		return err
	}
	if err := syscall.Kill(int(target), sig); err != nil && !errors.Is(err, syscall.ESRCH) {
		return ioError(errnoText(err))
	}
	return nil
}

// liveGroupMembers lists the members of group that have not exited; an unreaped zombie member
// counts as exited.
func liveGroupMembers(group int32) ([]int64, error) {
	members, err := groupMembers(group)
	if err != nil {
		return nil, err
	}
	var live []int64
	for _, pid := range members {
		gone, err := stopped(pid)
		if err != nil {
			return nil, err
		}
		if !gone {
			live = append(live, pid)
		}
	}
	return live, nil
}

// recoverDescendants ends the recorded child's process group when the recorded child generation
// still anchors it, escalating from SIGTERM to SIGKILL.
func recoverDescendants(run runRecord, deadlines shutdownDeadlines, c clock) (shutdownOutcome, error) {
	outcome := shutdownOutcomeComplete(none[childExit]())
	recorded, err := run.childProcess()
	if err != nil {
		return shutdownOutcome{}, err
	}
	child, set := recorded.get()
	if !set {
		return outcome, nil
	}
	group, err := pidGroup(child.pid)
	if err != nil {
		return shutdownOutcome{}, err
	}
	alive, err := child.isAlive()
	if err != nil {
		return shutdownOutcome{}, err
	}
	if !alive {
		live, err := liveGroupMembers(group)
		if err != nil {
			return shutdownOutcome{}, err
		}
		if len(live) > 0 {
			outcome.fail("descendant ownership cannot be anchored to the recorded child generation")
		}
		return outcome, nil
	}
	owned := map[int64]processIdentity{child.pid: child}
	inOrder := func() []processIdentity {
		processes := make([]processIdentity, 0, len(owned))
		for _, pid := range slices.Sorted(maps.Keys(owned)) {
			processes = append(processes, owned[pid])
		}
		return processes
	}
	anyProcess := func(test func(processIdentity) (bool, error)) (bool, error) {
		for _, process := range inOrder() {
			found, err := test(process)
			if err != nil || found {
				return found, err
			}
		}
		return false, nil
	}
	for _, sig := range []syscall.Signal{syscall.SIGTERM, syscall.SIGKILL} {
		anchored, err := anyProcess(processIdentity.matches)
		if err != nil {
			return shutdownOutcome{}, err
		}
		if !anchored {
			live, err := liveGroupMembers(group)
			if err != nil {
				return shutdownOutcome{}, err
			}
			if len(live) > 0 {
				outcome.fail("descendant ownership lost its recorded process generation")
			}
			break
		}
		members, err := groupMembers(group)
		if err != nil {
			return shutdownOutcome{}, err
		}
		for _, pid := range members {
			process, present, err := processIdentityRead(pid)
			if err != nil {
				return shutdownOutcome{}, err
			}
			if !present {
				continue
			}
			if previous, known := owned[pid]; known {
				if err := ensure(previous == process, "recovery descendant PID generation changed"); err != nil {
					return shutdownOutcome{}, err
				}
			} else {
				owned[pid] = process
			}
		}
		for _, process := range inOrder() {
			alive, err := process.isAlive()
			if err != nil {
				return shutdownOutcome{}, err
			}
			if alive {
				if err := signalPID(process.pid, sig); err != nil {
					return shutdownOutcome{}, err
				}
			}
		}
		until := deadlines.storageDeadline()
		if sig == syscall.SIGTERM {
			until = deadlines.childDeadline()
		}
		for c.now().Before(until) {
			alive, err := anyProcess(processIdentity.isAlive)
			if err != nil {
				return shutdownOutcome{}, err
			}
			if !alive {
				break
			}
			napUntil(c, until)
		}
	}
	for _, process := range inOrder() {
		alive, err := process.isAlive()
		if err != nil {
			return shutdownOutcome{}, err
		}
		if alive {
			outcome.fail("verified descendants remain after the original cleanup deadline")
			break
		}
	}
	return outcome, nil
}

// ptySession owns the native child, its process group and its shutdown clock. child is the
// PID of the direct child until it is reaped; owned pins every process generation of the
// native's group the session has observed.
type ptySession struct {
	clock             clock
	master            *os.File
	slave             *os.File
	io                opt[int]
	child             opt[int]
	native            opt[processIdentity]
	exit              opt[childExit]
	owned             map[int64]processIdentity
	groupClosed       bool
	shutdownDeadlines opt[shutdownDeadlines]
}

// ptySessionPrepare opens a PTY of size whose master is read and written through a
// non-blocking duplicate.
func ptySessionPrepare(size terminalSize, c clock) (*ptySession, error) {
	if err := size.validate(); err != nil {
		return nil, err
	}
	master, slave, err := pty.Open()
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	session := &ptySession{clock: c, master: master, slave: slave, owned: map[int64]processIdentity{}}
	if err := setWindowSize(master, size); err != nil {
		closeFile(master)
		closeFile(slave)
		return nil, err
	}
	duplicate, err := unix.FcntlInt(master.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err == nil {
		err = unix.SetNonblock(duplicate, true)
	}
	if err != nil {
		closeFile(master)
		closeFile(slave)
		return nil, ioError(ioErrorText(err))
	}
	session.io = some(duplicate)
	return session, nil
}

func setWindowSize(master *os.File, size terminalSize) error {
	if err := unix.IoctlSetWinsize(int(master.Fd()), unix.TIOCSWINSZ, &unix.Winsize{Row: size.rows, Col: size.cols}); err != nil {
		return ioError(errnoText(err))
	}
	return nil
}

// ptyResetSignals start at their default disposition in a PTY child (F2): with a Go handler
// installed for each, the forked child resets it to the default even when cafleet inherited
// it ignored.
var ptyResetSignals = []os.Signal{syscall.SIGCHLD, syscall.SIGHUP, syscall.SIGINT, syscall.SIGQUIT, syscall.SIGTERM, syscall.SIGALRM}

// spawn starts spec's argv once as a session leader with the PTY slave as its controlling
// terminal, then closes the parent's slave so every byte the child writes reaches the master
// (F23).
func (s *ptySession) spawn(spec spawnSpec) (processIdentity, error) {
	_, identified := s.native.get()
	if err := ensure(!identified && !s.hasChild(), "native child spawn is single-use"); err != nil {
		return processIdentity{}, err
	}
	if err := spec.validate(); err != nil {
		return processIdentity{}, err
	}
	if s.slave == nil {
		return processIdentity{}, runtimeError("prepared PTY slave required")
	}
	environment := ptyEnvironment(spec.environment)
	program, found := searchPath(spec.argv[0], spec.cwd, environment)
	if !found {
		return processIdentity{}, runtimeError("native child spawn failed")
	}
	pid, err := forkPTYChild(program, spec, environment, s.slave)
	if err != nil {
		return processIdentity{}, runtimeError("native child spawn failed")
	}
	s.child = some(pid)
	closeFile(s.slave)
	s.slave = nil
	native, present, err := processIdentityRead(int64(pid))
	if err != nil {
		return processIdentity{}, err
	}
	if !present {
		return processIdentity{}, runtimeError("native child start identity unavailable")
	}
	s.native = some(native)
	s.owned[native.pid] = native
	if err := s.refreshOwned(); err != nil {
		return processIdentity{}, err
	}
	return native, nil
}

// forkPTYChild forks program on a locked thread whose blocked set is empty, so the child
// starts with an empty mask (F2), and starts no goroutine that could reap it (F4).
func forkPTYChild(program string, spec spawnSpec, environment map[string]string, slave *os.File) (int, error) {
	defaults := make(chan os.Signal, len(ptyResetSignals))
	signal.Notify(defaults, ptyResetSignals...)
	defer signal.Stop(defaults)
	env := make([]string, 0, len(environment))
	for _, name := range slices.Sorted(maps.Keys(environment)) {
		env = append(env, name+"="+environment[name])
	}
	attributes := &syscall.ProcAttr{
		Dir:   spec.cwd,
		Env:   env,
		Files: []uintptr{slave.Fd(), slave.Fd(), slave.Fd()},
		Sys:   &syscall.SysProcAttr{Setsid: true, Setctty: true, Ctty: 0},
	}
	type forked struct {
		pid int
		err error
	}
	result := make(chan forked)
	go func() {
		// The goroutine ends still locked, so the runtime discards the thread and its mask.
		runtime.LockOSThread()
		if err := threadSetSignalMask(0); err != nil {
			result <- forked{err: err}
			return
		}
		pid, err := syscall.ForkExec(program, spec.argv, attributes)
		result <- forked{pid: pid, err: err}
	}()
	outcome := <-result
	return outcome.pid, outcome.err
}

// ptyEnvironment is the child's environment as portable-pty builds it: the inherited
// environment with SHELL defaulted to the user's shell, then the spec's overrides.
func ptyEnvironment(overrides [][2]string) map[string]string {
	environment := map[string]string{}
	for _, entry := range os.Environ() {
		name, value, _ := strings.Cut(entry, "=")
		environment[name] = value
	}
	for _, entry := range overrides {
		environment[entry[0]] = entry[1]
	}
	if _, set := environment["SHELL"]; !set {
		environment["SHELL"] = userShell()
	}
	return environment
}

// userShell is the passwd database shell of the current user when it is executable, else
// /bin/sh.
func userShell() string {
	passwd, err := os.ReadFile("/etc/passwd")
	if err != nil {
		return "/bin/sh"
	}
	uid := strconv.Itoa(os.Getuid())
	for _, line := range strings.Split(string(passwd), "\n") {
		fields := strings.Split(line, ":")
		if len(fields) == 7 && fields[2] == uid {
			if unix.Access(fields[6], unix.X_OK) == nil {
				return fields[6]
			}
			break
		}
	}
	return "/bin/sh"
}

// searchPath resolves the native executable as portable-pty does: an absolute path must be an
// executable non-directory; a `./` or `../` path is taken relative to cwd; any other relative
// name is searched in the child environment's PATH entries, each relative to cwd.
func searchPath(executable, cwd string, environment map[string]string) (string, bool) {
	runnable := func(candidate string) bool {
		info, err := os.Stat(candidate)
		return err == nil && !info.IsDir() && unix.Access(candidate, unix.X_OK) == nil
	}
	switch {
	case strings.HasPrefix(executable, "/"):
		return executable, runnable(executable)
	case executable == "." || executable == ".." || strings.HasPrefix(executable, "./") || strings.HasPrefix(executable, "../"):
		candidate := joinPath(cwd, executable)
		return candidate, runnable(candidate)
	}
	path, set := environment["PATH"]
	if !set {
		return "", false
	}
	for _, entry := range strings.Split(path, ":") {
		if candidate := joinPath(joinPath(cwd, entry), executable); runnable(candidate) {
			return candidate, true
		}
	}
	return "", false
}

// tryRead reads available master output without blocking; EIO and a zero-byte read both end
// the stream (F22).
func (s *ptySession) tryRead(buffer []byte) (int, error) {
	fd, open := s.io.get()
	if !open {
		return 0, nil
	}
	count, err := unix.Read(fd, buffer)
	if errors.Is(err, syscall.EIO) {
		return 0, nil
	}
	return max(count, 0), err
}

func (s *ptySession) tryWrite(buffer []byte) (int, error) {
	fd, open := s.io.get()
	if !open {
		return 0, syscall.EPIPE
	}
	count, err := unix.Write(fd, buffer)
	return max(count, 0), err
}

func (s *ptySession) resize(size terminalSize) error {
	if err := size.validate(); err != nil {
		return err
	}
	if s.master == nil {
		return runtimeError("PTY is closed")
	}
	return setWindowSize(s.master, size)
}

func (s *ptySession) close() error {
	if fd, open := s.io.get(); open {
		// The duplicate is this session's own descriptor; its close reports nothing to act on.
		_ = unix.Close(fd)
		s.io = none[int]()
	}
	for _, file := range []*os.File{s.master, s.slave} {
		if file != nil {
			closeFile(file)
		}
	}
	s.master, s.slave = nil, nil
	return nil
}

// shutdown terminates the owned group within deadlines: SIGTERM, then drain and wait until the
// child deadline, SIGKILL, close, and wait until the storage deadline, recording every failure
// as a diagnostic.
func (s *ptySession) shutdown(deadlines shutdownDeadlines) shutdownOutcome {
	deadlines = s.beginShutdown(deadlines)
	var diagnostics []string
	note(&diagnostics, struct{}{}, s.signalVerified(syscall.SIGTERM))
	buffer := make([]byte, 16384)
	for s.clock.now().Before(deadlines.childDeadline()) {
		_, _, waitErr := s.tryWait()
		if _, ok := note(&diagnostics, struct{}{}, waitErr); !ok {
			break
		}
		alive, aliveErr := s.survivors()
		if alive, ok := note(&diagnostics, alive, aliveErr); !ok || !alive {
			break
		}
		if _, err := s.tryRead(buffer); err != nil && !errors.Is(err, syscall.EAGAIN) {
			diagnostics = append(diagnostics, ioErrorText(err))
			break
		}
		s.clock.sleep(min(5*time.Millisecond, max(deadlines.childDeadline().Sub(s.clock.now()), 0)))
	}
	note(&diagnostics, struct{}{}, s.signalVerified(syscall.SIGKILL))
	note(&diagnostics, struct{}{}, s.close())
	for s.clock.now().Before(deadlines.storageDeadline()) {
		_, exited, waitErr := s.tryWait()
		alive, aliveErr := s.survivors()
		if failure := cmp.Or(waitErr, aliveErr); failure != nil {
			diagnostics = append(diagnostics, cafleetErrorOf(failure).message())
			break
		}
		_, identified := s.native.get()
		if !alive && (exited || !identified) {
			break
		}
		napUntil(s.clock, deadlines.storageDeadline())
	}
	alive, aliveErr := s.survivors()
	if alive, ok := note(&diagnostics, alive, aliveErr); ok && alive {
		diagnostics = append(diagnostics, "owned descendants remain unconfirmed")
	}
	_, identified := s.native.get()
	_, exited := s.exit.get()
	if s.hasChild() && !identified {
		diagnostics = append(diagnostics, "native start identity unavailable; descendant cleanup remains unverified")
	}
	if s.hasChild() && !exited {
		diagnostics = append(diagnostics, "native child reaping remains unconfirmed")
	}
	return newShutdownOutcome(s.exit, diagnostics)
}

// finish shuts down a session whose child is unreaped or whose PTY is still open, reporting
// each cleanup diagnostic on stderr.
func (s *ptySession) finish() {
	_, exited := s.exit.get()
	if !s.hasChild() || (exited && s.master == nil) {
		return
	}
	outcome := s.shutdown(shutdownDeadlinesStartingNow(s.clock))
	for _, diagnostic := range outcome.diagnostics {
		fmt.Fprintln(os.Stderr, diagnostic)
	}
}

func (s *ptySession) hasChild() bool {
	_, set := s.child.get()
	return set
}

func (s *ptySession) ownedInOrder() []processIdentity {
	processes := make([]processIdentity, 0, len(s.owned))
	for _, pid := range slices.Sorted(maps.Keys(s.owned)) {
		processes = append(processes, s.owned[pid])
	}
	return processes
}

func (s *ptySession) tryWait() (childExit, bool, error) {
	return s.tryWaitWith(nativeProcessObservations{})
}

// tryWaitWith pins the process group only once the leader is a zombie: until it is reaped, the
// zombie keeps its PID and therefore the group ID, so the scan still sees every live member.
func (s *ptySession) tryWaitWith(observed processObservations) (childExit, bool, error) {
	if exit, exited := s.exit.get(); exited {
		return exit, true, nil
	}
	pid, hasChild := s.child.get()
	if !hasChild {
		return nil, false, nil
	}
	if native, identified := s.native.get(); identified {
		gone, err := observed.stopped(native.pid)
		if err != nil {
			return nil, false, err
		}
		if !gone {
			return nil, false, nil
		}
	}
	if err := s.refreshOwnedWith(observed); err != nil {
		return nil, false, err
	}
	var status syscall.WaitStatus
	reaped, err := syscall.Wait4(pid, &status, syscall.WNOHANG, nil)
	if err != nil {
		return nil, false, ioError(ioErrorText(err))
	}
	if reaped == 0 {
		return nil, false, nil
	}
	var exit childExit
	switch {
	case status.Signaled():
		exit = childExitSignal{signal: waitStatusByte(int(status.Signal()))}
	case status.Exited():
		exit = childExitCode{code: waitStatusByte(status.ExitStatus())}
	default:
		return nil, false, runtimeError("native exit lacks code or signal")
	}
	s.exit = some(exit)
	s.beginShutdown(shutdownDeadlinesStartingNow(s.clock))
	return exit, true, nil
}

// waitStatusByte is an exit code or signal number from a wait status, which holds one byte.
func waitStatusByte(value int) int32 {
	if value < 0 || value > 0xff {
		panic("a wait status carries a one-byte exit code or signal")
	}
	return int32(value)
}

func (s *ptySession) refreshOwned() error {
	return s.refreshOwnedWith(nativeProcessObservations{})
}

func (s *ptySession) refreshOwnedWith(observed processObservations) error {
	native, identified := s.native.get()
	if !identified || s.groupClosed {
		return nil
	}
	group, err := pidGroup(native.pid)
	if err != nil {
		return err
	}
	members, err := observed.members(group)
	if err != nil {
		return err
	}
	if len(members) == 0 {
		s.groupClosed = true
		return nil
	}
	var active []processIdentity
	for _, pid := range members {
		process, present, err := observed.identity(pid)
		if err != nil {
			return err
		}
		if !present {
			continue
		}
		gone, err := observed.stopped(pid)
		if err != nil {
			return err
		}
		if !gone {
			active = append(active, process)
		}
	}
	if len(active) == 0 {
		s.groupClosed = true
		return nil
	}
	_, exited := s.exit.get()
	anchored := s.hasChild() && !exited
	for _, identity := range s.ownedInOrder() {
		current, present, err := observed.identity(identity.pid)
		if err != nil {
			return err
		}
		anchored = anchored || (present && current == identity)
	}
	if !anchored {
		remaining, err := observed.members(group)
		if err != nil {
			return err
		}
		if len(remaining) == 0 {
			s.groupClosed = true
			return nil
		}
	}
	if err := ensure(anchored, "owned process group lost its pinned generation"); err != nil {
		return err
	}
	for _, process := range active {
		if prior, pinned := s.owned[process.pid]; pinned {
			if err := ensure(prior == process, "owned PID was replaced by another generation"); err != nil {
				return err
			}
		} else {
			s.owned[process.pid] = process
		}
	}
	return nil
}

func (s *ptySession) signalVerified(sig syscall.Signal) error {
	_, identified := s.native.get()
	_, exited := s.exit.get()
	if pid, hasChild := s.child.get(); !identified && !exited && hasChild {
		if err := signalPID(int64(pid), sig); err != nil {
			return err
		}
	}
	if err := s.refreshOwned(); err != nil {
		return err
	}
	for _, process := range s.ownedInOrder() {
		gone, err := stopped(process.pid)
		if err != nil {
			return err
		}
		if gone {
			continue
		}
		current, present, err := processIdentityRead(process.pid)
		if err != nil {
			return err
		}
		if !present {
			continue
		}
		if err := ensure(current == process, "process generation changed before termination"); err != nil {
			return err
		}
		gone, err = stopped(process.pid)
		if err != nil {
			return err
		}
		if gone {
			continue
		}
		if err := signalPID(process.pid, sig); err != nil {
			return err
		}
	}
	return nil
}

func (s *ptySession) survivors() (bool, error) {
	return s.survivorsWith(nativeProcessObservations{})
}

func (s *ptySession) survivorsWith(observed processObservations) (bool, error) {
	if err := s.refreshOwnedWith(observed); err != nil {
		return false, err
	}
	for _, process := range s.ownedInOrder() {
		gone, err := observed.stopped(process.pid)
		if err != nil {
			return false, err
		}
		if gone {
			continue
		}
		current, present, err := observed.identity(process.pid)
		if err != nil {
			return false, err
		}
		if present {
			if err := ensure(current == process, "owned PID generation changed during cleanup"); err != nil {
				return false, err
			}
			return true, nil
		}
	}
	return s.unpinnedGroupMember(observed)
}

// unpinnedGroupMember reports a live member of the native's process group that the ownership
// refresh never pinned: it still belongs to this run, so cleanup must not report it as gone.
func (s *ptySession) unpinnedGroupMember(observed processObservations) (bool, error) {
	native, identified := s.native.get()
	if !identified {
		return false, nil
	}
	group, err := pidGroup(native.pid)
	if err != nil {
		return false, err
	}
	members, err := observed.members(group)
	if err != nil {
		return false, err
	}
	for _, pid := range members {
		_, present, err := observed.identity(pid)
		if err != nil {
			return false, err
		}
		if !present {
			continue
		}
		gone, err := observed.stopped(pid)
		if err != nil {
			return false, err
		}
		if !gone {
			return true, nil
		}
	}
	return false, nil
}

// beginShutdown keeps the earliest shutdown clock the session has begun.
func (s *ptySession) beginShutdown(deadlines shutdownDeadlines) shutdownDeadlines {
	first := deadlines
	if prior, begun := s.shutdownDeadlines.get(); begun && !prior.childDeadline().After(deadlines.childDeadline()) {
		first = prior
	}
	s.shutdownDeadlines = some(first)
	return first
}

type processEntry struct {
	parent int64
	argv   []string
}

type processTable interface {
	ownPID() int64
	// entry reports false when the process no longer exists.
	entry(pid int64) (processEntry, bool, error)
	// nestedPIDNamespace reports whether this process runs in a PID namespace nested below the
	// initial one.
	nestedPIDNamespace() (bool, error)
}

type systemProcessTable struct{}

func (systemProcessTable) ownPID() int64 {
	return int64(syscall.Getpid())
}

func parseStatParent(pid int64, stat []byte) (int64, error) {
	text := lossyUTF8(stat)
	if end := strings.LastIndexByte(text, ')'); end >= 0 {
		if fields := strings.Fields(text[end+1:]); len(fields) > 1 {
			if parent, err := parseI64(fields[1]); err == nil {
				return parent, nil
			}
		}
	}
	return 0, appErrorf("malformed /proc/%d/stat", pid)
}

func parseCmdline(cmdline []byte) []string {
	parts := strings.Split(string(cmdline), "\x00")
	argv := make([]string, len(parts))
	for i, part := range parts {
		argv[i] = lossyUTF8([]byte(part))
	}
	if argv[len(argv)-1] == "" {
		argv = argv[:len(argv)-1]
	}
	return argv
}

func parseNspidNested(status string) (bool, error) {
	for _, line := range lines(status) {
		if !strings.HasPrefix(line, "NSpid:") {
			continue
		}
		fields := strings.Fields(line[len("NSpid:"):])
		for _, field := range fields {
			if _, err := parseI64(field); err != nil {
				return false, appErrorf("malformed /proc/self/status NSpid line: %s", line)
			}
		}
		return len(fields) > 1, nil
	}
	return false, nil
}

func parsePsOutput(pid int64, output string) (processEntry, bool, error) {
	for _, line := range lines(output) {
		if strings.TrimSpace(line) == "" {
			continue
		}
		fields := strings.Fields(line)
		parent, err := parseI64(fields[0])
		if err != nil {
			return processEntry{}, false, appErrorf("ps printed no parent for %d: %s", pid, line)
		}
		if len(fields) == 1 {
			return processEntry{}, false, appErrorf("ps printed no command for %d: %s", pid, line)
		}
		return processEntry{parent: parent, argv: fields[1:]}, true, nil
	}
	return processEntry{}, false, nil
}

func procPath(pid int64, file string) string {
	return fmt.Sprintf("/proc/%d/%s", pid, file)
}
