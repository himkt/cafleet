package core

import (
	"cmp"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"os"
	"slices"
	"strconv"
	"strings"
	"time"
)

func fleetRun(ctx context.Context, conn *dbConn, command fleetCommand) error {
	switch command := command.(type) {
	case fleetCommandList:
		fleets, err := listFleets(ctx, conn)
		if err != nil {
			return err
		}
		payload := make([]jsonObject, 0, len(fleets))
		for _, fleet := range fleets {
			payload = append(payload, outputJSONFleetSummary(fleet))
		}
		return emit(command.json, payload, func() string { return formatFleetList(fleets) })
	case fleetCommandShow:
		fleet, err := requireFleet(ctx, conn, command.fleetID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONFleet(fleet), func() string { return formatFleetShow(fleet) })
	case fleetCommandCreate, fleetCommandDelete:
		return commandNotYetPorted()
	default:
		panic("invalid fleetCommand")
	}
}

type fleetCreateRequest struct {
	name      string
	agent     codingAgent
	pane      opt[string]
	muxServer opt[string]
}

func fleetCreate(ctx context.Context, conn *dbConn, request fleetCreateRequest, json bool) error {
	env, err := callerEnv()
	if err != nil {
		return err
	}
	fleet, err := createWith(ctx, conn, request, env, systemRunner{}, systemProcessTable{})
	if err != nil {
		return err
	}
	return emit(json, outputJSONCreatedFleet(fleet), func() string { return formatFleetCreate(fleet) })
}

func cliFleetRefusal(body string) error {
	return appError("cafleet fleet create " + body)
}

func multiplexerContextDebug(c multiplexerContext) string {
	return "MultiplexerContext { session: " + quoted(c.session) + ", window_id: " + quoted(c.windowID) + ", pane_id: " + quoted(c.paneID) + " }"
}

func optionalMultiplexerContextDebug(c opt[multiplexerContext]) string {
	if value, set := c.get(); set {
		return "Some(" + multiplexerContextDebug(value) + ")"
	}
	return "None"
}

// createWith registers the fleet with every external read injected.
func createWith(ctx context.Context, conn *dbConn, request fleetCreateRequest, env map[string]string, runner commandRunner, table processTable) (createdFleet, error) {
	var target callerMux
	var pane string
	if explicit, set := request.pane.get(); set {
		resolved, err := resolvePaneMux(explicit, request.muxServer, env, runner)
		if err != nil {
			return createdFleet{}, err
		}
		target, pane = resolved, explicit
	} else {
		resolved, verified, err := verifiedEnvPane(env, runner, table)
		if err != nil {
			return createdFleet{}, err
		}
		target, pane = resolved, verified
	}
	found, muxErr := target.mux.paneContext(pane)
	if muxErr != nil {
		return createdFleet{}, muxErr.cafleet()
	}
	paneContext, exists := found.get()
	if !exists {
		return createdFleet{}, appErrorf("pane %s was not found on the %s server %s; pass --pane and --mux-server for the Director's pane",
			pane, target.backend.name(), target.server)
	}
	attemptedFleetID := none[int64]()
	fleet, err := createFleetWith(ctx, conn, request.name, func(fleetID int64) (newPlacement, error) {
		windowName := fleetWindowName(fleetID, 0)
		attemptedFleetID = some(fleetID)
		if err := target.mux.renameWindow(paneContext.windowID, windowName); err != nil {
			return newPlacement{}, err.cafleet()
		}
		actualName, muxErr := target.mux.windowName(paneContext.windowID)
		if muxErr != nil {
			return newPlacement{}, muxErr.cafleet()
		}
		if actualName != windowName {
			return newPlacement{}, appErrorf("Director window name verification failed: expected %s, found %s", quoted(windowName), quoted(actualName))
		}
		actualContext, muxErr := target.mux.paneContext(pane)
		if muxErr != nil {
			return newPlacement{}, muxErr.cafleet()
		}
		if actualContext != some(paneContext) {
			return newPlacement{}, appErrorf("Director pane context changed during window naming: expected %s, found %s",
				multiplexerContextDebug(paneContext), optionalMultiplexerContextDebug(actualContext))
		}
		return newPlacementDirector(target.backend.name(), paneContext.session, request.agent.name(), target.server, placementPane{
			muxWindowID:   paneContext.windowID,
			muxWindowName: windowName,
			muxPaneID:     paneContext.paneID,
		}), nil
	})
	if err == nil {
		return fleet, nil
	}
	fleetID, attempted := attemptedFleetID.get()
	if !attempted {
		return createdFleet{}, err
	}
	var inspection string
	switch target.backend {
	case muxBackendTmux:
		inspection = fmt.Sprintf("tmux -S %s display-message -p -t %s '#{window_id}|#{window_name}'", quoted(target.server), quoted(paneContext.windowID))
	case muxBackendHerdr:
		inspection = fmt.Sprintf("herdr --session %s tab get %s", quoted(target.server), quoted(paneContext.windowID))
	default:
		panic("invalid muxBackend")
	}
	return createdFleet{}, cafleetErrorOf(err).withDiagnostic(fmt.Sprintf(
		"fleet create naming attempt: fleet_id=%d backend=%s server=%s window_id=%s pane_id=%s requested_name=%s; the terminal name may have changed. Inspect 'cafleet fleet list' and `%s`. The attempted fleet ID can be reused after rollback.",
		fleetID, target.backend.name(), quoted(target.server), quoted(paneContext.windowID), quoted(paneContext.paneID),
		quoted(fleetWindowName(fleetID, 0)), inspection))
}

