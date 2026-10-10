//! The broker notice texts (SPEC §6.6 *Broker notices*). Every notice starts
//! with `[cafleet] ` and is posted to the fleet's Director with
//! `broker::post_notice`.

use crate::broker::records::{ExecMember, MemberExec, SilentMember};
use crate::error::CafleetError;
use crate::time::parse_lenient;

/// Whole seconds an exec's command ran, from its start claim to its end.
pub(crate) fn exec_run_seconds(exec: &MemberExec) -> Result<i64, CafleetError> {
    let started_at = exec
        .started_at
        .as_deref()
        .expect("an exec that reported an exit code was started");
    let finished_at = exec
        .finished_at
        .as_deref()
        .expect("an exec that reported an exit code was finished");
    Ok((parse_lenient(finished_at)? - parse_lenient(started_at)?).num_seconds())
}

pub(crate) fn exec_finished(
    exec: &MemberExec,
    member: &ExecMember,
    exit_code: i64,
) -> Result<String, CafleetError> {
    Ok(format!(
        "[cafleet] exec {} on member {} ({}) exited {exit_code} after {} s.",
        exec.exec_id,
        member.member_id,
        member.name,
        exec_run_seconds(exec)?
    ))
}

/// The notice for an exec the loop closed without an exit status: one that
/// never started, or one whose process ended without reporting.
pub(crate) fn exec_lost(exec: &MemberExec, member: &ExecMember) -> String {
    let (exec_id, member_id, name) = (exec.exec_id, member.member_id, &member.name);
    if exec.started_at.is_none() {
        format!(
            "[cafleet] exec {exec_id} on member {member_id} ({name}) did not start within \
             {} s of dispatch. Inspect the pane with cafleet member capture {member_id} and \
             run it again if still needed.",
            crate::broker::EXEC_START_GRACE_SECONDS
        )
    } else {
        format!(
            "[cafleet] exec {exec_id} on member {member_id} ({name}) ended without reporting \
             an exit status. Inspect the pane with cafleet member capture {member_id}."
        )
    }
}

pub(crate) fn silent_member(member: &SilentMember, silent_seconds: i64) -> String {
    let member_id = member.member_id;
    format!(
        "[cafleet] member {member_id} ({}) has sent no message {silent_seconds} s after spawn. \
         Its broker commands may be denied: inspect it with cafleet member capture {member_id} \
         and run cafleet doctor.",
        member.name
    )
}
