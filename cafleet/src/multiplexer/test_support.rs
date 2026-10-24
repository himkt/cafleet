//! Injectable subprocess fixtures for multiplexer contract tests.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::Duration;

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
    /// Simulated remaining budget; `run` costs and `sleep` seconds consume it.
    pub budget: Cell<Option<Duration>>,
    pub run_costs: RefCell<VecDeque<Duration>>,
    pub run_timeouts: RefCell<Vec<(Vec<String>, Duration)>>,
}

impl FakeRunner {
    fn build(binaries: Vec<String>) -> Rc<Self> {
        Rc::new(FakeRunner {
            binaries,
            events: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::new()),
            budget: Cell::new(None),
            run_costs: RefCell::new(VecDeque::new()),
            run_timeouts: RefCell::new(Vec::new()),
        })
    }

    pub fn with_binary(name: &str) -> Rc<Self> {
        Self::build(vec![name.to_string()])
    }

    pub fn without_binaries() -> Rc<Self> {
        Self::build(Vec::new())
    }

    /// Queue the next `run` result; an empty queue answers `Ok("")`.
    pub fn respond(&self, response: Result<String, RunError>) {
        self.responses.borrow_mut().push_back(response);
    }

    /// Report `budget` from `remaining_budget` and consume it on every run and sleep.
    pub fn with_budget(self: Rc<Self>, budget: Duration) -> Rc<Self> {
        self.budget.set(Some(budget));
        self
    }

    /// The next `run` consumes `cost` of the simulated budget; an empty queue costs nothing.
    pub fn cost_next_run(&self, cost: Duration) {
        self.run_costs.borrow_mut().push_back(cost);
    }

    /// The exact timeout each deadline-bounded `run_for` received, with its argv.
    pub fn run_timeouts(&self) -> Vec<(Vec<String>, Duration)> {
        self.run_timeouts.borrow().clone()
    }

    fn consume(&self, elapsed: Duration) {
        if let Some(budget) = self.budget.get() {
            self.budget.set(Some(budget.saturating_sub(elapsed)));
        }
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
        let cost = self.run_costs.borrow_mut().pop_front().unwrap_or_default();
        self.consume(cost);
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or(Ok(String::new()))
    }

    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        self.run_timeouts
            .borrow_mut()
            .push((argv.to_vec(), timeout));
        self.run(
            argv,
            Some(timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0)),
        )
    }

    fn sleep(&self, seconds: f64) {
        self.events.borrow_mut().push(Event::Sleep { seconds });
        self.consume(Duration::from_secs_f64(seconds));
    }

    fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
        Ok(self.budget.get())
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
