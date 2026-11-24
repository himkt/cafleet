package core

import (
	"embed"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"runtime"
	"strconv"
	"syscall"

	"golang.org/x/sys/unix"
)

func Main(args []string, _ embed.FS) int {
	if err := processStartup(); err != nil {
		failure := cafleetErrorOf(err)
		fmt.Fprintf(os.Stderr, "Error: %s\n", failure.message())
		return failure.exitCode()
	}
	return runArgs(args)
}

// processStartup runs before any other open: every inherited descriptor above 2 becomes
// close-on-exec, so no native child inherits it (F1), and SIGPIPE is caught rather than
// ignored, so a broken stdout pipe fails the write while children still start with SIGPIPE at
// its default action (F3).
func processStartup() error {
	if err := markInheritedDescriptorsCloseOnExec(); err != nil {
		return err
	}
	signal.Notify(make(chan os.Signal, 1), syscall.SIGPIPE)
	return nil
}

func descriptorDirectory() string {
	if runtime.GOOS == "darwin" {
		return "/dev/fd"
	}
	return "/proc/self/fd"
}

func markInheritedDescriptorsCloseOnExec() error {
	directory := descriptorDirectory()
	entries, err := os.ReadDir(directory)
	if err != nil {
		return appErrorf("cannot list inherited descriptors in %s: %s", directory, ioErrorText(err))
	}
	for _, entry := range entries {
		fd, err := strconv.Atoi(entry.Name())
		if err != nil || fd <= 2 {
			continue
		}
		flags, err := unix.FcntlInt(uintptr(fd), unix.F_GETFD, 0)
		if errors.Is(err, syscall.EBADF) {
			// The listing's own descriptor is closed by the time it is checked.
			continue
		}
		if err != nil {
			return appErrorf("cannot read the flags of descriptor %d: %s", fd, ioErrorText(err))
		}
		if _, err := unix.FcntlInt(uintptr(fd), unix.F_SETFD, flags|unix.FD_CLOEXEC); err != nil {
			return appErrorf("cannot mark descriptor %d close-on-exec: %s", fd, ioErrorText(err))
		}
	}
	return nil
}
