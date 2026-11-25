package core

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

const (
	releasedLockGrace = 2 * time.Second
	testWorkReserve   = 3500 * time.Millisecond
)

// paneLockAcquireReleased acquires a lock the caller has just seen released. A child forked by
// a concurrent test shares the released descriptor's flock until its exec closes it, so it
// retries briefly.
func paneLockAcquireReleased(t *testing.T, databaseURL string, p placement) *paneLock {
	t.Helper()
	held, acquired, err := paneLockAcquireWithin(databaseURL, p, deadlineWithin(releasedLockGrace))
	if err != nil || !acquired {
		t.Fatalf("the released pane lock is acquired: %v %v", acquired, err)
	}
	return held
}

// lockFixture is a database with one fleet whose worker has the concrete pane %2.
type lockFixture struct {
	dir      string
	url      string
	conn     *dbConn
	settings settings
	director memberRecord
	member   memberRecord
}

func newLockFixture(t *testing.T) lockFixture {
	t.Helper()
	conn, url := initializedFileConn(t)
	fleet, director := createFleetFixture(t, conn, "locks")
	worker := register(t, conn, fleet, "worker", some("%2"))
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	return lockFixture{
		dir:      filepath.Dir(path),
		url:      url,
		conn:     conn,
		settings: settingsForDatabase(path),
		director: requireMember(t, conn, director, fleet),
		member:   requireMember(t, conn, worker, fleet),
	}
}

func (f lockFixture) hold(t *testing.T) *paneLock {
	t.Helper()
	held, acquired, err := paneLockTryAcquire(f.url, f.member.placement)
	if err != nil || !acquired {
		t.Fatalf("the fixture lock is free: %v %v", acquired, err)
	}
	return held
}

// holdFor holds the fixture lock on another goroutine for hold; the returned channel closes once
// it is released.
func (f lockFixture) holdFor(t *testing.T, hold time.Duration) <-chan struct{} {
	t.Helper()
	held := f.hold(t)
	released := make(chan struct{})
	go func() {
		time.Sleep(hold)
		held.close()
		close(released)
	}()
	return released
}

func writerDeadline() deadline {
	return newDeadline().reserving(testWorkReserve)
}

