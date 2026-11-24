package core

import (
	"fmt"
	"io"
	"os"
	"os/exec"
	"slices"
	"syscall"
	"testing"
)

// reapedStatRead opens a child's /proc stat, reaps the child, then reads the open file, which
// fails with ESRCH.
func reapedStatRead(t *testing.T) error {
	t.Helper()
	child := exec.Command("true")
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	stat, err := os.Open(fmt.Sprintf("/proc/%d/stat", child.Process.Pid))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := stat.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := child.Wait(); err != nil {
		t.Fatal(err)
	}
	if _, err := io.ReadAll(stat); err != nil {
		return err
	}
	t.Fatal("a stat read of a reaped task fails")
	return nil
}

func TestRuntimeProcessLinux_OwnershipRefreshTests_ReapedStat_RunnerProcessEsrchStatReadIsAVanishedProcess(t *testing.T) {
	t.Parallel()
	identity, present, err := processIdentityFromStat(refreshB, nil, reapedStatRead(t))
	if err != nil || present {
		t.Fatalf("an ESRCH stat read is a vanished process: %#v (%v) %v", identity, present, err)
	}
	gone, err := stoppedFromStat(nil, reapedStatRead(t))
	if err != nil || !gone {
		t.Fatalf("an ESRCH stat read is stopped: %v %v", gone, err)
	}
}

func TestRuntimeProcessLinux_OwnershipRefreshTests_ReapedStat_RunnerProcessOtherStatReadErrorsStillFail(t *testing.T) {
	t.Parallel()
	denied := &os.PathError{Op: "read", Path: "/proc/41002/stat", Err: syscall.EACCES}
	if _, _, err := processIdentityFromStat(refreshB, nil, denied); err == nil {
		t.Fatal("a denied stat read fails the identity")
	}
	if _, err := stoppedFromStat(nil, denied); err == nil {
		t.Fatal("a denied stat read fails the stopped check")
	}
}

func TestRuntimeProcessLinux_OwnershipRefreshTests_ReapedStat_RunnerProcessLiveStatTextKeepsItsIdentityAndRunningState(t *testing.T) {
	t.Parallel()
	pid := int64(os.Getpid())
	text, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		t.Fatal(err)
	}
	current, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	identity, present, err := processIdentityFromStat(pid, text, nil)
	if err != nil || !present || identity != current {
		t.Fatalf("live stat identity %#v (%v) %v, want %#v", identity, present, err, current)
	}
	gone, err := stoppedFromStat(text, nil)
	if err != nil || gone {
		t.Fatalf("a live stat is running: %v %v", gone, err)
	}
}

func TestRuntimeProcessLinux_OwnershipRefreshTests_ReapedStat_RunnerRefreshMemberReapedBetweenEnumerationAndStatReadKeepsTheRunnerAlive(t *testing.T) {
	t.Parallel()
	vanishedIdentity := func(t *testing.T) []observationStep {
		identity, present, err := processIdentityFromStat(refreshB, nil, reapedStatRead(t))
		return []observationStep{{kind: observeIdentity, pid: refreshB, identity: identity, present: present, err: err}}
	}
	vanishedStopped := func(t *testing.T) []observationStep {
		gone, err := stoppedFromStat(nil, reapedStatRead(t))
		return slices.Concat(observedStep(syntheticIdentity(refreshB)), []observationStep{{kind: observeStopped, pid: refreshB, stopped: gone, err: err}})
	}
	cases := []struct {
		name     string
		vanished func(*testing.T) []observationStep
	}{
		{name: "identity read after the reap", vanished: vanishedIdentity},
		{name: "stopped read after the reap", vanished: vanishedStopped},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			session := inertSession(syntheticIdentity(refreshA))
			clock := boundaries(t, session)
			script := newObservationScript(
				membersStep(refreshA, refreshB),
				activeSteps(syntheticIdentity(refreshA)),
				tc.vanished(t),
				observedStep(syntheticIdentity(refreshA)),
			)
			if err := session.refreshOwnedWith(script); err != nil {
				t.Fatalf("%s: %v", tc.name, err)
			}
			script.finished(t, tc.name)
			assertSessionState(t, tc.name, session, pinnedMap(syntheticIdentity(refreshA)), false, clock)
		})
	}
}
