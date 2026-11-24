package core

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
