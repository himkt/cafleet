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
	case memberCommandShow:
		return memberShow(ctx, conn, command.memberID, command.json)
	case memberCommandCreate, memberCommandDelete, memberCommandPrompt,
		memberCommandExec, memberCommandPress, memberCommandCapture:
		return commandNotYetPorted()
	default:
		panic("invalid memberCommand")
	}
}

// memberShow prints a member; a member with a runtime also needs the runner observation probe
// of the runtime control protocol, which is not ported yet.
func memberShow(ctx context.Context, conn *dbConn, memberID int64, json bool) error {
	member, err := activeMember(ctx, conn, memberID)
	if err != nil {
		return err
	}
	if _, set := member.runtime.get(); set {
		return commandNotYetPorted()
	}
	return emit(json, outputJSONMember(member), func() string { return formatMemberDetail(member) })
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
