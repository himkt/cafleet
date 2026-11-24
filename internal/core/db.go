package core

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/ncruces/go-sqlite3"
)

const schemaSQL = `PRAGMA application_id = 1128351289;

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL
);
CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT,
    last_window_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(last_window_ordinal >= 0)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT,
    mux_window_name TEXT,
    window_ordinal INTEGER NOT NULL CHECK(window_ordinal >= 0),
    column_side TEXT NOT NULL CHECK(column_side IN ('left','right')),
    mux_pane_id TEXT,
    backend TEXT NOT NULL,
    mux_server TEXT NOT NULL,
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK((mux_pane_id IS NULL) = (mux_window_id IS NULL)
          AND (mux_window_id IS NULL) = (mux_window_name IS NULL))
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    from_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    to_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    text TEXT NOT NULL,
    notified_at TEXT
);
CREATE INDEX idx_messages_to_member_status_ts ON messages (to_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_created_at ON messages (from_member_id, created_at);

CREATE TABLE runtime_database_identity (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    instance_id TEXT NOT NULL CHECK(length(instance_id) = 32 AND instance_id NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO runtime_database_identity VALUES(1, lower(hex(randomblob(16))));

CREATE TABLE cafleet_database (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    cafleet_version TEXT NOT NULL CHECK(length(trim(cafleet_version)) > 0)
);

CREATE TABLE member_runs (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT,
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode','hermes','pi')),
    database_identity_json TEXT NOT NULL CHECK(json_valid(database_identity_json) AND json_type(database_identity_json) = 'object'),
    launch_nonce TEXT NOT NULL CHECK(length(launch_nonce) = 64 AND launch_nonce NOT GLOB '*[^0-9a-f]*'),
    runtime_path TEXT NOT NULL CHECK(length(trim(runtime_path)) > 0),
    requested_mode TEXT,
    resolved_mode TEXT NOT NULL CHECK(length(trim(resolved_mode)) > 0),
    mode_source TEXT NOT NULL CHECK(mode_source IN ('default','explicit')),
    permission_argv_json TEXT NOT NULL CHECK(json_valid(permission_argv_json) AND json_type(permission_argv_json) = 'array'),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_version TEXT,
    owner_epoch INTEGER NOT NULL CHECK(owner_epoch > 0),
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('creator','runner','reconciler')),
    owner_pid INTEGER NOT NULL CHECK(owner_pid > 0),
    owner_start_identity TEXT NOT NULL CHECK(length(trim(owner_start_identity)) > 0),
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('complete','incomplete')),
    runtime_dir_identity_json TEXT CHECK(runtime_dir_identity_json IS NULL OR (json_valid(runtime_dir_identity_json) AND json_type(runtime_dir_identity_json) = 'object')),
    run_lock_identity_json TEXT CHECK(run_lock_identity_json IS NULL OR (json_valid(run_lock_identity_json) AND json_type(run_lock_identity_json) = 'object')),
    runner_pid INTEGER CHECK(runner_pid IS NULL OR runner_pid > 0),
    runner_start_identity TEXT,
    child_pid INTEGER CHECK(child_pid IS NULL OR child_pid > 0),
    child_start_identity TEXT,
    started_at TEXT,
    last_health_at TEXT,
    ended_at TEXT,
    exit_code INTEGER,
    cleanup_diagnostics_json TEXT CHECK(cleanup_diagnostics_json IS NULL OR (json_valid(cleanup_diagnostics_json) AND json_type(cleanup_diagnostics_json) = 'array')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object')),
    CHECK((mode_source = 'default' AND requested_mode IS NULL) OR (mode_source = 'explicit' AND requested_mode IS NOT NULL))
);
CREATE UNIQUE INDEX idx_member_runs_active ON member_runs(member_id) WHERE status NOT IN ('exited','failed','lost');

CREATE TABLE member_interactions (
    interaction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    episode_seq INTEGER NOT NULL CHECK(episode_seq > 0),
    classification TEXT NOT NULL CHECK(classification IN ('permission_request','question','unknown_interaction')),
    evidence_digest TEXT NOT NULL,
    opened_at TEXT NOT NULL,
    closed_at TEXT,
    close_reason TEXT CHECK(close_reason IS NULL OR close_reason IN ('not_visible','superseded','process_exited','monitor_lost')),
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json) AND json_type(evidence_json) = 'object'),
    UNIQUE(run_id, episode_seq),
    CHECK((closed_at IS NULL) = (close_reason IS NULL))
);
CREATE UNIQUE INDEX idx_member_interactions_open ON member_interactions(run_id) WHERE closed_at IS NULL;

CREATE TABLE member_runtime_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    interaction_id INTEGER REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_closed','input_delivery_result','input_delivery_stalled','runner_degraded','runner_lost','runner_exited')),
    observed_at TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
    notification_state TEXT NOT NULL CHECK(notification_state IN ('inbox','archived')),
    message_id INTEGER UNIQUE REFERENCES messages(message_id) ON DELETE RESTRICT,
    archive_reason TEXT CHECK(archive_reason IS NULL OR archive_reason IN ('member_inactive','fleet_deleted','director_inactive','head_not_ready','duplicate_delivery')),
    UNIQUE(run_id, event_seq),
    CHECK((notification_state = 'inbox' AND message_id IS NOT NULL AND archive_reason IS NULL) OR
          (notification_state = 'archived' AND message_id IS NULL AND archive_reason IS NOT NULL))
);

CREATE TABLE member_input_operations (
    input_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    operation_key TEXT NOT NULL CHECK(length(trim(operation_key)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('prompt','exec')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    delivery_method TEXT CHECK(delivery_method IS NULL OR delivery_method IN ('profile','fallback')),
    delivery_reason TEXT CHECK(delivery_reason IS NULL OR length(trim(delivery_reason)) > 0),
    deferred_at TEXT,
    defer_reason TEXT CHECK((deferred_at IS NULL AND defer_reason IS NULL)
        OR (deferred_at IS NOT NULL AND defer_reason IS NOT NULL
            AND kind IN ('prompt','exec'))),
    stop_cause TEXT CHECK(stop_cause IS NULL
        OR (stop_cause IN ('interaction_open','interaction_candidate','literal_mismatch',
                          'submit_unconfirmed','deadline_expired','short_write','write_failed',
                          'runner_stopped')
            AND state IN ('uncertain','settled'))),
    UNIQUE(run_id, operation_key),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);

CREATE TABLE member_runner_diagnostics (
    diagnostic_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    source TEXT NOT NULL CHECK(source IN ('event_writer','observation','deferred_input','queue_housekeeping','preview_sweep','director_delivery','input_result','permission_answer','shutdown')),
    message TEXT NOT NULL CHECK(length(trim(message)) > 0 AND length(CAST(message AS BLOB)) <= 512),
    first_at TEXT NOT NULL,
    last_at TEXT NOT NULL,
    count INTEGER NOT NULL CHECK(count > 0),
    UNIQUE(run_id, source, message)
);
CREATE INDEX idx_member_runner_diagnostics_recent ON member_runner_diagnostics(run_id, last_at);
`

