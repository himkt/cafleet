package core

import (
	"context"
	"fmt"
	"os"
	"strings"
	"syscall"
	"testing"
)

const (
	recoveryFixtureNow     = "2026-10-26T20:00:00.000000+00:00"
	descriptorPrivateValue = "descriptor-private-fixture-canary"
)

type recoveryCase struct {
	fixture   *runnerFixture
	artifacts *runArtifacts
	held      *runLock
}

// newRecoveryCase is a run whose lock the current process holds as its running runner.
func newRecoveryCase(t *testing.T) *recoveryCase {
	t.Helper()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	held := acquireRunLock(t, fixture.conn, fixture.run.owner())
	if _, err := held.transferToRunner(ctx, fixture.conn, fixture.run.owner(), currentProcess(t)); err != nil {
		t.Fatal(err)
	}
	if _, err := fixture.conn.execute("UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2", recoveryFixtureNow, fixture.run.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	return &recoveryCase{fixture: fixture, artifacts: artifacts, held: held}
}

func (c *recoveryCase) refresh(t *testing.T) {
	t.Helper()
	c.fixture.refresh(t)
}

// describedRun is a running case whose launch descriptor carries a private argv value and a
// private environment name.
func describedRun(t *testing.T) *recoveryCase {
	t.Helper()
	c := newRecoveryCase(t)
	spec := c.fixture.spec()
	spec.argv = []string{"/usr/bin/false", descriptorPrivateValue}
	spec.environment = append(spec.environment, [2]string{"CAFLEET_TEST_PRIVATE_VALUE", descriptorPrivateValue})
	descriptor, err := newLaunchDescriptor(c.fixture.run, spec)
	if err != nil {
		t.Fatal(err)
	}
	if err := c.artifacts.writeDescriptor(descriptor); err != nil {
		t.Fatal(err)
	}
	return c
}

type fileIdentity struct {
	device, inode string
	size          int64
	modified      int64
}

func pathIdentity(t *testing.T, path string) fileIdentity {
	t.Helper()
	info, err := os.Lstat(path)
	if err != nil {
		t.Fatal(err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		t.Fatalf("%s has no stat record", path)
	}
	return fileIdentity{device: fmt.Sprint(stat.Dev), inode: fmt.Sprint(stat.Ino), size: info.Size(), modified: info.ModTime().UnixNano()}
}

// ownedBytes reads the descriptor while the current process holds the run's validated lock.
func ownedBytes(t *testing.T, c *recoveryCase) []byte {
	t.Helper()
	ctx := context.Background()
	run, err := getRun(ctx, c.fixture.conn, c.fixture.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	current := currentProcess(t)
	if run.ownerPID != current.pid || run.ownerStartIdentity != current.startIdentity {
		t.Fatalf("the current process owns the run: %d %q", run.ownerPID, run.ownerStartIdentity)
	}
	if err := c.held.validate(ctx, c.fixture.conn, run.owner()); err != nil {
		t.Fatal(err)
	}
	bytes, err := os.ReadFile(c.artifacts.paths.descriptor)
	if err != nil {
		t.Fatal(err)
	}
	return bytes
}

func writePrivate(t *testing.T, path string, bytes []byte) {
	t.Helper()
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.Write(bytes); err != nil {
		t.Fatal(err)
	}
	if err := file.Sync(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}

func secretFree(t *testing.T, text string) {
	t.Helper()
	if strings.Contains(text, descriptorPrivateValue) {
		t.Fatalf("descriptor diagnostics preserve private spawn data: %q", text)
	}
	if strings.Contains(text, "CAFLEET_TEST_PRIVATE_VALUE") {
		t.Fatalf("descriptor diagnostics preserve private environment names: %q", text)
	}
}

func bumpEpoch(t *testing.T, conn *dbConn, runID int64) {
	t.Helper()
	if changed, err := conn.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1", runID); err != nil || changed != 1 {
		t.Fatalf("bumpEpoch changed %d rows: %v", changed, err)
	}
}

func currentProcess(t *testing.T) processIdentity {
	t.Helper()
	process, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	return process
}
