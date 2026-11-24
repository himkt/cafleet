package core

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

// testClock is a virtual clock: time moves only through advance, setElapsed and sleep, and
// each sleep is recorded. Its wall time and CLOCK_MONOTONIC readings follow the same elapsed
// time from their origins.
type testClock struct {
	origin time.Time
	mu     sync.Mutex
	state  testClockState
}

type testClockState struct {
	elapsed      time.Duration
	sleeps       []time.Duration
	shutdownUnit time.Duration
	wallOrigin   time.Time
}

const testClockMonotonicOrigin = 1_000_000 * time.Second

var _ clock = (*testClock)(nil)

func testClockStartingAt(origin time.Time) *testClock {
	return &testClock{
		origin: origin,
		state: testClockState{
			shutdownUnit: shutdownUnit,
			wallOrigin:   time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC),
		},
	}
}

func (c *testClock) withShutdownUnit(unit time.Duration) *testClock {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state.shutdownUnit = unit
	return c
}

func (c *testClock) withWallOrigin(origin time.Time) *testClock {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state.wallOrigin = origin
	return c
}

func (c *testClock) advance(duration time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state.elapsed += duration
}

func (c *testClock) elapsed() time.Duration {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.state.elapsed
}

func (c *testClock) setElapsed(elapsed time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state.elapsed = elapsed
}

func (c *testClock) sleeps() []time.Duration {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]time.Duration(nil), c.state.sleeps...)
}

func (c *testClock) now() time.Time {
	return c.origin.Add(c.elapsed())
}

func (c *testClock) wallNow() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.state.wallOrigin.Add(c.state.elapsed)
}

func (c *testClock) monotonicSample() (monotonicSample, error) {
	elapsed := c.elapsed()
	now := c.origin.Add(elapsed)
	reading := testClockMonotonicOrigin + elapsed
	if reading < 0 {
		panic("the test monotonic reading is nonnegative")
	}
	return monotonicSample{
		before:      now,
		monotonicNs: uint64(reading),
		after:       now,
	}, nil
}

