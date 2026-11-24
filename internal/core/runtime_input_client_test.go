package core

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ncruces/go-sqlite3"
	"golang.org/x/sys/unix"
)

const inputClientFixtureNow = "2026-10-26T22:00:00.000000+00:00"

// inputClientCase is a running run with one opened permission request.
type inputClientCase struct {
	fixture  *runnerFixture
	owner    runOwner
	settings settings
}

func newInputClientCase(t *testing.T) *inputClientCase {
	t.Helper()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	_, _, owner := fixture.runningOwner(t)
	changed, err := sqlExecute(fixture.conn,
		"UPDATE member_runs SET native_version='0.162.0',coverage='tested',started_at=?1 WHERE run_id=?2",
		inputClientFixtureNow, fixture.run.runID)
	if err != nil || changed != 1 {
		t.Fatalf("pinning the run's native version changed %d rows: %v", changed, err)
	}
	fixture.refresh(t)
	evidence, err := metadataEvidence(jsontext.Value(fmt.Sprintf(
		`{"backend":"codex","native_version":%s,"rule_id":"fixture-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false}`,
		mustJSON(t, fixture.run.nativeVersion))))
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := persistTransition(ctx, fixture.conn, owner, observedTransition{
		RunID:          owner.runID,
		EventSeq:       1,
		EpisodeSeq:     some[int64](1),
		Kind:           runtimeEventKindInteractionOpened,
		Classification: some(interactionClassPermissionRequest),
		ObservedAt:     inputClientFixtureNow,
		EvidenceJSON:   evidence,
		CloseReason:    none[interactionCloseReason](),
	}, persistenceModeLive); err != nil {
		t.Fatal(err)
	}
	return &inputClientCase{fixture: fixture, owner: owner, settings: settingsForDatabase(fixture.databaseFile(t))}
}

// noLiveHost refuses every live operation, recording the name of each one it was asked for.
type noLiveHost struct {
	calls []string
}

func (h *noLiveHost) refuse(operation string) *cafleetError {
	h.calls = append(h.calls, operation)
	return appError("history fixture reached a forbidden live " + operation)
}

func (h *noLiveHost) clock() clock { return systemClock{} }

func (h *noLiveHost) lock(settings, placement, deadline) (*paneLock, bool, error) {
	return nil, false, h.refuse("lock")
}

func (h *noLiveHost) health(runRecord, time.Time) (healthSnapshot, error) {
	return healthSnapshot{}, h.refuse("health")
}

func (h *noLiveHost) exchange(runRecord, inputControlRequest, time.Time) (inputControlReply, error) {
	return nil, h.refuse("exchange")
}

func runMemberPlacement(t *testing.T, conn *dbConn, run runRecord) placement {
	t.Helper()
	member, err := getMember(context.Background(), conn, run.memberID, run.fleetID)
	if err != nil {
		t.Fatal(err)
	}
	record, ok := member.get()
	if !ok {
		t.Fatalf("member %d is registered", run.memberID)
	}
	return record.placement
}

func healthRecords(t *testing.T, conn *dbConn) []tableSnapshot {
	t.Helper()
	return takeTableSnapshot(t, conn, func(table string) bool {
		return table == "member_interactions" || table == "member_runtime_events" || table == "messages"
	})
}

func totalChanges(conn *dbConn) int64 {
	return conn.raw.TotalChanges()
}

func storedInput(t *testing.T, conn *dbConn, inputID int64) inputOperation {
	t.Helper()
	input, err := getInput(context.Background(), conn, inputID)
	if err != nil {
		t.Fatal(err)
	}
	return input
}

// inputHistory is a running run holding one retained ordinary input in a chosen state.
type inputHistory struct {
	fixture   *runnerFixture
	settings  settings
	placement placement
	request   submitInput
	row       inputOperation
}

func newInputHistory(t *testing.T, state inputState, disposition opt[inputDisposition]) *inputHistory {
	t.Helper()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	_, _, owner := fixture.runningOwner(t)
	request := submitInput{
		OperationKey: fmt.Sprintf("prompt:%d:%s", owner.runID, strings.Repeat("b", 64)),
		InputKind:    inputKindPrompt,
		Literal:      "retained ordinary λ literal",
	}
	row, err := reserveInput(ctx, fixture.conn, owner, newInput{
		operationKey:  request.OperationKey,
		kind:          request.InputKind,
		payloadDigest: inputPayloadDigest(owner.runID, request),
	})
	if err != nil {
		t.Fatal(err)
	}
	if settled, ok := disposition.get(); ok {
		_, err = settleInput(ctx, fixture.conn, owner, row.inputID, inputStateInProgress, settled, "fixture:external-history")
	} else if state == inputStateUncertain {
		_, err = recordInputUncertain(ctx, fixture.conn, owner, row.inputID, stopCauseWriteFailed)
	} else if state != inputStateInProgress {
		_, err = recordInputState(ctx, fixture.conn, owner, row.inputID, inputStateInProgress, state)
	}
	if err != nil {
		t.Fatal(err)
	}
	row = storedInput(t, fixture.conn, row.inputID)
	if row.state != state {
		t.Fatalf("the history input is %s, want %s", row.state.asStr(), state.asStr())
	}
	return &inputHistory{
		fixture:   fixture,
		settings:  settingsForDatabase(fixture.databaseFile(t)),
		placement: runMemberPlacement(t, fixture.conn, fixture.run),
		request:   request,
		row:       row,
	}
}

func (h *inputHistory) identity() permissionReceiptIdentity {
	return permissionReceiptIdentity{databaseIdentityJSON: h.fixture.run.databaseIdentityJSON, runID: h.fixture.run.runID}
}

func (h *inputHistory) assertUnchanged(t *testing.T, changes int64) {
	t.Helper()
	if got := totalChanges(h.fixture.conn); got != changes {
		t.Fatalf("the connection made %d changes, want %d", got, changes)
	}
	if got := storedInput(t, h.fixture.conn, h.row.inputID); got != h.row {
		t.Fatalf("input %+v, want %+v", got, h.row)
	}
}

func (h *inputHistory) deregisterMember(t *testing.T) {
	t.Helper()
	if _, err := sqlExecute(h.fixture.conn, "UPDATE members SET status='deregistered' WHERE member_id=?1", h.fixture.run.memberID); err != nil {
		t.Fatal(err)
	}
}

func holdPaneLock(t *testing.T, s settings, p placement) *paneLock {
	t.Helper()
	held, acquired, err := paneLockTryAcquire(s.databaseURL, p)
	if err != nil {
		t.Fatal(err)
	}
	if !acquired {
		t.Fatal("the pane lock is free")
	}
	t.Cleanup(held.close)
	return held
}

func assertPaneLockBusy(t *testing.T, s settings, p placement) {
	t.Helper()
	guard, acquired, err := paneLockTryAcquire(s.databaseURL, p)
	if err != nil {
		t.Fatal(err)
	}
	if acquired {
		guard.close()
		t.Fatal("the held pane lock stays busy")
	}
}

func TestRuntimeInputClient_LiveReceiptSettledHistoryIsImmutableBeforeLiveCallsAndTheOriginalSqlDeadline(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	history := newInputHistory(t, inputStateSettled, some(inputDispositionSubmitted))
	history.deregisterMember(t)
	held := holdPaneLock(t, history.settings, history.placement)
	changes := totalChanges(history.fixture.conn)
	beforeRun := mustRun(t, history.fixture.conn, history.row.runID)
	records := healthRecords(t, history.fixture.conn)
	original := time.Now().Add(realTimeBound)
	conn := history.fixture.conn
	reads := recordReadBudgets(t, conn, "member_input_operations")
	_, expired := reconcileReceiptWithHost(ctx, conn, history.settings, history.row.inputID, inputDispositionSubmitted,
		"fixture:replacement-evidence", deadlineUntil(time.Now()), &noLiveHost{})
	expiredReads := reads.take()
	var recorded inputOperation
	var historyFailure *inputFailure
	if _, err := withDatabaseBudget(ctx, conn, deadlineUntil(original), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		recorded, historyFailure = reconcileReceiptWithHost(ctx, conn, history.settings, history.row.inputID, inputDispositionSubmitted,
			"fixture:replacement-evidence", deadlineUntil(original.Add(realTimeBound)), &noLiveHost{})
		return struct{}{}, nil
	}); err != nil {
		t.Fatal(err)
	}
	historyReads := reads.take()
	if expired == nil || expired.source.kind != errorDeadlineExceeded {
		t.Fatalf("an expired reconciliation exceeds its deadline: %+v", expired)
	}
	if len(expiredReads) != 0 {
		t.Fatalf("an expired reconciliation reads %d input rows", len(expiredReads))
	}
	if historyFailure != nil {
		t.Fatal(historyFailure.asError())
	}
	if recorded != history.row {
		t.Fatalf("same disposition preserves the actual original row and evidence: %+v", recorded)
	}
	if len(historyReads) == 0 {
		t.Fatal("the settled history reads its input row")
	}
	for _, budget := range historyReads {
		if budget.After(original) {
			t.Fatalf("a history read budget %v exceeds the original %v", budget, original)
		}
	}
	if _, failure := reconcileReceiptWithHost(ctx, conn, history.settings, history.row.inputID, inputDispositionAbandonedUncertain,
		"fixture:conflicting-disposition", deadlineUntil(original), &noLiveHost{}); failure == nil {
		t.Fatal("a conflicting disposition is refused")
	}
	foreign := newRunnerFixture(t)
	foreignSettings := settings{databaseURL: "sqlite:///" + foreign.databaseFile(t), databaseURLExplicit: true}
	if _, failure := reconcileReceiptWithHost(ctx, conn, foreignSettings, history.row.inputID, inputDispositionSubmitted,
		"fixture:foreign-database", deadlineUntil(original), &noLiveHost{}); failure == nil {
		t.Fatal("a foreign configured database is refused")
	}
	history.assertUnchanged(t, changes)
	assertRunUnchanged(t, conn, beforeRun, "settled history")
	if !tableSnapshotsEqual(healthRecords(t, conn), records) {
		t.Fatal("settled history writes no health record")
	}
	if err := held.validateBinding(history.settings.databaseURL, history.placement); err != nil {
		t.Fatal(err)
	}
	for _, invalid := range []struct {
		inputID  int64
		evidence string
	}{{0, "fixture:invalid-id"}, {history.row.inputID, " "}} {
		if _, failure := reconcileReceiptWithHost(ctx, conn, history.settings, invalid.inputID, inputDispositionSubmitted,
			invalid.evidence, deadlineUntil(original), &noLiveHost{}); failure == nil {
			t.Fatalf("input %d with evidence %q is refused", invalid.inputID, invalid.evidence)
		}
	}
	history.assertUnchanged(t, changes)
}

