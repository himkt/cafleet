package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

const runtimeFixtureSecret = "fixture-secret-194-unique"

func createRun(ctx context.Context, conn *dbConn, spec newRun) (runRecord, error) {
	at := deadlineInheritedOrNew(ctx)
	return withDatabaseBudget(ctx, conn, at, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return inTransaction(conn, transactionImmediate, func() (runRecord, error) {
			run, err := insertRun(ctx, conn, spec)
			if err != nil {
				return runRecord{}, err
			}
			if _, err := at.remaining(); err != nil {
				return runRecord{}, err
			}
			return run, nil
		})
	})
}

type brokerRuntimeFixture struct {
	conn     *dbConn
	url      string
	fleet    int64
	director int64
	member   int64
	run      runRecord
}

func newBrokerRuntimeFixture(t *testing.T) *brokerRuntimeFixture {
	t.Helper()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "runtime-contract")
	member := register(t, conn, fleet, "worker", some("%2"))
	run, err := createRun(context.Background(), conn, brokerRuntimeFixtureSpec(t, conn, url, fleet, director, member))
	if err != nil {
		t.Fatal(err)
	}
	return &brokerRuntimeFixture{conn: conn, url: url, fleet: fleet, director: director, member: member, run: run}
}

func brokerRuntimeFixtureSpec(t *testing.T, conn *dbConn, url string, fleet, director, member int64) newRun {
	t.Helper()
	identity, err := databaseIdentity(context.Background(), conn)
	if err != nil {
		t.Fatal(err)
	}
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	return newRun{
		memberID:             member,
		fleetID:              fleet,
		directorMemberID:     director,
		backend:              "codex",
		databaseIdentityJSON: identity,
		launchNonce:          strings.Repeat("a", 64),
		runtimePath:          filepath.Join(filepath.Dir(path), ".cafleet-runs/fixture/run"),
		requestedMode:        none[string](),
		resolvedMode:         "never",
		modeSource:           modeSourceDefault,
		permissionArgvJSON:   jsontext.Value(`["--ask-for-approval","never","--sandbox","workspace-write"]`),
		nativeVersion:        some("0.163.0"),
		coverage:             coverageUnknown,
		ownerPID:             int64(os.Getpid()),
		ownerStartIdentity:   "fixture-start-identity",
	}
}

func (f *brokerRuntimeFixture) reload(t *testing.T) runOwner {
	t.Helper()
	run, err := getRun(context.Background(), f.conn, f.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	f.run = run
	return run.owner()
}

func (f *brokerRuntimeFixture) activate(t *testing.T) runOwner {
	t.Helper()
	if _, err := f.conn.execute("UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2 WHERE run_id=?1", f.run.runID); err != nil {
		t.Fatal(err)
	}
	return f.reload(t)
}

func (f *brokerRuntimeFixture) activateNextEpoch(t *testing.T) runOwner {
	t.Helper()
	if _, err := f.conn.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", f.run.runID); err != nil {
		t.Fatal(err)
	}
	return f.reload(t)
}

func (f *brokerRuntimeFixture) recoverer(t *testing.T) runOwner {
	t.Helper()
	if _, err := f.conn.execute("UPDATE member_runs SET status='recovering',owner_kind='reconciler',owner_epoch=owner_epoch+1 WHERE run_id=?1", f.run.runID); err != nil {
		t.Fatal(err)
	}
	return f.reload(t)
}

func (f *brokerRuntimeFixture) reopen(t *testing.T) *dbConn {
	t.Helper()
	return fileConn(t, f.url)
}

func runtimeTableRows(t *testing.T, conn *dbConn, table string) [][]any {
	t.Helper()
	return tableRows(t, conn, "SELECT * FROM "+table+" ORDER BY 1")
}

func durableState(t *testing.T, conn *dbConn) [][][]any {
	t.Helper()
	tables := []string{"member_runs", "member_interactions", "member_runtime_events", "member_input_operations", "messages"}
	state := make([][][]any, len(tables))
	for index, table := range tables {
		state[index] = runtimeTableRows(t, conn, table)
	}
	return state
}

