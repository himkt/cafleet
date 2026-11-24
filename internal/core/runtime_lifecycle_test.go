package core

import (
	"slices"
	"testing"
	"time"
)

type shutdownCall struct {
	name string
	at   time.Time
}

// scriptedShutdownHost records every shutdown call with the virtual time it ran at, advances its
// clock to each bounded phase's deadline and fails the calls named in fail.
type scriptedShutdownHost struct {
	testClock *testClock
	calls     []shutdownCall
	deadlines []shutdownCall
	fail      []string
	native    opt[childExit]
	dropped   uint64
	pending   uint64
	gap       uint64
}

func newScriptedShutdownHost(now time.Time) *scriptedShutdownHost {
	return &scriptedShutdownHost{testClock: testClockStartingAt(now), native: some[childExit](childExitCode{code: 0})}
}

func (h *scriptedShutdownHost) reach(deadline time.Time) {
	if remaining := deadline.Sub(h.testClock.now()); remaining > 0 {
		h.testClock.advance(remaining)
	}
}

func (h *scriptedShutdownHost) call(name string) error {
	h.calls = append(h.calls, shutdownCall{name: name, at: h.testClock.now()})
	if slices.Contains(h.fail, name) {
		return appError("fixture " + name + " failure")
	}
	return nil
}

func (h *scriptedShutdownHost) bounded(name string, deadline time.Time) error {
	h.deadlines = append(h.deadlines, shutdownCall{name: name, at: deadline})
	h.reach(deadline)
	return h.call(name)
}

func (h *scriptedShutdownHost) clock() clock { return h.testClock }

func (h *scriptedShutdownHost) gateProgrammaticInput() {
	h.calls = append(h.calls, shutdownCall{name: "gate", at: h.testClock.now()})
}

func (h *scriptedShutdownHost) termVerifiedDescendants() error { return h.call("term") }

func (h *scriptedShutdownHost) drainAndReapUntil(deadline time.Time) (opt[childExit], error) {
	if err := h.bounded("drain", deadline); err != nil {
		return none[childExit](), err
	}
	return h.native, nil
}

func (h *scriptedShutdownHost) killVerifiedSurvivors() error { return h.call("kill") }

func (h *scriptedShutdownHost) closePty() error { return h.call("close") }

func (h *scriptedShutdownHost) finalNativeExit() (opt[childExit], error) {
	return none[childExit](), nil
}

func (h *scriptedShutdownHost) nativeWasSpawned() bool { return true }

func (h *scriptedShutdownHost) flushDisplayUntil(deadline time.Time) (uint64, error) {
	if err := h.bounded("display", deadline); err != nil {
		return 0, err
	}
	return h.dropped, nil
}

func (h *scriptedShutdownHost) finishEventsUntil(deadline time.Time) (uint64, uint64, error) {
	if err := h.bounded("events", deadline); err != nil {
		return 0, 0, err
	}
	return h.pending, h.gap, nil
}

func (h *scriptedShutdownHost) restoreTerminal() error { return h.call("restore") }

var everyShutdownCall = []string{"gate", "term", "drain", "kill", "close", "display", "events", "restore"}

