//! Shared fixtures for the multiplexer's colocated contract tests (SPEC §6.5).
//!
//! Expected public API pinned by the module test suites:
//!
//! ```text
//! pub struct MultiplexerContext { pub session: String, pub window_id: String,
//!     pub pane_id: String }
//! pub struct MultiplexerError(..);  // Display renders the backend message
//!
//! // The injectable subprocess seam (SPEC §9: fake command runner, no real
//! // tmux/herdr). `sleep` goes through the runner so the Esc-settle (0.1 s)
//! // and submit (1.0 s) delays are observable events, not wall-clock waits.
//! pub enum RunError { BinaryNotFound(String), Timeout, Failed { stderr: String } }
//! pub trait CommandRunner {
//!     fn binary_exists(&self, name: &str) -> bool;
//!     fn run(&self, argv: &[String], timeout_secs: Option<u64>)
//!         -> Result<String, RunError>;
//!     fn sleep(&self, seconds: f64);
//! }
//!
//! // Shared wake payload (byte-identical across backends) + sanitizer.
//! pub fn sanitize_wake_field(value: &str) -> String
//! pub fn build_wake_payload(fleet_id: i64, members: &[Value], director: &Value)
//!     -> Result<String, MultiplexerError>   // Err aborts the wake (invalid agent)
//!
//! // Backend resolution precedence (SPEC §6.5): explicit override → registry
//! // key or error; else auto-detect from HERDR_ENV / TMUX (empty = unset).
//! pub fn resolve_multiplexer_name(override_value: Option<&str>,
//!     env: impl Fn(&str) -> Option<String>) -> Result<&'static str, MultiplexerError>
//!
//! // Backends (constructed with the runner seam + an env snapshot):
//! TmuxMultiplexer::new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>)
//! HerdrMultiplexer::new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>)
//! // shared method surface (both backends):
//! name(&self) -> &'static str
//! ensure_available(&self) -> Result<(), MultiplexerError>
//! context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError>
//! split_window(&self, reference: &MultiplexerContext, env: &[(String, String)],
//!     command: &[String]) -> Result<String, MultiplexerError>
//! send_exit(&self, target_pane_id: &str, ignore_missing: bool) -> Result<(), MultiplexerError>
//! send_poll_trigger(&self, target_pane_id: &str, member_id: i64) -> bool
//! send_wake_trigger(&self, target_pane_id: &str, fleet_id: i64, members: &[Value],
//!     director: &Value) -> Result<bool, MultiplexerError>  // Ok(false) = keystroke lost
//! send_inline_preview(&self, target_pane_id: &str, message_id: i64, sender_id: i64,
//!     ts: &str, text: &str) -> bool
//! send_prompt(&self, target_pane_id: &str, text: &str, shell: bool)
//!     -> Result<(), MultiplexerError>
//! capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError>
//! list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError>
//! kill_pane(&self, target_pane_id: &str, ignore_missing: bool) -> Result<(), MultiplexerError>
//! agent_status(&self, target_pane_id: &str) -> Result<Option<String>, MultiplexerError>
//!     // tmux: always Ok(None); herdr: native state, pane_not_found → Ok(None)
//! ```
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use super::{CommandRunner, RunError};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Run {
        argv: Vec<String>,
        timeout_secs: Option<u64>,
    },
    Sleep {
        seconds: f64,
    },
}

pub struct FakeRunner {
    pub binaries: Vec<String>,
    pub events: RefCell<Vec<Event>>,
    pub responses: RefCell<VecDeque<Result<String, RunError>>>,
}

impl FakeRunner {
    pub fn with_binary(name: &str) -> Rc<Self> {
        Rc::new(FakeRunner {
            binaries: vec![name.to_string()],
            events: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::new()),
        })
    }

    pub fn without_binaries() -> Rc<Self> {
        Rc::new(FakeRunner {
            binaries: Vec::new(),
            events: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::new()),
        })
    }

    /// Queue the next `run` result; an empty queue answers `Ok("")`.
    pub fn respond(&self, response: Result<String, RunError>) {
        self.responses.borrow_mut().push_back(response);
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.borrow().clone()
    }

    /// Just the argv of each `Run` event, in order.
    pub fn run_argvs(&self) -> Vec<Vec<String>> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Run { argv, .. } => Some(argv.clone()),
                Event::Sleep { .. } => None,
            })
            .collect()
    }
}

impl CommandRunner for FakeRunner {
    fn binary_exists(&self, name: &str) -> bool {
        self.binaries.iter().any(|b| b == name)
    }

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError> {
        self.events.borrow_mut().push(Event::Run {
            argv: argv.to_vec(),
            timeout_secs,
        });
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or(Ok(String::new()))
    }

    fn sleep(&self, seconds: f64) {
        self.events.borrow_mut().push(Event::Sleep { seconds });
    }
}

pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

pub fn run_event(parts: &[&str], timeout_secs: Option<u64>) -> Event {
    Event::Run {
        argv: argv(parts),
        timeout_secs,
    }
}

pub fn sleep_event(seconds: f64) -> Event {
    Event::Sleep { seconds }
}

pub fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A herdr success envelope wrapping `result`.
pub fn herdr_envelope(result: serde_json::Value) -> String {
    serde_json::json!({"id": 1, "result": result, "type": "response"}).to_string()
}

/// A herdr error `stderr` body carrying `code`, for `RunError::Failed`.
pub fn herdr_error_stderr(code: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": "boom"}, "id": 1}).to_string()
}