const applicationID int64 = 1128351289

// dbConn is one SQLite connection; the database-wait scopes over it travel in the context of each
// call (databaseScope).
type dbConn struct {
	raw *sqlite3.Conn
}

// databasePath is the file path a sqlite:///<path> database URL names.
func databasePath(databaseURL string) (string, error) {
	path, ok := strings.CutPrefix(databaseURL, "sqlite:///")
	if !ok {
		return "", appError("database URL must use the sqlite scheme (sqlite:///<path>); got '" + databaseURL + "'")
	}
	return path, nil
}

func connect(databaseURL string) (*dbConn, error) {
	return connectWithFlags(databaseURL, sqlite3.OPEN_READWRITE|sqlite3.OPEN_CREATE)
}

func connectReadOnly(databaseURL string) (*dbConn, error) {
	return connectWithFlags(databaseURL, sqlite3.OPEN_READONLY)
}

// openExisting opens the database file the URL names when it exists, never creating it.
func openExisting(databaseURL string, readOnly bool) (opt[*dbConn], error) {
	path, err := databasePath(databaseURL)
	if err != nil {
		return none[*dbConn](), err
	}
	exists, err := pathExists(path)
	if err != nil {
		return none[*dbConn](), appError("cannot inspect database at '" + path + "': " + ioErrorText(err))
	}
	if !exists {
		return none[*dbConn](), nil
	}
	flags := sqlite3.OPEN_READWRITE
	if readOnly {
		flags = sqlite3.OPEN_READONLY
	}
	conn, err := connectWithFlags(databaseURL, flags)
	if err != nil {
		return none[*dbConn](), err
	}
	return some(conn), nil
}

func connectWithFlags(databaseURL string, flags sqlite3.OpenFlag) (*dbConn, error) {
	path, err := databasePath(databaseURL)
	if err != nil {
		return nil, err
	}
	raw, err := sqlite3.OpenFlags(path, flags)
	if err != nil {
		return nil, appError("failed to open database at '" + path + "': " + sqliteMessage(err))
	}
	if err := raw.Exec("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;"); err != nil {
		closeErr := raw.Close()
		return nil, mergeCleanup(
			appError("failed to apply connection PRAGMAs: "+sqliteMessage(err)),
			sqliteCloseError(closeErr),
		)
	}
	return &dbConn{raw: raw}, nil
}

