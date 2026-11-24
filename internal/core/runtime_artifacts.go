package core

import (
	"context"
	"crypto/sha256"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"syscall"
	"time"
)

func namespace(identity jsontext.Value) string {
	canonical, err := canonicalJSON(identity)
	if err != nil {
		panic("a database identity is valid JSON: " + err.Error())
	}
	digest := sha256.Sum256([]byte(canonical))
	const alphabet = "abcdefghijklmnopqrstuvwxyz234567"
	out := make([]byte, 0, 26)
	var bits uint32
	count := 0
	for _, b := range digest {
		bits = bits<<8 | uint32(b)
		count += 8
		for count >= 5 {
			count -= 5
			out = append(out, alphabet[(bits>>count)&31])
			if len(out) == 26 {
				return string(out)
			}
		}
	}
	panic("SHA-256 contains enough namespace bits")
}

type runtimePaths struct {
	namespaceDir      string
	namespaceManifest string
	runDir            string
	descriptor        string
	controlSocket     string
	ownerLock         string
}

func runtimePathsForRun(identity jsontext.Value, runID int64) (runtimePaths, error) {
	if err := ensure(runID > 0, "runtime run ID must be positive"); err != nil {
		return runtimePaths{}, err
	}
	value, present := jsonLookup(identity, "canonical_path")
	path, isString := jsonString(value)
	if !present || !isString {
		return runtimePaths{}, runtimeError("canonical database path required")
	}
	if err := ensure(filepath.IsAbs(path), "absolute canonical database path required"); err != nil {
		return runtimePaths{}, err
	}
	if filepath.Clean(path) == "/" {
		return runtimePaths{}, runtimeError("database parent required")
	}
	namespaceDir := filepath.Join(filepath.Dir(path), ".cafleet-runs", namespace(identity))
	runDir := filepath.Join(namespaceDir, strconv.FormatInt(runID, 10))
	controlSocket := filepath.Join(runDir, "control.sock")
	maximum := 107
	if runtime.GOOS == "darwin" {
		maximum = 103
	}
	if err := ensure(len(controlSocket) <= maximum, "complete runtime socket path exceeds platform limit"); err != nil {
		return runtimePaths{}, err
	}
	return runtimePaths{
		namespaceDir:      namespaceDir,
		namespaceManifest: filepath.Join(namespaceDir, "identity.json"),
		runDir:            runDir,
		descriptor:        filepath.Join(runDir, "launch.json"),
		controlSocket:     controlSocket,
		ownerLock:         filepath.Join(runDir, "owner.lock"),
	}, nil
}

func uid() uint32 {
	return uint32(os.Geteuid()) //nolint:gosec // an effective user ID is never negative
}

func fileStat(info os.FileInfo) *syscall.Stat_t {
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		panic("a file's metadata on this platform is a syscall.Stat_t")
	}
	return stat
}

func identity(info os.FileInfo) jsontext.Value {
	stat := fileStat(info)
	return jsonValue(jsonObject{
		{"device", uint64(stat.Dev)}, //nolint:gosec // dev_t is a device number, never negative
		{"inode", stat.Ino},
	})
}

func privateMetadata(path string, expectedUID, mode uint32, directory bool) (os.FileInfo, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	kindMatches := info.Mode().IsRegular()
	if directory {
		kindMatches = info.IsDir()
	}
	if err := ensure(info.Mode()&fs.ModeSymlink == 0 && kindMatches, "runtime artifact type mismatch"); err != nil {
		return nil, err
	}
	stat := fileStat(info)
	if err := ensure(stat.Uid == expectedUID && uint32(stat.Mode)&0o777 == mode, "runtime artifact ownership or permissions mismatch"); err != nil {
		return nil, err
	}
	return info, nil
}

func validatePrivateFile(path string, expectedUID, mode uint32) error {
	_, err := privateMetadata(path, expectedUID, mode, false)
	return err
}

