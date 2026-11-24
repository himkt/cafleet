package core

import (
	"fmt"
	"math"
	"os"
	"sync"
)

const runnerDiagnosticBufferCapacity = 32

type runnerDiagnosticBuffer struct {
	mu      sync.Mutex
	entries []runnerDiagnosticEntry
	dropped uint64
}

func checkedAddUint64(a, b uint64, what string) uint64 {
	if b > math.MaxUint64-a {
		panic(what + " overflow")
	}
	return a + b
}

func entryCount(entry runnerDiagnosticEntry) uint64 {
	if entry.count <= 0 {
		panic("positive diagnostic count")
	}
	return uint64(entry.count)
}

func (b *runnerDiagnosticBuffer) merge(entry runnerDiagnosticEntry) {
	for index := range b.entries {
		held := &b.entries[index]
		if held.source == entry.source && held.message == entry.message {
			if entry.count > math.MaxInt64-held.count {
				panic("runner diagnostic count overflow")
			}
			held.count += entry.count
			held.firstAt = min(held.firstAt, entry.firstAt)
			held.lastAt = max(held.lastAt, entry.lastAt)
			return
		}
	}
	b.entries = append(b.entries, entry)
}

func (b *runnerDiagnosticBuffer) evictBeyondCapacity() {
	for len(b.entries) > runnerDiagnosticBufferCapacity {
		evicted := b.entries[0]
		b.entries = b.entries[1:]
		b.dropped = checkedAddUint64(b.dropped, entryCount(evicted), "dropped diagnostic count")
	}
}

// runnerDiagnostics is the runner's shared, bounded diagnostic buffer, which the event worker
// flushes into member_runner_diagnostics and shutdown drains into the cleanup transaction. Copies
// share one buffer.
type runnerDiagnostics struct {
	buffer *runnerDiagnosticBuffer
}

func newRunnerDiagnostics() runnerDiagnostics {
	return runnerDiagnostics{buffer: &runnerDiagnosticBuffer{}}
}

// record prints message to the runner's stderr and buffers it sanitized, merging a repeat of the
// same source and message.
func (d runnerDiagnostics) record(source runnerDiagnosticSource, message, at string) {
	fmt.Fprintln(os.Stderr, "member runtime: "+message)
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	d.buffer.merge(runnerDiagnosticEntry{source: source, message: diagnosticText(message), firstAt: at, lastAt: at, count: 1})
	d.buffer.evictBeyondCapacity()
}

// drain takes every buffered entry, oldest first, and the dropped count, leaving the buffer empty.
func (d runnerDiagnostics) drain() ([]runnerDiagnosticEntry, uint64) {
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	entries, dropped := d.buffer.entries, d.buffer.dropped
	d.buffer.entries, d.buffer.dropped = nil, 0
	return entries, dropped
}

// restore returns drained entries that a transient flush failure kept, ahead of newer records.
func (d runnerDiagnostics) restore(entries []runnerDiagnosticEntry, dropped uint64) {
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	newer := d.buffer.entries
	d.buffer.entries = nil
	for _, entry := range append(append([]runnerDiagnosticEntry{}, entries...), newer...) {
		d.buffer.merge(entry)
	}
	d.buffer.dropped = checkedAddUint64(d.buffer.dropped, dropped, "dropped diagnostic count")
	d.buffer.evictBeyondCapacity()
}

// discard counts drained entries that a permanent flush failure lost as dropped.
func (d runnerDiagnostics) discard(entries []runnerDiagnosticEntry, dropped uint64) {
	lost := dropped
	for _, entry := range entries {
		lost = checkedAddUint64(lost, entryCount(entry), "dropped diagnostic count")
	}
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	d.buffer.dropped = checkedAddUint64(d.buffer.dropped, lost, "dropped diagnostic count")
}

func (d runnerDiagnostics) pending() bool {
	d.buffer.mu.Lock()
	defer d.buffer.mu.Unlock()
	return len(d.buffer.entries) > 0 || d.buffer.dropped > 0
}