func sqliteCloseError(err error) error {
	if err == nil {
		return nil
	}
	return sqliteError(err)
}

func (c *dbConn) close() error {
	return sqliteCloseError(c.raw.Close())
}

// sqliteMessage is SQLite's own message text for err, without the driver's `sqlite3: ` prefix
// (F18, SPEC §6.4 Cause texts). The driver renders `sqlite3: <code text>[: <message>][: <system
// error>]` and omits the message when it equals the code text.
func sqliteMessage(err error) string {
	var failure *sqlite3.Error
	if errors.As(err, &failure) {
		codeText := strings.TrimPrefix(failure.Code().Error(), "sqlite3: ")
		rest := strings.TrimPrefix(strings.TrimPrefix(failure.Error(), "sqlite3: "), codeText)
		if system := failure.Unwrap(); system != nil {
			rest = strings.TrimSuffix(rest, ": "+system.Error())
		}
		if message := strings.TrimPrefix(rest, ": "); message != "" {
			return message
		}
		return codeText
	}
	var code sqlite3.ExtendedErrorCode
	if errors.As(err, &code) {
		return strings.TrimPrefix(code.Error(), "sqlite3: ")
	}
	panic("an SQLite driver error carries an SQLite result code: " + err.Error())
}

// sqliteError classifies a driver error by its SQLite result code (src/error.rs From).
func sqliteError(err error) *cafleetError {
	var failure *sqlite3.Error
	if errors.As(err, &failure) {
		return sqliteFailure(int(failure.ExtendedCode()), sqliteMessage(err))
	}
	var code sqlite3.ExtendedErrorCode
	if errors.As(err, &code) {
		return sqliteFailure(int(code), sqliteMessage(err))
	}
	return cafleetErrorOf(err)
}

func initialize(conn *dbConn, databaseURL string) (err error) {
	failed := func(cause error) error {
		return appError("database initialization failed: " + sqliteMessage(cause))
	}
	if beginErr := conn.raw.Exec("BEGIN IMMEDIATE"); beginErr != nil {
		return failed(beginErr)
	}
	defer func() {
		if conn.raw.GetAutocommit() {
			return
		}
		if rollbackErr := conn.raw.Exec("ROLLBACK"); rollbackErr != nil {
			err = mergeCleanup(err, failed(rollbackErr))
		}
	}()
	kind, err := classifySchema(conn)
	if err != nil {
		return err
	}
	switch kind {
	case classificationCurrent:
		return nil
	case classificationIncompatible:
		path, err := databasePath(databaseURL)
		if err != nil {
			return err
		}
		return incompatibleFormat(path)
	case classificationMissing:
	default:
		panic("invalid classification")
	}
	if schemaErr := conn.raw.Exec(schemaSQL); schemaErr != nil {
		return failed(schemaErr)
	}
	if _, insertErr := executeRaw(conn, "INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, ?1)", []any{Version}); insertErr != nil {
		return failed(insertErr)
	}
	if commitErr := conn.raw.Exec("COMMIT"); commitErr != nil {
		return failed(commitErr)
	}
	return nil
}

type schemaStateKind uint8

const (
	schemaMissing schemaStateKind = iota + 1
	schemaCurrent
	schemaIncompatible
	schemaUnreachable
)

type schemaState struct {
	kind  schemaStateKind
	path  string
	cause *cafleetError
}

func (s schemaState) requireCurrent() error {
	switch s.kind {
	case schemaCurrent:
		return nil
	case schemaIncompatible:
		return incompatibleFormat(s.path)
	case schemaMissing:
		return appError("no cafleet database; run 'cafleet setup'")
	case schemaUnreachable:
		return s.cause
	default:
		panic("invalid schemaStateKind")
	}
}

func isEmptyFile(conn *dbConn) (bool, error) {
	identity, err := queryRow(conn, "PRAGMA application_id", nil, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return false, err
	}
	populated, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
		nil, func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil {
		return false, err
	}
	return identity == 0 && !populated, nil
}

func recordsRunningVersion(conn *dbConn) (bool, error) {
	hasVersionTable, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='cafleet_database')",
		nil, func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil || !hasVersionTable {
		return false, err
	}
	recorded, err := queryRowOptional(conn,
		"SELECT cafleet_version FROM cafleet_database WHERE singleton_id=1",
		nil, func(row sqlRow) (string, error) { return row.text(0) })
	if err != nil {
		return false, err
	}
	return recorded == some(Version), nil
}

