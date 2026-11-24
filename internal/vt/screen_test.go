package vt

import (
	"bytes"
	"testing"
)

func TestScreen_CopyIntoSnapshotsAndReusesStorage(t *testing.T) {
	t.Parallel()
	cb := &titleCallbacks{}
	p := NewParser(4, 10, cb)
	p.Process([]byte("\x1b]0;title\x07\x1b[31mred\x1b[0m 日本\r\nsecond"))
	want := canonicalDump(p.Screen(), cb, nil)

	var snapshot Screen
	p.Screen().CopyInto(&snapshot)
	if got := canonicalDump(&snapshot, cb, nil); !bytes.Equal(got, want) {
		t.Fatalf("CopyInto snapshot differs from the screen:\n%s\nwant:\n%s", got, want)
	}
	if clone := canonicalDump(p.Screen().Clone(), cb, nil); !bytes.Equal(clone, want) {
		t.Fatalf("Clone differs from the screen:\n%s\nwant:\n%s", clone, want)
	}

	p.Process([]byte("\x1b[2J\x1b[Hchanged"))
	if got := canonicalDump(&snapshot, cb, nil); !bytes.Equal(got, want) {
		t.Fatalf("a later Process changed the snapshot:\n%s", got)
	}

	p.Screen().CopyInto(&snapshot)
	if got, live := canonicalDump(&snapshot, cb, nil), canonicalDump(p.Screen(), cb, nil); !bytes.Equal(got, live) {
		t.Fatalf("a reused snapshot differs from the screen:\n%s\nwant:\n%s", got, live)
	}
}

//nolint:paralleltest // testing.AllocsPerRun panics inside a parallel test.
func TestScreen_CopyIntoSameSizeDoesNotAllocate(t *testing.T) {
	p := NewParser(36, 120, NopCallbacks{})
	p.Process([]byte("\x1b[?1049h\x1b[44mwide 日本語 text\x1b[0m\r\nmore"))
	var snapshot Screen
	p.Screen().CopyInto(&snapshot)
	allocs := testing.AllocsPerRun(10, func() { p.Screen().CopyInto(&snapshot) })
	if allocs != 0 {
		t.Fatalf("CopyInto into a same-size snapshot allocated %v times per run, want 0", allocs)
	}
}

func TestScreen_CopyIntoFollowsResize(t *testing.T) {
	t.Parallel()
	cb := &titleCallbacks{}
	p := NewParser(3, 8, cb)
	p.Process([]byte("abcdefgh\r\nxyz"))
	var snapshot Screen
	p.Screen().CopyInto(&snapshot)

	p.SetSize(6, 20)
	p.Process([]byte("Q"))
	p.Screen().CopyInto(&snapshot)
	if got, want := canonicalDump(&snapshot, cb, nil), canonicalDump(p.Screen(), cb, nil); !bytes.Equal(got, want) {
		t.Fatalf("snapshot after a grow-resize differs:\n%s\nwant:\n%s", got, want)
	}
}
