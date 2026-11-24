package core

import (
	"encoding/binary"
	"errors"
	"os/exec"
	"strconv"
	"syscall"

	"golang.org/x/sys/unix"
)

type kernelProcess struct {
	identity processIdentity
	zombie   bool
}

// LP64 Darwin kinfo_proc: kp_proc.p_starttime (timeval) at 0, p_stat at 36, p_pid at 40.
const kinfoProcBytes = 648

const darwinSZOMB = 5

// kernelProcess reads pid's kinfo_proc through kern.proc.pid, which reports p_start for every
// process, including group members with another effective UID; an empty reply means the PID
// is absent.
func kernelProcessRead(pid int64) (kernelProcess, bool, error) {
	pid32, err := pidGroup(pid)
	if err != nil {
		return kernelProcess{}, false, err
	}
	record, err := unix.SysctlRaw("kern.proc.pid", int(pid32))
	if err != nil {
		return kernelProcess{}, false, runtimeError("cannot establish process start identity: " + ioErrorText(err))
	}
	if len(record) == 0 {
		return kernelProcess{}, false, nil
	}
	if err := ensure(len(record) == kinfoProcBytes, "kern.proc.pid returned an unexpected kinfo_proc size"); err != nil {
		return kernelProcess{}, false, err
	}
	if err := ensure(int64(kinfoPID(record)) == pid, "kern.proc.pid returned another process"); err != nil {
		return kernelProcess{}, false, err
	}
	seconds := int64(binary.NativeEndian.Uint64(record[0:8]))
	micros := int32(binary.NativeEndian.Uint32(record[8:12]))
	return kernelProcess{
		identity: processIdentity{pid: pid, startIdentity: strconv.FormatInt(seconds, 10) + ":" + strconv.FormatInt(int64(micros), 10)},
		zombie:   record[36] == darwinSZOMB,
	}, true, nil
}

func kinfoPID(record []byte) int32 {
	return int32(binary.NativeEndian.Uint32(record[40:44]))
}

func processIdentityRead(pid int64) (processIdentity, bool, error) {
	process, present, err := kernelProcessRead(pid)
	return process.identity, present, err
}

func stopped(pid int64) (bool, error) {
	process, present, err := kernelProcessRead(pid)
	if err != nil {
		return false, err
	}
	return !present || process.zombie, nil
}

// groupMembers lists the PIDs of group's kinfo_proc records, rereading when the group grows
// between the size query and the read.
func groupMembers(group int32) ([]int64, error) {
	const attempts = 3
	for range attempts {
		records, err := unix.SysctlRaw("kern.proc.pgrp", int(group))
		if errors.Is(err, syscall.ENOMEM) {
			continue
		}
		if err != nil {
			return nil, ioError(ioErrorText(err))
		}
		if err := ensure(len(records)%kinfoProcBytes == 0, "kern.proc.pgrp returned a partial kinfo_proc record"); err != nil {
			return nil, err
		}
		members := make([]int64, 0, len(records)/kinfoProcBytes)
		for offset := 0; offset < len(records); offset += kinfoProcBytes {
			members = append(members, int64(kinfoPID(records[offset:offset+kinfoProcBytes])))
		}
		return members, nil
	}
	return nil, runtimeError("kern.proc.pgrp kept growing during the read")
}

func (systemProcessTable) entry(pid int64) (processEntry, bool, error) {
	ps := exec.Command("ps", "-o", "ppid=", "-o", "args=", "-p", strconv.FormatInt(pid, 10))
	stdout, err := commandOutput(ps)
	if err != nil {
		return processEntry{}, false, appErrorf("cannot run ps: %s", ioErrorText(err))
	}
	return parsePsOutput(pid, lossyUTF8(stdout))
}

func (systemProcessTable) nestedPIDNamespace() (bool, error) {
	return false, nil
}
