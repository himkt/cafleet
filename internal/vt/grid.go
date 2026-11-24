// Transliterated from vt100 0.18.1 src/grid.rs (MIT, Copyright (c) 2016 Jesse Luehrs).
// Scrollback is supported only with capacity 0, the configuration cafleet uses: the state keeps
// the theoretical scrollback length, which decides where rows appear when the screen grows.

package vt

type Size struct{ Rows, Cols uint16 }

type Pos struct{ Row, Col uint16 }

type scrollbackState struct {
	unboundedLen uint
}

type grid struct {
	size            Size
	pos             Pos
	savedPos        Pos
	rows            []row
	scrollTop       uint16
	scrollBottom    uint16
	originMode      bool
	savedOriginMode bool
	scrollback      *scrollbackState
	backing         []Cell
}

type onRowFunc func(*row)

func noRow(*row) {}

func newGrid(size Size, scrollbackEnabled bool) grid {
	g := grid{size: size, scrollBottom: size.Rows - 1}
	if scrollbackEnabled {
		g.scrollback = &scrollbackState{}
	}
	return g
}

func (g *grid) allocateRows() {
	if len(g.rows) == 0 {
		g.rows = make([]row, g.size.Rows)
		for i := range g.rows {
			g.rows[i] = newRow(g.size.Cols)
		}
	}
}

func (g *grid) clear() {
	g.pos = Pos{}
	g.savedPos = Pos{}
	for i := range g.rows {
		g.rows[i].reset()
	}
	g.scrollTop = 0
	g.scrollBottom = g.size.Rows - 1
	g.originMode = false
	g.savedOriginMode = false
}

func absDiffU16(a, b uint16) uint16 {
	if a > b {
		return a - b
	}
	return b - a
}

func (g *grid) setSize(size Size, onRow onRowFunc) {
	if size == g.size {
		return
	}
	g.scrollTop = 0
	g.scrollBottom = g.size.Rows - 1

	diff := absDiffU16(g.size.Rows, size.Rows)
	isEmpty := len(g.rows) == 0
	var posDiff uint16
	switch {
	case isEmpty:
	case size.Rows < g.size.Rows:
		numExcessBottom := g.size.Rows - g.pos.Row - 1
		numPushed := saturatingSubU16(diff, numExcessBottom)
		g.scrollUp(numPushed, onRow)
		g.rows = g.rows[:size.Rows]
		posDiff = numPushed
	case g.scrollback != nil:
		numPulled := min(uint16(min(g.scrollback.unboundedLen, 0xFFFF)), diff)
		grown := make([]row, 0, size.Rows)
		for range numPulled {
			grown = append(grown, newRow(size.Cols))
		}
		grown = append(grown, g.rows...)
		for len(grown) < int(size.Rows) {
			grown = append(grown, newRow(size.Cols))
		}
		g.rows = grown
		g.scrollback.unboundedLen -= uint(numPulled)
		posDiff = numPulled
	default:
		for len(g.rows) < int(size.Rows) {
			g.rows = append(g.rows, newRow(size.Cols))
		}
	}

	if len(g.rows) > 0 {
		g.rows[len(g.rows)-1].wrap(false)
	}
	for i := range g.rows {
		g.rows[i].resize(size.Cols)
	}

	if size.Rows < g.size.Rows {
		g.pos.Row = saturatingSubU16(g.pos.Row, posDiff)
		g.savedPos.Row = min(saturatingSubU16(g.savedPos.Row, posDiff), size.Rows-1)
	} else {
		g.pos.Row += posDiff
		g.savedPos.Row += posDiff
	}
	g.pos.Col = min(g.pos.Col, size.Cols-1)
	g.savedPos.Col = min(g.savedPos.Col, size.Cols-1)
	g.scrollBottom = size.Rows - 1
	g.size = size
}

func (g *grid) setPos(pos Pos) {
	if g.originMode {
		pos.Row = saturatingAddU16(pos.Row, g.scrollTop)
	}
	g.pos = pos
	g.rowClampTop(g.originMode)
	g.rowClampBottom(g.originMode)
	g.colClamp()
}

func (g *grid) saveCursor() {
	g.savedPos = g.pos
	g.savedOriginMode = g.originMode
}

func (g *grid) restoreCursor() {
	g.pos = g.savedPos
	g.originMode = g.savedOriginMode
}

func (g *grid) row(i uint16) *row {
	if int(i) >= len(g.rows) {
		return nil
	}
	return &g.rows[i]
}

func (g *grid) cell(pos Pos) *Cell {
	r := g.row(pos.Row)
	if r == nil {
		return nil
	}
	return r.get(pos.Col)
}

func (g *grid) mustCell(pos Pos) *Cell {
	c := g.cell(pos)
	if c == nil {
		panic("vt100: called `Option::unwrap()` on a `None` value (cell)")
	}
	return c
}

