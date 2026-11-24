package core

import (
	"fmt"
	"math"
	"testing"
)

const layoutFleet int64 = 3

func TestMultiplexerLayout_FleetWindowNamesCoverTheDirectorAndOverflowOrdinals(t *testing.T) {
	t.Parallel()
	cases := []struct {
		fleetID int64
		ordinal int64
		want    string
	}{
		{fleetID: 3, ordinal: 0, want: "cafleet-3-0"},
		{fleetID: 3, ordinal: 1, want: "cafleet-3-1"},
		{fleetID: 42, ordinal: 12, want: "cafleet-42-12"},
		{fleetID: math.MaxInt64, ordinal: math.MaxInt64, want: "cafleet-9223372036854775807-9223372036854775807"},
	}
	for _, tc := range cases {
		if got := fleetWindowName(tc.fleetID, tc.ordinal); got != tc.want {
			t.Fatalf("fleetWindowName(%d, %d) = %q, want %q", tc.fleetID, tc.ordinal, got, tc.want)
		}
	}
}

func layoutPanes(prefix string, count int) []string {
	panes := make([]string, count)
	for index := range panes {
		panes[index] = fmt.Sprintf("%%%s%d", prefix, index)
	}
	return panes
}

func directorWindow(height int64, members int) windowOccupancy {
	return windowOccupancy{
		ordinal:      0,
		windowID:     "@1",
		windowName:   "main",
		height:       height,
		left:         []string{},
		right:        layoutPanes("1", members),
		directorPane: some("%0"),
	}
}

func overflowWindow(ordinal, height int64, left, right int) windowOccupancy {
	return windowOccupancy{
		ordinal:    ordinal,
		windowID:   fmt.Sprintf("@%d", 10+ordinal),
		windowName: fmt.Sprintf("cafleet-%d-%d", layoutFleet, ordinal),
		height:     height,
		left:       layoutPanes(fmt.Sprintf("%d0", ordinal), left),
		right:      layoutPanes(fmt.Sprintf("%d9", ordinal), right),
	}
}

func existingSlot(ordinal int64, side columnSide) slotChoice {
	if ordinal == 0 {
		return slotChoiceExisting{ordinal: 0, windowID: "@1", windowName: "main", side: side}
	}
	return slotChoiceExisting{
		ordinal:    ordinal,
		windowID:   fmt.Sprintf("@%d", 10+ordinal),
		windowName: fmt.Sprintf("cafleet-%d-%d", layoutFleet, ordinal),
		side:       side,
	}
}

func newWindowSlot(ordinal int64) slotChoice {
	return slotChoiceNewWindow{ordinal: ordinal, windowName: fmt.Sprintf("cafleet-%d-%d", layoutFleet, ordinal)}
}

func TestMultiplexerLayout_LayoutConstantsAreFifteenRowsAndEightPanes(t *testing.T) {
	t.Parallel()
	if minPaneHeight != 15 || maxPanesPerColumn != 8 {
		t.Fatalf("minPaneHeight = %d, maxPanesPerColumn = %d, want 15 and 8", minPaneHeight, maxPanesPerColumn)
	}
}

func TestMultiplexerLayout_CapacityIsTheWindowHeightOverFifteenRowsCappedAtEight(t *testing.T) {
	t.Parallel()
	cases := []struct {
		height int64
		want   int64
	}{
		{0, 0}, {14, 0}, {15, 1}, {25, 1}, {29, 1}, {30, 2}, {50, 3}, {60, 4}, {119, 7}, {120, 8}, {1000, 8},
	}
	for _, tc := range cases {
		if got := capacity(tc.height); got != tc.want {
			t.Fatalf("height %d: capacity = %d, want %d", tc.height, got, tc.want)
		}
	}
}

type slotCase struct {
	name              string
	fleetID           int64
	windows           []windowOccupancy
	lastWindowOrdinal int64
	want              slotChoice
}

func newSlotCase(name string, windows []windowOccupancy, lastWindowOrdinal int64, want slotChoice) slotCase {
	return slotCase{name: name, fleetID: layoutFleet, windows: windows, lastWindowOrdinal: lastWindowOrdinal, want: want}
}

func overflowSlotCase(name string, height int64, left, right int, side opt[columnSide]) slotCase {
	want := newWindowSlot(2)
	if chosen, ok := side.get(); ok {
		want = existingSlot(1, chosen)
	}
	director := directorWindow(30, 2)
	if height < 15 {
		director = directorWindow(10, 0)
	}
	return newSlotCase(name, []windowOccupancy{director, overflowWindow(1, height, left, right)}, 1, want)
}

