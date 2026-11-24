package core

import (
	"context"
	"testing"
)

func TestBroker_CorruptDirectorStatusReturnsTheDomainVariantFromALiveTypedRead(t *testing.T) {
	t.Parallel()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "typed-records")
	mustExecBatch(t, conn, "PRAGMA ignore_check_constraints=ON")
	if _, err := conn.execute("UPDATE members SET status='corrupt-status' WHERE member_id=?1", director); err != nil {
		t.Fatal(err)
	}
	_, err := getMember(context.Background(), conn, director, fleet)
	assertInvalidStoredValue(t, err, "members.status", "corrupt-status")
}

func TestBroker_CorruptRecipientStatusStopsASendWithTheDomainCauseBeforeInserting(t *testing.T) {
	t.Parallel()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "typed-records")
	worker := register(t, conn, fleet, "worker", some("%2"))
	mustExecBatch(t, conn, "PRAGMA ignore_check_constraints=ON")
	if _, err := conn.execute("UPDATE members SET status='corrupt-recipient' WHERE member_id=?1", worker); err != nil {
		t.Fatal(err)
	}
	_, err := sendMessage(context.Background(), conn, director, worker, "pending assignment")
	assertInvalidStoredValue(t, err, "members.status", "corrupt-recipient")
	if got := tableCount(t, conn, "messages"); got != 0 {
		t.Fatalf("messages = %d", got)
	}
}

func TestBroker_InvalidRequiredStringStorageReturnsSqlConversionFailures(t *testing.T) {
	t.Parallel()
	for _, damage := range []string{
		"UPDATE members SET name=X'00' WHERE member_id=1",
		"UPDATE member_placements SET mux_session=X'00' WHERE member_id=1",
	} {
		t.Run(damage, func(t *testing.T) {
			t.Parallel()
			ctx := context.Background()
			conn, _ := initializedFileConn(t)
			fleet, director := createFleetFixture(t, conn, "typed-records")
			mustExecBatch(t, conn, damage)
			if _, err := getMember(ctx, conn, director, fleet); err == nil {
				t.Fatalf("%s: getMember fails decoding", damage)
			}
			if _, err := listMembers(ctx, conn, fleet); err == nil {
				t.Fatalf("%s: listMembers fails decoding", damage)
			}
		})
	}
}