func TestRuntimeInputClient_ExternalInputRetainedHistoryPrecedesBusyPaneAndInactiveOwner(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		state       inputState
		disposition opt[inputDisposition]
		status      submissionStatus
		stage       submissionStage
	}{
		{"accepted", inputStateAccepted, none[inputDisposition](), submissionStatusAccepted, submissionStageSubmitted},
		{"uncertain", inputStateUncertain, none[inputDisposition](), submissionStatusUncertain, submissionStageUnknown},
		{"settled abandoned", inputStateSettled, some(inputDispositionAbandonedUncertain), submissionStatusRejected, submissionStageZeroBytes},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			history := newInputHistory(t, c.state, c.disposition)
			history.deregisterMember(t)
			holdPaneLock(t, history.settings, history.placement)
			changes := totalChanges(history.fixture.conn)
			found, failure := findSubmission(ctx, history.fixture.conn, history.identity(), history.request,
				deadlineUntil(time.Now().Add(realTimeBound)), systemClock{})
			if failure != nil {
				t.Fatalf("%s: %v", c.name, failure.asError())
			}
			if found != some(history.row) {
				t.Fatalf("%s: found %+v", c.name, found)
			}
			result, failure := submitWrappedInputWithHost(ctx, history.fixture.conn, history.settings, history.fixture.run,
				history.placement, history.request, deadlineUntil(time.Now().Add(realTimeBound)), &noLiveHost{})
			if failure != nil {
				t.Fatalf("%s: %v", c.name, failure.asError())
			}
			if result.InputID != some(history.row.inputID) || result.Status != c.status || result.Stage != c.stage ||
				result.OutputSeq != none[uint64]() {
				t.Fatalf("%s: result %+v", c.name, result)
			}
			if c.status == submissionStatusRejected {
				if reason, ok := result.Reason.get(); !ok || reason == "" {
					t.Fatalf("%s: a rejected history names its reason", c.name)
				}
			}
			history.assertUnchanged(t, changes)
			assertPaneLockBusy(t, history.settings, history.placement)
		})
	}
}

