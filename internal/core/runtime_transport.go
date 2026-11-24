package core

import (
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

// rawTerminalGuard holds a terminal in raw mode (F16) until restore or close returns it to its
// original attributes.
type rawTerminalGuard struct {
	terminal *os.File
	original unix.Termios
	restored bool
}

func rawTerminalGuardEnter(terminal *os.File) (*rawTerminalGuard, error) {
	duplicate, err := unix.FcntlInt(terminal.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	guard := &rawTerminalGuard{terminal: os.NewFile(uintptr(duplicate), terminal.Name())}
	original, err := getTermios(duplicate)
	if err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	guard.original = *original
	raw := *original
	cfmakeraw(&raw)
	if err := setTermios(duplicate, &raw); err != nil {
		closeFile(guard.terminal)
		return nil, ioError(errnoText(err))
	}
	return guard, nil
}

func (g *rawTerminalGuard) restore() error {
	if g.restored {
		return nil
	}
	if err := setTermios(int(g.terminal.Fd()), &g.original); err != nil {
		return runtimeError("terminal restoration requires host recovery: " + errnoText(err))
	}
	g.restored = true
	return nil
}

// close restores the terminal, reporting a failed restoration on stderr, and releases the
// guard's descriptor.
func (g *rawTerminalGuard) close() {
	if err := g.restore(); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
	closeFile(g.terminal)
}
