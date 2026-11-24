package core

import (
	"context"
	"errors"
	"slices"
	"strings"
	"testing"
	"time"
)

const (
	testWindowID   = "@1"
	testWindowName = "main"
	testSession    = "main"
	testServer     = "/nonexistent/cafleet-test/tmux.sock"
)

func testPane(pane string) placementPane {
	return placementPane{muxWindowID: testWindowID, muxWindowName: testWindowName, muxPaneID: pane}
}

// testPlacement is a Director-window member placement, concrete in window @1 when pane is set.
func testPlacement(pane opt[string]) newPlacement {
	placed := none[placementPane]()
	if id, ok := pane.get(); ok {
		placed = some(testPane(id))
	}
	return newPlacement{
		backend:       "tmux",
		muxSession:    testSession,
		windowOrdinal: 0,
		columnSide:    columnSideRight,
		codingAgent:   "claude",
		muxServer:     testServer,
		pane:          placed,
	}
}

func directorNewPlacement(pane string) newPlacement {
	return newPlacementDirector("tmux", testSession, "claude", testServer, testPane(pane))
}

func slotPlacement(windowOrdinal int64, side columnSide) newPlacement {
	slot := testPlacement(none[string]())
	slot.windowOrdinal = windowOrdinal
	slot.columnSide = side
	return slot
}

func registerMember(ctx context.Context, conn *dbConn, fleetID int64, name, description string, p newPlacement) (registeredMember, error) {
	return inTransaction(conn, transactionImmediate, func() (registeredMember, error) {
		return registerMemberRows(ctx, conn, fleetID, name, description, p)
	})
}

func deregisterMember(ctx context.Context, conn *dbConn, memberID int64) (bool, error) {
	return inTransaction(conn, transactionDeferred, func() (bool, error) {
		return deregisterMemberRows(ctx, conn, memberID)
	})
}

// registerInSlot registers a pending member in the given slot and, when concrete is set,
// publishes its window and pane.
func registerInSlot(t *testing.T, conn *dbConn, fleet int64, name string, windowOrdinal int64, side columnSide, concrete opt[placementPane]) int64 {
	t.Helper()
	member, err := registerMember(context.Background(), conn, fleet, name, "assignment", slotPlacement(windowOrdinal, side))
	if err != nil {
		t.Fatal(err)
	}
	if pane, ok := concrete.get(); ok {
		changed, err := conn.execute(`UPDATE member_placements SET mux_pane_id=?1, mux_window_id=?2, mux_window_name=?3
			WHERE member_id=?4 AND mux_pane_id IS NULL`, pane.muxPaneID, pane.muxWindowID, pane.muxWindowName, member.memberID)
		if err != nil || changed != 1 {
			t.Fatalf("publishing %s: %d %v", name, changed, err)
		}
	}
	return member.memberID
}

func register(t *testing.T, conn *dbConn, fleet int64, name string, pane opt[string]) int64 {
	t.Helper()
	member, err := registerMember(context.Background(), conn, fleet, name, "assignment", testPlacement(pane))
	if err != nil {
		t.Fatal(err)
	}
	return member.memberID
}

func requireMember(t *testing.T, conn *dbConn, memberID, fleetID int64) memberRecord {
	t.Helper()
	member, err := getMember(context.Background(), conn, memberID, fleetID)
	if err != nil {
		t.Fatal(err)
	}
	found, ok := member.get()
	if !ok {
		t.Fatalf("member %d is active in fleet %d", memberID, fleetID)
	}
	return found
}

func assertInvalidStoredValue(t *testing.T, err error, field, value string) {
	t.Helper()
	var failure *cafleetError
	if !errors.As(err, &failure) || failure.kind != errorInvalidStoredValue || failure.field != field || failure.value != value {
		t.Fatalf("error %v, want an invalid stored value %s=%q", err, field, value)
	}
}

func TestBrokerMembers_RegistrationAndPlacementUpdatesPreserveRequiredFieldsAndValidAbsence(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "alpha")
	registered, err := registerMember(ctx, conn, fleet, "worker", "literal assignment", testPlacement(none[string]()))
	if err != nil {
		t.Fatal(err)
	}
	if registered.name != "worker" {
		t.Fatalf("name = %q", registered.name)
	}
	if _, ok := parseLenient(registered.registeredAt); !ok {
		t.Fatalf("registeredAt %q is a timestamp", registered.registeredAt)
	}
	member := requireMember(t, conn, registered.memberID, fleet)
	if member.description != "literal assignment" || member.kind != memberKindMember || member.placement.muxPaneID != none[string]() {
		t.Fatalf("member = %+v", member)
	}
}

