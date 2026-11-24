package core

import (
	"cmp"
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

func validInput(text string) bool {
	return strings.TrimSpace(text) != "" && !strings.ContainsFunc(text, func(r rune) bool {
		return r <= 0x1f || r == 0x7f
	})
}

func memberPaneInputError(memberID int64) func(error) error {
	return func(err error) error {
		return appErrorf("member %d pane input: %s", memberID, cafleetErrorOf(err).message())
	}
}

const layoutLockWait = 60 * time.Second

const paneInput = "pane input"

// databaseLockPath is `<canonical database file><suffix>/<sha256 hex of key>.lock`.
func databaseLockPath(databaseURL, suffix, purpose string, key []byte) (string, error) {
	path, err := databasePath(databaseURL)
	if err != nil {
		return "", err
	}
	database, err := canonicalPath(path)
	if err != nil {
		return "", appErrorf("cannot resolve database %s-lock identity: %s", purpose, ioErrorText(err))
	}
	return joinPath(database+suffix, sha256Hex(key)+".lock"), nil
}

// canonicalPath resolves path to an absolute path with every symbolic link resolved, failing
// when it does not exist.
func canonicalPath(path string) (string, error) {
	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		return "", err
	}
	return filepath.Abs(resolved)
}

// openAndTryLock opens (creating it 0600, and its directory, on demand) and tries the lock at
// path; false means another holder has it.
func openAndTryLock(path, label string) (*os.File, bool, error) {
	directory := filepath.Dir(path)
	if err := os.MkdirAll(directory, 0o777); err != nil {
		return nil, false, appErrorf("cannot create %s-lock directory %s: %s", label, directory, ioErrorText(err))
	}
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o600)
	if err != nil {
		return nil, false, appErrorf("cannot open %s lock %s: %s", label, path, ioErrorText(err))
	}
	locked, err := tryLock(file, path, label)
	if err != nil || !locked {
		closeFile(file)
		return nil, false, err
	}
	return file, true, nil
}

// closeFile closes a file opened for reading or locking; closing releases its lock, and a close
// error leaves nothing for the caller to act on.
func closeFile(file *os.File) {
	_ = file.Close()
}

// tryLock takes an exclusive flock without waiting; false means another holder has it.
func tryLock(file *os.File, path, label string) (bool, error) {
	err := unix.Flock(int(file.Fd()), unix.LOCK_EX|unix.LOCK_NB)
	switch {
	case err == nil:
		return true, nil
	case errors.Is(err, syscall.EWOULDBLOCK):
		return false, nil
	default:
		return false, appErrorf("cannot lock %s %s: %s", label, path, ioErrorText(err))
	}
}

// retryUntil repeats attempt every 50 ms until it yields a value, making one last attempt after
// the sleep that reaches deadline; false means every attempt came up empty.
func retryUntil[T any](d deadline, attempt func() (T, bool, error)) (T, bool, error) {
	for {
		value, found, err := attempt()
		if err != nil || found {
			return value, found, err
		}
		left, err := d.remaining()
		if err != nil {
			var zero T
			return zero, false, nil
		}
		time.Sleep(min(left, 50*time.Millisecond))
	}
}

func lockPath(databaseURL string, p placement) (string, error) {
	pane, concrete := p.muxPaneID.get()
	if !concrete {
		return "", appError("pane input requires a concrete placement")
	}
	key := p.backend + "\x00" + p.muxSession + "\x00" + pane
	return databaseLockPath(databaseURL, ".pane-locks", "input", []byte(key))
}

func layoutLockPath(databaseURL string, fleetID int64) (string, error) {
	return databaseLockPath(databaseURL, ".layout-locks", "layout", []byte(strconv.FormatInt(fleetID, 10)))
}

// layoutLock is the fleet-scoped lock serializing every member pane creation and close; close
// releases it.
type layoutLock struct {
	file *os.File
}

func layoutLockTryAcquire(databaseURL string, fleetID int64) (*layoutLock, bool, error) {
	path, err := layoutLockPath(databaseURL, fleetID)
	if err != nil {
		return nil, false, err
	}
	file, locked, err := openAndTryLock(path, "pane layout")
	if err != nil || !locked {
		return nil, false, err
	}
	return &layoutLock{file: file}, true, nil
}

// layoutLockAcquireWithin retries every 50 ms until lockDeadline; false means the lock stayed
// busy.
func layoutLockAcquireWithin(databaseURL string, fleetID int64, lockDeadline deadline) (*layoutLock, bool, error) {
	return retryUntil(lockDeadline, func() (*layoutLock, bool, error) {
		return layoutLockTryAcquire(databaseURL, fleetID)
	})
}

