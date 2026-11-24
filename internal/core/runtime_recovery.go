package core

import (
	"context"
	"encoding/json/jsontext"
	"math"
	"os"
	"time"
)

type processEvidence uint8

const (
	processEvidenceAlive processEvidence = iota + 1
	processEvidenceTerminated
	processEvidenceUnknown
)

//sumtype:decl
type recoveryHealth interface{ isRecoveryHealth() }

type recoveryHealthMatching struct{}

type recoveryHealthUnavailable struct{ reason string }

func (recoveryHealthMatching) isRecoveryHealth()    {}
func (recoveryHealthUnavailable) isRecoveryHealth() {}

// recoveryBoundary names a point of recovery at which a test host may interrupt it.
type recoveryBoundary uint8

const (
	recoveryBoundaryAfterSnapshot recoveryBoundary = iota + 1
	recoveryBoundaryBeforeTakeover
	recoveryBoundaryAfterTakeover
	recoveryBoundaryAfterCommit
	recoveryBoundaryBeforeFinish
)

type recoveryHost interface {
	clock() clock
	currentProcess() (processIdentity, error)
	processEvidence(process processIdentity, deadline time.Time) (processEvidence, error)
	health(run runRecord, deadline time.Time) (recoveryHealth, error)
	cleanupOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error)
	unlinkDescriptor(path string, deadline time.Time) error
}

// recoveryCheckpointer is a recovery host that tests interrupt at each recoveryBoundary;
// production hosts do not implement it.
type recoveryCheckpointer interface {
	checkpoint(boundary recoveryBoundary, run runRecord, deadline time.Time) error
}

func checkpoint(host recoveryHost, boundary recoveryBoundary, run runRecord, deadline time.Time) error {
	if checkpointer, ok := host.(recoveryCheckpointer); ok {
		return checkpointer.checkpoint(boundary, run, deadline)
	}
	return nil
}

func unlinkDescriptorFile(path string, deadline time.Time, c clock) error {
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return err
	}
	if err := os.Remove(path); err != nil {
		return runtimeError("descriptor unlink failed: " + ioErrorText(err))
	}
	return nil
}

type recoveryOutcome uint8

const (
	recoveryOutcomeRecovered recoveryOutcome = iota + 1
	recoveryOutcomeRecovering
	recoveryOutcomeLiveOrUnreachable
)

type recoveryReport struct {
	memberID           int64
	runID              int64
	outcome            recoveryOutcome
	status             runStatus
	cleanupDiagnostics []jsontext.Value
}

type recoveryCompletion struct {
	report recoveryReport
	run    runRecord
}

func validateRecoveryCompletion(expected runRecord, completion recoveryCompletion) error {
	proven := expected
	proven.ownerEpoch = completion.run.ownerEpoch
	proven.ownerKind = completion.run.ownerKind
	proven.ownerPID = completion.run.ownerPID
	proven.ownerStartIdentity = completion.run.ownerStartIdentity
	advanced := expected.ownerEpoch != math.MaxInt64 && expected.ownerEpoch+1 == completion.run.ownerEpoch &&
		completion.run.ownerKind == ownerKindReconciler
	return ensure(
		jsonEqual(stableRunIdentity(proven), stableRunIdentity(completion.run)) &&
			completion.report.memberID == expected.memberID &&
			completion.report.runID == expected.runID &&
			completion.report.status == completion.run.status &&
			(completion.run.owner().equal(expected.owner()) || advanced),
		"recovery completion does not match the pinned deletion target",
	)
}

func stableRunIdentity(run runRecord) jsontext.Value {
	optionalJSONValue := func(value jsontext.Value) jsontext.Value {
		if value == nil {
			return jsonNull
		}
		return value
	}
	return jsonValue(jsonObject{
		{"database", run.databaseIdentityJSON},
		{"fleet_id", run.fleetID},
		{"member_id", run.memberID},
		{"director_member_id", run.directorMemberID},
		{"backend", run.backend},
		{"run_id", run.runID},
		{"nonce", run.launchNonce},
		{"epoch", run.ownerEpoch},
		{"owner_kind", run.ownerKind.asStr()},
		{"owner_pid", run.ownerPID},
		{"owner_start", run.ownerStartIdentity},
		{"runtime_path", run.runtimePath},
		{"directory", optionalJSONValue(run.runtimeDirIdentityJSON)},
		{"lock", optionalJSONValue(run.runLockIdentityJSON)},
		{"runner_pid", run.runnerPID},
		{"runner_start", run.runnerStartIdentity},
		{"child_pid", run.childPID},
		{"child_start", run.childStartIdentity},
	})
}