func TestRuntimeLifecycle_RunnerShutdownFollowsOneOriginalScheduleAndKeepsEveryOutcome(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name            string
		hostStartsAfter time.Duration
		drained         opt[childExit]
		fail            []string
		dropped         uint64
		pending         uint64
		gap             uint64
		reported        opt[childExit]
		calls           []string
		nativeExit      opt[childExit]
		cleanupState    cleanupState
		diagnostics     []string
	}{
		{
			name:         "one clock bounds descendants, display, events and restoration",
			drained:      none[childExit](),
			reported:     some[childExit](childExitCode{code: 7}),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 7}),
			cleanupState: cleanupStateComplete,
		},
		{
			name:         "every failure is collected and the terminal is still restored without an extra budget",
			drained:      none[childExit](),
			fail:         []string{"term", "drain", "kill", "close", "display", "events", "restore"},
			reported:     some[childExit](childExitCode{code: 9}),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 9}),
			cleanupState: cleanupStateIncomplete,
			diagnostics: []string{
				"fixture term failure",
				"fixture drain failure",
				"fixture kill failure",
				"fixture close failure",
				"fixture display failure",
				"fixture events failure",
				"fixture restore failure",
			},
		},
		{
			name:         "display truncation, pending frames and unsaved gaps stay separate",
			drained:      some[childExit](childExitCode{code: 0}),
			dropped:      17,
			pending:      2,
			gap:          3,
			reported:     none[childExit](),
			calls:        everyShutdownCall,
			nativeExit:   some[childExit](childExitCode{code: 0}),
			cleanupState: cleanupStateIncomplete,
			diagnostics: []string{
				"display truncated: 17 undelivered bytes",
				"pending events=2 unsaved observation gap=3",
			},
		},
		{
			name:            "a clock started before the current phase is never rebased",
			hostStartsAfter: 3 * time.Second,
			drained:         none[childExit](),
			reported:        some[childExit](childExitSignal{signal: 15}),
			calls:           everyShutdownCall,
			nativeExit:      some[childExit](childExitSignal{signal: 15}),
			cleanupState:    cleanupStateComplete,
		},
	}
	for _, c := range cases {
		start := time.Now()
		hostStart := start.Add(c.hostStartsAfter)
		host := newScriptedShutdownHost(hostStart)
		host.native = c.drained
		host.fail = c.fail
		host.dropped = c.dropped
		host.pending = c.pending
		host.gap = c.gap
		outcome := shutdown(host, shutdownDeadlinesStartingAt(start, shutdownUnit), c.reported)
		var calls []string
		for _, call := range host.calls {
			calls = append(calls, call.name)
		}
		if !slices.Equal(calls, c.calls) {
			t.Fatalf("%s: calls %v, want %v", c.name, calls, c.calls)
		}
		wantDeadlines := []shutdownCall{
			{"drain", start.Add(2 * time.Second)},
			{"display", start.Add(3 * time.Second)},
			{"events", start.Add(4 * time.Second)},
		}
		if !slices.EqualFunc(host.deadlines, wantDeadlines, func(a, b shutdownCall) bool { return a.name == b.name && a.at.Equal(b.at) }) {
			t.Fatalf("%s: deadlines %v, want %v", c.name, host.deadlines, wantDeadlines)
		}
		childDeadline := start.Add(2 * time.Second)
		if hostStart.After(childDeadline) {
			childDeadline = hostStart
		}
		for _, call := range host.calls {
			if (call.name == "kill" || call.name == "close") && !call.at.Equal(childDeadline) {
				t.Fatalf("%s: %s ran at %v, want %v", c.name, call.name, call.at, childDeadline)
			}
		}
		if now := host.testClock.now(); !now.Equal(start.Add(4 * time.Second)) {
			t.Fatalf("%s: the clock ends at %v, want %v", c.name, now, start.Add(4*time.Second))
		}
		if outcome.nativeExit != c.nativeExit {
			t.Fatalf("%s: native exit %v, want %v", c.name, outcome.nativeExit, c.nativeExit)
		}
		if outcome.cleanupState != c.cleanupState {
			t.Fatalf("%s: cleanup state %s, want %s", c.name, outcome.cleanupState.asStr(), c.cleanupState.asStr())
		}
		if !slices.Equal(outcome.diagnostics, c.diagnostics) {
			t.Fatalf("%s: diagnostics %q, want %q", c.name, outcome.diagnostics, c.diagnostics)
		}
	}
}

func TestRuntimeLifecycle_RunnerExitStatusKeepsNativeCodeSignalAndCleanupOutcomeDistinct(t *testing.T) {
	t.Parallel()
	cases := []struct {
		native   opt[childExit]
		complete bool
		expected int32
	}{
		{some[childExit](childExitCode{code: 0}), true, 0},
		{some[childExit](childExitCode{code: 7}), true, 7},
		{some[childExit](childExitSignal{signal: 15}), true, 143},
		{some[childExit](childExitCode{code: 0}), false, 1},
		{some[childExit](childExitCode{code: 7}), false, 7},
		{some[childExit](childExitSignal{signal: 15}), false, 143},
		{none[childExit](), false, 1},
	}
	for _, c := range cases {
		status, err := runnerExitStatus(c.native, c.complete)
		if err != nil || status != c.expected {
			t.Fatalf("native %v, cleanup complete %v: status %d, %v; want %d", c.native, c.complete, status, err, c.expected)
		}
	}
	if _, err := runnerExitStatus(none[childExit](), true); err == nil {
		t.Fatal("complete cleanup without a native exit is refused")
	}
}
