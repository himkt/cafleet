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

func inputRun(ctx context.Context, conn *dbConn, command inputCommand) error {
	switch command := command.(type) {
	case inputCommandShow:
		input, err := getInput(ctx, conn, command.inputID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONInputOperation(input), func() string { return formatInputOperation(input) })
	case inputCommandReconcile:
		return commandNotYetPorted()
	default:
		panic("invalid inputCommand")
	}
}