// verifiedEnvPane is steps 2-6 of `fleet create` without --pane: the caller's multiplexer and its
// verified pane.
func verifiedEnvPane(env map[string]string, runner commandRunner, table processTable) (callerMux, string, error) {
	ancestry := walk(table)
	shared, err := fleetCreateSharedServer(ancestry)
	if err != nil {
		return callerMux{}, "", err
	}
	if server, found := shared.get(); found {
		return callerMux{}, "", cliFleetRefusal(sharedServerMessage(server))
	}
	backend, detectErr := muxBackendDetect(lookup(env))
	if detectErr != nil {
		return callerMux{}, "", appError("cafleet fleet create must be run inside a tmux or herdr session")
	}
	caller, set := envPane(backend, env).get()
	if !set {
		return callerMux{}, "", cliFleetRefusal(unsetMessage(backend))
	}
	mux, err := resolveEnvMux(env, runner)
	if err != nil {
		return callerMux{}, "", err
	}
	verdict, err := verifyPane(mux.mux, table, ancestry, caller.pane)
	if err != nil {
		return callerMux{}, "", err
	}
	switch verdict := verdict.(type) {
	case verificationVerified:
		return mux, caller.pane, nil
	case verificationRefused:
		return callerMux{}, "", cliFleetRefusal(unverifiedMessage(caller, verdict.refusal, mux))
	default:
		panic("invalid verification")
	}
}

// fleetCreateSharedServer funnels row 1 of caller verification until sharedServerOf reaches
// this branch with the next main merge.
func fleetCreateSharedServer(ancestry) (opt[sharedServer], error) {
	return none[sharedServer](), commandNotYetPorted()
}

type deletionPlacement struct {
	memberID  int64
	placement placement
	path      string
	// runIdentity is the member's stable run identity; nil when the member has no run.
	runIdentity jsontext.Value
}

func (d deletionPlacement) equal(other deletionPlacement) bool {
	return d.memberID == other.memberID && d.placement == other.placement && d.path == other.path &&
		sameRunIdentity(d.runIdentity, other.runIdentity)
}

func sameRunIdentity(left, right jsontext.Value) bool {
	return (left == nil) == (right == nil) && (left == nil || jsonEqual(left, right))
}