func diagnostics(run runRecord) ([]jsontext.Value, error) {
	if run.cleanupDiagnosticsJSON == nil {
		return []jsontext.Value{}, nil
	}
	items, ok := jsonArray(run.cleanupDiagnosticsJSON)
	if !ok {
		return nil, runtimeError("runtime cleanup diagnostics must be an array")
	}
	return items, nil
}

func completion(run runRecord, outcome recoveryOutcome) (recoveryCompletion, error) {
	items, err := diagnostics(run)
	if err != nil {
		return recoveryCompletion{}, err
	}
	return recoveryCompletion{
		report: recoveryReport{memberID: run.memberID, runID: run.runID, outcome: outcome, status: run.status, cleanupDiagnostics: items},
		run:    run,
	}, nil
}

type recoverySnapshot struct {
	run    runRecord
	pinned opt[placement]
}

func snapshot(ctx context.Context, conn *dbConn, memberID int64, until time.Time) (recoverySnapshot, error) {
	return withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (recoverySnapshot, error) {
		runtime, err := memberRuntime(ctx, conn, memberID)
		if err != nil {
			return recoverySnapshot{}, err
		}
		recorded, set := runtime.get()
		if !set {
			return recoverySnapshot{}, missingRun(memberID)
		}
		if err := ensureSameDatabase(ctx, conn, recorded.run.databaseIdentityJSON); err != nil {
			return recoverySnapshot{}, err
		}
		pinned, err := placementOf(ctx, conn, memberID)
		if err != nil {
			return recoverySnapshot{}, err
		}
		return recoverySnapshot{run: recorded.run, pinned: pinned}, nil
	})
}

func checkSnapshot(ctx context.Context, conn *dbConn, expected runRecord, expectedPlacement opt[placement]) (runRecord, error) {
	runtime, err := memberRuntime(ctx, conn, expected.memberID)
	if err != nil {
		return runRecord{}, err
	}
	recorded, set := runtime.get()
	if !set {
		return runRecord{}, runtimeError("recorded runtime disappeared during recovery")
	}
	if err := ensureSameDatabase(ctx, conn, expected.databaseIdentityJSON); err != nil {
		return runRecord{}, err
	}
	current, err := placementOf(ctx, conn, expected.memberID)
	if err != nil {
		return runRecord{}, err
	}
	if err := ensure(jsonEqual(stableRunIdentity(recorded.run), stableRunIdentity(expected)) && current == expectedPlacement,
		"runtime ownership or placement changed during recovery"); err != nil {
		return runRecord{}, err
	}
	return recorded.run, nil
}

func advanceRecoveryOwner(ctx context.Context, conn *dbConn, expected runRecord, process processIdentity) (runRecord, error) {
	current, err := validateOwner(ctx, conn, expected.owner())
	if err != nil {
		return runRecord{}, err
	}
	if err := ensureSameDatabase(ctx, conn, expected.databaseIdentityJSON); err != nil {
		return runRecord{}, err
	}
	if err := ensure(jsonEqual(stableRunIdentity(current), stableRunIdentity(expected)), "recovery ownership changed before CAS"); err != nil {
		return runRecord{}, err
	}
	if current.ownerEpoch == math.MaxInt64 {
		return runRecord{}, runtimeError("owner epoch exhausted")
	}
	changed, err := conn.execute(
		"UPDATE member_runs SET owner_epoch=?1,owner_kind='reconciler',owner_pid=?2,owner_start_identity=?3,recovery_origin_status=COALESCE(recovery_origin_status,status),status='recovering' WHERE run_id=?4 AND owner_epoch=?5",
		current.ownerEpoch+1, process.pid, process.startIdentity, current.runID, current.ownerEpoch)
	if err != nil {
		return runRecord{}, err
	}
	if err := ensure(changed == 1, "recovery takeover lost CAS"); err != nil {
		return runRecord{}, err
	}
	return getRun(ctx, conn, expected.runID)
}