func TestRuntimeInputClient_ExternalInputBorrowedHistoryKeepsThePreparedDeadlineBeforeFirstSql(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	history := newInputHistory(t, inputStateAccepted, none[inputDisposition]())
	held := holdPaneLock(t, history.settings, history.placement)
	conn := history.fixture.conn
	changes := totalChanges(conn)
	controlDeadline := time.Now().Add(realTimeBound)
	runBefore := mustRun(t, conn, history.fixture.run.runID)
	recordsBefore := healthRecords(t, conn)
	control, err := newPreparedSubmission(history.request, controlDeadline, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	result, failure := submitWrappedInputUnderPaneLockWithHost(ctx, conn, history.settings, history.fixture.run, history.placement,
		held, control, deadlineUntil(controlDeadline), &noLiveHost{})
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if result.Status != submissionStatusAccepted || result.InputID != some(history.row.inputID) {
		t.Fatalf("borrowed history %+v", result)
	}

	original := time.Now().Add(100 * time.Millisecond)
	expiredAttempt, err := newPreparedSubmission(history.request, original, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(time.Until(original) + time.Millisecond)
	if time.Now().Before(original) {
		t.Fatal("the checked prepared attempt actually expired")
	}
	originalActive := time.Now().Add(realTimeBound)
	reads := recordReadBudgets(t, conn, "member_input_operations")
	_, expired := submitWrappedInputUnderPaneLockWithHost(ctx, conn, history.settings, history.fixture.run, history.placement,
		held, expiredAttempt, deadlineUntil(time.Now().Add(3*realTimeBound)), &noLiveHost{})
	expiredReads := reads.take()
	activeAttempt, err := newPreparedSubmission(history.request, originalActive, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	inherited := deadlineUntil(time.Now().Add(2 * realTimeBound))
	supplied := deadlineUntil(time.Now().Add(3 * realTimeBound))
	var active submissionResult
	var activeFailure *inputFailure
	if _, err := withDatabaseBudget(ctx, conn, inherited, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		active, activeFailure = submitWrappedInputUnderPaneLockWithHost(ctx, conn, history.settings, history.fixture.run,
			history.placement, held, activeAttempt, supplied, &noLiveHost{})
		return struct{}{}, nil
	}); err != nil {
		t.Fatal(err)
	}
	budgets := reads.take()
	history.assertUnchanged(t, changes)
	assertRunUnchanged(t, conn, runBefore, "borrowed history")
	if !tableSnapshotsEqual(healthRecords(t, conn), recordsBefore) {
		t.Fatal("borrowed history writes no health record")
	}
	assertPaneLockBusy(t, history.settings, history.placement)
	if expired == nil {
		t.Fatal("expired prepared history rejects before lookup")
	}
	if expired.source.kind != errorDeadlineExceeded || expired.context.runID != some(history.fixture.run.runID) ||
		expired.context.operationKey != some(history.request.OperationKey) || expired.context.inputID != none[int64]() {
		t.Fatalf("expired failure %+v", expired)
	}
	if len(expiredReads) != 0 {
		t.Fatal("an expired original budget starts no receipt SQL")
	}
	if activeFailure != nil {
		t.Fatal(activeFailure.asError())
	}
	if active.Status != submissionStatusAccepted || active.InputID != some(history.row.inputID) {
		t.Fatalf("active borrowed history %+v", active)
	}
	if len(budgets) == 0 {
		t.Fatal("the valid original budget reaches actual receipt SQL")
	}
	for _, budget := range budgets {
		if budget.After(originalActive) {
			t.Fatalf("first history SQL inherits the prepared minimum, not a later caller or outer budget: %v", budget)
		}
	}
}

func TestRuntimeInputClient_ExternalInputDiscoveryRejectsPayloadAssociationAndDatabaseConflicts(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	history := newInputHistory(t, inputStateAccepted, none[inputDisposition]())
	conn := history.fixture.conn
	changes := totalChanges(conn)
	identity := history.identity()
	d := deadlineUntil(time.Now().Add(realTimeBound))
	found, failure := findSubmission(ctx, conn, identity, history.request, d, systemClock{})
	if failure != nil || found != some(history.row) {
		t.Fatalf("the retained input is found: %+v %v", found, failure)
	}
	var reads atomic.Int64
	if err := conn.raw.SetAuthorizer(func(action sqlite3.AuthorizerActionCode, table, _, _, _ string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_READ && table == "member_input_operations" {
			reads.Add(1)
			return sqlite3.AUTH_DENY
		}
		return sqlite3.AUTH_OK
	}); err != nil {
		t.Fatal(err)
	}
	_, denied := findSubmission(ctx, conn, identity, history.request, d, systemClock{})
	removed := conn.raw.SetAuthorizer(nil)
	if denied == nil {
		t.Fatalf("supplied-connection read refusal is authoritative; cleanup=%v", removed)
	}
	if removed != nil {
		t.Fatal(removed)
	}
	if reads.Load() == 0 {
		t.Fatal("actual ordinary receipt SQL uses the caller connection")
	}
	history.assertUnchanged(t, changes)
	for _, change := range []string{"literal", "kind"} {
		request := history.request
		switch change {
		case "literal":
			request.Literal += "!"
		case "kind":
			request.InputKind = inputKindExec
		}
		_, failure := findSubmission(ctx, conn, identity, request, d, systemClock{})
		if failure == nil {
			t.Fatalf("%s: a reused key binds the original complete payload", change)
		}
		if strings.Contains(failure.source.message(), request.Literal) {
			t.Fatalf("%s: the conflict keeps the literal private", change)
		}
		if failure.context.runID != some(history.fixture.run.runID) || failure.context.operationKey != some(history.request.OperationKey) {
			t.Fatalf("%s: failure context %+v", change, failure.context)
		}
		history.assertUnchanged(t, changes)
		if _, failure := submitWrappedInputWithHost(ctx, conn, history.settings, history.fixture.run, history.placement,
			request, d, &noLiveHost{}); failure == nil {
			t.Fatalf("%s: the conflicting submission is refused", change)
		}
	}
	other := newRunnerFixture(t)
	foreign := permissionReceiptIdentity{databaseIdentityJSON: other.run.databaseIdentityJSON, runID: identity.runID}
	if _, failure := findSubmission(ctx, conn, foreign, history.request, d, systemClock{}); failure == nil {
		t.Fatal("a foreign database identity is refused")
	}
	history.assertUnchanged(t, changes)
	missing := submitInput{OperationKey: "prompt:missing", InputKind: inputKindPrompt, Literal: "missing input"}
	found, failure = findSubmission(ctx, conn, identity, missing, d, systemClock{})
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if _, present := found.get(); present {
		t.Fatal("an unknown operation key finds no input")
	}
	history.assertUnchanged(t, changes)
}

func TestRuntimeInputClient_ExternalInputStaleOrUnreachableHealthRefusesBeforeExchangeAndPreservesOpenHistory(t *testing.T) {
	t.Parallel()
	for _, stale := range []bool{false, true} {
		t.Run(fmt.Sprintf("stale=%v", stale), func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			history := newInputHistory(t, inputStateReady, none[inputDisposition]())
			conn := history.fixture.conn
			run := history.fixture.run
			evidence, err := metadataEvidence(jsontext.Value(`{"backend":"codex","native_version":"0.163.0","rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false}`))
			if err != nil {
				t.Fatal(err)
			}
			if _, _, err := persistTransition(ctx, conn, run.owner(), observedTransition{
				RunID:          run.runID,
				EventSeq:       1,
				EpisodeSeq:     some[int64](1),
				Kind:           runtimeEventKindInteractionOpened,
				Classification: some(interactionClassQuestion),
				ObservedAt:     "2026-10-27T16:44:00Z",
				EvidenceJSON:   evidence,
				CloseReason:    none[interactionCloseReason](),
			}, persistenceModeLive); err != nil {
				t.Fatal(err)
			}
			before := mustRun(t, conn, run.runID)
			records := healthRecords(t, conn)
			if len(records[0].rows) != 1 {
				t.Fatalf("meaningful open interaction control: %d rows", len(records[0].rows))
			}
			until := time.Now().Add(realTimeBound)
			host := newSocketHost(t, history.settings, history.placement, until)
			if stale {
				runner := currentProcess(t)
				host.healthReply = jsontext.Value(fmt.Sprintf(`{"identity":%s,"runner":%s,"child":null,"output_seq":120}`,
					mustJSON(t, controlIdentityFromRun(run)), processWire(t, runner.pid, runner.startIdentity+":stale-health-reply")))
			} else {
				paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
				if err != nil {
					t.Fatal(err)
				}
				if _, err := os.Lstat(paths.controlSocket); !os.IsNotExist(err) {
					t.Fatalf("owned unreachable endpoint is absent: %v", err)
				}
			}
			_, failure := submitWrappedInputWithHost(ctx, conn, history.settings, run, history.placement, history.request,
				deadlineUntil(until), host)
			if failure == nil {
				t.Fatal("stale or unreachable health refuses ordinary input")
			}
			if host.calls != [3]int{1, 1, 0} {
				t.Fatalf("live calls %v", host.calls)
			}
			if failure.context.runID != some(run.runID) || failure.context.operationKey != some(history.request.OperationKey) {
				t.Fatalf("failure context %+v", failure.context)
			}
			if strings.Contains(failure.source.message(), history.request.Literal) {
				t.Fatal("the refusal keeps the literal private")
			}
			after := mustRun(t, conn, run.runID)
			after.lastHealthAt = before.lastHealthAt
			if !after.equal(before) {
				t.Fatalf("refusal preserves lifecycle, owner, coverage and event reservation: %+v", after)
			}
			if !tableSnapshotsEqual(healthRecords(t, conn), records) {
				t.Fatal("the refusal writes no health record")
			}
			if got := storedInput(t, conn, history.row.inputID); got != history.row {
				t.Fatalf("input %+v, want %+v", got, history.row)
			}
		})
	}
}

func TestRuntimeInputClient_InputFailure_InputFailureContextWrapsTheSourceWithOnlyItsKnownIdentities(t *testing.T) {
	t.Parallel()
	const key = "fixture:key\nwith\"quotes\"\\tail"
	fields := []string{"run_id", "interaction_id", "input_id", "operation_key"}
	cases := []struct {
		name    string
		context inputContext
		known   []string
	}{
		{"no known identity", inputContext{}, nil},
		{"every identity", inputContext{runID: some[int64](29), interactionID: some[int64](17), inputID: some[int64](31), operationKey: some(key)}, fields},
		{"one identity", inputContext{interactionID: some[int64](17)}, []string{"interaction_id"}},
	}
	causes := func() []*cafleetError {
		return []*cafleetError{
			usageError("fixture usage"),
			usageError("nested usage").withDiagnostic("original diagnostic"),
		}
	}
	for _, c := range cases {
		for _, original := range causes() {
			converted := c.context.failure(original).asError()
			if converted.exitCode() != original.exitCode() {
				t.Fatalf("%s: exit code %d, want %d", c.name, converted.exitCode(), original.exitCode())
			}
			if len(c.known) == 0 {
				if converted.kind != original.kind || converted.message() != original.message() {
					t.Fatalf("%s: converted %#v, want the source %#v", c.name, converted, original)
				}
				continue
			}
			if converted.kind != errorWithDiagnostics {
				t.Fatalf("%s: known identities retain the source with context: %#v", c.name, converted)
			}
			if converted.cause.kind != original.kind || converted.cause.message() != original.message() {
				t.Fatalf("%s: cause %#v, want %#v", c.name, converted.cause, original)
			}
			if len(converted.diagnostics) != 1 {
				t.Fatalf("%s: diagnostics %q", c.name, converted.diagnostics)
			}
			text := converted.diagnostics[0]
			previous := -1
			for _, field := range c.known {
				position := strings.Index(text, field)
				if position <= previous {
					t.Fatalf("%s: %s appears in order in %q", c.name, field, text)
				}
				previous = position
			}
			for _, absent := range fields {
				if !slices.Contains(c.known, absent) && strings.Contains(text, absent) {
					t.Fatalf("%s: %s is absent from %q", c.name, absent, text)
				}
			}
			if slices.Contains(c.known, "operation_key") && !strings.Contains(text, jsonQuoted(t, key)) {
				t.Fatalf("%s: the operation key is JSON-quoted in %q", c.name, text)
			}
			if strings.Contains(text, "\n") {
				t.Fatalf("%s: the context is one line: %q", c.name, text)
			}
		}
	}
}

func openedPermission(t *testing.T, c *inputClientCase) (int64, int64) {
	t.Helper()
	rows := tableRows(t, c.fixture.conn,
		"SELECT event_id,interaction_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'", c.fixture.run.runID)
	if len(rows) != 1 {
		t.Fatalf("opened interactions %v", rows)
	}
	return rows[0][0].(int64), rows[0][1].(int64)
}

func copiedInspectionDatabase(t *testing.T, c *inputClientCase) (settings, *dbConn) {
	t.Helper()
	path := filepath.Join(c.fixture.dir, "copied-inspection.db")
	copyFile(t, c.fixture.databaseFile(t), path)
	copied := settingsForDatabase(path)
	conn, err := connect(copied.databaseURL)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	return copied, conn
}

func refusedInspection(t *testing.T, conn *dbConn, s settings, eventID, interactionID, directorMemberID int64) *cafleetError {
	t.Helper()
	_, failure := inspectPermission(context.Background(), conn, s, permissionInspectInput{
		EventID:          eventID,
		InteractionID:    interactionID,
		DirectorMemberID: directorMemberID,
	}, deadlineUntil(time.Now().Add(realTimeBound)))
	if failure == nil {
		t.Fatal("this inspection is refused")
	}
	return failure.source
}

func TestRuntimeInputClient_PermissionHistory_InspectErrors_PermissionInspectReportsAMissingInteractionBeforeEventRunOrIdentityChecks(t *testing.T) {
	t.Parallel()
	c := newInputClientCase(t)
	eventID, interactionID := openedPermission(t, c)
	director := c.fixture.run.directorMemberID
	missing := interactionID + 1000
	copied, conn := copiedInspectionDatabase(t, c)
	for _, event := range []struct {
		id    int64
		label string
	}{{eventID, "an existing event"}, {eventID + 1000, "a missing event"}} {
		failure := refusedInspection(t, conn, copied, event.id, missing, director)
		if failure.message() != fmt.Sprintf("Interaction %d not found", missing) || failure.exitCode() != 1 {
			t.Fatalf("%s with a foreign database identity: %q exit %d", event.label, failure.message(), failure.exitCode())
		}
	}
}

func TestRuntimeInputClient_PermissionHistory_InspectErrors_PermissionInspectNamesTheRunDatabaseIdentityFieldOnAMismatch(t *testing.T) {
	t.Parallel()
	c := newInputClientCase(t)
	eventID, interactionID := openedPermission(t, c)
	copied, conn := copiedInspectionDatabase(t, c)
	failure := refusedInspection(t, conn, copied, eventID, interactionID, c.fixture.run.directorMemberID)
	if failure.kind != errorInvalidStoredValue || failure.field != "member_runs.database_identity_json" ||
		failure.value != "selected database identity mismatch" {
		t.Fatalf("identity mismatch %#v", failure)
	}
	if failure.message() != "invalid stored value for member_runs.database_identity_json: selected database identity mismatch" ||
		failure.exitCode() != 1 {
		t.Fatalf("message %q exit %d", failure.message(), failure.exitCode())
	}
}

var uncertainReplyMessages = [4]string{"identity", "contradiction", "missing", "unmatched"}

func uncertainInput(t *testing.T, cause opt[stopCause]) inputOperation {
	t.Helper()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	input := fixture.input(t, owner, "prompt:reason", inputKindPrompt)
	recorded, ok := cause.get()
	if !ok {
		return input
	}
	uncertain, err := recordInputUncertain(context.Background(), fixture.conn, owner, input.inputID, recorded)
	if err != nil {
		t.Fatal(err)
	}
	return uncertain
}

func uncertainReply(input inputOperation, reason opt[string]) submissionResult {
	return submissionResult{
		Status:    submissionStatusUncertain,
		InputID:   some(input.inputID),
		Stage:     submissionStageUnknown,
		OutputSeq: none[uint64](),
		Reason:    reason,
		Delivery:  some(input.delivery()),
	}
}

func TestRuntimeInputClient_UncertainReason_DurableOutcomeReportsTheRecordedCauseAsTheUncertainReason(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		cause opt[stopCause]
	}{
		{"a recorded cause", some(stopCauseShortWrite)},
		{"a causeless in-progress receipt", none[stopCause]()},
	}
	for _, c := range cases {
		input := uncertainInput(t, c.cause)
		reason := none[string]()
		if cause, ok := c.cause.get(); ok {
			reason = some(cause.asStr())
		} else if input.state != inputStateInProgress {
			t.Fatalf("%s: state %s", c.name, input.state.asStr())
		}
		want := some(submissionOutcome{status: submissionStatusUncertain, stage: submissionStageUnknown, reason: reason})
		if got := durableOutcome(input); got != want {
			t.Fatalf("%s: outcome %+v, want %+v", c.name, got, want)
		}
	}
}

