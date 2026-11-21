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
    format!("{header}\n{body}")
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

pub(crate) fn format_message_sent(message: &MessageRecord) -> String {
    format!("Message sent.\n{}", format_message(message))
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

/// The placement's window name, or `(pending)` while creation has not published it.
fn window_label(placement: &Placement) -> &str {
    placement.mux_window_name.as_deref().unwrap_or(PENDING)
}

const PENDING: &str = "(pending)";

pub(crate) fn format_member_detail(member: &MemberRecord) -> String {
    let mut text = format!(
        "{} {} {} window={}",
        member.member_id,
        member.name,
        member.status.as_str(),
        window_label(&member.placement)
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
    let pane = placement.mux_pane_id.as_deref().unwrap_or(PENDING);
    format!(
        "{} {} backend={} pane={pane} window={}",
        member.member_id,
        member.name,
        placement.coding_agent,
        window_label(placement)
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
        "  member_id  name           kind      backend   window         pane_id  idle".into(),
    ];
    for row in members {
        let member = &row.member;
        let backend = member.placement.coding_agent.as_str();
        let window = window_label(&member.placement);
        let pane = member.placement.mux_pane_id.as_deref().unwrap_or(PENDING);
        lines.push(format!(
            "  {:<9}  {:<13}  {:<8}  {backend:<8}  {window:<13}  {pane:<7}  {}",
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
        "Runtime: run={} mode={} status={} coverage={}",
        run.run_id,
        run.resolved_mode,
        run.status.as_str(),
        run.coverage.as_str(),
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
    format!(
        "Input: {} run={} kind={} state={} disposition={}",
        input.input_id,
        input.run_id,
        input.kind.as_str(),
        input.state.as_str(),
        input
            .disposition
            .map_or("-", |disposition| disposition.as_str()),
    )
}

pub(crate) fn format_permission_response_result(
    row: &crate::runtime::PermissionResponseResult,
) -> String {
    let mut text = format!(
        "Permission response: interaction={} run={} decision={} choice={} status={} stage={} output_seq={}",
        row.interaction_id,
        row.run_id,
        row.key.decision().map_or("-", |decision| decision.as_str()),
        row.key.choice().unwrap_or("-"),
        row.submission.status.as_str(),
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
        fn message_sent_text_is_the_confirmation_line_then_the_envelope() {
            let message = MessageRecord::fixture(9, 2, 3, "hello");
            assert_eq!(
                format_message_sent(&message),
                format!("Message sent.\n{}", format_message(&message))
            );
        }
    }

    mod quoted_text {
        use crate::broker::runtime::DeliveryMethod;
        use crate::broker::runtime::DeliveryProjection;
        use crate::broker::runtime::DeliveryStatus;
        use crate::output::text::tests::*;

        const CASES: &[(&str, &str, &str)] = &[
            ("double quote", "\"", r#""\"""#),
            ("backslash", "\\", r#""\\""#),
            ("tab", "\t", r#""\t""#),
            ("line feed", "\n", r#""\n""#),
            ("carriage return", "\r", r#""\r""#),
            ("nul", "\0", r#""\0""#),
            ("Cc bell", "\u{7}", r#""\u{7}""#),
            ("Cc escape", "\u{1b}", r#""\u{1b}""#),
            ("Cc delete", "\u{7f}", r#""\u{7f}""#),
            ("Cc next line", "\u{85}", r#""\u{85}""#),
            ("Cf soft hyphen", "\u{ad}", r#""\u{ad}""#),
            ("Cf zero width space", "\u{200b}", r#""\u{200b}""#),
            ("Cf byte order mark", "\u{feff}", r#""\u{feff}""#),
            ("Co private use", "\u{e000}", r#""\u{e000}""#),
            ("Cn noncharacter", "\u{ffff}", r#""\u{ffff}""#),
            ("Cn plane 16 noncharacter", "\u{10ffff}", r#""\u{10ffff}""#),
            ("Zl line separator", "\u{2028}", r#""\u{2028}""#),
            ("Zp paragraph separator", "\u{2029}", r#""\u{2029}""#),
            ("Zs no-break space", "\u{a0}", r#""\u{a0}""#),
            ("Zs ideographic space", "\u{3000}", r#""\u{3000}""#),
            ("Grapheme_Extend alone", "\u{301}", r#""\u{301}""#),
            ("Grapheme_Extend after a base", "e\u{301}", r#""e\u{301}""#),
            (
                "Grapheme_Extend enclosing mark",
                "\u{20dd}",
                r#""\u{20dd}""#,
            ),
            (
                "Grapheme_Extend zero width non-joiner",
                "\u{200c}",
                r#""\u{200c}""#,
            ),
            ("space", " ", r#"" ""#),
            ("single quote", "'", r#""'""#),
            ("solidus", "/", r#""/""#),
            ("latin small e with acute", "\u{e9}", "\"\u{e9}\""),
            ("cjk ideographs", "日本", "\"日本\""),
            ("euro sign", "€", "\"€\""),
            ("emoji", "\u{1f4a9}", "\"\u{1f4a9}\""),
        ];

        #[test]
        fn member_prompt_text_quotes_each_scalar_class_per_the_quoted_text_definition() {
            let submitted = DeliveryProjection {
                status: DeliveryStatus::Submitted,
                method: Some(DeliveryMethod::Profile),
                reason: None,
            };
            for (name, scalar, quoted) in CASES {
                let text = format_member_prompt(7, scalar, "worker", "%2", None, &submitted);
                assert_eq!(
                    text.split_once('\n').map(|(first, _)| first),
                    Some(format!("Sent prompt {quoted} to member worker (%2).").as_str()),
                    "{name}"
                );
            }
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
        assert_eq!(format_messages(&[]), "No messages found.");
        assert_eq!(
            format_messages(&[message.clone(), message.clone()]),
            format!("{0}\n\n{0}", format_message(&message))
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
                mux_window_id: None,
                mux_window_name: None,
                window_ordinal: 0,
                column_side: crate::multiplexer::layout::ColumnSide::Right,
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
            last_consumed: None,
            idle: None,
        };
        assert_eq!(format_member_list(&[]), "0 members.");
        row.idle = Some(60);
        assert!(
            format_member_list(&[row.clone()]).contains("codex     (pending)      (pending)  1m")
        );
        assert_eq!(
            format_member_detail(&row.member),
            "7 worker active window=(pending)"
        );
        row.member.placement.mux_pane_id = Some("%9".into());
        row.member.placement.mux_window_id = Some("@1".into());
        row.member.placement.mux_window_name = Some("main".into());
        assert!(format_member_list(&[row.clone()]).contains("codex     main           %9"));
        assert_eq!(
            format_member_detail(&row.member),
            "7 worker active window=main"
        );
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

    mod window_text {
        use crate::multiplexer::layout::ColumnSide;
        use crate::output::text::tests::*;

        fn placement(window: Option<(&str, &str)>, pane: Option<&str>) -> Placement {
            Placement {
                backend: "tmux".into(),
                mux_session: "main".into(),
                mux_window_id: window.map(|(id, _)| id.into()),
                mux_window_name: window.map(|(_, name)| name.into()),
                window_ordinal: 0,
                column_side: ColumnSide::Right,
                mux_pane_id: pane.map(str::to_owned),
                coding_agent: "claude".into(),
                created_at: "created".into(),
                mux_server: "/tmp/tmux-1000/default".into(),
            }
        }

        fn row(
            member_id: i64,
            name: &str,
            kind: MemberKind,
            placement: Placement,
            idle: i64,
        ) -> MemberActivity {
            MemberActivity {
                member: MemberRecord {
                    member_id,
                    fleet_id: 3,
                    name: name.into(),
                    description: "assignment".into(),
                    status: MemberStatus::Active,
                    kind,
                    registered_at: "registered".into(),
                    placement,
                    runtime: None,
                },
                last_sent: None,
                last_recv: None,
                last_consumed: None,
                idle: Some(idle),
            }
        }

        #[test]
        fn member_list_shows_each_members_window_between_backend_and_pane() {
            let rows = [
                row(
                    8,
                    "director",
                    MemberKind::Director,
                    placement(Some(("@1", "main")), Some("%3")),
                    12,
                ),
                row(
                    9,
                    "drafter",
                    MemberKind::Member,
                    placement(Some(("@6", "cafleet-3-1")), Some("%12")),
                    5,
                ),
            ];
            assert_eq!(
                format_member_list(&rows),
                [
                    "2 members:",
                    "  member_id  name           kind      backend   window         pane_id  idle",
                    "  8          director       director  claude    main           %3       12s",
                    "  9          drafter        member    claude    cafleet-3-1    %12      5s",
                ]
                .join(
                    "
"
                )
            );
        }

        #[test]
        fn member_list_shows_a_pending_window_and_pane() {
            let rows = [row(
                10,
                "tester",
                MemberKind::Member,
                placement(None, None),
                0,
            )];
            assert_eq!(
                format_member_list(&rows).lines().nth(2).unwrap(),
                "  10         tester         member    claude    (pending)      (pending)  0s"
            );
        }

        #[test]
        fn member_create_text_appends_the_window_after_the_pane() {
            let registered = RegisteredMember {
                member_id: 9,
                name: "drafter".into(),
                registered_at: "registered".into(),
            };
            assert_eq!(
                format_member(
                    &registered,
                    &placement(Some(("@6", "cafleet-3-1")), Some("%12"))
                ),
                "9 drafter backend=claude pane=%12 window=cafleet-3-1"
            );
            assert_eq!(
                format_member(&registered, &placement(None, None)),
                "9 drafter backend=claude pane=(pending) window=(pending)"
            );
        }

        #[test]
        fn member_show_text_appends_the_window_after_the_status() {
            let concrete = row(
                9,
                "drafter",
                MemberKind::Member,
                placement(Some(("@6", "cafleet-3-1")), Some("%12")),
                5,
            );
            assert_eq!(
                format_member_detail(&concrete.member),
                "9 drafter active window=cafleet-3-1"
            );
            let pending = row(10, "tester", MemberKind::Member, placement(None, None), 0);
            assert_eq!(
                format_member_detail(&pending.member),
                "10 tester active window=(pending)"
            );
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