func deletionPlacements(ctx context.Context, conn *dbConn, s settings, fleetID int64) ([]deletionPlacement, error) {
	type row struct {
		memberID  int64
		placement placement
	}
	rows, err := queryRows(conn, "SELECT m.member_id, "+placementColumns+`
             FROM members m JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL`,
		[]any{fleetID}, func(r sqlRow) (row, error) {
			memberID, err := r.int64(0)
			if err != nil {
				return row{}, err
			}
			p, err := placementFromRow(r, 1)
			return row{memberID: memberID, placement: p}, err
		})
	if err != nil {
		return nil, err
	}
	placements := make([]deletionPlacement, 0, len(rows))
	for _, entry := range rows {
		path, err := lockPath(s.databaseURL, entry.placement)
		if err != nil {
			return nil, memberPaneInputError(entry.memberID)(err)
		}
		runtime, err := memberRuntime(ctx, conn, entry.memberID)
		if err != nil {
			return nil, err
		}
		placements = append(placements, deletionPlacement{memberID: entry.memberID, placement: entry.placement, path: path, runIdentity: optionalRunIdentity(runtime)})
	}
	slices.SortFunc(placements, func(left, right deletionPlacement) int {
		if order := strings.Compare(left.path, right.path); order != 0 {
			return order
		}
		return cmp.Compare(left.memberID, right.memberID)
	})
	return placements, nil
}

const pendingPaneStatus = "(pending — no pane)"

// deletionHost runs the native steps of ending a member. Embed runtimeDeletionSteps for the
// runtime's own recovery and shutdown.
type deletionHost interface {
	recover(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, held *paneLock, d deadline) (recoveryCompletion, error)
	shutdown(ctx context.Context, conn *dbConn, run runRecord, d deadline) (shutdownOutcome, error)
	settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, d deadline) error
	closePane(p placement, d deadline) error
	// restoreLayout restores the layout after the preceding closePane.
	restoreLayout() error
}

type runtimeDeletionSteps struct{}

func (runtimeDeletionSteps) recover(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, held *paneLock, d deadline) (recoveryCompletion, error) {
	return recoverMemberUnderPaneLock(ctx, conn, s, expectedRun, expectedPlacement, d.instant(), held)
}

func (runtimeDeletionSteps) shutdown(ctx context.Context, conn *dbConn, run runRecord, d deadline) (shutdownOutcome, error) {
	return deletionRequestShutdown(ctx, conn, run, d.instant())
}

// deletionRequestShutdown funnels the runner shutdown request until requestShutdown lands in
// runtime_control.go.
func deletionRequestShutdown(context.Context, *dbConn, runRecord, time.Time) (shutdownOutcome, error) {
	return shutdownOutcome{}, commandNotYetPorted()
}

// multiplexerDeletionHost runs each deletion step through mux bounded by that step's deadline, so
// a Hermes settlement's budget never shortens the pane close that follows it.
type multiplexerDeletionHost struct {
	runtimeDeletionSteps
	mux multiplexer
	// closing is the bounded multiplexer of the last pane close; layout restoration reads the
	// window it recorded.
	closing multiplexer
}

func newMultiplexerDeletionHost(mux multiplexer) *multiplexerDeletionHost {
	return &multiplexerDeletionHost{mux: mux}
}

func (h *multiplexerDeletionHost) settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, d deadline) error {
	return settleRun(ctx, conn, h.mux.bounded(d), run, paneID, d)
}

func (h *multiplexerDeletionHost) closePane(p placement, d deadline) error {
	pane, concrete := p.muxPaneID.get()
	if !concrete {
		panic("deletion target has a concrete pane")
	}
	h.closing = h.mux.bounded(d)
	if err := h.closing.closePane(pane, p.concreteWindowID()); err != nil {
		return err.cafleet()
	}
	return nil
}

func (h *multiplexerDeletionHost) restoreLayout() error {
	if h.closing == nil {
		panic("layout restoration follows a pane close")
	}
	if err := h.closing.restoreAfterClose(); err != nil {
		return err.cafleet()
	}
	return nil
}

