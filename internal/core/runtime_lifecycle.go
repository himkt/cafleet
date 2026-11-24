package core

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"time"
)

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

// childExitWire is a childExit's externally tagged wire object: exactly one member is set.
type childExitWire struct {
	Code   opt[int32] `json:"Code"`
	Signal opt[int32] `json:"Signal"`
}

type shutdownOutcomeWire struct {
	NativeExit   opt[childExitWire] `json:"native_exit"`
	CleanupState cleanupState       `json:"cleanup_state"`
	Diagnostics  []string           `json:"diagnostics"`
}

func (*shutdownOutcomeWire) ignoresUnknownFields() {}

func (o shutdownOutcome) MarshalJSONTo(encoder *jsontext.Encoder) error {
	wire := shutdownOutcomeWire{CleanupState: o.cleanupState, Diagnostics: o.diagnostics}
	if exit, exited := o.nativeExit.get(); exited {
		switch exit := exit.(type) {
		case childExitCode:
			wire.NativeExit = some(childExitWire{Code: some(exit.code)})
		case childExitSignal:
			wire.NativeExit = some(childExitWire{Signal: some(exit.signal)})
		default:
			panic("invalid childExit")
		}
	}
	return json.MarshalEncode(encoder, jsonObject{
		{"native_exit", wire.NativeExit},
		{"cleanup_state", wire.CleanupState},
		{"diagnostics", wire.Diagnostics},
	})
}

func (w childExitWire) MarshalJSONTo(encoder *jsontext.Encoder) error {
	if code, set := w.Code.get(); set {
		return json.MarshalEncode(encoder, jsonObject{{"Code", code}})
	}
	signal, set := w.Signal.get()
	if !set {
		panic("a childExit wire object has one member")
	}
	return json.MarshalEncode(encoder, jsonObject{{"Signal", signal}})
}

func shutdownOutcomeDecode(value jsontext.Value) (shutdownOutcome, error) {
	var wire shutdownOutcomeWire
	if err := decodeStrict(value, &wire); err != nil {
		return shutdownOutcome{}, ioError(cafleetErrorOf(err).message())
	}
	nativeExit := none[childExit]()
	if exit, exited := wire.NativeExit.get(); exited {
		code, isCode := exit.Code.get()
		signal, isSignal := exit.Signal.get()
		switch {
		case isCode && !isSignal:
			nativeExit = some[childExit](childExitCode{code: code})
		case isSignal && !isCode:
			nativeExit = some[childExit](childExitSignal{signal: signal})
		default:
			return shutdownOutcome{}, ioError("invalid native exit variant")
		}
	}
	return shutdownOutcome{nativeExit: nativeExit, cleanupState: wire.CleanupState, diagnostics: wire.Diagnostics}, nil
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

type shutdownHost interface {
	clock() clock
	gateProgrammaticInput()
	termVerifiedDescendants() error
	drainAndReapUntil(deadline time.Time) (opt[childExit], error)
	killVerifiedSurvivors() error
	closePty() error
	finalNativeExit() (opt[childExit], error)
	nativeWasSpawned() bool
	flushDisplayUntil(deadline time.Time) (uint64, error)
	finishEventsUntil(deadline time.Time) (uint64, uint64, error)
	restoreTerminal() error
}

func shutdown(host shutdownHost, deadlines shutdownDeadlines, native opt[childExit]) shutdownOutcome {
	outcome := shutdownOutcomeComplete(native)
	reconcileExit := func(exit opt[childExit], conflict string) {
		observed, set := exit.get()
		if !set {
			return
		}
		if recorded, known := outcome.nativeExit.get(); !known {
			outcome.nativeExit = exit
		} else if recorded != observed {
			outcome.diagnostics = append(outcome.diagnostics, conflict)
		}
	}
	host.gateProgrammaticInput()
	note(&outcome.diagnostics, struct{}{}, host.termVerifiedDescendants())
	drained, err := host.drainAndReapUntil(deadlines.childDeadline())
	drained, _ = note(&outcome.diagnostics, drained, err)
	reconcileExit(drained, "conflicting native exit status")
	note(&outcome.diagnostics, struct{}{}, host.killVerifiedSurvivors())
	note(&outcome.diagnostics, struct{}{}, host.closePty())
	finalExit, err := host.finalNativeExit()
	finalExit, _ = note(&outcome.diagnostics, finalExit, err)
	reconcileExit(finalExit, "conflicting final native exit status")
	droppedDisplayBytes, err := host.flushDisplayUntil(deadlines.displayDeadline())
	droppedDisplayBytes, _ = note(&outcome.diagnostics, droppedDisplayBytes, err)
	pendingEventFrames, unsavedObservationGap, err := host.finishEventsUntil(deadlines.storageDeadline())
	if _, finished := note(&outcome.diagnostics, struct{}{}, err); !finished {
		pendingEventFrames, unsavedObservationGap = 0, 0
	}
	note(&outcome.diagnostics, struct{}{}, host.restoreTerminal())
	if host.clock().now().After(deadlines.storageDeadline()) {
		outcome.diagnostics = append(outcome.diagnostics, "shutdown exceeded its original storage deadline")
	}
	if droppedDisplayBytes > 0 {
		outcome.diagnostics = append(outcome.diagnostics, fmt.Sprintf("display truncated: %d undelivered bytes", droppedDisplayBytes))
	}
	if pendingEventFrames > 0 || unsavedObservationGap > 0 {
		outcome.diagnostics = append(outcome.diagnostics, fmt.Sprintf("pending events=%d unsaved observation gap=%d", pendingEventFrames, unsavedObservationGap))
	}
	if !outcome.nativeExit.set && host.nativeWasSpawned() {
		outcome.diagnostics = append(outcome.diagnostics, "native exit remains unconfirmed")
	}
	if len(outcome.diagnostics) > 0 {
		outcome.cleanupState = cleanupStateIncomplete
	}
	return outcome
}

type controlIdentity struct {
	DatabaseIdentityJSON jsontext.Value `json:"database_identity_json"`
	FleetID              int64          `json:"fleet_id"`
	MemberID             int64          `json:"member_id"`
	DirectorMemberID     int64          `json:"director_member_id"`
	RunID                int64          `json:"run_id"`
	OwnerEpoch           int64          `json:"owner_epoch"`
	LaunchNonce          string         `json:"launch_nonce"`
}

func controlIdentityFromRun(run runRecord) controlIdentity {
	return controlIdentity{
		DatabaseIdentityJSON: run.databaseIdentityJSON,
		FleetID:              run.fleetID,
		MemberID:             run.memberID,
		DirectorMemberID:     run.directorMemberID,
		RunID:                run.runID,
		OwnerEpoch:           run.ownerEpoch,
		LaunchNonce:          run.launchNonce,
	}
}

func (c controlIdentity) equal(other controlIdentity) bool {
	return jsonEqual(c.DatabaseIdentityJSON, other.DatabaseIdentityJSON) && c.FleetID == other.FleetID &&
		c.MemberID == other.MemberID && c.DirectorMemberID == other.DirectorMemberID && c.RunID == other.RunID &&
		c.OwnerEpoch == other.OwnerEpoch && c.LaunchNonce == other.LaunchNonce
}

func (c controlIdentity) validate(run runRecord) error {
	return ensure(c.equal(controlIdentityFromRun(run)), "control run/database/nonce/epoch identity mismatch")
}
