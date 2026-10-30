use crate::broker::runtime::{InputOperation, RuntimeEventView, RuntimeSnapshot};

pub(crate) fn format_runtime_recovery(report: &crate::runtime::runner::RecoveryReport) -> String {
    let mut text = format!(
        "Recovery: member={} run={} outcome={} status={} replayed={} inbox={} archived={}",
        report.member_id,
        report.run_id,
        report.outcome.as_str(),
        report.status.as_str(),
        report.replayed_count,
        report.inbox_count,
        report.archived_count
    );
    for diagnostic in &report.cleanup_diagnostics {
        text.push_str("\nCleanup: ");
        match diagnostic {
            serde_json::Value::String(value) => text.push_str(value),
            value => text.push_str(&value.to_string()),
        }
    }
    text
}

pub(crate) fn format_runtime_snapshot(snapshot: &RuntimeSnapshot) -> String {
    let run = &snapshot.run;
    format!(
        "Runtime: run={} mode={} status={} coverage={} review={}",
        run.run_id,
        run.resolved_mode,
        run.status.as_str(),
        run.coverage.as_str(),
        run.effective_policy_json.as_ref().map_or("-", |record| {
            record["automatic_review"]
                .as_str()
                .expect("validated effective policy review state")
        })
    )
}

pub(crate) fn format_runtime_event(view: &RuntimeEventView) -> String {
    let event = &view.event;
    let message = event
        .message_id
        .map_or_else(|| "-".into(), |id| id.to_string());
    let mut text = format!(
        "Event: {} run={} seq={} kind={} notification={} message={}\n{}",
        event.event_id,
        event.run_id,
        event.event_seq,
        event.kind.as_str(),
        event.notification_state.as_str(),
        message,
        format_runtime_snapshot(&view.run)
    );
    if let Some(interaction) = &view.interaction {
        text.push_str(&format!(
            "\nInteraction: {} classification={} closed_at={} close_reason={}",
            interaction.interaction_id,
            interaction.classification.as_str(),
            interaction.closed_at.as_deref().unwrap_or("-"),
            interaction
                .close_reason
                .map_or("-", |reason| reason.as_str())
        ));
    }
    text
}

pub(crate) fn format_input_operation(input: &InputOperation) -> String {
    let mut text = format!(
        "Input: {} run={} kind={} state={} disposition={} message={}",
        input.input_id,
        input.run_id,
        input.kind.as_str(),
        input.state.as_str(),
        input
            .disposition
            .map_or("-", |disposition| disposition.as_str()),
        input
            .message_id
            .map_or_else(|| "-".into(), |id| id.to_string())
    );
    if let Some(response) = &input.permission_response {
        let metadata = &response.metadata;
        text.push_str(&format!("\nPermission decision: interaction={} event={} decision={} source={} mode={} director={} epoch={}",
            metadata.interaction_id,metadata.event_id,metadata.decision.as_str(),metadata.decision_source.as_str(),metadata.director_mode.as_str(),metadata.director_member_id,metadata.expected_owner_epoch));
    }
    text
}

pub(crate) fn format_permission_response_result(
    row: &crate::runtime::runner::PermissionResponseResult,
) -> String {
    let mut text = format!(
        "Permission response: interaction={} run={} decision={} source={} status={} input={} stage={} output_seq={}",
        row.interaction_id,
        row.run_id,
        row.decision.as_str(),
        row.decision_source.as_str(),
        row.submission.status.as_str(),
        row.submission
            .input_id
            .map_or_else(|| "-".into(), |value| value.to_string()),
        row.submission.stage.as_str(),
        row.submission
            .output_seq
            .map_or_else(|| "-".into(), |value| value.to_string())
    );
    if let Some(reason) = &row.submission.reason {
        text.push_str(&format!("\nReason: {reason}"));
    }
    text
}

pub(crate) fn format_permission_inspection(
    row: &crate::runtime::runner::PermissionInspection,
) -> String {
    let decisions = row
        .supported_decisions
        .iter()
        .map(|value| value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let visibility = row.visibility.as_str();
    let mut text = format!(
        "Permission: interaction={} event={} member={} run={} epoch={} visibility={} answers={}\nRequest: token={} output_seq={} profile={} rule={}",
        row.interaction_id,
        row.event_id,
        row.member_id,
        row.run_id,
        row.owner_epoch,
        visibility,
        if decisions.is_empty() {
            "-"
        } else {
            &decisions
        },
        row.request_token.as_deref().unwrap_or("-"),
        row.output_seq,
        row.answer_profile_version.as_deref().unwrap_or("-"),
        row.answer_rule_id.as_deref().unwrap_or("-")
    );
    if let Some(reason) = &row.capability_reason {
        text.push_str(&format!("\nCapability: {reason}"));
    }
    if !row.missing_evidence.is_empty() {
        text.push_str(&format!(
            "\nMissing evidence: {}",
            row.missing_evidence
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    text
}