type nativeFleetDeletionHost struct {
	runtimeDeletionSteps
	// director is read before deletion removes the placement rows; directorErr surfaces only
	// when a pane actually needs the multiplexer.
	director    placement
	directorErr error
	host        *multiplexerDeletionHost
}

func (h *nativeFleetDeletionHost) resolved() (*multiplexerDeletionHost, error) {
	if h.host == nil {
		if h.directorErr != nil {
			return nil, h.directorErr
		}
		mux, err := muxForPlacement(h.director, none[deadline]())
		if err != nil {
			return nil, err
		}
		h.host = newMultiplexerDeletionHost(mux)
	}
	return h.host, nil
}

func (h *nativeFleetDeletionHost) settleHermes(ctx context.Context, conn *dbConn, run runRecord, paneID string, d deadline) error {
	host, err := h.resolved()
	if err != nil {
		return err
	}
	return host.settleHermes(ctx, conn, run, paneID, d)
}

func (h *nativeFleetDeletionHost) closePane(p placement, d deadline) error {
	host, err := h.resolved()
	if err != nil {
		return err
	}
	return host.closePane(p, d)
}

func (h *nativeFleetDeletionHost) restoreLayout() error {
	host, err := h.resolved()
	if err != nil {
		return err
	}
	return host.restoreLayout()
}

// optionalRunIdentity is the stable identity of runtime's run, or nil without a runtime.
func optionalRunIdentity(runtime opt[*runtimeSnapshot]) jsontext.Value {
	if snapshot, set := runtime.get(); set {
		return stableRunIdentity(snapshot.run)
	}
	return nil
}

func memberDeletionMatches(expected, current memberRecord) bool {
	return expected.memberID == current.memberID && expected.fleetID == current.fleetID &&
		expected.kind == current.kind && expected.placement == current.placement &&
		sameRunIdentity(optionalRunIdentity(expected.runtime), optionalRunIdentity(current.runtime))
}

func revalidateMember(ctx context.Context, conn *dbConn, expected memberRecord, stage string) error {
	current, err := activeMember(ctx, conn, expected.memberID)
	if err != nil {
		return err
	}
	if !memberDeletionMatches(expected, current) {
		return appError("member placement or runtime ownership changed " + stage)
	}
	return nil
}