// borrowedPane is the deletion's pinned run and placement with the pane lock it already holds.
type borrowedPane struct {
	run       runRecord
	placement placement
	guard     *paneLock
}

func recoverMemberUnderPaneLock(ctx context.Context, conn *dbConn, s settings, expectedRun runRecord, expectedPlacement placement, until time.Time, held *paneLock) (recoveryCompletion, error) {
	borrowed := &borrowedPane{run: expectedRun, placement: expectedPlacement, guard: held}
	return runtimeRecoveryRecover(ctx, conn, s, expectedRun.memberID, until, borrowed, nativeRecoveryHost{})
}

func runtimeRecoveryRecover(ctx context.Context, conn *dbConn, s settings, memberID int64, until time.Time, borrowed *borrowedPane, host recoveryHost) (recoveryCompletion, error) {
	deadlines := shutdownDeadlinesStartingNow(host.clock()).clipped(until)
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return recoveryCompletion{}, err
	}
	if err := ensure(memberID > 0, "positive recovery member ID required"); err != nil {
		return recoveryCompletion{}, err
	}
	pinnedSnapshot, err := snapshot(ctx, conn, memberID, until)
	if err != nil {
		return recoveryCompletion{}, err
	}
	run, pinned := pinnedSnapshot.run, pinnedSnapshot.pinned
	if borrowed != nil {
		if err := ensure(jsonEqual(stableRunIdentity(run), stableRunIdentity(borrowed.run)) && pinned == some(borrowed.placement),
			"borrowed recovery starting identity mismatch"); err != nil {
			return recoveryCompletion{}, err
		}
		if err := borrowed.guard.validateBinding(s.databaseURL, borrowed.placement); err != nil {
			return recoveryCompletion{}, err
		}
	}
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return recoveryCompletion{}, err
	}
	if err := checkpoint(host, recoveryBoundaryAfterSnapshot, run, until); err != nil {
		return recoveryCompletion{}, err
	}
	var paneProof *paneLock
	if borrowed != nil {
		paneProof = borrowed.guard
	} else if target, set := pinned.get(); set && target.muxPaneID.set {
		for {
			if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
				return recoveryCompletion{}, err
			}
			guard, acquired, err := paneLockTryAcquire(s.databaseURL, target)
			if err != nil {
				return recoveryCompletion{}, err
			}
			if acquired {
				paneProof = guard
				defer guard.close()
				break
			}
			owned, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (opt[*runLock], error) {
				return runLockAcquire(ctx, conn, run.owner())
			})
			if err == nil {
				lock, set := owned.get()
				if !set {
					return completion(run, recoveryOutcomeLiveOrUnreachable)
				}
				closeFile(lock.file)
			}
			host.clock().sleep(min(5*time.Millisecond, max(until.Sub(host.clock().now()), 0)))
		}
	}
	checkPane := func() error {
		if paneProof == nil {
			return nil
		}
		target, set := pinned.get()
		if !set {
			panic("held concrete placement")
		}
		return paneProof.validateBinding(s.databaseURL, target)
	}
	if err := checkPane(); err != nil {
		return recoveryCompletion{}, err
	}
	run, err = withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return checkSnapshot(ctx, conn, run, pinned)
	})
	if err != nil {
		return recoveryCompletion{}, err
	}
	if run.isSettled() {
		return completion(run, recoveryOutcomeRecovered)
	}
	owned, ownedErr := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (opt[*runLock], error) {
		return runLockAcquire(ctx, conn, run.owner())
	})
	var lock *runLock
	pathFailure := false
	if ownedErr != nil {
		pathFailure = true
	} else {
		acquired, set := owned.get()
		if !set {
			return completion(run, recoveryOutcomeLiveOrUnreachable)
		}
		lock = acquired
		defer closeFile(lock.file)
	}
	owner, err := host.processEvidence(run.ownerProcess(), until)
	if err != nil {
		return recoveryCompletion{}, err
	}
	health, err := host.health(run, until)
	if err != nil {
		return recoveryCompletion{}, err
	}
	var healthLive bool
	switch health := health.(type) {
	case recoveryHealthMatching:
		healthLive = true
	case recoveryHealthUnavailable:
		if err := ensure(health.reason != "", "health uncertainty requires a diagnostic"); err != nil {
			return recoveryCompletion{}, err
		}
	default:
		panic("invalid recoveryHealth")
	}
	if owner != processEvidenceTerminated || run.ownerKind == ownerKindCreator || healthLive {
		return completion(run, recoveryOutcomeLiveOrUnreachable)
	}
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return recoveryCompletion{}, err
	}
	if err := checkpoint(host, recoveryBoundaryBeforeTakeover, run, until); err != nil {
		return recoveryCompletion{}, err
	}
	process, err := host.currentProcess()
	if err != nil {
		return recoveryCompletion{}, err
	}
	recheck := func(ctx context.Context, conn *dbConn) error {
		if err := checkPane(); err != nil {
			return err
		}
		_, err := checkSnapshot(ctx, conn, run, pinned)
		return err
	}
	if pathFailure {
		return reconcileMissingRuntime(ctx, conn, run, pinned, process, until, recheck, host.clock())
	}
	current, err := lock.transferToReconcilerChecked(ctx, conn, run, process, until, recheck, host.clock())
	if err != nil {
		return recoveryCompletion{}, err
	}
	if current.isSettled() {
		return completion(current, recoveryOutcomeRecovered)
	}
	if _, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, settleLostDeferred(ctx, conn, current.owner())
	}); err != nil {
		return recoveryCompletion{}, err
	}
	return reconcileRetainedRuntime(ctx, conn, current, pinned, lock, host, deadlines, until)
}