func (g *grid) mustRow(i uint16) *row {
	r := g.row(i)
	if r == nil {
		panic("vt100: called `Option::unwrap()` on a `None` value (row)")
	}
	return r
}

func (g *grid) currentRow() *row { return g.mustRow(g.pos.Row) }

func (g *grid) writeContents(contents []byte) []byte {
	wrapping := false
	for i := range g.rows {
		r := &g.rows[i]
		contents = r.writeContents(contents, 0, g.size.Cols, wrapping)
		if !r.wrapped {
			contents = append(contents, '\n')
		}
		wrapping = r.wrapped
	}
	for len(contents) > 0 && contents[len(contents)-1] == '\n' {
		contents = contents[:len(contents)-1]
	}
	return contents
}

func (g *grid) eraseAll(a Attrs) {
	for i := range g.rows {
		g.rows[i].clear(a)
	}
}

func (g *grid) eraseAllForward(a Attrs) {
	for i := int(g.pos.Row) + 1; i < len(g.rows); i++ {
		g.rows[i].clear(a)
	}
	g.eraseRowForward(a)
}

func (g *grid) eraseAllBackward(a Attrs) {
	for i := 0; i < int(g.pos.Row) && i < len(g.rows); i++ {
		g.rows[i].clear(a)
	}
	g.eraseRowBackward(a)
}

func (g *grid) eraseAllScrollback() {
	if g.scrollback != nil {
		g.scrollback.unboundedLen = 0
	}
}

func (g *grid) eraseRow(a Attrs) { g.currentRow().clear(a) }

func (g *grid) eraseRowForward(a Attrs) {
	r := g.currentRow()
	for col := g.pos.Col; col < g.size.Cols; col++ {
		r.erase(col, a)
	}
}

func (g *grid) eraseRowBackward(a Attrs) {
	r := g.currentRow()
	last := min(g.pos.Col, g.size.Cols-1)
	for col := uint16(0); ; col++ {
		r.erase(col, a)
		if col == last {
			break
		}
	}
}

func (g *grid) insertCells(count uint16) { g.currentRow().insert(g.pos.Col, count) }

func (g *grid) deleteCells(count uint16) {
	g.currentRow().remove(g.pos.Col, min(saturatingAddU16(g.pos.Col, count), g.size.Cols))
}

func (g *grid) eraseCells(count uint16, a Attrs) {
	r := g.currentRow()
	end := min(saturatingAddU16(g.pos.Col, count), g.size.Cols)
	for col := g.pos.Col; col < end; col++ {
		r.erase(col, a)
	}
}

func (g *grid) insertLines(count uint16) {
	if count == 0 {
		return
	}
	if g.pos.Row < g.scrollTop || g.pos.Row > g.scrollBottom {
		return
	}
	between := g.rows[g.pos.Row : int(g.scrollBottom)+1]
	n := min(int(count), len(between))
	rotateRight(between, n)
	for i := range between[:n] {
		between[i].reset()
	}
	g.rows[g.scrollBottom].wrap(false)
}

func (g *grid) deleteLines(count uint16) {
	if g.pos.Row < g.scrollTop || g.pos.Row > g.scrollBottom {
		return
	}
	between := g.rows[g.pos.Row : int(g.scrollBottom)+1]
	n := min(int(count), len(between))
	for i := range between[:n] {
		between[i].reset()
	}
	rotateLeft(between, n)
}

func (g *grid) scrollUp(count uint16, onRow onRowFunc) {
	scrollbackEnabled := g.scrollTop == 0
	between := g.rows[g.scrollTop : int(g.scrollBottom)+1]
	n := min(int(count), len(between))
	if g.scrollback != nil && scrollbackEnabled {
		g.scrollback.unboundedLen = saturatingAddUint(g.scrollback.unboundedLen, uint(n))
	}
	for i := range between[:n] {
		if scrollbackEnabled {
			onRow(&between[i])
		}
		between[i].reset()
	}
	rotateLeft(between, n)
}

func saturatingAddUint(a, b uint) uint {
	if a+b < a {
		return ^uint(0)
	}
	return a + b
}

func (g *grid) scrollDown(count uint16) {
	if count == 0 {
		return
	}
	between := g.rows[g.scrollTop : int(g.scrollBottom)+1]
	n := min(int(count), len(between))
	rotateRight(between, n)
	for i := range between[:n] {
		between[i].reset()
	}
	g.rows[g.scrollBottom].wrap(false)
}

func (g *grid) setScrollRegion(top, bottom uint16) {
	bottom = min(bottom, g.size.Rows-1)
	if top < bottom {
		g.scrollTop = top
		g.scrollBottom = bottom
	} else {
		g.scrollTop = 0
		g.scrollBottom = g.size.Rows - 1
	}
	g.pos.Row = g.scrollTop
	g.pos.Col = 0
}

func (g *grid) inScrollRegion() bool { return g.pos.Row >= g.scrollTop && g.pos.Row <= g.scrollBottom }

