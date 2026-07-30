use crate::error::CafleetError;

const BRACE_HINT: &str = "Double literal braces ({{, }}) to keep them as text.";

/// The spawn-placeholder mini-formatter used by `member create` (SPEC §6.3):
/// Python brace grammar — `{{` / `}}` escapes, the four placeholders, and the
/// unknown-vs-malformed usage-error taxonomy (both exit 2).
pub fn substitute_spawn_placeholders(
    body: &str,
    fleet_id: i64,
    member_id: i64,
    director_member_id: i64,
    coding_agent: &str,
) -> Result<String, CafleetError> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    continue;
                }
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') => {
                            return Err(malformed("unexpected '{' in placeholder name"));
                        }
                        Some(k) => key.push(k),
                        None => return Err(malformed("expected '}' before end of string")),
                    }
                }
                match key.as_str() {
                    "fleet_id" => out.push_str(&fleet_id.to_string()),
                    "member_id" => out.push_str(&member_id.to_string()),
                    "director_member_id" => out.push_str(&director_member_id.to_string()),
                    "coding_agent" => out.push_str(coding_agent),
                    _ => {
                        return Err(CafleetError::Usage(format!(
                            "Unknown placeholder '{key}' in custom prompt. Supported \
                             placeholders: {{fleet_id}}, {{member_id}}, \
                             {{director_member_id}}, {{coding_agent}}. {BRACE_HINT}"
                        )));
                    }
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push('}');
                } else {
                    return Err(malformed("single '}' encountered outside a placeholder"));
                }
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

fn malformed(detail: &str) -> CafleetError {
    CafleetError::Usage(format!("Malformed custom prompt: {detail}. {BRACE_HINT}"))
}