type nativeRecoveryHost struct{}

func (nativeRecoveryHost) clock() clock { return systemClock{} }

func (nativeRecoveryHost) currentProcess() (processIdentity, error) {
	return processIdentityCurrent()
}

func (h nativeRecoveryHost) processEvidence(process processIdentity, until time.Time) (processEvidence, error) {
	if err := runtimeCheckDeadline(h.clock().now(), until); err != nil {
		return 0, err
	}
	alive, err := process.isAlive()
	switch {
	case err != nil:
		return processEvidenceUnknown, nil
	case alive:
		return processEvidenceAlive, nil
	default:
		return processEvidenceTerminated, nil
	}
}

func (h nativeRecoveryHost) health(run runRecord, until time.Time) (recoveryHealth, error) {
	value, err := runtimeControlRequest(run, lifecycleRequestHealth(run), until, h.clock())
	if err != nil {
		return recoveryHealthUnavailable{reason: cafleetErrorOf(err).message()}, nil
	}
	if _, err := healthSnapshotDecode(value, run); err != nil {
		return recoveryHealthUnavailable{reason: cafleetErrorOf(err).message()}, nil
	}
	return recoveryHealthMatching{}, nil
}

func (h nativeRecoveryHost) cleanupOwned(run runRecord, deadlines shutdownDeadlines) (shutdownOutcome, error) {
	return recoverDescendants(run, deadlines, h.clock())
}

func (h nativeRecoveryHost) unlinkDescriptor(path string, deadline time.Time) error {
	return unlinkDescriptorFile(path, deadline, h.clock())
}

func deliveryResultTransition(runID, eventSeq int64, receipt unpublishedDeliveryResult, observedAt string, evidenceJSON jsontext.Value) observedTransition {
	return observedTransition{
		RunID:        runID,
		EventSeq:     eventSeq,
		Kind:         runtimeEventKindInputDeliveryResult,
		ObservedAt:   observedAt,
		EvidenceJSON: evidenceJSON,
		CauseJSON:    inputDeliveryResultCause(receipt.inputID, receipt.stage, receipt.stopCause),
	}
}

