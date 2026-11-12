use crate::broker;
use crate::broker::MemberKind;
use crate::broker::MessageRecord;
use crate::broker::runtime as storage;
use crate::broker::runtime::InputDisposition;
use crate::broker::runtime::InputKind;
use crate::broker::runtime::InputState;
use crate::broker::runtime::PermissionReceiptIdentity;
use crate::config::Settings;
use crate::db::complete_database_budget;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::runtime::InputCommandHost;
use crate::runtime::InputContext;
use crate::runtime::NativeInputHost;
use crate::runtime::PreparedSubmission;
use crate::runtime::RealInputDeadlineClock;
use crate::runtime::SubmissionStatus;
use crate::runtime::SubmitInput;
use crate::runtime::find_submission;
use crate::runtime::submit_wrapped_input_under_pane_lock_with_host;
use crate::runtime::validate_submission_target;
use crate::time::Clock;
use crate::time::Deadline;
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::time::Duration;

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

pub(crate) fn fallback_preview_line(canonical: &str) -> String {
    format!("{canonical} {DISMISSED_PROMPT_NOTE}")
}

pub(crate) fn canonical_preview_line(message: &broker::MessageRecord) -> String {
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

type PreviewSnapshot = (broker::MemberRecord, broker::MessageRecord);

fn eligible_member(
    conn: &Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
) -> Result<Option<broker::MemberRecord>, CafleetError> {
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

fn deliver_pending_with(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
    host: &mut impl InputCommandHost,
) -> Result<AttemptOutcome, CafleetError> {
    let deadline = Deadline::until(crate::db::bounded_by_inherited(conn, deadline.instant()));
    with_database_budget(conn, deadline, |conn| {
        let ended = |end| Ok(AttemptOutcome::ended(end));
        let Some((member, initial)) =
            preview_snapshot(conn, member_id, expected_fleet, deadline, attempted)?
        else {
            return ended(AttemptEnd::NoPending);
        };
        let run = member
            .runtime
            .as_ref()
            .expect("every ordinary member records its run")
            .run
            .clone();
        if member.placement.mux_pane_id.is_none() {
            return ended(AttemptEnd::Held);
        }
        deliver_wrapped_pending(
            conn,
            settings,
            (member, initial),
            run,
            deadline,
            attempted,
            host,
        )
    })
}

fn deliver_wrapped_pending(
    conn: &mut Connection,
    settings: &Settings,
    initial: PreviewSnapshot,
    run: crate::broker::runtime::RunRecord,
    deadline: Deadline,
    attempted: &BTreeSet<i64>,
    host: &mut impl InputCommandHost,
) -> Result<AttemptOutcome, CafleetError> {
    let (member, mut message) = initial;
    let placement = &member.placement.clone();
    let identity = PermissionReceiptIdentity::for_run(&run);
    enum Stage {
        Submission,
        Retirement,
        NextPreview,
    }

    let mut guard = None;
    let mut sent = Vec::new();
    let mut held_reason = None;
    let mut skipped = attempted.clone();
    let mut operation_key = format!("preview:{}:{}", run.run_id, message.message_id);
    let mut known_input_id = None;
    let mut stage = Stage::Submission;
    let result = (|| {
        let end = 'drain: loop {
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
            let accepted = 'submit: {
                if retained.as_ref().is_some_and(|input| {
                    input.state == InputState::Ready && input.deferred_at.is_some()
                }) {
                    break 'submit None;
                }
                if guard.is_none() {
                    guard = host.lock(settings, &run, placement, deadline)?;
                    if guard.is_none() {
                        break 'drain AttemptEnd::Failed(notification_error(
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
                        match result.status {
                            SubmissionStatus::Accepted => {}
                            SubmissionStatus::Queued => break 'submit None,
                            SubmissionStatus::Held => {
                                held_reason = result.reason;
                                break 'drain AttemptEnd::Held;
                            }
                            SubmissionStatus::Rejected | SubmissionStatus::Uncertain => {
                                break 'drain AttemptEnd::Held;
                            }
                        }
                        result
                            .input_id
                            .expect("accepted Preview has its validated durable input ID")
                    }
                };
                Some(input_id)
            };
            if let Some(input_id) = accepted {
                let held = guard.as_ref().expect("wrapped delivery owns its pane lock");
                known_input_id = Some(input_id);
                let settled = complete_database_budget(conn, deadline, |conn| {
                    validate_submission_target(conn, settings, &run, placement)?;
                    storage::settle_preview(conn, &run.owner(), input_id)
                })
                .map_err(|source| {
                    CafleetError::from(
                        InputContext {
                            run_id: Some(run.run_id),
                            interaction_id: None,
                            input_id: Some(input_id),
                            operation_key: Some(request.operation_key.clone()),
                        }
                        .failure(source),
                    )
                })?;
                sent.push(message.message_id);
                stage = Stage::Retirement;
                deadline.remaining()?;
                held.validate_binding(&settings.database_url, placement)?;
                with_database_budget(conn, deadline, |conn| {
                    validate_submission_target(conn, settings, &run, placement)
                })?;
                let retirement = attempt.retirement_request(&run, settled.input_id)?;
                host.exchange(&run, &retirement, deadline.instant())?;
                with_database_budget(conn, deadline, |conn| {
                    validate_submission_target(conn, settings, &run, placement)
                })?;
            } else {
                skipped.insert(message.message_id);
            }
            deadline.remaining()?;
            stage = Stage::NextPreview;
            let Some((current, next)) = preview_snapshot(
                conn,
                member.member_id,
                Some(member.fleet_id),
                deadline,
                &skipped,
            )?
            else {
                break AttemptEnd::NoPending;
            };
            if current.placement != member.placement
                || current
                    .runtime
                    .as_ref()
                    .map(|runtime| crate::runtime::stable_run_identity(&runtime.run))
                    != Some(crate::runtime::stable_run_identity(&run))
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
            let cause = CafleetError::from(
                InputContext {
                    run_id: Some(run.run_id),
                    interaction_id: None,
                    input_id: known_input_id,
                    operation_key: Some(operation_key),
                }
                .failure(source),
            );
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
    Ok(AttemptOutcome {
        sent,
        end,
        held_reason,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingPreview {
    pub(crate) member_id: i64,
    pub(crate) message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveSnapshot {
    pub(crate) messages: Vec<MessageRecord>,
    pub(crate) pending: Vec<PendingPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationError {
    pub(crate) member_id: i64,
    pub(crate) message_id: i64,
    pub(crate) error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptOutcome {
    pub(crate) sent: Vec<i64>,
    pub(crate) end: AttemptEnd,
    /// The recipient runner's reason when it held the preview without accepting it.
    pub(crate) held_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptEnd {
    NoPending,
    Held,
    Failed(NotificationError),
}

impl AttemptOutcome {
    pub(crate) fn ended(end: AttemptEnd) -> Self {
        Self {
            sent: Vec::new(),
            end,
            held_reason: None,
        }
    }
}

pub(crate) trait ReceiveIo {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError>;
    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
        attempted: &BTreeSet<i64>,
    ) -> Result<AttemptOutcome, CafleetError>;
}

pub(crate) fn wait<C: Clock, I: ReceiveIo>(
    clock: &C,
    io: &mut I,
    timeout: Duration,
) -> Result<Vec<MessageRecord>, CafleetError> {
    let started = clock.now();
    let expires = started + timeout;
    let mut next_check = started;
    let mut last_interval = None;
    let mut cursor = None;
    let mut messages = Vec::new();
    let mut attempted = BTreeSet::new();
    loop {
        let now = clock.now();
        if now >= expires {
            break;
        }
        if now < next_check {
            clock.sleep(next_check.min(expires).duration_since(now));
            continue;
        }
        let snapshot = match io.check(Deadline::until(expires.min(now + Duration::from_secs(5)))) {
            Ok(snapshot) => snapshot,
            Err(CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded) => {
                let now = clock.now();
                if now >= expires {
                    break;
                }
                next_check =
                    started + Duration::from_secs(now.duration_since(started).as_secs() + 1);
                continue;
            }
            Err(error) => return Err(error),
        };
        messages = snapshot.messages;
        let now = clock.now();
        if !messages.is_empty() || now >= expires {
            break;
        }
        let interval = now.duration_since(started).as_secs();
        let selected = if last_interval == Some(interval) {
            None
        } else {
            let eligible = snapshot
                .pending
                .iter()
                .filter(|preview| !attempted.contains(&preview.message_id));
            eligible
                .clone()
                .filter(|preview| cursor.is_none_or(|cursor| preview.member_id > cursor))
                .min_by_key(|preview| preview.member_id)
                .or_else(|| eligible.min_by_key(|preview| preview.member_id))
        };
        if let Some(preview) = selected {
            let member = preview.member_id;
            cursor = Some(member);
            last_interval = Some(interval);
            let outcome =
                match io.attempt(
                    member,
                    Deadline::until(now + Duration::from_secs(5)),
                    &attempted,
                ) {
                    Ok(outcome) => outcome,
                    Err(
                        error @ (CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded),
                    ) => AttemptOutcome::ended(AttemptEnd::Failed(
                        crate::delivery::notification_error(member, preview.message_id, error),
                    )),
                    Err(error) => return Err(error),
                };
            if let AttemptEnd::Failed(error) = outcome.end {
                assert_eq!(
                    error.member_id, member,
                    "the owned attempt reports its selected member"
                );
                assert!(
                    attempted.insert(error.message_id),
                    "one invocation attempts each failed message at most once"
                );
                eprintln!("{}", error.error);
            }
            next_check = clock.now();
        } else {
            next_check = started + Duration::from_secs(interval + 1);
        }
    }
    Ok(messages)
}

pub(crate) struct BrokerReceiver<'a> {
    conn: &'a mut Connection,
    settings: &'a Settings,
    member_id: i64,
    fleet_id: Option<i64>,
}

impl<'a> BrokerReceiver<'a> {
    pub(crate) fn new(conn: &'a mut Connection, settings: &'a Settings, member_id: i64) -> Self {
        Self {
            conn,
            settings,
            member_id,
            fleet_id: None,
        }
    }
}

fn subject_fleet(
    conn: &Connection,
    member_id: i64,
    expected: Option<i64>,
) -> Result<i64, CafleetError> {
    let fleet = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    broker::members::require_pinned_fleet(member_id, fleet, expected, "receive")?;
    broker::fleets::require_live_fleet(conn, fleet)?;
    Ok(fleet)
}

impl ReceiveIo for BrokerReceiver<'_> {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError> {
        let (snapshot, fleet) = crate::db::with_database_budget(self.conn, deadline, |conn| {
            let tx = conn.transaction()?;
            let fleet = subject_fleet(&tx, self.member_id, self.fleet_id)?;
            let messages = broker::poll_messages(&tx, self.member_id)?;
            let pending = broker::messaging::pending_previews(&tx, fleet, self.member_id)?
                .into_iter()
                .map(|(member_id, message_id)| PendingPreview {
                    member_id,
                    message_id,
                })
                .collect();
            tx.commit()?;
            Ok((ReceiveSnapshot { messages, pending }, fleet))
        })?;
        self.fleet_id = Some(fleet);
        Ok(snapshot)
    }

    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
        attempted: &BTreeSet<i64>,
    ) -> Result<AttemptOutcome, CafleetError> {
        let fleet = crate::db::with_database_budget(self.conn, deadline, |conn| {
            subject_fleet(conn, self.member_id, self.fleet_id)
        })?;
        crate::delivery::deliver_pending(
            self.conn,
            self.settings,
            member_id,
            Some(fleet),
            deadline,
            attempted,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::delivery::AttemptEnd;
    use crate::delivery::*;
    use crate::pane_lock::PaneLock;
    use crate::runtime::ControlIdentity;
    use crate::runtime::HealthSnapshot;
    use crate::runtime::InputCommandHost;
    use crate::runtime::InputControlOperation;
    use crate::runtime::InputControlReply;
    use crate::runtime::InputControlRequest;
    use crate::runtime::NativeInputHost;
    use crate::runtime::ReceiptAction;
    use crate::runtime::ReceiptReply;
    use crate::runtime::input_client::test_support::NoLiveHost;
    use std::collections::BTreeSet;
    use std::time::Duration;
    use std::time::Instant;

    fn settings_for(conn: &Connection) -> Settings {
        Settings::for_database(std::path::Path::new(conn.path().unwrap()))
    }

    fn wrapped_line(message: &crate::broker::MessageRecord, body: &str) -> String {
        format!(
            "[cafleet msg {} from {} {}] {body} [cafleet] Resume your work if something was still running.",
            message.message_id, message.from_member_id, message.created_at
        )
    }

    fn placement_of(conn: &Connection, member_id: i64, fleet_id: i64) -> broker::Placement {
        broker::get_member(conn, member_id, fleet_id)
            .unwrap()
            .unwrap()
            .placement
    }

    fn wrapped_attempt(
        fixture: &mut crate::runtime::service::test_support::ServiceFixture,
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

    struct DeliveryHost {
        inner: NativeInputHost,
        settings: Settings,
        placement: crate::broker::Placement,
        until: Instant,
        calls: [usize; 3],
        expiration: Option<u64>,
        retired: Vec<i64>,
        expire_retirement: bool,
        after_retirement: Option<Box<dyn FnMut()>>,
        runner_held: Option<&'static str>,
        take_lock_after_reply: bool,
        taken: Option<PaneLock>,
    }
    impl DeliveryHost {
        fn new(settings: &Settings, placement: &crate::broker::Placement, until: Instant) -> Self {
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
                runner_held: None,
                take_lock_after_reply: false,
                taken: None,
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
            placement: &crate::broker::Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, crate::error::CafleetError> {
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
        ) -> std::result::Result<HealthSnapshot, crate::error::CafleetError> {
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
        ) -> std::result::Result<InputControlReply, crate::error::CafleetError> {
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
            if let Some(reason) = self.runner_held {
                return Ok(InputControlReply::Submission(
                    crate::runtime::SubmissionResult::held(reason),
                ));
            }
            let reply = self.inner.exchange(run, request, deadline)?;
            if self.take_lock_after_reply {
                self.taken =
                    PaneLock::try_acquire(&self.settings.database_url, &self.placement).unwrap();
                assert!(
                    self.taken.is_some(),
                    "another writer takes the released lock"
                );
            }
            Ok(reply)
        }
    }

    #[test]
    fn wrapped_send_is_submitted_and_stamped_when_another_writer_takes_the_released_lock() {
        crate::runtime::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run = fixture.fixture.run.clone();
                let settings = settings_for(&fixture.fixture.conn);
                let placement = placement_of(&fixture.fixture.conn, run.member_id, run.fleet_id);
                let message = broker::send_message(
                    &mut fixture.fixture.conn,
                    run.director_member_id,
                    run.member_id,
                    "held by the runner, then submitted before the lock is taken",
                )
                .unwrap();
                let until = outer.min(Instant::now() + Duration::from_secs(1));
                let mut held = DeliveryHost::new(&settings, &placement, until);
                held.runner_held = Some("observer_unavailable");
                let outcome = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(until),
                    &BTreeSet::new(),
                    &mut held,
                )
                .unwrap();
                assert_eq!(outcome.end, AttemptEnd::Held);
                assert_eq!(outcome.held_reason.as_deref(), Some("observer_unavailable"));
                assert!(outcome.sent.is_empty());
                let until = outer.min(Instant::now() + Duration::from_secs(1));
                let mut host = DeliveryHost::new(&settings, &placement, until);
                host.take_lock_after_reply = true;
                let outcome = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(until),
                    &BTreeSet::new(),
                    &mut host,
                )
                .expect("a submission the runner accepted is a delivered send");
                assert_eq!(outcome.sent, vec![message.message_id]);
                assert!(
                    preview_stamp(&fixture.fixture.conn, message.message_id).is_some(),
                    "the submitted preview is stamped"
                );
                drop(host.taken.take());
            },
        );
    }

    fn assert_later_failure_preserves_committed_prefix(later_candidate: bool) {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;

        crate::runtime::service::test_support::input_service::with_input_service(
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
                crate::runtime::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                let records =
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture,
                    );
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
                let mut no_live = NoLiveHost::default();
                let next = deliver_pending_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(outer),
                    &BTreeSet::from([second.message_id]),
                    &mut no_live,
                )
                .unwrap();
                assert!(no_live.calls.is_empty(), "{:?}", no_live.calls);
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
        use crate::broker::runtime as storage;
        use crate::broker::runtime::InputDisposition;
        use crate::broker::runtime::InputKind;
        use crate::broker::runtime::InputState;
        let mut fixture = crate::runtime::test_support::Fixture::new();
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2 WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        let outer = Instant::now() + Duration::from_secs(5);
        let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let settings = settings_for(&fixture.conn);
        let placement = placement_of(&fixture.conn, run.member_id, run.fleet_id);
        let message = broker::send_message(
            &mut fixture.conn,
            run.director_member_id,
            run.member_id,
            "retained preview",
        )
        .unwrap();
        let literal = wrapped_line(&message, "retained preview");
        let original = storage::test_support::reserve_input(
            &mut fixture.conn,
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
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &run.owner(),
                    original.input_id,
                    storage::UncertainCause::WriteFailed,
                )
                .unwrap();
            } else if state == InputState::Settled {
                storage::test_support::settle_input(
                    &mut fixture.conn,
                    &run.owner(),
                    original.input_id,
                    InputState::Uncertain,
                    InputDisposition::AbandonedUncertain,
                    "owned retained preview",
                )
                .unwrap();
            }
            let before = storage::get_input(&fixture.conn, original.input_id).unwrap();
            let changes = fixture.conn.total_changes();
            let mut no_live = NoLiveHost::default();
            let result = deliver_pending_with(
                &mut fixture.conn,
                &settings,
                run.member_id,
                Some(run.fleet_id),
                Deadline::until(outer),
                &BTreeSet::new(),
                &mut no_live,
            )
            .unwrap();
            assert!(no_live.calls.is_empty(), "{:?}", no_live.calls);
            assert_eq!(result, AttemptOutcome::ended(AttemptEnd::Held));
            assert_eq!(fixture.conn.total_changes(), changes);
            assert_eq!(
                storage::get_input(&fixture.conn, original.input_id).unwrap(),
                before
            );
            assert_eq!(preview_stamp(&fixture.conn, message.message_id), None);
        }
        fixture
            .conn
            .execute(
                "UPDATE messages SET text='conflicting private preview' WHERE message_id=?1",
                [message.message_id],
            )
            .unwrap();
        let changes = fixture.conn.total_changes();
        let mut no_live = NoLiveHost::default();
        let failure = deliver_pending_with(
            &mut fixture.conn,
            &settings,
            run.member_id,
            Some(run.fleet_id),
            Deadline::until(outer),
            &BTreeSet::new(),
            &mut no_live,
        )
        .unwrap_err();
        assert!(no_live.calls.is_empty(), "{:?}", no_live.calls);
        assert!(!failure.to_string().contains("conflicting private preview"));
        assert_eq!(fixture.conn.total_changes(), changes);
    }

    #[test]
    fn wrapped_delivery_retirement_expiry_preserves_the_notified_prefix_without_retyping() {
        crate::runtime::service::test_support::input_service::with_input_service(
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
                crate::runtime::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture
                    )
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
            &crate::runtime::SubmitInput {
                operation_key: format!("preview:{run_id}:{message_id}"),
                input_kind: InputKind::Preview,
                message_id: Some(message_id),
                literal: literal.into(),
            },
        )
    }

    #[test]
    fn wrapped_delivery_submits_real_preview_and_atomically_settles_the_actual_message() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;
        crate::runtime::service::test_support::input_service::with_input_service(
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
                assert_eq!(outcome.end, AttemptEnd::NoPending, "{outcome:?}");
                crate::runtime::service::test_support::input_service::wait_input_service_record(
                    fixture, &expected, outer,
                );
                crate::runtime::service::test_support::input_service::input_service_barrier(
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
                let records =
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture,
                    );
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
        crate::runtime::service::test_support::input_service::with_input_service(
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
                crate::runtime::service::test_support::input_service::wait_input_service_record(
                    fixture, &expected, outer,
                );
                let row =
                    preview_row(&fixture.fixture.conn, run.run_id, message.message_id).unwrap();
                assert_eq!(row.state, broker::runtime::InputState::Accepted);
                assert_eq!(
                    preview_stamp(&fixture.fixture.conn, message.message_id),
                    None
                );
                let settings = settings_for(&fixture.fixture.conn);
                let mut independent = crate::db::connect(&settings.database_url).unwrap();
                let outcome = deliver_pending(
                    &mut independent,
                    &settings,
                    run.member_id,
                    Some(run.fleet_id),
                    Deadline::until(outer),
                    &BTreeSet::new(),
                )
                .unwrap();
                assert_eq!(outcome.sent, vec![message.message_id], "{outcome:?}");
                let settled = preview_row(&independent, run.run_id, message.message_id).unwrap();
                assert_eq!(
                    outcome.end,
                    AttemptEnd::NoPending,
                    "retry requires actual retirement acknowledgment: {outcome:?}"
                );
                assert_eq!(settled.input_id, row.input_id);
                assert_eq!(settled.state, broker::runtime::InputState::Settled);
                assert_eq!(
                    settled.disposition,
                    Some(broker::runtime::InputDisposition::Submitted)
                );
                assert!(preview_stamp(&independent, message.message_id).is_some());
                crate::runtime::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture
                    )
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
    fn director_delivery_ends_before_resolving_or_locking_a_pane() {
        let backend = "opencode";
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
        let lock = crate::pane_lock::lock_path(&settings.database_url, &member.placement).unwrap();
        let outcome = deliver_pending(
            &mut conn,
            &settings,
            director,
            Some(fleet),
            Deadline::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(outcome.sent.is_empty());
        assert!(matches!(outcome.end, AttemptEnd::NoPending));
        assert!(!lock.exists(), "{backend}: {}", lock.display());
        assert_eq!(preview_stamp(&conn, reply.message_id), None);
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), vec![reply]);
    }

    #[test]
    fn preview_reads_share_one_snapshot_when_another_connection_changes_placement_and_text() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
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
        assert_eq!(member.placement.mux_pane_id.as_deref(), Some("%2"));
        assert_eq!(message.text, "original");
        let current: String = conn
            .query_row("SELECT text FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(current, "changed");
    }

    use crate::broker::MessageRecord;

    const TS: &str = MessageRecord::FIXTURE_STAMP;
    const CLAUSES: &str = "[cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

    fn record(text: &str) -> MessageRecord {
        MessageRecord::fixture(57, 15, 16, text)
    }

    fn line_body(text: &str) -> String {
        let line = fallback_preview_line(&canonical_preview_line(&record(text)));
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
            fallback_preview_line(&canonical_preview_line(&record(
                "ready (doc)\nsee the reviewer notes"
            ))),
            format!("[cafleet msg 57 from 15 {TS}] ready (doc)⏎see the reviewer notes {CLAUSES}")
        );
    }

    #[test]
    fn multiline_tab_and_control_characters_are_folded_visibly() {
        assert_eq!(line_body("a\r\nb\nc\rd\tx\x1b\x7f"), "a⏎b⏎c⏎d␉x\\x1b\\x7f");
        let line = fallback_preview_line(&canonical_preview_line(&record(
            "! printf danger\r\nnext\u{0}",
        )));
        assert!(crate::pane_lock::valid_input(&line), "{line:?}");
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

    mod deferred_preview {
        use crate::broker::runtime::RunRecord;
        use crate::delivery::tests::*;
        use crate::runtime::service::test_support::ServiceFixture;
        use crate::runtime::service::test_support::input_service::FORM_LINE;
        use crate::runtime::service::test_support::input_service::input_service_barrier;
        use crate::runtime::service::test_support::input_service::input_service_records;
        use crate::runtime::service::test_support::input_service::manual;
        use crate::runtime::service::test_support::input_service::wait_input_service_record;
        use crate::runtime::service::test_support::input_service::with_detecting_input_service;
        use crate::runtime::*;

        /// Consumes the runtime-event notifications the detected interaction sends the
        /// Director, so only the preview's own delivery is observed.
        fn acknowledge_runtime_events(fixture: &mut ServiceFixture, run: &RunRecord) {
            for pending in
                crate::broker::poll_messages(&fixture.fixture.conn, run.director_member_id).unwrap()
            {
                assert_eq!(pending.from_member_id, run.member_id);
                crate::broker::ack_message(&mut fixture.fixture.conn, pending.message_id).unwrap();
            }
        }

        fn stamp(fixture: &ServiceFixture, message_id: i64) -> Option<String> {
            fixture
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[test]
        fn deferred_previews_sent_during_one_interaction_queue_separately_and_drain_in_order() {
            with_detecting_input_service(|fixture, until| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let settings = settings_for(&fixture.fixture.conn);
                manual(fixture, format!("{FORM_LINE}\r").as_bytes(), until);
                loop {
                    assert!(Instant::now() < until, "the detected interaction opens");
                    let health = NativeInputHost.health(&run, until).unwrap();
                    let observation = health.observation.as_ref().unwrap();
                    if observation.eligibility == InputEligibility::Hold(GateReason::Interaction)
                        && observation.pending_count == 0
                        && !observation.pending_evidence
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                acknowledge_runtime_events(fixture, &run);
                let mut sent = Vec::new();
                for body in ["first deferred preview", "second deferred preview"] {
                    let message = crate::broker::send_message(
                        &mut fixture.fixture.conn,
                        run.director_member_id,
                        run.member_id,
                        body,
                    )
                    .unwrap();
                    let deadline =
                        Deadline::until(until.min(Instant::now() + Duration::from_secs(5)));
                    let outcome = deliver_pending(
                        &mut fixture.fixture.conn,
                        &settings,
                        run.member_id,
                        None,
                        deadline,
                        &std::collections::BTreeSet::new(),
                    )
                    .unwrap();
                    assert!(outcome.sent.is_empty(), "{body}: {outcome:?}");
                    let key = format!("preview:{}:{}", run.run_id, message.message_id);
                    let queued = storage::find_input(&fixture.fixture.conn, run.run_id, &key)
                        .unwrap()
                        .unwrap_or_else(|| panic!("{body}: a durable queued receipt"));
                    assert_eq!(queued.state, InputState::Ready, "{body}");
                    let delivery = queued.delivery().unwrap();
                    assert_eq!(
                        delivery.status,
                        broker::runtime::DeliveryStatus::Queued,
                        "{body}"
                    );
                    assert_eq!(
                        delivery.reason.as_deref(),
                        Some("interaction_open"),
                        "{body}: an open interaction is why each preview first queued"
                    );
                    sent.push((message, queued.input_id));
                }
                assert_ne!(sent[0].1, sent[1].1, "each preview has its own input ID");
                assert!(
                    input_service_records(fixture).is_empty(),
                    "zero bytes while queued"
                );

                input_service_barrier(fixture, until);
                let lines: Vec<String> = sent
                    .iter()
                    .map(|(message, _)| canonical_preview_line(message))
                    .collect();
                wait_input_service_record(fixture, &lines[1], until);
                for (message, input_id) in &sent {
                    while stamp(fixture, message.message_id).is_none() {
                        assert!(Instant::now() < until, "each drained preview is stamped");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    let delivered = storage::get_input(&fixture.fixture.conn, *input_id).unwrap();
                    assert!(matches!(
                        delivered.state,
                        InputState::Accepted | InputState::Settled
                    ));
                }
                let mut expected = vec!["barrier".to_owned()];
                expected.extend(lines);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .map(|record| record["line"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    expected,
                    "both queued previews drain once, in order, without another send or wait"
                );
            });
        }
    }
    use crate::time::test_support::TestClock;
    use std::cell::Cell;

    struct RecordingIo<C, A> {
        check: C,
        attempt: A,
    }

    impl<C, A> ReceiveIo for RecordingIo<C, A>
    where
        C: FnMut(Deadline) -> Result<ReceiveSnapshot, CafleetError>,
        A: FnMut(i64, Deadline, &BTreeSet<i64>) -> Result<AttemptOutcome, CafleetError>,
    {
        fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError> {
            (self.check)(deadline)
        }

        fn attempt(
            &mut self,
            member: i64,
            deadline: Deadline,
            excluded: &BTreeSet<i64>,
        ) -> Result<AttemptOutcome, CafleetError> {
            (self.attempt)(member, deadline, excluded)
        }
    }

    fn snapshot(pending: &[(i64, i64)]) -> ReceiveSnapshot {
        ReceiveSnapshot {
            messages: Vec::new(),
            pending: pending
                .iter()
                .map(|&(member_id, message_id)| PendingPreview {
                    member_id,
                    message_id,
                })
                .collect(),
        }
    }

    fn message(id: i64) -> MessageRecord {
        MessageRecord::fixture(id, 2, 1, &"雪\nfull reply".repeat(300))
    }

    fn no_attempt(_: i64, _: Deadline, _: &BTreeSet<i64>) -> Result<AttemptOutcome, CafleetError> {
        panic!("the subject inbox or empty candidate set takes priority");
    }

    #[test]
    fn existing_inbox_returns_full_records_before_retry_or_sleep() {
        let clock = TestClock::new();
        let expected = vec![message(9), message(8)];
        let mut io = RecordingIo {
            check: |_| {
                Ok(ReceiveSnapshot {
                    messages: expected.clone(),
                    pending: snapshot(&[(2, 3)]).pending,
                })
            },
            attempt: no_attempt,
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(30)).unwrap(),
            expected
        );
        assert_eq!(clock.elapsed.get(), Duration::ZERO);
    }

    #[test]
    fn idle_checks_cap_work_and_finish_at_the_original_expiry() {
        let clock = TestClock::new();
        let mut budgets = Vec::new();
        let mut io = RecordingIo {
            check: |deadline: Deadline| {
                budgets.push(deadline.remaining_at(clock.now()).unwrap());
                Ok(snapshot(&[]))
            },
            attempt: no_attempt,
        };
        assert!(
            wait(&clock, &mut io, Duration::from_millis(6500))
                .unwrap()
                .is_empty()
        );
        assert_eq!(clock.elapsed.get(), Duration::from_millis(6500));
        assert_eq!(
            budgets,
            [5000, 5000, 4500, 3500, 2500, 1500, 500].map(Duration::from_millis)
        );
    }

    #[test]
    fn retries_rotate_members_and_exclude_each_failed_message_for_one_invocation() {
        let clock = TestClock::new();
        let mut attempts = Vec::new();
        let mut io = RecordingIo {
            check: |_| Ok(snapshot(&[(2, 20), (2, 21), (5, 50)])),
            attempt: |member, deadline: Deadline, excluded: &BTreeSet<i64>| {
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(5)
                );
                let id = match member {
                    2 if excluded.contains(&20) => 21,
                    2 => 20,
                    5 => 50,
                    _ => panic!("unexpected retry member {member}"),
                };
                attempts.push((clock.elapsed.get(), member, excluded.clone()));
                Ok(AttemptOutcome::ended(AttemptEnd::Failed(
                    crate::delivery::notification_error(member, id, "unavailable"),
                )))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(4)).unwrap();
        assert_eq!(
            attempts,
            vec![
                (Duration::ZERO, 2, BTreeSet::new()),
                (Duration::from_secs(1), 5, BTreeSet::from([20])),
                (Duration::from_secs(2), 2, BTreeSet::from([20, 50])),
            ]
        );
        let mut retried = false;
        let mut io = RecordingIo {
            check: |_| Ok(snapshot(&[(2, 20)])),
            attempt: |_, _, excluded: &BTreeSet<i64>| {
                assert!(excluded.is_empty());
                retried = true;
                clock.advance(Duration::from_secs(1));
                Ok(AttemptOutcome::ended(AttemptEnd::NoPending))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(1)).unwrap();
        assert!(retried);
    }

    #[test]
    fn a_reply_after_a_retry_precedes_the_next_retry() {
        let clock = TestClock::new();
        let attempted = Cell::new(false);
        let mut io = RecordingIo {
            check: |_| {
                Ok(if attempted.get() {
                    ReceiveSnapshot {
                        messages: vec![message(7)],
                        pending: snapshot(&[(5, 50)]).pending,
                    }
                } else {
                    snapshot(&[(2, 20), (5, 50)])
                })
            },
            attempt: |member, _, _: &BTreeSet<i64>| {
                assert_eq!(member, 2);
                assert!(!attempted.replace(true));
                Ok(AttemptOutcome::ended(AttemptEnd::Held))
            },
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(10)).unwrap(),
            vec![message(7)]
        );
        assert_eq!(clock.elapsed.get(), Duration::ZERO);
    }

    #[test]
    fn live_membership_and_slow_attempts_preserve_rotation_without_catch_up() {
        let clock = TestClock::new();
        let mut members = Vec::new();
        let mut io = RecordingIo {
            check: |_| {
                Ok(if clock.elapsed.get().is_zero() {
                    snapshot(&[(2, 20), (5, 50)])
                } else {
                    snapshot(&[(1, 10), (8, 80)])
                })
            },
            attempt: |member, _, _: &BTreeSet<i64>| {
                members.push((clock.elapsed.get(), member));
                clock.advance(Duration::from_millis(2400));
                Ok(AttemptOutcome::ended(AttemptEnd::Held))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(5)).unwrap();
        assert_eq!(
            members,
            [
                (Duration::ZERO, 2),
                (Duration::from_millis(2400), 8),
                (Duration::from_millis(4800), 1)
            ]
        );
    }

    #[test]
    fn an_attempt_near_expiry_keeps_its_own_budget_and_performs_no_final_read() {
        let clock = TestClock::new();
        let mut checks = 0;
        let mut io = RecordingIo {
            check: |deadline: Deadline| {
                checks += 1;
                assert_eq!(checks, 1);
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(1)
                );
                clock.advance(Duration::from_millis(900));
                Ok(snapshot(&[(2, 20)]))
            },
            attempt: |_, deadline: Deadline, _: &BTreeSet<i64>| {
                assert_eq!(
                    deadline.remaining_at(clock.now()).unwrap(),
                    Duration::from_secs(5)
                );
                clock.advance(Duration::from_secs(2));
                Ok(AttemptOutcome::ended(AttemptEnd::NoPending))
            },
        };
        wait(&clock, &mut io, Duration::from_secs(1)).unwrap();
        assert_eq!(checks, 1);
    }

    #[test]
    fn transient_snapshot_contention_recovers_and_fatal_errors_keep_their_cause() {
        let clock = TestClock::new();
        let mut calls = 0;
        let mut io = RecordingIo {
            check: |_| {
                calls += 1;
                if calls == 1 {
                    Err(CafleetError::DatabaseBusy("held writer".into()))
                } else {
                    Ok(ReceiveSnapshot {
                        messages: vec![message(3)],
                        pending: Vec::new(),
                    })
                }
            },
            attempt: no_attempt,
        };
        assert_eq!(
            wait(&clock, &mut io, Duration::from_secs(3)).unwrap(),
            vec![message(3)]
        );
        assert_eq!(clock.elapsed.get(), Duration::from_secs(1));
        for during_attempt in [false, true] {
            let mut io = RecordingIo {
                check: |_| {
                    if during_attempt {
                        Ok(snapshot(&[(2, 20)]))
                    } else {
                        Err(CafleetError::App("corrupt subject".into()))
                    }
                },
                attempt: |_, _, _: &BTreeSet<i64>| Err(CafleetError::App("corrupt subject".into())),
            };
            assert!(
                matches!(wait(&clock, &mut io, Duration::from_secs(1)), Err(CafleetError::App(cause)) if cause == "corrupt subject")
            );
        }
    }

    #[test]
    fn attempt_contention_excludes_the_selected_message_and_late_checks_start_no_input() {
        for late in [false, true] {
            let clock = TestClock::new();
            let mut calls = 0;
            let mut io = RecordingIo {
                check: |_| {
                    if late {
                        clock.advance(Duration::from_secs(1));
                    }
                    Ok(snapshot(&[(2, 20)]))
                },
                attempt: |_, _, excluded: &BTreeSet<i64>| {
                    calls += 1;
                    assert!(excluded.is_empty());
                    Err(CafleetError::DeadlineExceeded)
                },
            };
            wait(
                &clock,
                &mut io,
                Duration::from_secs(if late { 1 } else { 3 }),
            )
            .unwrap();
            assert_eq!(calls, usize::from(!late));
        }
    }
}