func TestMultiplexerLayout_ChooseSlotTakesTheFirstColumnBelowCapacityOrTheNextWindowOrdinal(t *testing.T) {
	t.Parallel()
	var cases []slotCase
	for members := range 4 {
		cases = append(cases, newSlotCase(
			fmt.Sprintf("director window holding %d of 4 members", members),
			[]windowOccupancy{directorWindow(60, members)}, 0, existingSlot(0, columnSideRight)))
	}
	cases = append(cases,
		overflowSlotCase("overflow 1 left 0 right", 30, 1, 0, some(columnSideLeft)),
		overflowSlotCase("overflow 2 left 0 right", 30, 2, 0, some(columnSideRight)),
		overflowSlotCase("overflow 2 left 1 right", 30, 2, 1, some(columnSideRight)),
		overflowSlotCase("overflow 2 left 2 right", 30, 2, 2, none[columnSide]()),
		overflowSlotCase("short overflow 0 left 0 right", 10, 0, 0, some(columnSideLeft)),
		overflowSlotCase("short overflow 1 left 0 right", 10, 1, 0, some(columnSideRight)),
		overflowSlotCase("short overflow 1 left 1 right", 10, 1, 1, none[columnSide]()),
		newSlotCase("full director window", []windowOccupancy{directorWindow(60, 4)}, 0, newWindowSlot(1)),
		newSlotCase("one-slot director window holding one member", []windowOccupancy{directorWindow(25, 1)}, 0, newWindowSlot(1)),
		newSlotCase("director window shorter than one slot", []windowOccupancy{directorWindow(14, 0)}, 0, newWindowSlot(1)),
		newSlotCase("short director window skipped for an overflow window",
			[]windowOccupancy{directorWindow(10, 0), overflowWindow(1, 30, 1, 0)}, 1, existingSlot(1, columnSideLeft)),
		newSlotCase("new window after the last allocated ordinal",
			[]windowOccupancy{directorWindow(30, 2), overflowWindow(2, 30, 2, 2)}, 5, newWindowSlot(6)),
		slotCase{
			name: "new window named for its fleet", fleetID: 42,
			windows: []windowOccupancy{directorWindow(30, 2)}, lastWindowOrdinal: 0,
			want: slotChoiceNewWindow{ordinal: 1, windowName: "cafleet-42-1"},
		},
		newSlotCase("overflow windows scanned in ascending ordinal order", []windowOccupancy{
			directorWindow(30, 2), overflowWindow(1, 30, 2, 2), overflowWindow(3, 30, 2, 1), overflowWindow(4, 30, 0, 0),
		}, 4, existingSlot(3, columnSideRight)),
		newSlotCase("freed director slot reused before an overflow window",
			[]windowOccupancy{directorWindow(60, 3), overflowWindow(1, 30, 1, 0)}, 1, existingSlot(0, columnSideRight)),
		newSlotCase("emptied left column refilled before the right column",
			[]windowOccupancy{directorWindow(30, 2), overflowWindow(1, 30, 0, 1)}, 1, existingSlot(1, columnSideLeft)),
	)
	for _, tc := range cases {
		if got := chooseSlot(tc.fleetID, tc.windows, tc.lastWindowOrdinal); got != tc.want {
			t.Fatalf("%s: chooseSlot = %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

func TestMultiplexerLayout_ChooseSlotRefusesWindowsThatDoNotStartAtTheDirectorInAscendingOrder(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name              string
		windows           []windowOccupancy
		lastWindowOrdinal int64
	}{
		{name: "no director window first", windows: []windowOccupancy{overflowWindow(1, 30, 0, 0)}, lastWindowOrdinal: 1},
		{name: "no windows", windows: nil, lastWindowOrdinal: 0},
		{name: "descending ordinals", windows: []windowOccupancy{
			directorWindow(30, 2), overflowWindow(2, 30, 0, 0), overflowWindow(1, 30, 0, 0),
		}, lastWindowOrdinal: 2},
		{name: "repeated ordinal", windows: []windowOccupancy{
			directorWindow(30, 2), overflowWindow(1, 30, 2, 2), overflowWindow(1, 30, 0, 0),
		}, lastWindowOrdinal: 1},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			defer func() {
				if recover() == nil {
					t.Fatalf("%s: chooseSlot accepted the windows", tc.name)
				}
			}()
			chooseSlot(layoutFleet, tc.windows, tc.lastWindowOrdinal)
		})
	}
}
