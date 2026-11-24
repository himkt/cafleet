// Transliterated from vt100 0.18.1 src/row.rs (MIT, Copyright (c) 2016 Jesse Luehrs).

package vt

import "slices"

type row struct {
	cells   []Cell
	wrapped bool
}

func newRow(cols uint16) row { return row{cells: make([]Cell, cols)} }

func (r *row) cols() uint16 { return colIndex(len(r.cells)) }

// colIndex converts a cell index; a row never exceeds the u16 column count of its Size.
func colIndex(i int) uint16 {
	if i < 0 || i > 0xFFFF {
		panic("vt: column index outside u16")
	}
	return uint16(i)
}

func (r *row) clear(a Attrs) {
	for i := range r.cells {
		r.cells[i].clear(a)
	}
	r.wrapped = false
}

func (r *row) reset() {
	clear(r.cells)
	r.wrapped = false
}

func (r *row) get(col uint16) *Cell {
	if int(col) >= len(r.cells) {
		return nil
	}
	return &r.cells[col]
}

func rotateRight[T any](s []T, k int) { rotateLeft(s, len(s)-k) }

func rotateLeft[T any](s []T, k int) {
	if k > len(s) {
		panic("vt100: rotate amount exceeds slice length")
	}
	slices.Reverse(s[:k])
	slices.Reverse(s[k:])
	slices.Reverse(s)
}

func (r *row) insert(i, count uint16) {
	n := min(uint(count), uint(len(r.cells))-uint(i))
	if n == 0 {
		r.wrapped = false
		return
	}
	wasWideContinuation := false
	if cell := r.get(i); cell != nil && cell.IsWideContinuation() {
		cell.setWideContinuation(false)
		wasWideContinuation = true
	}
	s := r.cells[i:]
	rotateRight(s, int(n))
	clear(s[:n])
	if wasWideContinuation {
		r.get(i).setWideContinuation(true)
	}
	r.setTruncated()
}

func (r *row) remove(start, end uint16) {
	r.wrapped = false
	if start >= end {
		return
	}
	r.clearWide(start)
	last := end - 1
	if last != start {
		r.clearWide(last)
	}
	count := int(end - start)
	s := r.cells[start:]
	clear(s[:count])
	rotateLeft(s, count)
}

func (r *row) erase(i uint16, a Attrs) {
	wide := r.cells[i].IsWide()
	r.clearWide(i)
	r.cells[i].clear(a)
	sub := uint16(1)
	if wide {
		sub = 2
	}
	if i == saturatingSubU16(r.cols(), sub) {
		r.wrapped = false
	}
}

func (r *row) truncate(n uint16) {
	if int(n) < len(r.cells) {
		r.cells = r.cells[:n]
	}
	r.setTruncated()
}

func (r *row) setTruncated() {
	r.wrapped = false
	if len(r.cells) > 0 {
		last := &r.cells[len(r.cells)-1]
		if last.IsWide() {
			last.clear(last.attrs)
		}
	}
}

func (r *row) resize(n uint16) {
	switch {
	case int(n) == len(r.cells):
	case int(n) < len(r.cells):
		r.truncate(n)
	default:
		grown := make([]Cell, n)
		copy(grown, r.cells)
		r.cells = grown
		r.wrapped = false
	}
}

func (r *row) wrap(w bool) { r.wrapped = w }

func (r *row) clearWide(col uint16) {
	cell := &r.cells[col]
	var other *Cell
	switch {
	case cell.IsWide():
		next := int(col) + 1
		if next >= len(r.cells) {
			cell.clear(cell.attrs)
			return
		}
		other = &r.cells[next]
	case cell.IsWideContinuation():
		other = &r.cells[col-1]
	default:
		return
	}
	other.clear(other.attrs)
}

func (r *row) writeContents(contents []byte, start, width uint16, wrapping bool) []byte {
	prevWasWide := false
	prevCol := start
	end := min(len(r.cells), int(start)+int(width))
	for col := int(start); col < end; col++ {
		cell := &r.cells[col]
		if prevWasWide {
			prevWasWide = false
			continue
		}
		prevWasWide = cell.IsWide()
		c := colIndex(col)
		if cell.HasContents() {
			for range c - prevCol {
				contents = append(contents, ' ')
			}
			prevCol += c - prevCol
			contents = append(contents, cell.ContentsBytes()...)
			if cell.IsWide() {
				prevCol += 2
			} else {
				prevCol++
			}
		}
	}
	if prevCol == start && wrapping {
		contents = append(contents, '\n')
	}
	return contents
}

func saturatingSubU16(a, b uint16) uint16 {
	if b > a {
		return 0
	}
	return a - b
}

func saturatingAddU16(a, b uint16) uint16 {
	r := uint32(a) + uint32(b)
	if r > 0xFFFF {
		return 0xFFFF
	}
	return uint16(r)
}
