package core

import (
	"cmp"
	"context"
	"fmt"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/ncruces/go-sqlite3"
)

func messageStatusState(t *testing.T, conn *dbConn, messageID int64) string {
	t.Helper()
	return queryText(t, conn, "SELECT status_state FROM messages WHERE message_id=?1", messageID)
}

func messageNotifiedAt(t *testing.T, conn *dbConn, messageID int64) opt[string] {
	t.Helper()
	rows := tableRows(t, conn, "SELECT notified_at FROM messages WHERE message_id=?1", messageID)
	if len(rows) != 1 {
		t.Fatalf("message %d rows = %v", messageID, rows)
	}
	if rows[0][0] == nil {
		return none[string]()
	}
	return some(rows[0][0].(string))
}

func consumedRecord(record messageRecord, statusTimestamp string) messageRecord {
	record.status = messageStatusCompleted
	record.statusTimestamp = statusTimestamp
	return record
}

func mustSend(t *testing.T, conn *dbConn, from, to int64, text string) messageRecord {
	t.Helper()
	message, err := sendMessage(context.Background(), conn, from, to, text)
	if err != nil {
		t.Fatal(err)
	}
	return message
}

func mustPoll(t *testing.T, conn *dbConn, member int64) []messageRecord {
	t.Helper()
	messages, err := pollMessages(context.Background(), conn, member)
	if err != nil {
		t.Fatal(err)
	}
	return messages
}

func messageIDs(messages []messageRecord) []int64 {
	ids := make([]int64, len(messages))
	for index, message := range messages {
		ids[index] = message.messageID
	}
	return ids
}

func TestBrokerMessaging_SendPollAndShowPreserveTheCompleteBodyAndPollConsumesWhatItReturns(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "messages")
	worker := register(t, conn, fleet, "worker", some("%2"))
	text := "Unicode 世界\n" + strings.Repeat("界", 4096) + "\n\"quoted\" \x1b"
	first := mustSend(t, conn, director, worker, text)
	second := mustSend(t, conn, director, worker, "next")
	if first.text != text || first.fromMemberID != director || first.toMemberID != worker ||
		first.status != messageStatusInputRequired || first.createdAt != first.statusTimestamp {
		t.Fatalf("sent record = %+v", first)
	}
	if shown, err := getMessage(ctx, conn, first.messageID); err != nil || shown != first {
		t.Fatalf("getMessage = %+v %v", shown, err)
	}
	polled := mustPoll(t, conn, worker)
	if len(polled) != 2 {
		t.Fatalf("polled %d messages", len(polled))
	}
	consumedAt := polled[0].statusTimestamp
	if consumedAt < second.statusTimestamp {
		t.Fatalf("consumption time %s precedes the send %s", consumedAt, second.statusTimestamp)
	}
	want := []messageRecord{consumedRecord(second, consumedAt), consumedRecord(first, consumedAt)}
	if !slices.Equal(polled, want) {
		t.Fatalf("poll returns the post-consumption records newest first with one consumption time: %+v", polled)
	}
	if messageNotifiedAt(t, conn, first.messageID) != none[string]() {
		t.Fatal("polling leaves the preview state unchanged")
	}
	for _, record := range polled {
		if shown, err := getMessage(ctx, conn, record.messageID); err != nil || shown != record {
			t.Fatalf("getMessage(%d) = %+v %v", record.messageID, shown, err)
		}
	}
	if again := mustPoll(t, conn, worker); len(again) != 0 {
		t.Fatalf("a second poll returns %v", again)
	}
	later := mustSend(t, conn, director, worker, "after the poll")
	next := mustPoll(t, conn, worker)
	if len(next) != 1 || next[0].messageID != later.messageID || next[0].status != messageStatusCompleted {
		t.Fatalf("next poll = %+v", next)
	}
	if _, err := getMessage(ctx, conn, 999); err == nil || err.Error() != "Message 999 not found" {
		t.Fatalf("getMessage(999) = %v", err)
	}
}

