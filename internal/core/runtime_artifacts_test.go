package core

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"
)

// shortTempdir is a database directory under /tmp, because runtimePathsForRun places the control
// socket beneath the database parent and refuses socket paths past the platform limit.
func shortTempdir(t *testing.T) string {
	t.Helper()
	dir, err := os.MkdirTemp("/tmp", ".cafleet-test-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.RemoveAll(dir); err != nil {
			t.Errorf("removing %s: %v", dir, err)
		}
	})
	return dir
}

func createRunArtifacts(ctx context.Context, conn *dbConn, runID int64) (*runArtifacts, error) {
	return runArtifactsCreateWithDeadline(ctx, conn, runID, fiveSecondsFromNow())
}

func acquireRunLock(t *testing.T, conn *dbConn, owner runOwner) *runLock {
	t.Helper()
	acquired, err := runLockAcquire(context.Background(), conn, owner)
	if err != nil {
		t.Fatal(err)
	}
	lock, ok := acquired.get()
	if !ok {
		t.Fatal("the run lock is free")
	}
	t.Cleanup(func() {
		if err := lock.close(); err != nil {
			t.Errorf("closing the run lock: %v", err)
		}
	})
	return lock
}

func (l *runLock) transferToRunner(ctx context.Context, conn *dbConn, owner runOwner, process processIdentity) (runOwner, error) {
	return l.transferToRunnerWithDeadline(ctx, conn, owner, process, fiveSecondsFromNow(), systemClock{})
}

func (l *runLock) transferToReconcilerWithDeadline(ctx context.Context, conn *dbConn, expected runRecord, process processIdentity, deadline time.Time) (runOwner, error) {
	run, err := l.transferToReconcilerChecked(ctx, conn, expected, process, deadline,
		func(context.Context, *dbConn) error { return nil }, systemClock{})
	if err != nil {
		return runOwner{}, err
	}
	return run.owner(), nil
}