func TestRuntimeInputClient_UncertainReason_UncertainReplyMustCarryTheReceiptCause(t *testing.T) {
	t.Parallel()
	input := uncertainInput(t, some(stopCauseShortWrite))
	if err := validateReplyAgainstDurable(uncertainReply(input, some("short_write")), some(input), uncertainReplyMessages); err != nil {
		t.Fatal(err)
	}
	for _, wrong := range []opt[string]{none[string](), some("write_failed"), some("short write")} {
		err := validateReplyAgainstDurable(uncertainReply(input, wrong), some(input), uncertainReplyMessages)
		if err == nil || !strings.Contains(cafleetErrorOf(err).message(), "contradiction") {
			t.Fatalf("%v: %v", wrong, err)
		}
	}
}

func TestRuntimeInputClient_UncertainReason_UncertainReplyForACauselessReceiptCarriesNoReason(t *testing.T) {
	t.Parallel()
	input := uncertainInput(t, none[stopCause]())
	if err := validateReplyAgainstDurable(uncertainReply(input, none[string]()), some(input), uncertainReplyMessages); err != nil {
		t.Fatal(err)
	}
	err := validateReplyAgainstDurable(uncertainReply(input, some("manual_input")), some(input), uncertainReplyMessages)
	if err == nil || !strings.Contains(cafleetErrorOf(err).message(), "contradiction") {
		t.Fatalf("a reason on a causeless receipt contradicts it: %v", err)
	}
}

func TestRuntimeInputClient_AbsentPlacementReceiptStaysUncertainWithoutLiveCalls(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	c := newInputClientCase(t)
	conn := c.fixture.conn
	reserved, err := reserveInput(ctx, conn, c.owner, newInput{
		operationKey:  "prompt:absent-placement",
		kind:          inputKindPrompt,
		payloadDigest: strings.Repeat("d", 64),
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := recordInputState(ctx, conn, c.owner, reserved.inputID, inputStateInProgress, inputStateAccepted); err != nil {
		t.Fatal(err)
	}
	memberID := c.fixture.run.memberID
	if _, err := sqlExecute(conn, "UPDATE members SET status='deregistered' WHERE member_id=?1", memberID); err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(conn, "DELETE FROM member_placements WHERE member_id=?1", memberID); err != nil {
		t.Fatal(err)
	}
	original := storedInput(t, conn, reserved.inputID)
	host := &noLiveHost{}
	_, failure := reconcileReceiptWithHost(ctx, conn, c.settings, reserved.inputID, inputDispositionSubmitted,
		"fixture:absent-placement-receipt", deadlineUntil(time.Now().Add(realTimeBound)), host)
	if failure == nil {
		t.Fatal("an absent placement refuses live reconciliation")
	}
	if failure.source.message() != "receipt reconciliation requires the member's live runner; the input stays uncertain" {
		t.Fatalf("message %q", failure.source.message())
	}
	if len(host.calls) != 0 {
		t.Fatalf("live calls %v", host.calls)
	}
	if got := storedInput(t, conn, reserved.inputID); got != original {
		t.Fatalf("input %+v, want %+v", got, original)
	}
}

type socketReplyMode uint8

const (
	socketReplyLive socketReplyMode = iota + 1
	socketReplyUnknownID
	socketReplyDisconnected
)

// socketHost drives the actual pane lock, health probe and control socket, asserting the
// deadline and lock state each live call receives; its modes and hooks alter one call.
type socketHost struct {
	t                *testing.T
	settings         settings
	placement        placement
	until            time.Time
	expiration       opt[uint64]
	calls            [3]int
	requests         []jsontext.Value
	reply            socketReplyMode
	afterHealth      func()
	healthReply      jsontext.Value
	exchangeLimit    int
	wrongQueryID     opt[int64]
	skipReconcile    bool
	tamperInspection func(*permissionInspection)
}

func newSocketHost(t *testing.T, s settings, p placement, until time.Time) *socketHost {
	return &socketHost{t: t, settings: s, placement: p, until: until, reply: socketReplyLive, exchangeLimit: 1}
}

func (h *socketHost) clock() clock { return systemClock{} }

func (h *socketHost) lock(s settings, p placement, d deadline) (*paneLock, bool, error) {
	h.calls[0]++
	if !d.instant().Equal(h.until) {
		h.t.Errorf("the lock receives the original deadline, got %v", d.instant())
	}
	return paneLockAcquireWithin(s.databaseURL, p, d)
}

func (h *socketHost) paneLockHeld() bool {
	guard, acquired, err := paneLockTryAcquire(h.settings.databaseURL, h.placement)
	if err != nil {
		h.t.Fatalf("probing the pane lock: %v", err)
	}
	if acquired {
		guard.close()
	}
	return !acquired
}

func (h *socketHost) health(run runRecord, deadline time.Time) (healthSnapshot, error) {
	h.calls[1]++
	if !deadline.Equal(h.until) {
		h.t.Errorf("health receives the original deadline, got %v", deadline)
	}
	if !h.paneLockHeld() {
		h.t.Error("the client holds the pane lock across its health probe")
	}
	var snapshot healthSnapshot
	var err error
	if reply := h.healthReply; reply != nil {
		h.healthReply = nil
		snapshot, err = healthSnapshotDecode(reply, run)
	} else {
		snapshot, err = probeHealth(run, deadline, systemClock{})
	}
	if err != nil {
		return snapshot, err
	}
	if h.afterHealth != nil {
		h.afterHealth()
	}
	return snapshot, nil
}

func (h *socketHost) exchange(run runRecord, request inputControlRequest, deadline time.Time) (inputControlReply, error) {
	h.calls[2]++
	if h.calls[2] > h.exchangeLimit {
		h.t.Errorf("an explicit attempt uses only its declared exchanges: call %d of %d", h.calls[2], h.exchangeLimit)
	}
	if !deadline.Equal(h.until) {
		h.t.Errorf("the exchange receives the original deadline, got %v", deadline)
	}
	clientHoldsLock := h.paneLockHeld()
	switch request.Operation.(type) {
	case submitPromptOperation, submitExecOperation:
		if clientHoldsLock {
			h.t.Error("a wrapped client awaits the runner's submission reply without the pane lock")
		}
	case permissionInspectOperation, permissionRespondOperation, receiptOperation:
		if !clientHoldsLock {
			h.t.Error("the actual guard spans every receipt exchange")
		}
	default:
		h.t.Fatalf("operation %T", request.Operation)
	}
	if !request.Identity.equal(controlIdentityFromRun(run)) {
		h.t.Errorf("the request carries the run's control identity: %+v", request.Identity)
	}
	if expected, set := h.expiration.get(); set {
		if request.ExpiresAtMonoNS != expected {
			h.t.Errorf("expiration %d, want %d", request.ExpiresAtMonoNS, expected)
		}
	} else {
		h.expiration = some(request.ExpiresAtMonoNS)
	}
	encoded, err := encodeInputRequest(request)
	if err != nil {
		return nil, err
	}
	h.requests = append(h.requests, jsontext.Value(encoded[4:]))
	if operation, ok := request.Operation.(receiptOperation); ok {
		switch action := operation.action.(type) {
		case receiptReconcile:
			if h.skipReconcile {
				return receiptSettled{InputID: action.InputID}, nil
			}
		case receiptRetire:
			independent, err := connect(h.settings.databaseURL)
			if err != nil {
				return nil, err
			}
			retired, err := getInput(context.Background(), independent, action.InputID)
			closeConnection(h.t, independent)
			if err != nil {
				return nil, err
			}
			if retired.state != inputStateSettled {
				h.t.Errorf("Retire follows the committed durable reread: state %s", retired.state.asStr())
			}
		case receiptQuery:
		default:
			h.t.Fatalf("receipt action %T", action)
		}
	}
	if h.reply == socketReplyUnknownID {
		return submissionResult{
			Status: submissionStatusAccepted, Stage: submissionStageSubmitted, InputID: some[int64](math.MaxInt64),
			OutputSeq: some[uint64](1), Reason: none[string](),
			Delivery: some(deliveryProjection{Status: deliveryStatusSubmitted, Method: some(deliveryMethodProfile), Reason: none[string]()}),
		}, nil
	}
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		return nil, err
	}
	client, err := controlClientConnect(paths.controlSocket, deadline, systemClock{})
	if err != nil {
		return nil, err
	}
	if h.reply == socketReplyDisconnected {
		return nil, h.disconnectedExchange(client, run, request, encoded, deadline)
	}
	reply, err := client.exchangeInput(request, deadline, systemClock{})
	if closeErr := client.close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return nil, err
	}
	if wrong, set := h.wrongQueryID.get(); set {
		present, ok := reply.(receiptPresent)
		if !ok {
			h.t.Fatalf("owned Query returns a real Present receipt, got %T", reply)
		}
		present.InputID = wrong
		present.Submission.InputID = some(wrong)
		reply = present
	}
	if h.tamperInspection != nil {
		inspection, ok := reply.(permissionInspection)
		if !ok {
			h.t.Fatalf("a tampered exchange is a permission inspection, got %T", reply)
		}
		h.tamperInspection(&inspection)
		reply = inspection
	}
	return reply, nil
}

