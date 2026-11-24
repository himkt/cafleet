package core

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"syscall"
	"time"
	"unicode/utf8"
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

// guarded runs cleanup, recording its error, or panicked when it panics, in notes.
func guarded[T any](notes *[]string, panicked string, cleanup func() (T, error)) (value T, ok bool) {
	defer func() {
		if recover() != nil {
			*notes = append(*notes, panicked)
			var zero T
			value, ok = zero, false
		}
	}()
	result, err := cleanup()
	return note(notes, result, err)
}

type pendingMemberSpec struct {
	fleetID     int64
	name        string
	description string
	placement   newPlacement
	// windowName is the chosen slot's window name, published with the pane.
	windowName string
	// advancesWindowOrdinalFrom is the fleet's last_window_ordinal as read under the layout lock,
	// for a new-window slot.
	advancesWindowOrdinalFrom opt[int64]
}

type runSettings struct {
	databaseIdentityJSON jsontext.Value
	creator              processIdentity
	launchNonce          string
	requestedMode        opt[string]
	resolvedMode         string
	modeSource           modeSource
	permissionArgvJSON   jsontext.Value
	nativeVersion        opt[string]
	coverage             coverage
}

type pendingRegistration struct {
	member registeredMember
	run    runRecord
	paths  runtimePaths
}

func registerPendingRun(ctx context.Context, conn *dbConn, member pendingMemberSpec, settings runSettings, deadline time.Time) (pendingRegistration, error) {
	return withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (pendingRegistration, error) {
		if err := ensureSameDatabase(ctx, conn, settings.databaseIdentityJSON); err != nil {
			return pendingRegistration{}, err
		}
		return inTransaction(conn, transactionImmediate, func() (pendingRegistration, error) {
			fleet, err := requireLiveFleet(ctx, conn, member.fleetID)
			if err != nil {
				return pendingRegistration{}, err
			}
			registered, err := registerMemberRows(ctx, conn, member.fleetID, member.name, member.description, member.placement)
			if err != nil {
				return pendingRegistration{}, err
			}
			if read, set := member.advancesWindowOrdinalFrom.get(); set {
				changed, err := conn.execute("UPDATE fleets SET last_window_ordinal=?1 WHERE fleet_id=?2 AND last_window_ordinal=?3",
					member.placement.windowOrdinal, member.fleetID, read)
				if err != nil {
					return pendingRegistration{}, err
				}
				if changed != 1 {
					return pendingRegistration{}, runtimeError(fmt.Sprintf("fleet %d window ordinal changed under the layout lock", member.fleetID))
				}
			}
			next, err := queryRow(conn, "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1", nil,
				func(row sqlRow) (int64, error) { return row.int64(0) })
			if err != nil {
				return pendingRegistration{}, err
			}
			paths, err := runtimePathsForRun(settings.databaseIdentityJSON, next)
			if err != nil {
				return pendingRegistration{}, err
			}
			run, err := insertRun(ctx, conn, newRun{
				memberID:             registered.memberID,
				fleetID:              member.fleetID,
				directorMemberID:     fleet.directorMemberID,
				backend:              member.placement.codingAgent,
				databaseIdentityJSON: settings.databaseIdentityJSON,
				launchNonce:          settings.launchNonce,
				runtimePath:          paths.runDir,
				requestedMode:        settings.requestedMode,
				resolvedMode:         settings.resolvedMode,
				modeSource:           settings.modeSource,
				permissionArgvJSON:   settings.permissionArgvJSON,
				nativeVersion:        settings.nativeVersion,
				coverage:             settings.coverage,
				ownerPID:             settings.creator.pid,
				ownerStartIdentity:   settings.creator.startIdentity,
			})
			if err != nil {
				return pendingRegistration{}, err
			}
			if err := ensure(run.runID == next, "atomic run allocation changed its reserved ID"); err != nil {
				return pendingRegistration{}, err
			}
			return pendingRegistration{member: registered, run: run, paths: paths}, nil
		})
	})
}

type prepared struct {
	Identity controlIdentity `json:"identity"`
	Runner   processIdentity `json:"runner"`
}

type started struct {
	Identity controlIdentity `json:"identity"`
	Runner   processIdentity `json:"runner"`
	Child    processIdentity `json:"child"`
}

func (*prepared) ignoresUnknownFields() {}
func (*started) ignoresUnknownFields()  {}

type launchHost interface {
	clock() clock
	currentExecutable() (string, error)
	splitRunner(paneName string, argv []string, deadline time.Time) (spawnedPane, error)
	waitPrepared(descriptor launchDescriptor, deadline time.Time) (prepared, error)
	start(prepared prepared, p placement, deadline time.Time) (started, error)
	shutdownOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error)
	closePane(pane spawnedPane, deadline time.Time) error
	settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, deadline time.Time) error
}

