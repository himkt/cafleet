package core

func outputJSONPlacement(row placement) jsonObject {
	return jsonObject{
		{"backend", row.backend},
		{"mux_session", row.muxSession},
		{"mux_window_id", row.muxWindowID},
		{"mux_window_name", row.muxWindowName},
		{"window_ordinal", row.windowOrdinal},
		{"column_side", row.columnSide.asStr()},
		{"mux_pane_id", row.muxPaneID},
		{"coding_agent", row.codingAgent},
		{"created_at", row.createdAt},
	}
}

func outputJSONMemberActivity(row memberActivity) jsonObject {
	return jsonObject{
		{"member_id", row.member.memberID},
		{"name", row.member.name},
		{"kind", row.member.kind.asStr()},
		{"placement", outputJSONPlacement(row.member.placement)},
		{"last_sent", row.lastSent},
		{"last_recv", row.lastRecv},
		{"last_consumed", row.lastConsumed},
		{"idle", row.idle},
	}
}

func outputJSONMessage(row messageRecord) jsonObject {
	return jsonObject{
		{"message_id", row.messageID},
		{"from_member_id", row.fromMemberID},
		{"to_member_id", row.toMemberID},
		{"created_at", row.createdAt},
		{"status_state", row.status.asStr()},
		{"status_timestamp", row.statusTimestamp},
		{"text", row.text},
	}
}

func outputJSONMessageEnvelope(row messageRecord) jsonObject {
	return jsonObject{{"message", outputJSONMessage(row)}}
}

func outputJSONFleet(row fleetRecord) jsonObject {
	return jsonObject{
		{"fleet_id", row.fleetID},
		{"name", row.name},
		{"created_at", row.createdAt},
		{"deleted_at", row.deletedAt},
		{"director_member_id", row.directorMemberID},
	}
}

func outputJSONFleetSummary(row fleetSummary) jsonObject {
	return jsonObject{
		{"fleet_id", row.fleetID},
		{"name", row.name},
		{"created_at", row.createdAt},
		{"director_member_id", row.directorMemberID},
		{"member_count", row.memberCount},
	}
}

func outputJSONDeletedFleet(row deletedFleet) jsonObject {
	members := make([]jsonObject, 0, len(row.members))
	for _, member := range row.members {
		members = append(members, jsonObject{{"member_id", member.memberID}, {"pane_status", member.paneStatus}})
	}
	return jsonObject{
		{"fleet_id", row.fleetID},
		{"deregistered_count", row.deregisteredCount},
		{"members", members},
	}
}