// disconnectedExchange sends the request with its read side shut, waits until the runner commits
// the receipt as accepted, then drops the connection without reading the reply.
func (h *socketHost) disconnectedExchange(client *controlClient, run runRecord, request inputControlRequest, encoded []byte, deadline time.Time) error {
	defer func() { _ = client.close() }()
	if err := unix.Shutdown(client.fd, unix.SHUT_RD); err != nil {
		return ioError(ioErrorText(err))
	}
	for offset := 0; offset < len(encoded); {
		if err := runtimeCheckDeadline(time.Now(), deadline); err != nil {
			return err
		}
		count, err := unix.Write(client.fd, encoded[offset:])
		switch {
		case err == nil && count == 0:
			h.t.Fatal("owned external request disconnected during send")
		case err == nil:
			offset += count
		case retryable(err):
			if err := pause(deadline, systemClock{}); err != nil {
				return err
			}
		default:
			return ioError(ioErrorText(err))
		}
	}
	var key string
	switch operation := request.Operation.(type) {
	case submitPromptOperation:
		key = operation.input.OperationKey
	case submitExecOperation:
		key = operation.input.OperationKey
	case permissionInspectOperation, permissionRespondOperation, receiptOperation:
		h.t.Fatalf("disconnected fixture sends a durable input operation, got %T", operation)
	default:
		h.t.Fatalf("operation %T", operation)
	}
	observed, err := connect(h.settings.databaseURL)
	if err != nil {
		return err
	}
	defer closeConnection(h.t, observed)
	for {
		retained, err := withDatabaseBudget(context.Background(), observed, deadlineUntil(deadline), func(ctx context.Context, conn *dbConn) (opt[inputOperation], error) {
			return findInput(ctx, conn, run.runID, key)
		})
		if err != nil {
			return err
		}
		if input, ok := retained.get(); ok && input.state == inputStateAccepted {
			return appError("owned reply consumer closed")
		}
		if err := pause(deadline, systemClock{}); err != nil {
			return err
		}
	}
}

// openedInteraction is the event, interaction and message of the run's opened interaction,
// polled until it commits or until passes.
func openedInteraction(t *testing.T, f *serviceFixture, until time.Time) (int64, int64, int64) {
	t.Helper()
	for {
		rows, err := withDatabaseBudget(context.Background(), f.fixture.conn, deadlineUntil(until), func(_ context.Context, conn *dbConn) ([][]any, error) {
			return sqlRows(conn, "SELECT event_id,interaction_id,message_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'", f.fixture.run.runID)
		})
		if err != nil {
			t.Fatal(err)
		}
		if len(rows) == 1 {
			return rows[0][0].(int64), rows[0][1].(int64), rows[0][2].(int64)
		}
		if !time.Now().Before(until) {
			t.Fatal("the opened interaction commits before the original deadline")
		}
		time.Sleep(2 * time.Millisecond)
	}
}

func inputRows(t *testing.T, conn *dbConn) int64 {
	t.Helper()
	return queryInt64(t, conn, "SELECT COUNT(*) FROM member_input_operations")
}

func messageStamped(t *testing.T, conn *dbConn, messageID int64) bool {
	t.Helper()
	return queryInt64(t, conn, "SELECT notified_at IS NOT NULL FROM messages WHERE message_id=?1", messageID) == 1
}

// nativeAnswersThroughBarrier types barrier and requires the native answers journaled since start
// to be exactly prior followed by the barrier's.
func nativeAnswersThroughBarrier(t *testing.T, f *serviceFixture, start int, barrier string, prior []string) {
	t.Helper()
	until := time.Now().Add(realTimeBound)
	expected := "review-answer=" + barrier
	f.writeOuter(barrier + "\r")
	buffer := make([]byte, 4096)
	for {
		interval := lossyUTF8(f.output[start:])
		if position := strings.Index(interval, expected); position >= 0 && strings.Contains(interval[position:], "\n") {
			var answers []string
			for line := range strings.Lines(interval) {
				if at := strings.Index(line, "review-answer="); at >= 0 {
					answers = append(answers, strings.TrimRight(line[at:], "\r\n"))
				}
			}
			if want := append(slices.Clone(prior), expected); !slices.Equal(answers, want) {
				t.Fatalf("complete retained native answer interval %q: answers %q, want %q", interval, answers, want)
			}
			return
		}
		if !time.Now().Before(until) {
			t.Fatalf("complete manual barrier within its original bound: %q", interval)
		}
		count, err := unix.Read(f.masterIO, buffer)
		switch {
		case err == nil && count == 0:
			t.Fatal("owned outer PTY closed before complete inspection barrier")
		case err == nil:
			f.output = append(f.output, buffer[:count]...)
		case retryable(err):
		default:
			t.Fatalf("owned inspection barrier read: %v", err)
		}
		time.Sleep(min(2*time.Millisecond, time.Until(until)))
	}
}

func inspectionBarrier(t *testing.T, f *serviceFixture, start int, barrier string) {
	t.Helper()
	nativeAnswersThroughBarrier(t, f, start, barrier, nil)
}

// overwriteObservationCatalog replaces the catalog in the observation-services file that
// syntheticObservationServices already wrote.
func overwriteObservationCatalog(t *testing.T, fixture *runnerFixture, catalog any) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(fixture.dir, "observation-services.json"), jsonBytes(jo("catalog", catalog)), 0o600); err != nil {
		t.Fatal(err)
	}
}

func serviceTarget(t *testing.T, f *serviceFixture) (settings, runRecord, placement) {
	t.Helper()
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	return settingsForDatabase(f.fixture.databaseFile(t)), run, runMemberPlacement(t, f.fixture.conn, run)
}

func ordinaryInput(run runRecord, key string) submitInput {
	return submitInput{OperationKey: fmt.Sprintf("prompt:%d:%s", run.runID, key), InputKind: inputKindPrompt, Literal: "external " + key + " λ"}
}

func inspectionFixture(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
	t.Helper()
	syntheticObservationServices(t, fixture, spec)
	overwriteObservationCatalog(t, fixture, inspectionFixtureDefinition())
}

func TestRuntimeInputClient_PermissionInspectionActualServiceProjectsCurrentUnsupportedRequestWithoutAnswering(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, syntheticObservationServices)
	f.startRunning()
	until := time.Now().Add(realTimeBound)
	f.writeOuter("form-inspection\r")
	f.readUntil("Operation: write form-inspection.txt")
	outputStart := len(f.output)
	eventID, interactionID, messageID := openedInteraction(t, f, until)
	run := mustRun(t, f.fixture.conn, f.fixture.run.runID)
	before := inputRows(t, f.fixture.conn)
	inspected, failure := inspectPermission(context.Background(), f.fixture.conn, settingsForDatabase(f.fixture.databaseFile(t)),
		permissionInspectInput{InteractionID: interactionID, EventID: eventID, DirectorMemberID: run.directorMemberID}, deadlineUntil(until))
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if inspected.EventID != eventID || inspected.InteractionID != interactionID || inspected.MessageID != messageID ||
		inspected.RunID != run.runID || inspected.OwnerEpoch != run.ownerEpoch || inspected.MemberID != run.memberID ||
		inspected.FleetID != run.fleetID || inspected.DirectorMemberID != run.directorMemberID {
		t.Fatalf("identity %+v", inspected)
	}
	if inspected.Classification != some(interactionClassPermissionRequest) || inspected.Visibility != visibilityVisible ||
		inspected.AnswerCapability != coverageUnknown {
		t.Fatalf("projection %+v", inspected)
	}
	if _, reason := inspected.CapabilityReason.get(); !reason || len(inspected.SupportedDecisions) != 0 || inspected.RequestToken.set {
		t.Fatalf("an unsupported request carries a reason and no answer: %+v", inspected)
	}
	if !jsonIsNull(jsonIndex(inspected.Evidence, "operation")) {
		t.Fatalf("evidence %s", inspected.Evidence)
	}
	var missing []string
	for _, field := range inspected.MissingEvidence {
		missing = append(missing, field.asStr())
	}
	if want := []string{"operation", "arguments", "cwd", "environment", "targets", "effects", "choice_scope"}; !slices.Equal(missing, want) {
		t.Fatalf("missing evidence %q", missing)
	}
	if inputRows(t, f.fixture.conn) != before || messageStamped(t, f.fixture.conn, messageID) {
		t.Fatal("inspection writes no receipt and stamps nothing")
	}
	inspectionBarrier(t, f, outputStart, "inspection-barrier")
	f.finish()
}