func durableStatesEqual(left, right [][][]any) bool {
	return slices.EqualFunc(left, right, rowsEqual)
}

// withInstanceID is identity with its instance_id replaced by id.
func withInstanceID(t *testing.T, identity jsontext.Value, id string) jsontext.Value {
	t.Helper()
	var instance string
	if err := identityInstanceID(identity, &instance); err != nil {
		t.Fatal(err)
	}
	replaced := bytes.Replace(identity, []byte(`"`+instance+`"`), []byte(`"`+id+`"`), 1)
	if bytes.Equal(replaced, identity) {
		t.Fatalf("identity %s carries its instance_id once", identity)
	}
	return replaced
}

func identityInstanceID(identity jsontext.Value, instance *string) error {
	decoder := jsontext.NewDecoder(bytes.NewReader(identity))
	if _, err := decoder.ReadToken(); err != nil {
		return err
	}
	for decoder.PeekKind() != '}' {
		token, err := decoder.ReadToken()
		if err != nil {
			return err
		}
		name := token.String()
		value, err := decoder.ReadValue()
		if err != nil {
			return err
		}
		if name == "instance_id" {
			token := jsontext.NewDecoder(bytes.NewReader(value))
			text, err := token.ReadToken()
			if err != nil {
				return err
			}
			*instance = text.String()
			return nil
		}
	}
	return fmt.Errorf("identity %s has no instance_id", identity)
}

func TestBrokerRuntime_RunCreationRecordsTheDefaultOrExplicitModeAndUnreachedPhases(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name      string
		requested opt[string]
		resolved  string
		source    modeSource
	}{
		{name: "the backend default", requested: none[string](), resolved: "never", source: modeSourceDefault},
		{name: "an explicit mode", requested: some("on-request"), resolved: "on-request", source: modeSourceExplicit},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			fixture := newBrokerRuntimeFixture(t)
			member := register(t, fixture.conn, fixture.fleet, "worker-run", none[string]())
			spec := brokerRuntimeFixtureSpec(t, fixture.conn, fixture.url, fixture.fleet, fixture.director, member)
			spec.requestedMode = tc.requested
			spec.resolvedMode = tc.resolved
			spec.modeSource = tc.source
			spec.permissionArgvJSON = jsontext.Value(`["--ask-for-approval","` + tc.resolved + `"]`)
			created, err := createRun(ctx, fixture.conn, spec)
			if err != nil {
				t.Fatal(err)
			}
			run, err := getRun(ctx, fixture.conn, created.runID)
			if err != nil {
				t.Fatal(err)
			}
			if run.requestedMode != tc.requested || run.resolvedMode != tc.resolved || run.modeSource != tc.source {
				t.Fatalf("%s: mode = %v %q %v", tc.name, run.requestedMode, run.resolvedMode, run.modeSource)
			}
			if !jsonEqual(run.permissionArgvJSON, spec.permissionArgvJSON) {
				t.Fatalf("%s: permission argv = %s", tc.name, run.permissionArgvJSON)
			}
			if run.status != runStatusStarting || run.ownerEpoch != 1 || run.ownerKind != ownerKindCreator ||
				run.cleanupState != cleanupStateIncomplete || run.childPID != none[int64]() ||
				run.exitCode != none[int64]() || run.coverage != coverageUnknown {
				t.Fatalf("%s: unreached phases = %+v", tc.name, run)
			}
		})
	}
}

func TestBrokerRuntime_CorruptOrMissingRequiredStorageFailsItsTypedReader(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name       string
		corruption string
		field      string
	}{
		{name: "status", corruption: "PRAGMA ignore_check_constraints=ON; UPDATE member_runs SET status='invented'", field: "status"},
		{name: "database identity", corruption: "PRAGMA ignore_check_constraints=ON; UPDATE member_runs SET database_identity_json='[]'", field: "database_identity_json"},
		{name: "instance identity", corruption: "DELETE FROM runtime_database_identity"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			fixture := newBrokerRuntimeFixture(t)
			mustExecBatch(t, fixture.conn, tc.corruption)
			var err error
			if tc.field == "" {
				_, err = databaseIdentity(ctx, fixture.conn)
			} else {
				_, err = getRun(ctx, fixture.conn, fixture.run.runID)
			}
			if err == nil || !strings.Contains(err.Error(), tc.field) {
				t.Fatalf("%s: reader error %v, want one naming %q", tc.name, err, tc.field)
			}
		})
	}
}