func (l *layoutLock) close() {
	closeFile(l.file)
}

type paneLock struct {
	file *os.File
	path string
}

func (l *paneLock) validateBinding(databaseURL string, p placement) error {
	path, err := lockPath(databaseURL, p)
	if err != nil {
		return err
	}
	if l.path != path {
		return appError("held pane lock binding mismatch")
	}
	var open, named unix.Stat_t
	if err := unix.Fstat(int(l.file.Fd()), &open); err != nil {
		return appError(ioErrorText(err))
	}
	if err := unix.Lstat(path, &named); err != nil {
		return appError(ioErrorText(err))
	}
	if named.Mode&unix.S_IFMT != unix.S_IFREG || open.Dev != named.Dev || open.Ino != named.Ino {
		return appError("held pane lock pathname identity mismatch")
	}
	return nil
}

func paneLockTryAcquire(databaseURL string, p placement) (*paneLock, bool, error) {
	path, err := lockPath(databaseURL, p)
	if err != nil {
		return nil, false, err
	}
	file, locked, err := openAndTryLock(path, paneInput)
	if err != nil || !locked {
		return nil, false, err
	}
	return &paneLock{file: file, path: path}, true, nil
}

// release unlocks this held lock so the member's runner can write its staged input.
func (l *paneLock) release() error {
	if err := unix.Flock(int(l.file.Fd()), unix.LOCK_UN); err != nil {
		return appErrorf("cannot release pane input %s: %s", l.path, ioErrorText(err))
	}
	return nil
}

// reacquireWithin retakes a released lock, retrying every 50 ms until lockDeadline; false means
// it stayed busy.
func (l *paneLock) reacquireWithin(lockDeadline deadline) (bool, error) {
	_, reacquired, err := retryUntil(lockDeadline, func() (struct{}, bool, error) {
		locked, err := tryLock(l.file, l.path, paneInput)
		return struct{}{}, locked, err
	})
	return reacquired, err
}

// paneLockAcquireWithin retries a held lock every 50 ms until lockDeadline; false means it
// stayed busy.
func paneLockAcquireWithin(databaseURL string, p placement, lockDeadline deadline) (*paneLock, bool, error) {
	return retryUntil(lockDeadline, func() (*paneLock, bool, error) {
		return paneLockTryAcquire(databaseURL, p)
	})
}

func (l *paneLock) close() {
	closeFile(l.file)
}

type lockedMember struct {
	member memberRecord
	guard  *paneLock
}

func (m *lockedMember) paneLock(databaseURL string) (*paneLock, error) {
	if err := m.guard.validateBinding(databaseURL, m.member.placement); err != nil {
		return nil, err
	}
	return m.guard, nil
}

func (m *lockedMember) close() {
	m.guard.close()
}

// memberPlacement is one deletion target: a member and the placement whose pane lock it needs.
type memberPlacement struct {
	memberID  int64
	placement placement
}

// lockedDeletion holds the pane locks of a deletion's members and the termination-deferral
// span opened after the last of them was taken.
type lockedDeletion[S, T any] struct {
	snapshot S
	value    T
	span     *deferTermination
	guards   []*paneLock
}

func paneLockFor(guards []*paneLock, databaseURL string, p placement) (*paneLock, error) {
	path, err := lockPath(databaseURL, p)
	if err != nil {
		return nil, err
	}
	index := slices.IndexFunc(guards, func(guard *paneLock) bool { return guard.path == path })
	if index < 0 {
		return nil, appError("matching held pane lock required")
	}
	if err := guards[index].validateBinding(databaseURL, p); err != nil {
		return nil, err
	}
	return guards[index], nil
}

func (d *lockedDeletion[S, T]) paneLock(databaseURL string, p placement) (*paneLock, error) {
	return paneLockFor(d.guards, databaseURL, p)
}

func (d *lockedDeletion[S, T]) snapshotAndPaneLock(databaseURL string, p placement) (*S, *paneLock, error) {
	guard, err := paneLockFor(d.guards, databaseURL, p)
	if err != nil {
		return nil, nil, err
	}
	return &d.snapshot, guard, nil
}

// close ends the span, re-raising a deferred termination signal, then releases the locks.
func (d *lockedDeletion[S, T]) close() {
	d.span.close()
	closePaneLocks(d.guards)
}