func incompatibleFormat(path string) *cafleetError {
	return appError("database format is incompatible at '" + path + "'; choose an empty database path and run 'cafleet setup'.")
}

type classification uint8

const (
	classificationMissing classification = iota + 1
	classificationCurrent
	classificationIncompatible
)

func classifySchema(conn *dbConn) (classification, error) {
	empty, err := isEmptyFile(conn)
	if err != nil {
		return 0, err
	}
	if empty {
		return classificationMissing, nil
	}
	identity, err := queryRow(conn, "PRAGMA application_id", nil, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return 0, err
	}
	if identity != applicationID {
		return classificationIncompatible, nil
	}
	current, err := recordsRunningVersion(conn)
	if err != nil {
		return 0, err
	}
	if !current {
		return classificationIncompatible, nil
	}
	return classificationCurrent, nil
}

func classifySelectedSchema(conn *dbConn, databaseURL string) schemaState {
	kind, err := classifySchema(conn)
	if err != nil {
		return schemaState{kind: schemaUnreachable, cause: cafleetErrorOf(err)}
	}
	switch kind {
	case classificationMissing:
		return schemaState{kind: schemaMissing}
	case classificationCurrent:
		return schemaState{kind: schemaCurrent}
	case classificationIncompatible:
		path, err := databasePath(databaseURL)
		if err != nil {
			return schemaState{kind: schemaUnreachable, cause: cafleetErrorOf(err)}
		}
		return schemaState{kind: schemaIncompatible, path: path}
	default:
		panic("invalid classification")
	}
}

type scopeKind uint8

const (
	scopeWait scopeKind = iota + 1
	scopeCompletion
	scopeCancellable
)

// sharedDeadline is a deadline another goroutine may lower while a database scope that reads it
// is running.
type sharedDeadline struct {
	mu sync.Mutex
	at opt[time.Time]
}

func (s *sharedDeadline) instant() opt[time.Time] {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.at
}

func (s *sharedDeadline) lower(until time.Time) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if current, set := s.at.get(); !set || until.Before(current) {
		s.at = some(until)
	}
}

type databaseWait struct {
	deadline   deadline
	completion bool
	// Statements abort through the interrupt check once this passes.
	cancellation opt[time.Time]
	shared       *sharedDeadline
}

func (w databaseWait) lowered(until time.Time) time.Time {
	if w.shared == nil {
		return until
	}
	if shared, set := w.shared.instant().get(); set && shared.Before(until) {
		return shared
	}
	return until
}

// databaseScope is the chain of open database-wait scopes a context carries: the connection that
// owns them and their frames, outermost first. Each nested scope gets its own copy.
type databaseScope struct {
	owner  *dbConn
	frames []databaseWait
	busy   func(context.Context, int) bool
}

type databaseScopeKey struct{}

// databaseScopeContext is the context an operation inside a database scope receives, and the
// connection's interrupt context while that scope is innermost. The driver polls Err every
// 1,000 VM steps and before each busy retry; Err reports an expired cancellable deadline, and
// Done is nil because nothing signals that expiry.
type databaseScopeContext struct {
	context.Context
	scope *databaseScope
}

func (c databaseScopeContext) Value(key any) any {
	if key == (databaseScopeKey{}) {
		return c.scope
	}
	return c.Context.Value(key)
}

func (databaseScopeContext) Deadline() (time.Time, bool) { return time.Time{}, false }
func (databaseScopeContext) Done() <-chan struct{}       { return nil }

func (c databaseScopeContext) Err() error {
	if c.scope.cancelled() {
		return context.DeadlineExceeded
	}
	return nil
}

func scopeOf(ctx context.Context) *databaseScope {
	scope, _ := ctx.Value(databaseScopeKey{}).(*databaseScope)
	return scope
}

// innermostScopes are the frames from the innermost completion scope inward, so mandatory
// finalization after an irreversible effect is not bounded by an enclosing, already expired
// budget.
func (s *databaseScope) innermostScopes() []databaseWait {
	start := 0
	for index := len(s.frames) - 1; index >= 0; index-- {
		if s.frames[index].completion {
			start = index
			break
		}
	}
	return s.frames[start:]
}

func (s *databaseScope) innermostDeadline() deadline {
	scopes := s.innermostScopes()
	tightest := scopes[0].deadline
	for _, frame := range scopes[1:] {
		if frame.deadline.instant().Before(tightest.instant()) {
			tightest = frame.deadline
		}
	}
	return tightest
}