func TestBrokerRuntime_RunCreationValidatesMemberFleetAndPinnedDirectorBeforeAllocating(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	otherFleet, otherDirector := createFleetFixture(t, fixture.conn, "foreign")
	member := register(t, fixture.conn, fixture.fleet, "new-worker", none[string]())
	for _, mismatch := range []string{"member", "fleet", "director", "database"} {
		spec := brokerRuntimeFixtureSpec(t, fixture.conn, fixture.url, fixture.fleet, fixture.director, member)
		switch mismatch {
		case "member":
			spec.memberID = otherDirector
		case "fleet":
			spec.fleetID = otherFleet
		case "director":
			spec.directorMemberID = otherDirector
		case "database":
			spec.databaseIdentityJSON = withInstanceID(t, spec.databaseIdentityJSON, strings.Repeat("0", 32))
		}
		before := durableState(t, fixture.conn)
		if _, err := createRun(ctx, fixture.conn, spec); err == nil {
			t.Fatalf("%s: a mismatched run is refused", mismatch)
		}
		if !durableStatesEqual(durableState(t, fixture.conn), before) {
			t.Fatalf("%s: a refused run changed durable state", mismatch)
		}
	}
}

func TestBrokerRuntime_HistoricalOwnershipRequiresOneFleetRelationshipAndSelectsTheArchiveReason(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	run := fixture.run
	if reason, err := historicalOwnership(ctx, fixture.conn, run); err != nil || reason != none[archiveReason]() {
		t.Fatalf("a live run's ownership = %v %v", reason, err)
	}
	otherFleet, otherDirector := createFleetFixture(t, fixture.conn, "other")
	foreignMember := register(t, fixture.conn, otherFleet, "foreign", none[string]())
	peer := register(t, fixture.conn, run.fleetID, "peer", none[string]())
	changes := []struct {
		conjunct string
		change   func(*runRecord)
	}{
		{"member fleet", func(r *runRecord) { r.memberID = foreignMember }},
		{"director fleet", func(r *runRecord) { r.directorMemberID = otherDirector }},
		{"fleet director", func(r *runRecord) { r.directorMemberID = peer }},
		{"member is director", func(r *runRecord) { r.memberID = run.directorMemberID }},
	}
	for _, tc := range changes {
		changed := run
		tc.change(&changed)
		if _, err := historicalOwnership(ctx, fixture.conn, changed); err == nil || !strings.Contains(err.Error(), "member/fleet/Director mismatch") {
			t.Fatalf("%s: historicalOwnership = %v", tc.conjunct, err)
		}
	}
	reasons := []struct {
		sql    string
		id     int64
		reason archiveReason
	}{
		{"UPDATE members SET status='deregistered' WHERE member_id=?1", run.directorMemberID, archiveReasonDirectorInactive},
		{"UPDATE members SET status='deregistered' WHERE member_id=?1", run.memberID, archiveReasonMemberInactive},
		{"UPDATE fleets SET deleted_at=created_at WHERE fleet_id=?1", run.fleetID, archiveReasonFleetDeleted},
	}
	for _, tc := range reasons {
		if _, err := fixture.conn.execute(tc.sql, tc.id); err != nil {
			t.Fatal(err)
		}
		if reason, err := historicalOwnership(ctx, fixture.conn, run); err != nil || reason != some(tc.reason) {
			t.Fatalf("%v: historicalOwnership = %v %v", tc.reason, reason, err)
		}
	}
}

type diagnosticRow struct {
	source, message, firstAt, lastAt string
	count                            int64
}

func diagnosticStamp(second int) string {
	return fmt.Sprintf("2026-11-21T12:%02d:%02d.000000+00:00", second/60, second%60)
}

