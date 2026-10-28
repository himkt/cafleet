mod press;
pub(crate) mod runner;
pub mod system;
pub(crate) use press::{KeyAcknowledgment, press_member};

#[cfg(test)]
mod runner_tests;
#[cfg(test)]
pub(crate) use runner_tests::control_service::{
    ServiceFixture as InputServiceFixture,
    input_service::{
        barrier as input_service_barrier, composer as input_service_composer,
        records as input_service_records, wait_record as wait_input_service_record,
        with_service as with_input_service,
    },
};
#[cfg(test)]
pub(crate) use runner_tests::owned_child::OwnedChild;

use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, MultiplexerError, resolve_multiplexer};
use rusqlite::Connection;
use std::rc::Rc;
use system::SystemRunner;

fn env_snapshot() -> std::collections::HashMap<String, String> {
    std::env::vars().collect()
}

pub fn resolve_mux(settings: &Settings) -> Result<AnyMultiplexer, MultiplexerError> {
    resolve_multiplexer(
        settings.multiplexer.as_deref(),
        env_snapshot(),
        Rc::new(SystemRunner),
    )
}

pub(crate) fn deliver_pending(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: crate::input::Deadline,
    attempted: &std::collections::BTreeSet<i64>,
) -> Result<crate::receive::AttemptOutcome, CafleetError> {
    crate::delivery::deliver_pending(
        conn,
        settings,
        member_id,
        expected_fleet,
        deadline,
        attempted,
        || resolve_mux(settings),
    )
}
