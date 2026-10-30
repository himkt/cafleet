use crate::broker;
use crate::broker::records::{MemberKind, Placement};
use crate::broker::runtime::{
    self as storage, InputDisposition, InputKind, InputState, PermissionReceiptIdentity,
};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, PaneLock, complete_database_budget, with_database_budget};
use crate::multiplexer::{
    AnyMultiplexer, INPUT_RESERVATION, INSUFFICIENT_INPUT_BUDGET, Lead, MIN_LITERAL_BUDGET,
    MultiplexerError,
};
use crate::receive::{AttemptEnd, AttemptOutcome, NotificationError};
use crate::runtime::runner::{
    InputCommandHost, InputFailure, NativeInputHost, PreparedSubmission, RealInputDeadlineClock,
    SubmissionStatus, SubmitInput, find_submission, submit_wrapped_input_under_pane_lock_with_host,
    validate_submission_target,
};
use rusqlite::Connection;
use std::collections::BTreeSet;

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
    format!(
        "{} {DISMISSED_PROMPT_NOTE}",
        canonical_preview_line(message)
    )
}

pub(crate) fn fallback_preview_line(canonical: &str) -> String {
    format!("{canonical} {DISMISSED_PROMPT_NOTE}")
}

pub(crate) fn canonical_preview_line(message: &broker::records::MessageRecord) -> String {
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
        "[cafleet msg {} from {} {}] {body} {RESUME_CLAUSE}",
        message.message_id, message.from_member_id, message.created_at
    )
}

pub(crate) fn notification_error(
    member_id: i64,
    message_id: i64,
    cause: impl std::fmt::Display,
) -> NotificationError {
    NotificationError {
        member_id,
        message_id,
        error: format!(
            "message {message_id} was not notified to member {member_id}: {cause}; the next send to member {member_id} or the next message wait retries it."
        ),
    }
}

fn storage_failure(
    member_id: i64,
    message_id: i64,
    error: CafleetError,
) -> Result<AttemptEnd, CafleetError> {
    match error {
        CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded => Ok(AttemptEnd::Failed(
            notification_error(member_id, message_id, error),
        )),
        error => Err(error),
    }
}

pub(crate) fn deliver_pending(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
) -> Result<AttemptOutcome, CafleetError> {
    deliver_pending_with(
        conn,
        settings,
        member_id,
        expected_fleet,
        deadline,
        attempted,
        || crate::runtime::resolve_mux(settings),
        &mut NativeInputHost,
    )
}

