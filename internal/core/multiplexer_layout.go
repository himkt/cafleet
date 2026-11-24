package core

import "fmt"

type columnSide uint8

const (
	columnSideLeft columnSide = iota + 1
	columnSideRight
)

func (s columnSide) asStr() string {
	switch s {
	case columnSideLeft:
		return "left"
	case columnSideRight:
		return "right"
	default:
		panic("invalid columnSide")
	}
}

func columnSideTryFrom(value string) (columnSide, error) {
	switch value {
	case "left":
		return columnSideLeft, nil
	case "right":
		return columnSideRight, nil
	default:
		return 0, invalidStoredValueError("member_placements.column_side", value)
	}
}

func (s columnSide) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

func (s *columnSide) UnmarshalText(text []byte) error {
	parsed, err := columnSideTryFrom(string(text))
	if err != nil {
		return err
	}
	*s = parsed
	return nil
}

const (
	minPaneHeight     int64 = 15
	maxPanesPerColumn int64 = 8
)

// capacity is the member panes one column of a window height rows tall holds.
func capacity(height int64) int64 {
	floor := height / minPaneHeight
	if height%minPaneHeight < 0 {
		floor--
	}
	return min(maxPanesPerColumn, floor)
}

type windowOccupancy struct {
	ordinal      int64
	windowID     string
	windowName   string
	height       int64
	left         []string
	right        []string
	directorPane opt[string]
}

//sumtype:decl
type slotChoice interface {
	isSlotChoice()
}

type slotChoiceExisting struct {
	ordinal    int64
	windowID   string
	windowName string
	side       columnSide
}

type slotChoiceNewWindow struct {
	ordinal    int64
	windowName string
}

func (slotChoiceExisting) isSlotChoice()  {}
func (slotChoiceNewWindow) isSlotChoice() {}

func fleetWindowName(fleetID, ordinal int64) string {
	return fmt.Sprintf("cafleet-%d-%d", fleetID, ordinal)
}

// chooseSlot picks the first column with a free slot: the Director window's right column, then
// each overflow window's left and right columns, else a new window after lastWindowOrdinal.
func chooseSlot(fleetID int64, windows []windowOccupancy, lastWindowOrdinal int64) slotChoice {
	if len(windows) == 0 || windows[0].ordinal != 0 {
		panic("slot selection starts at the Director window")
	}
	for i := 1; i < len(windows); i++ {
		if windows[i-1].ordinal >= windows[i].ordinal {
			panic("window ordinals are strictly ascending")
		}
	}
	type column struct {
		side    columnSide
		members []string
		limit   int64
	}
	for _, window := range windows {
		columns := []column{{columnSideRight, window.right, capacity(window.height)}}
		if window.ordinal != 0 {
			overflow := max(capacity(window.height), 1)
			columns = []column{{columnSideLeft, window.left, overflow}, {columnSideRight, window.right, overflow}}
		}
		for _, candidate := range columns {
			if int64(len(candidate.members)) < candidate.limit {
				return slotChoiceExisting{ordinal: window.ordinal, windowID: window.windowID, windowName: window.windowName, side: candidate.side}
			}
		}
	}
	ordinal := lastWindowOrdinal + 1
	return slotChoiceNewWindow{ordinal: ordinal, windowName: fleetWindowName(fleetID, ordinal)}
}
