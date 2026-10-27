use std::time::Duration;

use rusqlite::Connection;

use crate::broker::{
    self,
    records::{MemberKind, MemberRecord},
};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::multiplexer::{AnyMultiplexer, PressKeys};

pub(crate) struct KeyAcknowledgment {
    pub member_id: i64,
    pub pane_id: String,
    pub keys: Vec<String>,
}

fn effect_error(member_id: i64, cause: CafleetError, started: bool) -> CafleetError {
    let diagnostic = if started {
        format!(
            "key delivery to member {member_id} is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry."
        )
    } else {
        format!("keys were not dispatched to member {member_id}.")
    };
    CafleetError::WithDiagnostics {
        cause: Box::new(cause),
        diagnostics: vec![diagnostic],
    }
}

fn load_member(conn: &Connection, member_id: i64) -> Result<MemberRecord, CafleetError> {
    let fleet = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    broker::get_member(conn, member_id, fleet)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))
}

pub(crate) fn press_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &PressKeys,
    deadline: Deadline,
) -> Result<KeyAcknowledgment, CafleetError> {
    deadline
        .remaining()
        .map_err(|cause| effect_error(member_id, cause, false))?;
    let mux = super::resolve_mux(settings)
        .map_err(|cause| effect_error(member_id, CafleetError::App(cause.to_string()), false))?;
    press_member_with_mux(conn, settings, member_id, keys, deadline, &mux)
}

pub(crate) fn press_member_with_mux(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &PressKeys,
    deadline: Deadline,
    mux: &AnyMultiplexer,
) -> Result<KeyAcknowledgment, CafleetError> {
    let mut started = false;
    let result = (|| {
        let member = with_database_budget(conn, deadline, |conn| load_member(conn, member_id))?;
        if member.kind == MemberKind::Director {
            return Err(CafleetError::App(
                "cannot press keys in the Director's own pane".into(),
            ));
        }
        let placement = member.placement.as_ref().ok_or_else(|| CafleetError::App(format!("member {member_id} has no placement row; it was not spawned via `cafleet member create`.")))?;
        let pane_id = placement.mux_pane_id.as_ref().ok_or_else(|| {
            CafleetError::App(format!(
                "member {member_id} has no pane yet (pending placement) — nothing to press."
            ))
        })?;
        if mux.name() != placement.backend {
            return Err(CafleetError::App(
                "press backend does not match the recorded placement".into(),
            ));
        }
        let _span = crate::signals::DeferTermination::begin()?;
        let locked = crate::input::lock_member_with_reservation(
            conn,
            settings,
            &member,
            deadline,
            Duration::from_secs(1),
        )?
        .ok_or_else(|| {
            CafleetError::App(format!("member {member_id}'s pane input remained busy"))
        })?;
        if locked.member.fleet_id != member.fleet_id
            || locked.member.kind != member.kind
            || locked.member.placement != member.placement
        {
            return Err(CafleetError::App(format!(
                "member {member_id} placement changed before key dispatch"
            )));
        }
        mux.validate_key_target(placement, deadline)
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        deadline.remaining()?;
        let current = with_database_budget(conn, deadline, |conn| load_member(conn, member_id))?;
        if current.fleet_id != member.fleet_id
            || current.kind != member.kind
            || current.placement != member.placement
        {
            return Err(CafleetError::App(format!(
                "member {member_id} placement changed before key dispatch"
            )));
        }
        locked.pane_lock(&settings.database_url)?;
        deadline.remaining()?;
        mux.send_keys(pane_id, keys, deadline).map_err(|cause| {
            started = cause.submission_started();
            CafleetError::App(cause.to_string())
        })?;
        Ok(KeyAcknowledgment {
            member_id,
            pane_id: pane_id.clone(),
            keys: keys.canonical(),
        })
    })();
    result.map_err(|cause| effect_error(member_id, cause, started))
}

#[cfg(test)]
mod contract_tests;