func (g *grid) setOriginMode(mode bool) {
	g.originMode = mode
	g.setPos(Pos{})
}

func (g *grid) rowIncClamp(count uint16) {
	in := g.inScrollRegion()
	g.pos.Row = saturatingAddU16(g.pos.Row, count)
	g.rowClampBottom(in)
}

func (g *grid) rowIncScroll(count uint16, onRow onRowFunc) uint16 {
	in := g.inScrollRegion()
	g.pos.Row = saturatingAddU16(g.pos.Row, count)
	lines := g.rowClampBottom(in)
	if in {
		g.scrollUp(lines, onRow)
		return lines
	}
	return 0
}

func (g *grid) rowDecClamp(count uint16) {
	in := g.inScrollRegion()
	g.pos.Row = saturatingSubU16(g.pos.Row, count)
	g.rowClampTop(in)
}

func (g *grid) rowDecScroll(count uint16) {
	in := g.inScrollRegion()
	extra := saturatingSubU16(count, g.pos.Row)
	g.pos.Row = saturatingSubU16(g.pos.Row, count)
	lines := g.rowClampTop(in)
	g.scrollDown(lines + extra)
}

func (g *grid) rowSet(i uint16) {
	g.pos.Row = i
	g.rowClamp()
}

func (g *grid) colInc(count uint16)      { g.pos.Col = saturatingAddU16(g.pos.Col, count) }
func (g *grid) colIncClamp(count uint16) { g.colInc(count); g.colClamp() }
func (g *grid) colDec(count uint16)      { g.pos.Col = saturatingSubU16(g.pos.Col, count) }

func (g *grid) colTab() {
	g.pos.Col -= g.pos.Col % 8
	g.pos.Col = saturatingAddU16(g.pos.Col, 8)
	g.colClamp()
}

func (g *grid) colSet(i uint16) {
	g.pos.Col = i
	g.colClamp()
}

func (g *grid) colWrap(width uint16, wrap bool, onRow onRowFunc) {
	if g.pos.Col <= g.size.Cols-width {
		return
	}
	prev := g.pos
	g.pos.Col = 0
	scrolled := g.rowIncScroll(1, onRow)
	if prev.Row == 0 && scrolled > 0 {
		return
	}
	prev.Row -= scrolled
	g.mustRow(prev.Row).wrap(wrap && prev.Row+1 == g.pos.Row)
}

func (g *grid) rowClampTop(limit bool) uint16 {
	if limit && g.pos.Row < g.scrollTop {
		n := g.scrollTop - g.pos.Row
		g.pos.Row = g.scrollTop
		return n
	}
	return 0
}

func (g *grid) rowClampBottom(limit bool) uint16 {
	bottom := g.size.Rows - 1
	if limit {
		bottom = g.scrollBottom
	}
	if g.pos.Row > bottom {
		n := g.pos.Row - bottom
		g.pos.Row = bottom
		return n
	}
	return 0
}

func (g *grid) rowClamp() {
	if g.pos.Row > g.size.Rows-1 {
		g.pos.Row = g.size.Rows - 1
	}
}

func (g *grid) colClamp() {
	if g.pos.Col > g.size.Cols-1 {
		g.pos.Col = g.size.Cols - 1
	}
}

// copyInto deep-copies g into dst, reusing dst's cell storage when it is large enough.
func (g *grid) copyInto(dst *grid) {
	backing := dst.backing
	rows := dst.rows[:0]
	scrollback := dst.scrollback
	*dst = *g
	if g.scrollback != nil {
		if scrollback == nil || scrollback == g.scrollback {
			scrollback = new(scrollbackState)
		}
		*scrollback = *g.scrollback
		dst.scrollback = scrollback
	}
	total := 0
	for i := range g.rows {
		total += len(g.rows[i].cells)
	}
	if cap(backing) < total {
		backing = make([]Cell, total)
	}
	backing = backing[:total]
	off := 0
	for i := range g.rows {
		n := len(g.rows[i].cells)
		cells := backing[off : off+n : off+n]
		copy(cells, g.rows[i].cells)
		rows = append(rows, row{cells: cells, wrapped: g.rows[i].wrapped})
		off += n
	}
	dst.rows = rows
	dst.backing = backing
}

func (g *grid) clone() grid {
	c := *g
	if g.scrollback != nil {
		sb := *g.scrollback
		c.scrollback = &sb
	}
	c.rows = make([]row, len(g.rows))
	total := 0
	for i := range g.rows {
		total += len(g.rows[i].cells)
	}
	backing := make([]Cell, total)
	off := 0
	for i := range g.rows {
		n := len(g.rows[i].cells)
		cells := backing[off : off+n : off+n]
		copy(cells, g.rows[i].cells)
		c.rows[i] = row{cells: cells, wrapped: g.rows[i].wrapped}
		off += n
	}
	c.backing = backing
	return c
}
