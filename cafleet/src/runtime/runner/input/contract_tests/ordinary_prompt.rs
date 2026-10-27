use super::super::*;
use crate::broker::runtime as storage;
use crate::coding_agent::ProfileCatalog;
use crate::error::CafleetError;
use crate::runtime::runner::*;
use crate::runtime::runner_tests::Fixture;
use std::collections::VecDeque;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod guards;
mod sql;
mod support;
use support::*;

fn submitted(reply: InputControlReply) -> SubmissionResult {
    match reply {
        InputControlReply::Submission(value) => value,
        _ => panic!("ordinary Prompt returns its actual submission"),
    }
}

#[test]
fn step5_prompt_reservation_and_each_hook_require_a_fresh_drain_before_one_real_write() {
    with_host(|h| {
        let id = h.arm("prompt:fixture-complete");
        assert!(h.advance().is_none());
        assert!(h.pty.program.is_empty());
        h.drain();
        let expected_seq = h.transport.snapshot().screen_seq;
        let reply = submitted(h.advance().expect("actual completed write reply"));
        assert!(matches!(reply.status, SubmissionStatus::Accepted));
        assert!(matches!(reply.stage, SubmissionStage::Submitted));
        assert_eq!(reply.input_id, Some(id));
        assert_eq!(reply.output_seq, Some(expected_seq));
        assert_eq!(h.pty.program, vec![encoded()]);
        assert!(matches!(
            storage::get_input(&h.fixture.conn, id).unwrap().state,
            storage::InputState::Accepted
        ));
        let retained = h.coordinator.retained_checkpoint(id).unwrap();
        assert!(matches!(retained.stage, SubmissionStage::Submitted));
        assert_eq!(
            retained.written_bytes,
            u64::try_from(encoded().len()).unwrap()
        );
        assert_eq!(retained.output_seq, expected_seq);
        let repeat = h.request("prompt:fixture-complete");
        let replay = submitted(
            h.coordinator
                .handle(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    repeat,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .expect("retained key lookup"),
        );
        assert!(matches!(replay.status, SubmissionStatus::Accepted));
        assert_eq!(replay.input_id, Some(id));
        assert_eq!(h.pty.program.len(), 1);
        assert_eq!(h.hooks.count("reserved"), 1);
        assert_eq!(h.hooks.count("before"), 1);
        assert_eq!(h.hooks.count("after"), 1);
    });
}

#[test]
fn step5_prompt_positive_short_write_retains_partial_uncertainty_without_a_second_stage() {
    with_host(|h| {
        let id = h.arm("prompt:fixture-short");
        h.pty.short = Some(3);
        h.drain();
        let reply = submitted(h.advance().expect("actual short-write outcome"));
        assert!(matches!(reply.status, SubmissionStatus::Uncertain));
        assert!(matches!(reply.stage, SubmissionStage::Partial));
        assert_eq!(reply.input_id, Some(id));
        let point = h.coordinator.retained_checkpoint(id).unwrap();
        assert!(matches!(point.stage, SubmissionStage::Partial));
        assert_eq!(point.written_bytes, 3);
        assert!(matches!(
            storage::get_input(&h.fixture.conn, id).unwrap().state,
            storage::InputState::Uncertain
        ));
        h.drain();
        assert!(h.advance().is_none());
        assert_eq!(h.pty.program, vec![encoded()]);
        assert_eq!(h.hooks.count("after"), 1);
        let after = h
            .hooks
            .points
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.boundary == "after")
            .unwrap()
            .clone();
        assert_eq!(after.count, 3);
        assert_eq!(after.seq, point.output_seq);
    });
}
