use crate::broker;
use crate::broker::records::MemberKind;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{
    Deadline, PaneLock, complete_database_budget, retry_database_completion, with_database_budget,
};
use crate::multiplexer::{
    AnyMultiplexer, INPUT_RESERVATION, INSUFFICIENT_INPUT_BUDGET, Lead, MIN_LITERAL_BUDGET,
    Multiplexer, MultiplexerError,
};
use crate::receive::{AttemptEnd, AttemptOutcome, Delivery, NotificationError};
use crate::time::format_utc;
use rusqlite::Connection;

pub(crate) const INLINE_BODY_LIMIT: usize = 4000;
const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
const DISMISSED_PROMPT_NOTE: &str = "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

fn fold_visible(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                result.push('⏎');
            }
            '\n' => result.push('⏎'),
            '\t' => result.push('␉'),
            '\u{00}'..='\u{1f}' | '\u{7f}' => result.push_str(&format!("\\x{:02x}", ch as u32)),
            _ => result.push(ch),
        }
    }
    result
}

pub(crate) fn notification_line(message: &broker::records::MessageRecord) -> String {
    let folded = fold_visible(&message.text);
    let body = if folded.chars().count() <= INLINE_BODY_LIMIT {
        folded
    } else {
        format!(
            "Body is {} characters; run cafleet message show {} --json to read it.",
            message.text.chars().count(),
            message.message_id
        )
    };
    format!(
        "[cafleet msg {} from {} {}] {body} {RESUME_CLAUSE} {DISMISSED_PROMPT_NOTE}",
        message.message_id, message.from_member_id, message.created_at
    )
}

pub(crate) fn deliver_pending(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
) -> Result<AttemptOutcome, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        deliver_pending_scoped(conn, settings, member_id, expected_fleet, deadline, resolve)
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
        let Some(member) = broker::get_member(conn, member_id, fleet_id)? else {
            return Ok(None);
        };
        if member.kind != MemberKind::Member {
            return Ok(None);
        }
        let Some(message) = broker::oldest_pending_preview(conn, member_id)? else {
            return Ok(None);
        };
        Ok(Some((member, message)))
    })
}

enum ClaimSkip {
    Ineligible,
    /// The placement changed, or an earlier candidate appeared.
    Held,
    /// The message was ACKed or stamped meanwhile.
    Settled,
}

enum PreviewClaim {
    Ready(broker::records::MessageRecord),
    Skipped(ClaimSkip),
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
        return Ok(PreviewClaim::Skipped(ClaimSkip::Ineligible));
    };
    if current.kind != MemberKind::Member {
        return Ok(PreviewClaim::Skipped(ClaimSkip::Ineligible));
    }
    if current.placement != member.placement {
        return Ok(PreviewClaim::Skipped(ClaimSkip::Held));
    }
    let Some(message) = broker::messaging::claim_preview(&tx, message_id, member.member_id)? else {
        return Ok(PreviewClaim::Skipped(ClaimSkip::Settled));
    };
    if let Some(earlier) = broker::oldest_pending_preview(&tx, member.member_id)?
        && (&earlier.created_at, earlier.message_id) < (&message.created_at, message.message_id)
    {
        return Ok(PreviewClaim::Skipped(ClaimSkip::Held));
    }
    deadline.remaining()?;
    tx.commit().map_err(broker::members::db_err)?;
    Ok(PreviewClaim::Ready(message))
}