func reconcileMissingRuntime(ctx context.Context, conn *dbConn, expected runRecord, pinned opt[placement], process processIdentity, until time.Time, check func(context.Context, *dbConn) error, c clock) (recoveryCompletion, error) {
	return withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (recoveryCompletion, error) {
		return inTransaction(conn, transactionImmediate, func() (recoveryCompletion, error) {
			if err := check(ctx, conn); err != nil {
				return recoveryCompletion{}, err
			}
			current, err := checkSnapshot(ctx, conn, expected, pinned)
			if err != nil {
				return recoveryCompletion{}, err
			}
			if current.isSettled() {
				if err := runtimeCheckDeadline(c.now(), until); err != nil {
					return recoveryCompletion{}, err
				}
				return completion(current, recoveryOutcomeRecovered)
			}
			established, err := terminalOutcome(conn, current)
			if err != nil {
				return recoveryCompletion{}, err
			}
			run, err := advanceRecoveryOwner(ctx, conn, current, process)
			if err != nil {
				return recoveryCompletion{}, err
			}
			recovering, err := completion(run, recoveryOutcomeRecovering)
			if err != nil {
				return recoveryCompletion{}, err
			}
			report := recovering.report
			report.cleanupDiagnostics = append(report.cleanupDiagnostics, jsonValue("runtime_path_missing"))
			if _, err := conn.execute("UPDATE member_runs SET cleanup_diagnostics_json=?1 WHERE run_id=?2",
				string(jsonValue(report.cleanupDiagnostics)), run.runID); err != nil {
				return recoveryCompletion{}, err
			}
			target, terminal := established.get()
			if !terminal {
				first, err := nextEventSeq(ctx, conn, run)
				if err != nil {
					return recoveryCompletion{}, err
				}
				transitions, err := lossTransitions(ctx, conn, run, first, true, c)
				if err != nil {
					return recoveryCompletion{}, err
				}
				for _, transition := range transitions {
					_, inserted, err := persistTransitionInTransaction(ctx, conn, run, transition, persistenceModeRecovery)
					if err != nil {
						return recoveryCompletion{}, err
					}
					if err := ensure(inserted, "missing-path recovery requires fresh transitions"); err != nil {
						return recoveryCompletion{}, err
					}
				}
				target = runStatusLost
			}
			if err := settleLostDeferredInTransaction(ctx, conn, run); err != nil {
				return recoveryCompletion{}, err
			}
			unpublished, err := unpublishedDeliveryResults(ctx, conn, run.runID)
			if err != nil {
				return recoveryCompletion{}, err
			}
			if len(unpublished) > 0 {
				evidence, err := runEvidence(run, "runtime-recovery", 0)
				if err != nil {
					return recoveryCompletion{}, err
				}
				eventSeq, err := nextEventSeq(ctx, conn, run)
				if err != nil {
					return recoveryCompletion{}, err
				}
				for _, receipt := range unpublished {
					transition := deliveryResultTransition(run.runID, eventSeq, receipt, wallStamp(c), evidence)
					if _, _, err := persistTransitionInTransaction(ctx, conn, run, transition, persistenceModeRecovery); err != nil {
						return recoveryCompletion{}, err
					}
					eventSeq++
				}
			}
			if _, err := conn.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2) WHERE run_id=?3",
				target.asStr(), wallStamp(c), run.runID); err != nil {
				return recoveryCompletion{}, err
			}
			run, err = getRun(ctx, conn, run.runID)
			if err != nil {
				return recoveryCompletion{}, err
			}
			if err := runtimeCheckDeadline(c.now(), until); err != nil {
				return recoveryCompletion{}, err
			}
			report.outcome = recoveryOutcomeRecovered
			report.status = run.status
			return recoveryCompletion{report: report, run: run}, nil
		})
	})
}

func nextEventSeq(ctx context.Context, conn *dbConn, run runRecord) (int64, error) {
	latest, err := latestEventSeq(ctx, conn, run.runID)
	if err != nil {
		return 0, err
	}
	seq, _ := latest.get()
	return seq + 1, nil
}

func reconcileRetainedRuntime(ctx context.Context, conn *dbConn, run runRecord, pinned opt[placement], lock *runLock, host recoveryHost, deadlines shutdownDeadlines, until time.Time) (recoveryCompletion, error) {
	result, err := completion(run, recoveryOutcomeRecovering)
	if err != nil {
		return recoveryCompletion{}, err
	}
	attempt := perform(ctx, conn, &result, pinned, lock, host, deadlines, until)
	switch {
	case attempt == nil:
		return result, nil
	case cafleetErrorOf(attempt).kind == errorDeadlineExceeded || !host.clock().now().Before(until):
		result.report.cleanupDiagnostics = append(result.report.cleanupDiagnostics, jsonValue(cafleetErrorOf(attempt).message()))
		return result, nil
	default:
		return recoveryCompletion{}, attempt
	}
}

