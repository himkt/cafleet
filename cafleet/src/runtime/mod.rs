pub mod system;

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

pub fn deliver_preview(
    conn: &mut Connection,
    settings: &Settings,
    mux: Option<&AnyMultiplexer>,
    member_id: i64,
    message_id: i64,
) -> Result<bool, CafleetError> {
    let Some(mux) = mux else {
        return Ok(false);
    };
    Ok(
        crate::delivery::deliver_pane(conn, mux, settings, member_id, crate::time::now_utc())?
            == Some(message_id),
    )
}