// remainingWait is the time left before the tightest relevant scope deadline, or false once one
// has passed.
func (s *databaseScope) remainingWait() (time.Duration, bool) {
	remaining := time.Duration(1<<63 - 1)
	for _, wait := range s.innermostScopes() {
		left, err := deadlineUntil(wait.lowered(wait.deadline.instant())).remaining()
		if err != nil {
			return 0, false
		}
		remaining = min(remaining, left)
	}
	return remaining, true
}

func (s *databaseScope) waitForDeadline(context.Context, int) bool {
	remaining, ok := s.remainingWait()
	if !ok {
		return false
	}
	time.Sleep(min(remaining, 10*time.Millisecond))
	_, ok = s.remainingWait()
	return ok
}

// cancelled reports whether the earliest cancellable deadline of the chain has passed; a chain
// without a cancellable scope never cancels.
func (s *databaseScope) cancelled() bool {
	var until opt[time.Time]
	for _, frame := range s.frames {
		cancellation, set := frame.cancellation.get()
		if !set {
			continue
		}
		lowered := frame.lowered(cancellation)
		if current, ok := until.get(); !ok || lowered.Before(current) {
			until = some(lowered)
		}
	}
	at, set := until.get()
	return set && !time.Now().Before(at)
}

func inheritedDeadline(ctx context.Context) opt[deadline] {
	scope := scopeOf(ctx)
	if scope == nil {
		return none[deadline]()
	}
	return some(scope.innermostDeadline())
}

func boundedByInherited(ctx context.Context, until time.Time) time.Time {
	if inherited, set := inheritedDeadline(ctx).get(); set && inherited.instant().Before(until) {
		return inherited.instant()
	}
	return until
}

// databaseBudget is one open database-wait scope; restore closes it exactly once, reinstating
// the busy handling and interrupt context in place before it opened.
type databaseBudget struct {
	conn              *dbConn
	parent            *databaseScope
	previousTimeout   time.Duration
	previousInterrupt context.Context
	active            bool
}

func openDatabaseScope(ctx context.Context, conn *dbConn, d deadline, kind scopeKind, shared *sharedDeadline) (databaseScopeContext, *databaseBudget, error) {
	parent := scopeOf(ctx)
	if parent != nil && parent.owner != conn {
		panic("nested database scopes use the same connection")
	}
	previous, err := queryRow(conn, "PRAGMA busy_timeout", nil, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return databaseScopeContext{}, nil, err
	}
	var frames []databaseWait
	if parent != nil {
		frames = append(frames, parent.frames...)
	}
	wait := databaseWait{deadline: d, completion: kind == scopeCompletion, shared: shared}
	if kind == scopeCancellable {
		cancellation := d.instant()
		if parent != nil && parent.innermostDeadline().instant().Before(cancellation) {
			cancellation = parent.innermostDeadline().instant()
		}
		wait.cancellation = some(cancellation)
	}
	scope := &databaseScope{owner: conn, frames: append(frames, wait)}
	scope.busy = scope.waitForDeadline
	if err := conn.raw.BusyHandler(scope.busy); err != nil {
		return databaseScopeContext{}, nil, sqliteError(err)
	}
	scopeCtx := databaseScopeContext{Context: ctx, scope: scope}
	budget := &databaseBudget{
		conn:              conn,
		parent:            parent,
		previousTimeout:   time.Duration(previous) * time.Millisecond,
		previousInterrupt: conn.raw.SetInterrupt(scopeCtx),
		active:            true,
	}
	return scopeCtx, budget, nil
}

func (b *databaseBudget) restore() error {
	if !b.active {
		return nil
	}
	b.active = false
	b.conn.raw.SetInterrupt(b.previousInterrupt)
	var restored error
	if b.parent != nil {
		restored = b.conn.raw.BusyHandler(b.parent.busy)
	} else {
		restored = b.conn.raw.BusyTimeout(b.previousTimeout)
	}
	return sqliteCloseError(restored)
}

func withDatabaseBudget[T any](ctx context.Context, conn *dbConn, d deadline, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	if _, err := d.remaining(); err != nil {
		var zero T
		return zero, err
	}
	return scopedDatabaseBudget(ctx, conn, d, scopeWait, nil, operation)
}

// withDatabaseDeadline is withDatabaseBudget until `until`, and statements also abort once it
// passes. A value or an interrupted statement that returns after `until` fails with
// deadlineExceeded; every other error returns unchanged.
func withDatabaseDeadline[T any](ctx context.Context, conn *dbConn, until time.Time, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	d := deadlineUntil(until)
	if _, err := d.remaining(); err != nil {
		var zero T
		return zero, err
	}
	value, err := scopedDatabaseBudget(ctx, conn, d, scopeCancellable, nil, operation)
	return pastDeadline(value, err, func() deadline { return d })
}