func TestBrokerMessaging_PollingAnUnknownOrInactiveMemberFailsAndConsumesNothing(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "unknown")
	worker := register(t, conn, fleet, "worker", some("%2"))
	message := mustSend(t, conn, director, worker, "held")
	if _, err := pollMessages(ctx, conn, 999); err == nil || err.Error() != "Member 999 not found" {
		t.Fatalf("poll(999) = %v", err)
	}
	if got := messageStatusState(t, conn, message.messageID); got != "input_required" {
		t.Fatalf("status = %q", got)
	}
	if _, err := conn.execute("UPDATE members SET status='deregistered' WHERE member_id=?1", worker); err != nil {
		t.Fatal(err)
	}
	if _, err := pollMessages(ctx, conn, worker); err == nil || err.Error() != fmt.Sprintf("Member %d not found", worker) {
		t.Fatalf("poll(inactive) = %v", err)
	}
	if got := messageStatusState(t, conn, message.messageID); got != "input_required" {
		t.Fatalf("status = %q", got)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("the refused poll leaves no transaction")
	}
}

func TestBrokerMessaging_APollThatFailsBeforeCommitRollsBackAndConsumesNothing(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "rollback")
	worker := register(t, conn, fleet, "worker", some("%2"))
	first := mustSend(t, conn, director, worker, "first")
	second := mustSend(t, conn, director, worker, "second")
	mustExecBatch(t, conn, `CREATE TRIGGER reject_second_consumption BEFORE UPDATE OF status_state ON messages
		WHEN NEW.message_id=2
		BEGIN SELECT RAISE(ABORT, 'injected consumption failure'); END;`)
	if second.messageID != 2 {
		t.Fatalf("second message id = %d", second.messageID)
	}
	if _, err := pollMessages(ctx, conn, worker); err == nil || !strings.Contains(err.Error(), "injected consumption failure") {
		t.Fatalf("poll = %v", err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("the failed poll releases its transaction")
	}
	for _, message := range []messageRecord{first, second} {
		if got := messageStatusState(t, conn, message.messageID); got != "input_required" {
			t.Fatalf("message %d status = %q", message.messageID, got)
		}
	}
	mustExecBatch(t, conn, "DROP TRIGGER reject_second_consumption")
	if got := mustPoll(t, conn, worker); len(got) != 2 {
		t.Fatalf("retried poll = %v", got)
	}
}

func TestBrokerMessaging_APollHoldsTheWriteLockFromSelectionThroughConsumption(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "poll-race")
	worker := register(t, conn, fleet, "worker", some("%2"))
	message := mustSend(t, conn, director, worker, "only once")
	other := fileConn(t, url)
	mustExecBatch(t, other, "PRAGMA busy_timeout=0")
	intercepted := false
	var rivalErr error
	rivalRan := false
	setAuthorizer(t, conn, func(action sqlite3.AuthorizerActionCode, table, _ string) sqlite3.AuthorizerReturnCode {
		if !intercepted && action == sqlite3.AUTH_READ && table == "messages" {
			intercepted = true
			_, rivalErr = pollMessages(ctx, other, worker)
			rivalRan = true
		}
		return sqlite3.AUTH_OK
	})
	polled, err := pollMessages(ctx, conn, worker)
	clearAuthorizer(t, conn)
	if err != nil {
		t.Fatal(err)
	}
	if !rivalRan {
		t.Fatal("the rival poll ran")
	}
	if rivalErr == nil {
		t.Fatal("a rival poll during the selection cannot take the write lock")
	}
	if got := messageIDs(polled); !slices.Equal(got, []int64{message.messageID}) {
		t.Fatalf("polled %v", got)
	}
	if got := messageStatusState(t, conn, message.messageID); got != "completed" {
		t.Fatalf("status = %q", got)
	}
}