type launchOutcome struct {
	member    registeredMember
	run       runRecord
	placement placement
	started   started
}

func pendingFailure(ctx context.Context, conn *dbConn, run runRecord, deadlines shutdownDeadlines, diagnostics []string, c clock) error {
	_, err := withDatabaseDeadline(ctx, conn, deadlines.storageDeadline(), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := ensureSameDatabase(ctx, conn, run.databaseIdentityJSON); err != nil {
			return struct{}{}, err
		}
		return inTransaction(conn, transactionImmediate, func() (struct{}, error) {
			state := cleanupStateIncomplete
			if len(diagnostics) == 0 {
				state = cleanupStateComplete
			}
			if _, err := conn.execute("UPDATE member_runs SET status='failed',ended_at=COALESCE(ended_at,?1),cleanup_state=?2,cleanup_diagnostics_json=?3 WHERE run_id=?4 AND launch_nonce=?5",
				wallStamp(c), state.asStr(), string(jsonValue(diagnostics)), run.runID, run.launchNonce); err != nil {
				return struct{}{}, err
			}
			_, err := deregisterMemberRows(ctx, conn, run.memberID)
			return struct{}{}, err
		})
	})
	return err
}

func removePendingDescriptor(ctx context.Context, conn *dbConn, run runRecord) error {
	if run.runtimeDirIdentityJSON == nil {
		return nil
	}
	artifacts, err := runArtifactsFromRun(ctx, conn, run)
	if err != nil {
		return err
	}
	return artifacts.removeDescriptor()
}

// pendingGuard rolls a pending creation back unless the launch disarms it.
type pendingGuard struct {
	ctx   context.Context
	conn  *dbConn
	run   runRecord
	armed bool
	clock clock
}

func (g *pendingGuard) rollback(primary error, deadlines shutdownDeadlines, diagnostics []string) error {
	g.armed = false
	read, err := getRun(g.ctx, g.conn, g.run.runID)
	current, ok := note(&diagnostics, read, err)
	if !ok {
		current = g.run
	}
	note(&diagnostics, struct{}{}, removePendingDescriptor(g.ctx, g.conn, current))
	if err := pendingFailure(g.ctx, g.conn, current, deadlines, diagnostics, g.clock); err != nil {
		diagnostics = append(diagnostics, cafleetErrorOf(err).message())
	}
	failure := cafleetErrorOf(primary)
	for _, diagnostic := range diagnostics {
		failure = failure.withCleanup(diagnostic)
	}
	return failure
}

// close rolls back a creation that neither completed nor failed through its own rollback, as
// when a panic unwinds it.
func (g *pendingGuard) close() {
	if !g.armed {
		return
	}
	cause := g.rollback(runtimeError("pending creation was interrupted"), shutdownDeadlinesStartingNow(g.clock), nil)
	fmt.Fprintln(os.Stderr, cafleetErrorOf(cause).message())
}

func createRegistered(ctx context.Context, conn *dbConn, member pendingMemberSpec, settings runSettings,
	prepare func(context.Context, *dbConn, pendingRegistration) (*runArtifacts, spawnSpec, error), host launchHost, deadline time.Time) (launchOutcome, error) {
	pending, err := registerPendingRun(ctx, conn, member, settings, deadline)
	if err != nil {
		return launchOutcome{}, err
	}
	guard := &pendingGuard{ctx: ctx, conn: conn, run: pending.run, armed: true, clock: host.clock()}
	defer guard.close()
	artifacts, spawn, err := prepare(ctx, conn, pending)
	if err != nil {
		return launchOutcome{}, guard.rollback(err, shutdownDeadlinesStartingNow(host.clock()), nil)
	}
	return launchRegistered(guard, pending, member.windowName, artifacts, spawn, host, deadline)
}

