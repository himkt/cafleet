package core

import (
	"os"
	"slices"
	"strings"
	"time"
)

func runtimeError(message string) *cafleetError { return appError(message) }

func ioError(cause string) *cafleetError { return runtimeError("member runtime: " + cause) }

func ensure(condition bool, message string) error {
	if condition {
		return nil
	}
	return runtimeError(message)
}

type terminalSize struct {
	rows uint16
	cols uint16
}

func (s terminalSize) validate() error {
	return ensure(s.rows > 0 && s.cols > 0, "terminal dimensions must be positive")
}

// napUntil sleeps one polling tick on c, never past deadline.
func napUntil(c clock, deadline time.Time) {
	c.sleep(min(2*time.Millisecond, max(deadline.Sub(c.now()), 0)))
}

type spawnSpec struct {
	argv []string
	cwd  string
	// environment holds name and value pairs set over the inherited environment.
	environment [][2]string
	size        terminalSize
}

func (s spawnSpec) validate() error {
	if err := ensure(len(s.argv) > 0 && s.argv[0] != "", "native argv requires an executable"); err != nil {
		return err
	}
	if err := ensure(!slices.ContainsFunc(s.argv, func(arg string) bool { return strings.Contains(arg, "\x00") }), "native argv contains a NUL"); err != nil {
		return err
	}
	info, err := os.Stat(s.cwd)
	if err := ensure(strings.HasPrefix(s.cwd, "/") && err == nil && info.IsDir(), "native cwd must be an existing absolute directory"); err != nil {
		return err
	}
	validEntry := func(entry [2]string) bool {
		return entry[0] != "" && !strings.ContainsAny(entry[0], "=\x00") && !strings.Contains(entry[1], "\x00")
	}
	if err := ensure(!slices.ContainsFunc(s.environment, func(entry [2]string) bool { return !validEntry(entry) }), "invalid native environment override"); err != nil {
		return err
	}
	return s.size.validate()
}

func checkDeadline(now, deadline time.Time) error {
	if !now.Before(deadline) {
		return deadlineExceededError()
	}
	return nil
}

// spawnSpecWire is the serde encoding of a spawnSpec: environment pairs are two-element arrays,
// and unknown members are ignored as SpawnSpec and TerminalSize do not deny them.
type spawnSpecWire struct {
	Argv        []string         `json:"argv"`
	Cwd         string           `json:"cwd"`
	Environment [][2]string      `json:"environment"`
	Size        terminalSizeWire `json:"size"`
}

type terminalSizeWire struct {
	Rows uint16 `json:"rows"`
	Cols uint16 `json:"cols"`
}

func (*spawnSpecWire) ignoresUnknownFields()    {}
func (*terminalSizeWire) ignoresUnknownFields() {}

func (s spawnSpec) wire() spawnSpecWire {
	environment := s.environment
	if environment == nil {
		environment = [][2]string{}
	}
	return spawnSpecWire{
		Argv:        s.argv,
		Cwd:         s.cwd,
		Environment: environment,
		Size:        terminalSizeWire{Rows: s.size.rows, Cols: s.size.cols},
	}
}

func (w spawnSpecWire) spawnSpec() spawnSpec {
	return spawnSpec{
		argv:        w.Argv,
		cwd:         w.Cwd,
		environment: w.Environment,
		size:        terminalSize{rows: w.Size.Rows, cols: w.Size.Cols},
	}
}