func TestRuntimeInputClient_PermissionInspectionReplyBindsBackendNativeVersionAndTestedCapabilityFields(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, inspectionFixture)
	f.startRunning()
	// Fifteen sequential inspections share this one deadline.
	outer := time.Now().Add(3 * realTimeBound)
	f.writeOuter("form-reply-binding\r")
	f.readUntil("Operation: write form-reply-binding.txt")
	outputStart := len(f.output)
	eventID, interactionID, _ := openedInteraction(t, f, outer)
	s, run, p := serviceTarget(t, f)
	request := permissionInspectInput{EventID: eventID, InteractionID: interactionID, DirectorMemberID: run.directorMemberID}
	inspect := func(tamper func(*permissionInspection)) (permissionInspection, *inputFailure) {
		t.Helper()
		host := newSocketHost(t, s, p, outer)
		host.tamperInspection = tamper
		result, failure := inspectPermissionWithHost(context.Background(), f.fixture.conn, s, request, deadlineUntil(outer), host)
		if host.calls != [3]int{1, 1, 1} {
			t.Fatalf("live calls %v", host.calls)
		}
		return result, failure
	}
	live, failure := inspect(nil)
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if live.AnswerCapability != coverageTested || live.Backend != run.backend || live.NativeVersion != run.nativeVersion ||
		!jsonEqual(jsonIndex(live.Evidence, "native_version"), mustJSON(t, run.nativeVersion)) || live.AnswerRuleID.set {
		t.Fatalf("live %+v", live)
	}
	shared, failure := inspect(func(reply *permissionInspection) { reply.AnswerRuleID = some("inspection-shared-plan") })
	if failure != nil || shared.AnswerRuleID != some("inspection-shared-plan") {
		t.Fatalf("a shared answer rule is accepted: %+v %v", shared.AnswerRuleID, failure)
	}
	tampers := []struct {
		label  string
		tamper func(*permissionInspection)
	}{
		{"backend", func(r *permissionInspection) { r.Backend = "claude" }},
		{"native version", func(r *permissionInspection) { r.NativeVersion = some("0.0.2-synthetic") }},
		{"absent native version", func(r *permissionInspection) { r.NativeVersion = none[string]() }},
		{"evidence backend", func(r *permissionInspection) { r.Evidence = jsonWith(r.Evidence, "backend", "claude") }},
		{"evidence native version", func(r *permissionInspection) { r.Evidence = jsonWith(r.Evidence, "native_version", "0.0.2-synthetic") }},
		{"token", func(r *permissionInspection) { r.RequestToken = none[string]() }},
		{"decisions", func(r *permissionInspection) { r.SupportedDecisions = nil }},
		{"classification", func(r *permissionInspection) { r.Classification = none[interactionClass]() }},
		{"visibility", func(r *permissionInspection) { r.Visibility = visibilityObscured }},
		{"reason", func(r *permissionInspection) { r.CapabilityReason = some("unavailable") }},
		{"blank rule", func(r *permissionInspection) { r.AnswerRuleID = some(" ") }},
		{"oversized rule", func(r *permissionInspection) { r.AnswerRuleID = some(strings.Repeat("r", 257)) }},
	}
	for _, c := range tampers {
		if _, failure := inspect(c.tamper); failure == nil {
			t.Fatalf("%s: the tampered reply is refused", c.label)
		}
	}
	bounded, failure := inspect(func(r *permissionInspection) { r.AnswerRuleID = some(strings.Repeat("r", 256)) })
	if rule, _ := bounded.AnswerRuleID.get(); failure != nil || len(rule) != 256 {
		t.Fatalf("a 256-byte rule is accepted: %v", failure)
	}
	inspectionBarrier(t, f, outputStart, "reply-binding-barrier")
	f.finish()
}

func TestRuntimeInputClient_PermissionInspectionClientAcceptsATestedReplyForARunWithoutANativeVersion(t *testing.T) {
	t.Parallel()
	f := configuredServiceFixture(t, false, func(t *testing.T, fixture *runnerFixture, spec *spawnSpec) {
		inspectionFixture(t, fixture, spec)
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET native_version=NULL WHERE run_id=?1", fixture.run.runID); err != nil {
			t.Fatal(err)
		}
		fixture.refresh(t)
	})
	f.startRunning()
	outer := time.Now().Add(realTimeBound)
	f.writeOuter("form-unprobed\r")
	f.readUntil("Operation: write form-unprobed.txt")
	outputStart := len(f.output)
	eventID, interactionID, _ := openedInteraction(t, f, outer)
	s, run, p := serviceTarget(t, f)
	if run.nativeVersion.set {
		t.Fatal("the run records no native version")
	}
	inspected, failure := inspectPermissionWithHost(context.Background(), f.fixture.conn, s,
		permissionInspectInput{EventID: eventID, InteractionID: interactionID, DirectorMemberID: run.directorMemberID},
		deadlineUntil(outer), newSocketHost(t, s, p, outer))
	if failure != nil {
		t.Fatal(failure.asError())
	}
	if inspected.AnswerCapability != coverageTested || inspected.NativeVersion.set || !jsonIsNull(jsonIndex(inspected.Evidence, "native_version")) ||
		!inspected.RequestToken.set || len(inspected.SupportedDecisions) == 0 || inspected.CapabilityReason.set {
		t.Fatalf("inspected %+v", inspected)
	}
	inspectionBarrier(t, f, outputStart, "unprobed-barrier")
	f.finish()
}