func present(path string) (bool, error) {
	_, err := os.Lstat(path)
	switch {
	case err == nil:
		return true, nil
	case errors.Is(err, fs.ErrNotExist):
		return false, nil
	default:
		return false, ioError(ioErrorText(err))
	}
}

func readManifest(path string) (jsontext.Value, error) {
	file, err := openPrivate(path, false)
	if err != nil {
		return nil, err
	}
	defer file.Close() //nolint:errcheck // a read-only use; the close result carries no data
	data, err := io.ReadAll(io.LimitReader(file, 65537))
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	value := jsontext.Value(data)
	if !value.IsValid(jsontext.AllowDuplicateNames(true)) {
		return nil, ioError("invalid runtime namespace manifest JSON")
	}
	return value, nil
}

func directory(path string) error {
	if err := os.Mkdir(path, 0o700); err != nil && !errors.Is(err, fs.ErrExist) {
		return ioError(ioErrorText(err))
	}
	_, err := privateMetadata(path, uid(), 0o700, true)
	return err
}

func openPrivate(path string, create bool) (*os.File, error) {
	flags := os.O_RDWR | syscall.O_NOFOLLOW | syscall.O_CLOEXEC
	if create {
		flags |= os.O_CREATE | os.O_EXCL
	}
	file, err := os.OpenFile(path, flags, 0o600)
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	named, err := privateMetadata(path, uid(), 0o600, false)
	if err != nil {
		return nil, mergeCleanup(err, file.Close())
	}
	opened, err := file.Stat()
	if err != nil {
		return nil, mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	if err := ensure(jsonEqual(identity(opened), identity(named)), "runtime artifact changed while opening"); err != nil {
		return nil, mergeCleanup(err, file.Close())
	}
	return file, nil
}

func ensureSameDatabase(ctx context.Context, conn *dbConn, expected jsontext.Value) error {
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		return err
	}
	return ensure(jsonEqual(identity, expected), "runtime database identity changed")
}

func placementOf(_ context.Context, conn *dbConn, memberID int64) (opt[placement], error) {
	return queryRowOptional(conn, "SELECT "+placementColumns+" FROM member_placements p WHERE member_id=?1",
		[]any{memberID}, func(row sqlRow) (placement, error) { return placementFromRow(row, 0) })
}

func storedIdentityMatches(stored, actual jsontext.Value) bool {
	return stored != nil && jsonEqual(stored, actual)
}

func validateRunPaths(ctx context.Context, conn *dbConn, run runRecord) (runtimePaths, error) {
	if err := ensureSameDatabase(ctx, conn, run.databaseIdentityJSON); err != nil {
		return runtimePaths{}, err
	}
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		return runtimePaths{}, err
	}
	if err := ensure(filepath.Clean(paths.runDir) == filepath.Clean(run.runtimePath), "runtime registry path mismatch"); err != nil {
		return runtimePaths{}, err
	}
	if _, err := privateMetadata(filepath.Dir(paths.namespaceDir), uid(), 0o700, true); err != nil {
		return runtimePaths{}, err
	}
	if _, err := privateMetadata(paths.namespaceDir, uid(), 0o700, true); err != nil {
		return runtimePaths{}, err
	}
	dir, err := privateMetadata(paths.runDir, uid(), 0o700, true)
	if err != nil {
		return runtimePaths{}, err
	}
	lock, err := privateMetadata(paths.ownerLock, uid(), 0o600, false)
	if err != nil {
		return runtimePaths{}, err
	}
	unchanged := storedIdentityMatches(run.runtimeDirIdentityJSON, identity(dir)) &&
		storedIdentityMatches(run.runLockIdentityJSON, identity(lock))
	if err := ensure(unchanged, "runtime directory or owner lock identity changed; explicit recovery required"); err != nil {
		return runtimePaths{}, err
	}
	manifest, err := readManifest(paths.namespaceManifest)
	if err != nil {
		return runtimePaths{}, err
	}
	if err := ensure(jsonEqual(manifest, run.databaseIdentityJSON), "runtime namespace identity collision"); err != nil {
		return runtimePaths{}, err
	}
	return paths, nil
}

