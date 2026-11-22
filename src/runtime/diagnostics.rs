use crate::broker::runtime::RunnerDiagnosticEntry;
use crate::broker::runtime::RunnerDiagnosticSource;
use crate::broker::runtime::diagnostic_text;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

const RUNNER_DIAGNOSTIC_BUFFER: usize = 32;

#[derive(Default)]
struct Buffer {
    entries: VecDeque<RunnerDiagnosticEntry>,
    dropped: u64,
}

impl Buffer {
    fn merge(&mut self, entry: RunnerDiagnosticEntry) {
        match self
            .entries
            .iter_mut()
            .find(|held| held.source == entry.source && held.message == entry.message)
        {
            Some(held) => {
                held.count = held
                    .count
                    .checked_add(entry.count)
                    .expect("runner diagnostic count overflow");
                held.first_at = held.first_at.clone().min(entry.first_at);
                held.last_at = held.last_at.clone().max(entry.last_at);
            }
            None => self.entries.push_back(entry),
        }
    }

    fn evict_beyond_capacity(&mut self) {
        while self.entries.len() > RUNNER_DIAGNOSTIC_BUFFER {
            let evicted = self.entries.pop_front().expect("an over-full buffer");
            self.dropped = self
                .dropped
                .checked_add(u64::try_from(evicted.count).expect("positive diagnostic count"))
                .expect("dropped diagnostic count overflow");
        }
    }
}

/// The runner's shared, bounded diagnostic buffer, which the event worker flushes into
/// `member_runner_diagnostics` and shutdown drains into the cleanup transaction.
#[derive(Clone, Default)]
pub(crate) struct RunnerDiagnostics(Arc<Mutex<Buffer>>);

impl RunnerDiagnostics {
    fn buffer(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.0.lock().expect("runner diagnostics buffer")
    }

    /// Prints `message` to the runner's stderr and buffers it sanitized, merging a repeat of the
    /// same source and message.
    pub(crate) fn record(&self, source: RunnerDiagnosticSource, message: &str, at: &str) {
        eprintln!("member runtime: {message}");
        let mut buffer = self.buffer();
        buffer.merge(RunnerDiagnosticEntry {
            source,
            message: diagnostic_text(message),
            first_at: at.to_owned(),
            last_at: at.to_owned(),
            count: 1,
        });
        buffer.evict_beyond_capacity();
    }

    /// Takes every buffered entry, oldest first, and the dropped count, leaving the buffer empty.
    pub(crate) fn drain(&self) -> (Vec<RunnerDiagnosticEntry>, u64) {
        let mut buffer = self.buffer();
        let entries = buffer.entries.drain(..).collect();
        (entries, std::mem::take(&mut buffer.dropped))
    }

    /// Returns drained entries that a transient flush failure kept, ahead of newer records.
    pub(crate) fn restore(&self, entries: Vec<RunnerDiagnosticEntry>, dropped: u64) {
        let mut buffer = self.buffer();
        let newer: Vec<_> = buffer.entries.drain(..).collect();
        for entry in entries.into_iter().chain(newer) {
            buffer.merge(entry);
        }
        buffer.dropped = buffer
            .dropped
            .checked_add(dropped)
            .expect("dropped diagnostic count overflow");
        buffer.evict_beyond_capacity();
    }

    /// Counts drained entries that a permanent flush failure lost as dropped.
    pub(crate) fn discard(&self, entries: &[RunnerDiagnosticEntry], dropped: u64) {
        let lost = entries.iter().fold(dropped, |total, entry| {
            total
                .checked_add(u64::try_from(entry.count).expect("positive diagnostic count"))
                .expect("dropped diagnostic count overflow")
        });
        let mut buffer = self.buffer();
        buffer.dropped = buffer
            .dropped
            .checked_add(lost)
            .expect("dropped diagnostic count overflow");
    }

