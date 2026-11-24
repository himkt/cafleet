package core

import (
	"context"
	"encoding/json/jsontext"
	"math"
)

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
