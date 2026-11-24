package core

// A stored enum is a uint8 type whose constants start at iota + 1, with asStr for its stored text,
// <enum>TryFrom for parsing a stored text, and MarshalText/UnmarshalText so JSON carries the
// stored text. rowStoredEnum reads one from a column.
func rowStoredEnum[T any](row sqlRow, column int, tryFrom func(string) (T, error)) (T, error) {
	text, err := row.text(column)
	if err != nil {
		var zero T
		return zero, err
	}
	return tryFrom(text)
}

type memberStatus uint8

const (
	memberStatusActive memberStatus = iota + 1
	memberStatusDeregistered
)

func (s memberStatus) asStr() string {
	switch s {
	case memberStatusActive:
		return "active"
	case memberStatusDeregistered:
		return "deregistered"
	default:
		panic("invalid memberStatus")
	}
}

func memberStatusTryFrom(value string) (memberStatus, error) {
	switch value {
	case "active":
		return memberStatusActive, nil
	case "deregistered":
		return memberStatusDeregistered, nil
	default:
		return 0, invalidStoredValueError("members.status", value)
	}
}

func (s memberStatus) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

func (s *memberStatus) UnmarshalText(text []byte) error {
	parsed, err := memberStatusTryFrom(string(text))
	if err != nil {
		return err
	}
	*s = parsed
	return nil
}

type memberKind uint8

const (
	memberKindDirector memberKind = iota + 1
	memberKindMember
)

func (k memberKind) asStr() string {
	switch k {
	case memberKindDirector:
		return "director"
	case memberKindMember:
		return "member"
	default:
		panic("invalid memberKind")
	}
}

type messageStatus uint8

const (
	messageStatusInputRequired messageStatus = iota + 1
	messageStatusCompleted
)

func (s messageStatus) asStr() string {
	switch s {
	case messageStatusInputRequired:
		return "input_required"
	case messageStatusCompleted:
		return "completed"
	default:
		panic("invalid messageStatus")
	}
}

func messageStatusTryFrom(value string) (messageStatus, error) {
	switch value {
	case "input_required":
		return messageStatusInputRequired, nil
	case "completed":
		return messageStatusCompleted, nil
	default:
		return 0, invalidStoredValueError("messages.status_state", value)
	}
}

func (s messageStatus) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

func (s *messageStatus) UnmarshalText(text []byte) error {
	parsed, err := messageStatusTryFrom(string(text))
	if err != nil {
		return err
	}
	*s = parsed
	return nil
}

type placement struct {
	backend       string
	muxSession    string
	muxWindowID   opt[string]
	muxWindowName opt[string]
	windowOrdinal int64
	columnSide    columnSide
	muxPaneID     opt[string]
	codingAgent   string
	createdAt     string
	muxServer     string
}

// placementColumns are the member_placements columns placementFromRow reads, in order, for the
// `p` alias.
const placementColumns = "p.backend, p.mux_session, p.mux_window_id, p.mux_window_name, p.window_ordinal, p.column_side, p.mux_pane_id, p.coding_agent, p.created_at, p.mux_server"

const placementColumnCount = 10

func placementFromRow(row sqlRow, offset int) (placement, error) {
	var p placement
	var err error
	if p.backend, err = row.text(offset); err != nil {
		return placement{}, err
	}
	if p.muxSession, err = row.text(offset + 1); err != nil {
		return placement{}, err
	}
	if p.muxWindowID, err = row.optText(offset + 2); err != nil {
		return placement{}, err
	}
	if p.muxWindowName, err = row.optText(offset + 3); err != nil {
		return placement{}, err
	}
	if p.windowOrdinal, err = row.int64(offset + 4); err != nil {
		return placement{}, err
	}
	if p.columnSide, err = rowStoredEnum(row, offset+5, columnSideTryFrom); err != nil {
		return placement{}, err
	}
	if p.muxPaneID, err = row.optText(offset + 6); err != nil {
		return placement{}, err
	}
	if p.codingAgent, err = row.text(offset + 7); err != nil {
		return placement{}, err
	}
	if p.createdAt, err = row.text(offset + 8); err != nil {
		return placement{}, err
	}
	if p.muxServer, err = row.text(offset + 9); err != nil {
		return placement{}, err
	}
	return p, nil
}

func (p placement) concreteWindowID() string {
	window, set := p.muxWindowID.get()
	if !set {
		panic("a concrete placement records its window")
	}
	return window
}

type memberRecord struct {
	memberID     int64
	fleetID      int64
	name         string
	description  string
	registeredAt string
	status       memberStatus
	kind         memberKind
	placement    placement
}

type memberActivity struct {
	member       memberRecord
	lastSent     opt[string]
	lastRecv     opt[string]
	lastConsumed opt[string]
	idle         opt[int64]
}

type registeredMember struct {
	memberID     int64
	name         string
	registeredAt string
}

type messageRecord struct {
	messageID       int64
	fromMemberID    int64
	toMemberID      int64
	createdAt       string
	status          messageStatus
	statusTimestamp string
	text            string
}

type fleetRecord struct {
	fleetID          int64
	name             string
	createdAt        string
	deletedAt        opt[string]
	directorMemberID int64
}

type fleetSummary struct {
	fleetID          int64
	name             string
	createdAt        string
	directorMemberID int64
	memberCount      int64
}

type createdFleet struct {
	fleetID   int64
	name      string
	createdAt string
	director  memberRecord
}

type deletedFleet struct {
	fleetID           int64
	deregisteredCount int
	members           []deletedMember
}

type deletedMember struct {
	memberID   int64
	paneStatus string
}