// runLockAcquireReleased is the run-lock counterpart of paneLockAcquireReleased.
func runLockAcquireReleased(ctx context.Context, conn *dbConn, owner runOwner) (opt[*runLock], error) {
	until := time.Now().Add(releasedLockGrace)
	for {
		acquired, err := runLockAcquire(ctx, conn, owner)
		if err != nil {
			return none[*runLock](), err
		}
		if _, ok := acquired.get(); ok || !time.Now().Before(until) {
			return acquired, nil
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func fileMode(t *testing.T, path string) os.FileMode {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info.Mode().Perm()
}

func exists(t *testing.T, path string) bool {
	t.Helper()
	_, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return false
	}
	if err != nil {
		t.Fatal(err)
	}
	return true
}

func mustRun(t *testing.T, conn *dbConn, runID int64) runRecord {
	t.Helper()
	run, err := getRun(context.Background(), conn, runID)
	if err != nil {
		t.Fatal(err)
	}
	return run
}

func assertRunUnchanged(t *testing.T, conn *dbConn, want runRecord, context string) {
	t.Helper()
	if got := mustRun(t, conn, want.runID); !got.equal(want) {
		t.Fatalf("%s: run = %+v, want %+v", context, got, want)
	}
}

func removeOwned(selected string) error {
	if err := os.Remove(selected); err != nil {
		return appError("owned descriptor removal failed")
	}
	return nil
}

func TestRuntimeArtifacts_RunnerRecoveryHistoricalDescriptorPinRemoveUsesTheSuppliedConnectionBudget(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	for _, takeovers := range []int{1, 2} {
		clock := testClockStartingAt(time.Now())
		c := describedRun(t)
		for range takeovers {
			if _, err := c.held.transferToReconcilerWithDeadline(ctx, c.fixture.conn, c.fixture.run, currentProcess(t), time.Now().Add(time.Second)); err != nil {
				t.Fatal(err)
			}
			c.refresh(t)
		}
		if c.fixture.run.ownerEpoch <= 1 {
			t.Fatalf("%d takeovers: epoch %d", takeovers, c.fixture.run.ownerEpoch)
		}
		before := c.fixture.run
		path := c.artifacts.paths.descriptor
		until := clock.now().Add(time.Second)
		attempts := 0
		_, err := withDatabaseDeadline(ctx, c.fixture.conn, until, func(ctx context.Context, conn *dbConn) (struct{}, error) {
			pinned, err := recoveryDescriptorPin(ctx, conn, before, c.held, until, clock)
			if err != nil {
				return struct{}{}, err
			}
			if _, err := loadDescriptor(ctx, conn, path); err == nil {
				t.Errorf("%d takeovers: launch-time reading still requires exact creator ownership", takeovers)
			}
			return struct{}{}, pinned.remove(ctx, conn, before, c.held, until, clock, func(selected string, deadline time.Time) error {
				attempts++
				if selected != path || !deadline.Equal(until) || !clock.now().Before(deadline) {
					t.Errorf("%d takeovers: unlink(%s, %v) at %v", takeovers, selected, deadline, clock.now())
				}
				return removeOwned(selected)
			})
		})
		if err != nil {
			t.Fatalf("%d takeovers: %v", takeovers, err)
		}
		if attempts != 1 || exists(t, path) {
			t.Fatalf("%d takeovers: %d attempts, descriptor present %v", takeovers, attempts, exists(t, path))
		}
		assertRunUnchanged(t, c.fixture.conn, before, "pin and removal change no run state")
	}
}

func TestRuntimeArtifacts_RunnerRecoveryDescriptorPinDropPreservesPathForExplicitCompletion(t *testing.T) {
	t.Parallel()
	clock := testClockStartingAt(time.Now())
	c := describedRun(t)
	original := ownedBytes(t, c)
	if _, err := recoveryDescriptorPin(context.Background(), c.fixture.conn, c.fixture.run, c.held, clock.now().Add(time.Second), clock); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(ownedBytes(t, c), original) {
		t.Fatal("pin ownership alone schedules no pathname removal")
	}
}

func TestRuntimeArtifacts_RunnerRecoveryDescriptorRemoveRevalidatesOwnerAndExpiredDeadlineBeforeDispatch(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	for _, changed := range []string{"epoch", "deadline"} {
		clock := testClockStartingAt(time.Now())
		c := describedRun(t)
		before := pathIdentity(t, c.artifacts.paths.descriptor)
		until := clock.now().Add(time.Second)
		pinned, err := recoveryDescriptorPin(ctx, c.fixture.conn, c.fixture.run, c.held, until, clock)
		if err != nil {
			t.Fatal(err)
		}
		deadline := until
		switch changed {
		case "epoch":
			bumpEpoch(t, c.fixture.conn, c.fixture.run.runID)
		case "deadline":
			deadline = clock.now().Add(-time.Millisecond)
		}
		dispatched := false
		err = pinned.remove(ctx, c.fixture.conn, c.fixture.run, c.held, deadline, clock, func(string, time.Time) error {
			dispatched = true
			return nil
		})
		if err == nil {
			t.Fatalf("changed %s fails descriptor removal before dispatch", changed)
		}
		secretFree(t, err.Error())
		if dispatched || pathIdentity(t, c.artifacts.paths.descriptor) != before {
			t.Fatalf("%s: dispatched %v or the descriptor changed", changed, dispatched)
		}
	}
}

func TestRuntimeArtifacts_RunnerRecoveryActualDescriptorUnlinkClosureFailurePreservesFileAndRetries(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	clock := testClockStartingAt(time.Now())
	c := describedRun(t)
	original := ownedBytes(t, c)
	until := clock.now().Add(time.Second)
	pinned, err := recoveryDescriptorPin(ctx, c.fixture.conn, c.fixture.run, c.held, until, clock)
	if err != nil {
		t.Fatal(err)
	}
	attempts := 0
	err = pinned.remove(ctx, c.fixture.conn, c.fixture.run, c.held, until, clock, func(_ string, deadline time.Time) error {
		attempts++
		if !deadline.Equal(until) {
			t.Errorf("deadline = %v, want %v", deadline, until)
		}
		return appError("owned descriptor unlink-dispatch failure")
	})
	if err == nil || !strings.Contains(err.Error(), "owned descriptor unlink-dispatch failure") {
		t.Fatalf("unlink failure remains explicit: %v", err)
	}
	secretFree(t, err.Error())
	if attempts != 1 || !bytes.Equal(ownedBytes(t, c), original) {
		t.Fatalf("a failed unlink leaves its selected descriptor: %d attempts", attempts)
	}
	err = pinned.remove(ctx, c.fixture.conn, c.fixture.run, c.held, until, clock, func(path string, deadline time.Time) error {
		attempts++
		if !deadline.Equal(until) {
			t.Errorf("deadline = %v, want %v", deadline, until)
		}
		if err := os.Remove(path); err != nil {
			return appError("owned descriptor retry failed")
		}
		return nil
	})
	if err != nil || attempts != 2 || exists(t, c.artifacts.paths.descriptor) {
		t.Fatalf("retry: %v, %d attempts", err, attempts)
	}
}

func identityOf(t *testing.T, conn *dbConn) jsontext.Value {
	t.Helper()
	identity, err := databaseIdentity(context.Background(), conn)
	if err != nil {
		t.Fatal(err)
	}
	return identity
}

func pathsFor(t *testing.T, identity jsontext.Value, runID int64) runtimePaths {
	t.Helper()
	paths, err := runtimePathsForRun(identity, runID)
	if err != nil {
		t.Fatal(err)
	}
	return paths
}

func connectedTo(t *testing.T, path string) *dbConn {
	t.Helper()
	conn, err := connect(databaseURLFor(path))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	return conn
}

func copyFile(t *testing.T, from, to string) {
	t.Helper()
	data, err := os.ReadFile(from)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(to, data, 0o600); err != nil {
		t.Fatal(err)
	}
}

func mustRename(t *testing.T, from, to string) {
	t.Helper()
	if err := os.Rename(from, to); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_RunnerArtifactNamespaceIsDatabaseAdjacentAndSeparatesSiblingDatabases(t *testing.T) {
	t.Parallel()
	first := newRunnerFixture(t)
	second, _ := initializedConnAt(t, filepath.Join(first.dir, "sibling.db"))
	a := pathsFor(t, identityOf(t, first.conn), 1)
	b := pathsFor(t, identityOf(t, second), 1)
	if a.namespaceDir == b.namespaceDir || a.runDir == b.runDir || a.controlSocket == b.controlSocket {
		t.Fatalf("sibling databases share a namespace: %+v %+v", a, b)
	}
	canonical, err := filepath.EvalSymlinks(first.dir)
	if err != nil {
		t.Fatal(err)
	}
	if got, want := filepath.Dir(a.namespaceDir), filepath.Join(canonical, ".cafleet-runs"); got != want {
		t.Fatalf("namespace parent = %s, want %s", got, want)
	}
	if len(filepath.Base(a.namespaceDir)) != 26 || filepath.Base(a.runDir) != "1" {
		t.Fatalf("paths = %+v", a)
	}
}

func TestRuntimeArtifacts_RunnerArtifactIdentityChangesForCopiedOrReplacedDatabaseFiles(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	original := identityOf(t, fixture.conn)
	copied := filepath.Join(fixture.dir, "copy.db")
	selected := fixture.databaseFile(t)
	copyFile(t, selected, copied)
	copiedIdentity := identityOf(t, connectedTo(t, copied))
	assertJSONEqual(t, jsonAt(t, copiedIdentity, "instance_id"), jsonAt(t, original, "instance_id"), "a copy keeps the instance id")
	if pathsFor(t, original, 1).namespaceDir == pathsFor(t, copiedIdentity, 1).namespaceDir {
		t.Fatal("a copied database has its own namespace")
	}
	mustRename(t, selected, filepath.Join(fixture.dir, "old.db"))
	copyFile(t, copied, selected)
	replacement := identityOf(t, connectedTo(t, selected))
	assertJSONEqual(t, jsonAt(t, replacement, "instance_id"), jsonAt(t, original, "instance_id"), "a replacement keeps the instance id")
	if jsonEqual(jsonAt(t, replacement, "inode"), jsonAt(t, original, "inode")) {
		t.Fatal("a replacement file has another inode")
	}
	if pathsFor(t, original, 1).namespaceDir == pathsFor(t, replacement, 1).namespaceDir {
		t.Fatal("a replaced database has its own namespace")
	}
}

func TestRuntimeArtifacts_RunnerArtifactAllocationPinsPrivateDirectoryAndLockInodes(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	for _, directory := range []string{artifacts.paths.namespaceDir, artifacts.paths.runDir} {
		if mode := fileMode(t, directory); mode != 0o700 {
			t.Fatalf("%s mode %o", directory, mode)
		}
	}
	if mode := fileMode(t, artifacts.paths.ownerLock); mode != 0o600 {
		t.Fatalf("owner lock mode %o", mode)
	}
	if fixture.run.runtimeDirIdentityJSON == nil || fixture.run.runLockIdentityJSON == nil {
		t.Fatalf("the run pins its directory and lock identities: %+v", fixture.run)
	}
	if fixture.run.runtimePath != artifacts.paths.runDir {
		t.Fatalf("runtime path = %s, want %s", fixture.run.runtimePath, artifacts.paths.runDir)
	}
}

func TestRuntimeArtifacts_RunnerArtifactExistingDirectoryAndConflictingNamespaceManifestArePreserved(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	sentinel := filepath.Join(artifacts.paths.runDir, "retained")
	if err := os.WriteFile(sentinel, []byte("existing-owner-data"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := createRunArtifacts(ctx, fixture.conn, fixture.run.runID); err == nil {
		t.Fatal("a second allocation of an existing run directory is refused")
	}
	if data, err := os.ReadFile(sentinel); err != nil || string(data) != "existing-owner-data" {
		t.Fatalf("sentinel = %q %v", data, err)
	}
	other := newRunnerFixture(t)
	paths := pathsFor(t, other.run.databaseIdentityJSON, other.run.runID)
	if err := os.MkdirAll(paths.namespaceDir, 0o700); err != nil {
		t.Fatal(err)
	}
	for _, directory := range []string{filepath.Dir(paths.namespaceDir), paths.namespaceDir} {
		if err := os.Chmod(directory, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	foreign := testJSONWith(t, other.run.databaseIdentityJSON, mustJSON(t, strings.Repeat("0", 32)), "instance_id")
	if err := os.WriteFile(paths.namespaceManifest, foreign, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(paths.namespaceManifest, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := createRunArtifacts(ctx, other.conn, other.run.runID); err == nil {
		t.Fatal("a conflicting namespace manifest is refused")
	}
	if data, err := os.ReadFile(paths.namespaceManifest); err != nil || !bytes.Equal(data, foreign) {
		t.Fatalf("manifest = %q %v", data, err)
	}
	if exists(t, paths.runDir) {
		t.Fatal("a refused allocation creates no run directory")
	}
}

func TestRuntimeArtifacts_RunnerArtifactAllocationRejectsASymlinkedRuntimeNamespace(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	paths := pathsFor(t, fixture.run.databaseIdentityJSON, fixture.run.runID)
	target := filepath.Join(fixture.dir, "foreign")
	if err := os.Mkdir(target, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Dir(paths.namespaceDir), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, paths.namespaceDir); err != nil {
		t.Fatal(err)
	}
	if _, err := createRunArtifacts(context.Background(), fixture.conn, fixture.run.runID); err == nil {
		t.Fatal("a symlinked namespace is refused")
	}
	if entries, err := os.ReadDir(target); err != nil || len(entries) != 0 {
		t.Fatalf("the symlink target is untouched: %v %v", entries, err)
	}
}

func writeFixtureDescriptor(t *testing.T, fixture *runnerFixture, artifacts *runArtifacts, spec spawnSpec) {
	t.Helper()
	descriptor, err := newLaunchDescriptor(fixture.run, spec)
	if err != nil {
		t.Fatal(err)
	}
	if err := artifacts.writeDescriptor(descriptor); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_RunnerDescriptorRoundTripsLiteralArgvCwdEnvironmentAndPrivatePermissions(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	spec := fixture.spec()
	writeFixtureDescriptor(t, fixture, artifacts, spec)
	if mode := fileMode(t, artifacts.paths.descriptor); mode != 0o600 {
		t.Fatalf("descriptor mode %o", mode)
	}
	read, err := loadDescriptor(context.Background(), fixture.conn, artifacts.paths.descriptor)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(read.Spawn.argv, spec.argv) || read.Spawn.cwd != spec.cwd || !slices.Equal(read.Spawn.environment, spec.environment) || read.Spawn.size != spec.size {
		t.Fatalf("spawn = %+v, want %+v", read.Spawn, spec)
	}
	if read.Identity.RunID != fixture.run.runID || read.Identity.LaunchNonce != fixture.run.launchNonce || read.Identity.OwnerEpoch != fixture.run.ownerEpoch {
		t.Fatalf("identity = %+v", read.Identity)
	}
	for range 2 {
		if err := artifacts.removeDescriptor(); err != nil {
			t.Fatal(err)
		}
	}
	if exists(t, artifacts.paths.descriptor) {
		t.Fatal("the descriptor is removed")
	}
}

type descriptorDamage uint8

const (
	descriptorDamageMalformed descriptorDamage = iota + 1
	descriptorDamageNonce
	descriptorDamageEpoch
	descriptorDamageRun
	descriptorDamageDatabase
	descriptorDamageWorldReadable
	descriptorDamageSymlink
)

func damageDescriptor(t *testing.T, fixture *runnerFixture, artifacts *runArtifacts, damage descriptorDamage) {
	t.Helper()
	path := artifacts.paths.descriptor
	value, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var damaged jsontext.Value
	switch damage {
	case descriptorDamageMalformed:
		damaged = jsontext.Value("{")
	case descriptorDamageWorldReadable:
		if err := os.Chmod(path, 0o644); err != nil {
			t.Fatal(err)
		}
		return
	case descriptorDamageSymlink:
		moved := filepath.Join(artifacts.paths.runDir, "moved-descriptor")
		mustRename(t, path, moved)
		if err := os.Symlink(moved, path); err != nil {
			t.Fatal(err)
		}
		return
	case descriptorDamageNonce:
		damaged = testJSONWith(t, value, mustJSON(t, strings.Repeat("b", 64)), "identity", "launch_nonce")
	case descriptorDamageEpoch:
		damaged = testJSONWith(t, value, mustJSON(t, fixture.run.ownerEpoch+1), "identity", "owner_epoch")
	case descriptorDamageRun:
		damaged = testJSONWith(t, value, mustJSON(t, fixture.run.runID+1), "identity", "run_id")
	case descriptorDamageDatabase:
		damaged = testJSONWith(t, value, mustJSON(t, strings.Repeat("0", 32)), "identity", "database_identity_json", "instance_id")
	default:
		panic("invalid descriptorDamage")
	}
	if err := os.WriteFile(path, damaged, 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_RunnerDescriptorLoadRefusesEveryDamagedOrMismatchedLaunchInput(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		damage descriptorDamage
	}{
		{"malformed JSON", descriptorDamageMalformed},
		{"a different launch nonce", descriptorDamageNonce},
		{"a different owner epoch", descriptorDamageEpoch},
		{"a different run", descriptorDamageRun},
		{"a different database instance", descriptorDamageDatabase},
		{"world-readable permissions", descriptorDamageWorldReadable},
		{"a symlink to the moved descriptor", descriptorDamageSymlink},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			fixture := newRunnerFixture(t)
			artifacts := fixture.artifacts(t)
			writeFixtureDescriptor(t, fixture, artifacts, fixture.spec())
			damageDescriptor(t, fixture, artifacts, tc.damage)
			if _, err := loadDescriptor(context.Background(), fixture.conn, artifacts.paths.descriptor); err == nil {
				t.Fatalf("%s: the damaged descriptor is refused", tc.name)
			}
		})
	}
}

func fileUID(t *testing.T, path string) uint32 {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		t.Fatalf("%s has no stat record", path)
	}
	return stat.Uid
}

func TestRuntimeArtifacts_RunnerPrivateMetadataValidationRejectsADifferentOwnerWithoutPrivilegedChown(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	owner := fileUID(t, artifacts.paths.ownerLock)
	if err := validatePrivateFile(artifacts.paths.ownerLock, owner, 0o600); err != nil {
		t.Fatal(err)
	}
	if owner == ^uint32(0) {
		t.Fatal("the owner uid has a successor")
	}
	if err := validatePrivateFile(artifacts.paths.ownerLock, owner+1, 0o600); err == nil {
		t.Fatal("another owner is refused")
	}
}

func TestRuntimeArtifacts_RunnerLockKeepsTheOriginalInodeExclusiveEvenIfItsPathIsUnlinked(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	owner := fixture.run.owner()
	held := acquireRunLock(t, fixture.conn, owner)
	if again, err := runLockAcquire(ctx, fixture.conn, owner); err != nil || again != none[*runLock]() {
		t.Fatalf("a held lock is not acquired again: %v %v", again, err)
	}
	original := pathIdentity(t, artifacts.paths.ownerLock).inode
	if err := os.Remove(artifacts.paths.ownerLock); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(artifacts.paths.ownerLock, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(artifacts.paths.ownerLock, 0o600); err != nil {
		t.Fatal(err)
	}
	if pathIdentity(t, artifacts.paths.ownerLock).inode == original {
		t.Fatal("the replacement lock has another inode")
	}
	if _, err := runLockAcquire(ctx, fixture.conn, owner); err == nil {
		t.Fatal("the replaced lock path is refused")
	}
	if err := held.validate(ctx, fixture.conn, owner); err == nil {
		t.Fatal("the held lock no longer matches its path")
	}
	if got := mustRun(t, fixture.conn, owner.runID).ownerEpoch; got != owner.ownerEpoch {
		t.Fatalf("owner epoch = %d, want %d", got, owner.ownerEpoch)
	}
}

func TestRuntimeArtifacts_RunnerLockTransferAdvancesEpochAndFencesTheOldOwner(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	fixture.artifacts(t)
	owner := fixture.run.owner()
	acquired, err := runLockAcquire(ctx, fixture.conn, owner)
	if err != nil {
		t.Fatal(err)
	}
	held, ok := acquired.get()
	if !ok {
		t.Fatal("the run lock is free")
	}
	next, err := held.transferToRunner(ctx, fixture.conn, owner, currentProcess(t))
	if err != nil {
		t.Fatal(err)
	}
	if next.ownerEpoch != owner.ownerEpoch+1 || next.ownerKind != ownerKindRunner {
		t.Fatalf("next = %+v", next)
	}
	if err := held.validate(ctx, fixture.conn, owner); err == nil {
		t.Fatal("the old owner is fenced")
	}
	if err := held.validate(ctx, fixture.conn, next); err != nil {
		t.Fatal(err)
	}
	if again, err := runLockAcquire(ctx, fixture.conn, next); err != nil || again != none[*runLock]() {
		t.Fatalf("the transferred lock stays held: %v %v", again, err)
	}
	if err := held.close(); err != nil {
		t.Fatal(err)
	}
	released, err := runLockAcquireReleased(ctx, fixture.conn, next)
	if err != nil {
		t.Fatal(err)
	}
	lock, ok := released.get()
	if !ok {
		t.Fatal("the released lock is acquired")
	}
	if err := lock.close(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_RunnerArtifactPathsValidateCompleteSocketLengthBeforeCreation(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	long := testJSONWith(t, fixture.run.databaseIdentityJSON, mustJSON(t, "/private/tmp/"+strings.Repeat("x", 240)+"/registry.db"), "canonical_path")
	if _, err := runtimePathsForRun(long, 1); err == nil {
		t.Fatal("an overlong socket path is refused")
	}
	for _, run := range []int64{0, -1} {
		if _, err := runtimePathsForRun(fixture.run.databaseIdentityJSON, run); err == nil {
			t.Fatalf("run %d is refused", run)
		}
	}
}

func TestRuntimeArtifacts_RunnerDatabaseReplacementFencesDescriptorsAndOwnedLockChecks(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	writeFixtureDescriptor(t, fixture, artifacts, fixture.spec())
	owner := fixture.run.owner()
	held := acquireRunLock(t, fixture.conn, owner)
	selected := fixture.databaseFile(t)
	replacement := filepath.Join(fixture.dir, "replacement.db")
	copyFile(t, selected, replacement)
	mustRename(t, selected, filepath.Join(fixture.dir, "old.db"))
	mustRename(t, replacement, selected)
	if err := held.validate(ctx, fixture.conn, owner); err == nil {
		t.Fatal("a replaced database fences the held lock")
	}
	if _, err := loadDescriptor(ctx, fixture.conn, artifacts.paths.descriptor); err == nil {
		t.Fatal("a replaced database fences the descriptor")
	}
	if !exists(t, artifacts.paths.runDir) {
		t.Fatal("the run directory stays")
	}
	if got := queryInt64(t, fixture.conn, "SELECT owner_epoch FROM member_runs WHERE run_id="+strconv.FormatInt(owner.runID, 10)); got != owner.ownerEpoch {
		t.Fatalf("owner epoch = %d, want %d", got, owner.ownerEpoch)
	}
}

func TestRuntimeArtifacts_RunnerArtifactCanonicalAliasesShareOneDatabaseNamespace(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	alias := filepath.Join(fixture.dir, "database-alias.db")
	if err := os.Symlink(fixture.databaseFile(t), alias); err != nil {
		t.Fatal(err)
	}
	identity := identityOf(t, connectedTo(t, alias))
	assertJSONEqual(t, identity, fixture.run.databaseIdentityJSON, "an alias resolves to the canonical identity")
	if pathsFor(t, identity, 1).runDir != pathsFor(t, fixture.run.databaseIdentityJSON, 1).runDir {
		t.Fatal("aliases share one run directory")
	}
}

func TestRuntimeArtifacts_RunnerDescriptorRemovalReportsAnUnlinkFailureWithoutClaimingCleanup(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	writeFixtureDescriptor(t, fixture, artifacts, fixture.spec())
	retained := filepath.Join(artifacts.paths.runDir, "retained-descriptor")
	mustRename(t, artifacts.paths.descriptor, retained)
	if err := os.Mkdir(artifacts.paths.descriptor, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := artifacts.removeDescriptor(); err == nil {
		t.Fatal("a directory at the descriptor path is not reported removed")
	}
	if info, err := os.Stat(retained); err != nil || !info.Mode().IsRegular() {
		t.Fatalf("the retained descriptor stays: %v", err)
	}
	if info, err := os.Stat(artifacts.paths.descriptor); err != nil || !info.IsDir() {
		t.Fatalf("the directory stays: %v", err)
	}
}

func TestRuntimeArtifacts_RunnerDescriptorConstructionValidatesRequiredSpawnInputsBeforeWriting(t *testing.T) {
	t.Parallel()
	for _, missing := range []string{"argv", "cwd", "rows", "cols"} {
		fixture := newRunnerFixture(t)
		artifacts := fixture.artifacts(t)
		spec := fixture.spec()
		switch missing {
		case "argv":
			spec.argv = nil
		case "cwd":
			spec.cwd = "relative"
		case "rows":
			spec.size.rows = 0
		case "cols":
			spec.size.cols = 0
		}
		if _, err := newLaunchDescriptor(fixture.run, spec); err == nil {
			t.Fatalf("%s: the descriptor is refused", missing)
		}
		if exists(t, artifacts.paths.descriptor) {
			t.Fatalf("%s: no descriptor is written", missing)
		}
	}
}

type lockedRun struct {
	fixture   *runnerFixture
	artifacts *runArtifacts
	lock      *runLock
}

func newLockedRun(t *testing.T) lockedRun {
	t.Helper()
	fixture := newRunnerFixture(t)
	artifacts := fixture.artifacts(t)
	return lockedRun{fixture: fixture, artifacts: artifacts, lock: acquireRunLock(t, fixture.conn, fixture.run.owner())}
}

func TestRuntimeArtifacts_LockTransfer_RunnerRecoveryLockTransferCommitsFreshEpochsAndPreservesFirstOriginStatus(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	locked := newLockedRun(t)
	fixture := locked.fixture
	original := fixture.run
	process := currentProcess(t)
	for offset := int64(1); offset <= 2; offset++ {
		owner, err := locked.lock.transferToReconcilerWithDeadline(ctx, fixture.conn, fixture.run, process, time.Now().Add(time.Second))
		if err != nil {
			t.Fatal(err)
		}
		fixture.refresh(t)
		run := fixture.run
		if !owner.equal(run.owner()) || run.ownerEpoch != original.ownerEpoch+offset || run.ownerKind != ownerKindReconciler ||
			run.status != runStatusRecovering || run.recoveryOriginStatus != some(original.status) ||
			run.ownerPID != process.pid || run.ownerStartIdentity != process.startIdentity {
			t.Fatalf("takeover %d: run = %+v, owner = %+v", offset, run, owner)
		}
		assertJSONEqual(t, run.runtimeDirIdentityJSON, original.runtimeDirIdentityJSON, "the directory identity")
		assertJSONEqual(t, run.runLockIdentityJSON, original.runLockIdentityJSON, "the lock identity")
		if err := locked.lock.validate(ctx, fixture.conn, owner); err != nil {
			t.Fatal(err)
		}
	}
	if _, _, err := persistTransition(ctx, fixture.conn, original.owner(), opening(t, original.runID, 1, 1, runtimeMetadata(t)), persistenceModeLive); err == nil {
		t.Fatal("the original owner is fenced")
	}
}

func TestRuntimeArtifacts_LockTransfer_RunnerRecoveryLockTransferRejectsEveryChangedStableExpectedIdentity(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	locked := newLockedRun(t)
	before := locked.fixture.run
	expected := before
	expected.launchNonce = strings.Repeat("b", 64)
	if _, err := locked.lock.transferToReconcilerWithDeadline(ctx, locked.fixture.conn, expected, currentProcess(t), time.Now().Add(time.Second)); err == nil {
		t.Fatal("a changed stable identity is refused")
	}
	assertRunUnchanged(t, locked.fixture.conn, before, "the refused transfer")
	if err := locked.lock.validate(ctx, locked.fixture.conn, before.owner()); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_LockTransfer_RunnerRecoveryLockTransferCommitFailureReturnsNoUncommittedOwner(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	locked := newLockedRun(t)
	fixture := locked.fixture
	refuse := faultPlanRefuseCommit()
	refuse.arm(t, fixture.conn)
	_, err := locked.lock.transferToReconcilerWithDeadline(ctx, fixture.conn, fixture.run, currentProcess(t), time.Now().Add(time.Second))
	refuse.clear(t, fixture.conn)
	if err == nil {
		t.Fatal("a refused commit fails the transfer")
	}
	assertRunUnchanged(t, fixture.conn, fixture.run, "the refused commit")
	if err := locked.lock.validate(ctx, fixture.conn, fixture.run.owner()); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeArtifacts_LockTransfer_RunnerRecoveryLockTransferContentionUsesTheOriginalAbsoluteDeadline(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	locked := newLockedRun(t)
	fixture := locked.fixture
	writer := connectedTo(t, fixture.databaseFile(t))
	mustExecBatch(t, writer, "BEGIN IMMEDIATE")
	started := time.Now()
	_, err := locked.lock.transferToReconcilerWithDeadline(ctx, fixture.conn, fixture.run, currentProcess(t), started.Add(80*time.Millisecond))
	elapsed := time.Since(started)
	mustExecBatch(t, writer, "ROLLBACK")
	if err == nil || elapsed >= 750*time.Millisecond {
		t.Fatalf("contention fails at its deadline: %v after %v", err, elapsed)
	}
	assertRunUnchanged(t, fixture.conn, fixture.run, "the contended transfer")
}

func overwrite(t *testing.T, path string, data []byte) {
	t.Helper()
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_TRUNC, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.Write(data); err != nil {
		t.Fatal(err)
	}
	if err := file.Sync(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}

func pinCase(c *recoveryCase) (*recoveryDescriptor, error) {
	clock := testClockStartingAt(time.Now())
	return recoveryDescriptorPin(context.Background(), c.fixture.conn, c.fixture.run, c.held, clock.now().Add(time.Second), clock)
}

func removeCase(c *recoveryCase, pinned *recoveryDescriptor) (bool, error) {
	clock := testClockStartingAt(time.Now())
	dispatched := false
	err := pinned.remove(context.Background(), c.fixture.conn, c.fixture.run, c.held, clock.now().Add(time.Second), clock, func(path string, _ time.Time) error {
		dispatched = true
		return removeOwned(path)
	})
	return dispatched, err
}

type descriptorChange uint8

const (
	descriptorChangeReplacement descriptorChange = iota + 1
	descriptorChangeSymlink
	descriptorChangeContent
	descriptorChangeAppeared
)

func TestRuntimeArtifacts_RecoveryDescriptor_RecoveryDescriptorRemovePreservesADescriptorChangedAfterItsPin(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		change descriptorChange
	}{
		{"replacement", descriptorChangeReplacement},
		{"symlink", descriptorChangeSymlink},
		{"content", descriptorChangeContent},
		{"appeared", descriptorChangeAppeared},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			c := describedRun(t)
			original := ownedBytes(t, c)
			path := c.artifacts.paths.descriptor
			retained := filepath.Join(c.artifacts.paths.runDir, "retained-launch.json")
			if tc.change == descriptorChangeAppeared {
				if err := c.artifacts.removeDescriptor(); err != nil {
					t.Fatal(err)
				}
			}
			pinned, err := pinCase(c)
			if err != nil {
				t.Fatal(err)
			}
			expected := original
			switch tc.change {
			case descriptorChangeReplacement:
				mustRename(t, path, retained)
				writePrivate(t, path, original)
				if pathIdentity(t, path).inode == pathIdentity(t, retained).inode {
					t.Fatal("the replacement has another inode")
				}
			case descriptorChangeSymlink:
				mustRename(t, path, retained)
				if err := os.Symlink(retained, path); err != nil {
					t.Fatal(err)
				}
			case descriptorChangeContent:
				var argv []string
				if err := json.Unmarshal(jsonAt(t, original, "spawn", "argv"), &argv); err != nil {
					t.Fatal(err)
				}
				expected = testJSONWith(t, original, mustJSON(t, append(argv, "owned-in-place-change")), "spawn", "argv")
				before := pathIdentity(t, path)
				overwrite(t, path, expected)
				if after := pathIdentity(t, path); after.device != before.device || after.inode != before.inode {
					t.Fatal("an in-place change keeps the inode")
				}
			case descriptorChangeAppeared:
				writePrivate(t, path, original)
			}
			named := pathIdentity(t, path)
			dispatched, err := removeCase(c, pinned)
			if err == nil {
				t.Fatalf("%s: a descriptor changed after its pin is refused", tc.name)
			}
			secretFree(t, err.Error())
			if dispatched || pathIdentity(t, path) != named {
				t.Fatalf("%s: dispatched %v or the named file changed", tc.name, dispatched)
			}
			read := path
			if tc.change == descriptorChangeSymlink {
				if info, err := os.Lstat(path); err != nil || info.Mode()&os.ModeSymlink == 0 {
					t.Fatalf("the symlink stays: %v", err)
				}
				read = retained
			}
			if data, err := os.ReadFile(read); err != nil || !bytes.Equal(data, expected) {
				t.Fatalf("%s: content = %q %v", tc.name, data, err)
			}
		})
	}
}

func TestRuntimeArtifacts_RecoveryDescriptor_RecoveryDescriptorDisappearingAfterItsPinIsASafeAbsence(t *testing.T) {
	t.Parallel()
	c := describedRun(t)
	pinned, err := pinCase(c)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(c.artifacts.paths.descriptor); err != nil {
		t.Fatal(err)
	}
	dispatched, err := removeCase(c, pinned)
	if err != nil || dispatched || exists(t, c.artifacts.paths.descriptor) {
		t.Fatalf("a vanished descriptor is a safe absence: %v, dispatched %v", err, dispatched)
	}
}

func TestRuntimeArtifacts_RecoveryDescriptor_RecoveryDescriptorPinRefusesAnInvalidDescriptorPrivately(t *testing.T) {
	t.Parallel()
	for _, damage := range []string{"size", "json", "member", "permissions", "symlink"} {
		t.Run(damage, func(t *testing.T) {
			t.Parallel()
			c := describedRun(t)
			path := c.artifacts.paths.descriptor
			value := ownedBytes(t, c)
			switch damage {
			case "size":
				overwrite(t, path, bytes.Repeat([]byte(" "), 1024*1024+1))
			case "json":
				overwrite(t, path, []byte("{"))
			case "member":
				overwrite(t, path, testJSONWith(t, value, mustJSON(t, c.fixture.run.memberID+1), "identity", "member_id"))
			case "permissions":
				if err := os.Chmod(path, 0o644); err != nil {
					t.Fatal(err)
				}
			case "symlink":
				retained := filepath.Join(c.artifacts.paths.runDir, "invalid-target.json")
				mustRename(t, path, retained)
				if err := os.Symlink(retained, path); err != nil {
					t.Fatal(err)
				}
			}
			before := pathIdentity(t, path)
			_, err := pinCase(c)
			if err == nil {
				t.Fatalf("an invalid descriptor is refused at pin: %s", damage)
			}
			secretFree(t, err.Error())
			if pathIdentity(t, path) != before {
				t.Fatalf("%s: the refused pin changed the descriptor", damage)
			}
		})
	}
}