func closePaneLocks(guards []*paneLock) {
	for _, guard := range guards {
		guard.close()
	}
}

type lockTarget struct {
	path   string
	target memberPlacement
}

// lockDeletion loads the deletion's targets, takes every target's pane lock in path order,
// opens a termination-deferral span and revalidates the snapshot under the locks, starting over
// while revalidation reports a change.
func lockDeletion[S, T any](
	ctx context.Context,
	conn *dbConn,
	s settings,
	d deadline,
	load func(context.Context, *dbConn) (S, []memberPlacement, error),
	revalidate func(context.Context, *dbConn, *S, []*paneLock) (T, bool, error),
	busy func(int64) error,
) (*lockedDeletion[S, T], error) {
	type loaded struct {
		snapshot S
		targets  []memberPlacement
	}
	for {
		state, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (loaded, error) {
			snapshot, targets, err := load(ctx, conn)
			return loaded{snapshot, targets}, err
		})
		if err != nil {
			return nil, err
		}
		guards, err := lockTargets(s, d, state.targets, busy)
		if err != nil {
			return nil, err
		}
		span, err := deferTerminationBegin()
		if err != nil {
			closePaneLocks(guards)
			return nil, err
		}
		snapshot := state.snapshot
		type revalidated struct {
			value T
			valid bool
		}
		result, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (revalidated, error) {
			value, valid, err := revalidate(ctx, conn, &snapshot, guards)
			return revalidated{value, valid}, err
		})
		if err != nil {
			span.close()
			closePaneLocks(guards)
			return nil, err
		}
		if result.valid {
			return &lockedDeletion[S, T]{snapshot: snapshot, value: result.value, span: span, guards: guards}, nil
		}
		span.close()
		closePaneLocks(guards)
		if _, err := d.remaining(); err != nil {
			return nil, err
		}
	}
}

func lockTargets(s settings, d deadline, targets []memberPlacement, busy func(int64) error) ([]*paneLock, error) {
	ordered := make([]lockTarget, 0, len(targets))
	for _, target := range targets {
		path, err := lockPath(s.databaseURL, target.placement)
		if err != nil {
			return nil, memberPaneInputError(target.memberID)(err)
		}
		ordered = append(ordered, lockTarget{path: path, target: target})
	}
	slices.SortFunc(ordered, func(left, right lockTarget) int {
		if order := strings.Compare(left.path, right.path); order != 0 {
			return order
		}
		return cmp.Compare(left.target.memberID, right.target.memberID)
	})
	ordered = slices.CompactFunc(ordered, func(left, right lockTarget) bool { return left.path == right.path })
	var guards []*paneLock
	for _, entry := range ordered {
		if _, err := d.remaining(); err != nil {
			closePaneLocks(guards)
			return nil, err
		}
		guard, acquired, err := retryUntil(d, func() (*paneLock, bool, error) {
			guard, acquired, err := paneLockTryAcquire(s.databaseURL, entry.target.placement)
			if err != nil {
				return nil, false, memberPaneInputError(entry.target.memberID)(err)
			}
			return guard, acquired, nil
		})
		if err == nil && !acquired {
			err = busy(entry.target.memberID)
		}
		if err != nil {
			closePaneLocks(guards)
			return nil, err
		}
		guards = append(guards, guard)
	}
	return guards, nil
}

const pressLockReserve = time.Second

// lockMemberWithReservation takes the member's pane lock, leaving pressLockReserve of the
// deadline, and rereads the member under it; false means the lock stayed busy.
func lockMemberWithReservation(ctx context.Context, conn *dbConn, s settings, member memberRecord, d deadline) (*lockedMember, bool, error) {
	guard, acquired, err := paneLockAcquireWithin(s.databaseURL, member.placement, d.reserving(pressLockReserve))
	if err != nil {
		return nil, false, memberPaneInputError(member.memberID)(err)
	}
	if !acquired {
		return nil, false, nil
	}
	current, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (opt[memberRecord], error) {
		return getMember(ctx, conn, member.memberID, member.fleetID)
	})
	if err != nil {
		guard.close()
		return nil, false, err
	}
	record, found := current.get()
	if !found {
		guard.close()
		return nil, false, appErrorf("Member %d not found", member.memberID)
	}
	if record.placement != member.placement {
		guard.close()
		return nil, false, appErrorf("member %d placement changed while acquiring pane input", member.memberID)
	}
	return &lockedMember{member: record, guard: guard}, true, nil
}
