package core

import (
	"context"
	"fmt"
)

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

func memberRun(ctx context.Context, conn *dbConn, s settings, command memberCommand, d deadline) error {
	switch command := command.(type) {
	case memberCommandList:
		return memberList(ctx, conn, command.fleetID, command.json)
	case memberCommandShow:
		return memberShow(ctx, conn, command.memberID, command.json)
	case memberCommandPrompt:
		return cliMemberPrompt(ctx, conn, s, command.memberID, command.text, command.json, d)
	case memberCommandExec:
		text, resolved := command.body.command.get()
		if !resolved {
			panic("exec input was resolved before its deadline")
		}
		return cliMemberExec(ctx, conn, s, command.memberID, text, command.json, d)
	case memberCommandCreate, memberCommandDelete, memberCommandPress, memberCommandCapture:
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

func requirePane(member memberRecord, action string) (string, error) {
	pane, placed := member.placement.muxPaneID.get()
	if !placed {
		return "", appErrorf("member %d has no pane yet (pending placement) — nothing to %s.", member.memberID, action)
	}
	return pane, nil
}

func wrappedDeliveryRefusal(label string, memberID int64, result submissionResult) string {
	if result.Status == submissionStatusRejected && result.Stage == submissionStageZeroBytes {
		cause, caused := result.Reason.get()
		if !caused {
			panic("a pre-write rejection has its cause")
		}
		return fmt.Sprintf("%s was not dispatched to member %d: %s", label, memberID, cause)
	}
	var description string
	switch result.Status {
	case submissionStatusHeld:
		description = "held"
	case submissionStatusRejected:
		description = "rejected"
	case submissionStatusUncertain:
		description = "uncertain"
	case submissionStatusAccepted, submissionStatusQueued:
		panic("submitted and queued results are not refusals")
	default:
		panic("invalid submissionStatus")
	}
	reason := fmt.Sprintf("%s delivery to member %d is %s", label, memberID, description)
	if detail, set := result.Reason.get(); set {
		reason += ": " + detail
	}
	if delivery, delivered := result.Delivery.get(); delivered {
		if method, set := delivery.Method.get(); set {
			reason += " (method=" + method.asStr() + ")"
		}
	}
	return reason
}

func dispatchInput(ctx context.Context, conn *dbConn, s settings, member memberRecord, text string, kind inputKind, label string, d deadline) (opt[int64], deliveryProjection, error) {
	snapshot, recorded := member.runtime.get()
	if !recorded {
		panic("every ordinary member records its run")
	}
	run := snapshot.run
	random, err := randomHex32()
	if err != nil {
		return none[int64](), deliveryProjection{}, appError("cannot generate input operation key: " + cafleetErrorOf(err).message())
	}
	request := submitInput{
		OperationKey: fmt.Sprintf("%s:%d:%s", kind.asStr(), run.runID, random),
		InputKind:    kind,
		Literal:      text,
	}
	result, failure := submitWrappedInput(ctx, conn, s, run, member.placement, request, d)
	if failure != nil {
		return none[int64](), deliveryProjection{}, failure.asError()
	}
	if result.Status != submissionStatusAccepted && result.Status != submissionStatusQueued {
		scope := inputContext{
			runID:         some(run.runID),
			interactionID: snapshot.currentInteractionID,
			inputID:       result.InputID,
			operationKey:  some(request.OperationKey),
		}
		return none[int64](), deliveryProjection{}, scope.failure(appError(wrappedDeliveryRefusal(label, member.memberID, result))).asError()
	}
	delivery, delivered := result.Delivery.get()
	if !delivered {
		panic("validated submission replies carry their delivery")
	}
	return result.InputID, delivery, nil
}

type cliMemberDispatch struct {
	kind            inputKind
	command         string
	label           string
	directorRefusal string
}

type dispatchedInput struct {
	member   memberRecord
	pane     string
	inputID  opt[int64]
	delivery deliveryProjection
}

func dispatchToMember(ctx context.Context, conn *dbConn, s settings, memberID int64, text string, dispatch cliMemberDispatch, d deadline) (dispatchedInput, error) {
	member, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (memberRecord, error) {
		return activeMember(ctx, conn, memberID)
	})
	if err != nil {
		return dispatchedInput{}, err
	}
	if member.kind == memberKindDirector {
		return dispatchedInput{}, appError(dispatch.directorRefusal)
	}
	pane, err := requirePane(member, dispatch.command)
	if err != nil {
		return dispatchedInput{}, err
	}
	inputID, delivery, err := dispatchInput(ctx, conn, s, member, text, dispatch.kind, dispatch.label, d)
	if err != nil {
		return dispatchedInput{}, err
	}
	return dispatchedInput{member: member, pane: pane, inputID: inputID, delivery: delivery}, nil
}

func cliMemberPrompt(ctx context.Context, conn *dbConn, s settings, memberID int64, text string, json bool, d deadline) error {
	if !validInput(text) {
		return usageError("text must be one non-empty line without control characters.")
	}
	sent, err := dispatchToMember(ctx, conn, s, memberID, text, cliMemberDispatch{
		kind:            inputKindPrompt,
		command:         "prompt",
		label:           "prompt",
		directorRefusal: "cannot prompt the Director's own pane",
	}, d)
	if err != nil {
		return err
	}
	return emit(json, outputJSONMemberPrompt(memberID, sent.pane, text, sent.inputID, sent.delivery), func() string {
		return formatMemberPrompt(memberID, text, sent.member.name, sent.pane, sent.inputID, sent.delivery)
	})
}

func cliMemberExec(ctx context.Context, conn *dbConn, s settings, memberID int64, command string, json bool, d deadline) error {
	sent, err := dispatchToMember(ctx, conn, s, memberID, command, cliMemberDispatch{
		kind:            inputKindExec,
		command:         "exec",
		label:           "native command",
		directorRefusal: "cannot exec in the Director's own pane",
	}, d)
	if err != nil {
		return err
	}
	agent := sent.member.placement.codingAgent
	return emit(json, outputJSONMemberExec(memberID, sent.pane, agent, command, sent.inputID, sent.delivery), func() string {
		return formatMemberExec(memberID, sent.pane, sent.inputID, sent.delivery)
	})
}
