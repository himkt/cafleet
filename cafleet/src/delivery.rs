use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, PaneLock, complete_database_budget, with_database_budget};
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
    let Some(fleet_id) = with_database_budget(conn, deadline, |conn| {
        broker::active_member_fleet(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    if expected_fleet.is_some_and(|expected| expected != fleet_id) {
        return Ok(AttemptOutcome::NoPending);
    }
    let member = with_database_budget(conn, deadline, |conn| {
        broker::get_member(conn, member_id, fleet_id)
    })?;
    let Some(member) = member else {
        return Ok(AttemptOutcome::NoPending);
    };
    let Some(placement) = member
        .placement
        .as_ref()
        .filter(|p| p.mux_pane_id.is_some())
    else {
        return Ok(AttemptOutcome::Held);
    };
    if member.kind == broker::records::MemberKind::Director {
        return Ok(AttemptOutcome::NoPending);
    }
    let Some(initial) = with_database_budget(conn, deadline, |conn| {
        broker::oldest_pending_preview(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
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
    let current = with_database_budget(conn, deadline, |conn| {
        broker::get_member(conn, member_id, fleet_id)
    })?;
    let Some(current) = current else {
        return Ok(AttemptOutcome::NoPending);
    };
    if current.placement != member.placement {
        return Ok(AttemptOutcome::Held);
    }
    let Some(message) = with_database_budget(conn, deadline, |conn| {
        broker::oldest_pending_preview(conn, member_id)
    })?
    else {
        return Ok(AttemptOutcome::NoPending);
    };
    if message.from_member_id == member_id {
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
    let hold = with_database_budget(conn, deadline, |conn| {
        let Some(current) = broker::get_member(conn, member_id, fleet_id)? else {
            return Ok(Some(AttemptOutcome::NoPending));
        };
        if !broker::messaging::is_preview_pending(conn, message.message_id)? {
            return Ok(Some(AttemptOutcome::Settled {
                message_id: message.message_id,
            }));
        }
        if current.placement != member.placement {
            return Ok(Some(AttemptOutcome::Held));
        }
        let Some(pending) = broker::oldest_pending_preview(conn, member_id)? else {
            return Ok(Some(AttemptOutcome::NoPending));
        };
        if pending.message_id != message.message_id {
            return Ok(Some(AttemptOutcome::Held));
        }
        Ok(None)
    })?;
    if let Some(hold) = hold {
        return Ok(hold);
    }
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
            failed(message.message_id, Delivery::NotSent, error.to_string())
        });
    }
    match complete_database_budget(conn, deadline, |conn| {
        broker::stamp_notified(
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
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(2200));
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            Rc::new(FinishingRunner { deadline }),
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
        assert!(
            broker::oldest_pending_preview(&conn, worker)
                .unwrap()
                .is_none()
        );
    }
}
