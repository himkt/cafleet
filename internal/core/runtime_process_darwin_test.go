package core

import (
	"os"
	"slices"
	"syscall"
	"testing"
	"time"
)

const setuidHelper = "/usr/bin/su"

func TestRuntimeProcessDarwin_ProcessGroupTests_KernProcPgrpListsASpawnedGroupLeaderAndThenNothing(t *testing.T) {
	t.Parallel()
	leader, identity := groupLeader(t)
	group := requireGroup(t, identity.pid)
	members, err := groupMembers(group)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(members, []int64{identity.pid}) {
		t.Fatalf("a fresh process group lists %v, want its leader %d alone", members, identity.pid)
	}
	killAndReap(t, leader)
	members, err = groupMembers(group)
	if err != nil {
		t.Fatal(err)
	}
	if len(members) != 0 {
		t.Fatalf("a reaped group has no members: %v", members)
	}
}

func TestRuntimeProcessDarwin_SetuidMemberTests_RefreshPinsALiveSetuidRootGroupMemberAndCleanupTerminatesIt(t *testing.T) {
	t.Parallel()
	info, err := os.Stat(setuidHelper)
	if err != nil {
		t.Fatal(err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok || stat.Uid != 0 || info.Mode()&os.ModeSetuid == 0 {
		t.Fatalf("%s must be setuid root for this fixture", setuidHelper)
	}
	size := terminalSize{rows: 24, cols: 80}
	owned := prepareOwnedPty(t, size)
	native, err := owned.session.spawn(spawnSpec{
		argv: []string{"/bin/sh", "-c", setuidHelper + ` root </dev/tty & printf 'helper=%s\n' "$!"; wait`},
		cwd:  "/",
		size: size,
	})
	if err != nil {
		t.Fatal(err)
	}
	pid := announcedPID(t, ptyReadUntil(t, owned.session, "helper=", "Password:"), "helper=")
	if exit, present, err := owned.session.tryWait(); err != nil || present {
		t.Fatalf("a live native leader keeps the session running: %#v %v %v", exit, present, err)
	}
	if err := owned.session.refreshOwned(); err != nil {
		t.Fatal(err)
	}
	helper := requireIdentity(t, pid)
	if pinned, ok := owned.session.owned[pid]; !ok || pinned != helper {
		t.Fatalf("the prompting setuid helper %#v is pinned as %#v (%v)", helper, pinned, ok)
	}
	if pinned, ok := owned.session.owned[native.pid]; !ok || pinned != native {
		t.Fatalf("the native %#v is pinned as %#v (%v)", native, pinned, ok)
	}
	owned.deadlines = some(shutdownDeadlinesStartingAt(time.Now(), shutdownUnit))
	outcome := owned.stop()
	assertCleanupComplete(t, "setuid helper", outcome)
	if _, exited := outcome.nativeExit.get(); !exited {
		t.Fatalf("shutdown outcome %#v records no native exit", outcome)
	}
	_, present, err := processIdentityRead(pid)
	if err != nil {
		t.Fatal(err)
	}
	if present {
		gone, err := stopped(pid)
		if err != nil {
			t.Fatal(err)
		}
		if !gone {
			t.Fatal("the setuid helper remains running after cleanup")
		}
	}
}
