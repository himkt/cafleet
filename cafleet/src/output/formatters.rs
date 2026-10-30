use crate::broker::records::{
    CreatedFleet, DeletedFleet, FleetRecord, FleetSummary, MemberActivity, MemberRecord,
    MessageRecord, Placement, RegisteredMember,
};

use super::truncate_text;
use crate::broker::runtime::{DeliveryProjection, DeliveryStatus};

pub(crate) fn format_message(message: &MessageRecord, limit: usize) -> String {
    let header = format!(
        "[{} | from:{} | {}]",
        message.message_id, message.from_member_id, message.status_timestamp
    );
    let body = truncate_text(&message.text, limit);
    if body.is_empty() {
        header
    } else {
        format!("{header}\n{body}")
    }
}

pub(crate) fn format_messages(messages: &[MessageRecord], limit: usize) -> String {
    if messages.is_empty() {
        return "No messages found.".into();
    }
    messages
        .iter()
        .map(|message| format_message(message, limit))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn format_message_sent(
    message: &MessageRecord,
    limit: usize,
    input_id: Option<i64>,
    delivery: &DeliveryProjection,
) -> String {
    with_delivery(
        format!("Message sent.\n{}", format_message(message, limit)),
        input_id,
        delivery,
    )
}

pub(crate) fn format_message_acknowledged(message: &MessageRecord, limit: usize) -> String {
    format!("Message acknowledged.\n{}", format_message(message, limit))
}

pub(crate) fn format_message_wait(
    member_id: i64,
    messages: &[MessageRecord],
    limit: usize,
) -> String {
    if messages.is_empty() {
        format!("Timed out waiting for member {member_id}.")
    } else {
        format!(
            "Messages available for member {member_id}.\n{}",
            format_messages(messages, limit)
        )
    }
}

pub(crate) fn format_fleet_list(fleets: &[FleetSummary]) -> String {
    if fleets.is_empty() {
        return "No fleets found.".into();
    }
    let mut lines = vec!["FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT".into()];
    for fleet in fleets {
        let director = fleet.director_member_id.map(|id| id.to_string());
        lines.push(format!(
            "{} {} {} {} {}",
            fleet.fleet_id,
            director.as_deref().unwrap_or("-"),
            fleet.name.as_deref().unwrap_or("-"),
            fleet.member_count,
            fleet.created_at
        ));
    }
    lines.join("\n")
}

pub(crate) fn format_fleet_show(fleet: &FleetRecord) -> String {
    let mut text = format!(
        "fleet_id: {}\nname: {}\ncreated_at: {}",
        fleet.fleet_id,
        fleet.name.as_deref().unwrap_or("-"),
        fleet.created_at
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
        text.push_str(&super::format_runtime_snapshot(runtime));
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
        let (backend, pane) = member.placement.as_ref().map_or(("-", "-"), |placement| {
            (
                placement.coding_agent.as_str(),
                placement.mux_pane_id.as_deref().unwrap_or("(pending)"),
            )
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::records::{DeletedMember, MemberKind, MemberStatus, MessageStatus};

    mod delivery_text {
        use super::*;
        use crate::broker::runtime::{DeliveryMethod, DeliveryProjection, DeliveryStatus};

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
                Some("composer_unverified"),
            );
            assert_eq!(
                format_member_exec(7, "%2", Some(6), &fallback),
                "Dispatched native command to member 7 (%2). Inspect the pane for its result.\nDelivery: status=submitted method=fallback reason=composer_unverified\nInput: 6"
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
            let envelope = format_message(&message, 2000);
            let submitted = projection(
                DeliveryStatus::Submitted,
                Some(DeliveryMethod::Profile),
                None,
            );
            assert_eq!(
                format_message_sent(&message, 2000, Some(4), &submitted),
                format!(
                    "Message sent.\n{envelope}\nDelivery: status=submitted method=profile reason=-\nInput: 4"
                )
            );
            let director = projection(DeliveryStatus::NotApplicable, None, Some("director_stdout"));
            assert_eq!(
                format_message_sent(&message, 2000, None, &director),
                format!(
                    "Message sent.\n{envelope}\nDelivery: status=not_applicable method=- reason=director_stdout"
                )
            );
            let queued = projection(DeliveryStatus::Queued, None, Some("earlier_input"));
            let text = format_message_sent(&message, 2000, Some(5), &queued);
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
            text: "界🙂a\nnext".into(),
        };
        assert_eq!(format_message(&message, 2), "[5 | from:2 | acked]\n界🙂…");
        assert_eq!(
            crate::presentation::message(&message)["text"],
            "界🙂a\nnext"
        );
        assert_eq!(format_message(&message, 0), "[5 | from:2 | acked]\n…");
        message.text.clear();
        assert_eq!(format_message(&message, 0), "[5 | from:2 | acked]");
        assert_eq!(format_messages(&[], 0), "No messages found.");
        assert_eq!(
            format_messages(&[message.clone(), message], 0),
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
            placement: None,
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
        assert!(format_member_list(&[row.clone()]).contains("member    -         -        -"));
        row.member.placement = Some(Placement {
            backend: "tmux".into(),
            mux_session: "s".into(),
            mux_window_id: "@1".into(),
            mux_pane_id: None,
            coding_agent: "codex".into(),
            created_at: "created".into(),
        });
        row.idle = Some(60);
        assert!(format_member_list(&[row.clone()]).contains("codex     (pending)  1m"));
        row.member.placement.as_mut().unwrap().mux_pane_id = Some("%9".into());
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
            crate::presentation::deleted_fleet(&deleted).to_string(),
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
}