// runArtifacts is a run's allocated runtime directory. Its owner calls close, the Rust Drop, which
// removes a descriptor this value wrote.
type runArtifacts struct {
	paths runtimePaths
	// descriptorIdentity is the identity of the descriptor this value wrote; nil before it writes one.
	descriptorIdentity jsontext.Value
}

func runArtifactsCreateWithDeadline(ctx context.Context, conn *dbConn, runID int64, deadline time.Time) (*runArtifacts, error) {
	run, err := getRun(ctx, conn, runID)
	if err != nil {
		return nil, err
	}
	if err := ensureSameDatabase(ctx, conn, run.databaseIdentityJSON); err != nil {
		return nil, err
	}
	if err := ensure(run.ownerKind == ownerKindCreator && !run.status.terminal(), "artifact allocation requires the pending creator"); err != nil {
		return nil, err
	}
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, runID)
	if err != nil {
		return nil, err
	}
	if err := directory(filepath.Dir(paths.namespaceDir)); err != nil {
		return nil, err
	}
	if err := directory(paths.namespaceDir); err != nil {
		return nil, err
	}
	if err := writeManifest(paths.namespaceManifest, run.databaseIdentityJSON); err != nil {
		return nil, err
	}
	if err := os.Mkdir(paths.runDir, 0o700); err != nil {
		return nil, runtimeError("runtime directory requires exclusive allocation or explicit recovery: " + ioErrorText(err))
	}
	lock, err := openPrivate(paths.ownerLock, true)
	if err != nil {
		return nil, err
	}
	defer lock.Close() //nolint:errcheck // the lock file is reopened by its owner; this handle only names it
	dir, err := privateMetadata(paths.runDir, uid(), 0o700, true)
	if err != nil {
		return nil, err
	}
	lockInfo, err := lock.Stat()
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	dirID, lockID := identity(dir), identity(lockInfo)
	if _, err := withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (struct{}, error) {
		if err := ensureSameDatabase(ctx, conn, run.databaseIdentityJSON); err != nil {
			return struct{}{}, err
		}
		changed, err := conn.execute(
			"UPDATE member_runs SET runtime_path=?1,runtime_dir_identity_json=?2,run_lock_identity_json=?3 WHERE run_id=?4 AND owner_epoch=?5 AND owner_kind='creator' AND runtime_dir_identity_json IS NULL",
			paths.runDir, jsonText(dirID), jsonText(lockID), runID, run.ownerEpoch)
		if err != nil {
			return struct{}{}, err
		}
		return struct{}{}, ensure(changed == 1, "artifact ownership changed during allocation")
	}); err != nil {
		return nil, err
	}
	return &runArtifacts{paths: paths}, nil
}