func refresh(ctx context.Context, conn *dbConn, result *recoveryCompletion, until time.Time) error {
	run, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return getRun(ctx, conn, result.run.runID)
	})
	if err != nil {
		return err
	}
	result.run = run
	result.report.status = run.status
	return nil
}

func runtimeRecoveryPersist(ctx context.Context, conn *dbConn, result *recoveryCompletion, transition observedTransition, host recoveryHost, until time.Time) error {
	if _, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		_, _, err := persistTransition(ctx, conn, result.run.owner(), transition, persistenceModeRecovery)
		return struct{}{}, err
	}); err != nil {
		return err
	}
	if err := refresh(ctx, conn, result, until); err != nil {
		return err
	}
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return err
	}
	return checkpoint(host, recoveryBoundaryAfterCommit, result.run, until)
}

func lossTransitions(ctx context.Context, conn *dbConn, run runRecord, first int64, missing bool, c clock) ([]observedTransition, error) {
	recorded, err := brokerRuntimeSnapshot(ctx, conn, run.runID)
	if err != nil {
		return nil, err
	}
	next := first
	now := wallStamp(c)
	var transitions []observedTransition
	if interactionID, open := recorded.currentInteractionID.get(); open {
		eventID, err := queryRow(conn,
			"SELECT event_id FROM member_runtime_events WHERE interaction_id=?1 ORDER BY event_seq LIMIT 1",
			[]any{interactionID}, func(row sqlRow) (int64, error) { return row.int64(0) })
		if err != nil {
			return nil, err
		}
		view, err := getEvent(ctx, conn, eventID)
		if err != nil {
			return nil, err
		}
		interaction, found := view.interaction.get()
		if !found {
			return nil, runtimeError("pending recovery interaction missing")
		}
		transitions = append(transitions, observedTransition{
			RunID:          run.runID,
			EventSeq:       next,
			EpisodeSeq:     some(interaction.episodeSeq),
			Kind:           runtimeEventKindInteractionClosed,
			Classification: some(interaction.classification),
			ObservedAt:     now,
			EvidenceJSON:   interaction.evidenceJSON,
			CloseReason:    some(interactionCloseReasonMonitorLost),
		})
		if next == math.MaxInt64 {
			return nil, runtimeError("recovery sequence overflow")
		}
		next++
	}
	evidence, err := runEvidence(run, "runtime-recovery", 0)
	if err != nil {
		return nil, err
	}
	cause := runtimeCauseKindMonitorLost
	if missing {
		cause = runtimeCauseKindRuntimePathMissing
	}
	return append(transitions, observedTransition{
		RunID:        run.runID,
		EventSeq:     next,
		Kind:         runtimeEventKindRunnerLost,
		ObservedAt:   now,
		EvidenceJSON: evidence,
		CauseJSON:    jsonValue(jsonObject{{"kind", cause.asStr()}}),
	}), nil
}

func publishDeliveryResults(ctx context.Context, conn *dbConn, result *recoveryCompletion, host recoveryHost, until time.Time) error {
	type pending struct {
		unpublished []unpublishedDeliveryResult
		first       int64
	}
	found, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (pending, error) {
		unpublished, err := unpublishedDeliveryResults(ctx, conn, result.run.runID)
		if err != nil {
			return pending{}, err
		}
		first, err := nextEventSeq(ctx, conn, result.run)
		return pending{unpublished: unpublished, first: first}, err
	})
	if err != nil {
		return err
	}
	evidence, err := runEvidence(result.run, "runtime-recovery", 0)
	if err != nil {
		return err
	}
	for offset, receipt := range found.unpublished {
		transition := deliveryResultTransition(result.run.runID, found.first+int64(offset), receipt, wallStamp(host.clock()), evidence)
		if err := runtimeRecoveryPersist(ctx, conn, result, transition, host, until); err != nil {
			return err
		}
	}
	return nil
}

