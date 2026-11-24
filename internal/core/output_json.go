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

func outputJSONMember(row memberRecord) jsonObject {
	var runtime any
	if snapshot, set := row.runtime.get(); set {
		runtime = outputJSONRuntimeSnapshot(*snapshot)
	}
	return jsonObject{
		{"member_id", row.memberID},
		{"name", row.name},
		{"description", row.description},
		{"status", row.status.asStr()},
		{"registered_at", row.registeredAt},
		{"kind", row.kind.asStr()},
		{"placement", outputJSONPlacement(row.placement)},
		{"runtime", runtime},
	}
}

func outputJSONRuntimeSnapshot(row runtimeSnapshot) jsonObject {
	run := row.run
	return jsonObject{
		{"run_id", run.runID},
		{"resolved_mode", run.resolvedMode},
		{"status", run.status.asStr()},
		{"owner_epoch", run.ownerEpoch},
		{"last_health_at", run.lastHealthAt},
		{"coverage", run.coverage.asStr()},
		{"current_interaction_id", row.currentInteractionID},
		{"unresolved_input_ids", row.unresolvedInputIDs},
		{"owner_kind", run.ownerKind.asStr()},
		{"owner_pid", run.ownerPID},
		{"owner_start_identity", run.ownerStartIdentity},
		{"runner_pid", run.runnerPID},
		{"runner_start_identity", run.runnerStartIdentity},
		{"child_pid", run.childPID},
		{"child_start_identity", run.childStartIdentity},
	}
}

func outputJSONRuntimeInteraction(row interactionRecord) jsonObject {
	return jsonObject{
		{"interaction_id", row.interactionID},
		{"run_id", row.runID},
		{"episode_seq", row.episodeSeq},
		{"classification", row.classification.asStr()},
		{"evidence_digest", row.evidenceDigest},
		{"opened_at", row.openedAt},
		{"evidence_json", row.evidenceJSON},
		{"closed_at", row.closedAt},
		{"close_reason", optAsStr(row.closeReason)},
	}
}

func outputJSONRuntimeEvent(view runtimeEventView) jsonObject {
	event := view.event
	var interaction any
	if record, set := view.interaction.get(); set {
		interaction = outputJSONRuntimeInteraction(*record)
	}
	return jsonObject{
		{"event", event.payloadJSON},
		{"notification_state", event.notificationState.asStr()},
		{"message_id", event.messageID},
		{"archive_reason", optAsStr(event.archiveReason)},
		{"run", outputJSONRuntimeSnapshot(view.run)},
		{"interaction", interaction},
	}
}

func outputJSONInputOperation(row inputOperation) jsonObject {
	return jsonObject{{"input", jsonObject{
		{"input_id", row.inputID},
		{"run_id", row.runID},
		{"operation_key", row.operationKey},
		{"kind", row.kind.asStr()},
		{"payload_digest", row.payloadDigest},
		{"state", row.state.asStr()},
		{"created_at", row.createdAt},
		{"updated_at", row.updatedAt},
		{"disposition", optAsStr(row.disposition)},
		{"evidence_ref", row.evidenceRef},
		{"delivery", row.delivery().toJSON()},
		{"deferred_at", row.deferredAt},
		{"defer_reason", row.deferReason},
		{"stop_cause", optAsStr(row.stopCause)},
	}}}
}

type storedEnum interface {
	comparable
	asStr() string
}

// optAsStr is the stored text of an optional enum.
func optAsStr[T storedEnum](value opt[T]) opt[string] {
	if inner, set := value.get(); set {
		return some(inner.asStr())
	}
	return none[string]()
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

func outputJSONMemberPrompt(memberID int64, paneID, text string, inputID opt[int64], delivery deliveryProjection) jsonObject {
	return jsonObject{
		{"member_id", memberID},
		{"pane_id", paneID},
		{"text", text},
		{"input_id", inputID},
		{"delivery", delivery.toJSON()},
	}
}

func outputJSONMemberExec(memberID int64, paneID, codingAgent, command string, inputID opt[int64], delivery deliveryProjection) jsonObject {
	return jsonObject{
		{"member_id", memberID},
		{"pane_id", paneID},
		{"coding_agent", codingAgent},
		{"command", command},
		{"dispatched", delivery.Status == deliveryStatusSubmitted},
		{"input_id", inputID},
		{"delivery", delivery.toJSON()},
	}
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

func outputJSONCreatedFleet(row createdFleet) jsonObject {
	return jsonObject{
		{"fleet_id", row.fleetID},
		{"name", row.name},
		{"created_at", row.createdAt},
		{"director", outputJSONMember(row.director)},
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

func outputJSONPermissionResponseResult(row permissionResponseResult) jsonObject {
	answer := answerWireOf(row.Key)
	return jsonObject{{"response", jsonObject{
		{"status", row.Submission.Status.asStr()},
		{"member_id", row.MemberID},
		{"run_id", row.RunID},
		{"owner_epoch", row.OwnerEpoch},
		{"interaction_id", row.InteractionID},
		{"event_id", row.EventID},
		{"decision", answer.Decision},
		{"choice", answer.Choice},
		{"stage", row.Submission.Stage.asStr()},
		{"output_seq", row.Submission.OutputSeq},
		{"reason", row.Submission.Reason},
	}}}
}

func outputJSONPermissionInspection(row permissionInspection) jsonObject {
	return jsonObject{{"request", jsonObject{
		{"event_id", row.EventID},
		{"message_id", row.MessageID},
		{"member_id", row.MemberID},
		{"fleet_id", row.FleetID},
		{"director_member_id", row.DirectorMemberID},
		{"run_id", row.RunID},
		{"owner_epoch", row.OwnerEpoch},
		{"interaction_id", row.InteractionID},
		{"episode_seq", row.EpisodeSeq},
		{"classification", row.Classification},
		{"visibility", row.Visibility},
		{"backend", row.Backend},
		{"native_version", row.NativeVersion},
		{"answer_rule_id", row.AnswerRuleID},
		{"answer_capability", row.AnswerCapability},
		{"capability_reason", row.CapabilityReason},
		{"supported_decisions", row.SupportedDecisions},
		{"supported_choices", row.SupportedChoices},
		{"request_token", row.RequestToken},
		{"output_seq", row.OutputSeq},
		{"evidence", row.Evidence},
		{"missing_evidence", row.MissingEvidence},
	}}}
}

func outputJSONMemberCapture(capture memberCapture) jsonObject {
	return jsonObject{
		{"member_id", capture.memberID},
		{"pane_id", capture.paneID},
		{"lines", capture.lines},
		{"content", capture.snapshot.content},
		{"captured_at", capture.snapshot.capturedAt},
		{"content_sha256", capture.snapshot.contentSHA256},
	}
}