// withSharedDatabaseDeadline is withDatabaseDeadline, and the scope also ends once shared, which
// another goroutine may lower mid-statement, passes.
func withSharedDatabaseDeadline[T any](ctx context.Context, conn *dbConn, until time.Time, shared *sharedDeadline, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	effective := func() deadline {
		if lowered, set := shared.instant().get(); set && lowered.Before(until) {
			return deadlineUntil(lowered)
		}
		return deadlineUntil(until)
	}
	if _, err := effective().remaining(); err != nil {
		var zero T
		return zero, err
	}
	value, err := scopedDatabaseBudget(ctx, conn, deadlineUntil(until), scopeCancellable, shared, operation)
	return pastDeadline(value, err, effective)
}

func pastDeadline[T any](value T, err error, current func() deadline) (T, error) {
	var zero T
	if err == nil {
		if _, expired := current().remaining(); expired != nil {
			return zero, expired
		}
		return value, nil
	}
	if cafleetErrorOf(err).interrupted() {
		if _, expired := current().remaining(); expired != nil {
			return zero, deadlineExceededError()
		}
	}
	return zero, err
}

// completionGrace bounds mandatory storage finalization after irreversible pane side effects,
// independently of the input deadline.
const completionGrace = time.Second

func completeDatabaseBudget[T any](ctx context.Context, conn *dbConn, d deadline, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	until := d.instant()
	if grace := time.Now().Add(completionGrace); grace.After(until) {
		until = grace
	}
	return scopedDatabaseBudget(ctx, conn, deadlineUntil(until), scopeCompletion, nil, operation)
}

func scopedDatabaseBudget[T any](ctx context.Context, conn *dbConn, d deadline, kind scopeKind, shared *sharedDeadline, operation func(context.Context, *dbConn) (T, error)) (T, error) {
	scopeCtx, budget, err := openDatabaseScope(ctx, conn, d, kind, shared)
	if err != nil {
		var zero T
		return zero, err
	}
	value, err := func() (value T, err error) {
		defer func() {
			if recovered := recover(); recovered != nil {
				if restoreErr := budget.restore(); restoreErr != nil {
					panic("restoring a database wait while unwinding failed: " + restoreErr.Error())
				}
				panic(recovered)
			}
		}()
		return operation(scopeCtx, conn)
	}()
	var restored error
	if restoreErr := budget.restore(); restoreErr != nil {
		restored = appError("restoring database wait failed: " + cafleetErrorOf(restoreErr).message())
	}
	if merged := mergeCleanup(err, restored); merged != nil {
		var zero T
		return zero, merged
	}
	return value, nil
}

type transactionBehavior uint8

const (
	transactionDeferred transactionBehavior = iota + 1
	transactionImmediate
)

func (c *dbConn) begin(behavior transactionBehavior) error {
	var sql string
	switch behavior {
	case transactionDeferred:
		sql = "BEGIN DEFERRED"
	case transactionImmediate:
		sql = "BEGIN IMMEDIATE"
	default:
		panic("invalid transactionBehavior")
	}
	return c.executeBatch(sql)
}

// isAutocommit reports whether no transaction is open, including after SQLite ended one itself.
func (c *dbConn) isAutocommit() bool {
	return c.raw.GetAutocommit()
}

// rollbackIfOpen rolls back the open transaction, if SQLite has not already ended it.
func (c *dbConn) rollbackIfOpen() error {
	if c.isAutocommit() {
		return nil
	}
	return c.executeBatch("ROLLBACK")
}

// inTransaction runs operation inside one transaction that commits when operation succeeds and
// rolls back when operation or the commit fails, or operation panics.
func inTransaction[T any](conn *dbConn, behavior transactionBehavior, operation func() (T, error)) (T, error) {
	var zero T
	if err := conn.begin(behavior); err != nil {
		return zero, err
	}
	value, err := func() (value T, err error) {
		defer func() {
			if recovered := recover(); recovered != nil {
				if rollbackErr := conn.rollbackIfOpen(); rollbackErr != nil {
					panic(fmt.Sprintf("%v; rolling back the transaction also failed: %v", recovered, rollbackErr))
				}
				panic(recovered)
			}
		}()
		return operation()
	}()
	if err == nil {
		err = conn.executeBatch("COMMIT")
	}
	if err != nil {
		return zero, mergeCleanup(err, conn.rollbackIfOpen())
	}
	return value, nil
}

