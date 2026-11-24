package core

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ncruces/go-sqlite3"
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

// socketHost drives the actual pane lock, health probe and control socket, asserting the
// deadline and lock state each live call receives.
type socketHost struct {
	t           *testing.T
	settings    settings
	placement   placement
	until       time.Time
	calls       [3]int
	requests    []jsontext.Value
	healthReply jsontext.Value
}

func newSocketHost(t *testing.T, s settings, p placement, until time.Time) *socketHost {
	return &socketHost{t: t, settings: s, placement: p, until: until}
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
	if reply := h.healthReply; reply != nil {
		h.healthReply = nil
		return healthSnapshotDecode(reply, run)
	}
	return probeHealth(run, deadline, systemClock{})
}

func (h *socketHost) exchange(run runRecord, request inputControlRequest, deadline time.Time) (inputControlReply, error) {
	h.calls[2]++
	if !deadline.Equal(h.until) {
		h.t.Errorf("the exchange receives the original deadline, got %v", deadline)
	}
	encoded, err := encodeInputRequest(request)
	if err != nil {
		return nil, err
	}
	h.requests = append(h.requests, jsontext.Value(encoded[4:]))
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		return nil, err
	}
	client, err := controlClientConnect(paths.controlSocket, deadline, systemClock{})
	if err != nil {
		return nil, err
	}
	reply, err := client.exchangeInput(request, deadline, systemClock{})
	return reply, mergeCleanup(err, client.close())
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
	original := time.Now().Add(2 * time.Second)
	conn := history.fixture.conn
	reads := recordReadBudgets(t, conn, "member_input_operations")
	_, expired := reconcileReceiptWithHost(ctx, conn, history.settings, history.row.inputID, inputDispositionSubmitted,
		"fixture:replacement-evidence", deadlineUntil(time.Now()), &noLiveHost{})
	expiredReads := reads.take()
	var recorded inputOperation
	var historyFailure *inputFailure
	if _, err := withDatabaseBudget(ctx, conn, deadlineUntil(original), func(ctx context.Context, conn *dbConn) (struct{}, error) {
		recorded, historyFailure = reconcileReceiptWithHost(ctx, conn, history.settings, history.row.inputID, inputDispositionSubmitted,
			"fixture:replacement-evidence", deadlineUntil(original.Add(2*time.Second)), &noLiveHost{})
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
				deadlineUntil(time.Now().Add(time.Second)), systemClock{})
			if failure != nil {
				t.Fatalf("%s: %v", c.name, failure.asError())
			}
			if found != some(history.row) {
				t.Fatalf("%s: found %+v", c.name, found)
			}
			result, failure := submitWrappedInputWithHost(ctx, history.fixture.conn, history.settings, history.fixture.run,
				history.placement, history.request, deadlineUntil(time.Now().Add(150*time.Millisecond)), &noLiveHost{})
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
	controlDeadline := time.Now().Add(2 * time.Second)
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
	originalActive := time.Now().Add(time.Second)
	reads := recordReadBudgets(t, conn, "member_input_operations")
	_, expired := submitWrappedInputUnderPaneLockWithHost(ctx, conn, history.settings, history.fixture.run, history.placement,
		held, expiredAttempt, deadlineUntil(time.Now().Add(3*time.Second)), &noLiveHost{})
	expiredReads := reads.take()
	activeAttempt, err := newPreparedSubmission(history.request, originalActive, systemClock{})
	if err != nil {
		t.Fatal(err)
	}
	inherited := deadlineUntil(time.Now().Add(2 * time.Second))
	supplied := deadlineUntil(time.Now().Add(3 * time.Second))
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
	d := deadlineUntil(time.Now().Add(2 * time.Second))
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
			until := time.Now().Add(time.Second)
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
	}, deadlineUntil(time.Now().Add(time.Second)))
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
		"fixture:absent-placement-receipt", deadlineUntil(time.Now().Add(2*time.Second)), host)
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
