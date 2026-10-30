mod press;
pub(crate) mod runner;
pub(crate) mod system;
pub(crate) use press::{KeyAcknowledgment, press_member};

use crate::config::Settings;
use crate::multiplexer::{AnyMultiplexer, MultiplexerError, resolve_multiplexer};
use std::rc::Rc;
use system::SystemRunner;

pub(crate) fn resolve_mux(settings: &Settings) -> Result<AnyMultiplexer, MultiplexerError> {
    resolve_multiplexer(
        settings.multiplexer.as_deref(),
        std::env::vars().collect(),
        Rc::new(SystemRunner),
    )
}
