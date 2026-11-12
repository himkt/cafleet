use crate::broker::CreatedFleet;
use crate::broker::DeletedFleet;
use crate::broker::FleetRecord;
use crate::broker::FleetSummary;
use crate::broker::MemberActivity;
use crate::broker::MemberRecord;
use crate::broker::MessageRecord;
use crate::broker::Placement;
use crate::broker::RegisteredMember;
use crate::broker::runtime::DeliveryProjection;
use crate::broker::runtime::DeliveryStatus;
use crate::broker::runtime::InputOperation;
use crate::broker::runtime::RuntimeEventView;
use crate::broker::runtime::RuntimeSnapshot;
use crate::output::truncate_text;

const TEXT_BODY_LIMIT: usize = 200;

pub(crate) fn format_message(message: &MessageRecord) -> String {
    let header = format!(
        "[{} | from:{} | {}]",
        message.message_id, message.from_member_id, message.status_timestamp
    );
    let body = truncate_text(&message.text, TEXT_BODY_LIMIT);
    if body.is_empty() {
        header
    } else {
        format!("{header}\n{body}")
    }
}

pub(crate) fn format_messages(messages: &[MessageRecord]) -> String {
    if messages.is_empty() {
        return "No messages found.".into();
    }
    messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn format_message_sent(
    message: &MessageRecord,
    input_id: Option<i64>,
    delivery: &DeliveryProjection,
) -> String {
    with_delivery(
        format!("Message sent.\n{}", format_message(message)),
        input_id,
        delivery,
    )
}

pub(crate) fn format_message_acknowledged(message: &MessageRecord) -> String {
    format!("Message acknowledged.\n{}", format_message(message))
}

pub(crate) fn format_message_wait(member_id: i64, messages: &[MessageRecord]) -> String {
    if messages.is_empty() {
        format!("Timed out waiting for member {member_id}.")
    } else {
        format!(
            "Messages available for member {member_id}.\n{}",
            format_messages(messages)
        )
    }
}

pub(crate) fn format_fleet_list(fleets: &[FleetSummary]) -> String {
    if fleets.is_empty() {
        return "No fleets found.".into();
    }
    let mut lines = vec!["FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT".into()];
    for fleet in fleets {
        lines.push(format!(
            "{} {} {} {} {}",
            fleet.fleet_id,
            fleet.director_member_id,
            fleet.name,
            fleet.member_count,
            fleet.created_at
        ));
    }
    lines.join("\n")
}

pub(crate) fn format_fleet_show(fleet: &FleetRecord) -> String {
    let mut text = format!(
        "fleet_id: {}\nname: {}\ncreated_at: {}",
        fleet.fleet_id, fleet.name, fleet.created_at
    );
    if let Some(deleted) = &fleet.deleted_at {
        text.push_str(&format!("\ndeleted_at: {deleted}"));
    }
    text
}

pub(crate) fn format_member_delete(member_id: i64, pane_status: &str) -> String {
    format!("Member deleted.\n  member_id: {member_id}\n  pane_id:   {pane_status}")
}

fn with_delivery(mut text: String, input_id: Option<i64>, delivery: &DeliveryProjection) -> String {
    text.push_str(&format!(
        "\nDelivery: status={} method={} reason={}",
        delivery.status.as_str(),
        delivery.method.map_or("-", |method| method.as_str()),
        delivery.reason.as_deref().unwrap_or("-")
    ));
    if let Some(id) = input_id {
        text.push_str(&format!("\nInput: {id}"));
        if delivery.status == DeliveryStatus::Queued {
            text.push_str(&format!(
                "\nThe member's runner will retry it after the blocking work ends; run cafleet input show {id} --json for its result."
            ));
        }
    }
    text
}

pub(crate) fn format_member_prompt(
    member_id: i64,
    text: &str,
    name: &str,
    pane: &str,
    input_id: Option<i64>,
    delivery: &DeliveryProjection,
) -> String {
    let first = if delivery.status == DeliveryStatus::Queued {
        format!("Queued prompt for member {member_id} ({pane}).")
    } else {
        format!("Sent prompt {text:?} to member {name} ({pane}).")
    };
    with_delivery(first, input_id, delivery)
}

pub(crate) fn format_member_exec(
    member_id: i64,
    pane: &str,
    input_id: Option<i64>,
    delivery: &DeliveryProjection,
) -> String {
    let first = if delivery.status == DeliveryStatus::Queued {
        format!("Queued native command for member {member_id} ({pane}).")
    } else {
        format!(
            "Dispatched native command to member {member_id} ({pane}). Inspect the pane for its result."
        )
    };
    with_delivery(first, input_id, delivery)
}

pub(crate) fn format_member_press(value: &crate::runtime::KeyAcknowledgment) -> String {
    format!(
        "Acknowledged keys {} for member {} ({}).",
        serde_json::to_string(&value.keys).expect("key strings serialize"),
        value.member_id,
        value.pane_id
    )
}

pub(crate) fn format_member_detail(member: &MemberRecord) -> String {
    let mut text = format!(
        "{} {} {}",
        member.member_id,
        member.name,
        member.status.as_str()
    );
    if let Some(runtime) = &member.runtime {
        text.push('\n');
        text.push_str(&crate::output::format_runtime_snapshot(runtime));
    }
    text
}

pub(crate) fn format_fleet_create(fleet: &CreatedFleet) -> String {
    format!("{} director={}", fleet.fleet_id, fleet.director.member_id)
}

pub(crate) fn format_fleet_delete(deleted: &DeletedFleet) -> String {
    let mut lines = vec![format!(
        "Deleted fleet {}. Deregistered {} members.",
        deleted.fleet_id, deleted.deregistered_count
    )];
    for member in &deleted.members {
        lines.push(format!(
            "  member {}: {}",
            member.member_id, member.pane_status
        ));
    }
    lines.join("\n")
}

pub(crate) fn format_member(member: &RegisteredMember, placement: &Placement) -> String {
    let pane = placement.mux_pane_id.as_deref().unwrap_or("(pending)");
    format!(
        "{} {} backend={} pane={pane}",
        member.member_id, member.name, placement.coding_agent
    )
}

fn format_idle(idle: Option<i64>) -> String {
    assert!(
        idle.is_none_or(|seconds| seconds >= 0),
        "member idle is nonnegative"
    );
    match idle {
        None => "-".into(),
        Some(seconds) if seconds < 60 => format!("{seconds}s"),
        Some(seconds) if seconds < 3600 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{}h", seconds / 3600),
    }
}

pub(crate) fn format_member_list(members: &[MemberActivity]) -> String {
    if members.is_empty() {
        return "0 members.".into();
    }
    let plural = if members.len() > 1 { "s" } else { "" };
    let mut lines = vec![
        format!("{} member{plural}:", members.len()),
        "  member_id  name           kind      backend   pane_id  idle".into(),
    ];
    for row in members {
        let member = &row.member;
        let backend = member.placement.coding_agent.as_str();
        let pane = member
            .placement
            .mux_pane_id
            .as_deref()
            .unwrap_or("(pending)");
        lines.push(format!(
            "  {:<9}  {:<13}  {:<8}  {backend:<8}  {pane:<7}  {}",
            member.member_id,
            member.name,
            member.kind.as_str(),
            format_idle(row.idle)
        ));
    }
    lines.join("\n")
}

pub(crate) fn format_runtime_recovery(report: &crate::runtime::RecoveryReport) -> String {
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
        text.push_str(&format!("\nPermission decision: interaction={} event={} decision={} choice={} source={} mode={} director={} epoch={}",
            metadata.interaction_id,metadata.event_id,metadata.key.decision().map_or("-", |decision| decision.as_str()),metadata.key.choice().unwrap_or("-"),metadata.decision_source.as_str(),metadata.director_mode.as_str(),metadata.director_member_id,metadata.expected_owner_epoch));
    }
    text
}