// awaitRunnerExit waits within d for the runner to finish once a settlement has ended its client:
// until it exits or settles the run it still holds the run, and recovery would read the member
// as live.
func awaitRunnerExit(ctx context.Context, conn *dbConn, runID int64, d deadline) error {
	for {
		run, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (runRecord, error) {
			return getRun(ctx, conn, runID)
		})
		if err != nil {
			return err
		}
		if !run.status.terminal() || run.isSettled() {
			return nil
		}
		alive, err := run.ownerProcess().isAlive()
		if err != nil {
			return err
		}
		if !alive {
			return nil
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// endMember ends one member under its held pane lock and the fleet layout lock the caller holds
// for the whole call. scope revalidates the caller's wider deletion state, such as a fleet's
// placement set, before each irreversible step up to the pane close.
func endMember(ctx context.Context, conn *dbConn, s settings, snapshot *memberRecord, held *paneLock, _ *layoutLock, host deletionHost, d *deadline, scope func(context.Context, *dbConn) error) (string, error) {
	memberID := snapshot.memberID
	p := snapshot.placement
	paneID, concrete := p.muxPaneID.get()
	if !concrete {
		panic("an ended member has a concrete pane")
	}
	type hermesTarget struct {
		run   runRecord
		found bool
	}
	hermesRun, err := withDatabaseBudget(ctx, conn, *d, func(ctx context.Context, conn *dbConn) (hermesTarget, error) {
		if err := revalidateMember(ctx, conn, *snapshot, "before cleanup"); err != nil {
			return hermesTarget{}, err
		}
		if err := scope(ctx, conn); err != nil {
			return hermesTarget{}, err
		}
		runtime, err := memberRuntime(ctx, conn, memberID)
		if err != nil {
			return hermesTarget{}, err
		}
		if recorded, set := runtime.get(); set && recorded.run.backend == codingAgentHermes.name() {
			return hermesTarget{run: recorded.run, found: true}, nil
		}
		return hermesTarget{}, nil
	})
	if err != nil {
		return "", err
	}
	if run := hermesRun.run; hermesRun.found {
		// Settlement has its own budget; the time it takes is added to the deletion deadline.
		started := time.Now()
		if err := host.settleHermes(ctx, conn, run, paneID, settlementDeadline()); err != nil {
			return "", err
		}
		*d = deadlineUntil(d.instant().Add(time.Since(started)))
		if err := awaitRunnerExit(ctx, conn, run.runID, *d); err != nil {
			return "", err
		}
	}
	bound := *d
	if _, err := withDatabaseBudget(ctx, conn, bound, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, cleanUpMember(ctx, conn, s, snapshot, held, host, bound, scope)
	}); err != nil {
		return "", err
	}
	if _, err := bound.remaining(); err != nil {
		return "", err
	}
	if err := host.closePane(p, bound); err != nil {
		return "", appErrorf("kill_pane failed for pane %s: %s. Verify with 'cafleet doctor'.", paneID, cafleetErrorOf(err).message())
	}
	if _, err := completeDatabaseBudget(ctx, conn, bound, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return inTransaction(conn, transactionImmediate, func() (struct{}, error) {
			if err := revalidateMember(ctx, conn, *snapshot, "after pane close"); err != nil {
				return struct{}{}, err
			}
			changed, err := deregisterMemberRows(ctx, conn, memberID)
			if err != nil {
				return struct{}{}, err
			}
			if !changed {
				panic("the input-locked member stays active until deregistration")
			}
			return struct{}{}, nil
		})
	}); err != nil {
		return "", appErrorf("pane %s was closed, but deregister failed: %s", paneID, cafleetErrorOf(err).message())
	}
	if err := host.restoreLayout(); err != nil {
		fmt.Fprintf(os.Stderr, "Member %d was deregistered; optional layout restoration failed: %s\n", memberID, cafleetErrorOf(err).message())
	}
	return paneID + " (killed)", nil
}

// cleanUpMember stops a live runner, recovers the captured runtime and revalidates the member
// before its pane closes.
func cleanUpMember(ctx context.Context, conn *dbConn, s settings, snapshot *memberRecord, held *paneLock, host deletionHost, d deadline, scope func(context.Context, *dbConn) error) error {
	memberID := snapshot.memberID
	if err := revalidateMember(ctx, conn, *snapshot, "before cleanup"); err != nil {
		return err
	}
	if err := scope(ctx, conn); err != nil {
		return err
	}
	runtime, err := memberRuntime(ctx, conn, memberID)
	if err != nil {
		return err
	}
	if recorded, set := runtime.get(); set && !recorded.run.status.terminal() {
		alive, err := recorded.run.ownerProcess().isAlive()
		if err != nil {
			return err
		}
		if alive {
			outcome, err := host.shutdown(ctx, conn, recorded.run, d)
			if err != nil {
				return err
			}
			if outcome.cleanupState != cleanupStateComplete {
				return appErrorf("member %d child cleanup remains unconfirmed: %s; inspect with cafleet doctor", memberID, strings.Join(outcome.diagnostics, "; "))
			}
		}
	}
	if captured, set := snapshot.runtime.get(); set {
		expected := captured.run
		if err := scope(ctx, conn); err != nil {
			return err
		}
		current, err := getRun(ctx, conn, expected.runID)
		if err != nil {
			return err
		}
		if !jsonEqual(stableRunIdentity(current), stableRunIdentity(expected)) {
			return appError("member runtime ownership changed during cleanup")
		}
		if _, err := d.remaining(); err != nil {
			return err
		}
		completion, err := host.recover(ctx, conn, s, current, snapshot.placement, held, d)
		if err != nil {
			return err
		}
		if err := validateRecoveryCompletion(expected, completion); err != nil {
			return err
		}
		if completion.report.outcome != recoveryOutcomeRecovered {
			return appError("member recovery remains incomplete; inspect with cafleet doctor")
		}
		updated := *captured
		updated.run = completion.run
		snapshot.runtime = some(&updated)
	}
	if err := revalidateMember(ctx, conn, *snapshot, "before pane close"); err != nil {
		return err
	}
	return scope(ctx, conn)
}