func diagnosticEntry(source runnerDiagnosticSource, message string, first, last int, count int64) runnerDiagnosticEntry {
	return runnerDiagnosticEntry{source: source, message: message, firstAt: diagnosticStamp(first), lastAt: diagnosticStamp(last), count: count}
}

const diagnosticWriteAt = 30

func writeDiagnostics(t *testing.T, fixture *brokerRuntimeFixture, runID int64, entries []runnerDiagnosticEntry, dropped uint64) {
	t.Helper()
	_, err := inTransaction(fixture.conn, transactionDeferred, func() (struct{}, error) {
		return struct{}{}, writeRunnerDiagnostics(fixture.conn, runID, entries, dropped, diagnosticStamp(diagnosticWriteAt))
	})
	if err != nil {
		t.Fatal(err)
	}
}

func diagnosticRows(t *testing.T, fixture *brokerRuntimeFixture, runID int64) []diagnosticRow {
	t.Helper()
	var rows []diagnosticRow
	for _, row := range tableRows(t, fixture.conn, "SELECT source,message,first_at,last_at,count FROM member_runner_diagnostics WHERE run_id=?1 ORDER BY diagnostic_id", runID) {
		rows = append(rows, diagnosticRow{row[0].(string), row[1].(string), row[2].(string), row[3].(string), row[4].(int64)})
	}
	return rows
}

func secondRun(t *testing.T, fixture *brokerRuntimeFixture) int64 {
	t.Helper()
	member := register(t, fixture.conn, fixture.fleet, "second-worker", some("%3"))
	spec := brokerRuntimeFixtureSpec(t, fixture.conn, fixture.url, fixture.fleet, fixture.director, member)
	spec.launchNonce = strings.Repeat("b", 64)
	spec.runtimePath = strings.Replace(spec.runtimePath, "/fixture/", "/second/", 1)
	run, err := createRun(context.Background(), fixture.conn, spec)
	if err != nil {
		t.Fatal(err)
	}
	return run.runID
}

func TestBrokerRuntime_RunnerDiagnostics_EveryRunnerDiagnosticSourceIsStoredUnderItsName(t *testing.T) {
	t.Parallel()
	sources := []struct {
		source runnerDiagnosticSource
		name   string
	}{
		{runnerDiagnosticSourceEventWriter, "event_writer"},
		{runnerDiagnosticSourceObservation, "observation"},
		{runnerDiagnosticSourceDeferredInput, "deferred_input"},
		{runnerDiagnosticSourceQueueHousekeeping, "queue_housekeeping"},
		{runnerDiagnosticSourcePreviewSweep, "preview_sweep"},
		{runnerDiagnosticSourceDirectorDelivery, "director_delivery"},
		{runnerDiagnosticSourceInputResult, "input_result"},
		{runnerDiagnosticSourcePermissionAnswer, "permission_answer"},
		{runnerDiagnosticSourceShutdown, "shutdown"},
	}
	for _, tc := range sources {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			fixture := newBrokerRuntimeFixture(t)
			runID := fixture.run.runID
			writeDiagnostics(t, fixture, runID, []runnerDiagnosticEntry{diagnosticEntry(tc.source, "label: fixture diagnostic", 1, 1, 1)}, 0)
			if tc.source.asStr() != tc.name {
				t.Fatalf("asStr = %q, want %q", tc.source.asStr(), tc.name)
			}
			if rows := diagnosticRows(t, fixture, runID); rows[0].source != tc.name {
				t.Fatalf("%s: stored source %q", tc.name, rows[0].source)
			}
			latest, err := latestRunnerDiagnostics(context.Background(), fixture.conn, runID)
			if err != nil || latest[0].source != tc.source {
				t.Fatalf("%s: latest = %+v %v", tc.name, latest, err)
			}
		})
	}
}