// writeManifest writes a new namespace manifest, or checks that an existing one names the same
// database.
func writeManifest(path string, identity jsontext.Value) error {
	file, err := openPrivate(path, true)
	if err != nil {
		if _, statErr := os.Stat(path); statErr != nil {
			return err
		}
		manifest, err := readManifest(path)
		if err != nil {
			return err
		}
		return ensure(jsonEqual(manifest, identity), "runtime namespace identity collision")
	}
	canonical, err := canonicalJSON(identity)
	if err != nil {
		panic("a database identity is valid JSON: " + err.Error())
	}
	if _, err := file.WriteString(canonical); err != nil {
		return mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	if err := file.Sync(); err != nil {
		return mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	if err := file.Close(); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}

func runArtifactsFromRun(ctx context.Context, conn *dbConn, run runRecord) (*runArtifacts, error) {
	paths, err := validateRunPaths(ctx, conn, run)
	if err != nil {
		return nil, err
	}
	return &runArtifacts{paths: paths}, nil
}

func (a *runArtifacts) writeDescriptor(descriptor launchDescriptor) error {
	expected, err := runtimePathsForRun(descriptor.Identity.DatabaseIdentityJSON, descriptor.Identity.RunID)
	if err != nil {
		return err
	}
	if err := ensure(a.paths.runDir == expected.runDir, "descriptor path mismatch"); err != nil {
		return err
	}
	if err := descriptor.Spawn.validate(); err != nil {
		return err
	}
	file, err := openPrivate(a.paths.descriptor, true)
	if err != nil {
		return err
	}
	info, err := file.Stat()
	if err != nil {
		return mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	a.descriptorIdentity = identity(info)
	data := descriptor.encode()
	if err := ensure(len(data) <= descriptorLimit, "launch descriptor exceeds private size limit"); err != nil {
		return mergeCleanup(err, file.Close())
	}
	if _, err := file.Write(data); err != nil {
		return mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	if err := file.Sync(); err != nil {
		return mergeCleanup(ioError(ioErrorText(err)), file.Close())
	}
	if err := file.Close(); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}

func (a *runArtifacts) removeDescriptor() error {
	info, err := os.Lstat(a.paths.descriptor)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return ioError(ioErrorText(err))
	}
	if err := validatePrivateFile(a.paths.descriptor, uid(), 0o600); err != nil {
		return err
	}
	if a.descriptorIdentity != nil {
		if err := ensure(jsonEqual(identity(info), a.descriptorIdentity), "descriptor identity changed before unlink"); err != nil {
			return err
		}
	}
	if err := os.Remove(a.paths.descriptor); err != nil {
		return runtimeError("descriptor unlink failed: " + ioErrorText(err))
	}
	return nil
}

func (a *runArtifacts) close() {
	if a.descriptorIdentity == nil {
		return
	}
	if err := a.removeDescriptor(); err != nil {
		fmt.Fprintln(os.Stderr, cafleetErrorOf(err).message())
	}
}

type launchDescriptor struct {
	Identity controlIdentity
	Spawn    spawnSpec
}

// launchDescriptorWire is the serde encoding of a launchDescriptor, which denies unknown members.
type launchDescriptorWire struct {
	Identity controlIdentity `json:"identity"`
	Spawn    spawnSpecWire   `json:"spawn"`
}

func newLaunchDescriptor(run runRecord, spawn spawnSpec) (launchDescriptor, error) {
	if err := spawn.validate(); err != nil {
		return launchDescriptor{}, err
	}
	return launchDescriptor{Identity: controlIdentityFromRun(run), Spawn: spawn}, nil
}

func parseLaunchDescriptor(data []byte) (launchDescriptor, error) {
	var wire launchDescriptorWire
	if err := decodeStrict(data, &wire); err != nil {
		return launchDescriptor{}, runtimeError("malformed launch descriptor")
	}
	return launchDescriptor{Identity: wire.Identity, Spawn: wire.Spawn.spawnSpec()}, nil
}

func (d launchDescriptor) encode() []byte {
	return jsonValue(launchDescriptorWire{Identity: d.Identity, Spawn: d.Spawn.wire()})
}

const descriptorLimit = 1024 * 1024

func loadDescriptor(ctx context.Context, conn *dbConn, path string) (launchDescriptor, error) {
	if err := ensure(filepath.IsAbs(path), "absolute runner descriptor path required"); err != nil {
		return launchDescriptor{}, err
	}
	file, err := openPrivate(path, false)
	if err != nil {
		return launchDescriptor{}, err
	}
	defer file.Close() //nolint:errcheck // a read-only use; the close result carries no data
	info, err := file.Stat()
	if err != nil {
		return launchDescriptor{}, ioError(ioErrorText(err))
	}
	if err := ensure(info.Size() <= descriptorLimit, "launch descriptor exceeds private size limit"); err != nil {
		return launchDescriptor{}, err
	}
	data, err := io.ReadAll(file)
	if err != nil {
		return launchDescriptor{}, ioError(ioErrorText(err))
	}
	descriptor, err := parseLaunchDescriptor(data)
	if err != nil {
		return launchDescriptor{}, err
	}
	if err := descriptor.Spawn.validate(); err != nil {
		return launchDescriptor{}, err
	}
	run, err := getRun(ctx, conn, descriptor.Identity.RunID)
	if err != nil {
		return launchDescriptor{}, err
	}
	paths, err := validateRunPaths(ctx, conn, run)
	if err != nil {
		return launchDescriptor{}, err
	}
	if err := ensure(filepath.Clean(path) == filepath.Clean(paths.descriptor), "runner descriptor was not allocated for this run"); err != nil {
		return launchDescriptor{}, err
	}
	matches := run.ownerKind == ownerKindCreator && descriptor.Identity.equal(controlIdentityFromRun(run))
	if err := ensure(matches, "launch descriptor identity mismatch"); err != nil {
		return launchDescriptor{}, err
	}
	return descriptor, nil
}

// runLock is an open, flock-held owner lock; close releases it.
type runLock struct {
	file *os.File
}

func (l *runLock) transferToReconcilerChecked(ctx context.Context, conn *dbConn, expected runRecord, process processIdentity, deadline time.Time, check func(context.Context, *dbConn) error, c clock) (runRecord, error) {
	return withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (runRecord, error) {
		return inTransaction(conn, transactionImmediate, func() (runRecord, error) {
			if err := check(ctx, conn); err != nil {
				return runRecord{}, err
			}
			current, err := getRun(ctx, conn, expected.runID)
			if err != nil {
				return runRecord{}, err
			}
			if err := ensureSameDatabase(ctx, conn, expected.databaseIdentityJSON); err != nil {
				return runRecord{}, err
			}
			if err := ensure(jsonEqual(stableRunIdentity(current), stableRunIdentity(expected)), "recovery ownership changed before CAS"); err != nil {
				return runRecord{}, err
			}
			next := current
			if !current.isSettled() {
				if err := l.validate(ctx, conn, expected.owner()); err != nil {
					return runRecord{}, err
				}
				if next, err = advanceRecoveryOwner(ctx, conn, expected, process); err != nil {
					return runRecord{}, err
				}
			}
			if err := checkDeadline(c.now(), deadline); err != nil {
				return runRecord{}, err
			}
			return next, nil
		})
	})
}

func runLockAcquire(ctx context.Context, conn *dbConn, owner runOwner) (opt[*runLock], error) {
	run, err := getRun(ctx, conn, owner.runID)
	if err != nil {
		return none[*runLock](), err
	}
	if err := ensure(run.owner().equal(owner), "stale runtime owner"); err != nil {
		return none[*runLock](), err
	}
	paths, err := validateRunPaths(ctx, conn, run)
	if err != nil {
		return none[*runLock](), err
	}
	file, err := openPrivate(paths.ownerLock, false)
	if err != nil {
		return none[*runLock](), err
	}
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil { //nolint:gosec // a file descriptor fits in an int
		closeErr := file.Close()
		if errors.Is(err, syscall.EWOULDBLOCK) {
			if closeErr != nil {
				return none[*runLock](), ioError(ioErrorText(closeErr))
			}
			return none[*runLock](), nil
		}
		return none[*runLock](), mergeCleanup(ioError(ioErrorText(err)), closeErr)
	}
	lock := &runLock{file: file}
	if err := lock.validate(ctx, conn, owner); err != nil {
		return none[*runLock](), mergeCleanup(err, lock.close())
	}
	return some(lock), nil
}

func (l *runLock) validate(ctx context.Context, conn *dbConn, owner runOwner) error {
	run, err := getRun(ctx, conn, owner.runID)
	if err != nil {
		return err
	}
	if err := ensure(run.owner().equal(owner), "stale runtime owner"); err != nil {
		return err
	}
	if _, err := validateRunPaths(ctx, conn, run); err != nil {
		return err
	}
	info, err := l.file.Stat()
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return ensure(storedIdentityMatches(run.runLockIdentityJSON, identity(info)), "open owner lock identity mismatch")
}

func (l *runLock) transferToRunnerWithDeadline(ctx context.Context, conn *dbConn, owner runOwner, process processIdentity, deadline time.Time, c clock) (runOwner, error) {
	if err := ensure(owner.ownerKind == ownerKindCreator, "runner transfer requires creator ownership"); err != nil {
		return runOwner{}, err
	}
	return withDatabaseDeadline(ctx, conn, deadline, func(ctx context.Context, conn *dbConn) (runOwner, error) {
		return inTransaction(conn, transactionImmediate, func() (runOwner, error) {
			if err := l.validate(ctx, conn, owner); err != nil {
				return runOwner{}, err
			}
			if owner.ownerEpoch == math.MaxInt64 {
				return runOwner{}, runtimeError("owner epoch exhausted")
			}
			changed, err := conn.execute(
				"UPDATE member_runs SET owner_epoch=?1,owner_kind='runner',owner_pid=?2,owner_start_identity=?3,runner_pid=?2,runner_start_identity=?3,last_health_at=?4 WHERE run_id=?5 AND owner_epoch=?6 AND owner_kind='creator'",
				owner.ownerEpoch+1, process.pid, process.startIdentity, wallStamp(c), owner.runID, owner.ownerEpoch)
			if err != nil {
				return runOwner{}, err
			}
			if err := ensure(changed == 1, "creator ownership transfer lost CAS"); err != nil {
				return runOwner{}, err
			}
			next, err := getRun(ctx, conn, owner.runID)
			if err != nil {
				return runOwner{}, err
			}
			return next.owner(), nil
		})
	})
}

func (l *runLock) close() error {
	if err := l.file.Close(); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}

type pinnedFile struct {
	file     *os.File
	identity jsontext.Value
	digest   [32]byte
}

// recoveryDescriptor is a run's launch descriptor pinned for removal; a nil pinned file means
// the descriptor was absent when pinned.
type recoveryDescriptor struct {
	path   string
	pinned *pinnedFile
}

func validatedPaths(ctx context.Context, conn *dbConn, run runRecord, lock *runLock, until time.Time, c clock) (runtimePaths, error) {
	paths, err := withDatabaseDeadline(ctx, conn, until, func(ctx context.Context, conn *dbConn) (runtimePaths, error) {
		current, err := getRun(ctx, conn, run.runID)
		if err != nil {
			return runtimePaths{}, err
		}
		if err := ensure(jsonEqual(stableRunIdentity(current), stableRunIdentity(run)), "descriptor runtime ownership changed"); err != nil {
			return runtimePaths{}, err
		}
		if err := lock.validate(ctx, conn, run.owner()); err != nil {
			return runtimePaths{}, err
		}
		return validateRunPaths(ctx, conn, run)
	})
	if err != nil {
		return runtimePaths{}, err
	}
	process, err := processIdentityCurrent()
	if err != nil {
		return runtimePaths{}, err
	}
	if err := ensure(run.ownerPID == process.pid && run.ownerStartIdentity == process.startIdentity, "descriptor access requires the current runtime owner"); err != nil {
		return runtimePaths{}, err
	}
	if err := checkDeadline(c.now(), until); err != nil {
		return runtimePaths{}, err
	}
	return paths, nil
}

func readBounded(file *os.File, until time.Time, c clock) ([]byte, error) {
	if err := checkDeadline(c.now(), until); err != nil {
		return nil, err
	}
	info, err := file.Stat()
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	if err := ensure(info.Size() <= descriptorLimit, "launch descriptor exceeds private size limit"); err != nil {
		return nil, err
	}
	if _, err := file.Seek(0, io.SeekStart); err != nil {
		return nil, ioError(ioErrorText(err))
	}
	data, err := io.ReadAll(io.LimitReader(file, descriptorLimit+1))
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	if err := ensure(len(data) <= descriptorLimit, "launch descriptor exceeds private size limit"); err != nil {
		return nil, err
	}
	if err := checkDeadline(c.now(), until); err != nil {
		return nil, err
	}
	return data, nil
}

func validateNamed(path string, pinned *pinnedFile) error {
	named, err := privateMetadata(path, uid(), 0o600, false)
	if err != nil {
		return err
	}
	opened, err := pinned.file.Stat()
	if err != nil {
		return ioError(ioErrorText(err))
	}
	return ensure(jsonEqual(identity(named), pinned.identity) && jsonEqual(identity(opened), pinned.identity), "descriptor identity changed before unlink")
}

func recoveryDescriptorPin(ctx context.Context, conn *dbConn, run runRecord, lock *runLock, until time.Time, c clock) (*recoveryDescriptor, error) {
	paths, err := validatedPaths(ctx, conn, run, lock, until, c)
	if err != nil {
		return nil, err
	}
	path := paths.descriptor
	exists, err := present(path)
	if err != nil {
		return nil, err
	}
	if !exists {
		return &recoveryDescriptor{path: path}, nil
	}
	file, err := openPrivate(path, false)
	if err != nil {
		return nil, err
	}
	pinned, err := pinDescriptorFile(file, run, until, c)
	if err != nil {
		return nil, mergeCleanup(err, file.Close())
	}
	if err := validateNamed(path, pinned); err != nil {
		return nil, mergeCleanup(err, file.Close())
	}
	if err := checkDeadline(c.now(), until); err != nil {
		return nil, mergeCleanup(err, file.Close())
	}
	return &recoveryDescriptor{path: path, pinned: pinned}, nil
}

func pinDescriptorFile(file *os.File, run runRecord, until time.Time, c clock) (*pinnedFile, error) {
	info, err := file.Stat()
	if err != nil {
		return nil, ioError(ioErrorText(err))
	}
	data, err := readBounded(file, until, c)
	if err != nil {
		return nil, err
	}
	descriptor, err := parseLaunchDescriptor(data)
	if err != nil {
		return nil, err
	}
	recorded := descriptor.Identity
	current := recorded
	current.OwnerEpoch = run.ownerEpoch
	matches := recorded.OwnerEpoch > 0 && recorded.OwnerEpoch <= run.ownerEpoch &&
		current.equal(controlIdentityFromRun(run))
	if err := ensure(matches, "launch descriptor identity mismatch"); err != nil {
		return nil, err
	}
	return &pinnedFile{file: file, identity: identity(info), digest: sha256.Sum256(data)}, nil
}

func (r *recoveryDescriptor) remove(ctx context.Context, conn *dbConn, run runRecord, lock *runLock, until time.Time, c clock, unlink func(path string, deadline time.Time) error) error {
	paths, err := validatedPaths(ctx, conn, run, lock, until, c)
	if err != nil {
		return err
	}
	if err := ensure(paths.descriptor == r.path, "descriptor pathname changed"); err != nil {
		return err
	}
	exists, err := present(r.path)
	if err != nil {
		return err
	}
	if !exists {
		return r.unpin()
	}
	pinned := r.pinned
	if pinned == nil {
		return runtimeError("descriptor appeared after absence was pinned")
	}
	if err := validateNamed(r.path, pinned); err != nil {
		return err
	}
	data, err := readBounded(pinned.file, until, c)
	if err != nil {
		return err
	}
	if err := ensure(sha256.Sum256(data) == pinned.digest, "descriptor content changed before unlink"); err != nil {
		return err
	}
	if err := validateNamed(r.path, pinned); err != nil {
		return err
	}
	if err := checkDeadline(c.now(), until); err != nil {
		return err
	}
	if err := unlink(r.path, until); err != nil {
		return err
	}
	remains, err := present(r.path)
	if err != nil {
		return err
	}
	if err := ensure(!remains, "descriptor remains after unlink"); err != nil {
		return err
	}
	if err := checkDeadline(c.now(), until); err != nil {
		return err
	}
	return r.unpin()
}

// unpin drops the pinned file, closing its handle as the Rust assignment of None drops it.
func (r *recoveryDescriptor) unpin() error {
	pinned := r.pinned
	r.pinned = nil
	if pinned == nil {
		return nil
	}
	if err := pinned.file.Close(); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}
