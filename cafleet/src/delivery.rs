use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{
    Deadline, PaneLock, complete_database_budget, retry_database_completion, with_database_budget,
};
use crate::multiplexer::{AnyMultiplexer, Multiplexer, MultiplexerError};
use crate::output::truncate_text;
use crate::pane_state::{PaneState, classify};
use crate::receive::{AttemptOutcome, Delivery, NotificationError};
use crate::time::format_utc;
use rusqlite::Connection;

pub const DELIVERY_CAPTURE_LINES: i64 = 80;

pub(crate) fn attempt_preview(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
) -> Result<AttemptOutcome, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        attempt_preview_scoped(conn, settings, member_id, expected_fleet, deadline, resolve)
    })
}

fn read_snapshot<T>(
    conn: &mut Connection,
    deadline: Deadline,
    read: impl FnOnce(&Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    deadline.remaining()?;
    let tx = conn.transaction().map_err(broker::members::db_err)?;
    let result = read(&tx)?;
    tx.commit().map_err(broker::members::db_err)?;
    Ok(result)
}

type PreviewSnapshot = (
    broker::records::MemberRecord,
    broker::records::MessageRecord,
);

fn preview_snapshot(
    conn: &mut Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
) -> Result<Option<PreviewSnapshot>, CafleetError> {
    read_snapshot(conn, deadline, |conn| {
        let Some(fleet_id) = broker::active_member_fleet(conn, member_id)? else {
            return Ok(None);
        };
        if expected_fleet.is_some_and(|expected| expected != fleet_id) {
            return Ok(None);
        }
        let fleet = broker::fleets::fetch_fleet(conn, fleet_id)?
            .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
        if fleet.director_member_id == Some(member_id) {
            return Ok(None);
        }
        let Some(member) = broker::get_member(conn, member_id, fleet_id)? else {
            return Ok(None);
        };
        let Some(message) = broker::oldest_pending_preview(conn, member_id)? else {
            return Ok(None);
        };
        Ok(Some((member, message)))
    })
}

enum PreviewClaim {
    Ready(broker::records::MessageRecord),
    Skipped(AttemptOutcome),
}

fn claim_current_preview(
    conn: &mut Connection,
    member: &broker::records::MemberRecord,
    message_id: i64,
    deadline: Deadline,
) -> Result<PreviewClaim, CafleetError> {
    deadline.remaining()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(broker::members::db_err)?;
    let Some(current) = broker::get_member(&tx, member.member_id, member.fleet_id)? else {
        return Ok(PreviewClaim::Skipped(AttemptOutcome::NoPending));
    };
    if current.placement != member.placement {
        return Ok(PreviewClaim::Skipped(AttemptOutcome::Held));
    }
    let Some(message) = broker::messaging::claim_preview(&tx, message_id, member.member_id)? else {
        return Ok(PreviewClaim::Skipped(AttemptOutcome::Settled {
            message_id,
        }));
    };
    if let Some(earlier) = broker::oldest_pending_preview(&tx, member.member_id)?
        && (&earlier.created_at, earlier.message_id) < (&message.created_at, message.message_id)
    {
        return Ok(PreviewClaim::Skipped(AttemptOutcome::Held));
    }
    deadline.remaining()?;
    tx.commit().map_err(broker::members::db_err)?;
    Ok(PreviewClaim::Ready(message))
}

fn attempt_preview_scoped(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
) -> Result<AttemptOutcome, CafleetError> {
    let Some((member, initial)) = preview_snapshot(conn, member_id, expected_fleet, deadline)?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    let fleet_id = member.fleet_id;
    let Some(placement) = member
        .placement
        .as_ref()
        .filter(|p| p.mux_pane_id.is_some())
    else {
        return Ok(AttemptOutcome::Held);
    };
    let failed = |message_id, delivery, error| {
        AttemptOutcome::Failed(NotificationError {
            member_id,
            message_id,
            delivery,
            error,
        })
    };
    let _guard = match PaneLock::try_acquire(&settings.database_url, placement) {
        Ok(Some(guard)) => guard,
        Ok(None) => return Ok(AttemptOutcome::Held),
        Err(error) => {
            return Ok(failed(
                initial.message_id,
                Delivery::NotSent,
                format!("member {member_id} pane input: {}", error.message()),
            ));
        }
    };
    let Some((current, message)) = preview_snapshot(conn, member_id, Some(fleet_id), deadline)?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    if current.placement != member.placement {
        return Ok(AttemptOutcome::Held);
    }
    let mux = match resolve().and_then(|mux| {
        mux.ensure_available()?;
        Ok(mux.bounded(deadline))
    }) {
        Ok(mux) => mux,
        Err(error) => {
            return Ok(failed(
                message.message_id,
                Delivery::NotSent,
                error.to_string(),
            ));
        }
    };
    let pane = placement
        .mux_pane_id
        .as_deref()
        .expect("concrete preview placement");
    let content = match mux.capture_pane(pane, DELIVERY_CAPTURE_LINES) {
        Ok(content) => content,
        Err(error) => {
            return Ok(failed(
                message.message_id,
                Delivery::NotSent,
                error.to_string(),
            ));
        }
    };
    let state = classify(&placement.coding_agent, &content);
    if let Some(reason) = state.hold_reason() {
        return Ok(if matches!(state, PaneState::Draft | PaneState::Unknown) {
            failed(
                message.message_id,
                Delivery::NotSent,
                format!(
                    "member {member_id} is {reason}; preview {} was not sent.",
                    message.message_id
                ),
            )
        } else {
            AttemptOutcome::Held
        });
    }
    let message = match claim_current_preview(conn, &member, message.message_id, deadline)? {
        PreviewClaim::Ready(message) => message,
        PreviewClaim::Skipped(outcome) => return Ok(outcome),
    };
    let text =
        truncate_text(Some(&message.text), settings.max_text_len).expect("present message text");
    let uncertain = |cause| {
        failed(
            message.message_id,
            Delivery::Uncertain,
            format!(
                "preview {} delivery to member {member_id} is uncertain: {cause}. Inspect the pane before retrying.",
                message.message_id
            ),
        )
    };
    if let Err(error) = mux.send_inline_preview(
        pane,
        message.message_id,
        message.from_member_id,
        &message.created_at,
        &text,
    ) {
        return Ok(if error.submission_started() {
            uncertain(error.to_string())
        } else {
            match retry_database_completion(conn, deadline, |conn| {
                broker::messaging::release_preview(conn, message.message_id)
            }) {
                Ok(()) => failed(message.message_id, Delivery::NotSent, error.to_string()),
                Err(cleanup) => failed(
                    message.message_id,
                    Delivery::NotSent,
                    format!(
                        "{error}; releasing the input claim failed: {cleanup}. This preview remains suspended; later messages remain eligible."
                    ),
                ),
            }
        });
    }
    match complete_database_budget(conn, deadline, |conn| {
        broker::messaging::stamp_notified_validated(
            conn,
            message.message_id,
            &format_utc(crate::time::now_utc()),
        )
    }) {
        Ok(true) => Ok(AttemptOutcome::Sent {
            message_id: message.message_id,
        }),
        Ok(false) => Ok(AttemptOutcome::Settled {
            message_id: message.message_id,
        }),
        Err(error) => Ok(uncertain(error.message().into_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::test_support as common;
    use crate::multiplexer::{CommandRunner, RunError, TmuxMultiplexer};
    use std::{
        collections::HashMap,
        rc::Rc,
        time::{Duration, Instant},
    };

    struct FinishingRunner {
        deadline: Deadline,
        submitted: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[test]
    fn obsolete_claims_are_settled_without_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "settled");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let member = broker::get_member(&conn, worker, fleet).unwrap().unwrap();
        for state in ["acked", "notified", "claimed"] {
            let message =
                broker::send_message(&mut conn, director, &worker.to_string(), state).unwrap();
            match state {
                "acked" => {
                    broker::ack_message(&mut conn, message.message_id).unwrap();
                }
                "notified" => {
                    conn.execute(
                        "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
                        [message.message_id],
                    )
                    .unwrap();
                }
                "claimed" => {
                    assert!(
                        broker::messaging::claim_preview(&conn, message.message_id, worker)
                            .unwrap()
                            .is_some()
                    );
                }
                _ => unreachable!(),
            }
            assert!(matches!(
                claim_current_preview(&mut conn, &member, message.message_id, Deadline::new()).unwrap(),
                PreviewClaim::Skipped(AttemptOutcome::Settled { message_id }) if message_id == message.message_id
            ));
        }
    }

    #[test]
    fn an_earlier_candidate_rolls_back_the_tentative_claim() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "ordering");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let member = broker::get_member(&conn, worker, fleet).unwrap().unwrap();
        let selected =
            broker::send_message(&mut conn, director, &worker.to_string(), "selected").unwrap();
        let earlier =
            broker::send_message(&mut conn, director, &worker.to_string(), "earlier").unwrap();
        conn.execute(
            "UPDATE messages SET created_at='1900' WHERE message_id=?1",
            [earlier.message_id],
        )
        .unwrap();
        assert!(matches!(
            claim_current_preview(&mut conn, &member, selected.message_id, Deadline::new())
                .unwrap(),
            PreviewClaim::Skipped(AttemptOutcome::Held)
        ));
        assert!(
            !conn
                .query_row(
                    "SELECT preview_started_at IS NOT NULL FROM messages WHERE message_id=?1",
                    [selected.message_id],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker)
                .unwrap()
                .unwrap()
                .message_id,
            earlier.message_id
        );
    }

    #[test]
    fn a_director_without_a_placement_has_no_preview_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "director");
        conn.execute(
            "DELETE FROM member_placements WHERE member_id=?1",
            [director],
        )
        .unwrap();
        let settings = Settings::from_lookup(|_| None).unwrap();
        let outcome = attempt_preview(
            &mut conn,
            &settings,
            director,
            Some(fleet),
            Deadline::new(),
            || panic!("Director needs no multiplexer"),
        );
        assert_eq!(outcome.unwrap(), AttemptOutcome::NoPending);
    }

    impl CommandRunner for FinishingRunner {
        fn binary_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&self, argv: &[String], _: Option<u64>) -> Result<String, RunError> {
            if argv[1] == "capture-pane" {
                return Ok("Ready.\n────────────\n❯ \x1b[0m\n────────────\n? for shortcuts".into());
            }
            assert_eq!(argv[1], "send-keys");
            if argv.last().unwrap() == "Enter" {
                std::thread::sleep(self.deadline.remaining().unwrap() + Duration::from_millis(5));
                self.submitted
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(String::new())
        }
        fn sleep(&self, _: f64) {}
    }

    #[test]
    fn a_completed_submission_is_stamped_even_when_its_budget_has_just_expired() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "stamp");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let message =
            broker::send_message(&mut conn, director, &worker.to_string(), "assignment").unwrap();
        let settings = Settings::from_lookup(|name| {
            (name == "CAFLEET_DATABASE_URL").then(|| format!("sqlite:///{}", conn.path().unwrap()))
        })
        .unwrap();
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let scopes = Arc::new(AtomicUsize::new(0));
        let observed = scopes.clone();
        let submitted = Arc::new(AtomicBool::new(false));
        let stamped = submitted.clone();
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if stamped.load(Ordering::SeqCst)
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "sqlite_master" | "sqlite_schema" | "refinery_schema_history",
                        ..
                    }
                )
            {
                return Authorization::Deny;
            }
            if matches!(
                context.action,
                AuthAction::Pragma {
                    pragma_name: "busy_timeout",
                    ..
                }
            ) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }))
        .unwrap();
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(2200));
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            Rc::new(FinishingRunner {
                deadline,
                submitted,
            }),
            HashMap::from([("TMUX".into(), "test".into())]),
        ));
        assert_eq!(
            attempt_preview(&mut conn, &settings, worker, Some(fleet), deadline, || Ok(
                mux
            ))
            .unwrap(),
            AttemptOutcome::Sent {
                message_id: message.message_id
            }
        );
        assert!(deadline.remaining().is_err());
        assert_eq!(
            scopes.load(Ordering::SeqCst),
            2,
            "the input attempt and mandatory finalization own separate database budgets"
        );
        assert!(
            broker::oldest_pending_preview(&conn, worker)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn preview_reads_share_one_snapshot_when_another_connection_changes_placement_and_text() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        let (fleet, director) = common::create_fleet(&mut conn, "snapshot");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let message =
            broker::send_message(&mut conn, director, &worker.to_string(), "original").unwrap();
        let path = conn.path().unwrap().to_owned();
        let mut changed = false;
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if !changed
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "messages",
                        ..
                    }
                )
            {
                changed = true;
                let other = Connection::open(&path).unwrap();
                other
                    .execute(
                        "UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1",
                        [worker],
                    )
                    .unwrap();
                other
                    .execute(
                        "UPDATE messages SET text='changed' WHERE message_id=?1",
                        [message.message_id],
                    )
                    .unwrap();
            }
            Authorization::Allow
        }))
        .unwrap();
        let (member, message) = preview_snapshot(&mut conn, worker, Some(fleet), Deadline::new())
            .unwrap()
            .unwrap();
        assert_eq!(member.placement.unwrap().mux_pane_id.as_deref(), Some("%2"));
        assert_eq!(message.text, "original");
        let current: String = conn
            .query_row("SELECT text FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(current, "changed");
    }

    mod notification_line_tests {
        use super::super::{INLINE_BODY_LIMIT, notification_line};
        use crate::broker::records::{MessageRecord, MessageStatus};

        const TS: &str = "2026-10-24T09:00:00.000000+00:00";
        const CLAUSES: &str = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

        fn record(text: &str) -> MessageRecord {
            MessageRecord {
                message_id: 57,
                from_member_id: 15,
                to_member_id: 16,
                created_at: TS.into(),
                status: MessageStatus::InputRequired,
                status_timestamp: TS.into(),
                text: text.into(),
            }
        }

        fn line_body(text: &str) -> String {
            let line = notification_line(&record(text));
            let prefix = format!("[cafleet msg 57 from 15 {TS}] ");
            let suffix = format!(" {CLAUSES}");
            assert!(line.starts_with(&prefix), "{line}");
            assert!(line.ends_with(&suffix), "{line}");
            line[prefix.len()..line.len() - suffix.len()].to_owned()
        }

        fn pointer(stored_chars: usize) -> String {
            format!(
                "Body is {stored_chars} characters; run cafleet message show 57 --json to read it."
            )
        }

        #[test]
        fn the_inline_limit_is_4000() {
            assert_eq!(INLINE_BODY_LIMIT, 4000);
        }

        #[test]
        fn the_line_matches_the_specified_example() {
            assert_eq!(
                notification_line(&record("ready (doc)\nsee the reviewer notes")),
                format!("[cafleet msg 57 from 15 {TS}] ready (doc)⏎see the reviewer notes {CLAUSES}")
            );
        }

        #[test]
        fn a_single_line_body_is_inline_unchanged() {
            assert_eq!(line_body("assignment"), "assignment");
        }

        #[test]
        fn multiline_tab_and_control_characters_are_folded_visibly() {
            assert_eq!(
                line_body("a\r\nb\nc\rd\tx\x1b\x7f"),
                "a⏎b⏎c⏎d␉x\\x1b\\x7f"
            );
            let line = notification_line(&record("! printf danger\r\nnext\u{0}"));
            assert!(crate::input::valid_input(&line), "{line:?}");
        }

        #[test]
        fn a_folded_body_of_exactly_4000_characters_is_inline() {
            let body = "x".repeat(4000);
            assert_eq!(line_body(&body), body);
        }

        #[test]
        fn a_folded_body_of_4001_characters_becomes_the_show_pointer() {
            assert_eq!(line_body(&"x".repeat(4001)), pointer(4001));
        }

        #[test]
        fn the_limit_counts_unicode_scalar_values_not_bytes() {
            let body = "é".repeat(4000);
            assert_eq!(line_body(&body), body);
            assert_eq!(line_body(&"é".repeat(4001)), pointer(4001));
        }

        #[test]
        fn the_limit_applies_to_the_folded_body_and_the_pointer_counts_stored_text() {
            let crlf = "\r\n".repeat(4000);
            assert_eq!(
                line_body(&crlf),
                "⏎".repeat(4000),
                "8000 stored characters fold to 4000"
            );
            let controls = "\u{1}".repeat(1001);
            assert_eq!(
                line_body(&controls),
                pointer(1001),
                "1001 stored characters fold to 4004"
            );
        }

        #[test]
        fn inline_and_pointer_lines_both_carry_both_clauses() {
            for text in ["short", &"y".repeat(5000)] {
                let line = notification_line(&record(text));
                assert!(line.ends_with(&format!(" {CLAUSES}")), "{line}");
                assert_eq!(line.matches("[cafleet] ").count(), 2, "{line}");
            }
        }

        #[test]
        fn the_line_ignores_the_cli_text_length_limit() {
            let body = "z".repeat(3999);
            assert_eq!(line_body(&body), body);
        }
    }
}
