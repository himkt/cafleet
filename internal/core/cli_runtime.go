package core

import "context"

func eventRun(ctx context.Context, conn *dbConn, command eventCommand) error {
	switch command := command.(type) {
	case eventCommandShow:
		view, err := getEvent(ctx, conn, command.eventID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONRuntimeEvent(view), func() string { return formatRuntimeEvent(view) })
	default:
		panic("invalid eventCommand")
	}
}

func inputRun(ctx context.Context, conn *dbConn, s settings, command inputCommand, d deadline) error {
	switch command := command.(type) {
	case inputCommandShow:
		input, err := getInput(ctx, conn, command.inputID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONInputOperation(input), func() string { return formatInputOperation(input) })
	case inputCommandReconcile:
		input, failure := reconcileInput(ctx, conn, s, command.inputID, command.disposition.inputDisposition(), command.evidenceRef, d)
		if failure != nil {
			return failure.asError()
		}
		return emit(command.json, outputJSONInputOperation(input), func() string { return formatInputOperation(input) })
	default:
		panic("invalid inputCommand")
	}
}

func (d disposition) inputDisposition() inputDisposition {
	switch d {
	case dispositionSubmitted:
		return inputDispositionSubmitted
	case dispositionNotSubmitted:
		return inputDispositionNotSubmitted
	case dispositionAbandonedUncertain:
		return inputDispositionAbandonedUncertain
	default:
		panic("invalid disposition")
	}
}

func permissionRun(ctx context.Context, conn *dbConn, s settings, command permissionCommand, d deadline) error {
	switch command := command.(type) {
	case permissionCommandInspect:
		inspection, failure := inspectPermission(ctx, conn, s, permissionInspectInput{
			InteractionID:    command.interactionID,
			EventID:          command.eventID,
			DirectorMemberID: command.directorMemberID,
		}, d)
		if failure != nil {
			return failure.asError()
		}
		return emit(command.json, outputJSONPermissionInspection(inspection), func() string { return formatPermissionInspection(inspection) })
	case permissionCommandRespond:
		response, failure := respondPermissionWithHost(ctx, conn, s, permissionRespondInput{
			InteractionID:      command.interactionID,
			EventID:            command.eventID,
			DirectorMemberID:   command.directorMemberID,
			ExpectedRunID:      command.expectedRunID,
			ExpectedOwnerEpoch: command.expectedOwnerEpoch,
			RequestToken:       command.requestToken,
			Key:                command.answerKey(),
		}, d, nativeInputHost{})
		if failure != nil {
			return failure.asError()
		}
		if err := emit(command.json, outputJSONPermissionResponseResult(response), func() string { return formatPermissionResponseResult(response) }); err != nil {
			return err
		}
		if response.Submission.Status == submissionStatusAccepted {
			return nil
		}
		reason, reasoned := response.Submission.Reason.get()
		if !reasoned {
			panic("a nonaccepted permission response supplies its reason")
		}
		return appErrorf("permission response %s for interaction %d: %s", response.Submission.Status.asStr(), command.interactionID, reason)
	default:
		panic("invalid permissionCommand")
	}
}

func (c permissionCommandRespond) answerKey() answerKey {
	decision, decided := c.decision.get()
	choice, chosen := c.choice.get()
	switch {
	case decided && !chosen:
		parsed, err := permissionDecisionTryFrom(decision)
		if err != nil {
			panic("the parser admits only permission decisions: " + decision)
		}
		return answerKeyDecision{decision: parsed}
	case chosen && !decided:
		return answerKeyChoice{choice: choice}
	default:
		panic("the parser requires exactly one answer argument")
	}
}

// permissionError names the command's run and interaction on a failure before the permission
// operation starts.
func permissionError(command permissionCommand, source *cafleetError) *cafleetError {
	scope := inputContext{}
	switch command := command.(type) {
	case permissionCommandInspect:
		scope.interactionID = some(command.interactionID)
	case permissionCommandRespond:
		scope.runID = some(command.expectedRunID)
		scope.interactionID = some(command.interactionID)
	default:
		panic("invalid permissionCommand")
	}
	return scope.failure(source).asError()
}

// permissionDispatch runs a permission command under one deadline that also bounds opening its
// connection.
func permissionDispatch(ctx context.Context, s settings, command permissionCommand) (err error) {
	d := newDeadline()
	conn, err := invocationConnection(ctx, s.databaseURL, d)
	if err != nil {
		return permissionError(command, cafleetErrorOf(err))
	}
	defer func() {
		err = mergeCleanup(err, conn.close())
	}()
	if guarded, ok := directorOnlyOf(command); ok {
		if err := requireDirector(conn, guarded); err != nil {
			return err
		}
	}
	return permissionRun(ctx, conn, s, command, d)
}
