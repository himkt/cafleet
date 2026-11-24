package core

import "golang.org/x/sys/unix"

func threadSigmask(how int, set *signalSet) (signalSet, error) {
	var request, previous unix.Sigset_t
	requested := (*unix.Sigset_t)(nil)
	if set != nil {
		request.Val[0] = uint64(*set)
		requested = &request
	}
	if err := unix.PthreadSigmask(how, requested, &previous); err != nil {
		return 0, err
	}
	return signalSet(previous.Val[0]), nil
}

// threadBlockSignals adds set to the calling OS thread's blocked signals and returns the
// previous blocked set.
func threadBlockSignals(set signalSet) (signalSet, error) {
	return threadSigmask(unix.SIG_BLOCK, &set)
}

func threadSetSignalMask(set signalSet) error {
	_, err := threadSigmask(unix.SIG_SETMASK, &set)
	return err
}

func threadSignalMask() (signalSet, error) {
	return threadSigmask(unix.SIG_BLOCK, nil)
}
