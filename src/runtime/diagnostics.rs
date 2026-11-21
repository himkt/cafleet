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
            &entry(RunnerDiagnosticSource::Observation, "label: entry 01", 2, 2, 1)
        );
        assert_eq!(
            entries.last().unwrap(),
            &entry(RunnerDiagnosticSource::Shutdown, "label: newest", 40, 40, 1)
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.message == "label: oldest")
        );
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
        diagnostics.record(RunnerDiagnosticSource::DeferredInput, "label: deferred", &at(2));
        let (drained, _) = diagnostics.drain();
        diagnostics.record(RunnerDiagnosticSource::EventWriter, "label: busy", &at(5));
        diagnostics.record(RunnerDiagnosticSource::QueueHousekeeping, "label: queue", &at(6));
        diagnostics.restore(drained, 4);
        assert_eq!(
            diagnostics.snapshot(),
            (
                vec![
                    entry(RunnerDiagnosticSource::EventWriter, "label: busy", 1, 5, 2),
                    entry(RunnerDiagnosticSource::DeferredInput, "label: deferred", 2, 2, 1),
                    entry(RunnerDiagnosticSource::QueueHousekeeping, "label: queue", 6, 6, 1),
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
                vec![entry(RunnerDiagnosticSource::EventWriter, "label: shared", 1, 1, 1)],
                0
            )
        );
        assert_eq!(writer_side.snapshot(), (Vec::new(), 0));
    }
}
