package core

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/ncruces/go-sqlite3"
)

func createFleetFixture(t *testing.T, conn *dbConn, name string) (int64, int64) {
	t.Helper()
	fleet, err := createFleet(context.Background(), conn, name, directorNewPlacement("%0"))
	if err != nil {
		t.Fatal(err)
	}
	return fleet.fleetID, fleet.director.memberID
}

var registrationTables = []string{"fleets", "members", "member_placements"}

func assertNoRegistrationRows(t *testing.T, conn *dbConn, label string) {
	t.Helper()
	for _, table := range registrationTables {
		if got := tableCount(t, conn, table); got != 0 {
			t.Fatalf("%s: %s holds %d rows", label, table, got)
		}
	}
}

func setAuthorizer(t *testing.T, conn *dbConn, decide func(action sqlite3.AuthorizerActionCode, name3rd, name4th string) sqlite3.AuthorizerReturnCode) {
	t.Helper()
	err := conn.raw.SetAuthorizer(func(action sqlite3.AuthorizerActionCode, name3rd, name4th, _, _ string) sqlite3.AuthorizerReturnCode {
		return decide(action, name3rd, name4th)
	})
	if err != nil {
		t.Fatal(err)
	}
}

func clearAuthorizer(t *testing.T, conn *dbConn) {
	t.Helper()
	if err := conn.raw.SetAuthorizer(nil); err != nil {
		t.Fatal(err)
	}
}

func denyTransaction(operation string) func(sqlite3.AuthorizerActionCode, string, string) sqlite3.AuthorizerReturnCode {
	return func(action sqlite3.AuthorizerActionCode, name3rd, _ string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_TRANSACTION && name3rd == operation {
			return sqlite3.AUTH_DENY
		}
		return sqlite3.AUTH_OK
	}
}

func TestBrokerFleets_RegistrationIsAtomicAtEveryStageAndPreservesIdentity(t *testing.T) {
	t.Parallel()
	cases := []struct {
		trigger      string
		factoryCalls int
	}{
		{trigger: "BEFORE INSERT ON fleets", factoryCalls: 0},
		{trigger: "BEFORE INSERT ON members", factoryCalls: 1},
		{trigger: "BEFORE INSERT ON member_placements", factoryCalls: 1},
		{trigger: "BEFORE UPDATE OF director_member_id ON fleets", factoryCalls: 1},
	}
	for _, tc := range cases {
		t.Run(tc.trigger, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			conn, _ := initializedFileConn(t)
			mustExecBatch(t, conn, "CREATE TRIGGER fail_registration "+tc.trigger+" BEGIN SELECT RAISE(ABORT, 'registration failure'); END;")
			calls := 0
			_, err := createFleetWith(ctx, conn, "alpha", func(fleetID int64) (newPlacement, error) {
				if fleetID != 1 {
					t.Errorf("%s: the factory receives fleet %d, want 1", tc.trigger, fleetID)
				}
				calls++
				return directorNewPlacement("%0"), nil
			})
			if err == nil || !strings.Contains(err.Error(), "registration failure") {
				t.Fatalf("%s: createFleetWith = %v", tc.trigger, err)
			}
			if calls != tc.factoryCalls {
				t.Fatalf("%s: factory calls = %d, want %d", tc.trigger, calls, tc.factoryCalls)
			}
			if !conn.raw.GetAutocommit() {
				t.Fatalf("%s: the failed registration leaves no transaction", tc.trigger)
			}
			assertNoRegistrationRows(t, conn, tc.trigger)
			mustExecBatch(t, conn, "DROP TRIGGER fail_registration")
			fleet, director := createFleetFixture(t, conn, "alpha")
			if fleet != 1 || director != 1 {
				t.Fatalf("%s: retried registration = (%d, %d), want (1, 1)", tc.trigger, fleet, director)
			}
			stored, err := getFleet(ctx, conn, fleet)
			record, ok := stored.get()
			if err != nil || !ok || record.directorMemberID != director {
				t.Fatalf("%s: getFleet = %+v %v", tc.trigger, stored, err)
			}
			member := requireMember(t, conn, director, fleet)
			if member.kind != memberKindDirector || member.placement.muxPaneID != some("%0") {
				t.Fatalf("%s: Director = %+v", tc.trigger, member)
			}
		})
	}
}