fn deliver_pending_scoped(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
) -> Result<AttemptOutcome, CafleetError> {
    let ended = |end| Ok(AttemptOutcome::ended(end));
    let failed = |message_id, delivery, error| {
        AttemptEnd::Failed(NotificationError {
            member_id,
            message_id,
            delivery,
            error,
        })
    };
    let Some((member, initial)) = preview_snapshot(conn, member_id, expected_fleet, deadline)?
    else {
        return ended(AttemptEnd::NoPending);
    };
    let fleet_id = member.fleet_id;
    let Some(placement) = member
        .placement
        .as_ref()
        .filter(|p| p.mux_pane_id.is_some())
    else {
        return ended(AttemptEnd::Held);
    };
    let lock = PaneLock::acquire_within(
        &settings.database_url,
        placement,
        deadline.reserving(INPUT_RESERVATION),
    );
    let _guard = match lock {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            let still_pending = read_snapshot(conn, deadline, |conn| {
                broker::oldest_pending_preview(conn, member_id)
            })?;
            return ended(match still_pending {
                None => AttemptEnd::NoPending,
                Some(message) => failed(
                    message.message_id,
                    Delivery::NotSent,
                    format!(
                        "member {member_id}'s pane input remained busy; preview {} was not sent.",
                        message.message_id
                    ),
                ),
            });
        }
        Err(error) => {
            return ended(failed(
                initial.message_id,
                Delivery::NotSent,
                format!("member {member_id} pane input: {}", error.message()),
            ));
        }
    };
    let _span = crate::signals::DeferTermination::begin()?;
    let Some((current, first)) = preview_snapshot(conn, member_id, Some(fleet_id), deadline)?
    else {
        return ended(AttemptEnd::NoPending);
    };
    if current.placement != member.placement {
        return ended(AttemptEnd::Held);
    }
    let mux = match resolve().and_then(|mux| {
        mux.ensure_available()?;
        Ok(mux.bounded(deadline))
    }) {
        Ok(mux) => mux,
        Err(error) => {
            return ended(failed(
                first.message_id,
                Delivery::NotSent,
                error.to_string(),
            ));
        }
    };
    let pane = placement
        .mux_pane_id
        .as_deref()
        .expect("concrete preview placement");
    let mut sent = Vec::new();
    let mut lead = Lead::Escape;
    let end = loop {
        let Ok(remaining) = deadline.remaining() else {
            break if sent.is_empty() {
                failed(
                    first.message_id,
                    Delivery::NotSent,
                    INSUFFICIENT_INPUT_BUDGET.into(),
                )
            } else {
                AttemptEnd::BudgetExhausted
            };
        };
        let Some(candidate) = read_snapshot(conn, deadline, |conn| {
            broker::oldest_pending_preview(conn, member_id)
        })?
        else {
            break AttemptEnd::Drained;
        };
        if remaining < lead.reserved() + MIN_LITERAL_BUDGET {
            break if sent.is_empty() {
                failed(
                    candidate.message_id,
                    Delivery::NotSent,
                    INSUFFICIENT_INPUT_BUDGET.into(),
                )
            } else {
                AttemptEnd::BudgetExhausted
            };
        }
        let message = match claim_current_preview(conn, &member, candidate.message_id, deadline)? {
            PreviewClaim::Ready(message) => message,
            PreviewClaim::Skipped(ClaimSkip::Ineligible) => break AttemptEnd::NoPending,
            PreviewClaim::Skipped(ClaimSkip::Held) => break AttemptEnd::Held,
            PreviewClaim::Skipped(ClaimSkip::Settled) => continue,
        };
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
        if let Err(error) = mux.send_prompt(pane, &notification_line(&message), lead) {
            break if error.submission_started() {
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
            };
        }
        lead = Lead::Direct;
        match complete_database_budget(conn, deadline, |conn| {
            broker::messaging::stamp_notified_validated(
                conn,
                message.message_id,
                &format_utc(crate::time::now_utc()),
            )
        }) {
            Ok(_) => sent.push(message.message_id),
            Err(error) => break uncertain(error.message().into_owned()),
        }
    };
    Ok(AttemptOutcome { sent, end })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::test_support as common;
    use crate::multiplexer::{CommandRunner, RunError, TmuxMultiplexer};
    use crate::receive::AttemptEnd;
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
                claim_current_preview(&mut conn, &member, message.message_id, Deadline::new())
                    .unwrap(),
                PreviewClaim::Skipped(ClaimSkip::Settled)
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
            PreviewClaim::Skipped(ClaimSkip::Held)
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
    fn an_inactive_member_leaves_its_preview_unclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "gone");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let member = broker::get_member(&conn, worker, fleet).unwrap().unwrap();
        let message =
            broker::send_message(&mut conn, director, &worker.to_string(), "gone").unwrap();
        broker::deregister_member(&mut conn, worker).unwrap();
        assert!(matches!(
            claim_current_preview(&mut conn, &member, message.message_id, Deadline::new()).unwrap(),
            PreviewClaim::Skipped(_)
        ));
        assert!(
            conn.query_row(
                "SELECT preview_started_at IS NULL FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        );
    }

    fn settings_for(conn: &Connection) -> Settings {
        let url = format!("sqlite:///{}", conn.path().unwrap());
        Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone())).unwrap()
    }

    #[test]
    fn a_director_snapshot_preserves_its_pending_inbox_without_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "director");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let reply =
            broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
        assert!(
            preview_snapshot(&mut conn, director, Some(fleet), Deadline::new())
                .unwrap()
                .is_none()
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn director_delivery_ends_before_resolving_or_locking_a_pane() {
        for backend in ["claude", "codex", "opencode"] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = common::migrated_conn(&dir);
            let (fleet, director) = common::create_fleet(&mut conn, "director");
            let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
            conn.execute(
                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                rusqlite::params![backend, director],
            )
            .unwrap();
            let reply =
                broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
            let settings = settings_for(&conn);
            let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
            let lock =
                crate::input::lock_path(&settings.database_url, member.placement.as_ref().unwrap())
                    .unwrap();
            let outcome = deliver_pending(
                &mut conn,
                &settings,
                director,
                Some(fleet),
                Deadline::new(),
                || panic!("Director delivery must finish before multiplexer resolution"),
            )
            .unwrap();
            assert!(outcome.sent.is_empty());
            assert!(matches!(outcome.end, AttemptEnd::NoPending));
            assert!(!lock.exists(), "{backend}: {}", lock.display());
            let (notified, claimed): (Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT notified_at, preview_started_at FROM messages WHERE message_id=?1",
                    [reply.message_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!((notified, claimed), (None, None));
            assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
        }
    }

    #[test]
    fn a_member_promoted_to_root_director_is_ineligible_for_a_stale_claim() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "promotion");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let member = broker::get_member(&conn, worker, fleet).unwrap().unwrap();
        let message =
            broker::send_message(&mut conn, director, &worker.to_string(), "assignment").unwrap();
        conn.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![worker, fleet],
        )
        .unwrap();
        assert!(matches!(
            claim_current_preview(&mut conn, &member, message.message_id, Deadline::new()).unwrap(),
            PreviewClaim::Skipped(_)
        ));
        assert!(
            conn.query_row(
                "SELECT preview_started_at IS NULL AND notified_at IS NULL FROM messages WHERE message_id=?1",
                [message.message_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        );
        assert_eq!(broker::poll_messages(&conn, worker).unwrap(), vec![message]);
    }

    #[test]
    fn a_current_director_is_ineligible_for_a_direct_claim() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "director-claim");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
        let reply =
            broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
        assert!(matches!(
            claim_current_preview(&mut conn, &member, reply.message_id, Deadline::new()).unwrap(),
            PreviewClaim::Skipped(_)
        ));
        assert!(
            conn.query_row(
                "SELECT preview_started_at IS NULL AND notified_at IS NULL FROM messages WHERE message_id=?1",
                [reply.message_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        );
    }

    #[test]
    fn a_director_with_existing_preview_metadata_receives_no_automatic_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "director-metadata");
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        let reply =
            broker::send_message(&mut conn, worker, &director.to_string(), "reply").unwrap();
        conn.execute(
            "UPDATE messages SET preview_started_at=created_at WHERE message_id=?1",
            [reply.message_id],
        )
        .unwrap();
        let settings = settings_for(&conn);
        let outcome = deliver_pending(
            &mut conn,
            &settings,
            director,
            Some(fleet),
            Deadline::new(),
            || panic!("Director preview metadata belongs to broker reads"),
        )
        .unwrap();
        assert!(outcome.sent.is_empty());
        assert!(matches!(outcome.end, AttemptEnd::NoPending));
        let (notified, claimed): (Option<String>, String) = conn
            .query_row(
                "SELECT notified_at, preview_started_at FROM messages WHERE message_id=?1",
                [reply.message_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(notified, None);
        assert_eq!(claimed, reply.created_at);
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    impl CommandRunner for FinishingRunner {
        fn binary_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&self, argv: &[String], _: Option<u64>) -> Result<String, RunError> {
            assert_ne!(argv[1], "capture-pane", "delivery never captures the pane");
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
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(3200));
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            Rc::new(FinishingRunner {
                deadline,
                submitted,
            }),
            HashMap::from([("TMUX".into(), "test".into())]),
        ));
        let outcome = deliver_pending(&mut conn, &settings, worker, Some(fleet), deadline, || {
            Ok(mux)
        })
        .unwrap();
        assert_eq!(outcome.sent, vec![message.message_id]);
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
                format!(
                    "[cafleet msg 57 from 15 {TS}] ready (doc)⏎see the reviewer notes {CLAUSES}"
                )
            );
        }

        #[test]
        fn a_single_line_body_is_inline_unchanged() {
            assert_eq!(line_body("assignment"), "assignment");
        }

        #[test]
        fn multiline_tab_and_control_characters_are_folded_visibly() {
            assert_eq!(line_body("a\r\nb\nc\rd\tx\x1b\x7f"), "a⏎b⏎c⏎d␉x\\x1b\\x7f");
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