fn read_snapshot<T>(
    conn: &mut Connection,
    deadline: Deadline,
    read: impl FnOnce(&Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    deadline.remaining()?;
    let tx = conn.transaction()?;
    let result = read(&tx)?;
    tx.commit()?;
    Ok(result)
}

type PreviewSnapshot = (
    broker::records::MemberRecord,
    broker::records::MessageRecord,
);

fn eligible_member(
    conn: &Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
) -> Result<Option<broker::records::MemberRecord>, CafleetError> {
    let Some(fleet_id) = broker::active_member_fleet(conn, member_id)? else {
        return Ok(None);
    };
    if expected_fleet.is_some_and(|expected| expected != fleet_id) {
        return Ok(None);
    }
    Ok(broker::get_member(conn, member_id, fleet_id)?
        .filter(|member| member.kind == MemberKind::Member))
}

fn preview_snapshot(
    conn: &mut Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
) -> Result<Option<PreviewSnapshot>, CafleetError> {
    read_snapshot(conn, deadline, |conn| {
        let Some(member) = eligible_member(conn, member_id, expected_fleet)? else {
            return Ok(None);
        };
        let Some(message) = broker::oldest_pending_preview(conn, member_id, attempted)? else {
            return Ok(None);
        };
        Ok(Some((member, message)))
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the delivery seam takes every production argument plus the multiplexer resolver and input host that tests inject"
)]
fn deliver_pending_with(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
    resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
    host: &mut impl InputCommandHost,
) -> Result<AttemptOutcome, CafleetError> {
    let deadline = Deadline::until(crate::input::bounded_by_inherited(conn, deadline.instant()));
    with_database_budget(conn, deadline, |conn| {
        let ended = |end| Ok(AttemptOutcome::ended(end));
        let failed = |message_id, cause: String| {
            AttemptEnd::Failed(notification_error(member_id, message_id, cause))
        };
        let Some((member, initial)) =
            preview_snapshot(conn, member_id, expected_fleet, deadline, attempted)?
        else {
            return ended(AttemptEnd::NoPending);
        };
        let Some(placement) = member.placement.clone().filter(|p| p.mux_pane_id.is_some()) else {
            return ended(AttemptEnd::Held);
        };
        if member.runtime.is_some() {
            return deliver_wrapped_pending(
                conn,
                settings,
                (member, initial),
                &placement,
                deadline,
                attempted,
                host,
            );
        }
        let fleet_id = member.fleet_id;
        let lock = PaneLock::acquire_within(
            &settings.database_url,
            &placement,
            deadline.reserving(INPUT_RESERVATION),
        );
        let _guard = match lock {
            Ok(Some(guard)) => guard,
            Ok(None) => {
                let still_pending = match read_snapshot(conn, deadline, |conn| {
                    broker::oldest_pending_preview(conn, member_id, attempted)
                }) {
                    Ok(pending) => pending,
                    Err(error) => {
                        return ended(storage_failure(member_id, initial.message_id, error)?);
                    }
                };
                return ended(match still_pending {
                    None => AttemptEnd::NoPending,
                    Some(message) => failed(message.message_id, "pane input remained busy".into()),
                });
            }
            Err(error) => {
                return ended(failed(
                    initial.message_id,
                    format!("pane input: {}", error.message()),
                ));
            }
        };
        let _span = crate::signals::DeferTermination::begin()?;
        let (current, first) =
            match preview_snapshot(conn, member_id, Some(fleet_id), deadline, attempted) {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => return ended(AttemptEnd::NoPending),
                Err(error) => return ended(storage_failure(member_id, initial.message_id, error)?),
            };
        if current.placement != member.placement {
            return ended(AttemptEnd::Held);
        }
        let mux = match resolve().and_then(|mux| {
            mux.ensure_available()?;
            Ok(mux.bounded(deadline))
        }) {
            Ok(mux) => mux,
            Err(error) => return ended(failed(first.message_id, error.to_string())),
        };
        let pane = placement
            .mux_pane_id
            .as_deref()
            .expect("concrete preview placement");
        let mut sent = Vec::new();
        let mut lead = Lead::Escape;
        let mut candidate_id = first.message_id;
        let end = loop {
            if deadline.remaining().is_err() {
                break if sent.is_empty() {
                    failed(candidate_id, INSUFFICIENT_INPUT_BUDGET.into())
                } else {
                    AttemptEnd::BudgetExhausted
                };
            }
            let snapshot = read_snapshot(conn, deadline, |conn| {
                let Some(current) = eligible_member(conn, member_id, Some(fleet_id))? else {
                    return Ok(None);
                };
                let candidate = broker::oldest_pending_preview(conn, member_id, attempted)?;
                Ok(Some((current, candidate)))
            });
            let (current, candidate) = match snapshot {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => break AttemptEnd::NoPending,
                Err(error) => break storage_failure(member_id, candidate_id, error)?,
            };
            if current.placement != member.placement {
                break AttemptEnd::Held;
            }
            let Some(message) = candidate else {
                break AttemptEnd::Drained;
            };
            candidate_id = message.message_id;
            if !deadline
                .remaining()
                .is_ok_and(|remaining| remaining >= lead.reserved() + MIN_LITERAL_BUDGET)
            {
                break if sent.is_empty() {
                    failed(candidate_id, INSUFFICIENT_INPUT_BUDGET.into())
                } else {
                    AttemptEnd::BudgetExhausted
                };
            }
            if let Err(error) = mux.send_prompt(pane, &notification_line(&message), lead) {
                break failed(candidate_id, error.to_string());
            }
            lead = Lead::Direct;
            match complete_database_budget(conn, deadline, |conn| {
                broker::messaging::stamp_notified_validated(
                    conn,
                    candidate_id,
                    &crate::time::now_stamp(),
                )
            }) {
                Ok(_) => sent.push(candidate_id),
                Err(error) => break storage_failure(member_id, candidate_id, error)?,
            }
        };
        Ok(AttemptOutcome { sent, end })
    })
}

fn deliver_wrapped_pending(
    conn: &mut Connection,
    settings: &Settings,
    initial: PreviewSnapshot,
    placement: &Placement,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
    host: &mut impl InputCommandHost,
) -> Result<AttemptOutcome, CafleetError> {
    let (member, mut message) = initial;
    let run = member
        .runtime
        .as_ref()
        .expect("wrapped delivery has its run")
        .run
        .clone();
    let identity = PermissionReceiptIdentity::for_run(&run);
    enum Stage {
        Submission,
        Retirement,
        NextPreview,
    }

    let mut guard = None;
    let mut sent = Vec::new();
    let mut operation_key = format!("preview:{}:{}", run.run_id, message.message_id);
    let mut known_input_id = None;
    let mut stage = Stage::Submission;
    let result = (|| {
        let end = loop {
            deadline.remaining()?;
            let request = SubmitInput {
                operation_key: operation_key.clone(),
                input_kind: InputKind::Preview,
                message_id: Some(message.message_id),
                literal: canonical_preview_line(&message),
            };
            let retained =
                find_submission(conn, &identity, &request, deadline).map_err(CafleetError::from)?;
            known_input_id = retained.as_ref().map(|input| input.input_id);
            if retained.as_ref().is_some_and(|input| {
                matches!(input.state, InputState::InProgress | InputState::Uncertain)
                    || (input.state == InputState::Settled
                        && input.disposition != Some(InputDisposition::Submitted))
            }) {
                break AttemptEnd::Held;
            }
            let attempt =
                PreparedSubmission::new(&request, deadline.instant(), &mut RealInputDeadlineClock)?;
            if guard.is_none() {
                guard = host
                    .lock(settings, &run, placement, deadline)
                    .map_err(CafleetError::from)?;
                if guard.is_none() {
                    break AttemptEnd::Failed(notification_error(
                        member.member_id,
                        message.message_id,
                        "pane input remained busy",
                    ));
                }
            }
            let held = guard.as_ref().expect("wrapped delivery owns its pane lock");
            held.validate_binding(&settings.database_url, placement)?;
            with_database_budget(conn, deadline, |conn| {
                validate_submission_target(conn, settings, &run, placement)
            })?;
            let input_id = match retained.as_ref() {
                Some(input)
                    if matches!(input.state, InputState::Accepted | InputState::Settled) =>
                {
                    input.input_id
                }
                _ => {
                    let result = submit_wrapped_input_under_pane_lock_with_host(
                        conn, settings, &run, placement, held, &attempt, deadline, host,
                    )
                    .map_err(CafleetError::from)?;
                    if !matches!(result.status, SubmissionStatus::Accepted) {
                        break AttemptEnd::Held;
                    }
                    result
                        .input_id
                        .expect("accepted Preview has its validated durable input ID")
                }
            };
            known_input_id = Some(input_id);
            let settled = with_database_budget(conn, deadline, |conn| {
                validate_submission_target(conn, settings, &run, placement)?;
                storage::settle_preview(conn, &run.owner(), input_id)
            })
            .map_err(|source| {
                CafleetError::from(InputFailure {
                    source,
                    run_id: Some(run.run_id),
                    interaction_id: None,
                    input_id: Some(input_id),
                    operation_key: Some(request.operation_key.clone()),
                })
            })?;
            sent.push(message.message_id);
            stage = Stage::Retirement;
            deadline.remaining()?;
            held.validate_binding(&settings.database_url, placement)?;
            with_database_budget(conn, deadline, |conn| {
                validate_submission_target(conn, settings, &run, placement)
            })?;
            let retirement = attempt.retirement_request(&run, settled.input_id)?;
            host.exchange(&run, &retirement, deadline.instant())
                .map_err(CafleetError::from)?;
            with_database_budget(conn, deadline, |conn| {
                validate_submission_target(conn, settings, &run, placement)
            })?;
            deadline.remaining()?;
            stage = Stage::NextPreview;
            let Some((current, next)) = preview_snapshot(
                conn,
                member.member_id,
                Some(member.fleet_id),
                deadline,
                attempted,
            )?
            else {
                break AttemptEnd::NoPending;
            };
            if current.placement != member.placement
                || current
                    .runtime
                    .as_ref()
                    .map(|runtime| crate::runtime::runner::stable_run_identity(&runtime.run))
                    != Some(crate::runtime::runner::stable_run_identity(&run))
            {
                break AttemptEnd::Held;
            }
            message = next;
            operation_key = format!("preview:{}:{}", run.run_id, message.message_id);
            known_input_id = None;
            stage = Stage::Submission;
        };
        Ok::<_, CafleetError>(end)
    })();
    let end = match result {
        Ok(end) => end,
        Err(source) if sent.is_empty() => return Err(source),
        Err(source) => {
            let cause = CafleetError::from(InputFailure {
                source,
                run_id: Some(run.run_id),
                interaction_id: None,
                input_id: known_input_id,
                operation_key: Some(operation_key),
            });
            let phase = match stage {
                Stage::Submission => None,
                Stage::Retirement => Some("receipt retirement"),
                Stage::NextPreview => Some("next preview lookup"),
            };
            let warning = match phase {
                None => notification_error(member.member_id, message.message_id, cause),
                Some(phase) => {
                    let input_id = known_input_id.expect("committed Preview has its input ID");
                    NotificationError {
                        member_id: member.member_id,
                        message_id: message.message_id,
                        error: format!(
                            "message {} was notified to member {}; input {input_id} is settled/submitted; {phase} failed: {}",
                            message.message_id,
                            member.member_id,
                            cause.message()
                        ),
                    }
                }
            };
            AttemptEnd::Failed(warning)
        }
    };
    Ok(AttemptOutcome { sent, end })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::multiplexer::{CommandRunner, RunError, TmuxMultiplexer};
    use crate::receive::AttemptEnd;
    use crate::runtime::runner::{
        ControlIdentity, HealthSnapshot, InputCommandHost, InputControlOperation,
        InputControlReply, InputControlRequest, InputFailure, NativeInputHost, ReceiptAction,
        ReceiptReply,
    };
    use std::{
        collections::{BTreeSet, HashMap},
        rc::Rc,
        time::{Duration, Instant},
    };

    struct FinishingRunner {
        deadline: Deadline,
        submitted: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    fn settings_for(conn: &Connection) -> Settings {
        Settings::for_database(std::path::Path::new(conn.path().unwrap()), None)
    }

    fn deliver_natively(
        conn: &mut Connection,
        settings: &Settings,
        member_id: i64,
        expected_fleet: Option<i64>,
        deadline: Deadline,
        attempted: &BTreeSet<i64>,
        resolve: impl FnOnce() -> Result<AnyMultiplexer, MultiplexerError>,
    ) -> Result<AttemptOutcome, CafleetError> {
        deliver_pending_with(
            conn,
            settings,
            member_id,
            expected_fleet,
            deadline,
            attempted,
            resolve,
            &mut NativeInputHost,
        )
    }

    fn wrapped_line(message: &broker::records::MessageRecord, body: &str) -> String {
        format!(
            "[cafleet msg {} from {} {}] {body} [cafleet] Resume your work if something was still running.",
            message.message_id, message.from_member_id, message.created_at
        )
    }

    fn placement_of(conn: &Connection, member_id: i64, fleet_id: i64) -> Placement {
        broker::get_member(conn, member_id, fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap()
    }

    fn wrapped_attempt(
        fixture: &mut crate::runtime::runner::service::test_support::ServiceFixture,
        deadline: Deadline,
    ) -> Result<AttemptOutcome, CafleetError> {
        let settings = settings_for(&fixture.fixture.conn);
        let placement = placement_of(
            &fixture.fixture.conn,
            fixture.fixture.run.member_id,
            fixture.fixture.run.fleet_id,
        );
        let mut host = DeliveryHost::new(&settings, &placement, deadline.instant());
        let outcome = deliver_pending_with(
            &mut fixture.fixture.conn,
            &settings,
            fixture.fixture.run.member_id,
            Some(fixture.fixture.run.fleet_id),
            deadline,
            &BTreeSet::new(),
            || panic!("wrapped delivery uses its runner, not a multiplexer"),
            &mut host,
        );
        if let Ok(result) = &outcome {
            if result.sent.is_empty() {
                assert_eq!(host.calls, [0, 0, 0]);
            } else {
                assert_eq!(host.calls, [1, 1, 2]);
                assert_eq!(
                    host.retired.len(),
                    result.sent.len(),
                    "actual typed retirement acknowledgments"
                );
            }
        }
        outcome
    }

    struct NoDeliveryHost;
    impl InputCommandHost for NoDeliveryHost {
        fn lock(
            &mut self,
            _: &Settings,
            _: &broker::runtime::RunRecord,
            _: &broker::records::Placement,
            _: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            panic!("retained delivery is resolved before the pane wait")
        }
        fn health(
            &mut self,
            _: &broker::runtime::RunRecord,
            _: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            panic!("retained delivery needs no live health")
        }
        fn exchange(
            &mut self,
            _: &broker::runtime::RunRecord,
            _: &InputControlRequest,
            _: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
            panic!("retained delivery sends no native request")
        }
    }

    struct DeliveryHost {
        inner: NativeInputHost,
        settings: Settings,
        placement: broker::records::Placement,
        until: Instant,
        calls: [usize; 3],
        expiration: Option<u64>,
        retired: Vec<i64>,
        expire_retirement: bool,
        after_retirement: Option<Box<dyn FnMut()>>,
    }
    impl DeliveryHost {
        fn new(
            settings: &Settings,
            placement: &broker::records::Placement,
            until: Instant,
        ) -> Self {
            Self {
                inner: NativeInputHost,
                settings: settings.clone(),
                placement: placement.clone(),
                until,
                calls: [0; 3],
                expiration: None,
                retired: Vec::new(),
                expire_retirement: false,
                after_retirement: None,
            }
        }
        fn held(&self) {
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none(),
                "actual delivery guard remains held"
            );
        }
    }
    impl InputCommandHost for DeliveryHost {
        fn lock(
            &mut self,
            settings: &Settings,
            run: &broker::runtime::RunRecord,
            placement: &broker::records::Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            self.calls[0] += 1;
            assert_eq!(deadline.instant(), self.until);
            assert_eq!(placement, &self.placement);
            let result = self.inner.lock(settings, run, placement, deadline)?;
            result
                .as_ref()
                .expect("actual available pane lock")
                .validate_binding(&settings.database_url, placement)
                .unwrap();
            self.held();
            Ok(result)
        }
        fn health(
            &mut self,
            run: &broker::runtime::RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            self.calls[1] += 1;
            assert_eq!(deadline, self.until);
            self.held();
            self.inner.health(run, deadline)
        }
        fn exchange(
            &mut self,
            run: &broker::runtime::RunRecord,
            request: &InputControlRequest,
            deadline: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
            self.calls[2] += 1;
            assert_eq!(deadline, self.until);
            if matches!(
                request.operation,
                InputControlOperation::SubmitPrompt(_) | InputControlOperation::SubmitExec(_)
            ) {
                assert!(
                    PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                        .unwrap()
                        .is_some(),
                    "a wrapped delivery awaits the runner's submission reply without the pane lock"
                );
            } else {
                self.held();
            }
            assert_eq!(
                serde_json::to_value(&request.identity).unwrap(),
                serde_json::to_value(ControlIdentity::from_run(run)).unwrap()
            );
            if let Some(expiration) = self.expiration {
                assert_eq!(
                    request.expires_at_mono_ns, expiration,
                    "retirement retains the submission expiration"
                );
            } else {
                self.expiration = Some(request.expires_at_mono_ns);
            }
            if let InputControlOperation::Receipt(ReceiptAction::Retire { input_id }) =
                &request.operation
            {
                let independent = crate::db::connect(&self.settings.database_url).unwrap();
                let row = broker::runtime::get_input(&independent, *input_id).unwrap();
                assert_eq!(row.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    row.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert!(
                    preview_stamp(&independent, row.message_id.unwrap()).is_some(),
                    "retirement follows the committed stamp"
                );
                if self.expire_retirement {
                    std::thread::sleep(
                        deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(1),
                    );
                    assert!(Instant::now() >= deadline);
                }
                let reply: InputControlReply = self.inner.exchange(run, request, deadline)?;
                assert!(
                    matches!(reply, InputControlReply::Receipt(ReceiptReply::Retired { input_id: retired }) if retired == *input_id),
                    "actual matching retirement reply"
                );
                self.retired.push(*input_id);
                self.expiration = None;
                if let Some(callback) = &mut self.after_retirement {
                    callback();
                }
                return Ok(reply);
            }
            assert!(matches!(
                request.operation,
                InputControlOperation::SubmitPrompt(_)
            ));
            self.inner.exchange(run, request, deadline)
        }
    }

    fn assert_later_failure_preserves_committed_prefix(later_candidate: bool) {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run = fixture.fixture.run.clone();
                let settings = settings_for(&fixture.fixture.conn);
                let placement = placement_of(&fixture.fixture.conn, run.member_id, run.fleet_id);
                let first = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "private committed prefix λ",
                )
                .unwrap();
                let second = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "private later candidate λ",
                )
                .unwrap();
                let first_line = wrapped_line(&first, &first.text);
                let second_line = wrapped_line(&second, &second.text);
                if later_candidate {
                    fixture.fixture.conn.execute_batch(&format!("CREATE TRIGGER prefix_second_stamp_failure BEFORE UPDATE OF notified_at ON messages WHEN NEW.message_id={} BEGIN SELECT RAISE(ABORT,'owned later preview stamp failure'); END", second.message_id)).unwrap();
                }
                let until = outer.min(Instant::now() + Duration::from_secs(5));
                let armed = Arc::new(AtomicBool::new(false));
                let arm = armed.clone();
                let mut host = DeliveryHost::new(&settings, &placement, until);
                host.after_retirement = Some(Box::new(move || arm.store(true, Ordering::SeqCst)));
                let budgets = Arc::new(Mutex::new(Vec::new()));
                let observed = budgets.clone();
                fixture
                    .fixture
                    .conn
                    .authorizer(Some(move |context: AuthContext<'_>| {
                        let selected = if later_candidate {
                            matches!(
                                context.action,
                                AuthAction::Update {
                                    table_name: "messages",
                                    ..
                                }
                            )
                        } else {
                            matches!(
                                context.action,
                                AuthAction::Read {
                                    table_name: "messages",
                                    column_name: "text"
                                }
                            )
                        };
                        if armed.load(Ordering::SeqCst) && selected {
                            observed
                                .lock()
                                .unwrap()
                                .push(Deadline::inherited_or_new().instant());
                            if !later_candidate {
                                return Authorization::Deny;
                            }
                        }
                        Authorization::Allow
                    }))
                    .unwrap();
                let outcome = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(until),
                    &BTreeSet::new(),
                    || panic!("wrapped prefix regression uses its actual runner"),
                    &mut host,
                );
                let removed = fixture
                    .fixture
                    .conn
                    .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
                let trigger_cleanup = if later_candidate {
                    fixture
                        .fixture
                        .conn
                        .execute_batch("DROP TRIGGER prefix_second_stamp_failure")
                } else {
                    Ok(())
                };
                assert!(
                    removed.is_ok() && trigger_cleanup.is_ok(),
                    "delivery={outcome:?}; authorizer_cleanup={removed:?}; trigger_cleanup={trigger_cleanup:?}"
                );
                let committed =
                    preview_row(&fixture.fixture.conn, run.run_id, first.message_id).unwrap();
                let stamp = preview_stamp(&fixture.fixture.conn, first.message_id)
                    .expect("first notification actually committed");
                assert_eq!(committed.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    committed.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert_eq!(host.retired, vec![committed.input_id]);
                let uncommitted = preview_row(&fixture.fixture.conn, run.run_id, second.message_id);
                assert_eq!(
                    preview_stamp(&fixture.fixture.conn, second.message_id),
                    None
                );
                if later_candidate {
                    assert_eq!(
                        uncommitted.as_ref().unwrap().state,
                        broker::runtime::InputState::Accepted
                    );
                    assert_eq!(host.calls, [1, 2, 3]);
                } else {
                    assert!(uncommitted.is_none());
                    assert_eq!(host.calls, [1, 1, 2]);
                }
                let observed = budgets.lock().unwrap();
                assert!(
                    !observed.is_empty(),
                    "actual later SQL failure boundary reached"
                );
                assert!(
                    observed.iter().all(|deadline| *deadline <= until),
                    "later SQL retains the original inherited budget"
                );
                assert!(
                    Instant::now() < until,
                    "failure is the original SQL cause, before original expiry"
                );
                drop(observed);
                crate::runtime::runner::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                let records = crate::runtime::runner::service::test_support::input_service::input_service_records(fixture);
                assert_eq!(
                    records
                        .iter()
                        .filter(|record| record["line"] == first_line)
                        .count(),
                    1
                );
                assert_eq!(
                    records
                        .iter()
                        .filter(|record| record["line"] == second_line)
                        .count(),
                    usize::from(later_candidate)
                );
                let next = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(outer),
                    &BTreeSet::from([second.message_id]),
                    || panic!("committed history schedules no replay"),
                    &mut NoDeliveryHost,
                )
                .unwrap();
                assert_eq!(next, AttemptOutcome::ended(AttemptEnd::NoPending));
                assert_eq!(
                    preview_row(&fixture.fixture.conn, run.run_id, first.message_id),
                    Some(committed.clone())
                );
                assert_eq!(
                    preview_stamp(&fixture.fixture.conn, first.message_id),
                    Some(stamp)
                );
                let outcome =
                    outcome.expect("later failure preserves the actual committed sent prefix");
                assert_eq!(outcome.sent, vec![first.message_id]);
                let AttemptEnd::Failed(warning) = outcome.end else {
                    panic!("actual later failure retains its cause")
                };
                assert_eq!(warning.member_id, run.member_id);
                if later_candidate {
                    assert_eq!(warning.message_id, second.message_id);
                    assert!(
                        warning.error.contains("owned later preview stamp failure"),
                        "{}",
                        warning.error
                    );
                    assert!(warning.error.contains(&uncommitted.unwrap().operation_key));
                } else {
                    assert_eq!(warning.message_id, first.message_id);
                    assert!(
                        warning
                            .error
                            .contains("access to messages.text is prohibited"),
                        "{}",
                        warning.error
                    );
                    assert!(
                        !warning
                            .error
                            .contains(&format!("message {} was not notified", first.message_id))
                    );
                    assert!(warning.error.contains(&committed.operation_key));
                }
                assert!(!warning.error.contains(&first.text));
                assert!(!warning.error.contains(&second.text));
            },
        );
    }

    #[test]
    fn wrapped_delivery_later_lookup_failure_preserves_the_committed_sent_prefix() {
        assert_later_failure_preserves_committed_prefix(false);
    }

    #[test]
    fn wrapped_delivery_later_stamp_failure_preserves_only_the_committed_sent_prefix() {
        assert_later_failure_preserves_committed_prefix(true);
    }

    #[test]
    fn wrapped_delivery_history_and_conflicts_precede_the_real_held_pane_lock() {
        use broker::runtime::{self as storage, InputDisposition, InputKind, InputState};
        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let settings = settings_for(&fixture.fixture.conn);
                let placement = placement_of(&fixture.fixture.conn, run.member_id, run.fleet_id);
                let message = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "retained preview",
                )
                .unwrap();
                let literal = wrapped_line(&message, "retained preview");
                let original = storage::reserve_input(
                    &mut fixture.fixture.conn,
                    &run.owner(),
                    &storage::NewInput {
                        operation_key: format!("preview:{}:{}", run.run_id, message.message_id),
                        kind: InputKind::Preview,
                        payload_digest: preview_digest(run.run_id, message.message_id, &literal),
                        message_id: Some(message.message_id),
                    },
                )
                .unwrap();
                let _held = PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .unwrap();
                for state in [
                    InputState::InProgress,
                    InputState::Uncertain,
                    InputState::Settled,
                ] {
                    if state == InputState::Uncertain {
                        storage::record_input_state(
                            &mut fixture.fixture.conn,
                            &run.owner(),
                            original.input_id,
                            InputState::InProgress,
                            state,
                        )
                        .unwrap();
                    } else if state == InputState::Settled {
                        storage::settle_input(
                            &mut fixture.fixture.conn,
                            &run.owner(),
                            original.input_id,
                            InputState::Uncertain,
                            InputDisposition::AbandonedUncertain,
                            "owned retained preview",
                        )
                        .unwrap();
                    }
                    let before =
                        storage::get_input(&fixture.fixture.conn, original.input_id).unwrap();
                    let changes = fixture.fixture.conn.total_changes();
                    let result = deliver_pending_with(
                        &mut fixture.fixture.conn,
                        &settings,
                        run.member_id,
                        Some(run.fleet_id),
                        Deadline::until(outer),
                        &BTreeSet::new(),
                        || panic!("retained preview has no mux"),
                        &mut NoDeliveryHost,
                    )
                    .unwrap();
                    assert_eq!(result, AttemptOutcome::ended(AttemptEnd::Held));
                    assert_eq!(fixture.fixture.conn.total_changes(), changes);
                    assert_eq!(
                        storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
                        before
                    );
                    assert_eq!(
                        preview_stamp(&fixture.fixture.conn, message.message_id),
                        None
                    );
                }
                fixture
                .fixture
                .conn
                .execute(
                    "UPDATE messages SET text='conflicting private preview' WHERE message_id=?1",
                    [message.message_id],
                )
                .unwrap();
                let changes = fixture.fixture.conn.total_changes();
                let failure = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(outer),
                    &BTreeSet::new(),
                    || panic!("conflicting preview has no mux"),
                    &mut NoDeliveryHost,
                )
                .unwrap_err();
                assert!(!failure.to_string().contains("conflicting private preview"));
                assert_eq!(fixture.fixture.conn.total_changes(), changes);
                assert!(crate::runtime::runner::service::test_support::input_service::input_service_records(fixture).is_empty());
            },
        );
    }

    #[test]
    fn wrapped_delivery_retirement_expiry_preserves_the_notified_prefix_without_retyping() {
        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    broker::runtime::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                        .unwrap();
                let settings = settings_for(&fixture.fixture.conn);
                let placement = placement_of(&fixture.fixture.conn, run.member_id, run.fleet_id);
                let message = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "private retirement payload λ",
                )
                .unwrap();
                let expected = wrapped_line(&message, &message.text);
                let until = outer.min(Instant::now() + Duration::from_secs(2));
                let mut host = DeliveryHost::new(&settings, &placement, until);
                host.expire_retirement = true;
                let outcome = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(until),
                    &BTreeSet::new(),
                    || panic!("wrapped retirement uses no mux"),
                    &mut host,
                )
                .unwrap();
                assert_eq!(outcome.sent, vec![message.message_id]);
                let AttemptEnd::Failed(warning) = outcome.end else {
                    panic!("expired actual retirement reports its committed outcome")
                };
                let row =
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id).unwrap();
                assert_eq!(row.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    row.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert!(preview_stamp(&fixture.fixture.conn, message.message_id).is_some());
                assert!(warning.error.starts_with(&format!("message {} was notified to member {}; input {} is settled/submitted; receipt retirement failed:", message.message_id, run.member_id, row.input_id)), "{}", warning.error);
                assert!(warning.error.contains("deadline"), "{}", warning.error);
                assert!(warning.error.contains(&format!("run_id={}", run.run_id)));
                assert!(warning.error.contains(&row.operation_key));
                assert!(!warning.error.contains(&message.text));
                assert_eq!(host.calls, [1, 1, 2]);
                assert!(host.retired.is_empty());
                assert_eq!(
                    wrapped_attempt(fixture, Deadline::until(outer))
                        .unwrap()
                        .end,
                    AttemptEnd::NoPending
                );
                crate::runtime::runner::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                crate::runtime::runner::service::test_support::input_service::input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == expected)
                    .count(),
                1
            );
                assert_eq!(
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id),
                    Some(row)
                );
            },
        );
    }

    fn preview_row(
        conn: &Connection,
        run_id: i64,
        message_id: i64,
    ) -> Option<broker::runtime::InputOperation> {
        broker::runtime::find_input(conn, run_id, &format!("preview:{run_id}:{message_id}"))
            .unwrap()
    }

    fn preview_stamp(conn: &Connection, message_id: i64) -> Option<String> {
        conn.query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn preview_digest(run_id: i64, message_id: i64, literal: &str) -> String {
        storage::input_payload_digest(
            run_id,
            &crate::runtime::runner::SubmitInput {
                operation_key: format!("preview:{run_id}:{message_id}"),
                input_kind: InputKind::Preview,
                message_id: Some(message_id),
                literal: literal.into(),
            },
        )
    }

    #[test]
    fn wrapped_delivery_submits_real_preview_and_atomically_settles_the_actual_message() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run = fixture.fixture.run.clone();
                let message = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "first\n\tλ",
                )
                .unwrap();
                let expected = wrapped_line(&message, "first⏎␉λ");
                let deadline = Deadline::until(outer.min(Instant::now() + Duration::from_secs(5)));
                let settings = settings_for(&fixture.fixture.conn);
                let placement = placement_of(&fixture.fixture.conn, run.member_id, run.fleet_id);
                let stamped_under_guard = Arc::new(AtomicBool::new(false));
                let observed = stamped_under_guard.clone();
                fixture
                    .fixture
                    .conn
                    .authorizer(Some(move |context: AuthContext<'_>| {
                        if matches!(
                            context.action,
                            AuthAction::Update {
                                table_name: "messages",
                                ..
                            }
                        ) {
                            assert!(
                                PaneLock::try_acquire(&settings.database_url, &placement)
                                    .unwrap()
                                    .is_none(),
                                "supplied-connection settlement retains the physical guard"
                            );
                            observed.store(true, Ordering::SeqCst);
                        }
                        Authorization::Allow
                    }))
                    .unwrap();
                let outcome = wrapped_attempt(fixture, deadline);
                let removed = fixture
                    .fixture
                    .conn
                    .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
                assert!(
                    outcome.is_ok(),
                    "delivery={outcome:?}; authorizer_cleanup={removed:?}"
                );
                removed.unwrap();
                let outcome = outcome.unwrap();
                assert_eq!(outcome.sent, vec![message.message_id], "{outcome:?}");
                assert!(
                    stamped_under_guard.load(Ordering::SeqCst),
                    "actual stamp uses the supplied connection while holding the borrowed lock"
                );
                assert!(
                    matches!(outcome.end, AttemptEnd::Drained | AttemptEnd::NoPending),
                    "{outcome:?}"
                );
                crate::runtime::runner::service::test_support::input_service::wait_input_service_record(fixture, &expected, outer);
                crate::runtime::runner::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                let row =
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id).unwrap();
                assert_eq!(row.kind, broker::runtime::InputKind::Preview);
                assert_eq!(row.message_id, Some(message.message_id));
                assert_eq!(
                    row.payload_digest,
                    preview_digest(run.run_id, message.message_id, &expected)
                );
                assert_eq!(row.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    row.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert!(preview_stamp(&fixture.fixture.conn, message.message_id).is_some());
                let status: String = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT status_state FROM messages WHERE message_id=?1",
                        [message.message_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(status, "input_required");
                let records = crate::runtime::runner::service::test_support::input_service::input_service_records(fixture);
                assert_eq!(
                    records
                        .iter()
                        .filter(|record| record["line"] == expected)
                        .count(),
                    1
                );
                assert_eq!(
                    wrapped_attempt(fixture, Deadline::until(outer))
                        .unwrap()
                        .end,
                    AttemptEnd::NoPending
                );
                assert_eq!(
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id),
                    Some(row)
                );
            },
        );
    }

    #[test]
    fn wrapped_delivery_failed_stamp_retries_the_same_receipt_without_retyping() {
        crate::runtime::runner::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run = fixture.fixture.run.clone();
                let message = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "retry preview",
                )
                .unwrap();
                let expected = wrapped_line(&message, "retry preview");
                fixture.fixture.conn.execute_batch("CREATE TRIGGER wrapped_stamp_failure BEFORE UPDATE OF notified_at ON messages BEGIN SELECT RAISE(ABORT,'owned wrapped stamp failure'); END").unwrap();
                let first = wrapped_attempt(
                    fixture,
                    Deadline::until(outer.min(Instant::now() + Duration::from_secs(5))),
                );
                let removed = fixture
                    .fixture
                    .conn
                    .execute_batch("DROP TRIGGER wrapped_stamp_failure");
                assert!(
                    first.as_ref().is_err_and(|cause| cause
                        .to_string()
                        .contains("owned wrapped stamp failure")),
                    "first={first:?}; trigger_cleanup={removed:?}"
                );
                removed.unwrap();
                crate::runtime::runner::service::test_support::input_service::wait_input_service_record(fixture, &expected, outer);
                let row =
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id).unwrap();
                assert_eq!(row.state, broker::runtime::InputState::Accepted);
                assert_eq!(
                    preview_stamp(&fixture.fixture.conn, message.message_id),
                    None
                );
                let settings = settings_for(&fixture.fixture.conn);
                let mut independent = crate::db::connect(&settings.database_url).unwrap();
                let outcome = deliver_natively(
                    &mut independent,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(outer),
                    &BTreeSet::new(),
                    || panic!("wrapped retry uses its durable receipt"),
                )
                .unwrap();
                assert_eq!(outcome.sent, vec![message.message_id], "{outcome:?}");
                let settled = preview_row(&independent, run.run_id, message.message_id).unwrap();
                assert!(
                    matches!(outcome.end, AttemptEnd::Drained | AttemptEnd::NoPending),
                    "retry requires actual retirement acknowledgment: {outcome:?}"
                );
                assert_eq!(settled.input_id, row.input_id);
                assert_eq!(settled.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    settled.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert!(preview_stamp(&independent, message.message_id).is_some());
                crate::runtime::runner::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                crate::runtime::runner::service::test_support::input_service::input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == expected)
                    .count(),
                1
            );
                let count: i64 = independent.query_row("SELECT count(*) FROM member_input_operations WHERE run_id=?1 AND kind='preview'", [run.run_id], |row| row.get(0)).unwrap();
                assert_eq!(count, 1);
            },
        );
    }

    #[test]
    fn a_director_snapshot_preserves_its_pending_inbox_without_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "director");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let reply = broker::send_message(&mut conn, worker, director, "reply").unwrap();
        assert!(
            preview_snapshot(
                &mut conn,
                director,
                Some(fleet),
                Deadline::new(),
                &BTreeSet::new()
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn director_delivery_ends_before_resolving_or_locking_a_pane() {
        for backend in ["claude", "codex", "opencode"] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "director");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            conn.execute(
                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                rusqlite::params![backend, director],
            )
            .unwrap();
            let reply = broker::send_message(&mut conn, worker, director, "reply").unwrap();
            let settings = settings_for(&conn);
            let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
            let lock =
                crate::input::lock_path(&settings.database_url, member.placement.as_ref().unwrap())
                    .unwrap();
            let outcome = deliver_natively(
                &mut conn,
                &settings,
                director,
                Some(fleet),
                Deadline::new(),
                &BTreeSet::new(),
                || panic!("Director delivery must finish before multiplexer resolution"),
            )
            .unwrap();
            assert!(outcome.sent.is_empty());
            assert!(matches!(outcome.end, AttemptEnd::NoPending));
            assert!(!lock.exists(), "{backend}: {}", lock.display());
            assert_eq!(preview_stamp(&conn, reply.message_id), None);
            assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
        }
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
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "stamp");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        let settings = settings_for(&conn);
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
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
            Authorization::Allow
        }))
        .unwrap();
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(3200));
        let mux = fake_mux(Rc::new(FinishingRunner {
            deadline,
            submitted,
        }));
        let outcome = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            deadline,
            &BTreeSet::new(),
            || Ok(mux),
        )
        .unwrap();
        assert_eq!(outcome.sent, vec![message.message_id]);
        assert!(deadline.remaining().is_err());
        assert!(
            broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn preview_reads_share_one_snapshot_when_another_connection_changes_placement_and_text() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        let (fleet, director) = create_fleet_fixture(&mut conn, "snapshot");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "original").unwrap();
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
        let (member, message) = preview_snapshot(
            &mut conn,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(member.placement.unwrap().mux_pane_id.as_deref(), Some("%2"));
        assert_eq!(message.text, "original");
        let current: String = conn
            .query_row("SELECT text FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(current, "changed");
    }

    fn fake_mux(runner: Rc<dyn CommandRunner>) -> AnyMultiplexer {
        AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            runner,
            HashMap::from([("TMUX".into(), "test".into())]),
        ))
    }

    #[test]
    fn a_drain_submits_pending_messages_fifo_with_one_escape_and_keeps_full_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "fifo");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let bodies = [
            "雪\n\t\u{1b}! shell".to_owned(),
            "界".repeat(INLINE_BODY_LIMIT + 1),
        ];
        let messages: Vec<_> = bodies
            .iter()
            .map(|body| broker::send_message(&mut conn, director, worker, body).unwrap())
            .collect();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        runner.succeed(5);
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || Ok(fake_mux(runner.clone())),
        )
        .unwrap();
        assert_eq!(
            result.sent,
            messages
                .iter()
                .map(|message| message.message_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(result.end, AttemptEnd::Drained);
        let calls = runner.run_argvs();
        assert_eq!(calls.len(), 5);
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.last().unwrap() == "Escape")
                .count(),
            1
        );
        assert_eq!(calls[1].last().unwrap(), &notification_line(&messages[0]));
        assert_eq!(calls[3].last().unwrap(), &notification_line(&messages[1]));
        assert!(calls[3].last().unwrap().contains("cafleet message show"));
        assert_eq!(
            broker::poll_messages(&conn, worker)
                .unwrap()
                .iter()
                .map(|message| message.text.clone())
                .collect::<Vec<_>>(),
            bodies.into_iter().rev().collect::<Vec<_>>()
        );
        assert!(
            broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_worker_drain_with_both_directions_pending_never_names_the_root_directors_pane() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "both directions");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let reply = broker::send_message(&mut conn, worker, director, "reply").unwrap();
        let assignment = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        runner.succeed(3);
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || Ok(fake_mux(runner.clone())),
        )
        .unwrap();
        assert_eq!(result.sent, vec![assignment.message_id]);
        assert_eq!(result.end, AttemptEnd::Drained);
        let calls = runner.run_argvs();
        assert_eq!(calls.len(), 3);
        for call in &calls {
            assert!(call.iter().any(|arg| arg == "%2"), "{call:?}");
            assert!(!call.iter().any(|arg| arg.contains("%0")), "{call:?}");
        }
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.last().unwrap() == "Escape")
                .count(),
            1
        );
        assert_eq!(preview_stamp(&conn, reply.message_id), None);
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn placement_changes_during_a_real_lock_wait_are_detected_before_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "placement wait");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "pending").unwrap();
        let settings = settings_for(&conn);
        let placement = placement_of(&conn, worker, fleet);
        let guard = PaneLock::try_acquire(&settings.database_url, &placement)
            .unwrap()
            .unwrap();
        let path = conn.path().unwrap().to_owned();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1",
                    [worker],
                )
                .unwrap();
            drop(guard);
        });
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || panic!("changed placement finishes before transport resolution"),
        )
        .unwrap();
        release.join().unwrap();
        assert_eq!(result, AttemptOutcome::ended(AttemptEnd::Held));
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                .unwrap()
                .unwrap(),
            message
        );
    }

    #[test]
    fn self_sends_and_pending_placements_keep_durable_rows_without_resolving_transport() {
        for (target_self, pane) in [(true, Some("%2")), (false, None)] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "held preview");
            let worker = register(&mut conn, fleet, "worker", pane);
            let sender = if target_self { worker } else { director };
            let message = broker::send_message(&mut conn, sender, worker, "durable").unwrap();
            let settings = settings_for(&conn);
            let result = deliver_natively(
                &mut conn,
                &settings,
                worker,
                Some(fleet),
                Deadline::new(),
                &BTreeSet::new(),
                || panic!("ineligible preview has no transport effects"),
            )
            .unwrap();
            assert_eq!(
                result.end,
                if target_self {
                    AttemptEnd::NoPending
                } else {
                    AttemptEnd::Held
                }
            );
            assert_eq!(broker::poll_messages(&conn, worker).unwrap(), [message]);
            assert!(result.sent.is_empty());
        }
    }

    fn expected_failure(member_id: i64, message_id: i64, cause: &str) -> AttemptEnd {
        AttemptEnd::Failed(crate::receive::NotificationError {
            member_id,
            message_id,
            error: format!(
                "message {message_id} was not notified to member {member_id}: {cause}; the next send to member {member_id} or the next message wait retries it."
            ),
        })
    }

    #[test]
    fn delivery_skips_attempted_message_ids_and_stamps_only_the_later_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "exclusions");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let first = broker::send_message(&mut conn, director, worker, "failed earlier").unwrap();
        let next = broker::send_message(&mut conn, director, worker, "next message").unwrap();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        runner.succeed(3);
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::from([first.message_id]),
            || Ok(fake_mux(runner.clone())),
        )
        .unwrap();
        assert_eq!(result.sent, vec![next.message_id]);
        assert_eq!(result.end, AttemptEnd::Drained);
        let literals: Vec<_> = runner
            .run_argvs()
            .into_iter()
            .filter(|argv| argv.iter().any(|arg| arg == "-l"))
            .collect();
        assert_eq!(literals.len(), 1);
        assert!(
            literals[0]
                .last()
                .unwrap()
                .contains(&format!("[cafleet msg {} from ", next.message_id))
        );
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                .unwrap()
                .unwrap()
                .message_id,
            first.message_id
        );
    }

    #[test]
    fn resolution_and_availability_failures_use_the_exact_not_notified_diagnostic() {
        for resolution in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "multiplexer failure");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
            let settings = settings_for(&conn);
            let result = deliver_natively(
                &mut conn,
                &settings,
                worker,
                Some(fleet),
                Deadline::new(),
                &BTreeSet::new(),
                || {
                    if resolution {
                        Err(MultiplexerError::new("resolution unavailable"))
                    } else {
                        Ok(fake_mux(
                            crate::multiplexer::test_support::FakeRunner::without_binaries(),
                        ))
                    }
                },
            )
            .unwrap();
            assert!(result.sent.is_empty());
            assert_eq!(
                result.end,
                expected_failure(
                    worker,
                    message.message_id,
                    if resolution {
                        "resolution unavailable"
                    } else {
                        "tmux binary not found on PATH"
                    }
                )
            );
            assert_eq!(
                broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                    .unwrap()
                    .unwrap()
                    .message_id,
                message.message_id
            );
        }
    }

    #[test]
    fn busy_lock_and_lock_acquisition_errors_use_the_exact_not_notified_diagnostic() {
        for busy in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "lock failure");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
            let settings = settings_for(&conn);
            let placement = placement_of(&conn, worker, fleet);
            let guard;
            let cause = if busy {
                guard = PaneLock::try_acquire(&settings.database_url, &placement).unwrap();
                assert!(guard.is_some());
                "pane input remained busy".to_string()
            } else {
                let path = crate::input::lock_path(&settings.database_url, &placement).unwrap();
                std::fs::write(path.parent().unwrap(), "occupied").unwrap();
                let error = PaneLock::try_acquire(&settings.database_url, &placement)
                    .err()
                    .expect("a regular file cannot serve as the lock directory");
                guard = None;
                format!("pane input: {error}")
            };
            let result = deliver_natively(
                &mut conn,
                &settings,
                worker,
                Some(fleet),
                Deadline::until(Instant::now() + Duration::from_millis(500)),
                &BTreeSet::new(),
                || panic!("a failed lock finishes before multiplexer resolution"),
            )
            .unwrap();
            assert_eq!(
                result.end,
                expected_failure(worker, message.message_id, &cause)
            );
            assert!(result.sent.is_empty());
            drop(guard);
        }
    }

    #[test]
    fn an_insufficient_first_line_budget_reports_the_exact_not_notified_diagnostic() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "budget");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::until(Instant::now() + Duration::from_millis(500)),
            &BTreeSet::new(),
            || Ok(fake_mux(runner.clone())),
        )
        .unwrap();
        assert_eq!(
            result.end,
            expected_failure(
                worker,
                message.message_id,
                "insufficient pane input budget for literal input, settle and Enter"
            )
        );
        assert!(runner.events().is_empty());
    }

    #[test]
    fn every_transport_stage_uses_the_same_not_notified_template_and_preserves_the_cause() {
        for successes in 0..=2 {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "transport");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
            let settings = settings_for(&conn);
            let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
            let expected = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
            for fixture in [&runner, &expected] {
                for _ in 0..successes {
                    fixture.respond(Ok(String::new()));
                }
                fixture.respond(Err(RunError::Failed {
                    stderr: "distinct transport failure".into(),
                }));
            }
            let cause = fake_mux(expected)
                .send_prompt("%2", &notification_line(&message), Lead::Escape)
                .unwrap_err()
                .to_string();
            let result = deliver_natively(
                &mut conn,
                &settings,
                worker,
                Some(fleet),
                Deadline::new(),
                &BTreeSet::new(),
                || Ok(fake_mux(runner.clone())),
            )
            .unwrap();
            assert_eq!(
                result.end,
                expected_failure(worker, message.message_id, &cause)
            );
            assert!(result.sent.is_empty());
            assert_eq!(runner.run_argvs().len(), successes + 1);
            assert_eq!(
                broker::oldest_pending_preview(&conn, worker, &BTreeSet::new())
                    .unwrap()
                    .unwrap()
                    .message_id,
                message.message_id
            );
        }
    }

    #[test]
    fn a_failure_after_one_submission_keeps_the_sent_prefix_and_names_the_current_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "partial drain");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let first = broker::send_message(&mut conn, director, worker, "first").unwrap();
        let next = broker::send_message(&mut conn, director, worker, "next").unwrap();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        for _ in 0..3 {
            runner.respond(Ok(String::new()));
        }
        runner.respond(Err(RunError::Failed {
            stderr: "next literal failed".into(),
        }));
        let expected = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        expected.respond(Err(RunError::Failed {
            stderr: "next literal failed".into(),
        }));
        let cause = fake_mux(expected)
            .send_prompt("%2", &notification_line(&next), Lead::Direct)
            .unwrap_err()
            .to_string();
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || Ok(fake_mux(runner.clone())),
        )
        .unwrap();
        assert_eq!(result.sent, vec![first.message_id]);
        assert_eq!(
            result.end,
            expected_failure(worker, next.message_id, &cause)
        );
        assert_eq!(runner.run_argvs().len(), 4);
    }

    #[test]
    fn a_fatal_stamp_error_retains_its_storage_cause() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "fatal stamp");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_stamp BEFORE UPDATE OF notified_at ON messages
            BEGIN SELECT RAISE(ABORT, 'injected stamp corruption'); END;",
        )
        .unwrap();
        let settings = settings_for(&conn);
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        runner.succeed(3);
        let error = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || Ok(fake_mux(runner)),
        )
        .unwrap_err();
        assert!(
            matches!(error, CafleetError::App(ref cause) if cause.contains("injected stamp corruption"))
        );
        assert_eq!(preview_stamp(&conn, message.message_id), None);
    }

    #[test]
    fn a_fatal_under_lock_snapshot_error_retains_its_storage_cause() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "fatal snapshot");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        let settings = settings_for(&conn);
        let path = conn.path().unwrap().to_string();
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        let error = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
            || {
                Connection::open(path)
                    .unwrap()
                    .execute(
                        "UPDATE members SET description=zeroblob(1) WHERE member_id=?1",
                        [worker],
                    )
                    .unwrap();
                Ok(fake_mux(runner.clone()))
            },
        )
        .unwrap_err();
        assert!(
            error.message().contains("Invalid column type Blob"),
            "{error}"
        );
        assert!(runner.events().is_empty());
    }

    #[test]
    fn recipient_changes_after_resolution_are_revalidated_before_any_input() {
        for change in ["departure", "promotion", "placement"] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "revalidation");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
            let settings = settings_for(&conn);
            let path = conn.path().unwrap().to_string();
            let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
            let result = deliver_natively(
                &mut conn, &settings, worker, Some(fleet), Deadline::new(), &BTreeSet::new(),
                || {
                    let other = Connection::open(path).unwrap();
                    match change {
                        "departure" => { other.execute("UPDATE members SET status='deregistered' WHERE member_id=?1", [worker]).unwrap(); }
                        "promotion" => { other.execute("UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2", rusqlite::params![worker, fleet]).unwrap(); }
                        "placement" => { other.execute("UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1", [worker]).unwrap(); }
                        _ => unreachable!(),
                    }
                    Ok(fake_mux(runner.clone()))
                },
            ).unwrap();
            assert!(result.sent.is_empty());
            assert_eq!(
                result.end,
                if change == "placement" {
                    AttemptEnd::Held
                } else {
                    AttemptEnd::NoPending
                }
            );
            assert!(runner.events().is_empty());
            assert_eq!(preview_stamp(&conn, message.message_id), None);
        }
    }

    #[test]
    fn under_lock_snapshot_contention_returns_a_retryable_diagnostic_for_the_current_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "snapshot contention");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "assignment").unwrap();
        let settings = settings_for(&conn);
        let blocker = Connection::open(conn.path().unwrap()).unwrap();
        let runner = crate::multiplexer::test_support::FakeRunner::with_binary("tmux");
        let result = deliver_natively(
            &mut conn,
            &settings,
            worker,
            Some(fleet),
            Deadline::until(Instant::now() + Duration::from_millis(500)),
            &BTreeSet::new(),
            || {
                blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
                Ok(fake_mux(runner.clone()))
            },
        )
        .unwrap();
        assert!(result.sent.is_empty());
        let AttemptEnd::Failed(error) = result.end else {
            panic!("snapshot contention leaves a per-message retry diagnostic");
        };
        assert_eq!(
            (error.member_id, error.message_id),
            (worker, message.message_id)
        );
        let database = expected_failure(
            worker,
            message.message_id,
            "database error: database is locked",
        );
        let deadline = expected_failure(
            worker,
            message.message_id,
            "operation exceeded its deadline",
        );
        assert!(
            AttemptEnd::Failed(error.clone()) == database || AttemptEnd::Failed(error) == deadline
        );
        assert!(runner.events().is_empty());
        blocker.execute_batch("ROLLBACK").unwrap();
    }

    use crate::broker::records::MessageRecord;

    const TS: &str = MessageRecord::FIXTURE_STAMP;
    const CLAUSES: &str = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

    fn record(text: &str) -> MessageRecord {
        MessageRecord::fixture(57, 15, 16, text)
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
        format!("Body is {stored_chars} characters; run cafleet message show 57 --json to read it.")
    }

    #[test]
    fn the_line_matches_the_specified_example() {
        assert_eq!(
            notification_line(&record("ready (doc)\nsee the reviewer notes")),
            format!("[cafleet msg 57 from 15 {TS}] ready (doc)⏎see the reviewer notes {CLAUSES}")
        );
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

    mod external_finite_wait {
        use super::*;
        use crate::broker::runtime::RunRecord;
        use crate::runtime::runner::service::test_support::ServiceFixture;
        use crate::runtime::runner::service::test_support::input_service::{
            FORM_LINE, input_service_barrier, input_service_composer, input_service_records,
            manual, wait_input_service_record, with_detecting_input_service,
        };
        use crate::runtime::runner::*;

        type Result<T> = std::result::Result<T, CafleetError>;

        #[test]
        fn external_finite_wait_retries_held_preview_after_actual_composer_recovery() {
            use crate::receive::{
                AttemptEnd, AttemptOutcome, BrokerReceiver, ReceiveIo, ReceiveSnapshot, SystemClock,
            };
            use std::collections::BTreeSet;

            /// Consumes the runtime-event notifications the detected interaction sends the
            /// Director, so the finite wait observes only the preview retry.
            fn acknowledge_runtime_events(fixture: &mut ServiceFixture, run: &RunRecord) {
                for pending in
                    crate::broker::poll_messages(&fixture.fixture.conn, run.director_member_id)
                        .unwrap()
                {
                    assert_eq!(pending.from_member_id, run.member_id);
                    let event: serde_json::Value = serde_json::from_str(&pending.text).unwrap();
                    assert_eq!(event["run_id"], run.run_id, "{event}");
                    crate::broker::ack_message(&mut fixture.fixture.conn, pending.message_id)
                        .unwrap();
                }
            }

            struct ObservedReceiver<'a, 'b> {
                receiver: BrokerReceiver<'a>,
                fixture: &'b mut ServiceFixture,
                run: RunRecord,
                until: Instant,
                message_id: i64,
                key: String,
                line: String,
                outcomes: Vec<AttemptOutcome>,
            }
            impl ObservedReceiver<'_, '_> {
                fn untouched(&self) {
                    assert!(
                        storage::find_input(&self.fixture.fixture.conn, self.run.run_id, &self.key)
                            .unwrap()
                            .is_none()
                    );
                    let stamp: Option<String> = self
                        .fixture
                        .fixture
                        .conn
                        .query_row(
                            "SELECT notified_at FROM messages WHERE message_id=?1",
                            [self.message_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert!(stamp.is_none());
                    assert!(
                        input_service_records(self.fixture)
                            .iter()
                            .all(|record| record["line"].as_str() != Some(self.line.as_str()))
                    );
                }
            }
            impl ReceiveIo for ObservedReceiver<'_, '_> {
                fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot> {
                    self.receiver.check(deadline)
                }
                fn attempt(
                    &mut self,
                    member_id: i64,
                    deadline: Deadline,
                    attempted: &BTreeSet<i64>,
                ) -> Result<AttemptOutcome> {
                    assert_eq!(member_id, self.run.member_id);
                    assert!(!attempted.contains(&self.message_id));
                    let outcome = self.receiver.attempt(member_id, deadline, attempted)?;
                    if self.outcomes.is_empty() {
                        assert_eq!(
                            outcome.end,
                            AttemptEnd::Held,
                            "actual first outcome={outcome:?}"
                        );
                        assert!(outcome.sent.is_empty());
                        self.untouched();
                        let until = self.until.min(deadline.instant());
                        input_service_barrier(self.fixture, until);
                        let health = input_service_composer(self.fixture, until);
                        health.validate(&self.run).unwrap();
                        let observation = health.observation.unwrap();
                        assert_eq!(observation.eligibility, InputEligibility::ComposerEmpty);
                        assert_eq!(observation.visibility, Visibility::NotVisible);
                        assert!(!observation.degraded);
                        assert_eq!(observation.pending_count, 0);
                        assert!(!observation.pending_evidence);
                        acknowledge_runtime_events(self.fixture, &self.run);
                        self.untouched();
                        assert_eq!(
                            input_service_records(self.fixture)
                                .iter()
                                .map(|record| record["line"].as_str().unwrap().to_owned())
                                .collect::<Vec<_>>(),
                            vec!["barrier"]
                        );
                    }
                    self.outcomes.push(outcome.clone());
                    Ok(outcome)
                }
            }

            with_detecting_input_service(|fixture, until| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let settings = Settings::for_database(
                    std::path::Path::new(fixture.fixture.conn.path().unwrap()),
                    None,
                );
                manual(fixture, format!("{FORM_LINE}\r").as_bytes(), until);
                let mut previous = None;
                loop {
                    assert!(
                        Instant::now() < until,
                        "actual stable Hold setup within original fixture cap"
                    );
                    let health = NativeInputHost.health(&run, until).unwrap();
                    health.validate(&run).unwrap();
                    let observation = health.observation.as_ref().unwrap();
                    if observation.eligibility == InputEligibility::Hold(GateReason::Interaction)
                        && !observation.degraded
                        && observation.pending_count == 0
                        && !observation.pending_evidence
                    {
                        if previous == Some(health.output_seq) {
                            break;
                        }
                        previous = Some(health.output_seq);
                    } else {
                        previous = None;
                    }
                    std::thread::sleep(
                        Duration::from_millis(2)
                            .min(until.saturating_duration_since(Instant::now())),
                    );
                }
                acknowledge_runtime_events(fixture, &run);
                let message = crate::broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "actual external Held retry fixture",
                )
                .unwrap();
                assert!(
                    crate::broker::poll_messages(&fixture.fixture.conn, run.director_member_id)
                        .unwrap()
                        .is_empty()
                );
                assert!(input_service_records(fixture).is_empty());
                let line = format!(
                    "[cafleet msg {} from {} {}] {} [cafleet] Resume your work if something was still running.",
                    message.message_id, message.from_member_id, message.created_at, message.text,
                );
                let key = format!("preview:{}:{}", run.run_id, message.message_id);
                let timeout = Duration::from_secs(2);
                assert!(
                    until.saturating_duration_since(Instant::now()) > timeout,
                    "original fixture cap contains the finite retry and cleanup"
                );
                let mut conn = crate::db::connect(&settings.database_url).unwrap();
                let outcomes = with_database_budget(&mut conn, Deadline::until(until), |conn| {
                    let mut io = ObservedReceiver {
                        receiver: BrokerReceiver::new(conn, &settings, run.director_member_id),
                        fixture: &mut *fixture,
                        run: run.clone(),
                        until,
                        message_id: message.message_id,
                        key: key.clone(),
                        line: line.clone(),
                        outcomes: Vec::new(),
                    };
                    let waited = crate::receive::wait(&SystemClock, &mut io, timeout);
                    assert!(
                        waited.is_ok(),
                        "receive_error={:?}; actual_attempts={:?}",
                        waited.as_ref().err(),
                        io.outcomes
                    );
                    assert!(waited.unwrap().is_empty());
                    Ok(io.outcomes)
                })
                .unwrap();
                assert_eq!(outcomes.len(), 2);
                assert_eq!(outcomes[0].end, AttemptEnd::Held);
                assert!(outcomes[0].sent.is_empty());
                assert_eq!(outcomes[1].end, AttemptEnd::NoPending);
                assert_eq!(outcomes[1].sent, vec![message.message_id]);
                let input = storage::find_input(&fixture.fixture.conn, run.run_id, &key)
                    .unwrap()
                    .unwrap();
                assert_eq!(input.kind, InputKind::Preview);
                assert_eq!(input.message_id, Some(message.message_id));
                assert_eq!(input.state, InputState::Settled);
                assert_eq!(input.disposition, Some(InputDisposition::Submitted));
                assert_eq!(input.evidence_ref, None);
                let stamp: Option<String> = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT notified_at FROM messages WHERE message_id=?1",
                        [message.message_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert!(stamp.is_some());
                wait_input_service_record(fixture, &line, until);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .map(|record| record["line"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    vec!["barrier", line.as_str()]
                );
                let records = input_service_records(fixture);
                let native = records
                    .iter()
                    .find(|record| record["line"] == line)
                    .unwrap();
                assert!(
                    native["inputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row["input_id"] == input.input_id)
                );
            });
        }
    }
}