func TestBrokerMessaging_ConcurrentPollsOnSeparateConnectionsReturnDisjointSetsCoveringThePendingSet(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "concurrent")
	worker := register(t, conn, fleet, "worker", some("%2"))
	for round := range 20 {
		expected := map[int64]bool{}
		for index := range 10 {
			expected[mustSend(t, conn, director, worker, fmt.Sprintf("%d-%d", round, index)).messageID] = true
		}
		readers := []*dbConn{fileConn(t, url), fileConn(t, url)}
		results := make([][]messageRecord, len(readers))
		failures := make([]error, len(readers))
		var start, done sync.WaitGroup
		start.Add(1)
		for index, reader := range readers {
			done.Add(1)
			go func() {
				defer done.Done()
				start.Wait()
				results[index], failures[index] = pollMessages(ctx, reader, worker)
			}()
		}
		start.Done()
		done.Wait()
		union := map[int64]bool{}
		for index, polled := range results {
			if failures[index] != nil {
				t.Fatalf("round %d: poller %d: %v", round, index, failures[index])
			}
			for _, message := range polled {
				if message.status != messageStatusCompleted {
					t.Fatalf("round %d: message %d is returned consumed", round, message.messageID)
				}
				if union[message.messageID] {
					t.Fatalf("round %d: message %d returned by both polls", round, message.messageID)
				}
				union[message.messageID] = true
			}
		}
		if len(union) != len(expected) {
			t.Fatalf("round %d: polls returned %v, want %v", round, union, expected)
		}
		for id := range expected {
			if !union[id] {
				t.Fatalf("round %d: message %d was not returned", round, id)
			}
		}
		if leftover := mustPoll(t, conn, worker); len(leftover) != 0 {
			t.Fatalf("round %d: leftover %v", round, leftover)
		}
	}
}

func TestBrokerMessaging_MessageIdsFromInterleavedInsertsIncreaseInCommitOrder(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "commit-order")
	worker := register(t, conn, fleet, "worker", some("%2"))
	var commits atomic.Uint64
	type committedID struct {
		sequence uint64
		id       int64
	}
	writers := make([]*dbConn, 4)
	for index := range writers {
		writers[index] = fileConn(t, url)
	}
	observed := make([][]committedID, len(writers))
	failures := make([]error, len(writers))
	var start, done sync.WaitGroup
	start.Add(1)
	for sender, writer := range writers {
		done.Add(1)
		go func() {
			defer done.Done()
			var sequence uint64
			writer.raw.CommitHook(func() bool {
				sequence = commits.Add(1) - 1
				return true
			})
			start.Wait()
			for index := range 25 {
				message, err := sendMessage(ctx, writer, director, worker, fmt.Sprintf("%d-%d", sender, index))
				if err != nil {
					failures[sender] = err
					return
				}
				observed[sender] = append(observed[sender], committedID{sequence: sequence, id: message.messageID})
			}
		}()
	}
	start.Done()
	done.Wait()
	var all []committedID
	for sender, ids := range observed {
		if failures[sender] != nil {
			t.Fatalf("sender %d: %v", sender, failures[sender])
		}
		all = append(all, ids...)
	}
	slices.SortFunc(all, func(a, b committedID) int { return cmp.Compare(a.sequence, b.sequence) })
	if len(all) != 100 {
		t.Fatalf("committed %d messages", len(all))
	}
	for index := 1; index < len(all); index++ {
		if all[index-1].id >= all[index].id {
			t.Fatalf("IDs ordered by commit sequence must strictly increase: %v", all)
		}
	}
}

func TestBrokerMessaging_SelfAndPendingPlacementMessagesHaveDistinctPreviewStates(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "preview")
	pending := register(t, conn, fleet, "pending", none[string]())
	own := mustSend(t, conn, director, director, "self")
	assignment := mustSend(t, conn, director, pending, "pending")
	if got := messageNotifiedAt(t, conn, own.messageID); got != some(own.createdAt) {
		t.Fatalf("self-send notified_at = %v, want %s", got, own.createdAt)
	}
	if got := messageNotifiedAt(t, conn, assignment.messageID); got != none[string]() {
		t.Fatalf("pending assignment notified_at = %v", got)
	}
	if shown, err := getMessage(ctx, conn, own.messageID); err != nil || shown != own {
		t.Fatalf("getMessage = %+v %v", shown, err)
	}
	if got := messageIDs(mustPoll(t, conn, pending)); !slices.Equal(got, []int64{assignment.messageID}) {
		t.Fatalf("pending poll = %v", got)
	}
}

