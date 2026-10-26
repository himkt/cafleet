use crate::broker::records::{
    CreatedFleet, MemberActivity, MemberRecord, MessageRecord, Placement, RegisteredMember,
};

use super::truncate_text;

pub fn format_message(message: &MessageRecord, limit: usize) -> String {
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

pub fn format_messages(messages: &[MessageRecord], limit: usize) -> String {
    if messages.is_empty() {
        return "No messages found.".into();
    }
    messages
        .iter()
        .map(|message| format_message(message, limit))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn format_member_detail(member: &MemberRecord) -> String {
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

pub fn format_fleet_create(fleet: &CreatedFleet) -> String {
    format!("{} director={}", fleet.fleet_id, fleet.director.member_id)
}

pub fn format_member(member: &RegisteredMember, placement: &Placement) -> String {
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

pub fn format_member_list(members: &[MemberActivity]) -> String {
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
    use crate::broker::records::{MemberKind, MemberStatus, MessageStatus};

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
}
