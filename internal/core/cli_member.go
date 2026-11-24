package core

import "context"

// prepareInput resolves an exec command's native command line before the command's deadline
// starts.
func prepareInput(command memberCommand) (memberCommand, error) {
	exec, ok := command.(memberCommandExec)
	if !ok {
		return command, nil
	}
	resolved, err := resolveCommandBody(exec.body.command, exec.body.file)
	if err != nil {
		return nil, err
	}
	exec.body = execBodyArgs{command: some(resolved), file: none[string]()}
	return exec, nil
}

func memberRun(ctx context.Context, conn *dbConn, command memberCommand) error {
	switch command := command.(type) {
	case memberCommandList:
		return memberList(ctx, conn, command.fleetID, command.json)
	case memberCommandCreate, memberCommandDelete, memberCommandShow, memberCommandPrompt,
		memberCommandExec, memberCommandPress, memberCommandCapture:
		return commandNotYetPorted()
	default:
		panic("invalid memberCommand")
	}
}

func memberList(ctx context.Context, conn *dbConn, fleetID int64, json bool) error {
	members, err := listMembers(ctx, conn, fleetID)
	if err != nil {
		return err
	}
	payload := make([]jsonObject, 0, len(members))
	for _, member := range members {
		payload = append(payload, outputJSONMemberActivity(member))
	}
	return emit(json, payload, func() string { return formatMemberList(members) })
}