func TestBrokerMessaging_SendingRequiresActiveSameFleetMembersAndALiveRoot(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "alpha")
	worker := register(t, conn, fleet, "worker", none[string]())
	_, other := createFleetFixture(t, conn, "other")
	cases := []struct {
		name              string
		sender, recipient int64
		want              string
	}{
		{name: "unknown sender", sender: 999, recipient: worker, want: "Sender member not found or not active: 999"},
		{name: "unknown recipient", sender: director, recipient: 999, want: "Destination member not found: 999"},
		{name: "other fleet", sender: director, recipient: other, want: fmt.Sprintf("members %d and %d are not in the same fleet.", director, other)},
	}
	for _, tc := range cases {
		_, err := sendMessage(ctx, conn, tc.sender, tc.recipient, "invalid")
		if kind, _ := cafleetKind(err); kind != errorApp || err.Error() != tc.want {
			t.Fatalf("%s: sendMessage = %v, want %q", tc.name, err, tc.want)
		}
	}
	if _, err := deregisterMember(ctx, conn, worker); err != nil {
		t.Fatal(err)
	}
	if _, err := sendMessage(ctx, conn, director, worker, "inactive recipient"); err == nil {
		t.Fatal("an inactive recipient is refused")
	}
	if _, err := sendMessage(ctx, conn, worker, director, "inactive sender"); err == nil {
		t.Fatal("an inactive sender is refused")
	}
	if _, err := conn.execute("UPDATE members SET status='deregistered' WHERE member_id=?1", director); err != nil {
		t.Fatal(err)
	}
	if _, err := sendMessage(ctx, conn, director, director, "inactive root"); err == nil {
		t.Fatal("an inactive root Director is refused")
	}
	if got := tableCount(t, conn, "messages"); got != 0 {
		t.Fatalf("messages = %d", got)
	}
}

func TestBrokerMessaging_CorruptRequiredMessageValuesSurfaceTheirDecodeErrors(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	_, director := createFleetFixture(t, conn, "corrupt")
	message := mustSend(t, conn, director, director, "body")
	if _, err := conn.execute("UPDATE messages SET status_state='corrupt' WHERE message_id=?1", message.messageID); err != nil {
		t.Fatal(err)
	}
	_, err := getMessage(ctx, conn, message.messageID)
	assertInvalidStoredValue(t, err, "messages.status_state", "corrupt")
	if got := mustPoll(t, conn, director); len(got) != 0 {
		t.Fatalf("poll = %v", got)
	}
	if _, err := conn.execute("UPDATE messages SET status_state='input_required', text=X'00' WHERE message_id=?1", message.messageID); err != nil {
		t.Fatal(err)
	}
	if _, err := getMessage(ctx, conn, message.messageID); err == nil {
		t.Fatal("a blob body fails decoding")
	}
	if _, err := pollMessages(ctx, conn, director); err == nil {
		t.Fatal("a poll over a blob body fails")
	}
	if got := messageStatusState(t, conn, message.messageID); got != "input_required" {
		t.Fatalf("status = %q", got)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("the failed poll leaves no transaction")
	}
}

