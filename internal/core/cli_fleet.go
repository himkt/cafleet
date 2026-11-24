package core

import "context"

func fleetRun(ctx context.Context, conn *dbConn, command fleetCommand) error {
	switch command := command.(type) {
	case fleetCommandList:
		fleets, err := listFleets(ctx, conn)
		if err != nil {
			return err
		}
		payload := make([]jsonObject, 0, len(fleets))
		for _, fleet := range fleets {
			payload = append(payload, outputJSONFleetSummary(fleet))
		}
		return emit(command.json, payload, func() string { return formatFleetList(fleets) })
	case fleetCommandShow:
		fleet, err := requireFleet(ctx, conn, command.fleetID)
		if err != nil {
			return err
		}
		return emit(command.json, outputJSONFleet(fleet), func() string { return formatFleetShow(fleet) })
	case fleetCommandCreate, fleetCommandDelete:
		return commandNotYetPorted()
	default:
		panic("invalid fleetCommand")
	}
}