func (c *testClock) sleep(duration time.Duration) {
	if duration <= 0 {
		panic("idle checks advance the clock")
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	c.state.sleeps = append(c.state.sleeps, duration)
	c.state.elapsed += duration
}

func (c *testClock) shutdownUnit() time.Duration {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.state.shutdownUnit
}

func referenceSecond() time.Time {
	return time.Date(2026, 7, 30, 9, 53, 43, 0, time.UTC)
}

func TestTime_FormatEmitsSixDigitMicrosecondsAndOffset(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		micros int
		want   string
	}{
		{name: "fraction", micros: 198_561, want: "2026-07-30T09:53:43.198561+00:00"},
		{name: "zero fraction", micros: 0, want: "2026-07-30T09:53:43.000000+00:00"},
		{name: "leading zeros", micros: 7, want: "2026-07-30T09:53:43.000007+00:00"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			got := formatUTC(referenceSecond().Add(time.Duration(tc.micros) * time.Microsecond))
			if got != tc.want {
				t.Fatalf("%s: formatUTC = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestTime_FormatWritesTheUtcInstantOfAnyLocation(t *testing.T) {
	t.Parallel()
	tokyo := time.FixedZone("+09:00", 9*60*60)
	cases := []struct {
		name  string
		value time.Time
		want  string
	}{
		{name: "utc", value: referenceSecond(), want: "2026-07-30T09:53:43.000000+00:00"},
		{name: "fixed offset", value: referenceSecond().In(tokyo), want: "2026-07-30T09:53:43.000000+00:00"},
		{name: "nanoseconds truncate", value: referenceSecond().Add(999_999_999), want: "2026-07-30T09:53:43.999999+00:00"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := formatUTC(tc.value); got != tc.want {
				t.Fatalf("%s: formatUTC = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestTime_FormatRoundTripsThroughParse(t *testing.T) {
	t.Parallel()
	value := referenceSecond().Add(198_561 * time.Microsecond)
	parsed, ok := parseLenient(formatUTC(value))
	if !ok || !parsed.Equal(value) {
		t.Fatalf("parseLenient(formatUTC) = %v %v, want %v", parsed, ok, value)
	}
	if parsed.Location() != time.UTC {
		t.Fatalf("parseLenient location = %v, want UTC", parsed.Location())
	}
}

func TestTime_ParseAcceptsMissingFractionalPart(t *testing.T) {
	t.Parallel()
	parsed, ok := parseLenient("2026-07-30T09:53:43+00:00")
	if !ok || !parsed.Equal(referenceSecond()) {
		t.Fatalf("parseLenient = %v %v", parsed, ok)
	}
}

func TestTime_ParseAcceptsZOffsetSpelling(t *testing.T) {
	t.Parallel()
	parsed, ok := parseLenient("2026-07-30T09:53:43Z")
	if !ok || !parsed.Equal(referenceSecond()) {
		t.Fatalf("parseLenient = %v %v", parsed, ok)
	}
}

func TestTime_ParseConvertsAnOffsetToUtc(t *testing.T) {
	t.Parallel()
	parsed, ok := parseLenient("2026-07-30T18:53:43.5+09:00")
	want := referenceSecond().Add(500 * time.Millisecond)
	if !ok || !parsed.Equal(want) || parsed.Location() != time.UTC {
		t.Fatalf("parseLenient = %v %v, want %v in UTC", parsed, ok, want)
	}
}

func TestTime_ParseRejectsGarbage(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		value string
	}{
		{name: "prose", value: "yesterday at noon"},
		{name: "empty", value: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if parsed, ok := parseLenient(tc.value); ok {
				t.Fatalf("%s: parseLenient(%q) = %v", tc.name, tc.value, parsed)
			}
		})
	}
}

func TestTime_FormattedTimestampsOrderLexicographically(t *testing.T) {
	t.Parallel()
	base := referenceSecond()
	cases := []struct {
		name           string
		earlier, later time.Time
	}{
		{name: "within a second", earlier: base.Add(99_999 * time.Microsecond), later: base.Add(100_000 * time.Microsecond)},
		{name: "across a second", earlier: base.Add(999_999 * time.Microsecond), later: base.Add(time.Second)},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if earlier, later := formatUTC(tc.earlier), formatUTC(tc.later); earlier >= later {
				t.Fatalf("%s: %q sorts at or after %q", tc.name, earlier, later)
			}
		})
	}
}

func TestTime_DeadlineReservation_ReservingMovesTheDeadlineEarlierByTheReserve(t *testing.T) {
	t.Parallel()
	const testWorkReserve = 3500 * time.Millisecond
	left, err := deadlineUntil(time.Now().Add(5 * time.Second)).reserving(testWorkReserve).remaining()
	if err != nil {
		t.Fatal(err)
	}
	if left > 1500*time.Millisecond || left <= 1400*time.Millisecond {
		t.Fatalf("remaining after the reserve = %v", left)
	}
}

func TestTime_DeadlineReservation_ReservingMoreThanRemainsIsAlreadyExpired(t *testing.T) {
	t.Parallel()
	const testWorkReserve = 3500 * time.Millisecond
	if left, err := deadlineUntil(time.Now().Add(3 * time.Second)).reserving(testWorkReserve).remaining(); err == nil {
		t.Fatalf("remaining = %v, want an expired deadline", left)
	}
}

func TestTime_DeadlineRemainingAtIsPositiveUntilTheDeadline(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	at := deadlineUntil(origin.Add(2 * time.Second))
	cases := []struct {
		name string
		now  time.Time
		left time.Duration
	}{
		{name: "before", now: origin, left: 2 * time.Second},
		{name: "one nanosecond before", now: origin.Add(2*time.Second - 1), left: 1},
		{name: "at the deadline", now: origin.Add(2 * time.Second)},
		{name: "after the deadline", now: origin.Add(3 * time.Second)},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			left, err := at.remainingAt(tc.now)
			if tc.left == 0 {
				var failure *cafleetError
				if !errors.As(err, &failure) || failure.kind != errorDeadlineExceeded {
					t.Fatalf("%s: remainingAt = %v %v, want a deadline-exceeded error", tc.name, left, err)
				}
				return
			}
			if err != nil || left != tc.left {
				t.Fatalf("%s: remainingAt = %v %v, want %v", tc.name, left, err, tc.left)
			}
		})
	}
	if !at.instant().Equal(origin.Add(2 * time.Second)) {
		t.Fatalf("instant = %v", at.instant())
	}
}

func TestTime_NewDeadlineIsFiveSecondsAway(t *testing.T) {
	t.Parallel()
	before := time.Now()
	at := newDeadline().instant()
	after := time.Now()
	if at.Before(before.Add(5*time.Second)) || at.After(after.Add(5*time.Second)) {
		t.Fatalf("newDeadline = %v, want five seconds after a moment in [%v, %v]", at, before, after)
	}
}

func TestTime_MonotonicSampleValidateRejectsAReversedBracket(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	cases := []struct {
		name   string
		sample monotonicSample
		valid  bool
	}{
		{name: "ordered", sample: monotonicSample{before: origin, after: origin.Add(time.Millisecond)}, valid: true},
		{name: "equal", sample: monotonicSample{before: origin, after: origin}, valid: true},
		{name: "reversed", sample: monotonicSample{before: origin.Add(time.Millisecond), after: origin}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			err := tc.sample.validate()
			if tc.valid {
				if err != nil {
					t.Fatalf("%s: validate = %v", tc.name, err)
				}
				return
			}
			var failure *cafleetError
			if !errors.As(err, &failure) || failure.kind != errorApp || failure.message() != "input clock bracket is reversed" {
				t.Fatalf("%s: validate = %v", tc.name, err)
			}
		})
	}
}

func TestTime_SystemClockBracketsItsMonotonicReading(t *testing.T) {
	t.Parallel()
	var source clock = systemClock{}
	first, err := source.monotonicSample()
	if err != nil {
		t.Fatal(err)
	}
	second, err := source.monotonicSample()
	if err != nil {
		t.Fatal(err)
	}
	if err := first.validate(); err != nil {
		t.Fatal(err)
	}
	if second.monotonicNs < first.monotonicNs || second.before.Before(first.after) {
		t.Fatalf("samples run backwards: %+v then %+v", first, second)
	}
	if source.shutdownUnit() != time.Second || shutdownUnit != time.Second {
		t.Fatalf("shutdown unit = %v", source.shutdownUnit())
	}
	if source.wallNow().Location() != time.UTC {
		t.Fatalf("wallNow location = %v, want UTC", source.wallNow().Location())
	}
}

func TestTime_WallStampFormatsTheClocksWallTime(t *testing.T) {
	t.Parallel()
	source := testClockStartingAt(time.Now()).withWallOrigin(referenceSecond())
	source.advance(198_561 * time.Microsecond)
	if got, want := wallStamp(source), "2026-07-30T09:53:43.198561+00:00"; got != want {
		t.Fatalf("wallStamp = %q, want %q", got, want)
	}
}

func TestTime_TestClockMovesOnlyThroughAdvanceAndRecordedSleeps(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	source := testClockStartingAt(origin).withShutdownUnit(10 * time.Millisecond)
	source.advance(2 * time.Second)
	source.sleep(250 * time.Millisecond)
	source.sleep(time.Millisecond)
	if got := source.now(); !got.Equal(origin.Add(2251 * time.Millisecond)) {
		t.Fatalf("now = %v", got.Sub(origin))
	}
	if got := source.sleeps(); len(got) != 2 || got[0] != 250*time.Millisecond || got[1] != time.Millisecond {
		t.Fatalf("sleeps = %v", got)
	}
	sample, err := source.monotonicSample()
	if err != nil || sample.monotonicNs != uint64(testClockMonotonicOrigin+2251*time.Millisecond) || !sample.before.Equal(sample.after) {
		t.Fatalf("monotonicSample = %+v %v", sample, err)
	}
	if got := source.wallNow(); !got.Equal(time.Date(2026, 1, 1, 0, 0, 2, 251_000_000, time.UTC)) {
		t.Fatalf("wallNow = %v", got)
	}
	source.setElapsed(time.Second)
	if source.elapsed() != time.Second || source.shutdownUnit() != 10*time.Millisecond {
		t.Fatalf("elapsed = %v, shutdown unit = %v", source.elapsed(), source.shutdownUnit())
	}
	defer func() {
		if recover() == nil {
			t.Fatal("a zero sleep panics")
		}
	}()
	source.sleep(0)
}

const deferralScenarioEnv = "CAFLEET_TEST_DEFERRAL_SCENARIO"

//nolint:paralleltest // changes process-wide signal handlers inside its own child process
func TestTime_DeferTerminationChild(t *testing.T) {
	skipUnlessChildTest(t)
	begin := func() *deferTermination {
		span, err := deferTerminationBegin()
		if err != nil {
			t.Fatal(err)
		}
		return span
	}
	raise := func(sig syscall.Signal) {
		if err := syscall.Kill(os.Getpid(), sig); err != nil {
			t.Fatal(err)
		}
		time.Sleep(200 * time.Millisecond)
	}
	report := func(line string) {
		fmt.Println(line)
	}
	switch scenario := os.Getenv(deferralScenarioEnv); scenario {
	case "signal inside a span":
		span := begin()
		raise(syscall.SIGTERM)
		report("deferred")
		span.close()
		time.Sleep(200 * time.Millisecond)
		report("survived")
	case "nested spans":
		outer := begin()
		inner := begin()
		raise(syscall.SIGINT)
		inner.close()
		inner.close()
		time.Sleep(200 * time.Millisecond)
		report("inner closed")
		outer.close()
		time.Sleep(200 * time.Millisecond)
		report("survived")
	case "first signal wins":
		span := begin()
		raise(syscall.SIGHUP)
		raise(syscall.SIGTERM)
		report("deferred")
		span.close()
		time.Sleep(200 * time.Millisecond)
		report("survived")
	case "no signal":
		span := begin()
		span.close()
		span.close()
		report("survived")
	case "ignored signal stays ignored":
		ignoreHangup(t)
		span := begin()
		raise(syscall.SIGHUP)
		span.close()
		raise(syscall.SIGHUP)
		report("survived")
	case "a later span defers again":
		begin().close()
		span := begin()
		raise(syscall.SIGTERM)
		report("deferred")
		span.close()
		time.Sleep(200 * time.Millisecond)
		report("survived")
	default:
		t.Fatalf("unknown deferral scenario %q", scenario)
	}
}

func ignoreHangup(t *testing.T) {
	t.Helper()
	signal.Ignore(syscall.SIGHUP)
	if !signal.Ignored(syscall.SIGHUP) {
		t.Fatal("SIGHUP is ignored before the span")
	}
}

func TestTime_DeferTerminationDefersTerminationSignalsUntilTheOutermostSpanEnds(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		lines  []string
		signal syscall.Signal
	}{
		{name: "signal inside a span", lines: []string{"deferred"}, signal: syscall.SIGTERM},
		{name: "nested spans", lines: []string{"inner closed"}, signal: syscall.SIGINT},
		{name: "first signal wins", lines: []string{"deferred"}, signal: syscall.SIGHUP},
		{name: "no signal", lines: []string{"survived"}},
		{name: "ignored signal stays ignored", lines: []string{"survived"}},
		{name: "a later span defers again", lines: []string{"deferred"}, signal: syscall.SIGTERM},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			command := childTestCommand(TestTime_DeferTerminationChild)
			command.Env = append(command.Env, deferralScenarioEnv+"="+tc.name)
			output, err := command.Output()
			text := string(output)
			for _, line := range tc.lines {
				if !strings.Contains(text, line+"\n") {
					t.Fatalf("%s: child output lacks %q:\n%s", tc.name, line, text)
				}
			}
			if tc.signal == 0 {
				if err != nil {
					t.Fatalf("%s: child failed: %v\n%s", tc.name, err, text)
				}
				return
			}
			if strings.Contains(text, "survived") {
				t.Fatalf("%s: the deferred signal did not take its default action:\n%s", tc.name, text)
			}
			var exit *exec.ExitError
			if !errors.As(err, &exit) {
				t.Fatalf("%s: child result %v, want termination by %v", tc.name, err, tc.signal)
			}
			status, ok := exit.Sys().(syscall.WaitStatus)
			if !ok || !status.Signaled() || status.Signal() != tc.signal {
				t.Fatalf("%s: child ended with %v, want termination by %v", tc.name, exit, tc.signal)
			}
		})
	}
}

