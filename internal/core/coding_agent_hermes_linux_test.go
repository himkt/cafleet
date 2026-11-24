package core

import (
	"os/exec"
	"strconv"
	"strings"
	"testing"
)

func TestCodingAgentHermesLinux_Settlement_PaneHostCountsARegisteredPidAsRunningOnlyWithItsRecordedStartTime(t *testing.T) {
	t.Parallel()
	host := &paneSettlementHost{paneID: "%0", runID: 1, deadline: settlementDeadline()}
	current, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	pid := uint32(current.pid) //nolint:gosec // the test process's own pid
	index := strings.LastIndex(current.startIdentity, ":")
	if index < 0 {
		t.Fatalf("a Linux process identity ends with its stat start field: %q", current.startIdentity)
	}
	start, err := strconv.ParseUint(current.startIdentity[index+1:], 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	if running, err := host.registeredProcessRunning(pid, start); err != nil || !running {
		t.Fatalf("the live pid with its start time runs: %v %v", running, err)
	}
	if running, err := host.registeredProcessRunning(pid, start+1); err != nil || running {
		t.Fatalf("a live pid with another start time is a reused pid: %v %v", running, err)
	}
	exited := exec.Command("/bin/true")
	if err := exited.Run(); err != nil {
		t.Fatal(err)
	}
	if running, err := host.registeredProcessRunning(uint32(exited.Process.Pid), start); err != nil || running { //nolint:gosec // a child pid
		t.Fatalf("an exited pid does not run: %v %v", running, err)
	}
}
