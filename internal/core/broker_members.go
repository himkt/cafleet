package core

import (
	"context"
	"fmt"
	"time"
)

func missingRun(memberID int64) *cafleetError {
	return appError(fmt.Sprintf("member %d has no recorded run", memberID))
}

type placementPane struct {
	muxWindowID   string
	muxWindowName string
	muxPaneID     string
}

// newPlacement with no pane registers a pending placement whose window and pane are published
// later.
type newPlacement struct {
	backend       string
	muxSession    string
	windowOrdinal int64
	columnSide    columnSide
	codingAgent   string
	muxServer     string
	pane          opt[placementPane]
}

func newPlacementDirector(backend, muxSession, codingAgent, muxServer string, pane placementPane) newPlacement {
	return newPlacement{
		backend:       backend,
		muxSession:    muxSession,
		windowOrdinal: 0,
		columnSide:    columnSideLeft,
		codingAgent:   codingAgent,
		muxServer:     muxServer,
		pane:          some(pane),
	}
}

// occupiedWindow is an overflow or Director window holding at least one active concrete
// ordinary member.
type occupiedWindow struct {
	windowOrdinal int64
	muxWindowID   string
	muxWindowName string
	left          []string
	right         []string
	// Every pane of the window, both columns, in registry order.
	panes []string
}