func terminalOutcome(conn *dbConn, run runRecord) (opt[runStatus], error) {
	terminal, err := queryRowOptional(conn,
		"SELECT kind FROM member_runtime_events WHERE run_id=?1 AND kind IN ('runner_exited','runner_lost') ORDER BY event_seq DESC LIMIT 1",
		[]any{run.runID}, func(row sqlRow) (string, error) { return row.text(0) })
	if err != nil {
		return none[runStatus](), err
	}
	if kind, found := terminal.get(); found {
		switch kind {
		case "runner_exited":
			return some(runStatusExited), nil
		case "runner_lost":
			return some(runStatusLost), nil
		default:
			return none[runStatus](), runtimeError("unexpected terminal recovery event")
		}
	}
	if origin, set := run.recoveryOriginStatus.get(); set && origin.terminal() {
		return some(origin), nil
	}
	if run.status.terminal() {
		return some(run.status), nil
	}
	return none[runStatus](), nil
}

func perform(ctx context.Context, conn *dbConn, result *recoveryCompletion, pinned opt[placement], lock *runLock, host recoveryHost, deadlines shutdownDeadlines, until time.Time) error {
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return err
	}
	if err := checkpoint(host, recoveryBoundaryAfterTakeover, result.run, until); err != nil {
		return err
	}
	descriptor, err := recoveryDescriptorPin(ctx, conn, result.run, lock, until, host.clock())
	if err != nil {
		return err
	}
	existing, err := withDatabaseDeadline(ctx, conn, until, func(_ context.Context, conn *dbConn) (opt[runStatus], error) {
		return terminalOutcome(conn, result.run)
	})
	if err != nil {
		return err
	}
	target, terminal := existing.get()
	if !terminal {
		transitions, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) ([]observedTransition, error) {
			first, err := nextEventSeq(ctx, conn, result.run)
			if err != nil {
				return nil, err
			}
			return lossTransitions(ctx, conn, result.run, first, false, host.clock())
		})
		if err != nil {
			return err
		}
		for _, transition := range transitions {
			if err := runtimeRecoveryPersist(ctx, conn, result, transition, host, until); err != nil {
				return err
			}
		}
		target = runStatusLost
	}
	if err := publishDeliveryResults(ctx, conn, result, host, until); err != nil {
		return err
	}
	cleanup, err := host.cleanupOwned(result.run, deadlines)
	if err != nil {
		return err
	}
	for _, diagnostic := range cleanup.diagnostics {
		result.report.cleanupDiagnostics = append(result.report.cleanupDiagnostics, jsonValue(diagnostic))
	}
	if cleanup.cleanupState != cleanupStateComplete {
		result.report.cleanupDiagnostics = append(result.report.cleanupDiagnostics, jsonValue("owned recovery cleanup remains incomplete"))
		return nil
	}
	if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
		return err
	}
	if err := checkpoint(host, recoveryBoundaryBeforeFinish, result.run, until); err != nil {
		return err
	}
	if _, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return checkSnapshot(ctx, conn, result.run, pinned)
	}); err != nil {
		return err
	}
	if err := descriptor.remove(ctx, conn, result.run, lock, until, host.clock(), host.unlinkDescriptor); err != nil {
		return err
	}
	finished, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return inTransaction(conn, transactionImmediate, func() (runRecord, error) {
			if _, err := checkSnapshot(ctx, conn, result.run, pinned); err != nil {
				return runRecord{}, err
			}
			if err := lock.validate(ctx, conn, result.run.owner()); err != nil {
				return runRecord{}, err
			}
			if _, err := validateOwner(ctx, conn, result.run.owner()); err != nil {
				return runRecord{}, err
			}
			if _, err := conn.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2),cleanup_diagnostics_json=?3 WHERE run_id=?4 AND owner_epoch=?5",
				target.asStr(), wallStamp(host.clock()), string(jsonValue(result.report.cleanupDiagnostics)), result.run.runID, result.run.ownerEpoch); err != nil {
				return runRecord{}, err
			}
			current, err := getRun(ctx, conn, result.run.runID)
			if err != nil {
				return runRecord{}, err
			}
			if err := runtimeCheckDeadline(host.clock().now(), until); err != nil {
				return runRecord{}, err
			}
			return current, nil
		})
	})
	if err != nil {
		return err
	}
	result.run = finished
	result.report.status = finished.status
	result.report.outcome = recoveryOutcomeRecovered
	return nil
}
