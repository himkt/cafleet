package core

import (
	"context"
	"strconv"
	"strings"
)

func messageRun(ctx context.Context, conn *dbConn, command messageCommand) error {
	switch command := command.(type) {
	case messageCommandSend:
		text, err := resolveBody(command.body.text, command.body.file)
		if err != nil {
			return err
		}
		message, err := withDatabaseBudget(ctx, conn, newDeadline(), func(ctx context.Context, conn *dbConn) (messageRecord, error) {
			return sendMessage(ctx, conn, command.fromMemberID, command.toMemberID, text)
		})
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONMessageEnvelope(message), func() string { return formatMessageSent(message) })
	case messageCommandPoll:
		messages, err := pollMessages(ctx, conn, command.memberID)
		if err != nil {
			return err
		}
		payload := make([]jsonObject, 0, len(messages))
		for _, message := range messages {
			payload = append(payload, outputJSONMessage(message))
		}
		return emit(command.json, payload, func() string { return formatMessages(messages) })
	case messageCommandShow:
		message, err := getMessage(ctx, conn, command.messageID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONMessageEnvelope(message), func() string { return formatMessage(message) })
	default:
		panic("invalid messageCommand")
	}
}

// pollCommand is the member's consuming poll, carrying the database URL as one shell word when
// given.
func pollCommand(memberID int64, databaseURL opt[string]) string {
	command := "cafleet message poll " + strconv.FormatInt(memberID, 10) + " --json"
	if url, set := databaseURL.get(); set {
		return command + " --database-url " + shellWord(url)
	}
	return command
}

func shellWord(value string) string {
	safe := func(r rune) bool {
		return r < 0x80 && (r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' ||
			strings.ContainsRune("_./:~@%+=,-", r))
	}
	for _, r := range value {
		if !safe(r) {
			return "'" + strings.ReplaceAll(value, "'", `'\''`) + "'"
		}
	}
	return value
}
