package core

import (
	"os/exec"
	"slices"
	"syscall"
	"testing"
)

func TestRuntimeProcessDarwin_ProcessGroupTests_KernProcPgrpListsASpawnedGroupLeaderAndThenNothing(t *testing.T) {
	t.Parallel()
	child := exec.Command("/bin/sleep", "30")
	child.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := child.Start(); err != nil {
		t.Fatal(err)
	}
	group := requireGroup(t, int64(child.Process.Pid))
	members, err := groupMembers(group)
	if err != nil || !slices.Equal(members, []int64{int64(group)}) {
		t.Fatalf("a fresh process group lists its leader alone: %v %v", members, err)
	}
	if err := child.Process.Kill(); err != nil {
		t.Fatal(err)
	}
	if err := child.Wait(); err == nil {
		t.Fatal("the killed leader reports its signal")
	}
	members, err = groupMembers(group)
	if err != nil || len(members) != 0 {
		t.Fatalf("a reaped group has no members: %v %v", members, err)
	}
}