func launchRegistered(guard *pendingGuard, pending pendingRegistration, windowName string, artifacts *runArtifacts, spawn spawnSpec, host launchHost, deadline time.Time) (launchOutcome, error) {
	ctx, conn := guard.ctx, guard.conn
	pane := none[spawnedPane]()
	var unwind any
	unwound := false
	result, primary := func() (result launchAttempt, err error) {
		defer func() {
			if payload := recover(); payload != nil {
				unwind, unwound = payload, true
				err = runtimeError("native launch unwound; performing owned compensation")
			}
		}()
		return attemptLaunch(ctx, conn, pending, windowName, artifacts, spawn, host, deadline, &pane)
	}()
	if primary == nil {
		guard.armed = false
		return launchOutcome{member: pending.member, run: result.run, placement: result.placement, started: result.started}, nil
	}
	deadlines := shutdownDeadlinesStartingNow(host.clock())
	var notes []string
	current, err := getRun(ctx, conn, pending.run.runID)
	switch {
	case err != nil:
		notes = append(notes, cafleetErrorOf(err).message())
	case current.ownerKind == ownerKindRunner:
		if spawned, set := pane.get(); set && current.backend == codingAgentHermes.name() && current.childPID.set {
			until := settlementDeadline().instant()
			note(&notes, struct{}{}, host.settleHermes(ctx, conn, current, spawned.paneID, until))
		}
		outcome, ok := guarded(&notes, "owned child cleanup panicked; explicit reconciliation required", func() (shutdownOutcome, error) {
			return host.shutdownOwned(current, deadlines)
		})
		if ok {
			notes = append(notes, outcome.diagnostics...)
			if outcome.cleanupState == cleanupStateIncomplete && len(notes) == 0 {
				notes = append(notes, "owned child cleanup remains unconfirmed")
			}
		}
	}
	if spawned, set := pane.get(); set {
		guarded(&notes, "owned pane cleanup panicked; explicit reconciliation required", func() (struct{}, error) {
			return struct{}{}, host.closePane(spawned, deadlines.storageDeadline())
		})
	}
	failure := guard.rollback(primary, deadlines, notes)
	if unwound {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(failure).message())
		panic(unwind)
	}
	return launchOutcome{}, failure
}

// launchAttempt is a launch whose native child started.
type launchAttempt struct {
	run       runRecord
	placement placement
	started   started
}

// attemptLaunch writes the descriptor, spawns the runner pane, publishes the prepared placement
// and starts the native child, recording the spawned pane in pane for compensation.
func attemptLaunch(ctx context.Context, conn *dbConn, pending pendingRegistration, windowName string, artifacts *runArtifacts, spawn spawnSpec, host launchHost, deadline time.Time, pane *opt[spawnedPane]) (launchAttempt, error) {
	type attempted = launchAttempt
	if err := runtimeCheckDeadline(host.clock().now(), deadline); err != nil {
		return attempted{}, err
	}
	run, err := getRun(ctx, conn, pending.run.runID)
	if err != nil {
		return attempted{}, err
	}
	if err := ensure(pending.paths.runDir == artifacts.paths.runDir, "prepared artifacts belong to a different pending run"); err != nil {
		return attempted{}, err
	}
	if _, err := validateRunPaths(ctx, conn, run); err != nil {
		return attempted{}, err
	}
	descriptor, err := newLaunchDescriptor(run, spawn)
	if err != nil {
		return attempted{}, err
	}
	exists, err := pathExists(artifacts.paths.descriptor)
	if err != nil {
		return attempted{}, ioError(ioErrorText(err))
	}
	if exists {
		existing, err := loadDescriptor(ctx, conn, artifacts.paths.descriptor)
		if err != nil {
			return attempted{}, err
		}
		if err := ensure(jsonEqual(existing.encode(), descriptor.encode()), "pending launch descriptor differs from prepared inputs"); err != nil {
			return attempted{}, err
		}
	} else if err := artifacts.writeDescriptor(descriptor); err != nil {
		return attempted{}, err
	}
	executable, err := host.currentExecutable()
	if err != nil {
		return attempted{}, err
	}
	if err := ensure(filepath.IsAbs(executable), "absolute CAFleet executable required"); err != nil {
		return attempted{}, err
	}
	if !utf8.ValidString(executable) {
		return attempted{}, runtimeError("CAFleet executable requires UTF-8 path")
	}
	canonical, present := jsonLookup(run.databaseIdentityJSON, "canonical_path")
	canonicalPath, isString := jsonString(canonical)
	if !present || !isString {
		panic("validated database path")
	}
	if !utf8.ValidString(artifacts.paths.descriptor) {
		return attempted{}, runtimeError("descriptor requires UTF-8 path")
	}
	argv := []string{executable, "--database-url", "sqlite:///" + canonicalPath, "__member-run", "--descriptor", artifacts.paths.descriptor}
	spawned, err := host.splitRunner(memberPaneName(pending.member.memberID, pending.member.name), argv, deadline)
	if err != nil {
		return attempted{}, err
	}
	*pane = some(spawned)
	if err := runtimeCheckDeadline(host.clock().now(), deadline); err != nil {
		return attempted{}, err
	}
	ready, err := host.waitPrepared(descriptor, deadline)
	if err != nil {
		return attempted{}, err
	}
	if err := runtimeCheckDeadline(host.clock().now(), deadline); err != nil {
		return attempted{}, err
	}
	readRun := func(ctx context.Context, conn *dbConn) (runRecord, error) { return getRun(ctx, conn, run.runID) }
	current, err := withDatabaseDeadline(ctx, conn, deadline, readRun)
	if err != nil {
		return attempted{}, err
	}
	if err := ready.Identity.validate(current); err != nil {
		return attempted{}, err
	}
	if run.ownerEpoch == math.MaxInt64 {
		return attempted{}, runtimeError("owner epoch exhausted")
	}
	if err := ensure(current.ownerEpoch == run.ownerEpoch+1 && current.ownerKind == ownerKindRunner &&
		current.ownerPID == ready.Runner.pid && current.ownerStartIdentity == ready.Runner.startIdentity &&
		!current.childPID.set, "prepared owner or child state mismatch"); err != nil {
		return attempted{}, err
	}
	published, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (placement, error) {
		return publishPreparedPlacement(ctx, conn, run, current, spawned, windowName, host.clock())
	})
	if err != nil {
		return attempted{}, err
	}
	begun, err := host.start(ready, published, deadline)
	if err != nil {
		return attempted{}, err
	}
	if err := runtimeCheckDeadline(host.clock().now(), deadline); err != nil {
		return attempted{}, err
	}
	current, err = withDatabaseDeadline(ctx, conn, deadline, readRun)
	if err != nil {
		return attempted{}, err
	}
	if err := begun.Identity.validate(current); err != nil {
		return attempted{}, err
	}
	statusStarted := current.status == runStatusRunning || current.status == runStatusDegraded || current.status == runStatusExited
	if err := ensure(begun.Runner == ready.Runner &&
		current.runnerPID == some(begun.Runner.pid) && current.runnerStartIdentity == some(begun.Runner.startIdentity) &&
		current.childPID == some(begun.Child.pid) && current.childStartIdentity == some(begun.Child.startIdentity) &&
		current.startedAt.set && statusStarted, "started reply lacks a matching completed native spawn"); err != nil {
		return attempted{}, err
	}
	if err := artifacts.removeDescriptor(); err != nil {
		return attempted{}, err
	}
	return attempted{run: current, placement: published, started: begun}, nil
}

