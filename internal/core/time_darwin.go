package core

import (
	"syscall"
	"unsafe"

	"golang.org/x/sys/unix"
)

// <sys/signal.h> values, which x/sys/unix does not export for darwin.
const (
	darwinSigBlock   = 1
	darwinSigSetmask = 3
)

// threadSigmask is the single audited unsafe site of the package. __pthread_sigmask (syscall
// 329) changes only the calling thread's blocked set, and a direct-exec child forked from that
// thread inherits it; sigprocmask (48) would change every thread of the process on XNU.
//
// Safety: set and previous point to live uint32 values for the duration of the call, matching
// the kernel's 32-bit sigset_t; a nil set passes NULL, which only reads the current mask. The
// kernel writes at most one sigset_t through previous and keeps no pointer after returning.
func threadSigmask(how int, set *signalSet) (signalSet, error) {
	var request, previous uint32
	requested := uintptr(0)
	if set != nil {
		request = uint32(*set)
		requested = uintptr(unsafe.Pointer(&request))
	}
	_, _, errno := syscall.RawSyscall(unix.SYS___PTHREAD_SIGMASK, uintptr(how), requested, uintptr(unsafe.Pointer(&previous)))
	if errno != 0 {
		return 0, errno
	}
	return signalSet(previous), nil
}

// threadBlockSignals adds set to the calling OS thread's blocked signals and returns the
// previous blocked set.
func threadBlockSignals(set signalSet) (signalSet, error) {
	return threadSigmask(darwinSigBlock, &set)
}

func threadSetSignalMask(set signalSet) error {
	_, err := threadSigmask(darwinSigSetmask, &set)
	return err
}

func threadSignalMask() (signalSet, error) {
	return threadSigmask(darwinSigBlock, nil)
}
