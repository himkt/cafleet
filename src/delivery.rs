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
use crate::time::Deadline;
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

pub(crate) fn fallback_preview_line(canonical: &str) -> String {
    format!("{canonical} {DISMISSED_PROMPT_NOTE}")
}

pub(crate) fn canonical_preview_line(message: &broker::MessageRecord) -> String {
    let folded = fold_visible(&message.text);
    let body = if folded.chars().count() <= INLINE_BODY_LIMIT {
        folded
    } else {
        body_pointer(message)
    };
    format!(
        "[cafleet msg {} from {} {}] {body} {RESUME_CLAUSE}",
        message.message_id, message.from_member_id, message.created_at
    )
}

pub(crate) fn pointer_preview_line(message: &broker::MessageRecord) -> String {
    format!(
        "[cafleet msg {} from {} {}] {} {RESUME_CLAUSE}",
        message.message_id,
        message.from_member_id,
        message.created_at,
        body_pointer(message)
    )
}

fn body_pointer(message: &broker::MessageRecord) -> String {
    format!(
        "Body is {} characters; run {} to read it.",
        message.text.chars().count(),
        crate::cli::poll_command(message.to_member_id, None)
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
            "message {message_id} was not notified to member {member_id}: {cause}; member {member_id}'s runner retries it."
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

fn scoped_preview_error(
    conn: &Connection,
    run: &storage::RunRecord,
    message: &MessageRecord,
    input: &storage::InputOperation,
) -> Result<NotificationError, CafleetError> {
    if input.run_id != run.run_id
        || input.kind != InputKind::Preview
        || input.message_id != Some(message.message_id)
    {
        return Err(CafleetError::App(
            "preview diagnostic receipt association changed".into(),
        ));
    }
    let member = run.member_id;
    let id = input.input_id;
    let message_id = message.message_id;
    let text = if input.disposition == Some(InputDisposition::LeftInComposer) {
        let attempt = input
            .preview_attempt
            .expect("a left preview has its attempt")
            .checked_add(1)
            .ok_or_else(|| CafleetError::App("preview attempt exhausted".into()))?;
        let key = storage::preview_operation_key(run.run_id, message_id, attempt)?;
        let next = storage::find_input(conn, run.run_id, &key)?.ok_or_else(|| {
            CafleetError::App(format!(
                "preview input {id} was left in the composer without a re-delivery attempt"
            ))
        })?;
        let effect = match input.left_stage.expect("a left receipt has its stage") {
            storage::LeftStage::EscapeOnly => format!(
                "was not typed: Escape was sent to member {member} but the input was not typed"
            ),
            storage::LeftStage::Partial => {
                format!("was partly typed into member {member}'s composer and not submitted")
            }
            storage::LeftStage::LiteralComplete => {
                format!("was typed into member {member}'s composer but not submitted")
            }
            storage::LeftStage::Submitted => format!(
                "was typed into member {member}'s composer and its submit key sent, but the submission was not confirmed"
            ),
        };
        let cause = input
            .stop_cause
            .expect("a left receipt has its cause")
            .as_str();
        format!(
            "message {message_id} {effect}: {cause} (input_id={id}, redelivery input_id={}); the runner re-delivers it once the composer is empty.",
            next.input_id
        )
    } else if input.state == InputState::InProgress {
        format!(
            "message {message_id} was not notified to member {member}: input {id} is in progress; inspect it with cafleet input show {id} --json."
        )
    } else if input.state == InputState::Uncertain {
        let cause = input
            .stop_cause
            .expect("an uncertain receipt has its cause")
            .as_str();
        format!(
            "message {message_id} was not notified to member {member}: input {id} is uncertain ({cause}); inspect it with cafleet input show {id} --json and reconcile it with cafleet input reconcile."
        )
    } else if input.disposition == Some(InputDisposition::AbandonedUncertain) {
        format!(
            "message {message_id} was not notified to member {member}: input {id} was abandoned as uncertain; send its content again as a new message if it is still needed."
        )
    } else {
        return Err(CafleetError::App(
            "scoped preview diagnostics require a stopped or held receipt".into(),
        ));
    };
    Ok(NotificationError {
        member_id: member,
        message_id,
        error: text,
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
    let mut notification_errors = Vec::new();
    let mut attempted_inputs = std::collections::BTreeMap::new();
    let backend = crate::coding_agent::coding_agent(&run.backend)
        .ok_or_else(|| CafleetError::App("unknown preview backend".into()))?;
    let verified = crate::coding_agent::ProfileCatalog::bundled().verified_composer(
        backend,
        run.native_version.as_deref(),
        crate::coding_agent::ComposerOperation::Prompt,
    );
    let mut skipped = attempted.clone();
    let mut operation_key =
        broker::runtime::preview_operation_key(run.run_id, message.message_id, 1)?;
    let mut known_input_id = None;
    let mut stage = Stage::Submission;
    let result = (|| {
        let end = 'drain: loop {
            deadline.remaining()?;
            let latest = storage::find_latest_preview_input(conn, run.run_id, message.message_id)?;
            let attempt_number = match latest.as_ref() {
                None => 1,
                Some(input) if input.disposition == Some(InputDisposition::LeftInComposer) => {
                    return Err(CafleetError::App(format!(
                        "preview input {} was left in the composer without a re-delivery attempt",
                        input.input_id
                    )));
                }
                Some(input) if input.disposition == Some(InputDisposition::NotSubmitted) => input
                    .preview_attempt
                    .expect("a preview has its attempt")
                    .checked_add(1)
                    .ok_or_else(|| CafleetError::App("preview attempt exhausted".into()))?,
                Some(input) => input.preview_attempt.expect("a preview has its attempt"),
            };
            operation_key =
                storage::preview_operation_key(run.run_id, message.message_id, attempt_number)?;
            let request = SubmitInput {
                operation_key: operation_key.clone(),
                input_kind: InputKind::Preview,
                message_id: Some(message.message_id),
                literal: canonical_preview_line(&message),
            };
            let retained =
                find_submission(conn, &identity, &request, deadline).map_err(CafleetError::from)?;
            known_input_id = retained.as_ref().map(|input| input.input_id);
            attempted_inputs.insert(message.message_id, known_input_id);
            let scoped_hold = retained.as_ref().is_some_and(|input| {
                matches!(input.state, InputState::InProgress | InputState::Uncertain)
                    || (input.state == InputState::Settled
                        && input.disposition == Some(InputDisposition::AbandonedUncertain))
            });
            if scoped_hold && !verified {
                break AttemptEnd::Held;
            }
            let attempt =
                PreparedSubmission::new(&request, deadline.instant(), &mut RealInputDeadlineClock)?;
            let accepted = 'submit: {
                if scoped_hold {
                    notification_errors.push(scoped_preview_error(
                        conn,
                        &run,
                        &message,
                        retained.as_ref().expect("a scoped hold has its receipt"),
                    )?);
                    break 'submit None;
                }
                if retained.as_ref().is_some_and(|input| {
                    input.state == InputState::Ready && input.deferred_at.is_some()
                }) {
                    break 'submit None;
                }
                if guard.is_none() {
                    guard = host.lock(settings, placement, deadline)?;
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
                        attempted_inputs.insert(message.message_id, result.input_id);
                        match result.status {
                            SubmissionStatus::Accepted => {}
                            SubmissionStatus::Queued => break 'submit None,
                            SubmissionStatus::Held => {
                                held_reason = result.reason;
                                break 'drain AttemptEnd::Held;
                            }
                            SubmissionStatus::Uncertain | SubmissionStatus::LeftInComposer
                                if verified =>
                            {
                                let input = storage::get_input(
                                    conn,
                                    result.input_id.expect("a stopped preview has its receipt"),
                                )?;
                                notification_errors
                                    .push(scoped_preview_error(conn, &run, &message, &input)?);
                                break 'submit None;
                            }
                            SubmissionStatus::Rejected
                            | SubmissionStatus::Uncertain
                            | SubmissionStatus::LeftInComposer => {
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
            operation_key =
                broker::runtime::preview_operation_key(run.run_id, message.message_id, 1)?;
            known_input_id = None;
            stage = Stage::Submission;
        };
        Ok::<_, CafleetError>(end)
    })();
    let end = match result {
        Ok(end) => end,
        Err(source) if sent.is_empty() && notification_errors.is_empty() => return Err(source),
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
        notification_errors,
        attempted_inputs,
    })
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
    pub(crate) notification_errors: Vec<NotificationError>,
    pub(crate) attempted_inputs: std::collections::BTreeMap<i64, Option<i64>>,
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
            notification_errors: Vec::new(),
            attempted_inputs: std::collections::BTreeMap::new(),
        }
    }
}