func TestBrokerRuntime_RunnerDiagnostics_ARepeatedSourceAndMessageAddsItsCountAndMovesLastAt(t *testing.T) {
	t.Parallel()
	fixture := newBrokerRuntimeFixture(t)
	runID := fixture.run.runID
	writeDiagnostics(t, fixture, runID, []runnerDiagnosticEntry{
		diagnosticEntry(runnerDiagnosticSourceEventWriter, "label: busy", 1, 2, 1),
		diagnosticEntry(runnerDiagnosticSourceObservation, "label: busy", 1, 3, 2),
	}, 0)
	writeDiagnostics(t, fixture, runID, []runnerDiagnosticEntry{diagnosticEntry(runnerDiagnosticSourceEventWriter, "label: busy", 10, 12, 3)}, 0)
	want := []diagnosticRow{
		{"event_writer", "label: busy", diagnosticStamp(1), diagnosticStamp(12), 4},
		{"observation", "label: busy", diagnosticStamp(1), diagnosticStamp(3), 2},
	}
	if got := diagnosticRows(t, fixture, runID); !slices.Equal(got, want) {
		t.Fatalf("rows = %+v, want %+v", got, want)
	}
}

func TestBrokerRuntime_RunnerDiagnostics_DroppedEntriesAddOneOverflowRowNamingTheCount(t *testing.T) {
	t.Parallel()
	for _, dropped := range []uint64{0, 3} {
		t.Run(fmt.Sprintf("dropped=%d", dropped), func(t *testing.T) {
			t.Parallel()
			fixture := newBrokerRuntimeFixture(t)
			runID := fixture.run.runID
			writeDiagnostics(t, fixture, runID, []runnerDiagnosticEntry{diagnosticEntry(runnerDiagnosticSourceShutdown, "label: exit", 1, 1, 1)}, dropped)
			var overflow []diagnosticRow
			for _, row := range diagnosticRows(t, fixture, runID) {
				if strings.HasPrefix(row.message, "diagnostic buffer overflow") {
					overflow = append(overflow, row)
				}
			}
			if dropped == 0 {
				if len(overflow) != 0 {
					t.Fatalf("overflow rows = %+v", overflow)
				}
				return
			}
			want := diagnosticRow{"event_writer", "diagnostic buffer overflow: 3 entries dropped", diagnosticStamp(diagnosticWriteAt), diagnosticStamp(diagnosticWriteAt), 1}
			if len(overflow) != 1 || overflow[0] != want {
				t.Fatalf("one overflow row carrying the caller's stamp, got %+v", overflow)
			}
		})
	}
}

func TestBrokerRuntime_RunnerDiagnostics_EachRunKeepsOnlyItsNewest256Rows(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name           string
		total          int
		distinctLastAt bool
	}{
		{name: "older last_at rows are deleted first", total: 260, distinctLastAt: true},
		{name: "a last_at tie deletes the lower diagnostic_id first", total: 258},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			fixture := newBrokerRuntimeFixture(t)
			runID := fixture.run.runID
			other := secondRun(t, fixture)
			writeDiagnostics(t, fixture, other, []runnerDiagnosticEntry{diagnosticEntry(runnerDiagnosticSourceShutdown, "label: other run", 0, 0, 1)}, 0)
			entries := make([]runnerDiagnosticEntry, tc.total)
			for index := range entries {
				last := 1
				if tc.distinctLastAt {
					last = index + 1
				}
				entries[index] = diagnosticEntry(runnerDiagnosticSourceEventWriter, fmt.Sprintf("label: entry %03d", index), 0, last, 1)
			}
			writeDiagnostics(t, fixture, runID, entries, 0)
			kept := diagnosticRows(t, fixture, runID)
			if len(kept) != 256 {
				t.Fatalf("%s: kept %d rows", tc.name, len(kept))
			}
			if want := fmt.Sprintf("label: entry %03d", tc.total-256); kept[0].message != want {
				t.Fatalf("%s: the oldest rows are pruned: first kept %q, want %q", tc.name, kept[0].message, want)
			}
			if want := fmt.Sprintf("label: entry %03d", tc.total-1); kept[len(kept)-1].message != want {
				t.Fatalf("%s: last kept %q, want %q", tc.name, kept[len(kept)-1].message, want)
			}
			if got := len(diagnosticRows(t, fixture, other)); got != 1 {
				t.Fatalf("%s: other runs keep theirs, got %d", tc.name, got)
			}
		})
	}
}

