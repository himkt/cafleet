package core

import (
	"fmt"
	"slices"
	"testing"
)

func (d runnerDiagnostics) snapshot() ([]runnerDiagnosticEntry, uint64) {
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	return slices.Clone(d.buffer.entries), d.buffer.dropped
}

func diagnosticAt(second int) string {
	return fmt.Sprintf("2026-11-21T12:%02d:%02d.000000+00:00", second/60, second%60)
}

func bufferedEntry(source runnerDiagnosticSource, message string, first, last int, count int64) runnerDiagnosticEntry {
	return runnerDiagnosticEntry{source: source, message: message, firstAt: diagnosticAt(first), lastAt: diagnosticAt(last), count: count}
}

func assertDiagnostics(t *testing.T, gotEntries []runnerDiagnosticEntry, gotDropped uint64, wantEntries []runnerDiagnosticEntry, wantDropped uint64, context string) {
	t.Helper()
	if !slices.Equal(gotEntries, wantEntries) || gotDropped != wantDropped {
		t.Fatalf("%s: (%+v, %d), want (%+v, %d)", context, gotEntries, gotDropped, wantEntries, wantDropped)
	}
}

func TestRuntimeDiagnostics_RecordMergesARepeatOfTheSameSourceAndSanitizedMessage(t *testing.T) {
	t.Parallel()
	diagnostics := newRunnerDiagnostics()
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: busy\n", diagnosticAt(1))
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label:\tbusy", diagnosticAt(2))
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: busy", diagnosticAt(3))
	diagnostics.record(runnerDiagnosticSourceObservation, "label: busy", diagnosticAt(4))
	entries, dropped := diagnostics.snapshot()
	assertDiagnostics(t, entries, dropped, []runnerDiagnosticEntry{
		bufferedEntry(runnerDiagnosticSourceEventWriter, "label: busy", 1, 3, 3),
		bufferedEntry(runnerDiagnosticSourceObservation, "label: busy", 4, 4, 1),
	}, 0, "snapshot")
}

func TestRuntimeDiagnostics_RecordBeyond32EntriesEvictsTheOldestAndCountsItDropped(t *testing.T) {
	t.Parallel()
	diagnostics := newRunnerDiagnostics()
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: oldest", diagnosticAt(0))
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: oldest", diagnosticAt(1))
	for index := 1; index <= 31; index++ {
		diagnostics.record(runnerDiagnosticSourceObservation, fmt.Sprintf("label: entry %02d", index), diagnosticAt(index+1))
	}
	if full, dropped := diagnostics.snapshot(); len(full) != 32 || dropped != 0 {
		t.Fatalf("a full buffer holds %d entries with %d dropped", len(full), dropped)
	}
	diagnostics.record(runnerDiagnosticSourceShutdown, "label: newest", diagnosticAt(40))
	entries, dropped := diagnostics.snapshot()
	if len(entries) != 32 || dropped != 2 {
		t.Fatalf("after eviction: %d entries, %d dropped, want 32 and the evicted entry's count 2", len(entries), dropped)
	}
	if first := entries[0]; first != bufferedEntry(runnerDiagnosticSourceObservation, "label: entry 01", 2, 2, 1) {
		t.Fatalf("first = %+v", first)
	}
	if last := entries[len(entries)-1]; last != bufferedEntry(runnerDiagnosticSourceShutdown, "label: newest", 40, 40, 1) {
		t.Fatalf("last = %+v", last)
	}
	if slices.ContainsFunc(entries, func(entry runnerDiagnosticEntry) bool { return entry.message == "label: oldest" }) {
		t.Fatal("the oldest entry is evicted")
	}
}

func TestRuntimeDiagnostics_DrainReturnsEntriesOldestFirstAndClearsTheBuffer(t *testing.T) {
	t.Parallel()
	diagnostics := newRunnerDiagnostics()
	for index := 0; index <= 33; index++ {
		diagnostics.record(runnerDiagnosticSourcePreviewSweep, fmt.Sprintf("label: sweep %02d", index), diagnosticAt(index))
	}
	entries, dropped := diagnostics.drain()
	if dropped != 2 {
		t.Fatalf("dropped = %d", dropped)
	}
	var want []string
	for index := 2; index <= 33; index++ {
		want = append(want, fmt.Sprintf("label: sweep %02d", index))
	}
	var got []string
	for _, entry := range entries {
		got = append(got, entry.message)
	}
	if !slices.Equal(got, want) {
		t.Fatalf("drained messages = %q, want %q", got, want)
	}
	entries, dropped = diagnostics.snapshot()
	assertDiagnostics(t, entries, dropped, nil, 0, "the drained buffer")
	entries, dropped = diagnostics.drain()
	assertDiagnostics(t, entries, dropped, nil, 0, "a second drain")
}

func TestRuntimeDiagnostics_RestoreReturnsDrainedEntriesAheadOfNewerRecordsAndMergesARepeat(t *testing.T) {
	t.Parallel()
	diagnostics := newRunnerDiagnostics()
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: busy", diagnosticAt(1))
	diagnostics.record(runnerDiagnosticSourceDeferredInput, "label: deferred", diagnosticAt(2))
	drained, _ := diagnostics.drain()
	diagnostics.record(runnerDiagnosticSourceEventWriter, "label: busy", diagnosticAt(5))
	diagnostics.record(runnerDiagnosticSourceQueueHousekeeping, "label: queue", diagnosticAt(6))
	diagnostics.restore(drained, 4)
	entries, dropped := diagnostics.snapshot()
	assertDiagnostics(t, entries, dropped, []runnerDiagnosticEntry{
		bufferedEntry(runnerDiagnosticSourceEventWriter, "label: busy", 1, 5, 2),
		bufferedEntry(runnerDiagnosticSourceDeferredInput, "label: deferred", 2, 2, 1),
		bufferedEntry(runnerDiagnosticSourceQueueHousekeeping, "label: queue", 6, 6, 1),
	}, 4, "restored")
}

func TestRuntimeDiagnostics_ClonesShareOneBuffer(t *testing.T) {
	t.Parallel()
	diagnostics := newRunnerDiagnostics()
	writerSide := diagnostics
	writerSide.record(runnerDiagnosticSourceEventWriter, "label: shared", diagnosticAt(1))
	entries, dropped := diagnostics.drain()
	assertDiagnostics(t, entries, dropped, []runnerDiagnosticEntry{bufferedEntry(runnerDiagnosticSourceEventWriter, "label: shared", 1, 1, 1)}, 0, "drained through the original")
	entries, dropped = writerSide.snapshot()
	assertDiagnostics(t, entries, dropped, nil, 0, "the copy sees the drain")
}
