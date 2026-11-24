package core

type modeSource uint8

const (
	modeSourceDefault modeSource = iota + 1
	modeSourceExplicit
)

func (m modeSource) asStr() string {
	switch m {
	case modeSourceDefault:
		return "default"
	case modeSourceExplicit:
		return "explicit"
	default:
		panic("invalid modeSource")
	}
}

func modeSourceTryFrom(value string) (modeSource, error) {
	switch value {
	case "default":
		return modeSourceDefault, nil
	case "explicit":
		return modeSourceExplicit, nil
	default:
		return 0, invalidStoredValueError("member_runs.mode_source", value)
	}
}

type coverage uint8

const (
	coverageTested coverage = iota + 1
	coverageUnknown
)

func (c coverage) asStr() string {
	switch c {
	case coverageTested:
		return "tested"
	case coverageUnknown:
		return "unknown"
	default:
		panic("invalid coverage")
	}
}

func coverageTryFrom(value string) (coverage, error) {
	switch value {
	case "tested":
		return coverageTested, nil
	case "unknown":
		return coverageUnknown, nil
	default:
		return 0, invalidStoredValueError("member_runs.coverage", value)
	}
}

type interactionClass uint8

const (
	interactionClassPermissionRequest interactionClass = iota + 1
	interactionClassQuestion
	interactionClassUnknownInteraction
)

func (c interactionClass) asStr() string {
	switch c {
	case interactionClassPermissionRequest:
		return "permission_request"
	case interactionClassQuestion:
		return "question"
	case interactionClassUnknownInteraction:
		return "unknown_interaction"
	default:
		panic("invalid interactionClass")
	}
}

func interactionClassTryFrom(value string) (interactionClass, error) {
	switch value {
	case "permission_request":
		return interactionClassPermissionRequest, nil
	case "question":
		return interactionClassQuestion, nil
	case "unknown_interaction":
		return interactionClassUnknownInteraction, nil
	default:
		return 0, invalidStoredValueError("member_interactions.classification", value)
	}
}

type cleanupState uint8

const (
	cleanupStateComplete cleanupState = iota + 1
	cleanupStateIncomplete
)

func (c cleanupState) asStr() string {
	switch c {
	case cleanupStateComplete:
		return "complete"
	case cleanupStateIncomplete:
		return "incomplete"
	default:
		panic("invalid cleanupState")
	}
}

func cleanupStateTryFrom(value string) (cleanupState, error) {
	switch value {
	case "complete":
		return cleanupStateComplete, nil
	case "incomplete":
		return cleanupStateIncomplete, nil
	default:
		return 0, invalidStoredValueError("member_runs.cleanup_state", value)
	}
}

type permissionDecision uint8

const (
	permissionDecisionAllow permissionDecision = iota + 1
	permissionDecisionDeny
)

func (d permissionDecision) asStr() string {
	switch d {
	case permissionDecisionAllow:
		return "allow"
	case permissionDecisionDeny:
		return "deny"
	default:
		panic("invalid permissionDecision")
	}
}

func permissionDecisionTryFrom(value string) (permissionDecision, error) {
	switch value {
	case "allow":
		return permissionDecisionAllow, nil
	case "deny":
		return permissionDecisionDeny, nil
	default:
		return 0, invalidStoredValueError("response.decision", value)
	}
}

func (v modeSource) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *modeSource) UnmarshalText(text []byte) error {
	parsed, err := modeSourceTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v coverage) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *coverage) UnmarshalText(text []byte) error {
	parsed, err := coverageTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v interactionClass) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *interactionClass) UnmarshalText(text []byte) error {
	parsed, err := interactionClassTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v cleanupState) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *cleanupState) UnmarshalText(text []byte) error {
	parsed, err := cleanupStateTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v permissionDecision) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *permissionDecision) UnmarshalText(text []byte) error {
	parsed, err := permissionDecisionTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}