func TestRuntimeInputClient_PermissionInspectionClientUsesRealGuardOriginalSqlBoundAndValidatesLiveReply(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	f := configuredServiceFixture(t, false, inspectionFixture)
	f.startRunning()
	outer := time.Now().Add(realTimeBound)
	f.writeOuter("form-executable-inspection\r")
	f.readUntil("Operation: write form-executable-inspection.txt")
	outputStart := len(f.output)
	eventID, interactionID, messageID := openedInteraction(t, f, outer)
	s, run, p := serviceTarget(t, f)
	request := permissionInspectInput{EventID: eventID, InteractionID: interactionID, DirectorMemberID: run.directorMemberID}
	records := healthRecords(t, f.fixture.conn)
	expired := newSocketHost(t, s, p, time.Now())
	if _, failure := inspectPermissionWithHost(ctx, f.fixture.conn, s, request, deadlineUntil(expired.until), expired); failure == nil {
		t.Fatal("an expired inspection is refused")
	}
	if expired.calls != [3]int{0, 0, 0} || !tableSnapshotsEqual(healthRecords(t, f.fixture.conn), records) {
		t.Fatalf("an expired inspection makes no live call: %v", expired.calls)
	}
	inherited := outer.Add(-100 * time.Millisecond)
	host := newSocketHost(t, s, p, inherited)
	value, err := withDatabaseBudget(ctx, f.fixture.conn, deadlineUntil(inherited), func(ctx context.Context, conn *dbConn) (permissionInspection, error) {
		inspected, failure := inspectPermissionWithHost(ctx, conn, s, request, deadlineUntil(outer), host)
		if failure != nil {
			return inspected, failure.asError()
		}
		return inspected, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if host.calls != [3]int{1, 1, 1} || !host.expiration.set {
		t.Fatalf("live calls %v", host.calls)
	}
	if value.EventID != eventID || value.InteractionID != interactionID || value.MessageID != messageID ||
		value.AnswerCapability != coverageTested || !slices.Equal(value.SupportedDecisions, []permissionDecision{permissionDecisionAllow, permissionDecisionDeny}) ||
		value.AnswerRuleID.set || !jsonIsNull(jsonIndex(value.Evidence, "operation")) || !slices.Contains(value.MissingEvidence, missingEvidenceChoiceScope) {
		t.Fatalf("value %+v", value)
	}
	if token, _ := value.RequestToken.get(); len(token) != 64 {
		t.Fatalf("token %q", token)
	}
	sender := func(conn *dbConn) int64 {
		return queryInt64(t, conn, "SELECT from_member_id FROM messages WHERE message_id=?1", messageID)
	}
	originalSender := sender(f.fixture.conn)
	if originalSender != run.memberID {
		t.Fatalf("the member sent the interaction message, not %d", originalSender)
	}
	replacementSender := run.directorMemberID
	changed := newSocketHost(t, s, p, outer)
	changed.afterHealth = func() {
		conn, err := connect(s.databaseURL)
		if err != nil {
			t.Fatal(err)
		}
		defer closeConnection(t, conn)
		if before := sender(conn); before != originalSender {
			t.Errorf("sender %d before the change", before)
		}
		if changedRows, err := sqlExecute(conn, "UPDATE messages SET from_member_id=?1 WHERE message_id=?2", replacementSender, messageID); err != nil || changedRows != 1 {
			t.Errorf("replacing the sender changed %d rows: %v", changedRows, err)
		}
		if after := sender(conn); after != replacementSender {
			t.Errorf("sender %d after the change", after)
		}
	}
	if _, failure := inspectPermissionWithHost(ctx, f.fixture.conn, s, request, deadlineUntil(outer), changed); failure == nil {
		t.Fatal("a changed association is refused")
	}
	if changed.calls != [3]int{1, 1, 0} {
		t.Fatalf("durable association is reread after actual health wait: %v", changed.calls)
	}
	if sender(f.fixture.conn) != replacementSender || inputRows(t, f.fixture.conn) != 0 || messageStamped(t, f.fixture.conn, messageID) {
		t.Fatal("the refusal writes nothing")
	}
	inspectionBarrier(t, f, outputStart, "inspection-client-barrier")
	f.finish()
}

func acceptedServiceReceipt(t *testing.T, f *serviceFixture, outer time.Time, key string) (settings, runRecord, placement, submitInput, inputOperation) {
	t.Helper()
	s, run, p := serviceTarget(t, f)
	request := ordinaryInput(run, key)
	result, failure := submitWrappedInput(context.Background(), f.fixture.conn, s, run, p, request, deadlineUntil(earliestTime(outer, time.Now().Add(realTimeBound))))
	if failure != nil {
		t.Fatal(failure.asError())
	}
	id, ok := result.InputID.get()
	if result.Status != submissionStatusAccepted || !ok {
		t.Fatalf("result %+v", result)
	}
	row := storedInput(t, f.fixture.conn, id)
	if row.state != inputStateAccepted {
		t.Fatalf("row %+v", row)
	}
	waitInputServiceRecord(t, f, request.Literal, outer)
	inputServiceBarrier(t, f, outer)
	return s, run, p, request, row
}

func TestRuntimeInputClient_LiveReceiptClientBindsQueryAndCommitAckBeforeOneRealRetirement(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p, request, original := acceptedServiceReceipt(t, f, outer, "receipt-live")
		other, err := reserveInput(ctx, f.fixture.conn, run.owner(), newInput{operationKey: "prompt:other-live-receipt", kind: inputKindPrompt, payloadDigest: strings.Repeat("c", 64)})
		if err != nil {
			t.Fatal(err)
		}
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		wrong := newSocketHost(t, s, p, until)
		wrong.wrongQueryID = some(other.inputID)
		_, failure := reconcileReceiptWithHost(ctx, f.fixture.conn, s, original.inputID, inputDispositionSubmitted, "fixture:live-reconcile", deadlineUntil(until), wrong)
		if failure == nil || wrong.calls != [3]int{1, 1, 1} {
			t.Fatalf("a mismatched query is refused after one exchange: %v %v", failure, wrong.calls)
		}
		if failure.context.inputID != some(original.inputID) || failure.context.operationKey != some(request.OperationKey) {
			t.Fatalf("failure context %+v", failure.context)
		}
		if got := storedInput(t, f.fixture.conn, original.inputID); got != original {
			t.Fatalf("input %+v", got)
		}
		unverified := newSocketHost(t, s, p, until)
		unverified.exchangeLimit, unverified.skipReconcile = 2, true
		if _, failure := reconcileReceiptWithHost(ctx, f.fixture.conn, s, original.inputID, inputDispositionSubmitted, "fixture:live-reconcile", deadlineUntil(until), unverified); failure == nil {
			t.Fatal("a typed Settled acknowledgment supplies no durable settlement")
		}
		if unverified.calls != [3]int{1, 1, 2} {
			t.Fatalf("live calls %v", unverified.calls)
		}
		if got := storedInput(t, f.fixture.conn, original.inputID); got != original {
			t.Fatalf("input %+v", got)
		}
		host := newSocketHost(t, s, p, until)
		host.exchangeLimit = 3
		reads := recordReadBudgets(t, f.fixture.conn, "member_input_operations")
		var settled inputOperation
		var settleFailure *inputFailure
		if _, err := withDatabaseBudget(ctx, f.fixture.conn, deadlineUntil(until), func(ctx context.Context, conn *dbConn) (struct{}, error) {
			settled, settleFailure = reconcileReceiptWithHost(ctx, conn, s, original.inputID, inputDispositionSubmitted, "fixture:live-reconcile", deadlineUntil(outer), host)
			return struct{}{}, nil
		}); err != nil {
			t.Fatal(err)
		}
		observed := reads.take()
		if settleFailure != nil {
			t.Fatal(settleFailure.asError())
		}
		if settled.inputID != original.inputID || settled.state != inputStateSettled || settled.disposition != some(inputDispositionSubmitted) ||
			settled.evidenceRef != some("fixture:live-reconcile") {
			t.Fatalf("settled %+v", settled)
		}
		if host.calls != [3]int{1, 1, 3} {
			t.Fatalf("live calls %v", host.calls)
		}
		var actions []string
		for _, sent := range host.requests {
			action, _ := jsonString(jsonIndex(sent, "action"))
			actions = append(actions, action)
			if !jsonIsString(jsonIndex(sent, "kind"), "receipt") || jsonHas(t, sent, "literal") {
				t.Fatalf("a receipt request carries no literal: %s", sent)
			}
		}
		if !slices.Equal(actions, []string{"query", "reconcile", "retire"}) {
			t.Fatalf("actions %q", actions)
		}
		if !jsonIsString(jsonIndex(host.requests[1], "expected_state"), "accepted") || !jsonIsString(jsonIndex(host.requests[1], "evidence_ref"), "fixture:live-reconcile") {
			t.Fatalf("reconcile request %s", host.requests[1])
		}
		if id, _ := jsonInt64(jsonIndex(host.requests[2], "input_id")); id != original.inputID {
			t.Fatalf("retire request %s", host.requests[2])
		}
		if len(observed) == 0 {
			t.Fatal("actual client receipt reads use the supplied connection")
		}
		for _, budget := range observed {
			if budget.After(until) {
				t.Fatalf("a receipt read budget %v exceeds %v", budget, until)
			}
		}
		if got := storedInput(t, f.fixture.conn, other.inputID); got != other {
			t.Fatalf("the other input is untouched: %+v", got)
		}
		repeated, failure := submitWrappedInputWithHost(ctx, f.fixture.conn, s, run, p, request, deadlineUntil(outer), &noLiveHost{})
		if failure != nil {
			t.Fatal(failure.asError())
		}
		if repeated.InputID != some(original.inputID) || repeated.Status != submissionStatusAccepted {
			t.Fatalf("repeated %+v", repeated)
		}
		inputServiceBarrier(t, f, outer)
		if count := len(slices.DeleteFunc(recordLines(t, f), func(line string) bool { return line != request.Literal })); count != 1 {
			t.Fatalf("the literal reaches the native child once, %d times", count)
		}
	})
}

func TestRuntimeInputClient_ExternalInputBorrowedLockPreservesBindingExpirationAndActualSocketSubmission(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		request := ordinaryInput(run, "borrowed")
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		attempt, err := newPreparedSubmission(request, until, systemClock{})
		if err != nil {
			t.Fatal(err)
		}
		expiration := attempt.expiration.monotonicNs()
		held := holdPaneLock(t, s, p)
		otherPlacement := p
		otherPlacement.muxPaneID = some("%foreign")
		wrong := holdPaneLock(t, s, otherPlacement)
		d := deadlineUntil(until)
		if _, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, s, run, p, wrong, attempt, d, &noLiveHost{}); failure == nil {
			t.Fatal("a lock on another pane is refused")
		}
		foreign := newRunnerFixture(t)
		foreignSettings := settings{databaseURL: "sqlite:///" + foreign.databaseFile(t), databaseURLExplicit: true}
		if _, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, foreignSettings, run, p, held, attempt, d, &noLiveHost{}); failure == nil {
			t.Fatal("a foreign configured database is refused")
		}
		if found, err := findInput(ctx, f.fixture.conn, run.runID, request.OperationKey); err != nil || found.set {
			t.Fatalf("the refusals reserve nothing: %v", err)
		}
		time.Sleep(25 * time.Millisecond)
		host := newSocketHost(t, s, p, until)
		host.expiration = some(expiration)
		result, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, s, run, p, held, attempt, d, host)
		if failure != nil {
			t.Fatal(failure.asError())
		}
		if host.calls != [3]int{0, 1, 1} || !jsonIsString(jsonIndex(host.requests[0], "literal"), request.Literal) || !jsonIsString(jsonIndex(host.requests[0], "kind"), "submit_prompt") {
			t.Fatalf("live calls %v requests %s", host.calls, host.requests)
		}
		id, _ := result.InputID.get()
		row := storedInput(t, f.fixture.conn, id)
		if result.Status != submissionStatusAccepted || row.payloadDigest != inputPayloadDigest(run.runID, request) || row.state != inputStateAccepted {
			t.Fatalf("result %+v row %+v", result, row)
		}
		assertPaneLockBusy(t, s, p)
		held.close()
		paneLockAcquireReleased(t, s.databaseURL, p).close()
		waitInputServiceRecord(t, f, request.Literal, outer)
		inputServiceBarrier(t, f, outer)
		if count := len(slices.DeleteFunc(recordLines(t, f), func(line string) bool { return line != request.Literal })); count != 1 {
			t.Fatalf("the literal reaches the native child once, %d times", count)
		}
	})
}

// lockTakingHost takes the pane lock as soon as the runner replies, so the client cannot
// reacquire it.
type lockTakingHost struct {
	*socketHost
	taken *paneLock
}

func (h *lockTakingHost) exchange(run runRecord, request inputControlRequest, deadline time.Time) (inputControlReply, error) {
	reply, err := h.socketHost.exchange(run, request, deadline)
	if err != nil {
		return nil, err
	}
	taken, acquired, err := paneLockAcquireWithin(h.settings.databaseURL, h.placement, deadlineUntil(time.Now().Add(realTimeBound)))
	if err != nil || !acquired {
		h.t.Fatalf("the released pane lock is taken: %v", err)
	}
	h.taken = taken
	return reply, nil
}

func TestRuntimeInputClient_ExternalInputAcceptedReplySucceedsWhenAnotherWriterTakesTheReleasedLock(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		request := ordinaryInput(run, "lock-taken-after-reply")
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		attempt, err := newPreparedSubmission(request, until, systemClock{})
		if err != nil {
			t.Fatal(err)
		}
		held := holdPaneLock(t, s, p)
		inner := newSocketHost(t, s, p, until)
		inner.expiration = some(attempt.expiration.monotonicNs())
		host := &lockTakingHost{socketHost: inner}
		result, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, s, run, p, held, attempt, deadlineUntil(until), host)
		if failure != nil {
			t.Fatalf("a submission the runner accepted succeeds without the reacquired lock: %v", failure.asError())
		}
		found, err := findInput(ctx, f.fixture.conn, run.runID, request.OperationKey)
		if err != nil {
			t.Fatal(err)
		}
		row, ok := found.get()
		if !ok || result.Status != submissionStatusAccepted || result.InputID != some(row.inputID) {
			t.Fatalf("the runner committed the submission receipt: %+v", result)
		}
		host.taken.close()
		held.close()
		waitInputServiceRecord(t, f, request.Literal, outer)
	})
}