func TestPaneLock_PaneGuard_RunnerRecoveryPaneGuardValidatesTheActualDatabasePlacementAndNamedInode(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	p := fixture.member.placement
	held := fixture.hold(t)
	defer held.close()
	if err := held.validateBinding(fixture.url, p); err != nil {
		t.Fatalf("the held lock binds its own placement: %v", err)
	}
	changed := p
	changed.muxPaneID = some("%foreign")
	if err := held.validateBinding(fixture.url, changed); err == nil {
		t.Fatal("the held lock refuses another pane")
	}
	siblingURL := databaseURLFor(filepath.Join(fixture.dir, "sibling.db"))
	sibling, err := connect(siblingURL)
	if err != nil {
		t.Fatal(err)
	}
	if err := initialize(sibling, siblingURL); err != nil {
		t.Fatal(err)
	}
	closeConnection(t, sibling)
	if err := held.validateBinding(siblingURL, p); err == nil {
		t.Fatal("the held lock refuses another database")
	}
	path, err := lockPath(fixture.url, p)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Rename(path, path+".retained"); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("replacement lock identity"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := held.validateBinding(fixture.url, p); err == nil {
		t.Fatal("the held lock refuses a replaced lock file")
	}
	if content, err := os.ReadFile(path); err != nil || string(content) != "replacement lock identity" {
		t.Fatalf("validation leaves the replacement untouched: %q %v", content, err)
	}
}

func TestPaneLock_PaneGuard_RunnerRecoveryPaneGuardSelectionReusesADeduplicatedPhysicalGuard(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	p := requireMember(t, fixture.conn, fixture.run.memberID, fixture.run.fleetID).placement
	held, acquired, err := paneLockTryAcquire(fixture.url, p)
	if err != nil || !acquired {
		t.Fatalf("the member pane lock is free: %v %v", acquired, err)
	}
	defer held.close()
	guards := []*paneLock{held}
	shared := p
	shared.muxWindowID = some("@shared")
	for _, tc := range []struct {
		name      string
		placement placement
	}{
		{"the recorded placement", p},
		{"a placement sharing the physical pane", shared},
	} {
		selected, err := paneLockFor(guards, fixture.url, tc.placement)
		if err != nil || selected != guards[0] {
			t.Fatalf("%s: selected %p, want the held guard %p: %v", tc.name, selected, guards[0], err)
		}
	}
	assertPaneLockBusy(t, settingsForDatabase(filepath.Join(fixture.dir, "broker.db")), p)
	if _, err := paneLockFor(nil, fixture.url, p); err == nil {
		t.Fatal("selection without a held guard is refused")
	}
}

// directorAndMemberInKeyOrder is the fixture's Director and member placements, in physical
// lock-key order.
func directorAndMemberInKeyOrder(t *testing.T, fixture lockFixture) []memberPlacement {
	t.Helper()
	targets := []memberPlacement{
		{memberID: fixture.director.memberID, placement: fixture.director.placement},
		{memberID: fixture.member.memberID, placement: fixture.member.placement},
	}
	key := func(target memberPlacement) string {
		path, err := lockPath(fixture.settings.databaseURL, target.placement)
		if err != nil {
			t.Fatal(err)
		}
		return path
	}
	slices.SortFunc(targets, func(left, right memberPlacement) int { return strings.Compare(key(left), key(right)) })
	return targets
}

func TestPaneLock_DeletionLocks_DeletionDeduplicatesPhysicalPanesAndRevalidatesAfterAllLocks(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	p := fixture.member.placement
	member := fixture.member.memberID
	url := fixture.settings.databaseURL
	locked, err := lockDeletion(context.Background(), fixture.conn, fixture.settings, newDeadline(),
		func(context.Context, *dbConn) (struct{}, []memberPlacement, error) {
			return struct{}{}, []memberPlacement{{memberID: member, placement: p}, {memberID: member + 1, placement: p}}, nil
		},
		func(context.Context, *dbConn, *struct{}, []*paneLock) (struct{}, bool, error) {
			assertPaneLockBusy(t, fixture.settings, p)
			return struct{}{}, true, nil
		},
		func(int64) error { return appError("duplicate pane remained locked") },
	)
	if err != nil {
		t.Fatal(err)
	}
	assertPaneLockBusy(t, fixture.settings, p)
	locked.close()
	paneLockAcquireReleased(t, url, p).close()
}

func TestPaneLock_DeletionLocks_DeletionAcquiresPhysicalKeysInSortedOrder(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	targets := directorAndMemberInKeyOrder(t, fixture)
	url := fixture.settings.databaseURL
	highGuard, acquired, err := paneLockTryAcquire(url, targets[1].placement)
	if err != nil || !acquired {
		t.Fatalf("the higher key is free: %v %v", acquired, err)
	}
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	loaded := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		reversed := slices.Clone(targets)
		slices.Reverse(reversed)
		locked, err := lockDeletion(context.Background(), conn, fixture.settings, deadlineWithin(2*time.Second),
			func(context.Context, *dbConn) (struct{}, []memberPlacement, error) {
				close(loaded)
				return struct{}{}, reversed, nil
			},
			func(context.Context, *dbConn, *struct{}, []*paneLock) (struct{}, bool, error) {
				return struct{}{}, true, nil
			},
			func(int64) error { return appError("sorted lock wait expired") },
		)
		if err == nil {
			locked.close()
		}
		done <- err
	}()
	select {
	case <-loaded:
	case <-time.After(time.Second):
		t.Fatal("the deletion loads its targets")
	}
	limit := time.Now().Add(700 * time.Millisecond)
	for {
		held, acquired, err := paneLockTryAcquire(url, targets[0].placement)
		if err != nil {
			t.Fatal(err)
		}
		if !acquired {
			break
		}
		held.close()
		if !time.Now().Before(limit) {
			t.Fatal("the lower key is owned while the higher key waits")
		}
		time.Sleep(5 * time.Millisecond)
	}
	highGuard.close()
	if err := <-done; err != nil {
		t.Fatal(err)
	}
}

func TestPaneLock_PaneLockWaitTests_AnUncontendedLockIsAcquiredAtOnce(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	started := time.Now()
	held, acquired, err := paneLockAcquireWithin(fixture.url, fixture.member.placement, writerDeadline())
	if err != nil || !acquired {
		t.Fatalf("the uncontended lock is acquired: %v %v", acquired, err)
	}
	defer held.close()
	if elapsed := time.Since(started); elapsed >= 200*time.Millisecond {
		t.Fatalf("elapsed %v", elapsed)
	}
}

func TestPaneLock_PaneLockWaitTests_ALockReleasedWithinTheWaitIsAcquired(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	released := fixture.holdFor(t, 300*time.Millisecond)
	started := time.Now()
	held, acquired, err := paneLockAcquireWithin(fixture.url, fixture.member.placement, writerDeadline())
	waited := time.Since(started)
	<-released
	if err != nil || !acquired {
		t.Fatalf("the released lock is acquired: %v %v", acquired, err)
	}
	defer held.close()
	if waited < 250*time.Millisecond || waited >= time.Second {
		t.Fatalf("waited %v, want about the holder's 300 ms", waited)
	}
}

