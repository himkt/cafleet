//! Multiplexer — the pane-hosting backends (SPEC §6.5): the shared context /
//! error types, the injectable `CommandRunner` seam, the `resolve_multiplexer`
//! precedence, the shared wake-payload builder + unified sanitization map, and
//! the tmux / herdr backends. The colocated tests pin the contract; the
//! expected API is catalogued in [`test_support`].

pub mod herdr;
#[cfg(test)]
pub mod test_support;
pub mod tmux;

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::multiplexer::{build_wake_payload, resolve_multiplexer_name, sanitize_wake_field};

    fn due(
        member_id: i64,
        name: &str,
        is_director: bool,
        coding_agent: &str,
        reasons: &[&str],
    ) -> Value {
        json!({
            "member_id": member_id,
            "name": name,
            "is_director": is_director,
            "coding_agent": coding_agent,
            "wake_reasons": reasons,
        })
    }

    fn director(member_id: i64, coding_agent: &str) -> Value {
        json!({"member_id": member_id, "coding_agent": coding_agent})
    }

    mod sanitize_wake_field_tests {
        use super::*;

        #[test]
        fn crlf_collapses_to_a_single_return_symbol() {
            assert_eq!(sanitize_wake_field("a\r\nb"), "a⏎b");
        }

        #[test]
        fn each_line_break_and_tab_becomes_the_return_symbol() {
            assert_eq!(sanitize_wake_field("a\nb\rc\td"), "a⏎b⏎c⏎d");
            assert_eq!(sanitize_wake_field("x\r\n\r"), "x⏎⏎");
        }

        #[test]
        fn backtick_becomes_the_modifier_grave() {
            assert_eq!(sanitize_wake_field("run `ls` now"), "run ˋlsˋ now");
        }

        #[test]
        fn command_substitution_open_is_defused() {
            assert_eq!(sanitize_wake_field("$(rm -rf)"), "$﹙rm -rf)");
            assert_eq!(sanitize_wake_field("cost $5 (ok)"), "cost $5 (ok)");
        }

        #[test]
        fn pipe_becomes_the_box_drawing_bar() {
            assert_eq!(sanitize_wake_field("a|b"), "a│b");
        }

        #[test]
        fn plain_names_pass_through() {
            assert_eq!(sanitize_wake_field("worker-1"), "worker-1");
        }
    }

    mod build_wake_payload_tests {
        use super::*;

        #[test]
        fn single_member_uses_the_singular_noun() {
            let payload = build_wake_payload(
                &[due(4, "worker", false, "codex", &["interval"])],
                &director(1, "claude"),
            )
            .unwrap();
            assert_eq!(
                payload,
                "[monitor] wake: 1 member due — member 4 (worker; coding_agent=codex) \
                 [interval]. Director: 1 (coding_agent=claude). \
                 Follow your monitor role protocol."
            );
        }

        #[test]
        fn multiple_members_join_entries_with_comma_space() {
            let payload = build_wake_payload(
                &[
                    due(1, "Director", true, "claude", &["interval"]),
                    due(4, "worker", false, "codex", &["interval", "unacked"]),
                ],
                &director(1, "claude"),
            )
            .unwrap();
            assert_eq!(
                payload,
                "[monitor] wake: 2 members due — director 1 (Director; coding_agent=claude) \
                 [interval], member 4 (worker; coding_agent=codex) [interval,unacked]. \
                 Director: 1 (coding_agent=claude). Follow your monitor role protocol."
            );
        }

        #[test]
        fn member_names_are_sanitized_in_the_payload() {
            let payload = build_wake_payload(
                &[due(5, "e`vil$(x)|z\nq", false, "claude", &["interval"])],
                &director(1, "claude"),
            )
            .unwrap();
            assert!(
                payload.contains("member 5 (eˋvil$﹙x)│z⏎q; coding_agent=claude)"),
                "got: {payload}"
            );
            assert!(!payload.contains('`'));
            assert!(!payload.contains("$("));
            assert!(!payload.contains('\n'));
        }

        #[test]
        fn an_unregistered_coding_agent_aborts_the_wake() {
            let err = build_wake_payload(
                &[due(4, "worker", false, "python", &["interval"])],
                &director(1, "claude"),
            )
            .expect_err("an unknown member agent must abort");
            assert!(err.to_string().contains("invalid coding_agent"), "got: {err}");

            let err = build_wake_payload(
                &[due(4, "worker", false, "codex", &["interval"])],
                &director(1, "not-an-agent"),
            )
            .expect_err("an unknown Director agent must abort");
            assert!(err.to_string().contains("invalid coding_agent"), "got: {err}");
        }
    }

    mod resolver_tests {
        use super::*;

        fn no_env(_: &str) -> Option<String> {
            None
        }

        #[test]
        fn a_valid_override_wins() {
            assert_eq!(resolve_multiplexer_name(Some("tmux"), no_env).unwrap(), "tmux");
            assert_eq!(
                resolve_multiplexer_name(Some("herdr"), no_env).unwrap(),
                "herdr"
            );
        }

        #[test]
        fn an_unknown_override_is_rejected_with_the_sorted_key_list() {
            let err = resolve_multiplexer_name(Some("bogus"), no_env)
                .expect_err("an unknown override must be rejected");
            assert_eq!(
                err.to_string(),
                "CAFLEET_MULTIPLEXER='bogus' is not a supported multiplexer \
                 (expected one of: herdr, tmux)"
            );
        }

        #[test]
        fn auto_detect_picks_the_single_present_backend() {
            let herdr_only = |name: &str| match name {
                "HERDR_ENV" => Some("1".to_string()),
                _ => None,
            };
            assert_eq!(resolve_multiplexer_name(None, herdr_only).unwrap(), "herdr");

            let tmux_only = |name: &str| match name {
                "TMUX" => Some("/tmp/tmux-1000/default,123,0".to_string()),
                _ => None,
            };
            assert_eq!(resolve_multiplexer_name(None, tmux_only).unwrap(), "tmux");
        }

        #[test]
        fn empty_presence_values_count_as_unset() {
            let empty_both = |name: &str| match name {
                "HERDR_ENV" | "TMUX" => Some(String::new()),
                _ => None,
            };
            let err = resolve_multiplexer_name(None, empty_both)
                .expect_err("empty presence vars mean no backend");
            assert!(err.to_string().starts_with("no supported multiplexer detected:"));
        }

        #[test]
        fn both_present_is_a_hard_error() {
            let both = |name: &str| match name {
                "HERDR_ENV" => Some("1".to_string()),
                "TMUX" => Some("/tmp/tmux".to_string()),
                _ => None,
            };
            let err = resolve_multiplexer_name(None, both).expect_err("ambiguous env");
            assert_eq!(
                err.to_string(),
                "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
                 set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate"
            );
        }

        #[test]
        fn neither_present_is_a_hard_error() {
            let err = resolve_multiplexer_name(None, no_env).expect_err("no backend");
            assert_eq!(
                err.to_string(),
                "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
                 run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER"
            );
        }
    }
}