func TestRuntimeInputClient_ExternalInputFullRunAndPlacementRevalidateBeforeEffectsAndAfterHealthWait(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		request := ordinaryInput(run, "identity")
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		d := deadlineUntil(until)
		attempt, err := newPreparedSubmission(request, until, systemClock{})
		if err != nil {
			t.Fatal(err)
		}
		held := holdPaneLock(t, s, p)
		foreignRun := run
		foreignRun.directorMemberID++
		if _, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, s, foreignRun, p, held, attempt, d, &noLiveHost{}); failure == nil {
			t.Fatal("a changed run identity is refused")
		}
		host := newSocketHost(t, s, p, until)
		host.afterHealth = func() {
			conn, err := connect(s.databaseURL)
			if err != nil {
				t.Fatal(err)
			}
			defer closeConnection(t, conn)
			if changed, err := sqlExecute(conn, "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1", run.memberID); err != nil || changed != 1 {
				t.Errorf("moving the placement changed %d rows: %v", changed, err)
			}
		}
		_, failure := submitWrappedInputUnderPaneLockWithHost(ctx, f.fixture.conn, s, run, p, held, attempt, d, host)
		if changed, err := sqlExecute(f.fixture.conn, "UPDATE member_placements SET mux_session=?1 WHERE member_id=?2", p.muxSession, run.memberID); err != nil || changed != 1 {
			t.Fatalf("restoring the placement changed %d rows: %v", changed, err)
		}
		if failure == nil || host.calls != [3]int{0, 1, 0} {
			t.Fatalf("a moved placement is refused after the health wait: %v %v", failure, host.calls)
		}
		if found, err := findInput(ctx, f.fixture.conn, run.runID, request.OperationKey); err != nil || found.set {
			t.Fatalf("the refusal reserves nothing: %v", err)
		}
		held.close()
		inputServiceBarrier(t, f, outer)
		if slices.Contains(recordLines(t, f), request.Literal) {
			t.Fatal("the refused literal never reaches the native child")
		}
	})
}

func TestRuntimeInputClient_ExternalInputTypedSuccessRequiresTheActualSuppliedConnectionReceipt(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		if _, err := sqlExecute(f.fixture.conn, "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1", run.runID); err != nil {
			t.Fatal(err)
		}
		before := mustRun(t, f.fixture.conn, run.runID)
		records := healthRecords(t, f.fixture.conn)
		var updated atomic.Bool
		if err := f.fixture.conn.raw.SetAuthorizer(func(action sqlite3.AuthorizerActionCode, table, _, _, _ string) sqlite3.AuthorizerReturnCode {
			if action == sqlite3.AUTH_UPDATE && table == "member_runs" {
				updated.Store(true)
			}
			return sqlite3.AUTH_OK
		}); err != nil {
			t.Fatal(err)
		}
		request := ordinaryInput(run, "unverified-reply")
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		host := newSocketHost(t, s, p, until)
		host.reply = socketReplyUnknownID
		_, failure := submitWrappedInputWithHost(ctx, f.fixture.conn, s, run, p, request, deadlineUntil(until), host)
		removed := f.fixture.conn.raw.SetAuthorizer(nil)
		if failure == nil {
			t.Fatalf("unverified reply rejects; authorizer_cleanup=%v", removed)
		}
		if removed != nil {
			t.Fatal(removed)
		}
		if host.calls != [3]int{1, 1, 1} || failure.context.runID != some(run.runID) || failure.context.operationKey != some(request.OperationKey) {
			t.Fatalf("a typed success alone proves no durable identity: %v %+v", host.calls, failure.context)
		}
		if strings.Contains(failure.source.message(), request.Literal) {
			t.Fatal("the refusal keeps the literal private")
		}
		if !updated.Load() {
			t.Fatal("genuine successful health updates the supplied connection")
		}
		refreshed := mustRun(t, f.fixture.conn, run.runID)
		if at, set := refreshed.lastHealthAt.get(); !set || at == "2000-01-01T00:00:00Z" {
			t.Fatalf("the health stamp advances: %v", refreshed.lastHealthAt)
		}
		expected := before
		expected.lastHealthAt = refreshed.lastHealthAt
		if !refreshed.equal(expected) {
			t.Fatalf("health preserves lifecycle, coverage and event reservation: %+v", refreshed)
		}
		if !tableSnapshotsEqual(healthRecords(t, f.fixture.conn), records) {
			t.Fatal("health writes no event record")
		}
		if found, err := findInput(ctx, f.fixture.conn, run.runID, request.OperationKey); err != nil || found.set {
			t.Fatalf("the refusal reserves nothing: %v", err)
		}
		inputServiceBarrier(t, f, outer)
		if slices.Contains(recordLines(t, f), request.Literal) {
			t.Fatal("the refused literal never reaches the native child")
		}
	})
}

func TestRuntimeInputClient_ExternalInputReadyHistoryReachesLiveServiceWithoutInventingZeroByteAuthority(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		request := ordinaryInput(run, "ready-without-live-proof")
		reserved, err := reserveInput(ctx, f.fixture.conn, run.owner(), newInput{operationKey: request.OperationKey, kind: request.InputKind, payloadDigest: inputPayloadDigest(run.runID, request)})
		if err != nil {
			t.Fatal(err)
		}
		row, err := recordInputState(ctx, f.fixture.conn, run.owner(), reserved.inputID, inputStateInProgress, inputStateReady)
		if err != nil {
			t.Fatal(err)
		}
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		host := newSocketHost(t, s, p, until)
		result, failure := submitWrappedInputWithHost(ctx, f.fixture.conn, s, run, p, request, deadlineUntil(until), host)
		if failure != nil {
			t.Fatal(failure.asError())
		}
		if host.calls != [3]int{1, 1, 1} || result.Status != submissionStatusHeld || result.Stage != submissionStageZeroBytes || result.InputID != some(row.inputID) {
			t.Fatalf("calls %v result %+v", host.calls, result)
		}
		if got := storedInput(t, f.fixture.conn, row.inputID); got != row {
			t.Fatalf("the ready history stays %+v, got %+v", row, got)
		}
		inputServiceBarrier(t, f, outer)
		if slices.Contains(recordLines(t, f), request.Literal) {
			t.Fatal("the held literal never reaches the native child")
		}
	})
}

func TestRuntimeInputClient_ExternalInputDisconnectedReplyIsDiscoverableAndMatchingHistoryNeverRetypes(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		request := ordinaryInput(run, "disconnected")
		until := earliestTime(outer, time.Now().Add(realTimeBound))
		host := newSocketHost(t, s, p, until)
		host.reply = socketReplyDisconnected
		_, failure := submitWrappedInputWithHost(ctx, f.fixture.conn, s, run, p, request, deadlineUntil(until), host)
		if failure == nil {
			t.Fatal("the real disconnected consumer retains an error")
		}
		if host.calls != [3]int{1, 1, 1} || failure.context.runID != some(run.runID) || failure.context.operationKey != some(request.OperationKey) {
			t.Fatalf("calls %v context %+v", host.calls, failure.context)
		}
		if failure.source.kind != errorApp || failure.source.message() != "owned reply consumer closed" {
			t.Fatalf("source %#v", failure.source)
		}
		presented := failure.asError()
		if presented.kind != errorWithDiagnostics || presented.cause.exitCode() != 1 || presented.cause.kind != errorApp ||
			presented.cause.message() != "owned reply consumer closed" {
			t.Fatalf("a possible submission retains its original failure and context: %#v", presented)
		}
		if want := []string{fmt.Sprintf("run_id=%d operation_key=%s", run.runID, jsonQuoted(t, request.OperationKey))}; !slices.Equal(presented.diagnostics, want) {
			t.Fatalf("diagnostics %q, want %q", presented.diagnostics, want)
		}
		found, discovery := findSubmission(ctx, f.fixture.conn, permissionReceiptIdentityForRun(run), request, deadlineUntil(outer), systemClock{})
		if discovery != nil {
			t.Fatal(discovery.asError())
		}
		row, ok := found.get()
		if !ok || row.state != inputStateAccepted || row.payloadDigest != inputPayloadDigest(run.runID, request) {
			t.Fatalf("actual committed receipt survives reply loss: %+v", row)
		}
		result, failure := submitWrappedInputWithHost(ctx, f.fixture.conn, s, run, p, request, deadlineUntil(outer), &noLiveHost{})
		if failure != nil {
			t.Fatal(failure.asError())
		}
		if result.Status != submissionStatusAccepted || result.InputID != some(row.inputID) {
			t.Fatalf("result %+v", result)
		}
		if got := storedInput(t, f.fixture.conn, row.inputID); got != row {
			t.Fatalf("the history stays %+v", got)
		}
		waitInputServiceRecord(t, f, request.Literal, outer)
		inputServiceBarrier(t, f, outer)
		if count := len(slices.DeleteFunc(recordLines(t, f), func(line string) bool { return line != request.Literal })); count != 1 {
			t.Fatalf("matching history never retypes: %d", count)
		}
		inputServiceComposer(t, f, outer)
	})
}