// acquireDeletionLayout acquires the fleet layout lock for a deletion of subject ("member" or
// "fleet"), moving the deletion deadline later by the wait.
func acquireDeletionLayout(s settings, fleetID int64, d deadline, layoutWait time.Duration, subject string) (*layoutLock, deadline, error) {
	started := time.Now()
	lock, acquired, err := layoutLockAcquireWithin(s.databaseURL, fleetID, deadlineUntil(started.Add(layoutWait)))
	if err != nil {
		return nil, deadline{}, err
	}
	if !acquired {
		return nil, deadline{}, appErrorf("fleet %d pane layout remained busy for 60 seconds; %s was not deleted.", fleetID, subject)
	}
	return lock, deadlineUntil(d.instant().Add(time.Since(started))), nil
}

func deleteFleet(ctx context.Context, conn *dbConn, s settings, fleetID int64, d deadline) (deletedFleet, error) {
	director, directorErr := directorPlacement(ctx, conn, fleetID)
	return deleteFleetWithHost(ctx, conn, s, fleetID, d, &nativeFleetDeletionHost{director: director, directorErr: directorErr})
}

func deleteFleetWithHost(ctx context.Context, conn *dbConn, s settings, fleetID int64, d deadline, host deletionHost) (deletedFleet, error) {
	return deleteFleetWithLayoutWait(ctx, conn, s, fleetID, d, layoutLockWait, host)
}