// publishPreparedPlacement marks the run prepared and records the spawned pane on the pending
// placement in one transaction, then reads the published placement back.
func publishPreparedPlacement(ctx context.Context, conn *dbConn, run, current runRecord, spawned spawnedPane, windowName string, c clock) (placement, error) {
	if _, err := validateRunPaths(ctx, conn, current); err != nil {
		return placement{}, err
	}
	if _, err := inTransaction(conn, transactionImmediate, func() (struct{}, error) {
		fleet, err := activeMemberFleet(ctx, conn, run.memberID)
		if err != nil {
			return struct{}{}, err
		}
		if err := ensure(fleet == some(run.fleetID), "pending member became inactive"); err != nil {
			return struct{}{}, err
		}
		changed, err := conn.execute("UPDATE member_runs SET status='prepared',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='starting'",
			wallStamp(c), run.runID, current.ownerEpoch)
		if err != nil {
			return struct{}{}, err
		}
		if err := ensure(changed == 1, "prepared publication lost run ownership"); err != nil {
			return struct{}{}, err
		}
		changed, err = conn.execute(`UPDATE member_placements SET mux_pane_id=?1, mux_window_id=?2, mux_window_name=?3
                 WHERE member_id=?4 AND mux_pane_id IS NULL`, spawned.paneID, spawned.windowID, windowName, run.memberID)
		if err != nil {
			return struct{}{}, err
		}
		return struct{}{}, ensure(changed == 1, "pending placement vanished or changed before start")
	}); err != nil {
		return placement{}, err
	}
	member, err := getMember(ctx, conn, run.memberID, run.fleetID)
	if err != nil {
		return placement{}, err
	}
	record, found := member.get()
	if !found {
		return placement{}, runtimeError("published placement vanished")
	}
	return record.placement, nil
}

// nativeLaunchHost spawns the runner pane at target, starting in cwd, named by each splitRunner
// call. close releases the control connection a waitPrepared opened.
type nativeLaunchHost struct {
	mux         multiplexer
	target      paneTarget
	cwd         string
	client      *controlClient
	clockSource clock
}

func newNativeLaunchHost(mux multiplexer, target paneTarget, cwd string, c clock) *nativeLaunchHost {
	return &nativeLaunchHost{mux: mux, target: target, cwd: cwd, clockSource: c}
}

