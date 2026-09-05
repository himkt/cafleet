//! Concrete process and notification adapters shared by CLI and HTTP.
pub mod system;

use crate::broker::InlinePreviewSender;
use crate::config::Settings;
use crate::multiplexer::{AnyMultiplexer, Multiplexer, MultiplexerError, resolve_multiplexer};
use std::rc::Rc;
use system::SystemRunner;

/// The environment snapshot the backends read presence variables from.
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

/// The broker-side preview notifier. Construction is infallible even though
/// it runs before `broker::send_message_record`: a multiplexer-resolution failure is
/// retained as its raw string and exposed only from an attempted
/// `send_inline_preview`, so it can never preempt the insert or fail an
/// intentional skip (SPEC §6.2).
pub struct RuntimeNotifier {
    mux: Result<AnyMultiplexer, String>,
}

impl RuntimeNotifier {
    pub fn new(settings: &Settings) -> Self {
        RuntimeNotifier {
            mux: resolve_mux(settings).map_err(|error| error.to_string()),
        }
    }
}

impl InlinePreviewSender for RuntimeNotifier {
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), String> {
        match &self.mux {
            Ok(mux) => mux
                .send_inline_preview(target_pane_id, message_id, sender_id, ts, text)
                .map_err(|error| error.to_string()),
            Err(retained) => Err(retained.clone()),
        }
    }
}
