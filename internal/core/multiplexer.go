package core

import "time"

type runErrorKind uint8

const (
	runErrorBinaryNotFound runErrorKind = iota + 1
	runErrorNotStarted
	runErrorTimeout
	runErrorFailed
)

// runError is a subprocess failure below the backend's message mapping. detail is the cause of
// a binary-not-found or not-started failure and the stderr of a failed run.
type runError struct {
	kind   runErrorKind
	detail string
}

func runErrorDeadlineExpired() *runError {
	return &runError{kind: runErrorNotStarted, detail: "command deadline expired before launch"}
}

type commandRunner interface {
	binaryExists(name string) bool
	run(argv []string) (string, *runError)
	runFor(argv []string, timeout time.Duration) (string, *runError)
}

type muxBackend uint8

const (
	muxBackendTmux muxBackend = iota + 1
	muxBackendHerdr
)

func (m muxBackend) name() string {
	switch m {
	case muxBackendTmux:
		return "tmux"
	case muxBackendHerdr:
		return "herdr"
	default:
		panic("invalid muxBackend")
	}
}

// namedKey is a key's press-token name, tmux key name and herdr key name.
type namedKey struct{ token, tmux, herdr string }

var namedKeys = []namedKey{
	{"Space", "Space", "space"},
	{"Escape", "Escape", "esc"},
	{"Enter", "Enter", "enter"},
	{"Tab", "Tab", "tab"},
	{"Backspace", "BSpace", "backspace"},
	{"Up", "Up", "up"},
	{"Down", "Down", "down"},
	{"Left", "Left", "left"},
	{"Right", "Right", "right"},
}

//sumtype:decl
type pressKey interface{ isPressKey() }

type pressKeyLiteral struct{ b uint8 }

type pressKeyNamed struct{ named *namedKey }

type pressKeyFunction struct{ n uint8 }

type pressKeyControl struct{ b uint8 }

type pressKeyShiftTab struct{}

func (pressKeyLiteral) isPressKey()  {}
func (pressKeyNamed) isPressKey()    {}
func (pressKeyFunction) isPressKey() {}
func (pressKeyControl) isPressKey()  {}
func (pressKeyShiftTab) isPressKey() {}

type pressKeys struct{ keys []pressKey }

func namedKeyOf(token string) *namedKey {
	for i := range namedKeys {
		if namedKeys[i].token == token {
			return &namedKeys[i]
		}
	}
	panic("a named key the byte mapping names")
}

// pressKeysFromBytes is the keys whose bytes are bytes; false when a byte has no explicit-key
// mapping or bytes is empty.
func pressKeysFromBytes(bytes []byte) (pressKeys, bool) {
	keys := make([]pressKey, 0, len(bytes))
	for _, b := range bytes {
		switch {
		case b >= '!' && b <= '~':
			keys = append(keys, pressKeyLiteral{b: b})
		case b == ' ':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Space")})
		case b == 0x1b:
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Escape")})
		case b == '\r':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Enter")})
		case b == '\t':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Tab")})
		case b == 0x7f:
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Backspace")})
		case b >= 0x01 && b <= 0x1a:
			keys = append(keys, pressKeyControl{b: 'a' + b - 1})
		default:
			return pressKeys{}, false
		}
	}
	if len(keys) == 0 {
		return pressKeys{}, false
	}
	return pressKeys{keys: keys}, true
}
