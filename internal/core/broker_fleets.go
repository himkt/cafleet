package core

import (
	"context"
	"fmt"
	"strconv"
)

const (
	directorName        = "Director"
	directorDescription = "Root Director for this fleet"
)

// requiredDirectorID reads a committed fleet's director_member_id, which is set in the same
// transaction that inserts the fleet.
func requiredDirectorID(row sqlRow, column int) (int64, error) {
	director, err := row.optInt64(column)
	if err != nil {
		return 0, err
	}
	id, set := director.get()
	if !set {
		panic("a committed fleet records its root Director")
	}
	return id, nil
}

func getFleet(_ context.Context, conn *dbConn, fleetID int64) (opt[fleetRecord], error) {
	return queryRowOptional(conn,
		"SELECT fleet_id, name, created_at, deleted_at, director_member_id FROM fleets WHERE fleet_id=?1",
		[]any{fleetID},
		func(row sqlRow) (fleetRecord, error) {
			var fleet fleetRecord
			var err error
			if fleet.fleetID, err = row.int64(0); err != nil {
				return fleetRecord{}, err
			}
			if fleet.name, err = row.text(1); err != nil {
				return fleetRecord{}, err
			}
			if fleet.createdAt, err = row.text(2); err != nil {
				return fleetRecord{}, err
			}
			if fleet.deletedAt, err = row.optText(3); err != nil {
				return fleetRecord{}, err
			}
			if fleet.directorMemberID, err = requiredDirectorID(row, 4); err != nil {
				return fleetRecord{}, err
			}
			return fleet, nil
		})
}

func requireFleet(ctx context.Context, conn *dbConn, fleetID int64) (fleetRecord, error) {
	fleet, err := getFleet(ctx, conn, fleetID)
	if err != nil {
		return fleetRecord{}, err
	}
	record, found := fleet.get()
	if !found {
		return fleetRecord{}, appError("fleet '" + strconv.FormatInt(fleetID, 10) + "' not found.")
	}
	return record, nil
}

// createFleet creates a fleet whose root Director occupies director, whose pane is required.
func createFleet(ctx context.Context, conn *dbConn, name string, director newPlacement) (createdFleet, error) {
	return createFleetWith(ctx, conn, name, func(int64) (newPlacement, error) { return director, nil })
}

func createFleetWith(ctx context.Context, conn *dbConn, name string, director func(fleetID int64) (newPlacement, error)) (createdFleet, error) {
	now := nowStamp()
	if err := conn.begin(transactionDeferred); err != nil {
		return createdFleet{}, err
	}
	defer func() {
		if recovered := recover(); recovered != nil {
			if rollbackErr := conn.rollbackIfOpen(); rollbackErr != nil {
				panic(fmt.Sprintf("%v; rolling back fleet registration also failed: %v", recovered, rollbackErr))
			}
			panic(recovered)
		}
	}()
	created, err := createFleetRows(ctx, conn, name, now, director)
	if err == nil {
		return created, nil
	}
	var cleanup error
	if !conn.isAutocommit() {
		if rollbackErr := conn.executeBatch("ROLLBACK"); rollbackErr != nil {
			cleanup = appError("rolling back fleet registration failed: " + cafleetErrorOf(rollbackErr).message())
		}
	}
	return createdFleet{}, mergeCleanup(err, cleanup)
}

func createFleetRows(ctx context.Context, conn *dbConn, name, now string, director func(int64) (newPlacement, error)) (createdFleet, error) {
	fleetID, err := queryRow(conn, "INSERT INTO fleets (name, created_at) VALUES (?1, ?2) RETURNING fleet_id",
		[]any{name, now}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return createdFleet{}, err
	}
	placementOfDirector, err := director(fleetID)
	if err != nil {
		return createdFleet{}, err
	}
	if _, hasPane := placementOfDirector.pane.get(); !hasPane ||
		placementOfDirector.windowOrdinal != 0 || placementOfDirector.columnSide != columnSideLeft {
		panic("the Director placement records its pane in the Director window's left column")
	}
	directorID, err := insertMemberRow(ctx, conn, fleetID, directorName, directorDescription, now)
	if err != nil {
		return createdFleet{}, err
	}
	if err := insertPlacementRow(ctx, conn, directorID, placementOfDirector, now); err != nil {
		return createdFleet{}, err
	}
	if _, err := conn.execute("UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2", directorID, fleetID); err != nil {
		return createdFleet{}, err
	}
	member, err := getMember(ctx, conn, directorID, fleetID)
	if err != nil {
		return createdFleet{}, err
	}
	directorRecord, active := member.get()
	if !active {
		panic("the created root Director is active")
	}
	created := createdFleet{fleetID: fleetID, name: name, createdAt: now, director: directorRecord}
	if err := conn.executeBatch("COMMIT"); err != nil {
		failure := cafleetErrorOf(err)
		if conn.isAutocommit() {
			return createdFleet{}, failure.withDiagnostic(
				"fleet registration outcome is unconfirmed: SQLite ended the transaction after a failed commit")
		}
		return createdFleet{}, failure
	}
	return created, nil
}