// onFreshThread runs test on a new goroutine locked to its own OS thread with an empty blocked
// set. The goroutine ends still locked, so the runtime discards the thread and its mask.
func onFreshThread(t *testing.T, test func() error) {
	t.Helper()
	failure := make(chan error)
	go func() {
		runtime.LockOSThread()
		defer func() {
			if recovered := recover(); recovered != nil {
				failure <- fmt.Errorf("panic: %v", recovered)
			}
		}()
		if err := threadSetSignalMask(0); err != nil {
			failure <- err
			return
		}
		failure <- test()
	}()
	if err := <-failure; err != nil {
		t.Fatal(err)
	}
}

func blockedNow() (signalSet, error) {
	return threadSignalMask()
}

func defersAll(mask signalSet) bool {
	return mask&deferredTerminationSet == deferredTerminationSet
}

func defersNone(mask signalSet) bool {
	return mask&deferredTerminationSet == 0
}

func TestTime_ASpanBlocksSigintSigtermAndSighupUntilDropped(t *testing.T) {
	t.Parallel()
	onFreshThread(t, func() error {
		previous, err := threadBlockSignals(deferredTerminationSet)
		if err != nil {
			return err
		}
		if mask, err := blockedNow(); err != nil || !defersAll(mask) {
			return fmt.Errorf("inside the span the thread blocks %#x (%v), want all of %#x", mask, err, deferredTerminationSet)
		}
		if err := threadSetSignalMask(previous); err != nil {
			return err
		}
		if mask, err := blockedNow(); err != nil || !defersNone(mask) {
			return fmt.Errorf("after the span the thread blocks %#x (%v), want none of %#x", mask, err, deferredTerminationSet)
		}
		return nil
	})
}