func TestBrokerMessaging_PollScopesByRecipientAndOrdersByStatusTimestampThenDescendingId(t *testing.T) {
	t.Parallel()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "ordering")
	worker := register(t, conn, fleet, "worker", none[string]())
	peer := register(t, conn, fleet, "peer", none[string]())
	var ids []int64
	for _, text := range []string{"first", "second", "third"} {
		ids = append(ids, mustSend(t, conn, director, worker, text).messageID)
	}
	mustSend(t, conn, director, peer, "other inbox")
	mustSend(t, conn, worker, director, "outgoing")
	if _, err := conn.execute("UPDATE messages SET status_timestamp='2026-02-01T00:00:00+00:00',created_at='2020-01-01T00:00:00+00:00' WHERE message_id IN (?1,?2)", ids[0], ids[1]); err != nil {
		t.Fatal(err)
	}
	if _, err := conn.execute("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00',created_at='2099-01-01T00:00:00+00:00' WHERE message_id=?1", ids[2]); err != nil {
		t.Fatal(err)
	}
	rows := mustPoll(t, conn, worker)
	if got := messageIDs(rows); !slices.Equal(got, []int64{ids[1], ids[0], ids[2]}) {
		t.Fatalf("the order is evaluated on the pre-consumption timestamps: %v", got)
	}
	for _, row := range rows {
		if row.toMemberID != worker || row.status != messageStatusCompleted || row.statusTimestamp != rows[0].statusTimestamp {
			t.Fatalf("consumed row = %+v", row)
		}
	}
	if rows[0].statusTimestamp <= "2026-02-01T00:00:00+00:00" {
		t.Fatalf("consumption time %s", rows[0].statusTimestamp)
	}
	for _, untouched := range []struct {
		member int64
		text   string
	}{{peer, "other inbox"}, {director, "outgoing"}} {
		got := mustPoll(t, conn, untouched.member)
		if len(got) != 1 || got[0].text != untouched.text {
			t.Fatalf("%s: poll = %+v", untouched.text, got)
		}
	}
}

func TestBrokerMessaging_SendFailureRollsBackTheMessageWithoutLeavingAnIdentifierGap(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "atomic")
	worker := register(t, conn, fleet, "worker", none[string]())
	mustExecBatch(t, conn, `CREATE TRIGGER reject_message BEFORE INSERT ON messages
		BEGIN SELECT RAISE(ABORT, 'injected message failure'); END;`)
	if _, err := sendMessage(ctx, conn, director, worker, "first"); err == nil || !strings.Contains(err.Error(), "injected message failure") {
		t.Fatalf("sendMessage = %v", err)
	}
	if got := tableCount(t, conn, "messages"); got != 0 {
		t.Fatalf("messages = %d", got)
	}
	mustExecBatch(t, conn, "DROP TRIGGER reject_message")
	if message := mustSend(t, conn, director, worker, "second"); message.messageID != 1 {
		t.Fatalf("message id = %d, want 1", message.messageID)
	}
}

func TestBrokerMessaging_OldestPendingPreviewUsesCreationOrderAndSkipsConsumedAndNotifiedRows(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "preview")
	worker := register(t, conn, fleet, "worker", none[string]())
	ids := []int64{mustSend(t, conn, director, worker, "consumed").messageID}
	if got := mustPoll(t, conn, worker); len(got) != 1 {
		t.Fatalf("poll = %v", got)
	}
	for _, text := range []string{"notified", "oldest", "newest"} {
		ids = append(ids, mustSend(t, conn, director, worker, text).messageID)
	}
	if _, err := conn.execute("UPDATE messages SET notified_at='2026-10-23T00:00:00+00:00' WHERE message_id=?1", ids[1]); err != nil {
		t.Fatal(err)
	}
	mustExecBatch(t, conn, `UPDATE messages SET created_at='2026-01-01T00:00:00+00:00', status_timestamp='2099-01-01T00:00:00+00:00' WHERE message_id=3;
		UPDATE messages SET created_at='2026-02-01T00:00:00+00:00', status_timestamp='2020-01-01T00:00:00+00:00' WHERE message_id=4;`)
	oldest, err := oldestPendingPreview(ctx, conn, worker)
	found, ok := oldest.get()
	if err != nil || !ok || found.messageID != ids[2] {
		t.Fatalf("oldestPendingPreview = %+v %v, want message %d", oldest, err, ids[2])
	}
}