func TestBrokerMembers_RegistrationRollsBackMemberAndIdentifierWhenPlacementFails(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "alpha")
	mustExecBatch(t, conn, "CREATE TRIGGER fail_placement BEFORE INSERT ON member_placements BEGIN SELECT RAISE(ABORT, 'placement failure'); END")
	if _, err := registerMember(ctx, conn, fleet, "worker", "assignment", testPlacement(none[string]())); err == nil || !strings.Contains(err.Error(), "placement failure") {
		t.Fatalf("registerMember = %v", err)
	}
	if members, err := listMembers(ctx, conn, fleet); err != nil || len(members) != 1 {
		t.Fatalf("listMembers = %v %v", members, err)
	}
	mustExecBatch(t, conn, "DROP TRIGGER fail_placement")
	if got := register(t, conn, fleet, "worker", none[string]()); got != director+1 {
		t.Fatalf("member %d, want %d", got, director+1)
	}
}

func TestBrokerMembers_LiveFleetAndDirectorChecksGuardMemberRegistration(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	if _, err := registerMember(ctx, conn, 999, "x", "assignment", testPlacement(none[string]())); err == nil {
		t.Fatal("registration in an absent fleet is refused")
	}
	fleet, director := createFleetFixture(t, conn, "alpha")
	if _, err := deregisterMember(ctx, conn, director); err == nil || err.Error() != "cannot deregister the root Director; use 'cafleet fleet delete' instead" {
		t.Fatalf("deregistering the root Director: %v", err)
	}
	if _, err := deleteFleetRows(ctx, conn, fleet); err != nil {
		t.Fatal(err)
	}
	if _, err := registerMember(ctx, conn, fleet, "worker", "assignment", testPlacement(none[string]())); err == nil {
		t.Fatal("registration in a deleted fleet is refused")
	}
}

func TestBrokerMembers_DeregistrationIsFleetScopedAndKeepsHistory(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "alpha")
	other, _ := createFleetFixture(t, conn, "other")
	worker := register(t, conn, fleet, "worker", some("%2"))
	message, err := sendMessage(ctx, conn, director, worker, "history")
	if err != nil {
		t.Fatal(err)
	}
	if found, err := getMember(ctx, conn, worker, other); err != nil || found != none[memberRecord]() {
		t.Fatalf("worker in another fleet: %v %v", found, err)
	}
	for round, want := range []bool{true, false} {
		if changed, err := deregisterMember(ctx, conn, worker); err != nil || changed != want {
			t.Fatalf("deregistration %d = %v %v, want %v", round, changed, err, want)
		}
	}
	if found, err := getMember(ctx, conn, worker, fleet); err != nil || found != none[memberRecord]() {
		t.Fatalf("deregistered worker: %v %v", found, err)
	}
	if stored, err := getMessage(ctx, conn, message.messageID); err != nil || stored != message {
		t.Fatalf("history = %+v %v, want %+v", stored, err, message)
	}
	if got := queryInt64(t, conn, "SELECT COUNT(*) FROM member_placements WHERE member_id=?1", worker); got != 0 {
		t.Fatalf("placements = %d", got)
	}
	if got := queryText(t, conn, "SELECT status FROM members WHERE member_id=?1", worker); got != "deregistered" {
		t.Fatalf("status = %q", got)
	}
}

func memberIDs(rows []memberActivity) []int64 {
	ids := make([]int64, len(rows))
	for index, row := range rows {
		ids[index] = row.member.memberID
	}
	return ids
}

func idleWithin(t *testing.T, idle opt[int64], low, high int64) {
	t.Helper()
	seconds, ok := idle.get()
	if !ok || seconds < low || seconds > high {
		t.Fatalf("idle = %v, want [%d, %d]", idle, low, high)
	}
}