func TestPaneLock_PaneLockWaitTests_ALockStillBusyAtTheReservedDeadlineReturnsNone(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	held := fixture.hold(t)
	defer held.close()
	started := time.Now()
	_, acquired, err := paneLockAcquireWithin(fixture.url, fixture.member.placement, writerDeadline())
	waited := time.Since(started)
	if err != nil || acquired {
		t.Fatalf("a busy lock reports none: %v %v", acquired, err)
	}
	if waited < 1400*time.Millisecond || waited >= 2*time.Second {
		t.Fatalf("waited %v, want the reserved 1.5 s", waited)
	}
}

func TestPaneLock_PaneLockWaitTests_PhysicalLockIdentityUsesCanonicalDatabaseBackendSessionAndPane(t *testing.T) {
	t.Parallel()
	fixture := newLockFixture(t)
	p := fixture.member.placement
	database, err := databasePath(fixture.url)
	if err != nil {
		t.Fatal(err)
	}
	alias := filepath.Join(fixture.dir, "alias.db")
	if err := os.Symlink(database, alias); err != nil {
		t.Fatal(err)
	}
	aliasURL := databaseURLFor(alias)
	path, err := lockPath(fixture.url, p)
	if err != nil {
		t.Fatal(err)
	}
	if aliased, err := lockPath(aliasURL, p); err != nil || aliased != path {
		t.Fatalf("an aliased database shares the lock path: %q %v, want %q", aliased, err, path)
	}
	duplicate := p
	duplicate.muxWindowID = some("@99")
	duplicate.muxWindowName = some("cafleet-1-9")
	duplicate.windowOrdinal = 9
	duplicate.columnSide = columnSideLeft
	duplicate.codingAgent = "codex"
	if same, err := lockPath(fixture.url, duplicate); err != nil || same != path {
		t.Fatalf("window, slot and backend fields leave the lock path unchanged: %q %v, want %q", same, err, path)
	}
	alternatives := []struct {
		name  string
		apply func(*placement)
	}{
		{name: "backend", apply: func(alternative *placement) { alternative.backend = "herdr" }},
		{name: "session", apply: func(alternative *placement) { alternative.muxSession = "other" }},
		{name: "pane", apply: func(alternative *placement) { alternative.muxPaneID = some("%99") }},
	}
	for _, tc := range alternatives {
		alternative := p
		tc.apply(&alternative)
		other, err := lockPath(fixture.url, alternative)
		if err != nil || other == path {
			t.Fatalf("another %s has its own lock path: %q %v", tc.name, other, err)
		}
		held, acquired, err := paneLockTryAcquire(fixture.url, alternative)
		if err != nil || !acquired {
			t.Fatalf("another %s's lock is free: %v %v", tc.name, acquired, err)
		}
		held.close()
	}
	guard := fixture.hold(t)
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("lock file mode %v, want 0600", info.Mode().Perm())
	}
	if err := os.WriteFile(path, []byte("operator marker"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, busy, err := paneLockTryAcquire(aliasURL, duplicate); err != nil || busy {
		t.Fatalf("the aliased duplicate placement shares the held lock: %v %v", busy, err)
	}
	guard.close()
	reacquired := paneLockAcquireReleased(t, aliasURL, duplicate)
	defer reacquired.close()
	after, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if !os.SameFile(info, after) {
		t.Fatal("reacquisition keeps the same lock file")
	}
	if content, err := os.ReadFile(path); err != nil || string(content) != "operator marker" {
		t.Fatalf("acquisition preserves the lock file's bytes: %q %v", content, err)
	}
}

func layoutLockDatabase(t *testing.T) (string, string) {
	t.Helper()
	_, url := initializedFileConn(t)
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	return filepath.Dir(path), url
}

func deadlineWithin(wait time.Duration) deadline {
	return deadlineUntil(time.Now().Add(wait))
}

func mustTryLayoutLock(t *testing.T, name string, url string, fleetID int64) (*layoutLock, bool) {
	t.Helper()
	held, acquired, err := layoutLockTryAcquire(url, fleetID)
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return held, acquired
}

func TestPaneLock_LayoutLock_TheLayoutLockWaitsUpToSixtySeconds(t *testing.T) {
	t.Parallel()
	if layoutLockWait != 60*time.Second {
		t.Fatalf("layoutLockWait = %v, want 60s", layoutLockWait)
	}
}

func TestPaneLock_LayoutLock_TheLayoutLockLivesBesideTheCanonicalDatabaseKeyedByTheFleetDigest(t *testing.T) {
	t.Parallel()
	dir, url := layoutLockDatabase(t)
	path, err := databasePath(url)
	if err != nil {
		t.Fatal(err)
	}
	canonical, err := filepath.EvalSymlinks(path)
	if err != nil {
		t.Fatal(err)
	}
	expected := filepath.Join(canonical+".layout-locks", sha256Hex([]byte("7"))+".lock")
	if got, err := layoutLockPath(url, 7); err != nil || got != expected {
		t.Fatalf("layout lock path %q (%v), want %q", got, err, expected)
	}
	alias := filepath.Join(dir, "alias.db")
	if err := os.Symlink(path, alias); err != nil {
		t.Fatal(err)
	}
	aliasURL := databaseURLFor(alias)
	if got, err := layoutLockPath(aliasURL, 7); err != nil || got != expected {
		t.Fatalf("aliased layout lock path %q (%v), want %q", got, err, expected)
	}
	if got, err := layoutLockPath(url, 8); err != nil || got == expected {
		t.Fatalf("fleet 8's layout lock path %q (%v) equals fleet 7's", got, err)
	}
	if _, err := os.Stat(filepath.Dir(expected)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("the lock directory exists before any acquisition: %v", err)
	}
	held, acquired := mustTryLayoutLock(t, "first acquisition", url, 7)
	if !acquired {
		t.Fatal("the free layout lock is acquired")
	}
	defer held.close()
	info, err := os.Stat(expected)
	if err != nil {
		t.Fatal(err)
	}
	if !info.Mode().IsRegular() || info.Mode().Perm() != 0o600 {
		t.Fatalf("lock file mode %v, want a regular 0600 file", info.Mode())
	}
	if _, acquired := mustTryLayoutLock(t, "aliased acquisition", aliasURL, 7); acquired {
		t.Fatal("an aliased database path shares the fleet's layout lock")
	}
}

func TestPaneLock_LayoutLock_ASecondAcquisitionBlocksUntilTheHolderReleases(t *testing.T) {
	t.Parallel()
	_, url := layoutLockDatabase(t)
	held, acquired := mustTryLayoutLock(t, "holder", url, 1)
	if !acquired {
		t.Fatal("the free layout lock is acquired")
	}
	if _, busy := mustTryLayoutLock(t, "second try", url, 1); busy {
		t.Fatal("a held layout lock is busy")
	}
	started := time.Now()
	released := make(chan struct{})
	go func() {
		time.Sleep(300 * time.Millisecond)
		held.close()
		close(released)
	}()
	waiter, acquired, err := layoutLockAcquireWithin(url, 1, deadlineWithin(5*time.Second))
	waited := time.Since(started)
	<-released
	if err != nil || !acquired {
		t.Fatalf("the waiter acquires once the holder releases: %v %v", acquired, err)
	}
	defer waiter.close()
	if waited < 250*time.Millisecond || waited >= 4*time.Second {
		t.Fatalf("waited %v, want about the holder's 300 ms", waited)
	}
	if _, busy := mustTryLayoutLock(t, "after the wait", url, 1); busy {
		t.Fatal("the waiter now holds the layout lock")
	}
}

func TestPaneLock_LayoutLock_AnExpiredWaitReportsBusyAfterItsFullBudget(t *testing.T) {
	t.Parallel()
	_, url := layoutLockDatabase(t)
	held, acquired := mustTryLayoutLock(t, "holder", url, 1)
	if !acquired {
		t.Fatal("the free layout lock is acquired")
	}
	defer held.close()
	started := time.Now()
	_, acquired, err := layoutLockAcquireWithin(url, 1, deadlineWithin(300*time.Millisecond))
	waited := time.Since(started)
	if err != nil || acquired {
		t.Fatalf("an expired wait reports busy: %v %v", acquired, err)
	}
	if waited < 300*time.Millisecond || waited >= 2*time.Second {
		t.Fatalf("waited %v, want the full 300 ms budget", waited)
	}
}

func TestPaneLock_LayoutLock_DistinctFleetsDoNotContend(t *testing.T) {
	t.Parallel()
	_, url := layoutLockDatabase(t)
	first, acquired := mustTryLayoutLock(t, "fleet 1", url, 1)
	if !acquired {
		t.Fatal("the free layout lock is acquired")
	}
	defer first.close()
	started := time.Now()
	second, acquired, err := layoutLockAcquireWithin(url, 2, deadlineWithin(5*time.Second))
	if err != nil || !acquired {
		t.Fatalf("another fleet's layout lock is free: %v %v", acquired, err)
	}
	defer second.close()
	if elapsed := time.Since(started); elapsed >= time.Second {
		t.Fatalf("elapsed %v", elapsed)
	}
}

func TestPaneLock_LayoutLock_AReleasedLayoutLockIsFreeForTheNextHolder(t *testing.T) {
	t.Parallel()
	_, url := layoutLockDatabase(t)
	held, acquired := mustTryLayoutLock(t, "holder", url, 1)
	if !acquired {
		t.Fatal("the free layout lock is acquired")
	}
	held.close()
	next, acquired, err := layoutLockAcquireWithin(url, 1, deadlineWithin(2*time.Second))
	if err != nil || !acquired {
		t.Fatalf("a released layout lock is free: %v %v", acquired, err)
	}
	next.close()
}