    pub(crate) fn pending(&self) -> bool {
        let buffer = self.buffer();
        !buffer.entries.is_empty() || buffer.dropped > 0
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> (Vec<RunnerDiagnosticEntry>, u64) {
        let buffer = self.buffer();
        (buffer.entries.iter().cloned().collect(), buffer.dropped)
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime::RunnerDiagnosticEntry;
    use crate::broker::runtime::RunnerDiagnosticSource;
    use crate::runtime::diagnostics::*;

    fn at(second: u32) -> String {
        format!(
            "2026-11-21T12:{:02}:{:02}.000000+00:00",
            second / 60,
            second % 60
        )
    }

    fn entry(
        source: RunnerDiagnosticSource,
        message: &str,
        first: u32,
        last: u32,
        count: i64,
    ) -> RunnerDiagnosticEntry {
        RunnerDiagnosticEntry {
            source,
            message: message.into(),
            first_at: at(first),
            last_at: at(last),
            count,
        }
    }

    #[test]
    fn record_merges_a_repeat_of_the_same_source_and_sanitized_message() {
        let diagnostics = RunnerDiagnostics::default();
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: busy\n", &at(1));
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label:\tbusy", &at(2));
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: busy", &at(3));
        diagnostics.record(RunnerDiagnosticSource::Observation, "label: busy", &at(4));
        assert_eq!(
            diagnostics.snapshot(),
            (
                vec![
                    entry(RunnerDiagnosticSource::EventWriter, "label: busy", 1, 3, 3),
                    entry(RunnerDiagnosticSource::Observation, "label: busy", 4, 4, 1),
                ],
                0
            )
        );
    }

    #[test]
    fn record_beyond_32_entries_evicts_the_oldest_and_counts_it_dropped() {
        let diagnostics = RunnerDiagnostics::default();
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: oldest", &at(0));
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: oldest", &at(1));
        for index in 1..=31 {
            diagnostics.record(
                RunnerDiagnosticSource::Observation,
                &format!("label: entry {index:02}"),
                &at(index + 1),
            );
        }
        let (full, dropped) = diagnostics.snapshot();
        assert_eq!(full.len(), 32);
        assert_eq!(dropped, 0);
        diagnostics.record(RunnerDiagnosticSource::Shutdown, "label: newest", &at(40));
        let (entries, dropped) = diagnostics.snapshot();
        assert_eq!(entries.len(), 32);
        assert_eq!(dropped, 2, "the evicted entry's count is dropped");
        assert_eq!(
            entries.first().unwrap(),
            &entry(
                RunnerDiagnosticSource::Observation,
                "label: entry 01",
                2,
                2,
                1
            )
        );
        assert_eq!(
            entries.last().unwrap(),
            &entry(RunnerDiagnosticSource::Shutdown, "label: newest", 40, 40, 1)
        );
        assert!(!entries.iter().any(|entry| entry.message == "label: oldest"));
    }

    #[test]
    fn drain_returns_entries_oldest_first_and_clears_the_buffer() {
        let diagnostics = RunnerDiagnostics::default();
        for index in 0..=33 {
            diagnostics.record(
                RunnerDiagnosticSource::PreviewSweep,
                &format!("label: sweep {index:02}"),
                &at(index),
            );
        }
        let (entries, dropped) = diagnostics.drain();
        assert_eq!(dropped, 2);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.message.as_str())
                .collect::<Vec<_>>(),
            (2..=33)
                .map(|index| format!("label: sweep {index:02}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(diagnostics.snapshot(), (Vec::new(), 0));
        assert_eq!(diagnostics.drain(), (Vec::new(), 0));
    }

    #[test]
    fn restore_returns_drained_entries_ahead_of_newer_records_and_merges_a_repeat() {
        let diagnostics = RunnerDiagnostics::default();
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: busy", &at(1));
        diagnostics.record(
            RunnerDiagnosticSource::DeferredInput,
            "label: deferred",
            &at(2),
        );
        let (drained, _) = diagnostics.drain();
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: busy", &at(5));
        diagnostics.record(
            RunnerDiagnosticSource::QueueHousekeeping,
            "label: queue",
            &at(6),
        );
        diagnostics.restore(drained, 4);
        assert_eq!(
            diagnostics.snapshot(),
            (
                vec![
                    entry(RunnerDiagnosticSource::EventWriter, "label: busy", 1, 5, 2),
                    entry(
                        RunnerDiagnosticSource::DeferredInput,
                        "label: deferred",
                        2,
                        2,
                        1
                    ),
                    entry(
                        RunnerDiagnosticSource::QueueHousekeeping,
                        "label: queue",
                        6,
                        6,
                        1
                    ),
                ],
                4
            )
        );
    }

    #[test]
    fn clones_share_one_buffer() {
        let diagnostics = RunnerDiagnostics::default();
        let writer_side = diagnostics.clone();
        writer_side.record(RunnerDiagnosticSource::EventWriter, "label: shared", &at(1));
        assert_eq!(
            diagnostics.drain(),
            (
                vec![entry(
                    RunnerDiagnosticSource::EventWriter,
                    "label: shared",
                    1,
                    1,
                    1
                )],
                0
            )
        );
        assert_eq!(writer_side.snapshot(), (Vec::new(), 0));
    }
}