func TestTime_ASpanRestoresSignalsBlockedBeforeItBegan(t *testing.T) {
	t.Parallel()
	onFreshThread(t, func() error {
		if err := threadSetSignalMask(signalSetOf(syscall.SIGUSR1, syscall.SIGTERM)); err != nil {
			return err
		}
		previous, err := threadBlockSignals(deferredTerminationSet)
		if err != nil {
			return err
		}
		if err := threadSetSignalMask(previous); err != nil {
			return err
		}
		mask, err := blockedNow()
		switch {
		case err != nil:
			return err
		case !mask.contains(syscall.SIGUSR1):
			return fmt.Errorf("SIGUSR1 stays blocked: %#x", mask)
		case !mask.contains(syscall.SIGTERM):
			return fmt.Errorf("a signal the caller had blocked stays blocked: %#x", mask)
		case mask.contains(syscall.SIGINT) || mask.contains(syscall.SIGHUP):
			return fmt.Errorf("the span's own signals are unblocked again: %#x", mask)
		}
		return nil
	})
}

// processSignalSet reads one signal set of process pid as the kernel reports it: the named
// /proc status line (SigBlk, SigIgn, ShdPnd) on Linux, the named ps column (sigmask, sigignore)
// elsewhere. The probe is a process that never changes its own signal state, such as sleep;
// shells clear and re-block their masks and cannot serve.
func processSignalSet(pid int, statusField, psColumn string) (signalSet, error) {
	if runtime.GOOS == "linux" {
		status, err := os.ReadFile(fmt.Sprintf("/proc/%d/status", pid))
		if err != nil {
			return 0, err
		}
		for _, line := range strings.Split(string(status), "\n") {
			if value, found := strings.CutPrefix(line, statusField+":"); found {
				mask, err := strconv.ParseUint(strings.TrimSpace(value), 16, 64)
				return signalSet(mask), err
			}
		}
		return 0, fmt.Errorf("/proc/%d/status has no %s line", pid, statusField)
	}
	output, err := exec.Command("ps", "-o", psColumn+"=", "-p", strconv.Itoa(pid)).Output()
	if err != nil {
		return 0, fmt.Errorf("ps -o %s: %w", psColumn, err)
	}
	mask, err := strconv.ParseUint(strings.TrimSpace(string(output)), 16, 64)
	return signalSet(mask), err
}