func TestBrokerMembers_RosterPreservesAllPlacementStatesAndActivity(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "activity")
	worker := register(t, conn, fleet, "worker", some("%2"))
	pending := register(t, conn, fleet, "pending", none[string]())
	message, err := sendMessage(ctx, conn, director, worker, "hello")
	if err != nil {
		t.Fatal(err)
	}
	rows, err := listMembers(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	if got := memberIDs(rows); !slices.Equal(got, []int64{director, worker, pending}) {
		t.Fatalf("roster = %v", got)
	}
	sender := rows[0]
	if sender.member.kind != memberKindDirector || sender.lastSent != some(message.createdAt) || sender.lastRecv != none[string]() {
		t.Fatalf("sender = %+v", sender)
	}
	idleWithin(t, sender.idle, 0, 5)
	if rows[1].lastRecv != some(message.createdAt) || rows[1].lastConsumed != none[string]() {
		t.Fatalf("recipient = %+v", rows[1])
	}
	if rows[2].member.placement.muxPaneID != none[string]() || rows[2].lastSent != none[string]() || rows[2].lastRecv != none[string]() || rows[2].idle != none[int64]() {
		t.Fatalf("pending = %+v", rows[2])
	}
	consumed, err := pollMessages(ctx, conn, worker)
	if err != nil || len(consumed) != 1 {
		t.Fatalf("poll = %v %v", consumed, err)
	}
	consumedRows, err := listMembers(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	if consumedRows[1].lastConsumed != some(consumed[0].statusTimestamp) {
		t.Fatalf("lastConsumed = %v, want %s", consumedRows[1].lastConsumed, consumed[0].statusTimestamp)
	}
	idleWithin(t, consumedRows[1].idle, 0, 5)
	if _, err := conn.execute("UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2", worker, fleet); err != nil {
		t.Fatal(err)
	}
	rows, err = listMembers(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	if rows[0].member.kind != memberKindMember || rows[1].member.kind != memberKindDirector {
		t.Fatalf("kinds after the Director moved: %v %v", rows[0].member.kind, rows[1].member.kind)
	}
	if _, err := deregisterMember(ctx, conn, pending); err != nil {
		t.Fatal(err)
	}
	if rows, err := listMembers(ctx, conn, fleet); err != nil || len(rows) != 2 {
		t.Fatalf("roster after deregistration = %v %v", rows, err)
	}
}

func activity(sent, received, consumed opt[string]) memberActivity {
	return memberActivity{
		member: memberRecord{
			memberID: 1,
			fleetID:  1,
			name:     "worker",
			status:   memberStatusActive,
			kind:     memberKindMember,
			placement: placement{
				backend:       "tmux",
				muxSession:    "main",
				muxWindowID:   none[string](),
				muxWindowName: none[string](),
				windowOrdinal: 0,
				columnSide:    columnSideRight,
				muxPaneID:     none[string](),
				codingAgent:   "claude",
				muxServer:     testServer,
			},
		},
		lastSent:     sent,
		lastRecv:     received,
		lastConsumed: consumed,
		idle:         none[int64](),
	}
}

func mustParseLenient(t *testing.T, value string) time.Time {
	t.Helper()
	parsed, ok := parseLenient(value)
	if !ok {
		t.Fatalf("%q is a timestamp", value)
	}
	return parsed
}

func assertIdle(t *testing.T, now string, row memberActivity, want opt[int64]) {
	t.Helper()
	got, err := idleSeconds(row, mustParseLenient(t, now))
	if err != nil || got != want {
		t.Fatalf("idleSeconds = %v %v, want %v", got, err, want)
	}
}

func TestBrokerMembers_IdleClampsFutureActivityToZero(t *testing.T) {
	t.Parallel()
	assertIdle(t, "2026-01-01T00:00:00Z", activity(none[string](), none[string](), some("2026-01-01T00:01:00Z")), some(int64(0)))
}

func TestBrokerMembers_IdleMeasuresFromAConsumptionLaterThanEverySendAndReceipt(t *testing.T) {
	t.Parallel()
	row := activity(some("2025-12-31T23:00:00Z"), some("2025-12-31T23:00:00Z"), some("2025-12-31T23:59:30Z"))
	assertIdle(t, "2026-01-01T00:00:00Z", row, some(int64(30)))
}

func TestBrokerMembers_IdleIsAbsentWithoutActivityAndRefusesACorruptLatestTimestamp(t *testing.T) {
	t.Parallel()
	assertIdle(t, "2026-01-01T00:00:00Z", activity(none[string](), none[string](), none[string]()), none[int64]())
	now := mustParseLenient(t, "2026-01-01T00:00:00Z")
	cases := []struct {
		name  string
		row   memberActivity
		field string
	}{
		{name: "corrupt send", row: activity(some("z-invalid"), some("2025-12-31T23:59:50Z"), none[string]()), field: "messages.created_at"},
		{name: "corrupt consumption", row: activity(none[string](), some("2025-12-31T23:59:50Z"), some("z-invalid")), field: "messages.status_timestamp"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			_, err := idleSeconds(tc.row, now)
			assertInvalidStoredValue(t, err, tc.field, "z-invalid")
		})
	}
}

func TestBrokerMembers_IdleSelectsRawLexicographicMaximumBeforeParsingOffsets(t *testing.T) {
	t.Parallel()
	row := activity(some("2026-01-01T08:00:00+09:00"), some("2025-12-31T23:59:50Z"), none[string]())
	assertIdle(t, "2026-01-01T00:00:00Z", row, some(int64(3600)))
}

func TestBrokerMembers_IdlePreservesFractionalSecondTruncation(t *testing.T) {
	t.Parallel()
	now := "2026-01-01T00:00:01.900000+00:00"
	assertIdle(t, now, activity(some("2026-01-01T00:00:00.950000+00:00"), none[string](), none[string]()), some(int64(0)))
	assertIdle(t, now, activity(none[string](), none[string](), some("2025-12-31T23:59:59.950000+00:00")), some(int64(1)))
}

type windowSummary struct {
	ordinal     int64
	id, name    string
	left, right []string
}

func summarizeWindows(windows []occupiedWindow) []windowSummary {
	summary := make([]windowSummary, len(windows))
	for index, window := range windows {
		summary[index] = windowSummary{ordinal: window.windowOrdinal, id: window.muxWindowID, name: window.muxWindowName, left: window.left, right: window.right}
	}
	return summary
}

func windowSummariesEqual(left, right []windowSummary) bool {
	return slices.EqualFunc(left, right, func(a, b windowSummary) bool {
		return a.ordinal == b.ordinal && a.id == b.id && a.name == b.name && slices.Equal(a.left, b.left) && slices.Equal(a.right, b.right)
	})
}

func concretePane(id, name, pane string) opt[placementPane] {
	return some(placementPane{muxWindowID: id, muxWindowName: name, muxPaneID: pane})
}

func TestBrokerMembers_Occupancy_OccupancyGroupsConcretePlacementsByOrdinalAndSideInRegistryOrder(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout")
	registrations := []struct {
		name     string
		ordinal  int64
		side     columnSide
		concrete opt[placementPane]
	}{
		{"a", 0, columnSideRight, concretePane("@1", "main", "%2")},
		{"b", 1, columnSideLeft, concretePane("@7", "cafleet-1-1", "%10")},
		{"c", 0, columnSideRight, concretePane("@1", "main", "%3")},
		{"d", 1, columnSideRight, concretePane("@7", "cafleet-1-1", "%12")},
		{"e", 1, columnSideLeft, concretePane("@7", "cafleet-1-1", "%11")},
		{"f", 3, columnSideLeft, concretePane("@9", "cafleet-1-3", "%20")},
	}
	for _, registration := range registrations {
		registerInSlot(t, conn, fleet, registration.name, registration.ordinal, registration.side, registration.concrete)
	}
	windows, err := concreteOccupancy(ctx, conn, fleet)
	if err != nil {
		t.Fatal(err)
	}
	want := []windowSummary{
		{ordinal: 0, id: "@1", name: "main", left: []string{}, right: []string{"%2", "%3"}},
		{ordinal: 1, id: "@7", name: "cafleet-1-1", left: []string{"%10", "%11"}, right: []string{"%12"}},
		{ordinal: 3, id: "@9", name: "cafleet-1-3", left: []string{"%20"}, right: []string{}},
	}
	if got := summarizeWindows(windows); !windowSummariesEqual(got, want) {
		t.Fatalf("occupancy = %+v, want %+v", got, want)
	}
	wantPanes := [][]string{{"%2", "%3"}, {"%10", "%12", "%11"}, {"%20"}}
	for index, window := range windows {
		if !slices.Equal(window.panes, wantPanes[index]) {
			t.Fatalf("window %d panes = %v, want %v in registry order", window.windowOrdinal, window.panes, wantPanes[index])
		}
	}
}

func TestBrokerMembers_Occupancy_OccupancyExcludesTheDirectorPendingDeregisteredAndOtherFleetRows(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout")
	other, _ := createFleetFixture(t, conn, "other")
	occupancy := func() []windowSummary {
		windows, err := concreteOccupancy(ctx, conn, fleet)
		if err != nil {
			t.Fatal(err)
		}
		return summarizeWindows(windows)
	}
	if got := occupancy(); len(got) != 0 {
		t.Fatalf("a fresh fleet's occupancy = %+v", got)
	}
	registerInSlot(t, conn, fleet, "pending", 1, columnSideLeft, none[placementPane]())
	departed := registerInSlot(t, conn, fleet, "departed", 2, columnSideLeft, concretePane("@8", "cafleet-1-2", "%30"))
	if _, err := deregisterMember(ctx, conn, departed); err != nil {
		t.Fatal(err)
	}
	registerInSlot(t, conn, other, "foreign", 1, columnSideLeft, concretePane("@5", "cafleet-2-1", "%40"))
	if got := occupancy(); len(got) != 0 {
		t.Fatalf("occupancy without live concrete members = %+v", got)
	}
	registerInSlot(t, conn, fleet, "live", 0, columnSideRight, concretePane("@1", "main", "%2"))
	want := []windowSummary{{ordinal: 0, id: "@1", name: "main", left: []string{}, right: []string{"%2"}}}
	if got := occupancy(); !windowSummariesEqual(got, want) {
		t.Fatalf("occupancy = %+v, want %+v", got, want)
	}
}

func TestBrokerMembers_Occupancy_WindowMembersListsEveryActiveMemberOfOneOrdinalAscending(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "layout")
	other, _ := createFleetFixture(t, conn, "other")
	first := registerInSlot(t, conn, fleet, "first", 1, columnSideRight, concretePane("@7", "cafleet-1-1", "%11"))
	registerInSlot(t, conn, fleet, "director-window", 0, columnSideRight, concretePane("@1", "main", "%2"))
	second := registerInSlot(t, conn, fleet, "second", 1, columnSideLeft, concretePane("@7", "cafleet-1-1", "%10"))
	departed := registerInSlot(t, conn, fleet, "departed", 1, columnSideLeft, concretePane("@7", "cafleet-1-1", "%12"))
	if _, err := deregisterMember(ctx, conn, departed); err != nil {
		t.Fatal(err)
	}
	registerInSlot(t, conn, other, "foreign", 1, columnSideLeft, concretePane("@5", "cafleet-2-1", "%40"))
	members := func(ordinal int64) []int64 {
		ids, err := windowMemberIDs(ctx, conn, fleet, ordinal)
		if err != nil {
			t.Fatal(err)
		}
		return ids
	}
	if got := members(1); !slices.Equal(got, []int64{first, second}) {
		t.Fatalf("window 1 members = %v, want %v", got, []int64{first, second})
	}
	if got := members(0); slices.Contains(got, director) {
		t.Fatalf("window 0 members %v exclude the Director", got)
	}
	if got := members(2); len(got) != 0 {
		t.Fatalf("window 2 members = %v", got)
	}
}

