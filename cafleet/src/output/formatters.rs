use serde_json::Value;

use crate::output::render::{format_json, is_truthy, render_message, truncate_text};

/// Render a scalar JSON value the way it appears in text output: strings
/// bare (no quotes), everything else in its JSON form.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Compact: `[<id> | from:<from> | <ts>[ | kind:<kind>][ | origin:<id>]]` +
/// the body line (omitted when empty). Verbose: the labeled block.
pub fn format_message(message: &Value, full: bool) -> String {
    let message = match message.get("message") {
        Some(inner) if inner.is_object() => inner,
        _ => message,
    };
    if !full {
        let rendered = render_message(message, false);
        let mut line1 = format!(
            "[{} | from:{} | {}",
            scalar(&rendered["id"]),
            scalar(&rendered["from"]),
            scalar(&rendered["ts"]),
        );
        if let Some(kind) = rendered.get("kind") {
            line1.push_str(&format!(" | kind:{}", scalar(kind)));
        }
        if let Some(origin) = rendered.get("origin") {
            line1.push_str(&format!(" | origin:{}", scalar(origin)));
        }
        line1.push(']');
        let body = rendered.get("text").map(scalar).unwrap_or_default();
        if body.is_empty() {
            return line1;
        }
        return format!("{line1}\n{body}");
    }
    let mut lines = vec![
        format!("  id:    {}", scalar(&message["message_id"])),
        format!("  state: {}", scalar(&message["status_state"])),
        format!("  from:  {}", scalar(&message["from_member_id"])),
    ];
    if !message["to_member_id"].is_null() {
        lines.push(format!("  to:    {}", scalar(&message["to_member_id"])));
    }
    lines.push(format!("  type:  {}", scalar(&message["type"])));
    if is_truthy(&message["text"]) {
        lines.push(format!("  text:  {}", scalar(&message["text"])));
    }
    lines.join("\n")
}

/// Join formatted items with a single blank line between them; items are not
/// numbered (members reference messages by `message_id`, not list index).
pub fn format_indexed_list(
    items: &[Value],
    formatter: impl Fn(&Value) -> String,
    empty_msg: &str,
) -> String {
    if items.is_empty() {
        return empty_msg.to_string();
    }
    items.iter().map(formatter).collect::<Vec<_>>().join("\n\n")
}

/// Compact: `<id> <name> <status>`. Verbose: the labeled block with the
/// 60-codepoint description truncation and the placement sub-block.
pub fn format_member_detail(member: &Value, full: bool) -> String {
    if !full {
        return format!(
            "{} {} {}",
            scalar(&member["member_id"]),
            scalar(&member["name"]),
            scalar(&member["status"]),
        );
    }
    let description = truncate_text(member["description"].as_str(), false, 60)
        .expect("member dict carries 'description'");
    let skills = &member["skills"];
    let skills_cell = if is_truthy(skills) {
        format_json(skills)
    } else {
        "-".to_string()
    };
    let mut lines = vec![
        format!("  member_id:   {}", scalar(&member["member_id"])),
        format!("  name:        {}", scalar(&member["name"])),
        format!("  description: {description}"),
        format!("  status:      {}", scalar(&member["status"])),
        format!("  kind:        {}", scalar(&member["kind"])),
        format!("  skills:      {skills_cell}"),
    ];
    let placement = &member["placement"];
    if placement.is_null() {
        lines.push("  placement:   none".to_string());
    } else {
        lines.push("  placement:".to_string());
        lines.push(format!(
            "    backend:    {}",
            scalar(&placement["coding_agent"])
        ));
        lines.push(format!(
            "    session:    {}",
            scalar(&placement["mux_session"])
        ));
        lines.push(format!(
            "    window_id:  {}",
            scalar(&placement["mux_window_id"])
        ));
        lines.push(format!(
            "    pane_id:    {}",
            dash_if_null(&placement["mux_pane_id"])
        ));
        lines.push(format!(
            "    created_at: {}",
            scalar(&placement["created_at"])
        ));
    }
    lines.join("\n")
}

fn dash_if_null(value: &Value) -> String {
    if value.is_null() {
        "-".to_string()
    } else {
        scalar(value)
    }
}

/// Compact: `<fleet_id> director=<id>`. Verbose: the 6-line block.
pub fn format_fleet_create(data: &Value, full: bool) -> String {
    let director = &data["director"];
    if !full {
        return format!(
            "{} director={}",
            scalar(&data["fleet_id"]),
            scalar(&director["member_id"]),
        );
    }
    let placement = &director["placement"];
    let name = if data["name"].is_null() {
        String::new()
    } else {
        scalar(&data["name"])
    };
    [
        scalar(&data["fleet_id"]),
        scalar(&director["member_id"]),
        format!("name:             {name}"),
        format!("created_at:       {}", scalar(&data["created_at"])),
        format!("director_name:    {}", scalar(&director["name"])),
        format!(
            "pane:             {}:{}:{}",
            scalar(&placement["mux_session"]),
            scalar(&placement["mux_window_id"]),
            scalar(&placement["mux_pane_id"]),
        ),
    ]
    .join("\n")
}

/// Compact: `<id> <name> backend=<coding_agent> pane=<pane_id>`. Verbose:
/// the 6-line `Member registered and spawned.` block.
pub fn format_member(data: &Value, full: bool) -> String {
    let placement = &data["placement"];
    if !full {
        let pane = if placement["mux_pane_id"].is_null() {
            "(pending)".to_string()
        } else {
            scalar(&placement["mux_pane_id"])
        };
        return format!(
            "{} {} backend={} pane={pane}",
            scalar(&data["member_id"]),
            scalar(&data["name"]),
            scalar(&placement["coding_agent"]),
        );
    }
    [
        "Member registered and spawned.".to_string(),
        format!("  member_id: {}", scalar(&data["member_id"])),
        format!("  name:      {}", scalar(&data["name"])),
        format!("  backend:   {}", scalar(&placement["coding_agent"])),
        format!("  pane_id:   {}", dash_if_null(&placement["mux_pane_id"])),
        format!("  window_id: {}", scalar(&placement["mux_window_id"])),
    ]
    .join("\n")
}

fn format_idle(value: &Value) -> String {
    match value.as_i64() {
        None => "-".to_string(),
        Some(seconds) if seconds < 60 => format!("{seconds}s"),
        Some(seconds) if seconds < 3600 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{}h", seconds / 3600),
    }
}

/// The `member list` table: one row per active registry entry, `-` cells for
/// a placementless row, `(pending)` for a placed row with no pane yet.
pub fn format_member_list(members: &[Value]) -> String {
    if members.is_empty() {
        return "0 members.".to_string();
    }
    let plural = if members.len() > 1 { "s" } else { "" };
    let mut lines = vec![
        format!("{} member{plural}:", members.len()),
        "  member_id  name           kind      backend   pane_id  idle".to_string(),
        "  ---------  -------------  --------  --------  -------  ----".to_string(),
    ];
    for member in members {
        let placement = &member["placement"];
        let (backend, pane) = if placement.is_null() {
            ("-".to_string(), "-".to_string())
        } else {
            let pane = if placement["mux_pane_id"].is_null() {
                "(pending)".to_string()
            } else {
                scalar(&placement["mux_pane_id"])
            };
            (scalar(&placement["coding_agent"]), pane)
        };
        lines.push(format!(
            "  {:<9}  {:<13}  {:<8}  {backend:<8}  {pane:<7}  {}",
            scalar(&member["member_id"]),
            scalar(&member["name"]),
            scalar(&member["kind"]),
            format_idle(&member["idle"]),
        ));
    }
    lines.join("\n")
}