// startedSleeper starts `sleep 30` with start; its signal state is the state it inherited.
func startedSleeper(start func(*exec.Cmd) error) (*exec.Cmd, error) {
	sleeper := exec.Command("sleep", "30")
	if err := start(sleeper); err != nil {
		return nil, err
	}
	return sleeper, nil
}

func endSleeper(sleeper *exec.Cmd) error {
	if err := sleeper.Process.Kill(); err != nil {
		return err
	}
	var exit *exec.ExitError
	if err := sleeper.Wait(); !errors.As(err, &exit) {
		return fmt.Errorf("the killed sleeper ended with %v", err)
	}
	return nil
}

// childBlockedSet starts a sleeper with start and returns the blocked set it inherited.
func childBlockedSet(start func(*exec.Cmd) error) (signalSet, error) {
	sleeper, err := startedSleeper(start)
	if err != nil {
		return 0, err
	}
	mask, readErr := processSignalSet(sleeper.Process.Pid, "SigBlk", "sigmask")
	return mask, errors.Join(readErr, endSleeper(sleeper))
}

func plainStart(cmd *exec.Cmd) error {
	return cmd.Start()
}

func TestTime_AHelperThreadStartedInsideASpanInheritsTheBlockedSet(t *testing.T) {
	t.Parallel()
	onFreshThread(t, func() error {
		if _, err := threadBlockSignals(deferredTerminationSet); err != nil {
			return err
		}
		inherited, err := childBlockedSet(plainStart)
		if err != nil {
			return err
		}
		if !defersAll(inherited) {
			return fmt.Errorf("a child started on the masked thread blocks %#x, want all of %#x", inherited, deferredTerminationSet)
		}
		return nil
	})
}