func deleteFleetWithLayoutWait(ctx context.Context, conn *dbConn, s settings, fleetID int64, d deadline, layoutWait time.Duration, host deletionHost) (deletedFleet, error) {
	if _, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (fleetRecord, error) {
		return requireFleet(ctx, conn, fleetID)
	}); err != nil {
		return deletedFleet{}, err
	}
	changedError := func() error {
		return appErrorf("fleet %d placement or runtime ownership changed during cleanup; fleet was not deleted.", fleetID)
	}
	locked, err := lockDeletion(ctx, conn, s, d,
		func(ctx context.Context, conn *dbConn) ([]deletionPlacement, []memberPlacement, error) {
			snapshot, err := deletionPlacements(ctx, conn, s, fleetID)
			if err != nil {
				return nil, nil, err
			}
			targets := make([]memberPlacement, 0, len(snapshot))
			for _, target := range snapshot {
				targets = append(targets, memberPlacement{memberID: target.memberID, placement: target.placement})
			}
			return snapshot, targets, nil
		},
		func(ctx context.Context, conn *dbConn, snapshot *[]deletionPlacement, _ []*paneLock) ([]memberRecord, bool, error) {
			current, err := deletionPlacements(ctx, conn, s, fleetID)
			if err != nil {
				return nil, false, err
			}
			if !slices.EqualFunc(current, *snapshot, deletionPlacement.equal) {
				return nil, false, nil
			}
			var members []memberRecord
			for _, target := range *snapshot {
				member, err := activeMember(ctx, conn, target.memberID)
				if err != nil {
					return nil, false, err
				}
				if member.kind != memberKindDirector {
					members = append(members, member)
				}
			}
			return members, true, nil
		},
		func(member int64) error {
			return appErrorf("fleet %d pane input remained busy for 5 seconds at member %d; fleet was not deleted.", fleetID, member)
		},
	)
	if err != nil {
		return deletedFleet{}, err
	}
	defer locked.close()
	layout, d, err := acquireDeletionLayout(s, fleetID, d, layoutWait, "fleet")
	if err != nil {
		return deletedFleet{}, err
	}
	defer layout.close()
	remaining := slices.Clone(locked.snapshot)
	members := locked.value
	locked.value = nil
	var ended []deletedMember
	for index := range members {
		member := &members[index]
		memberID := member.memberID
		held, err := locked.paneLock(s.databaseURL, member.placement)
		if err != nil {
			return deletedFleet{}, err
		}
		scopeChanged := false
		scope := func(ctx context.Context, conn *dbConn) error {
			current, err := deletionPlacements(ctx, conn, s, fleetID)
			if err != nil {
				return err
			}
			if samePlacements(current, remaining) {
				return nil
			}
			scopeChanged = true
			return changedError()
		}
		paneStatus, err := endMember(ctx, conn, s, member, held, layout, host, &d, scope)
		switch {
		case err == nil:
			ended = append(ended, deletedMember{memberID: memberID, paneStatus: paneStatus})
			remaining = slices.DeleteFunc(remaining, func(target deletionPlacement) bool { return target.memberID == memberID })
		case scopeChanged:
			return deletedFleet{}, err
		default:
			return deletedFleet{}, memberNotEnded(fleetID, memberID, err, ended)
		}
	}
	type finished struct {
		deregisteredCount int
		unplaced          []deletedMember
	}
	result, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (finished, error) {
		return inTransaction(conn, transactionImmediate, func() (finished, error) {
			current, err := deletionPlacements(ctx, conn, s, fleetID)
			if err != nil {
				return finished{}, err
			}
			if !samePlacements(current, remaining) {
				return finished{}, changedError()
			}
			unplaced, err := unplacedMemberOutcomes(conn, fleetID)
			if err != nil {
				return finished{}, err
			}
			count, err := deleteFleetRows(ctx, conn, fleetID)
			if err != nil {
				return finished{}, err
			}
			if _, err := d.remaining(); err != nil {
				return finished{}, err
			}
			return finished{deregisteredCount: count, unplaced: unplaced}, nil
		})
	})
	if err != nil {
		return deletedFleet{}, err
	}
	deregisteredCount := result.deregisteredCount + len(ended)
	ended = append(ended, result.unplaced...)
	slices.SortStableFunc(ended, func(left, right deletedMember) int { return cmp.Compare(left.memberID, right.memberID) })
	return deletedFleet{fleetID: fleetID, deregisteredCount: deregisteredCount, members: ended}, nil
}

func samePlacements(current, expected []deletionPlacement) bool {
	return slices.EqualFunc(current, expected, func(current, expected deletionPlacement) bool {
		return current.memberID == expected.memberID && current.placement == expected.placement
	})
}

func memberNotEnded(fleetID, memberID int64, cause error, ended []deletedMember) error {
	endedText := "none"
	if len(ended) > 0 {
		ids := make([]string, 0, len(ended))
		for _, member := range ended {
			ids = append(ids, strconv.FormatInt(member.memberID, 10))
		}
		endedText = strings.Join(ids, ", ")
	}
	return appErrorf("fleet %d was not deleted: member %d: %s. Members already deleted: %s.",
		fleetID, memberID, strings.TrimRight(cafleetErrorOf(cause).message(), "."), endedText)
}

func unplacedMemberOutcomes(conn *dbConn, fleetID int64) ([]deletedMember, error) {
	return queryRows(conn, `SELECT m.member_id
             FROM members m JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NULL
               AND m.member_id IS NOT (SELECT director_member_id FROM fleets WHERE fleet_id=?1)`,
		[]any{fleetID}, func(row sqlRow) (deletedMember, error) {
			memberID, err := row.int64(0)
			return deletedMember{memberID: memberID, paneStatus: pendingPaneStatus}, err
		})
}
