//! Step 3 contract tests: `cafleet::spawn_prompt` — the spawn-placeholder
//! mini-formatter used by `member create` (SPEC §6.3 *Spawn-prompt
//! resolution*): Python brace grammar with `{{` / `}}` escapes, the four
//! placeholders, and the unknown-vs-malformed usage-error taxonomy (exit 2).
//!
//! Expected public API:
//! - `substitute_spawn_placeholders(body: &str, fleet_id: i64, member_id: i64,
//!    director_member_id: i64, coding_agent: &str)
//!    -> Result<String, CafleetError>`
//!
//! The unknown-placeholder message is pinned byte-exact; the malformed-brace
//! message pins its frame (`Malformed custom prompt: <detail>. Double literal
//! braces ({{, }}) to keep them as text.`) with an implementation-worded
//! `<detail>` (SPEC §1 relaxation).

use cafleet::error::CafleetError;
use cafleet::spawn_prompt::substitute_spawn_placeholders;

fn assert_malformed_usage_error(result: Result<String, CafleetError>) {
    let err = result.expect_err("a malformed brace expression must error");
    assert!(matches!(err, CafleetError::Usage(_)));
    assert_eq!(err.exit_code(), 2);
    let msg = err.message().to_string();
    assert!(msg.starts_with("Malformed custom prompt: "), "got: {msg}");
    assert!(
        msg.ends_with("Double literal braces ({{, }}) to keep them as text."),
        "got: {msg}"
    );
}

#[test]
fn substitutes_all_four_placeholders() {
    let body = "FLEET ID: {fleet_id}\nYOUR MEMBER ID: {member_id}\n\
                DIRECTOR MEMBER ID: {director_member_id}\nCODING AGENT: {coding_agent}";
    let out = substitute_spawn_placeholders(body, 3, 14, 11, "claude").unwrap();
    assert_eq!(
        out,
        "FLEET ID: 3\nYOUR MEMBER ID: 14\nDIRECTOR MEMBER ID: 11\nCODING AGENT: claude"
    );
}

#[test]
fn substitutes_repeated_placeholders() {
    let out = substitute_spawn_placeholders("{fleet_id}-{fleet_id}", 7, 1, 2, "codex").unwrap();
    assert_eq!(out, "7-7");
}

#[test]
fn body_without_placeholders_is_returned_verbatim() {
    let out = substitute_spawn_placeholders("plain prompt, no braces", 1, 2, 3, "claude").unwrap();
    assert_eq!(out, "plain prompt, no braces");
}

#[test]
fn doubled_braces_render_literal_braces() {
    let out =
        substitute_spawn_placeholders("keep {{fleet_id}} literal", 1, 2, 3, "claude").unwrap();
    assert_eq!(out, "keep {fleet_id} literal");
    let out = substitute_spawn_placeholders("a{{b}}c", 1, 2, 3, "claude").unwrap();
    assert_eq!(out, "a{b}c");
}

#[test]
fn doubled_braces_compose_with_substitution() {
    let out = substitute_spawn_placeholders("{{x}} and {fleet_id}", 42, 2, 3, "opencode").unwrap();
    assert_eq!(out, "{x} and 42");
}

#[test]
fn unknown_placeholder_is_a_usage_error_with_the_pinned_message() {
    let err = substitute_spawn_placeholders("hello {foo}", 1, 2, 3, "claude")
        .expect_err("an unknown placeholder must error");
    assert!(matches!(err, CafleetError::Usage(_)));
    assert_eq!(err.exit_code(), 2);
    assert_eq!(
        err.message(),
        "Unknown placeholder 'foo' in custom prompt. \
         Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, \
         {coding_agent}. Double literal braces ({{, }}) to keep them as text."
    );
}

#[test]
fn stray_open_brace_is_a_malformed_usage_error() {
    assert_malformed_usage_error(substitute_spawn_placeholders("oops {", 1, 2, 3, "claude"));
}

#[test]
fn stray_close_brace_is_a_malformed_usage_error() {
    assert_malformed_usage_error(substitute_spawn_placeholders("oops }", 1, 2, 3, "claude"));
}

#[test]
fn unclosed_placeholder_is_a_malformed_usage_error() {
    assert_malformed_usage_error(substitute_spawn_placeholders(
        "start {fleet_id and never close",
        1,
        2,
        3,
        "claude",
    ));
}