func TestTime_UnwindingRestoresTheCallersSignalMask(t *testing.T) {
	t.Parallel()
	onFreshThread(t, func() error {
		previous, err := blockedNow()
		if err != nil {
			return err
		}
		var inside signalSet
		unwound := func() (recovered any) {
			defer func() { recovered = recover() }()
			spanPrevious, err := threadBlockSignals(deferredTerminationSet)
			if err != nil {
				return err
			}
			defer func() {
				if err := threadSetSignalMask(spanPrevious); err != nil {
					panic(err)
				}
			}()
			if inside, err = blockedNow(); err != nil {
				return err
			}
			panic("unwind protected work")
		}()
		if unwound != "unwind protected work" {
			return fmt.Errorf("the protected work unwound with %v", unwound)
		}
		if !defersAll(inside) {
			return fmt.Errorf("inside the span the thread blocks %#x", inside)
		}
		if after, err := blockedNow(); err != nil || after != previous {
			return fmt.Errorf("after unwinding the thread blocks %#x (%v), want %#x", after, err, previous)
		}
		return nil
	})
}

func TestTime_MaskedSpawnerThreadReadsTheSetWhileASecondLockedThreadReadsNone(t *testing.T) {
	t.Parallel()
	spawner, err := newMaskedSpawner()
	if err != nil {
		t.Fatal(err)
	}
	defer spawner.stop()
	var masked signalSet
	var maskedErr error
	spawner.do(func() { masked, maskedErr = threadSignalMask() })
	if maskedErr != nil || !defersAll(masked) {
		t.Fatalf("the spawner thread blocks %#x (%v), want all of %#x", masked, maskedErr, deferredTerminationSet)
	}
	onFreshThread(t, func() error {
		sibling, err := threadSignalMask()
		if err != nil || !defersNone(sibling) {
			return fmt.Errorf("a second locked thread blocks %#x (%v), want none of %#x", sibling, err, deferredTerminationSet)
		}
		return nil
	})
}

func spawnerStart(spawner *maskedSpawner) func(*exec.Cmd) error {
	return func(cmd *exec.Cmd) error {
		var err error
		spawner.do(func() { err = cmd.Start() })
		return err
	}
}

