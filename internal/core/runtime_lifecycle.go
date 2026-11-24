package core

import "time"

// childExit is how the native child ended: an exit code or a terminating signal.
//
//sumtype:decl
type childExit interface {
	isChildExit()
}

type childExitCode struct {
	code int32
}

type childExitSignal struct {
	signal int32
}

func (childExitCode) isChildExit()   {}
func (childExitSignal) isChildExit() {}

// shutdownDeadlines are the shutdown boundaries two, three and four units after start, each
// clipped to limit.
type shutdownDeadlines struct {
	start time.Time
	unit  time.Duration
	limit opt[time.Time]
}

func shutdownDeadlinesStartingNow(c clock) shutdownDeadlines {
	return shutdownDeadlinesStartingAt(c.now(), c.shutdownUnit())
}

func shutdownDeadlinesStartingAt(start time.Time, unit time.Duration) shutdownDeadlines {
	return shutdownDeadlines{start: start, unit: unit}
}

func (d shutdownDeadlines) clipped(deadline time.Time) shutdownDeadlines {
	if prior, set := d.limit.get(); set && prior.Before(deadline) {
		deadline = prior
	}
	d.limit = some(deadline)
	return d
}

func (d shutdownDeadlines) boundary(units int64) time.Time {
	original := d.start.Add(d.unit * time.Duration(units))
	if limit, set := d.limit.get(); set && limit.Before(original) {
		return limit
	}
	return original
}

func (d shutdownDeadlines) childDeadline() time.Time {
	return d.boundary(2)
}

func (d shutdownDeadlines) displayDeadline() time.Time {
	return d.boundary(3)
}

func (d shutdownDeadlines) storageDeadline() time.Time {
	return d.boundary(4)
}

func (d shutdownDeadlines) remainingStorageAt(now time.Time) time.Duration {
	return max(d.storageDeadline().Sub(now), 0)
}

func runnerExitStatus(native opt[childExit], complete bool) (int32, error) {
	var status int32
	exit, exited := native.get()
	switch exit := exit.(type) {
	case childExitCode:
		if err := ensure(exit.code >= 0, "negative native exit code"); err != nil {
			return 0, err
		}
		status = exit.code
	case childExitSignal:
		if err := ensure(exit.signal > 0, "positive native signal required"); err != nil {
			return 0, err
		}
		if exit.signal > maxInt32-128 {
			return 0, runtimeError("native exit signal overflow")
		}
		status = 128 + exit.signal
	case nil:
		if exited {
			panic("a recorded native exit is a code or a signal")
		}
		if err := ensure(!complete, "complete cleanup requires actual native status"); err != nil {
			return 0, err
		}
		status = 1
	default:
		panic("invalid childExit")
	}
	if !complete && status == 0 {
		return 1, nil
	}
	return status, nil
}

const maxInt32 = int32(^uint32(0) >> 1)

type shutdownOutcome struct {
	nativeExit   opt[childExit]
	cleanupState cleanupState
	diagnostics  []string
}

// newShutdownOutcome is an outcome whose cleanup is complete exactly when no diagnostic was
// recorded.
func newShutdownOutcome(nativeExit opt[childExit], diagnostics []string) shutdownOutcome {
	state := cleanupStateIncomplete
	if len(diagnostics) == 0 {
		state = cleanupStateComplete
	}
	return shutdownOutcome{nativeExit: nativeExit, cleanupState: state, diagnostics: diagnostics}
}

func shutdownOutcomeComplete(nativeExit opt[childExit]) shutdownOutcome {
	return newShutdownOutcome(nativeExit, nil)
}

// fail records diagnostic and marks the cleanup incomplete.
func (o *shutdownOutcome) fail(diagnostic string) {
	o.diagnostics = append(o.diagnostics, diagnostic)
	o.cleanupState = cleanupStateIncomplete
}

// note returns the value of a result, or records its error in diagnostics and reports false.
func note[T any](diagnostics *[]string, value T, err error) (T, bool) {
	if err != nil {
		*diagnostics = append(*diagnostics, cafleetErrorOf(err).message())
		var zero T
		return zero, false
	}
	return value, true
}
