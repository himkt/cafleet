package core

import (
	"errors"
	"os"
	"strconv"
	"strings"
	"syscall"
	"unicode"
	"unicode/utf8"

	"golang.org/x/sys/unix"
)

func processIdentityRead(pid int64) (processIdentity, bool, error) {
	stat, err := os.ReadFile(procPath(pid, "stat"))
	return processIdentityFromStat(pid, stat, err)
}

func processIdentityFromStat(pid int64, stat []byte, readErr error) (processIdentity, bool, error) {
	text, present, err := presentStat(stat, readErr)
	if err != nil || !present {
		return processIdentity{}, false, err
	}
	end := strings.LastIndexByte(text, ')')
	if end < 0 {
		return processIdentity{}, false, runtimeError("malformed process stat")
	}
	fields := strings.Fields(text[end+1:])
	if len(fields) <= 19 {
		return processIdentity{}, false, runtimeError("missing process birth time")
	}
	boot, err := readUTF8File("/proc/sys/kernel/random/boot_id")
	if err != nil {
		return processIdentity{}, false, ioError(err.Error())
	}
	return processIdentity{pid: pid, startIdentity: strings.TrimSpace(boot) + ":" + fields[19]}, true, nil
}

// readUTF8File reads a file as text, failing as Rust's read_to_string does; the error text is
// the rendered cause.
func readUTF8File(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", errors.New(ioErrorText(err))
	}
	if !utf8.Valid(data) {
		return "", errors.New(invalidUTF8StreamText)
	}
	return string(data), nil
}

// invalidUTF8StreamText is Rust's read_to_string error for bytes that are not UTF-8.
const invalidUTF8StreamText = "stream did not contain valid UTF-8"

// presentStat reads a stat file's text; a task reaped after its /proc entry was found fails the
// read with ESRCH rather than ENOENT, and both mean the process is gone.
func presentStat(stat []byte, readErr error) (string, bool, error) {
	switch {
	case readErr == nil && !utf8.Valid(stat):
		return "", false, ioError(invalidUTF8StreamText)
	case readErr == nil:
		return string(stat), true, nil
	case errors.Is(readErr, syscall.ENOENT), errors.Is(readErr, syscall.ESRCH):
		return "", false, nil
	default:
		return "", false, ioError(ioErrorText(readErr))
	}
}

func stopped(pid int64) (bool, error) {
	stat, err := os.ReadFile(procPath(pid, "stat"))
	return stoppedFromStat(stat, err)
}

func stoppedFromStat(stat []byte, readErr error) (bool, error) {
	text, present, err := presentStat(stat, readErr)
	if err != nil {
		return false, err
	}
	if !present {
		return true, nil
	}
	end := strings.LastIndexByte(text, ')')
	if end < 0 {
		return false, runtimeError("malformed process status")
	}
	return strings.HasPrefix(strings.TrimLeftFunc(text[end+1:], unicode.IsSpace), "Z"), nil
}

// groupMembers lists the PIDs whose process group is group, in /proc directory order.
func groupMembers(group int32) ([]int64, error) {
	directory, err := os.Open("/proc")
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	entries, err := directory.ReadDir(-1)
	closeFile(directory)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	var members []int64
	for _, entry := range entries {
		pid, err := strconv.ParseInt(entry.Name(), 10, 32)
		if err != nil {
			continue
		}
		pgid, err := unix.Getpgid(int(pid))
		switch {
		case errors.Is(err, syscall.ESRCH):
		case err != nil:
			return nil, ioError(errnoText(err))
		case int64(pgid) == int64(group):
			members = append(members, pid)
		}
	}
	return members, nil
}

func (systemProcessTable) entry(pid int64) (processEntry, bool, error) {
	read := func(file string) ([]byte, bool, error) {
		data, err := os.ReadFile(procPath(pid, file))
		switch {
		case err == nil:
			return data, true, nil
		case errors.Is(err, syscall.ENOENT):
			return nil, false, nil
		default:
			return nil, false, appErrorf("cannot read /proc/%d/%s: %s", pid, file, ioErrorText(err))
		}
	}
	stat, statPresent, err := read("stat")
	if err != nil {
		return processEntry{}, false, err
	}
	cmdline, cmdlinePresent, err := read("cmdline")
	if err != nil {
		return processEntry{}, false, err
	}
	if !statPresent || !cmdlinePresent {
		return processEntry{}, false, nil
	}
	parent, err := parseStatParent(pid, stat)
	if err != nil {
		return processEntry{}, false, err
	}
	return processEntry{parent: parent, argv: parseCmdline(cmdline)}, true, nil
}

func (systemProcessTable) nestedPIDNamespace() (bool, error) {
	status, err := readUTF8File("/proc/self/status")
	if err != nil {
		return false, appErrorf("cannot read /proc/self/status: %s", err)
	}
	return parseNspidNested(status)
}