func TestBrokerFleets_PlacementFactoryReceivesTheAllocatedIdOnceAndPreservesExistingNames(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	legacy, _ := createFleetFixture(t, conn, "legacy")
	var seen []int64
	created, err := createFleetWith(ctx, conn, "alpha", func(fleetID int64) (newPlacement, error) {
		seen = append(seen, fleetID)
		director := directorNewPlacement("%5")
		pane, _ := director.pane.get()
		pane.muxWindowName = fmt.Sprintf("cafleet-%d-0", fleetID)
		director.pane = some(pane)
		return director, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(seen, []int64{2}) || created.fleetID != 2 || created.director.name != "Director" {
		t.Fatalf("seen %v, created %+v", seen, created)
	}
	if created.director.placement.muxWindowName != some("cafleet-2-0") {
		t.Fatalf("window name = %v", created.director.placement.muxWindowName)
	}
	recorded, err := directorPlacement(ctx, conn, created.fleetID)
	if err != nil || recorded != created.director.placement {
		t.Fatalf("directorPlacement = %+v %v, want %+v", recorded, err, created.director.placement)
	}
	legacyPlacement, err := directorPlacement(ctx, conn, legacy)
	if err != nil || legacyPlacement.muxWindowName != some("main") {
		t.Fatalf("legacy placement = %+v %v", legacyPlacement, err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("registration leaves no transaction")
	}
}

func TestBrokerFleets_PlacementFactoryFailureRollsBackRegistrationAndAllowsIdReuse(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	calls := 0
	_, err := createFleetWith(ctx, conn, "alpha", func(fleetID int64) (newPlacement, error) {
		if fleetID != 1 {
			t.Errorf("the factory receives fleet %d, want 1", fleetID)
		}
		calls++
		return newPlacement{}, appError("placement factory failure")
	})
	if calls != 1 || err == nil || !strings.Contains(err.Error(), "placement factory failure") {
		t.Fatalf("calls %d, error %v", calls, err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("the failed registration leaves no transaction")
	}
	assertNoRegistrationRows(t, conn, "after the factory failure")
	if fleet, _ := createFleetFixture(t, conn, "retry"); fleet != 1 {
		t.Fatalf("retried fleet = %d, want 1", fleet)
	}
}

func TestBrokerFleets_BothRegistrationEntryPointsAssertTheDirectorPlacementContract(t *testing.T) {
	t.Parallel()
	invalidations := []struct {
		name   string
		damage func(*newPlacement)
	}{
		{name: "pending pane", damage: func(p *newPlacement) { p.pane = none[placementPane]() }},
		{name: "overflow window", damage: func(p *newPlacement) { p.windowOrdinal = 1 }},
		{name: "right column", damage: func(p *newPlacement) { p.columnSide = columnSideRight }},
	}
	for _, factory := range []bool{false, true} {
		for _, invalid := range invalidations {
			name := fmt.Sprintf("factory=%v, %s", factory, invalid.name)
			t.Run(name, func(t *testing.T) {
				t.Parallel()
				ctx := context.Background()
				conn, _ := initializedFileConn(t)
				director := directorNewPlacement("%0")
				invalid.damage(&director)
				func() {
					defer func() {
						if recover() == nil {
							t.Errorf("%s: the invalid Director placement panics", name)
						}
					}()
					if factory {
						_, _ = createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) { return director, nil })
					} else {
						_, _ = createFleet(ctx, conn, "alpha", director)
					}
				}()
				if !conn.raw.GetAutocommit() {
					t.Fatalf("%s: the panic leaves no transaction", name)
				}
				assertNoRegistrationRows(t, conn, name)
			})
		}
	}
}

func TestBrokerFleets_TransactionAcquisitionFailureKeepsThePlacementFactoryUninvoked(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	setAuthorizer(t, conn, denyTransaction("BEGIN"))
	_, err := createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) {
		panic("a transaction is acquired before invoking the placement factory")
	})
	clearAuthorizer(t, conn)
	if err == nil || !strings.Contains(err.Error(), "not authorized") {
		t.Fatalf("createFleetWith = %v", err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("no transaction is open")
	}
	assertNoRegistrationRows(t, conn, "after the refused BEGIN")
}

func TestBrokerFleets_PlacementFailureAndRollbackFailureBothRemainVisible(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	setAuthorizer(t, conn, denyTransaction("ROLLBACK"))
	_, err := createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) {
		return newPlacement{}, appError("placement factory failure")
	})
	transactionOpen := !conn.raw.GetAutocommit()
	clearAuthorizer(t, conn)
	if transactionOpen {
		mustExecBatch(t, conn, "ROLLBACK")
	}
	if err == nil || !strings.Contains(err.Error(), "placement factory failure") || !strings.Contains(err.Error(), "not authorized") {
		t.Fatalf("createFleetWith = %v, want both failures", err)
	}
	if !transactionOpen {
		t.Fatal("the refused rollback leaves the transaction open")
	}
	assertNoRegistrationRows(t, conn, "after the manual rollback")
}