func TestBrokerRuntime_RunnerDiagnostics_LatestRunnerDiagnosticsReturnsTheRunsNewestFiveByLastAtThenId(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	runID := fixture.run.runID
	other := secondRun(t, fixture)
	otherEntry := diagnosticEntry(runnerDiagnosticSourceShutdown, "label: other run", 99, 99, 1)
	writeDiagnostics(t, fixture, other, []runnerDiagnosticEntry{otherEntry}, 0)
	writeDiagnostics(t, fixture, runID, []runnerDiagnosticEntry{
		diagnosticEntry(runnerDiagnosticSourceEventWriter, "label: a", 1, 1, 1),
		diagnosticEntry(runnerDiagnosticSourceObservation, "label: b", 1, 7, 2),
		diagnosticEntry(runnerDiagnosticSourceDeferredInput, "label: c", 2, 3, 1),
		diagnosticEntry(runnerDiagnosticSourcePreviewSweep, "label: d", 4, 5, 1),
		diagnosticEntry(runnerDiagnosticSourceInputResult, "label: e", 5, 5, 3),
		diagnosticEntry(runnerDiagnosticSourceShutdown, "label: f", 6, 6, 1),
		diagnosticEntry(runnerDiagnosticSourceEventWriter, "label: g", 2, 2, 1),
	}, 0)
	want := []runnerDiagnosticEntry{
		diagnosticEntry(runnerDiagnosticSourceObservation, "label: b", 1, 7, 2),
		diagnosticEntry(runnerDiagnosticSourceShutdown, "label: f", 6, 6, 1),
		diagnosticEntry(runnerDiagnosticSourceInputResult, "label: e", 5, 5, 3),
		diagnosticEntry(runnerDiagnosticSourcePreviewSweep, "label: d", 4, 5, 1),
		diagnosticEntry(runnerDiagnosticSourceDeferredInput, "label: c", 2, 3, 1),
	}
	if got, err := latestRunnerDiagnostics(ctx, fixture.conn, runID); err != nil || !slices.Equal(got, want) {
		t.Fatalf("latest = %+v %v, want %+v", got, err, want)
	}
	if got, err := latestRunnerDiagnostics(ctx, fixture.conn, other); err != nil || !slices.Equal(got, []runnerDiagnosticEntry{otherEntry}) {
		t.Fatalf("other run latest = %+v %v", got, err)
	}
}

func reserveInput(ctx context.Context, conn *dbConn, owner runOwner, request newInput) (inputOperation, error) {
	reservation, err := reserveInputWithProvenance(ctx, conn, owner, request, newReservationPlain)
	if err != nil {
		return inputOperation{}, err
	}
	return reservation.operation, nil
}

func settleInput(ctx context.Context, conn *dbConn, owner runOwner, inputID int64, expected inputState, disposition inputDisposition, evidenceRef string) (inputOperation, error) {
	return ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (inputOperation, error) {
		return settleInputInTransaction(ctx, conn, run, inputID, expected, disposition, evidenceRef)
	})
}

func (f *brokerRuntimeFixture) open(t *testing.T, owner runOwner) runtimeEventRecord {
	t.Helper()
	return f.openAs(t, owner, interactionClassPermissionRequest)
}

func (f *brokerRuntimeFixture) openAs(t *testing.T, owner runOwner, classification interactionClass) runtimeEventRecord {
	t.Helper()
	transition := opening(t, f.run.runID, 1, 1, runtimeMetadata(t))
	transition.Classification = some(classification)
	event, _, err := persistTransition(context.Background(), f.conn, owner, transition, persistenceModeLive)
	if err != nil {
		t.Fatal(err)
	}
	return event
}

func (f *brokerRuntimeFixture) input(t *testing.T, owner runOwner, key string, kind inputKind) inputOperation {
	t.Helper()
	operation, err := reserveInput(context.Background(), f.conn, owner, newInput{operationKey: key, kind: kind, payloadDigest: strings.Repeat("d", 64)})
	if err != nil {
		t.Fatal(err)
	}
	return operation
}

