use crate::broker::runtime::{InputOperation, RuntimeEventView, RuntimeSnapshot};

pub fn format_runtime_snapshot(snapshot: &RuntimeSnapshot) -> String {
    let run = &snapshot.run;
    format!(
        "Runtime: run={} mode={} status={} coverage={}",
        run.run_id,
        run.resolved_mode,
        run.status.as_str(),
        run.coverage.as_str()
    )
}

pub fn format_runtime_event(view: &RuntimeEventView) -> String {
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

pub fn format_input_operation(input: &InputOperation) -> String {
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