// pathExists reports whether path names an existing file, following symbolic links as Rust's
// Path::try_exists does: a missing target is false, every other failure an error.
func pathExists(path string) (bool, error) {
	_, err := os.Stat(path)
	switch {
	case err == nil:
		return true, nil
	case errors.Is(err, fs.ErrNotExist):
		return false, nil
	default:
		return false, err
	}
}

// bindArgs binds args to the statement's positional parameters ?1, ?2, ….
func bindArgs(stmt *sqlite3.Stmt, args []any) error {
	if stmt.BindCount() != len(args) {
		panic(fmt.Sprintf("a statement with %d parameters is bound with %d arguments", stmt.BindCount(), len(args)))
	}
	for index, arg := range args {
		param := index + 1
		var err error
		switch value := arg.(type) {
		case nil:
			err = stmt.BindNull(param)
		case int64:
			err = stmt.BindInt64(param, value)
		case int:
			err = stmt.BindInt64(param, int64(value))
		case bool:
			err = stmt.BindBool(param, value)
		case float64:
			err = stmt.BindFloat(param, value)
		case string:
			err = stmt.BindText(param, value)
		case []byte:
			err = stmt.BindBlob(param, value)
		case opt[string]:
			if text, set := value.get(); set {
				err = stmt.BindText(param, text)
			} else {
				err = stmt.BindNull(param)
			}
		case opt[int64]:
			if number, set := value.get(); set {
				err = stmt.BindInt64(param, number)
			} else {
				err = stmt.BindNull(param)
			}
		default:
			panic(fmt.Sprintf("unsupported SQL argument type %T", arg))
		}
		if err != nil {
			return err
		}
	}
	return nil
}

// prepareOne prepares sql, which holds exactly one statement.
func prepareOne(conn *dbConn, sql string) (*sqlite3.Stmt, error) {
	stmt, tail, err := conn.raw.Prepare(sql)
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(tail) != "" {
		closeStatement(stmt)
		panic("one statement is prepared at a time, got a second: " + tail)
	}
	return stmt, nil
}

// closeStatement finalizes stmt. Finalization reports again the error of the statement's last
// step, which its caller already handled, so only the release matters here.
func closeStatement(stmt *sqlite3.Stmt) {
	_ = stmt.Close()
}

// executeRaw runs one statement that returns no rows and reports the rows it changed, with
// driver errors unclassified.
func executeRaw(conn *dbConn, sql string, args []any) (int64, error) {
	stmt, err := prepareOne(conn, sql)
	if err != nil {
		return 0, err
	}
	defer closeStatement(stmt)
	if err := bindArgs(stmt, args); err != nil {
		return 0, err
	}
	if stmt.Step() {
		panic("a statement run with execute returns no rows: " + sql)
	}
	if err := stmt.Err(); err != nil {
		return 0, err
	}
	return conn.raw.Changes(), nil
}

// execute runs one statement that returns no rows and reports the rows it changed.
func (c *dbConn) execute(sql string, args ...any) (int64, error) {
	changed, err := executeRaw(c, sql, args)
	if err != nil {
		return 0, sqliteError(err)
	}
	return changed, nil
}

// executeBatch runs every statement of sql in order.
func (c *dbConn) executeBatch(sql string) error {
	if err := c.raw.Exec(sql); err != nil {
		return sqliteError(err)
	}
	return nil
}

func queryRows[T any](conn *dbConn, sql string, args []any, scan func(sqlRow) (T, error)) ([]T, error) {
	stmt, err := prepareOne(conn, sql)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer closeStatement(stmt)
	if err := bindArgs(stmt, args); err != nil {
		return nil, sqliteError(err)
	}
	rows := []T{}
	for stmt.Step() {
		value, err := scan(sqlRow{stmt: stmt})
		if err != nil {
			return nil, err
		}
		rows = append(rows, value)
	}
	if err := stmt.Err(); err != nil {
		return nil, sqliteError(err)
	}
	return rows, nil
}

// queryRowOptional scans the first row sql returns, or reports none when it returns no row.
func queryRowOptional[T comparable](conn *dbConn, sql string, args []any, scan func(sqlRow) (T, error)) (opt[T], error) {
	stmt, err := prepareOne(conn, sql)
	if err != nil {
		return none[T](), sqliteError(err)
	}
	defer closeStatement(stmt)
	if err := bindArgs(stmt, args); err != nil {
		return none[T](), sqliteError(err)
	}
	if !stmt.Step() {
		if err := stmt.Err(); err != nil {
			return none[T](), sqliteError(err)
		}
		return none[T](), nil
	}
	value, err := scan(sqlRow{stmt: stmt})
	if err != nil {
		return none[T](), err
	}
	return some(value), nil
}