func TestBrokerFleets_ResultConstructionFailureRollsBackBeforeAnyCommitAttempt(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	var commits atomic.Int64
	conn.raw.CommitHook(func() bool {
		commits.Add(1)
		return true
	})
	setAuthorizer(t, conn, func(action sqlite3.AuthorizerActionCode, table, column string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_READ && table == "member_placements" && column == "backend" {
			return sqlite3.AUTH_DENY
		}
		return sqlite3.AUTH_OK
	})
	_, err := createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) { return directorNewPlacement("%0"), nil })
	clearAuthorizer(t, conn)
	conn.raw.CommitHook(nil)
	if err == nil || !strings.Contains(err.Error(), "prohibited") {
		t.Fatalf("createFleetWith = %v", err)
	}
	if got := commits.Load(); got != 0 {
		t.Fatalf("commit attempts = %d", got)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("no transaction is open")
	}
	assertNoRegistrationRows(t, conn, "after the refused read")
}

func TestBrokerFleets_AFailedCommitResolvesAnActiveTransactionAndPreservesRollbackErrors(t *testing.T) {
	t.Parallel()
	for _, denyRollback := range []bool{false, true} {
		t.Run(fmt.Sprintf("denyRollback=%v", denyRollback), func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			conn, _ := initializedFileConn(t)
			var mu sync.Mutex
			var attempts []string
			setAuthorizer(t, conn, func(action sqlite3.AuthorizerActionCode, operation, _ string) sqlite3.AuthorizerReturnCode {
				if action != sqlite3.AUTH_TRANSACTION {
					return sqlite3.AUTH_OK
				}
				mu.Lock()
				defer mu.Unlock()
				switch operation {
				case "COMMIT":
					attempts = append(attempts, "commit")
					return sqlite3.AUTH_DENY
				case "ROLLBACK":
					attempts = append(attempts, "rollback")
					if denyRollback {
						return sqlite3.AUTH_DENY
					}
				}
				return sqlite3.AUTH_OK
			})
			_, err := createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) { return directorNewPlacement("%0"), nil })
			transactionOpen := !conn.raw.GetAutocommit()
			clearAuthorizer(t, conn)
			if transactionOpen {
				mustExecBatch(t, conn, "ROLLBACK")
			}
			if err == nil || !strings.Contains(err.Error(), "not authorized") {
				t.Fatalf("createFleetWith = %v", err)
			}
			mu.Lock()
			observed := slices.Clone(attempts)
			mu.Unlock()
			if len(observed) == 0 || observed[0] != "commit" || !slices.Contains(observed[1:], "rollback") {
				t.Fatalf("transaction operations = %v", observed)
			}
			if transactionOpen != denyRollback {
				t.Fatalf("transaction open = %v, want %v", transactionOpen, denyRollback)
			}
			diagnostic := err.Error()
			if denyRollback {
				if !strings.Contains(strings.ToLower(diagnostic), "rollback") && !strings.Contains(diagnostic, "rolling back") {
					t.Fatalf("the rollback failure is reported: %s", diagnostic)
				}
				if strings.Count(diagnostic, "not authorized") < 2 {
					t.Fatalf("both refusals are reported: %s", diagnostic)
				}
			}
			assertNoRegistrationRows(t, conn, "after the failed commit")
		})
	}
}