pub(crate) fn format_permission_response_result(
    row: &crate::runtime::PermissionResponseResult,
) -> String {
    let mut text = format!(
        "Permission response: interaction={} run={} decision={} choice={} source={} status={} input={} stage={} output_seq={}",
        row.interaction_id,
        row.run_id,
        row.key.decision().map_or("-", |decision| decision.as_str()),
        row.key.choice().unwrap_or("-"),
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

pub(crate) fn format_permission_inspection(row: &crate::runtime::PermissionInspection) -> String {
    let decisions = row
        .supported_decisions
        .iter()
        .map(|value| value.as_str())
        .chain(
            row.supported_choices
                .iter()
                .map(|choice| choice.choice_id.as_str()),
        )
        .collect::<Vec<_>>()
        .join(",");
    let visibility = row.visibility.as_str();
    let mut text = format!(
        "Permission: interaction={} event={} member={} run={} epoch={} visibility={} answers={}\nRequest: token={} output_seq={} rule={}",
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

#[cfg(test)]
mod tests {
    use crate::broker::DeletedMember;
    use crate::broker::MemberKind;
    use crate::broker::MemberStatus;
    use crate::broker::MessageStatus;
    use crate::output::text::*;

    mod delivery_text {
        use crate::broker::runtime::DeliveryMethod;
        use crate::broker::runtime::DeliveryProjection;
        use crate::broker::runtime::DeliveryStatus;
        use crate::output::text::tests::*;

        fn projection(
            status: DeliveryStatus,
            method: Option<DeliveryMethod>,
            reason: Option<&str>,
        ) -> DeliveryProjection {
            DeliveryProjection {
                status,
                method,
                reason: reason.map(str::to_owned),
            }
        }

        #[test]
        fn delivery_text_submitted_prompt_and_exec_keep_their_success_lines_then_delivery_and_input()
         {
            let profile = projection(
                DeliveryStatus::Submitted,
                Some(DeliveryMethod::Profile),
                None,
            );
            assert_eq!(
                format_member_prompt(7, "do it", "worker", "%2", Some(5), &profile),
                "Sent prompt \"do it\" to member worker (%2).\nDelivery: status=submitted method=profile reason=-\nInput: 5"
            );
            let fallback = projection(
                DeliveryStatus::Submitted,
                Some(DeliveryMethod::Fallback),
                Some("submission_plan_unavailable"),
            );
            assert_eq!(
                format_member_exec(7, "%2", Some(6), &fallback),
                "Dispatched native command to member 7 (%2). Inspect the pane for its result.\nDelivery: status=submitted method=fallback reason=submission_plan_unavailable\nInput: 6"
            );
        }

        #[test]
        fn delivery_text_queued_prompt_and_exec_print_queue_lines_retry_and_the_receipt_pointer() {
            let queued = projection(DeliveryStatus::Queued, None, Some("interaction_open"));
            for (text, first) in [
                (
                    format_member_prompt(7, "do it", "worker", "%2", Some(8), &queued),
                    "Queued prompt for member 7 (%2).",
                ),
                (
                    format_member_exec(7, "%2", Some(8), &queued),
                    "Queued native command for member 7 (%2).",
                ),
            ] {
                assert_eq!(text.lines().next(), Some(first), "{text}");
                assert!(!text.contains("Sent prompt") && !text.contains("Dispatched"));
                assert!(
                    text.contains("\nDelivery: status=queued method=- reason=interaction_open")
                );
                assert!(text.contains("\nInput: 8"), "{text}");
                assert!(
                    text.contains("retry"),
                    "the queued result says the runner will retry: {text}"
                );
                assert!(text.contains("cafleet input show 8 --json"), "{text}");
            }
        }

        #[test]
        fn delivery_text_send_appends_delivery_then_input_only_when_allocated() {
            let message = MessageRecord::fixture(9, 2, 3, "hello");
            let envelope = format_message(&message);
            let submitted = projection(
                DeliveryStatus::Submitted,
                Some(DeliveryMethod::Profile),
                None,
            );
            assert_eq!(
                format_message_sent(&message, Some(4), &submitted),
                format!(
                    "Message sent.\n{envelope}\nDelivery: status=submitted method=profile reason=-\nInput: 4"
                )
            );
            let director = projection(DeliveryStatus::NotApplicable, None, Some("director_stdout"));
            assert_eq!(
                format_message_sent(&message, None, &director),
                format!(
                    "Message sent.\n{envelope}\nDelivery: status=not_applicable method=- reason=director_stdout"
                )
            );
            let queued = projection(DeliveryStatus::Queued, None, Some("earlier_input"));
            let text = format_message_sent(&message, Some(5), &queued);
            assert!(text.starts_with(&format!(
                "Message sent.\n{envelope}\nDelivery: status=queued method=- reason=earlier_input\nInput: 5"
            )));
            assert!(text.contains("retry"), "{text}");
        }
    }

    #[test]
    fn unicode_truncation_changes_only_human_message_bodies() {
        let mut message = MessageRecord {
            message_id: 5,
            from_member_id: 2,
            to_member_id: 3,
            created_at: "created".into(),
            status_timestamp: "acked".into(),
            status: MessageStatus::Completed,
            text: format!("{}🙂a\nnext", "界".repeat(TEXT_BODY_LIMIT - 1)),
        };
        assert_eq!(
            format_message(&message),
            format!(
                "[5 | from:2 | acked]\n{}🙂…",
                "界".repeat(TEXT_BODY_LIMIT - 1)
            )
        );
        assert_eq!(
            crate::output::json::message(&message)["text"],
            message.text.as_str()
        );
        message.text = "界".repeat(TEXT_BODY_LIMIT);
        assert_eq!(
            format_message(&message),
            format!("[5 | from:2 | acked]\n{}", message.text)
        );
        message.text.clear();
        assert_eq!(format_message(&message), "[5 | from:2 | acked]");
        assert_eq!(format_messages(&[]), "No messages found.");
        assert_eq!(
            format_messages(&[message.clone(), message]),
            "[5 | from:2 | acked]\n\n[5 | from:2 | acked]"
        );
    }

    #[test]
    fn roster_distinguishes_absent_pending_and_concrete_placement() {
        let member = MemberRecord {
            member_id: 7,
            fleet_id: 1,
            name: "worker".into(),
            description: "assignment".into(),
            status: MemberStatus::Active,
            kind: MemberKind::Member,
            registered_at: "registered".into(),
            placement: Placement {
                backend: "tmux".into(),
                mux_session: "s".into(),
                mux_window_id: "@1".into(),
                mux_pane_id: None,
                coding_agent: "codex".into(),
                created_at: "created".into(),
                mux_server: "/tmp/tmux-1000/default".into(),
            },
            runtime: None,
        };
        let mut row = MemberActivity {
            member,
            last_sent: None,
            last_recv: None,
            last_ack: None,
            idle: None,
        };
        assert_eq!(format_member_list(&[]), "0 members.");
        row.idle = Some(60);
        assert!(format_member_list(&[row.clone()]).contains("codex     (pending)  1m"));
        row.member.placement.mux_pane_id = Some("%9".into());
        assert!(format_member_list(&[row.clone()]).contains("codex     %9"));
        assert_eq!(format_member_detail(&row.member), "7 worker active");
        for (idle, expected) in [
            (None, "-"),
            (Some(0), "0s"),
            (Some(59), "59s"),
            (Some(60), "1m"),
            (Some(3599), "59m"),
            (Some(3600), "1h"),
        ] {
            assert_eq!(format_idle(idle), expected);
        }
    }

    #[test]
    fn fleet_deletion_reports_every_ordinary_member_outcome_in_text_and_json() {
        let deleted = DeletedFleet {
            fleet_id: 4,
            deregistered_count: 3,
            members: vec![
                DeletedMember {
                    member_id: 6,
                    pane_status: "%7 (killed)".into(),
                },
                DeletedMember {
                    member_id: 8,
                    pane_status: "(pending — no pane)".into(),
                },
            ],
        };
        assert_eq!(
            format_fleet_delete(&deleted),
            "Deleted fleet 4. Deregistered 3 members.\n  member 6: %7 (killed)\n  member 8: (pending — no pane)"
        );
        assert_eq!(
            crate::output::json::deleted_fleet(&deleted).to_string(),
            r#"{"fleet_id":4,"deregistered_count":3,"members":[{"member_id":6,"pane_status":"%7 (killed)"},{"member_id":8,"pane_status":"(pending — no pane)"}]}"#
        );
        let empty = DeletedFleet {
            members: Vec::new(),
            ..deleted
        };
        assert_eq!(
            format_fleet_delete(&empty),
            "Deleted fleet 4. Deregistered 3 members."
        );
    }
    use crate::runtime::control::test_support::inspection;

    #[test]
    fn permission_inspection_text_names_the_token_sequence_and_answer_rule() {
        let token = "a".repeat(64);
        assert_eq!(
            crate::output::text::format_permission_inspection(&inspection()),
            format!(
                "Permission: interaction=12 event=11 member=3 run=7 epoch=2 visibility=visible answers=allow,deny\nRequest: token={token} output_seq=5 rule=opencode-shell-allow-enter\nMissing evidence: arguments,cwd"
            )
        );
    }

    #[test]
    fn permission_inspection_text_marks_an_unsupported_request_with_dashes_and_its_reason() {
        let mut unsupported = inspection();
        unsupported.answer_rule_id = None;
        unsupported.request_token = None;
        unsupported.supported_decisions = Vec::new();
        unsupported.capability_reason = Some("native profile unavailable".into());
        unsupported.missing_evidence = Vec::new();
        assert_eq!(
            crate::output::text::format_permission_inspection(&unsupported),
            "Permission: interaction=12 event=11 member=3 run=7 epoch=2 visibility=visible answers=-\nRequest: token=- output_seq=5 rule=-\nCapability: native profile unavailable"
        );
    }

    #[test]
    fn permission_inspection_text_lists_a_questions_supported_choice_ids() {
        let token = "a".repeat(64);
        assert_eq!(
            crate::output::text::format_permission_inspection(
                &crate::runtime::control::test_support::question_inspection()
            ),
            format!(
                "Permission: interaction=12 event=11 member=3 run=7 epoch=2 visibility=visible answers=not-now,dont-show-again\nRequest: token={token} output_seq=5 rule=-"
            )
        );
    }

    #[test]
    fn permission_response_text_names_the_decision_or_the_choice_with_a_dash_for_the_other() {
        use crate::coding_agent::catalog::AnswerKey;
        use crate::runtime::PermissionResponseResult;
        use crate::runtime::SubmissionResult;
        let result = |key: AnswerKey| PermissionResponseResult {
            submission: SubmissionResult::held("another programmatic input is pending"),
            member_id: 3,
            run_id: 7,
            owner_epoch: 2,
            interaction_id: 12,
            event_id: 11,
            key,
            decision_source: crate::broker::runtime::DecisionSource::DirectorAuto,
        };
        let decision = format_permission_response_result(&result(AnswerKey::Decision(
            crate::broker::runtime::PermissionDecision::Deny,
        )));
        assert!(
            decision.starts_with("Permission response: interaction=12 run=7 decision=deny choice=- source=director-auto status=held"),
            "{decision}"
        );
        let choice =
            format_permission_response_result(&result(AnswerKey::Choice("not-now".into())));
        assert!(
            choice.starts_with("Permission response: interaction=12 run=7 decision=- choice=not-now source=director-auto status=held"),
            "{choice}"
        );
    }

    #[test]
    fn member_press_text_acknowledges_the_canonical_keys_without_effect_counts() {
        let keys = crate::multiplexer::PressKeys::parse(&["cTrL+A".into(), ";".into()]).unwrap();
        let value = crate::runtime::KeyAcknowledgment {
            member_id: 42,
            pane_id: "%5".into(),
            keys: keys.canonical(),
        };
        assert_eq!(
            crate::output::format_member_press(&value),
            "Acknowledged keys [\"Ctrl+a\",\";\"] for member 42 (%5).",
        );
    }
}
