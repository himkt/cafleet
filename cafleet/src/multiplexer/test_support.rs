//! Injectable subprocess fixtures for multiplexer contract tests.
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