// queryRow scans the first row sql returns; no row is an error.
func queryRow[T any](conn *dbConn, sql string, args []any, scan func(sqlRow) (T, error)) (T, error) {
	var zero T
	stmt, err := prepareOne(conn, sql)
	if err != nil {
		return zero, sqliteError(err)
	}
	defer closeStatement(stmt)
	if err := bindArgs(stmt, args); err != nil {
		return zero, sqliteError(err)
	}
	if !stmt.Step() {
		if err := stmt.Err(); err != nil {
			return zero, sqliteError(err)
		}
		return zero, appError("database error: Query returned no rows")
	}
	return scan(sqlRow{stmt: stmt})
}

// sqlRow is the current row of a statement being stepped.
type sqlRow struct {
	stmt *sqlite3.Stmt
}

func (r sqlRow) columnCount() int {
	return r.stmt.ColumnCount()
}

func sqliteTypeName(datatype sqlite3.Datatype) string {
	switch datatype {
	case sqlite3.INTEGER:
		return "Integer"
	case sqlite3.FLOAT:
		return "Real"
	case sqlite3.TEXT:
		return "Text"
	case sqlite3.BLOB:
		return "Blob"
	case sqlite3.NULL:
		return "Null"
	default:
		panic("invalid sqlite3.Datatype")
	}
}

func (r sqlRow) invalidType(column int) error {
	return appErrorf("database error: Invalid column type %s at index: %d, name: %s",
		sqliteTypeName(r.stmt.ColumnType(column)), column, r.stmt.ColumnName(column))
}

func (r sqlRow) int64(column int) (int64, error) {
	if r.stmt.ColumnType(column) != sqlite3.INTEGER {
		return 0, r.invalidType(column)
	}
	return r.stmt.ColumnInt64(column), nil
}

func (r sqlRow) bool(column int) (bool, error) {
	value, err := r.int64(column)
	return value != 0, err
}

func (r sqlRow) text(column int) (string, error) {
	if r.stmt.ColumnType(column) != sqlite3.TEXT {
		return "", r.invalidType(column)
	}
	raw := r.stmt.ColumnRawText(column)
	if !utf8.Valid(raw) {
		return "", appErrorf("database error: Conversion error from type Text at index: %d, %s", column, utf8ErrorText(raw))
	}
	return string(raw), nil
}

func (r sqlRow) optText(column int) (opt[string], error) {
	if r.stmt.ColumnType(column) == sqlite3.NULL {
		return none[string](), nil
	}
	text, err := r.text(column)
	if err != nil {
		return none[string](), err
	}
	return some(text), nil
}

func (r sqlRow) optInt64(column int) (opt[int64], error) {
	if r.stmt.ColumnType(column) == sqlite3.NULL {
		return none[int64](), nil
	}
	number, err := r.int64(column)
	if err != nil {
		return none[int64](), err
	}
	return some(number), nil
}

// value is the column's stored value: int64, float64, string, []byte or nil.
func (r sqlRow) value(column int) any {
	switch datatype := r.stmt.ColumnType(column); datatype {
	case sqlite3.INTEGER:
		return r.stmt.ColumnInt64(column)
	case sqlite3.FLOAT:
		return r.stmt.ColumnFloat(column)
	case sqlite3.TEXT:
		return string(r.stmt.ColumnRawText(column))
	case sqlite3.BLOB:
		return append([]byte{}, r.stmt.ColumnRawBlob(column)...)
	case sqlite3.NULL:
		return nil
	default:
		panic(fmt.Sprintf("invalid sqlite3.Datatype %d", datatype))
	}
}

// utf8ErrorText renders the first UTF-8 error in data as Rust's Utf8Error does.
func utf8ErrorText(data []byte) string {
	valid := 0
	for valid < len(data) {
		r, size := utf8.DecodeRune(data[valid:])
		if r == utf8.RuneError && size <= 1 {
			break
		}
		valid += size
	}
	invalid := maximalIllFormedPrefix(data[valid:])
	if valid+invalid == len(data) && invalid < utf8SequenceLength(data[valid]) {
		return fmt.Sprintf("incomplete utf-8 byte sequence from index %d", valid)
	}
	return fmt.Sprintf("invalid utf-8 sequence of %d bytes from index %d", invalid, valid)
}

// utf8SequenceLength is the sequence length a lead byte announces, or 1 for a byte that leads no
// valid sequence.
func utf8SequenceLength(lead byte) int {
	switch {
	case lead >= 0xc2 && lead <= 0xdf:
		return 2
	case lead >= 0xe0 && lead <= 0xef:
		return 3
	case lead >= 0xf0 && lead <= 0xf4:
		return 4
	default:
		return 1
	}
}