func (h *nativeLaunchHost) close() error {
	if h.client == nil {
		return nil
	}
	client := h.client
	h.client = nil
	return client.close()
}

func (h *nativeLaunchHost) clock() clock { return h.clockSource }

func (h *nativeLaunchHost) currentExecutable() (string, error) {
	executable, err := os.Executable()
	if err != nil {
		return "", ioError(ioErrorText(err))
	}
	return executable, nil
}

func (h *nativeLaunchHost) splitRunner(paneName string, argv []string, deadline time.Time) (spawnedPane, error) {
	spawned, err := h.mux.bounded(deadlineUntil(deadline)).spawnMemberPane(h.target, paneName, h.cwd, argv)
	if err != nil {
		return spawnedPane{}, runtimeError("runner pane spawn failed: " + err.Error())
	}
	return spawned, nil
}

func (h *nativeLaunchHost) waitPrepared(descriptor launchDescriptor, deadline time.Time) (prepared, error) {
	paths, err := runtimePathsForRun(descriptor.Identity.DatabaseIdentityJSON, descriptor.Identity.RunID)
	if err != nil {
		return prepared{}, err
	}
	client, err := controlClientConnectStarting(paths.controlSocket, deadline, h.clockSource)
	if err != nil {
		return prepared{}, err
	}
	remaining := min(max(deadline.Sub(h.clockSource.now()), 0).Milliseconds(), 10000)
	if remaining < 0 {
		panic("the remaining prepare budget is clamped to zero or more")
	}
	request := lifecyclePrepare{ProtocolVersion: controlProtocolVersion, Identity: descriptor.Identity, RemainingMS: uint64(remaining)}
	result, err := client.exchange(request, deadline, h.clockSource)
	if err != nil {
		return prepared{}, mergeCleanup(err, client.close())
	}
	var reply prepared
	if err := decodeStrict(result, &reply); err != nil {
		return prepared{}, mergeCleanup(ioError(cafleetErrorOf(err).message()), client.close())
	}
	if err := h.close(); err != nil {
		return prepared{}, mergeCleanup(err, client.close())
	}
	h.client = client
	return reply, nil
}

func (h *nativeLaunchHost) start(ready prepared, p placement, deadline time.Time) (started, error) {
	if h.client == nil {
		return started{}, runtimeError("prepared control connection required")
	}
	pane, set := p.muxPaneID.get()
	if !set {
		panic("published placement carries the created pane")
	}
	value, err := h.client.exchange(lifecycleStart{ProtocolVersion: controlProtocolVersion, Identity: ready.Identity, PaneID: pane}, deadline, h.clockSource)
	if err != nil {
		return started{}, err
	}
	var reply started
	if err := decodeStrict(value, &reply); err != nil {
		return started{}, ioError(cafleetErrorOf(err).message())
	}
	return reply, nil
}

func (h *nativeLaunchHost) settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, deadline time.Time) error {
	return settleRun(ctx, conn, h.mux, run, paneID, deadlineUntil(deadline))
}

func (h *nativeLaunchHost) shutdownOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error) {
	if value, err := runtimeControlRequest(run, lifecycleRequestShutdown(run), deadlines.storageDeadline(), h.clockSource); err == nil {
		return shutdownOutcomeDecode(value)
	}
	runner := run.ownerProcess()
	matches, err := runner.matches()
	if err != nil {
		return shutdownOutcome{}, err
	}
	if matches {
		pid, err := pidGroup(runner.pid)
		if err != nil {
			return shutdownOutcome{}, err
		}
		if err := syscall.Kill(int(pid), syscall.SIGTERM); err != nil {
			return shutdownOutcome{}, ioError(errnoText(err))
		}
		for {
			matches, err := runner.matches()
			if err != nil {
				return shutdownOutcome{}, err
			}
			if !matches || !h.clockSource.now().Before(deadlines.storageDeadline()) {
				break
			}
			if err := pause(deadlines.storageDeadline(), h.clockSource); err != nil {
				return shutdownOutcome{}, err
			}
		}
	}
	notes := []string{"runner control shutdown unavailable; descendant cleanup requires explicit reconciliation"}
	matches, err = runner.matches()
	if err != nil {
		return shutdownOutcome{}, err
	}
	if matches {
		notes = append(notes, "owned runner termination remains unconfirmed")
	}
	return newShutdownOutcome(none[childExit](), notes), nil
}

func (h *nativeLaunchHost) closePane(pane spawnedPane, deadline time.Time) error {
	if err := killPane(h.mux.bounded(deadlineUntil(deadline)), pane.paneID, pane.windowID); err != nil {
		return runtimeError("runner pane cleanup failed: " + err.Error())
	}
	return nil
}