func lastWindowOrdinal(_ context.Context, conn *dbConn, fleetID int64) (int64, error) {
	return queryRow(conn, "SELECT last_window_ordinal FROM fleets WHERE fleet_id=?1",
		[]any{fleetID}, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func concreteOccupancy(_ context.Context, conn *dbConn, fleetID int64) ([]occupiedWindow, error) {
	type occupancyRow struct {
		windowOrdinal int64
		muxWindowID   string
		muxWindowName string
		side          columnSide
		pane          string
	}
	rows, err := queryRows(conn,
		`SELECT p.window_ordinal, p.mux_window_id, p.mux_window_name, p.column_side, p.mux_pane_id
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL
           AND f.director_member_id IS NOT m.member_id
         ORDER BY p.window_ordinal, m.member_id`,
		[]any{fleetID}, func(row sqlRow) (occupancyRow, error) {
			var r occupancyRow
			var err error
			if r.windowOrdinal, err = row.int64(0); err != nil {
				return occupancyRow{}, err
			}
			if r.muxWindowID, err = row.text(1); err != nil {
				return occupancyRow{}, err
			}
			if r.muxWindowName, err = row.text(2); err != nil {
				return occupancyRow{}, err
			}
			if r.side, err = rowStoredEnum(row, 3, columnSideTryFrom); err != nil {
				return occupancyRow{}, err
			}
			if r.pane, err = row.text(4); err != nil {
				return occupancyRow{}, err
			}
			return r, nil
		})
	if err != nil {
		return nil, err
	}
	windows := []occupiedWindow{}
	for _, row := range rows {
		if len(windows) == 0 || windows[len(windows)-1].windowOrdinal != row.windowOrdinal {
			windows = append(windows, occupiedWindow{
				windowOrdinal: row.windowOrdinal,
				muxWindowID:   row.muxWindowID,
				muxWindowName: row.muxWindowName,
				left:          []string{},
				right:         []string{},
				panes:         []string{},
			})
		}
		window := &windows[len(windows)-1]
		window.panes = append(window.panes, row.pane)
		switch row.side {
		case columnSideLeft:
			window.left = append(window.left, row.pane)
		case columnSideRight:
			window.right = append(window.right, row.pane)
		default:
			panic("invalid columnSide")
		}
	}
	return windows, nil
}

func windowMemberIDs(_ context.Context, conn *dbConn, fleetID, windowOrdinal int64) ([]int64, error) {
	return queryRows(conn,
		`SELECT m.member_id
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND p.window_ordinal=?2
           AND f.director_member_id IS NOT m.member_id
         ORDER BY m.member_id`,
		[]any{fleetID, windowOrdinal}, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func registerMemberRows(ctx context.Context, conn *dbConn, fleetID int64, name, description string, p newPlacement) (registeredMember, error) {
	now := nowStamp()
	if _, err := requireLiveFleet(ctx, conn, fleetID); err != nil {
		return registeredMember{}, err
	}
	memberID, err := insertMemberRow(ctx, conn, fleetID, name, description, now)
	if err != nil {
		return registeredMember{}, err
	}
	if err := insertPlacementRow(ctx, conn, memberID, p, now); err != nil {
		return registeredMember{}, err
	}
	return registeredMember{memberID: memberID, name: name, registeredAt: now}, nil
}

func insertMemberRow(_ context.Context, conn *dbConn, fleetID int64, name, description, now string) (int64, error) {
	return queryRow(conn,
		`INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4) RETURNING member_id`,
		[]any{fleetID, name, description, now}, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func insertPlacementRow(_ context.Context, conn *dbConn, memberID int64, p newPlacement, now string) error {
	windowID, windowName, paneID := none[string](), none[string](), none[string]()
	if pane, set := p.pane.get(); set {
		windowID, windowName, paneID = some(pane.muxWindowID), some(pane.muxWindowName), some(pane.muxPaneID)
	}
	_, err := conn.execute(
		`INSERT INTO member_placements
         (member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side,
          mux_pane_id, backend, mux_server, coding_agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)`,
		memberID, p.muxSession, windowID, windowName, p.windowOrdinal, p.columnSide.asStr(),
		paneID, p.backend, p.muxServer, p.codingAgent, now)
	return err
}

const (
	memberFields   = 7
	activityOffset = memberFields + placementColumnCount
)

func memberColumns() string {
	return "m.member_id, m.fleet_id, m.name, m.description, m.status,\n         m.registered_at, f.director_member_id=m.member_id, " + placementColumns
}

const memberFrom = "FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id\n    JOIN member_placements p ON p.member_id=m.member_id"

func memberRow(row sqlRow) (memberRecord, error) {
	var member memberRecord
	var err error
	if member.memberID, err = row.int64(0); err != nil {
		return memberRecord{}, err
	}
	if member.fleetID, err = row.int64(1); err != nil {
		return memberRecord{}, err
	}
	if member.name, err = row.text(2); err != nil {
		return memberRecord{}, err
	}
	if member.description, err = row.text(3); err != nil {
		return memberRecord{}, err
	}
	if member.status, err = rowStoredEnum(row, 4, memberStatusTryFrom); err != nil {
		return memberRecord{}, err
	}
	if member.registeredAt, err = row.text(5); err != nil {
		return memberRecord{}, err
	}
	isDirector, err := row.bool(6)
	if err != nil {
		return memberRecord{}, err
	}
	member.kind = memberKindMember
	if isDirector {
		member.kind = memberKindDirector
	}
	if member.placement, err = placementFromRow(row, memberFields); err != nil {
		return memberRecord{}, err
	}
	return member, nil
}

func getMember(ctx context.Context, conn *dbConn, memberID, fleetID int64) (opt[memberRecord], error) {
	if _, err := requireLiveFleet(ctx, conn, fleetID); err != nil {
		return none[memberRecord](), err
	}
	found, err := queryRowOptional(conn,
		"SELECT "+memberColumns()+" "+memberFrom+"\n                 WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'",
		[]any{memberID, fleetID}, memberRow)
	if err != nil {
		return none[memberRecord](), err
	}
	member, set := found.get()
	if !set {
		return found, nil
	}
	if member.runtime, err = memberRuntime(ctx, conn, memberID); err != nil {
		return none[memberRecord](), err
	}
	return some(member), nil
}

// fleetMembership is a member's fleet and registration status.
type fleetMembership struct {
	fleetID int64
	status  memberStatus
}

func memberFleetStatus(_ context.Context, conn *dbConn, memberID int64) (opt[fleetMembership], error) {
	return queryRowOptional(conn, "SELECT fleet_id, status FROM members WHERE member_id=?1",
		[]any{memberID}, func(row sqlRow) (fleetMembership, error) {
			fleetID, err := row.int64(0)
			if err != nil {
				return fleetMembership{}, err
			}
			status, err := rowStoredEnum(row, 1, memberStatusTryFrom)
			if err != nil {
				return fleetMembership{}, err
			}
			return fleetMembership{fleetID: fleetID, status: status}, nil
		})
}

func activeMemberFleet(ctx context.Context, conn *dbConn, memberID int64) (opt[int64], error) {
	membership, err := memberFleetStatus(ctx, conn, memberID)
	if err != nil {
		return none[int64](), err
	}
	if found, set := membership.get(); set && found.status == memberStatusActive {
		return some(found.fleetID), nil
	}
	return none[int64](), nil
}

func deregisterMemberRows(_ context.Context, conn *dbConn, memberID int64) (bool, error) {
	isRoot, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM fleets WHERE director_member_id=?1 AND deleted_at IS NULL)",
		[]any{memberID}, func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil {
		return false, err
	}
	if isRoot {
		return false, appError("cannot deregister the root Director; use 'cafleet fleet delete' instead")
	}
	changed, err := conn.execute("UPDATE members SET status='deregistered' WHERE member_id=?1 AND status='active'", memberID)
	if err != nil {
		return false, err
	}
	if changed == 0 {
		return false, nil
	}
	if _, err := conn.execute("DELETE FROM member_placements WHERE member_id=?1", memberID); err != nil {
		return false, err
	}
	return true, nil
}

// idleSeconds is the whole seconds since the member's latest send, receipt or consumption,
// truncated toward zero and never negative, or none when it has no message.
func idleSeconds(row memberActivity, now time.Time) (opt[int64], error) {
	candidates := []struct {
		field string
		value opt[string]
	}{
		{"messages.created_at", row.lastSent},
		{"messages.created_at", row.lastRecv},
		{"messages.status_timestamp", row.lastConsumed},
	}
	field, latest, found := "", "", false
	for _, candidate := range candidates {
		value, set := candidate.value.get()
		if set && (!found || value >= latest) {
			field, latest, found = candidate.field, value, true
		}
	}
	if !found {
		return none[int64](), nil
	}
	parsed, ok := parseLenient(latest)
	if !ok {
		return none[int64](), invalidStoredValueError(field, latest)
	}
	return some(max(wholeSecondsBetween(parsed, now), 0)), nil
}

// wholeSecondsBetween is later minus earlier in whole seconds, truncated toward zero, without the
// saturation time.Time.Sub applies beyond about 292 years.
func wholeSecondsBetween(earlier, later time.Time) int64 {
	seconds := later.Unix() - earlier.Unix()
	nanoseconds := later.Nanosecond() - earlier.Nanosecond()
	switch {
	case seconds > 0 && nanoseconds < 0:
		seconds--
	case seconds < 0 && nanoseconds > 0:
		seconds++
	}
	return seconds
}

func listMembers(ctx context.Context, conn *dbConn, fleetID int64) ([]memberActivity, error) {
	if _, err := requireLiveFleet(ctx, conn, fleetID); err != nil {
		return nil, err
	}
	now := time.Now().UTC()
	activities, err := queryRows(conn,
		"SELECT "+memberColumns()+`,
                (SELECT MAX(created_at) FROM messages WHERE from_member_id=m.member_id),
                (SELECT MAX(created_at) FROM messages WHERE to_member_id=m.member_id),
                (SELECT MAX(status_timestamp) FROM messages WHERE to_member_id=m.member_id AND status_state='completed')
         `+memberFrom+`
         WHERE m.fleet_id=?1 AND m.status='active'
         ORDER BY m.member_id`,
		[]any{fleetID}, func(row sqlRow) (memberActivity, error) {
			member, err := memberRow(row)
			if err != nil {
				return memberActivity{}, err
			}
			activity := memberActivity{member: member, idle: none[int64]()}
			if activity.lastSent, err = row.optText(activityOffset); err != nil {
				return memberActivity{}, err
			}
			if activity.lastRecv, err = row.optText(activityOffset + 1); err != nil {
				return memberActivity{}, err
			}
			if activity.lastConsumed, err = row.optText(activityOffset + 2); err != nil {
				return memberActivity{}, err
			}
			return activity, nil
		})
	if err != nil {
		return nil, err
	}
	for index := range activities {
		if activities[index].idle, err = idleSeconds(activities[index], now); err != nil {
			return nil, err
		}
	}
	return activities, nil
}

func activeMember(ctx context.Context, conn *dbConn, memberID int64) (memberRecord, error) {
	missing := appError(fmt.Sprintf("Member %d not found", memberID))
	fleet, err := activeMemberFleet(ctx, conn, memberID)
	if err != nil {
		return memberRecord{}, err
	}
	fleetID, set := fleet.get()
	if !set {
		return memberRecord{}, missing
	}
	member, err := getMember(ctx, conn, memberID, fleetID)
	if err != nil {
		return memberRecord{}, err
	}
	record, found := member.get()
	if !found {
		return memberRecord{}, missing
	}
	return record, nil
}