func rawEvidence(t *testing.T, text string) jsontext.Value {
	t.Helper()
	return mustJSON(t, jsonObject{
		{"backend", "codex"}, {"native_version", "0.163.0"},
		{"rule_id", "native-tool-permission"}, {"screen_seq", 120},
		{"operation", "run " + text}, {"operation_rows", 1}, {"choices", []any{"allow " + text, "deny"}},
		{"excerpt", "request " + text},
		{"semantic_fingerprint", "fingerprint-" + text},
		{"truncated", false},
	})
}

func runtimeMetadata(t *testing.T) jsontext.Value {
	t.Helper()
	evidence, err := metadataEvidence(rawEvidence(t, runtimeFixtureSecret))
	if err != nil {
		t.Fatal(err)
	}
	return evidence
}

func opening(t *testing.T, run, eventSeq, episodeSeq int64, evidence jsontext.Value) observedTransition {
	t.Helper()
	return observedTransition{
		RunID:          run,
		EventSeq:       eventSeq,
		EpisodeSeq:     some(episodeSeq),
		Kind:           runtimeEventKindInteractionOpened,
		Classification: some(interactionClassPermissionRequest),
		ObservedAt:     runtimeFixtureNow,
		EvidenceJSON:   evidence,
		CloseReason:    none[interactionCloseReason](),
	}
}

func closure(t *testing.T, run, eventSeq, episodeSeq int64) observedTransition {
	t.Helper()
	transition := opening(t, run, eventSeq, episodeSeq, runtimeMetadata(t))
	transition.Kind = runtimeEventKindInteractionClosed
	transition.CloseReason = some(interactionCloseReasonNotVisible)
	return transition
}