// directorPlacement is the recorded placement of fleetID's root Director.
func directorPlacement(_ context.Context, conn *dbConn, fleetID int64) (placement, error) {
	found, err := queryRowOptional(conn,
		"SELECT "+placementColumns+" FROM fleets f JOIN member_placements p ON p.member_id=f.director_member_id\n             WHERE f.fleet_id=?1",
		[]any{fleetID}, func(row sqlRow) (placement, error) { return placementFromRow(row, 0) })
	if err != nil {
		return placement{}, err
	}
	recorded, set := found.get()
	if !set {
		return placement{}, appError(fmt.Sprintf(
			"fleet %d has no recorded Director placement; re-create the fleet with 'cafleet fleet create'.", fleetID))
	}
	return recorded, nil
}

func requireLiveFleet(ctx context.Context, conn *dbConn, fleetID int64) (fleetRecord, error) {
	fleet, err := requireFleet(ctx, conn, fleetID)
	if err != nil {
		return fleetRecord{}, err
	}
	if _, deleted := fleet.deletedAt.get(); deleted {
		return fleetRecord{}, appError(fmt.Sprintf("fleet %d is deleted", fleetID))
	}
	director := fleet.directorMemberID
	status, err := queryRowOptional(conn, "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2",
		[]any{director, fleetID}, func(row sqlRow) (memberStatus, error) { return rowStoredEnum(row, 0, memberStatusTryFrom) })
	if err != nil {
		return fleetRecord{}, err
	}
	if status == some(memberStatusActive) {
		return fleet, nil
	}
	return fleetRecord{}, appError(fmt.Sprintf("fleet %d's root Director (member %d) is not active.", fleetID, director))
}

func listFleets(_ context.Context, conn *dbConn) ([]fleetSummary, error) {
	return queryRows(conn,
		`SELECT f.fleet_id, f.name, f.created_at, f.director_member_id,
            (SELECT COUNT(*) FROM members m WHERE m.fleet_id=f.fleet_id AND m.status='active')
         FROM fleets f WHERE f.deleted_at IS NULL ORDER BY f.created_at DESC, f.fleet_id DESC`,
		nil, func(row sqlRow) (fleetSummary, error) {
			var summary fleetSummary
			var err error
			if summary.fleetID, err = row.int64(0); err != nil {
				return fleetSummary{}, err
			}
			if summary.name, err = row.text(1); err != nil {
				return fleetSummary{}, err
			}
			if summary.createdAt, err = row.text(2); err != nil {
				return fleetSummary{}, err
			}
			if summary.directorMemberID, err = requiredDirectorID(row, 3); err != nil {
				return fleetSummary{}, err
			}
			if summary.memberCount, err = row.int64(4); err != nil {
				return fleetSummary{}, err
			}
			return summary, nil
		})
}

func deleteFleetRows(ctx context.Context, conn *dbConn, fleetID int64) (int, error) {
	if _, err := requireFleet(ctx, conn, fleetID); err != nil {
		return 0, err
	}
	now := nowStamp()
	if _, err := conn.execute("UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2 AND deleted_at IS NULL", now, fleetID); err != nil {
		return 0, err
	}
	deregistered, err := conn.execute("UPDATE members SET status='deregistered' WHERE fleet_id=?1 AND status='active'", fleetID)
	if err != nil {
		return 0, err
	}
	if _, err := conn.execute("DELETE FROM member_placements WHERE member_id IN (SELECT member_id FROM members WHERE fleet_id=?1)", fleetID); err != nil {
		return 0, err
	}
	return int(deregistered), nil
}