func TestBrokerFleets_AFailedCommitWithAnEndedTransactionReportsAnUnconfirmedOutcome(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	conn.raw.CommitHook(func() bool { return false })
	_, err := createFleetWith(ctx, conn, "alpha", func(int64) (newPlacement, error) { return directorNewPlacement("%0"), nil })
	conn.raw.CommitHook(nil)
	if err == nil || !strings.Contains(err.Error(), "fleet registration outcome is unconfirmed: SQLite ended the transaction after a failed commit") {
		t.Fatalf("createFleetWith = %v", err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("no transaction is open")
	}
	assertNoRegistrationRows(t, conn, "after the refused commit")
}

func fleetIDs(rows []fleetSummary) []int64 {
	ids := make([]int64, len(rows))
	for index, row := range rows {
		ids[index] = row.fleetID
	}
	return ids
}

func TestBrokerFleets_FleetListingUsesTimeThenIdAndCountsOnlyActiveMembers(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	first, _ := createFleetFixture(t, conn, "first")
	second, _ := createFleetFixture(t, conn, "second")
	third, _ := createFleetFixture(t, conn, "third")
	worker := register(t, conn, first, "worker", none[string]())
	if _, err := conn.execute("UPDATE fleets SET created_at='2026-01-01T00:00:00Z'"); err != nil {
		t.Fatal(err)
	}
	if _, err := conn.execute("UPDATE fleets SET created_at='2026-01-02T00:00:00Z' WHERE fleet_id=?1", first); err != nil {
		t.Fatal(err)
	}
	list := func() []fleetSummary {
		rows, err := listFleets(ctx, conn)
		if err != nil {
			t.Fatal(err)
		}
		return rows
	}
	rows := list()
	if got := fleetIDs(rows); !slices.Equal(got, []int64{first, third, second}) {
		t.Fatalf("fleet order = %v", got)
	}
	if rows[0].memberCount != 2 {
		t.Fatalf("member count = %d", rows[0].memberCount)
	}
	if _, err := deregisterMember(ctx, conn, worker); err != nil {
		t.Fatal(err)
	}
	if got := list()[0].memberCount; got != 1 {
		t.Fatalf("member count after deregistration = %d", got)
	}
	if _, err := deleteFleetRows(ctx, conn, second); err != nil {
		t.Fatal(err)
	}
	if got := len(list()); got != 2 {
		t.Fatalf("listed fleets = %d", got)
	}
	deleted, err := getFleet(ctx, conn, second)
	record, ok := deleted.get()
	if _, stamped := record.deletedAt.get(); err != nil || !ok || !stamped {
		t.Fatalf("deleted fleet = %+v %v", deleted, err)
	}
	if absent, err := getFleet(ctx, conn, 999); err != nil || absent != none[fleetRecord]() {
		t.Fatalf("getFleet(999) = %v %v", absent, err)
	}
}

func TestBrokerFleets_RequireLiveFleetRefusesADeletedFleetOrAnInactiveRootDirector(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "alpha")
	if live, err := requireLiveFleet(ctx, conn, fleet); err != nil || live.fleetID != fleet {
		t.Fatalf("requireLiveFleet = %+v %v", live, err)
	}
	cases := []struct {
		damage string
		want   string
	}{
		{damage: fmt.Sprintf("UPDATE members SET status='deregistered' WHERE member_id=%d", director), want: fmt.Sprintf("fleet %d's root Director (member %d) is not active.", fleet, director)},
		{damage: "UPDATE fleets SET deleted_at='2026-01-01T00:00:00Z'", want: fmt.Sprintf("fleet %d is deleted", fleet)},
	}
	for _, tc := range cases {
		mustExecBatch(t, conn, "SAVEPOINT damage")
		mustExecBatch(t, conn, tc.damage)
		if _, err := requireLiveFleet(ctx, conn, fleet); err == nil || err.Error() != tc.want {
			t.Fatalf("%s: requireLiveFleet = %v, want %q", tc.damage, err, tc.want)
		}
		mustExecBatch(t, conn, "ROLLBACK TO damage; RELEASE damage")
	}
}

func TestBrokerFleets_DirectorPlacementReadsTheRecordedDirectorPaneAndServer(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "alpha")
	register(t, conn, fleet, "worker", some("%5"))
	expected := directorNewPlacement("%0")
	recorded, err := directorPlacement(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	if recorded.backend != expected.backend || recorded.muxSession != expected.muxSession ||
		recorded.muxWindowID != some("@1") || recorded.muxWindowName != some("main") ||
		recorded.windowOrdinal != 0 || recorded.columnSide != columnSideLeft || recorded.muxPaneID != some("%0") ||
		recorded.codingAgent != expected.codingAgent || recorded.muxServer != expected.muxServer {
		t.Fatalf("directorPlacement = %+v", recorded)
	}
	want := "fleet 999 has no recorded Director placement; re-create the fleet with 'cafleet fleet create'."
	if _, err := directorPlacement(ctx, conn, 999); err == nil || err.Error() != want {
		t.Fatalf("directorPlacement(999) = %v, want %q", err, want)
	}
}

func TestBrokerFleets_BootstrapRecordsTheDirectorWindowAsOrdinalZeroLeftAndStartsTheOrdinalAtZero(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	director := newPlacementDirector("herdr", "w4", "codex", "work", placementPane{muxWindowID: "w4:t1", muxWindowName: "agents", muxPaneID: "w4:p3N"})
	if director.windowOrdinal != 0 || director.columnSide != columnSideLeft {
		t.Fatalf("Director slot = (%d, %v)", director.windowOrdinal, director.columnSide)
	}
	fleet, err := createFleet(ctx, conn, "alpha", director)
	if err != nil {
		t.Fatal(err)
	}
	recorded := fleet.director.placement
	if recorded.windowOrdinal != 0 || recorded.columnSide != columnSideLeft || recorded.muxWindowID != some("w4:t1") ||
		recorded.muxWindowName != some("agents") || recorded.muxPaneID != some("w4:p3N") {
		t.Fatalf("Director placement = %+v", recorded)
	}
	if got := queryInt64(t, conn, "SELECT last_window_ordinal FROM fleets WHERE fleet_id=?1", fleet.fleetID); got != 0 {
		t.Fatalf("last_window_ordinal = %d", got)
	}
}