func TestTime_MaskedSpawnerChildInheritsTheSetAndDefersItsOwnSigterm(t *testing.T) {
	t.Parallel()
	spawner, err := newMaskedSpawner()
	if err != nil {
		t.Fatal(err)
	}
	defer spawner.stop()
	inherited, err := childBlockedSet(spawnerStart(spawner))
	if err != nil || !defersAll(inherited) {
		t.Fatalf("a spawner child blocks %#x (%v), want all of %#x", inherited, err, deferredTerminationSet)
	}
	sleeper, err := startedSleeper(spawnerStart(spawner))
	if err != nil {
		t.Fatal(err)
	}
	if err := sleeper.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	exited := make(chan error, 1)
	go func() { exited <- sleeper.Wait() }()
	select {
	case err := <-exited:
		t.Fatalf("a spawner child ended on SIGTERM while it blocks it: %v", err)
	case <-time.After(300 * time.Millisecond):
	}
	if runtime.GOOS == "linux" {
		pending, err := processSignalSet(sleeper.Process.Pid, "ShdPnd", "")
		if err != nil || !pending.contains(syscall.SIGTERM) {
			t.Fatalf("the deferred SIGTERM stays pending: %#x %v", pending, err)
		}
	}
	if err := sleeper.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	var exit *exec.ExitError
	if err := <-exited; !errors.As(err, &exit) {
		t.Fatalf("the killed spawner child ended with %v", err)
	}
}

func TestTime_MaskedSpawnerChildrenAloneInheritTheSetWhileOrdinaryGoroutinesSpawnConcurrently(t *testing.T) {
	t.Parallel()
	const (
		maskedChildren = 40
		siblings       = 8
		perSibling     = 10
	)
	spawner, err := newMaskedSpawner()
	if err != nil {
		t.Fatal(err)
	}
	defer spawner.stop()
	var mu sync.Mutex
	var siblingFailures []string
	var wg sync.WaitGroup
	start := make(chan struct{})
	for range siblings {
		wg.Go(func() {
			<-start
			for range perSibling {
				mask, err := childBlockedSet(plainStart)
				if err != nil || !defersNone(mask) {
					mu.Lock()
					siblingFailures = append(siblingFailures, fmt.Sprintf("%#x %v", mask, err))
					mu.Unlock()
				}
			}
		})
	}
	close(start)
	var maskedFailures []string
	for range maskedChildren {
		mask, err := childBlockedSet(spawnerStart(spawner))
		if err != nil || !defersAll(mask) {
			maskedFailures = append(maskedFailures, fmt.Sprintf("%#x %v", mask, err))
		}
	}
	wg.Wait()
	if len(maskedFailures) != 0 || len(siblingFailures) != 0 {
		t.Fatalf("spawner children missing the set: %v; ordinary children holding it: %v", maskedFailures, siblingFailures)
	}
}

//nolint:paralleltest // changes process-wide signal handlers inside its own child process
func TestTime_SpanChildStartsChild(t *testing.T) {
	skipUnlessChildTest(t)
	span, err := deferTerminationBegin()
	if err != nil {
		t.Fatal(err)
	}
	inSpan, err := childBlockedSet(span.start)
	if err != nil || !defersAll(inSpan) {
		t.Fatalf("a span child blocks %#x (%v), want all of %#x", inSpan, err, deferredTerminationSet)
	}
	routed := make(chan error)
	go func() {
		mask, err := childBlockedSet(startChild)
		if err == nil && !defersAll(mask) {
			err = fmt.Errorf("an in-span startChild child on another goroutine blocks %#x", mask)
		}
		routed <- err
	}()
	if err := <-routed; err != nil {
		t.Fatal(err)
	}
	span.close()
	after, err := childBlockedSet(startChild)
	if err != nil || !defersNone(after) {
		t.Fatalf("a child started after the span blocks %#x (%v), want none of %#x", after, err, deferredTerminationSet)
	}
	fmt.Println("span child checks passed")
}

func TestTime_InSpanStartsRouteThroughTheMaskedSpawnerAndLaterStartsDoNot(t *testing.T) {
	t.Parallel()
	output, err := childTestCommand(TestTime_SpanChildStartsChild, "-test.v").CombinedOutput()
	if err != nil || !strings.Contains(string(output), "span child checks passed\n") {
		t.Fatalf("span child failed: %v\n%s", err, output)
	}
}
