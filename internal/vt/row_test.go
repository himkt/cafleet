// Transliterated from vt100 0.18.1 src/row.rs tests (MIT, Copyright (c) 2016 Jesse Luehrs).

package vt

import "testing"

func TestRow_EraseWideCellInOneColumnRowDoesNotUnderflow(t *testing.T) {
	t.Parallel()
	var attrs Attrs
	r := newRow(1)
	r.get(0).set('中', attrs)
	if !r.get(0).IsWide() {
		t.Fatal("a 2-wide char in the only column is a wide cell")
	}

	r.erase(0, attrs)

	cell := r.get(0)
	if cell.IsWide() {
		t.Fatal("erased cell must no longer be wide")
	}
	if cell.HasContents() {
		t.Fatal("erased cell must be empty")
	}
}
