package core

import (
	"context"
	"fmt"
)

const messageColumns = "message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text"

const pendingPreview = "status_state='input_required' AND notified_at IS NULL"

func mapMessageRow(row sqlRow) (messageRecord, error) {
	var message messageRecord
	var err error
	if message.messageID, err = row.int64(0); err != nil {
		return messageRecord{}, err
	}
	if message.fromMemberID, err = row.int64(1); err != nil {
		return messageRecord{}, err
	}
	if message.toMemberID, err = row.int64(2); err != nil {
		return messageRecord{}, err
	}
	if message.createdAt, err = row.text(3); err != nil {
		return messageRecord{}, err
	}
	if message.status, err = rowStoredEnum(row, 4, messageStatusTryFrom); err != nil {
		return messageRecord{}, err
	}
	if message.statusTimestamp, err = row.text(5); err != nil {
		return messageRecord{}, err
	}
	if message.text, err = row.text(6); err != nil {
		return messageRecord{}, err
	}
	return message, nil
}

func getMessage(_ context.Context, conn *dbConn, messageID int64) (messageRecord, error) {
	found, err := queryRowOptional(conn, "SELECT "+messageColumns+" FROM messages WHERE message_id=?1",
		[]any{messageID}, mapMessageRow)
	if err != nil {
		return messageRecord{}, err
	}
	message, set := found.get()
	if !set {
		return messageRecord{}, appError(fmt.Sprintf("Message %d not found", messageID))
	}
	return message, nil
}

func senderFleet(ctx context.Context, conn *dbConn, fromMemberID int64) (int64, error) {
	fleet, err := activeMemberFleet(ctx, conn, fromMemberID)
	if err != nil {
		return 0, err
	}
	fleetID, set := fleet.get()
	if !set {
		return 0, appError(fmt.Sprintf("Sender member not found or not active: %d", fromMemberID))
	}
	return fleetID, nil
}

func sendMessage(ctx context.Context, conn *dbConn, fromMemberID, toMemberID int64, text string) (messageRecord, error) {
	return inTransaction(conn, transactionImmediate, func() (messageRecord, error) {
		return insertMessageValidated(ctx, conn, fromMemberID, toMemberID, text)
	})
}

func insertMessageValidated(ctx context.Context, conn *dbConn, fromMemberID, toMemberID int64, text string) (messageRecord, error) {
	fleetID, err := senderFleet(ctx, conn, fromMemberID)
	if err != nil {
		return messageRecord{}, err
	}
	if _, err := requireLiveFleet(ctx, conn, fleetID); err != nil {
		return messageRecord{}, err
	}
	recipient, err := memberFleetStatus(ctx, conn, toMemberID)
	if err != nil {
		return messageRecord{}, err
	}
	membership, found := recipient.get()
	if !found || membership.status != memberStatusActive {
		return messageRecord{}, appError(fmt.Sprintf("Destination member not found: %d", toMemberID))
	}
	if membership.fleetID != fleetID {
		return messageRecord{}, appError(fmt.Sprintf("members %d and %d are not in the same fleet.", fromMemberID, toMemberID))
	}
	now := nowStamp()
	// A self-send owes no keystroke, so it is settled at insert.
	notifiedAt := none[string]()
	if toMemberID == fromMemberID {
		notifiedAt = some(now)
	}
	return queryRow(conn,
		`INSERT INTO messages (from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at)
         VALUES (?1, ?2, ?3, 'input_required', ?3, ?4, ?5) RETURNING `+messageColumns,
		[]any{fromMemberID, toMemberID, now, text, notifiedAt}, mapMessageRow)
}

func oldestPendingPreview(_ context.Context, conn *dbConn, memberID int64) (opt[messageRecord], error) {
	return queryRowOptional(conn,
		"SELECT "+messageColumns+" FROM messages WHERE to_member_id=?1 AND "+pendingPreview+" ORDER BY created_at, message_id LIMIT 1",
		[]any{memberID}, mapMessageRow)
}

func pollMessages(ctx context.Context, conn *dbConn, memberID int64) ([]messageRecord, error) {
	return inTransaction(conn, transactionImmediate, func() ([]messageRecord, error) {
		fleet, err := activeMemberFleet(ctx, conn, memberID)
		if err != nil {
			return nil, err
		}
		if _, set := fleet.get(); !set {
			return nil, appError(fmt.Sprintf("Member %d not found", memberID))
		}
		pendingIDs, err := queryRows(conn,
			"SELECT message_id FROM messages WHERE to_member_id=?1 AND status_state='input_required' ORDER BY status_timestamp DESC, message_id DESC",
			[]any{memberID}, func(row sqlRow) (int64, error) { return row.int64(0) })
		if err != nil {
			return nil, err
		}
		consumedAt := nowStamp()
		consumed := []messageRecord{}
		for _, messageID := range pendingIDs {
			message, err := queryRow(conn,
				"UPDATE messages SET status_state='completed', status_timestamp=?1 WHERE message_id=?2 AND status_state='input_required' RETURNING "+messageColumns,
				[]any{consumedAt, messageID}, mapMessageRow)
			if err != nil {
				return nil, err
			}
			consumed = append(consumed, message)
		}
		return consumed, nil
	})
}
