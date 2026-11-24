package core

import (
	"errors"
	"math/bits"
	"os"
	"os/exec"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func nowStamp() string {
	return formatUTC(time.Now())
}

// formatUTC emits the canonical storage form: fixed-width 6-digit microseconds and the literal
// +00:00 offset, so rows sort lexicographically.
func formatUTC(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05.000000") + "+00:00"
}

// parseLenient parses a stored canonical timestamp into its UTC instant.
func parseLenient(value string) (time.Time, bool) {
	parsed, ok := parseRFC3339(value)
	if !ok {
		return time.Time{}, false
	}
	return parsed.UTC(), true
}

var deferredTerminationSignals = []os.Signal{syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP}

// terminationDeferral is the process-wide state behind every open deferTermination span; spans
// nest, and only the outermost begin and end change signal handling.
var terminationDeferral struct {
	mu       sync.Mutex
	depth    int
	captured chan os.Signal
}

// deferTermination captures SIGINT, SIGTERM and SIGHUP from its begin until its close; the first
// signal captured meanwhile takes its default action when the outermost span closes.
type deferTermination struct {
	closed bool
}

func deferTerminationBegin() (*deferTermination, error) {
	terminationDeferral.mu.Lock()
	defer terminationDeferral.mu.Unlock()
	if terminationDeferral.depth == 0 {
		captured := make(chan os.Signal, len(deferredTerminationSignals))
		for _, sig := range deferredTerminationSignals {
			if !signal.Ignored(sig) {
				signal.Notify(captured, sig)
			}
		}
		terminationDeferral.captured = captured
	}
	terminationDeferral.depth++
	return &deferTermination{}, nil
}

// start starts cmd as a child of the span. SPEC §6.5 requires in-span children to inherit the
// blocked SIGINT/SIGTERM/SIGHUP set; children started here do not yet, until the masked spawner
// (F5) forks them from a locked, masked OS thread.
func (d *deferTermination) start(cmd *exec.Cmd) error {
	if d.closed {
		panic("a closed termination-deferral span starts no child")
	}
	return cmd.Start()
}

func (d *deferTermination) close() {
	if d.closed {
		return
	}
	d.closed = true
	terminationDeferral.mu.Lock()
	terminationDeferral.depth--
	if terminationDeferral.depth > 0 {
		terminationDeferral.mu.Unlock()
		return
	}
	captured := terminationDeferral.captured
	terminationDeferral.captured = nil
	signal.Stop(captured)
	var first os.Signal
	select {
	case first = <-captured:
	default:
	}
	terminationDeferral.mu.Unlock()
	if first != nil {
		raiseDefaultAction(first.(syscall.Signal))
	}
}

// raiseDefaultAction re-delivers sig to this process, whose handling has returned to the default
// termination, and waits for that termination.
func raiseDefaultAction(sig syscall.Signal) {
	if err := syscall.Kill(syscall.Getpid(), sig); err != nil {
		panic("re-raising a deferred termination signal to this process failed: " + err.Error())
	}
	for {
		time.Sleep(time.Second)
	}
}

type deadline struct {
	at time.Time
}

func (d deadline) instant() time.Time {
	return d.at
}

func newDeadline() deadline {
	return deadlineUntil(time.Now().Add(5 * time.Second))
}

func deadlineUntil(instant time.Time) deadline {
	return deadline{at: instant}
}

func (d deadline) remaining() (time.Duration, error) {
	return d.remainingAt(time.Now())
}

func (d deadline) remainingAt(now time.Time) (time.Duration, error) {
	left := d.at.Sub(now)
	if left <= 0 {
		return 0, deadlineExceededError()
	}
	return left, nil
}

// reserving is the deadline reserve earlier; it is already expired when less than reserve
// remains.
func (d deadline) reserving(reserve time.Duration) deadline {
	return deadline{at: d.at.Add(-reserve)}
}

// shutdownUnit is the unit of the runner's two-, three- and four-unit shutdown boundaries.
const shutdownUnit = time.Second

// monotonicSample is a CLOCK_MONOTONIC reading in nanoseconds, bracketed by two readings of
// clock.now.
type monotonicSample struct {
	before      time.Time
	monotonicNs uint64
	after       time.Time
}

func (s monotonicSample) validate() error {
	if s.before.After(s.after) {
		return appError("input clock bracket is reversed")
	}
	return nil
}

// clock is shared by the goroutines of one runner.
type clock interface {
	now() time.Time
	wallNow() time.Time
	monotonicSample() (monotonicSample, error)
	sleep(duration time.Duration)
	shutdownUnit() time.Duration
}

// wallStamp is the canonical storage form of c.wallNow() (see formatUTC).
func wallStamp(c clock) string {
	return formatUTC(c.wallNow())
}

type systemClock struct{}

func (systemClock) now() time.Time {
	return time.Now()
}

func (systemClock) wallNow() time.Time {
	return time.Now().UTC()
}

func (systemClock) monotonicSample() (monotonicSample, error) {
	before := time.Now()
	var reading unix.Timespec
	if err := unix.ClockGettime(unix.CLOCK_MONOTONIC, &reading); err != nil {
		var errno syscall.Errno
		if !errors.As(err, &errno) {
			panic("clock_gettime failed without an errno: " + err.Error())
		}
		return monotonicSample{}, appError("member runtime: " + osErrorText(errno))
	}
	after := time.Now()
	if reading.Sec < 0 {
		return monotonicSample{}, appError("monotonic clock seconds must be nonnegative")
	}
	if reading.Nsec < 0 {
		return monotonicSample{}, appError("monotonic clock nanoseconds must be nonnegative")
	}
	if reading.Nsec >= 1_000_000_000 {
		return monotonicSample{}, appError("monotonic clock nanoseconds exceed their range")
	}
	high, low := bits.Mul64(uint64(reading.Sec), 1_000_000_000)
	total, carry := bits.Add64(low, uint64(reading.Nsec), 0)
	if high != 0 || carry != 0 {
		return monotonicSample{}, appError("monotonic clock nanoseconds overflow")
	}
	return monotonicSample{before: before, monotonicNs: total, after: after}, nil
}

func (systemClock) sleep(duration time.Duration) {
	time.Sleep(duration)
}

func (systemClock) shutdownUnit() time.Duration {
	return shutdownUnit
}