func TestBrokerMembers_Occupancy_AStoredColumnSideOutsideLeftAndRightFailsDecoding(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, _ := createFleetFixture(t, conn, "layout")
	worker := registerInSlot(t, conn, fleet, "worker", 0, columnSideRight, concretePane("@1", "main", "%2"))
	mustExecBatch(t, conn, "PRAGMA ignore_check_constraints=ON")
	if _, err := conn.execute("UPDATE member_placements SET column_side='middle' WHERE member_id=?1", worker); err != nil {
		t.Fatal(err)
	}
	mustExecBatch(t, conn, "PRAGMA ignore_check_constraints=OFF")
	_, getErr := getMember(ctx, conn, worker, fleet)
	assertInvalidStoredValue(t, getErr, "member_placements.column_side", "middle")
	_, listErr := listMembers(ctx, conn, fleet)
	assertInvalidStoredValue(t, listErr, "member_placements.column_side", "middle")
}

func TestBrokerMembers_Occupancy_MemberReadsDecodeTheRecordedSlotAndWindow(t *testing.T) {
	t.Parallel()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "layout")
	pending := registerInSlot(t, conn, fleet, "pending", 2, columnSideLeft, none[placementPane]())
	placed := registerInSlot(t, conn, fleet, "placed", 1, columnSideRight, concretePane("@7", "cafleet-1-1", "%11"))
	type slot struct {
		ordinal                int64
		side                   columnSide
		windowID, name, paneID opt[string]
	}
	cases := []struct {
		name   string
		member int64
		want   slot
	}{
		{name: "Director", member: director, want: slot{0, columnSideLeft, some("@1"), some("main"), some("%0")}},
		{name: "pending", member: pending, want: slot{2, columnSideLeft, none[string](), none[string](), none[string]()}},
		{name: "placed", member: placed, want: slot{1, columnSideRight, some("@7"), some("cafleet-1-1"), some("%11")}},
	}
	for _, tc := range cases {
		recorded := requireMember(t, conn, tc.member, fleet).placement
		got := slot{recorded.windowOrdinal, recorded.columnSide, recorded.muxWindowID, recorded.muxWindowName, recorded.muxPaneID}
		if got != tc.want {
			t.Fatalf("%s: slot = %+v, want %+v", tc.name, got, tc.want)
		}
	}
}
