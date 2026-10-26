#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "The approved Batch 2 service adapter consumes this component"
    )
)]
mod episodes;
pub(crate) use crate::coding_agent::{
    MatchEvidence, ObservationKind, ScreenObservation, SemanticFingerprint,
};
#[cfg(test)]
pub(crate) use episodes::*;
mod artifacts;
mod codec;
mod control;
mod creation;
mod health;
mod lifecycle;
mod process;
mod service;
mod terminal;
mod transport;
mod writer;

pub(crate) use artifacts::*;
pub(crate) use codec::*;
pub(crate) use control::{NativeLaunchHost, request_shutdown};
pub(crate) use creation::*;
pub(crate) use health::*;
pub(crate) use lifecycle::*;
pub(crate) use process::*;
pub(crate) use service::{run, terminal_size};
pub(crate) use terminal::*;
pub(crate) use transport::*;

use crate::error::CafleetError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

type Result<T> = std::result::Result<T, CafleetError>;

pub(crate) fn error(message: impl Into<String>) -> CafleetError {
    CafleetError::App(message.into())
}

pub(crate) fn io_error(cause: impl std::fmt::Display) -> CafleetError {
    error(format!("member runtime: {cause}"))
}

pub(crate) fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(error(message))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl TerminalSize {
    pub(crate) fn validate(self) -> Result<()> {
        ensure(
            self.rows > 0 && self.cols > 0,
            "terminal dimensions must be positive",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpawnSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment: Vec<(String, String)>,
    pub evidence_policy: Value,
    pub size: TerminalSize,
}

impl SpawnSpec {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure(
            !self.argv.is_empty() && !self.argv[0].is_empty(),
            "native argv requires an executable",
        )?;
        ensure(
            self.argv.iter().all(|s| !s.contains('\0')),
            "native argv contains a NUL",
        )?;
        ensure(
            self.cwd.is_absolute() && self.cwd.is_dir(),
            "native cwd must be an existing absolute directory",
        )?;
        ensure(
            self.environment
                .iter()
                .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
            "invalid native environment override",
        )?;
        ensure(
            self.evidence_policy.is_object(),
            "evidence policy must be an object",
        )?;
        self.size.validate()
    }
}