func replacePublicPayload(t *testing.T, conn *dbConn, event runtimeEventRecord, payload jsontext.Value) {
	t.Helper()
	_, err := inTransaction(conn, transactionDeferred, func() (struct{}, error) {
		if _, err := conn.execute("UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2", string(payload), event.eventID); err != nil {
			return struct{}{}, err
		}
		if messageID, ok := event.messageID.get(); ok {
			if _, err := conn.execute("UPDATE messages SET text=?1 WHERE message_id=?2", string(payload), messageID); err != nil {
				return struct{}{}, err
			}
		}
		return struct{}{}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func mustJSON(t *testing.T, value any) jsontext.Value {
	t.Helper()
	return jsontext.Value(mustEncodeJSON(t, value))
}

type orderedJSONMember struct {
	name  string
	value jsontext.Value
}

func testJSONMembers(t *testing.T, document jsontext.Value) []orderedJSONMember {
	t.Helper()
	decoder := jsontext.NewDecoder(bytes.NewReader(document))
	open, err := decoder.ReadToken()
	if err != nil || open.Kind() != '{' {
		t.Fatalf("%s is a JSON object: %v", document, err)
	}
	var members []orderedJSONMember
	for decoder.PeekKind() != '}' {
		token, err := decoder.ReadToken()
		if err != nil {
			t.Fatal(err)
		}
		name := token.String()
		value, err := decoder.ReadValue()
		if err != nil {
			t.Fatal(err)
		}
		members = append(members, orderedJSONMember{name: name, value: slices.Clone(value)})
	}
	return members
}

func testJSONFromMembers(members []orderedJSONMember) jsontext.Value {
	var out bytes.Buffer
	out.WriteByte('{')
	for index, member := range members {
		if index > 0 {
			out.WriteByte(',')
		}
		name, err := jsontext.AppendQuote(nil, member.name)
		if err != nil {
			panic(err)
		}
		out.Write(name)
		out.WriteByte(':')
		out.Write(member.value)
	}
	out.WriteByte('}')
	return out.Bytes()
}

// testJSONKeys is the member names of a JSON object in document order.
func testJSONKeys(t *testing.T, document jsontext.Value) []string {
	t.Helper()
	var keys []string
	for _, member := range testJSONMembers(t, document) {
		keys = append(keys, member.name)
	}
	return keys
}

// jsonAt is the value at the object path, or JSON null when a key is absent, as serde_json
// indexing reads it.
func jsonAt(t *testing.T, document jsontext.Value, path ...string) jsontext.Value {
	t.Helper()
	current := document
	for _, key := range path {
		if current.Kind() != '{' {
			return jsontext.Value("null")
		}
		found := jsontext.Value("null")
		for _, member := range testJSONMembers(t, current) {
			if member.name == key {
				found = member.value
			}
		}
		current = found
	}
	return current
}

func jsonHas(t *testing.T, document jsontext.Value, key string) bool {
	t.Helper()
	return slices.Contains(testJSONKeys(t, document), key)
}

// testJSONWith sets the value at the object path, replacing a present key in place and appending an
// absent one, as serde_json index assignment does.
func testJSONWith(t *testing.T, document jsontext.Value, value jsontext.Value, path ...string) jsontext.Value {
	t.Helper()
	if len(path) == 0 {
		return value
	}
	members := testJSONMembers(t, document)
	for index, member := range members {
		if member.name == path[0] {
			members[index].value = testJSONWith(t, member.value, value, path[1:]...)
			return testJSONFromMembers(members)
		}
	}
	nested := value
	if len(path) > 1 {
		nested = testJSONWith(t, jsontext.Value("{}"), value, path[1:]...)
	}
	return testJSONFromMembers(append(members, orderedJSONMember{name: path[0], value: nested}))
}

func testJSONWithout(t *testing.T, document jsontext.Value, key string) jsontext.Value {
	t.Helper()
	return testJSONFromMembers(slices.DeleteFunc(testJSONMembers(t, document), func(member orderedJSONMember) bool { return member.name == key }))
}

func assertJSONEqual(t *testing.T, got, want jsontext.Value, context string) {
	t.Helper()
	if !jsonEqual(got, want) {
		t.Fatalf("%s:\n%s\nwant\n%s", context, got, want)
	}
}

func TestBrokerRuntime_AStaleOwnerKindEpochOrDatabaseIdentityFencesEveryOwnerOperation(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	if run, err := validateOwner(ctx, fixture.conn, owner); err != nil || run.runID != owner.runID {
		t.Fatalf("validateOwner = %+v %v", run, err)
	}
	stale := []struct {
		name  string
		stale func(*runOwner)
	}{
		{"epoch", func(o *runOwner) { o.ownerEpoch++ }},
		{"kind", func(o *runOwner) { o.ownerKind = ownerKindReconciler }},
		{"database", func(o *runOwner) {
			o.databaseIdentityJSON = withInstanceID(t, o.databaseIdentityJSON, strings.Repeat("0", 32))
		}},
	}
	for _, tc := range stale {
		changed := owner
		tc.stale(&changed)
		before := durableState(t, fixture.conn)
		if _, err := validateOwner(ctx, fixture.conn, changed); err == nil {
			t.Fatalf("%s: validateOwner accepts a stale owner", tc.name)
		}
		if _, _, err := persistTransition(ctx, fixture.conn, changed, opening(t, owner.runID, 1, 1, runtimeMetadata(t)), persistenceModeLive); err == nil {
			t.Fatalf("%s: persistTransition accepts a stale owner", tc.name)
		}
		if _, err := reserveInput(ctx, fixture.conn, changed, newInput{operationKey: "stale", kind: inputKindPrompt, payloadDigest: strings.Repeat("d", 64)}); err == nil {
			t.Fatalf("%s: reserveInput accepts a stale owner", tc.name)
		}
		if !durableStatesEqual(durableState(t, fixture.conn), before) {
			t.Fatalf("%s: a stale owner changed durable state", tc.name)
		}
	}
	epoch := owner
	epoch.ownerEpoch++
	if _, err := validateOwner(ctx, fixture.conn, epoch); err == nil || !strings.Contains(err.Error(), "stale ownership token") {
		t.Fatalf("validateOwner = %v", err)
	}
}
