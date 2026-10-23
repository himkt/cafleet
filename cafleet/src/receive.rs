use std::time::{Duration, Instant};

use crate::broker::records::MessageRecord;
use crate::error::CafleetError;
use crate::input::Deadline;

pub(crate) trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingPreview {
    pub member_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveSnapshot {
    pub messages: Vec<MessageRecord>,
    pub pending: Vec<PendingPreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    NotSent,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationError {
    pub member_id: i64,
    pub message_id: i64,
    pub delivery: Delivery,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptOutcome {
    Held,
    Sent { message_id: i64 },
    Failed(NotificationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitOutcome {
    pub messages: Vec<MessageRecord>,
    pub notification_errors: Vec<NotificationError>,
}

pub(crate) trait ReceiveIo {
    fn check(&mut self, deadline: Deadline) -> Result<ReceiveSnapshot, CafleetError>;
    fn attempt(
        &mut self,
        member_id: i64,
        deadline: Deadline,
    ) -> Result<AttemptOutcome, CafleetError>;
}

pub(crate) fn wait<C: Clock, I: ReceiveIo>(
    _clock: &C,
    _io: &mut I,
    _timeout: Duration,
) -> Result<WaitOutcome, CafleetError> {
    unimplemented!("finite receive behavior follows the reviewed timing contracts")
}
