package core

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/ncruces/go-sqlite3"
)

func databaseURLFor(path string) string {
	return "sqlite:///" + path
}

func initializedFileConn(t *testing.T) (*dbConn, string) {
	t.Helper()
	url := databaseURLFor(filepath.Join(t.TempDir(), "broker.db"))
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	if err := initialize(conn, url); err != nil {
		t.Fatal(err)
	}
	return conn, url
}

func memoryConn(t *testing.T) *dbConn {
	t.Helper()
	conn, err := connect("sqlite:///:memory:")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	return conn
}

func fileConn(t *testing.T, url string) *dbConn {
	t.Helper()
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	return conn
}

func closeConnection(t *testing.T, conn *dbConn) {
	t.Helper()
	if err := conn.close(); err != nil {
		t.Errorf("closing a test connection: %v", err)
	}
}

func sqlExecute(conn *dbConn, sql string, args ...any) (int64, error) {
	return executeRaw(conn, sql, args)
}

func sqlRows(conn *dbConn, sql string, args ...any) ([][]any, error) {
	return queryRows(conn, sql, args, func(row sqlRow) ([]any, error) {
		values := make([]any, row.columnCount())
		for column := range values {
			values[column] = row.value(column)
		}
		return values, nil
	})
}

func classifiedFailure(err error) error {
	if err == nil {
		return nil
	}
	return sqliteError(err)
}

func tableRows(t *testing.T, conn *dbConn, sql string, args ...any) [][]any {
	t.Helper()
	rows, err := sqlRows(conn, sql, args...)
	if err != nil {
		t.Fatalf("%s: %v", sql, err)
	}
	return rows
}

func queryInt64(t *testing.T, conn *dbConn, sql string, args ...any) int64 {
	t.Helper()
	rows := tableRows(t, conn, sql, args...)
	if len(rows) != 1 || len(rows[0]) != 1 {
		t.Fatalf("%s: want one value, got %v", sql, rows)
	}
	value, ok := rows[0][0].(int64)
	if !ok {
		t.Fatalf("%s: value %v is not an integer", sql, rows[0][0])
	}
	return value
}

func queryText(t *testing.T, conn *dbConn, sql string, args ...any) string {
	t.Helper()
	rows := tableRows(t, conn, sql, args...)
	if len(rows) != 1 || len(rows[0]) != 1 {
		t.Fatalf("%s: want one value, got %v", sql, rows)
	}
	value, ok := rows[0][0].(string)
	if !ok {
		t.Fatalf("%s: value %v is not text", sql, rows[0][0])
	}
	return value
}

func mustExecBatch(t *testing.T, conn *dbConn, sql string) {
	t.Helper()
	if err := conn.raw.Exec(sql); err != nil {
		t.Fatalf("%s: %v", sql, err)
	}
}

func rowsEqual(left, right [][]any) bool {
	return slices.EqualFunc(left, right, func(a, b []any) bool {
		return slices.EqualFunc(a, b, func(x, y any) bool {
			xb, xBlob := x.([]byte)
			yb, yBlob := y.([]byte)
			if xBlob || yBlob {
				return xBlob && yBlob && string(xb) == string(yb)
			}
			return x == y
		})
	})
}

type tableSnapshot struct {
	name string
	rows [][]any
}

func takeTableSnapshot(t *testing.T, conn *dbConn, include func(string) bool) []tableSnapshot {
	t.Helper()
	snapshots := []tableSnapshot{}
	for _, row := range tableRows(t, conn, "SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name") {
		name, ok := row[0].(string)
		if !ok {
			t.Fatalf("table name %v is not text", row[0])
		}
		if !include(name) {
			continue
		}
		quoted := strings.ReplaceAll(name, `"`, `""`)
		snapshots = append(snapshots, tableSnapshot{name: name, rows: tableRows(t, conn, `SELECT * FROM "`+quoted+`" ORDER BY rowid`)})
	}
	return snapshots
}

func tableSnapshotsEqual(left, right []tableSnapshot) bool {
	return slices.EqualFunc(left, right, func(a, b tableSnapshot) bool {
		return a.name == b.name && rowsEqual(a.rows, b.rows)
	})
}

// readBudgets records the inherited database budget of every read of table on conn.
type readBudgets struct {
	mu      sync.Mutex
	budgets []time.Time
}

func recordReadBudgets(t *testing.T, conn *dbConn, table string) *readBudgets {
	t.Helper()
	recorded := &readBudgets{}
	err := conn.raw.SetAuthorizer(func(action sqlite3.AuthorizerActionCode, name3rd, _, _, _ string) sqlite3.AuthorizerReturnCode {
		if action == sqlite3.AUTH_READ && name3rd == table {
			recorded.mu.Lock()
			recorded.budgets = append(recorded.budgets, deadlineInheritedOrNew(conn.raw.GetInterrupt()).instant())
			recorded.mu.Unlock()
		}
		return sqlite3.AUTH_OK
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := conn.raw.SetAuthorizer(nil); err != nil {
			t.Errorf("clearing the read-budget authorizer: %v", err)
		}
	})
	return recorded
}

func (r *readBudgets) take() []time.Time {
	r.mu.Lock()
	defer r.mu.Unlock()
	taken := r.budgets
	r.budgets = nil
	return taken
}

// faultPlan is a storage fault a test arms and clears. An abort plan installs a trigger in the
// database file, so every connection's matching statement fails with message; a refuse-commit
// plan turns each commit on the arming connection into a rollback.
type faultPlan struct {
	trigger, on, message string
	refuseCommit         bool
}

func faultPlanAbort(trigger, on, message string) faultPlan {
	return faultPlan{trigger: trigger, on: on, message: message}
}

func faultPlanRefuseCommit() faultPlan {
	return faultPlan{refuseCommit: true}
}

func (p faultPlan) arm(t *testing.T, conn *dbConn) {
	t.Helper()
	if p.refuseCommit {
		conn.raw.CommitHook(func() bool { return false })
		return
	}
	mustExecBatch(t, conn, fmt.Sprintf("CREATE TRIGGER %s %s BEGIN SELECT RAISE(ABORT,'%s'); END", p.trigger, p.on, p.message))
}

func (p faultPlan) clear(t *testing.T, conn *dbConn) {
	t.Helper()
	if p.refuseCommit {
		conn.raw.CommitHook(nil)
		return
	}
	mustExecBatch(t, conn, "DROP TRIGGER "+p.trigger)
}

type fileBytes struct {
	path  string
	bytes []byte
}

func walSnapshot(t *testing.T, database string) [3]fileBytes {
	t.Helper()
	var snapshot [3]fileBytes
	for index, suffix := range []string{"", "-wal", "-shm"} {
		path := database + suffix
		bytes, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		snapshot[index] = fileBytes{path: path, bytes: bytes}
	}
	return snapshot
}

func noValue(operation func(context.Context, *dbConn) error) func(context.Context, *dbConn) (struct{}, error) {
	return func(ctx context.Context, conn *dbConn) (struct{}, error) {
		return struct{}{}, operation(ctx, conn)
	}
}

func cafleetKind(err error) (errorKind, bool) {
	var failure *cafleetError
	if !errors.As(err, &failure) {
		return 0, false
	}
	return failure.kind, true
}

func TestDb_ExistingFileOpener_AbsentFilesReturnNoneAndRemainAbsentInBothModes(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	for _, readOnly := range []bool{false, true} {
		path := filepath.Join(dir, fmt.Sprintf("missing-%v.db", readOnly))
		opened, err := openExisting(databaseURLFor(path), readOnly)
		if _, ok := opened.get(); err != nil || ok {
			t.Fatalf("readOnly=%v: openExisting = %v %v, want no connection", readOnly, opened, err)
		}
		if _, err := os.Lstat(path); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("readOnly=%v: opening an absent file preserves absence: %v", readOnly, err)
		}
	}
}

func existingFileWithProbe(t *testing.T) string {
	t.Helper()
	url := databaseURLFor(filepath.Join(t.TempDir(), "existing.db"))
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	mustExecBatch(t, conn, "CREATE TABLE probe(value INTEGER NOT NULL); INSERT INTO probe VALUES(7);")
	closeConnection(t, conn)
	return url
}

func assertConnectionPragmas(t *testing.T, conn *dbConn) {
	t.Helper()
	if got := queryInt64(t, conn, "PRAGMA foreign_keys"); got != 1 {
		t.Fatalf("foreign_keys = %d, want 1", got)
	}
	if got := queryInt64(t, conn, "PRAGMA busy_timeout"); got != 5000 {
		t.Fatalf("busy_timeout = %d, want 5000", got)
	}
}

func TestDb_ExistingFileOpener_AnExistingWritableConnectionKeepsPragmasAndPersistsWrites(t *testing.T) {
	t.Parallel()
	url := existingFileWithProbe(t)
	opened, err := openExisting(url, false)
	conn, ok := opened.get()
	if err != nil || !ok {
		t.Fatalf("openExisting = %v %v", ok, err)
	}
	assertConnectionPragmas(t, conn)
	if changed, err := sqlExecute(conn, "UPDATE probe SET value=11"); err != nil || changed != 1 {
		t.Fatalf("update = %d %v", changed, err)
	}
	closeConnection(t, conn)
	reopened, err := connectReadOnly(url)
	if err != nil {
		t.Fatal(err)
	}
	defer closeConnection(t, reopened)
	if got := queryInt64(t, reopened, "SELECT value FROM probe"); got != 11 {
		t.Fatalf("value = %d, want 11", got)
	}
}

func TestDb_ExistingFileOpener_AnExistingReadOnlyConnectionKeepsPragmasAndRefusesWrites(t *testing.T) {
	t.Parallel()
	url := existingFileWithProbe(t)
	opened, err := openExisting(url, true)
	conn, ok := opened.get()
	if err != nil || !ok {
		t.Fatalf("openExisting = %v %v", ok, err)
	}
	defer closeConnection(t, conn)
	assertConnectionPragmas(t, conn)
	_, failure := sqlExecute(conn, "UPDATE probe SET value=11")
	if !errors.Is(failure, sqlite3.READONLY) {
		t.Fatalf("a valid write is refused as read-only: %v", failure)
	}
	mapped := classifiedFailure(failure)
	if kind, _ := cafleetKind(mapped); kind != errorDatabaseIO || mapped.Error() != "database error: attempt to write a readonly database" {
		t.Fatalf("sqliteError = %v %q", kind, mapped)
	}
	if got := queryInt64(t, conn, "SELECT value FROM probe"); got != 7 {
		t.Fatalf("value = %d, want 7", got)
	}
}

func requireCurrentSchema(conn *dbConn) error {
	return classifySelectedSchema(conn, "sqlite:///:memory:").requireCurrent()
}

func initializedMemoryConn(t *testing.T) *dbConn {
	t.Helper()
	conn := memoryConn(t)
	if err := initialize(conn, "sqlite:///:memory:"); err != nil {
		t.Fatal(err)
	}
	return conn
}

func applicationRows(t *testing.T, conn *dbConn) [][][]any {
	t.Helper()
	queries := []string{
		"SELECT * FROM fleets ORDER BY fleet_id",
		"SELECT * FROM members ORDER BY member_id",
		"SELECT * FROM member_placements ORDER BY member_id",
		"SELECT message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at FROM messages ORDER BY message_id",
		"SELECT * FROM sqlite_sequence ORDER BY name",
	}
	result := make([][][]any, len(queries))
	for index, sql := range queries {
		result[index] = tableRows(t, conn, sql)
	}
	return result
}

func TestDb_RepeatedInitializationPreservesAllApplicationRows(t *testing.T) {
	t.Parallel()
	conn := initializedMemoryConn(t)
	mustExecBatch(t, conn, `INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'preserved', 'created');
		INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES
			(1, 1, 'Director', 'director assignment', 'active', 'registered'),
			(2, 1, 'worker', 'worker assignment', 'active', 'registered'),
			(3, 1, 'departed', 'finished assignment', 'deregistered', 'registered');
		INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side, mux_pane_id, backend, mux_server, coding_agent, created_at) VALUES
			(1, 'main', '@1', 'main', 0, 'left', '%0', 'tmux', '/tmp/tmux-0/default', 'claude', 'placed'),
			(2, 'main', NULL, NULL, 1, 'right', NULL, 'tmux', '/tmp/tmux-0/default', 'codex', 'placed');
		UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;
		INSERT INTO messages(message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at) VALUES
			(7, 1, 2, 'created-7', 'input_required', 'status-7', 'pending unicode 世界', NULL),
			(8, 1, 2, 'created-8', 'input_required', 'status-8', 'notified', 'notified-8'),
			(9, 2, 1, 'created-9', 'completed', 'status-9', 'completed', NULL);`)
	if err := requireCurrentSchema(conn); err != nil {
		t.Fatal(err)
	}
	before := applicationRows(t, conn)
	everyTable := takeTableSnapshot(t, conn, func(string) bool { return true })
	versionRow := tableRows(t, conn, "SELECT * FROM cafleet_database")
	for round := range 2 {
		if err := initialize(conn, "sqlite:///:memory:"); err != nil {
			t.Fatalf("round %d: %v", round, err)
		}
		after := applicationRows(t, conn)
		for index := range before {
			if !rowsEqual(after[index], before[index]) {
				t.Fatalf("round %d: application rows %d changed: %v -> %v", round, index, before[index], after[index])
			}
		}
		if !tableSnapshotsEqual(takeTableSnapshot(t, conn, func(string) bool { return true }), everyTable) {
			t.Fatalf("round %d: a table changed", round)
		}
		if !rowsEqual(tableRows(t, conn, "SELECT * FROM cafleet_database"), versionRow) {
			t.Fatalf("round %d: the version row changed", round)
		}
		if err := requireCurrentSchema(conn); err != nil {
			t.Fatalf("round %d: %v", round, err)
		}
	}
}

func TestDb_InitializationRecordsTheIdentityAndVersion(t *testing.T) {
	t.Parallel()
	if applicationID != 0x4341_4639 {
		t.Fatalf("applicationID = %#x, want ASCII CAF9", applicationID)
	}
	conn := initializedMemoryConn(t)
	if got := queryInt64(t, conn, "PRAGMA application_id"); got != applicationID {
		t.Fatalf("application_id = %d", got)
	}
	want := [][]any{{int64(1), Version}}
	if got := tableRows(t, conn, "SELECT singleton_id, cafleet_version FROM cafleet_database"); !rowsEqual(got, want) {
		t.Fatalf("cafleet_database = %v, want %v", got, want)
	}
}

type preparedFile struct {
	path  string
	url   string
	bytes []byte
}

func newPreparedFile(t *testing.T, name string, prepare func(*dbConn, string)) preparedFile {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	url := databaseURLFor(path)
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	prepare(conn, url)
	closeConnection(t, conn)
	bytes, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return preparedFile{path: path, url: url, bytes: bytes}
}

func (f preparedFile) incompatibleMessage() string {
	return "database format is incompatible at '" + f.path + "'; choose an empty database path and run 'cafleet setup'."
}

func (f preparedFile) assertUnchanged(t *testing.T, name string) {
	t.Helper()
	bytes, err := os.ReadFile(f.path)
	if err != nil {
		t.Fatal(err)
	}
	if string(bytes) != string(f.bytes) {
		t.Fatalf("%s: the file bytes changed", name)
	}
}

func TestDb_AV8FamilyFileIsIncompatibleAndLeftByteIdentical(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		damage string
	}{
		{name: "cafleet_v8.db", damage: "PRAGMA application_id=1128351288"},
		{name: "cafleet_0.0.0.db", damage: "UPDATE cafleet_database SET cafleet_version='0.0.0'"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			file := newPreparedFile(t, tc.name, func(conn *dbConn, url string) {
				if err := initialize(conn, url); err != nil {
					t.Fatal(err)
				}
				mustExecBatch(t, conn, tc.damage)
			})
			conn, err := connectReadOnly(file.url)
			if err != nil {
				t.Fatal(err)
			}
			state := classifySelectedSchema(conn, file.url)
			if state.kind != schemaIncompatible || state.path != file.path {
				t.Fatalf("%s: state = %#v, want incompatible at %s", tc.name, state, file.path)
			}
			if err := state.requireCurrent(); err == nil || err.Error() != file.incompatibleMessage() {
				t.Fatalf("%s: requireCurrent = %v", tc.name, err)
			}
			closeConnection(t, conn)
			file.assertUnchanged(t, tc.name)
		})
	}
}

func TestDb_InitializationRefusesEveryOtherFile(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		initialized bool
		damage      string
	}{
		{name: "recorded-other-version", initialized: true, damage: "UPDATE cafleet_database SET cafleet_version='0.0.0'"},
		{name: "identity-without-version-row", initialized: true, damage: "DELETE FROM cafleet_database"},
		{name: "identity-without-version-table", initialized: true, damage: "DROP TABLE cafleet_database"},
		{name: "unrelated-table-only", initialized: false, damage: "CREATE TABLE unrelated(value TEXT)"},
		{name: "foreign-identity-without-tables", initialized: false, damage: "PRAGMA application_id=42"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			file := newPreparedFile(t, tc.name+".db", func(conn *dbConn, url string) {
				if tc.initialized {
					if err := initialize(conn, url); err != nil {
						t.Fatal(err)
					}
				}
				mustExecBatch(t, conn, tc.damage)
			})
			conn, err := connect(file.url)
			if err != nil {
				t.Fatal(err)
			}
			failure := initialize(conn, file.url)
			if failure == nil || failure.Error() != file.incompatibleMessage() {
				t.Fatalf("%s: initialize = %v, want the refusal naming the URL's path", tc.name, failure)
			}
			closeConnection(t, conn)
			file.assertUnchanged(t, tc.name)
		})
	}
}

func TestDb_ConcurrentInitializationOfOneFreshFileAppliesTheSchemaOnce(t *testing.T) {
	t.Parallel()
	url := databaseURLFor(filepath.Join(t.TempDir(), "fresh.db"))
	var ready, start sync.WaitGroup
	ready.Add(2)
	start.Add(1)
	results := make([]error, 2)
	var done sync.WaitGroup
	for index := range results {
		done.Add(1)
		go func() {
			defer done.Done()
			conn, err := connect(url)
			if err != nil {
				results[index] = err
				ready.Done()
				return
			}
			ready.Done()
			start.Wait()
			results[index] = errors.Join(initialize(conn, url), conn.close())
		}()
	}
	ready.Wait()
	start.Done()
	done.Wait()
	for index, err := range results {
		if err != nil {
			t.Fatalf("initializer %d: %v", index, err)
		}
	}
	conn := fileConn(t, url)
	if rows := tableRows(t, conn, "SELECT * FROM cafleet_database"); len(rows) != 1 {
		t.Fatalf("cafleet_database rows = %v", rows)
	}
	if err := requireCurrentSchema(conn); err != nil {
		t.Fatal(err)
	}
}

func TestDb_ConnectionPragmasAndForeignKeysProtectRealWrites(t *testing.T) {
	t.Parallel()
	conn := initializedMemoryConn(t)
	if _, err := sqlExecute(conn, "INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(404, 'orphan', '', 'active', '')"); err == nil {
		t.Fatal("an orphan member is refused")
	}
	mustExecBatch(t, conn, "INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'valid', 'created'); INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES(1, 1, 'valid', '', 'active', 'registered');")
	for _, pair := range [][2]int{{404, 1}, {1, 404}} {
		if _, err := sqlExecute(conn, "INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '', 'input_required', '', '')", pair[0], pair[1]); err == nil {
			t.Fatalf("a message %v naming an absent member is refused", pair)
		}
	}
	for _, url := range []string{"postgresql:///registry", "mysql://localhost/registry"} {
		_, err := connect(url)
		want := "database URL must use the sqlite scheme (sqlite:///<path>); got '" + url + "'"
		if kind, _ := cafleetKind(err); err == nil || kind != errorApp || err.Error() != want {
			t.Fatalf("connect(%q) = %v, want %q", url, err, want)
		}
	}
}

func TestDb_DatabasePathNamesTheFileOfASqliteUrl(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		url   string
		path  string
		cause string
	}{
		{name: "absolute path", url: "sqlite:////srv/cafleet.db", path: "/srv/cafleet.db"},
		{name: "relative path", url: "sqlite:///cafleet.db", path: "cafleet.db"},
		{name: "memory", url: "sqlite:///:memory:", path: ":memory:"},
		{name: "two slashes", url: "sqlite://cafleet.db", cause: "database URL must use the sqlite scheme (sqlite:///<path>); got 'sqlite://cafleet.db'"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			path, err := databasePath(tc.url)
			if tc.cause != "" {
				if err == nil || err.Error() != tc.cause {
					t.Fatalf("%s: databasePath = %q %v, want %q", tc.name, path, err, tc.cause)
				}
				return
			}
			if err != nil || path != tc.path {
				t.Fatalf("%s: databasePath = %q %v, want %q", tc.name, path, err, tc.path)
			}
		})
	}
}

func TestDb_ConnectReportsAnUnopenableDatabase(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "absent-directory", "cafleet.db")
	_, err := connect(databaseURLFor(path))
	want := "failed to open database at '" + path + "': unable to open database file"
	if kind, _ := cafleetKind(err); err == nil || kind != errorApp || err.Error() != want {
		t.Fatalf("connect = %v, want %q", err, want)
	}
}

func placementFleetWithMember(t *testing.T) *dbConn {
	t.Helper()
	conn := initializedMemoryConn(t)
	mustExecBatch(t, conn, `INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'layout', 'created');
		INSERT INTO members(member_id, fleet_id, name, description, status, registered_at)
			VALUES(1, 1, 'worker', '', 'active', 'registered');`)
	return conn
}

func insertPlacement(conn *dbConn, windowID, windowName, paneID any, ordinal int, side string) error {
	_, err := sqlExecute(conn, `INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_window_name,
			window_ordinal, column_side, mux_pane_id, backend, mux_server, coding_agent, created_at)
		VALUES(1, 'main', ?1, ?2, ?3, ?4, ?5, 'tmux', '/tmp/tmux-0/default', 'claude', 'placed')`,
		windowID, windowName, ordinal, side, paneID)
	return err
}

func TestDb_PlacementShape_APlacementRecordsItsPaneWindowIdAndWindowNameAllTogetherOrNone(t *testing.T) {
	t.Parallel()
	accepted := [][3]any{{nil, nil, nil}, {"@3", "cafleet-1-1", "%7"}, {"@3", "", "%7"}}
	for _, fields := range accepted {
		if err := insertPlacement(placementFleetWithMember(t), fields[0], fields[1], fields[2], 1, "left"); err != nil {
			t.Fatalf("%v: %v", fields, err)
		}
	}
	refused := [][3]any{
		{"@3", nil, nil}, {nil, "cafleet-1-1", nil}, {nil, nil, "%7"},
		{"@3", "cafleet-1-1", nil}, {"@3", nil, "%7"}, {nil, "cafleet-1-1", "%7"},
	}
	for _, fields := range refused {
		if err := insertPlacement(placementFleetWithMember(t), fields[0], fields[1], fields[2], 1, "left"); err == nil {
			t.Fatalf("%v: a partial placement is refused", fields)
		}
	}
}

func TestDb_PlacementShape_PublishingAPendingPlacementRequiresEveryConcreteFieldInOneUpdate(t *testing.T) {
	t.Parallel()
	conn := placementFleetWithMember(t)
	if err := insertPlacement(conn, nil, nil, nil, 2, "right"); err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(conn, "UPDATE member_placements SET mux_pane_id='%7' WHERE member_id=1"); err == nil {
		t.Fatal("publishing only the pane is refused")
	}
	changed, err := sqlExecute(conn, "UPDATE member_placements SET mux_pane_id='%7', mux_window_id='@3', mux_window_name='cafleet-1-2' WHERE member_id=1 AND mux_pane_id IS NULL")
	if err != nil || changed != 1 {
		t.Fatalf("publishing every field = %d %v", changed, err)
	}
}

func TestDb_PlacementShape_WindowOrdinalAndColumnSideAcceptOnlyTheirDomains(t *testing.T) {
	t.Parallel()
	cases := []struct {
		ordinal  int
		side     string
		accepted bool
	}{
		{0, "left", true}, {0, "right", true}, {5, "left", true},
		{-1, "left", false}, {1, "middle", false}, {1, "Left", false}, {1, "", false},
	}
	for _, tc := range cases {
		err := insertPlacement(placementFleetWithMember(t), nil, nil, nil, tc.ordinal, tc.side)
		if (err == nil) != tc.accepted {
			t.Fatalf("%d %q: insert error %v, accepted want %v", tc.ordinal, tc.side, err, tc.accepted)
		}
	}
}

func TestDb_PlacementShape_AFleetStartsAtWindowOrdinalZeroAndRefusesANegativeOrdinal(t *testing.T) {
	t.Parallel()
	conn := placementFleetWithMember(t)
	if got := queryInt64(t, conn, "SELECT last_window_ordinal FROM fleets WHERE fleet_id=1"); got != 0 {
		t.Fatalf("last_window_ordinal = %d", got)
	}
	if _, err := sqlExecute(conn, "UPDATE fleets SET last_window_ordinal=-1 WHERE fleet_id=1"); err == nil {
		t.Fatal("a negative ordinal is refused")
	}
	if _, err := sqlExecute(conn, "UPDATE fleets SET last_window_ordinal=3 WHERE fleet_id=1"); err != nil {
		t.Fatal(err)
	}
}

func TestDb_FailedInitializationRollsBackIdentityAndTables(t *testing.T) {
	t.Parallel()
	conn := memoryConn(t)
	mustExecBatch(t, conn, "PRAGMA max_page_count=1")
	if err := initialize(conn, "sqlite:///:memory:"); err == nil || !strings.HasPrefix(err.Error(), "database initialization failed: ") {
		t.Fatalf("initialize = %v, want an initialization failure", err)
	}
	if !conn.raw.GetAutocommit() {
		t.Fatal("the failed initialization leaves no open transaction")
	}
	if got := queryInt64(t, conn, "PRAGMA application_id"); got != 0 {
		t.Fatalf("application_id = %d", got)
	}
	if rows := tableRows(t, conn, "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"); len(rows) != 0 {
		t.Fatalf("tables = %v", rows)
	}
}

const runtimeFixtureNow = "2026-10-26T10:00:00.000000+00:00"

type runtimeFixture struct {
	conn     *dbConn
	url      string
	fleet    int64
	director int64
	member   int64
}

func newRuntimeFixture(t *testing.T) runtimeFixture {
	t.Helper()
	conn, url := initializedFileConn(t)
	mustExecBatch(t, conn, `INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'runtime-schema', '`+runtimeFixtureNow+`');
		INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES
			(1, 1, 'Director', 'Root Director', 'active', '`+runtimeFixtureNow+`'),
			(2, 1, 'worker', 'worker', 'active', '`+runtimeFixtureNow+`');
		UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;`)
	return runtimeFixture{conn: conn, url: url, fleet: 1, director: 1, member: 2}
}

func (f runtimeFixture) run(status string) (int64, error) {
	return f.backendRun("codex", status, nil)
}

func (f runtimeFixture) backendRun(backend, status string, nativeSessionJSON any) (int64, error) {
	path, err := databasePath(f.url)
	if err != nil {
		return 0, err
	}
	identity := fmt.Sprintf(`{"canonical_path":%q,"instance_id":%q,"device":1,"inode":2}`, path, strings.Repeat("a", 32))
	_, err = sqlExecute(f.conn, `INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,
		database_identity_json,launch_nonce,runtime_path,resolved_mode,mode_source,
		permission_argv_json,coverage,status,
		owner_epoch,owner_kind,owner_pid,owner_start_identity,
		cleanup_state,native_session_json)
		VALUES(?1,?2,?3,?7,?4,?5,'/isolated/runtime','never','default',
		'["--ask-for-approval","never","--sandbox","workspace-write"]',
		'unknown',?6,1,'creator',1234,'fixture-owner',
		'incomplete',?8)`,
		f.member, f.fleet, f.director, identity, strings.Repeat("b", 64), status, backend, nativeSessionJSON)
	if err != nil {
		return 0, err
	}
	return f.conn.raw.LastInsertRowID(), nil
}

func (f runtimeFixture) interaction(run, seq int64, class string) (int64, error) {
	_, err := sqlExecute(f.conn, `INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json)
		VALUES(?1,?2,?3,?4,?5,'{}')`, run, seq, class, strings.Repeat("c", 64), runtimeFixtureNow)
	if err != nil {
		return 0, err
	}
	return f.conn.raw.LastInsertRowID(), nil
}

func (f runtimeFixture) archivedEvent(run, seq int64) (int64, error) {
	_, err := sqlExecute(f.conn, `INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,
		payload_json,notification_state,archive_reason)
		VALUES(?1,?2,'coverage_unknown',?3,'{}','archived','member_inactive')`, run, seq, runtimeFixtureNow)
	if err != nil {
		return 0, err
	}
	return f.conn.raw.LastInsertRowID(), nil
}

func (f runtimeFixture) input(run int64, key string) (int64, error) {
	_, err := sqlExecute(f.conn, `INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at)
		VALUES(?1,?2,'prompt',?3,'in_progress',?4,?4)`, run, key, strings.Repeat("d", 64), runtimeFixtureNow)
	if err != nil {
		return 0, err
	}
	return f.conn.raw.LastInsertRowID(), nil
}

func (f runtimeFixture) message(t *testing.T) int64 {
	t.Helper()
	if _, err := sqlExecute(f.conn, `INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text)
		VALUES(?1, ?2, ?3, 'input_required', ?3, 'runtime')`, f.member, f.director, runtimeFixtureNow); err != nil {
		t.Fatal(err)
	}
	return f.conn.raw.LastInsertRowID()
}

func mustID(t *testing.T) func(int64, error) int64 {
	t.Helper()
	return func(id int64, err error) int64 {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
		return id
	}
}

func tableCount(t *testing.T, conn *dbConn, table string) int64 {
	t.Helper()
	return queryInt64(t, conn, "SELECT COUNT(*) FROM "+table)
}

func TestDb_RuntimeTests_DatabaseInstanceIdentityIsSingletonStableAndDistinct(t *testing.T) {
	t.Parallel()
	first := newRuntimeFixture(t)
	second := newRuntimeFixture(t)
	identity := func(conn *dbConn) string {
		return queryText(t, conn, "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1")
	}
	original := identity(first.conn)
	if len(original) != 32 || strings.Trim(original, "0123456789abcdef") != "" {
		t.Fatalf("instance_id = %q", original)
	}
	if original == identity(second.conn) {
		t.Fatal("two databases share an instance identity")
	}
	if err := initialize(first.conn, first.url); err != nil {
		t.Fatal(err)
	}
	if got := identity(fileConn(t, first.url)); got != original {
		t.Fatalf("re-initialization changed the identity to %q", got)
	}
	if got := tableCount(t, first.conn, "runtime_database_identity"); got != 1 {
		t.Fatalf("identity rows = %d", got)
	}
	if _, err := sqlExecute(first.conn, "INSERT INTO runtime_database_identity(singleton_id,instance_id) VALUES(2,?1)", strings.Repeat("e", 32)); err == nil {
		t.Fatal("a second identity row is refused")
	}
}

func TestDb_RuntimeTests_EveryNonterminalStatusEnforcesOneLiveRunPerMember(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	for _, status := range []string{"starting", "prepared", "running", "degraded", "recovering"} {
		run := mustID(t)(fixture.run(status))
		if _, err := fixture.run("starting"); err == nil {
			t.Fatalf("%s: a second live run is refused", status)
		}
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET status='exited' WHERE run_id=?1", run); err != nil {
			t.Fatal(err)
		}
		next := mustID(t)(fixture.run("starting"))
		if next <= run {
			t.Fatalf("%s: run %d follows %d", status, next, run)
		}
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET status='exited' WHERE run_id=?1", next); err != nil {
			t.Fatal(err)
		}
	}
}

func TestDb_RuntimeTests_TerminalRunHistoryAllowsANewRunWithoutLosingTheOldOne(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	for index, status := range []string{"exited", "failed", "lost"} {
		old := mustID(t)(fixture.run(status))
		next := mustID(t)(fixture.run("starting"))
		if next <= old {
			t.Fatalf("%s: run %d follows %d", status, next, old)
		}
		if got := tableCount(t, fixture.conn, "member_runs"); got != int64(2*(index+1)) {
			t.Fatalf("%s: runs = %d", status, got)
		}
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET status='exited' WHERE run_id=?1", next); err != nil {
			t.Fatal(err)
		}
	}
}

func TestDb_RuntimeTests_RunSqlDomainsAndJsonShapesRejectCorruptValues(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("starting"))
	assignments := [][2]string{
		{"status", "bogus"}, {"mode_source", "inherited"}, {"owner_kind", "member"},
		{"cleanup_state", "unknown"}, {"coverage", "universal"}, {"backend", "bogus"},
		{"database_identity_json", "[]"}, {"permission_argv_json", "{}"},
		{"cleanup_diagnostics_json", "{}"}, {"runtime_dir_identity_json", "[]"}, {"run_lock_identity_json", "[]"},
	}
	for _, assignment := range assignments {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET "+assignment[0]+"=?1 WHERE run_id=?2", assignment[1], run); err == nil {
			t.Fatalf("%s=%s is refused", assignment[0], assignment[1])
		}
	}
	for _, field := range []string{"owner_epoch", "owner_pid"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET "+field+"=0 WHERE run_id=?1", run); err == nil {
			t.Fatalf("%s=0 is refused", field)
		}
	}
}

func TestDb_RuntimeTests_RunBackendCheckAcceptsEverySupportedBackendOnly(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	for _, backend := range []string{"claude", "codex", "opencode", "hermes", "pi"} {
		if _, err := fixture.backendRun(backend, "exited", nil); err != nil {
			t.Fatalf("%s: %v", backend, err)
		}
	}
	for _, backend := range []string{"gemini", "Hermes"} {
		if _, err := fixture.backendRun(backend, "exited", nil); err == nil {
			t.Fatalf("%s is refused", backend)
		}
	}
}

func TestDb_RuntimeTests_NativeSessionColumnAcceptsOnlyNullOrAJsonObject(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	for _, session := range []string{"{}", `{"session_key":"tool:abc","pid":12}`} {
		if _, err := fixture.backendRun("hermes", "exited", session); err != nil {
			t.Fatalf("%s: %v", session, err)
		}
	}
	for _, session := range []string{"[]", "not json"} {
		if _, err := fixture.backendRun("hermes", "exited", session); err == nil {
			t.Fatalf("%q is refused", session)
		}
	}
	run := mustID(t)(fixture.backendRun("hermes", "exited", nil))
	if _, err := sqlExecute(fixture.conn, `UPDATE member_runs SET native_session_json='{"session_key":"k"}' WHERE run_id=?1`, run); err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET native_session_json='[1]' WHERE run_id=?1", run); err == nil {
		t.Fatal("an array session is refused")
	}
}

func closeInteraction(t *testing.T, fixture runtimeFixture, interaction int64) {
	t.Helper()
	if _, err := sqlExecute(fixture.conn, "UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2", runtimeFixtureNow, interaction); err != nil {
		t.Fatal(err)
	}
}

func TestDb_RuntimeTests_InteractionIdentityAndTheSingleOpenEpisodeAreEnforced(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	first := mustID(t)(fixture.interaction(run, 1, "permission_request"))
	if _, err := fixture.interaction(run, 2, "question"); err == nil {
		t.Fatal("a second open interaction is refused")
	}
	closeInteraction(t, fixture, first)
	if _, err := fixture.interaction(run, 1, "question"); err == nil {
		t.Fatal("a repeated episode sequence is refused")
	}
	if next := mustID(t)(fixture.interaction(run, 2, "question")); next <= first {
		t.Fatalf("interaction %d follows %d", next, first)
	}
}

func TestDb_RuntimeTests_InteractionClassesPreserveDistinctNativeForms(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	var seq int64
	for _, class := range []string{"permission_request", "question", "unknown_interaction"} {
		seq++
		closeInteraction(t, fixture, mustID(t)(fixture.interaction(run, seq, class)))
	}
	for _, class := range []string{"composer_empty", "unknown", "bogus"} {
		if _, err := fixture.interaction(run, seq+1, class); err == nil {
			t.Fatalf("%s is refused", class)
		}
	}
}

func TestDb_RuntimeTests_InteractionClosureFieldsArePairedAndReasonsAreTyped(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	interaction := mustID(t)(fixture.interaction(run, 1, "permission_request"))
	for _, assignment := range []string{
		"closed_at='2026-10-26T10:00:00Z'",
		"close_reason='not_visible'",
		"closed_at='2026-10-26T10:00:00Z',close_reason='approved'",
		"evidence_json='[]'",
		"episode_seq=0",
	} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_interactions SET "+assignment+" WHERE interaction_id=?1", interaction); err == nil {
			t.Fatalf("%s is refused", assignment)
		}
	}
	for _, reason := range []string{"not_visible", "superseded", "process_exited", "monitor_lost"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_interactions SET closed_at=?1,close_reason=?2 WHERE interaction_id=?3", runtimeFixtureNow, reason, interaction); err != nil {
			t.Fatalf("%s: %v", reason, err)
		}
	}
}

func TestDb_RuntimeTests_RuntimeEventSequencesArePositiveAndUniqueWithinARun(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	mustID(t)(fixture.archivedEvent(run, 1))
	for _, seq := range []int64{1, 0, -1} {
		if _, err := fixture.archivedEvent(run, seq); err == nil {
			t.Fatalf("event sequence %d is refused", seq)
		}
	}
	old := mustID(t)(fixture.run("exited"))
	mustID(t)(fixture.archivedEvent(old, 1))
}

func TestDb_RuntimeTests_NotificationStateRequiresExactlyItsMessageOrArchiveReason(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	event := mustID(t)(fixture.archivedEvent(run, 1))
	message := fixture.message(t)
	for _, assignment := range []string{
		"notification_state='inbox'",
		"archive_reason=NULL",
		"archive_reason='notified'",
		"notification_state='unknown'",
		"kind='approved'",
		"payload_json='[]'",
		"payload_json='broken'",
	} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runtime_events SET "+assignment+" WHERE event_id=?1", event); err == nil {
			t.Fatalf("%s is refused", assignment)
		}
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runtime_events SET notification_state='inbox',archive_reason=NULL,message_id=?1 WHERE event_id=?2", message, event); err != nil {
		t.Fatal(err)
	}
	for _, assignment := range []string{"message_id=NULL", "archive_reason='member_inactive'"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runtime_events SET "+assignment+" WHERE event_id=?1", event); err == nil {
			t.Fatalf("%s is refused", assignment)
		}
	}
}

func TestDb_RuntimeTests_ArchiveReasonsAcceptTheSpecifiedDomain(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	event := mustID(t)(fixture.archivedEvent(run, 1))
	for _, reason := range []string{"member_inactive", "fleet_deleted", "director_inactive", "head_not_ready", "duplicate_delivery"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_runtime_events SET archive_reason=?1 WHERE event_id=?2", reason, event); err != nil {
			t.Fatalf("%s: %v", reason, err)
		}
	}
}

func TestDb_RuntimeTests_DurableInputKeysAreRunScopedAndSettlementsRequireADisposition(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	input := mustID(t)(fixture.input(run, "prompt:one"))
	if _, err := fixture.input(run, "prompt:one"); err == nil {
		t.Fatal("a repeated operation key is refused")
	}
	old := mustID(t)(fixture.run("exited"))
	mustID(t)(fixture.input(old, "prompt:one"))
	for _, assignment := range []string{"state='settled'", "kind='preview'", "kind='approval'", "state='running'", "disposition='approved'"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_input_operations SET "+assignment+" WHERE input_id=?1", input); err == nil {
			t.Fatalf("%s is refused", assignment)
		}
	}
	for _, disposition := range []string{"submitted", "not_submitted", "abandoned_uncertain"} {
		if _, err := sqlExecute(fixture.conn, "UPDATE member_input_operations SET state='settled',disposition=?1 WHERE input_id=?2", disposition, input); err != nil {
			t.Fatalf("%s: %v", disposition, err)
		}
	}
}

func TestDb_RuntimeTests_RuntimeEventSchemaAcceptsAndRetainsAnInputDeliveryStalledKind(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	event := mustID(t)(fixture.archivedEvent(mustID(t)(fixture.run("running")), 1))
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runtime_events SET kind='input_delivery_stalled' WHERE event_id=?1", event); err != nil {
		t.Fatal(err)
	}
	if got := queryText(t, fixture.conn, "SELECT kind FROM member_runtime_events WHERE event_id=?1", event); got != "input_delivery_stalled" {
		t.Fatalf("kind = %q", got)
	}
}

func TestDb_RuntimeTests_InputStopCausesAcceptLiteralMismatchOnAnUncertainInput(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	input := mustID(t)(fixture.input(mustID(t)(fixture.run("running")), "prompt:mismatch"))
	if _, err := sqlExecute(fixture.conn, "UPDATE member_input_operations SET state='uncertain',stop_cause='literal_mismatch' WHERE input_id=?1", input); err != nil {
		t.Fatal(err)
	}
	if got := queryText(t, fixture.conn, "SELECT stop_cause FROM member_input_operations WHERE input_id=?1", input); got != "literal_mismatch" {
		t.Fatalf("stop_cause = %q", got)
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_input_operations SET stop_cause='literal_garbled' WHERE input_id=?1", input); err == nil {
		t.Fatal("an unknown stop cause is refused")
	}
}

func TestDb_RuntimeTests_RunnerDiagnosticsAcceptTheDirectorDeliverySource(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	run := mustID(t)(fixture.run("running"))
	insert := func(source string) error {
		_, err := sqlExecute(fixture.conn, `INSERT INTO member_runner_diagnostics(run_id,source,message,first_at,last_at,count)
			VALUES(?1,?2,'director delivery: message 7 held: composer_busy',?3,?3,1)`, run, source, runtimeFixtureNow)
		return err
	}
	if err := insert("director_delivery"); err != nil {
		t.Fatal(err)
	}
	if err := insert("director_receiver"); err == nil {
		t.Fatal("an unknown diagnostic source is refused")
	}
}

func TestDb_RuntimeTests_RuntimeForeignKeysRetainHistoryAndRejectOrphans(t *testing.T) {
	t.Parallel()
	fixture := newRuntimeFixture(t)
	if _, err := fixture.interaction(404, 1, "question"); err == nil {
		t.Fatal("an orphan interaction is refused")
	}
	if _, err := fixture.archivedEvent(404, 1); err == nil {
		t.Fatal("an orphan event is refused")
	}
	if _, err := fixture.input(404, "orphan"); err == nil {
		t.Fatal("an orphan input is refused")
	}
	run := mustID(t)(fixture.run("exited"))
	mustID(t)(fixture.interaction(run, 1, "question"))
	mustID(t)(fixture.archivedEvent(run, 1))
	mustID(t)(fixture.input(run, "retained"))
	if _, err := sqlExecute(fixture.conn, "DELETE FROM member_runs WHERE run_id=?1", run); err == nil {
		t.Fatal("a run with history is retained")
	}
	if _, err := sqlExecute(fixture.conn, "DELETE FROM members WHERE member_id=?1", fixture.member); err == nil {
		t.Fatal("a member with runs is retained")
	}
	if got := tableCount(t, fixture.conn, "member_runs"); got != 1 {
		t.Fatalf("runs = %d", got)
	}
}

func TestDb_ClassificationPreservesMissingCurrentIdentityAndSqlCauses(t *testing.T) {
	t.Parallel()
	classified := func(conn *dbConn) classification {
		t.Helper()
		got, err := classifySchema(conn)
		if err != nil {
			t.Fatal(err)
		}
		return got
	}
	conn := memoryConn(t)
	if got := classified(conn); got != classificationMissing {
		t.Fatalf("fresh database: %v", got)
	}
	if err := initialize(conn, "sqlite:///:memory:"); err != nil {
		t.Fatal(err)
	}
	if got := classified(conn); got != classificationCurrent {
		t.Fatalf("initialized database: %v", got)
	}
	mustExecBatch(t, conn, "PRAGMA application_id=1")
	if got := classified(conn); got != classificationIncompatible {
		t.Fatalf("foreign identity: %v", got)
	}
	identified := memoryConn(t)
	mustExecBatch(t, identified, fmt.Sprintf("PRAGMA application_id=%d", applicationID))
	if got := classified(identified); got != classificationIncompatible {
		t.Fatalf("CAFleet identity without a cafleet_database table: %v", got)
	}
	path := filepath.Join(t.TempDir(), "not-sqlite.db")
	if err := os.WriteFile(path, []byte(strings.Repeat("not a sqlite database file\n", 64)), 0o600); err != nil {
		t.Fatal(err)
	}
	garbage := fileConn(t, databaseURLFor(path))
	_, failure := classifySchema(garbage)
	if failure == nil || !strings.Contains(failure.Error(), "file is not a database") {
		t.Fatalf("the SQL cause is preserved: %v", failure)
	}
	state := classifySelectedSchema(garbage, databaseURLFor(path))
	if state.kind != schemaUnreachable || state.cause.Error() != failure.Error() {
		t.Fatalf("classifySelectedSchema = %#v", state)
	}
	if err := state.requireCurrent(); err == nil || err.Error() != failure.Error() {
		t.Fatalf("requireCurrent = %v", err)
	}
	if err := classifySelectedSchema(memoryConn(t), "sqlite:///:memory:").requireCurrent(); err == nil || err.Error() != "no cafleet database; run 'cafleet setup'" {
		t.Fatalf("missing schema: requireCurrent = %v", err)
	}
}

func TestDb_SqliteErrorKeepsSqlitesOwnMessage(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "locked.db")
	writer := fileConn(t, databaseURLFor(path))
	mustExecBatch(t, writer, "CREATE TABLE probe(value INTEGER); BEGIN EXCLUSIVE")
	reader := fileConn(t, databaseURLFor(path))
	mustExecBatch(t, reader, "PRAGMA busy_timeout=0")
	_, raw := sqlRows(reader, "SELECT * FROM probe")
	mapped := classifiedFailure(raw)
	if kind, _ := cafleetKind(mapped); kind != errorDatabaseBusy || mapped.Error() != "database error: database is locked" {
		t.Fatalf("classified(%v) = %v %q", raw, kind, mapped)
	}
	if classifiedFailure(nil) != nil {
		t.Fatal("classifiedFailure(nil) is nil")
	}
}

func TestDb_DatabaseScopeTests_ExpiredCompletionCommitsUncontendedWork(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn := memoryConn(t)
	mustExecBatch(t, conn, "CREATE TABLE probe(value INTEGER);")
	expired := deadlineUntil(time.Now().Add(-time.Millisecond))
	_, err := withDatabaseBudget(ctx, conn, expired, noValue(func(context.Context, *dbConn) error { return nil }))
	if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded {
		t.Fatalf("an expired budget: %v", err)
	}
	_, err = completeDatabaseBudget(ctx, conn, expired, noValue(func(ctx context.Context, conn *dbConn) error {
		return classifiedFailure(conn.raw.Exec("BEGIN IMMEDIATE; INSERT INTO probe VALUES(42); COMMIT;"))
	}))
	if err != nil {
		t.Fatal(err)
	}
	if got := queryInt64(t, conn, "SELECT value FROM probe"); got != 42 {
		t.Fatalf("value = %d", got)
	}
}

func TestDb_DatabaseScopeTests_CompletionWaitsForAReaderToReleaseCommitAfterInputExpiry(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	url := databaseURLFor(filepath.Join(t.TempDir(), "reader.db"))
	conn := fileConn(t, url)
	mustExecBatch(t, conn, "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1)")
	reader := fileConn(t, url)
	mustExecBatch(t, reader, "BEGIN; SELECT * FROM probe")
	release := make(chan struct{})
	committed := make(chan error, 1)
	go func() {
		<-release
		time.Sleep(100 * time.Millisecond)
		committed <- reader.raw.Exec("COMMIT")
	}()
	at := deadlineUntil(time.Now().Add(20 * time.Millisecond))
	_, err := withDatabaseBudget(ctx, conn, at, noValue(func(ctx context.Context, conn *dbConn) error {
		time.Sleep(30 * time.Millisecond)
		_, err := completeDatabaseBudget(ctx, conn, at, noValue(func(ctx context.Context, conn *dbConn) error {
			if err := conn.raw.Exec("BEGIN IMMEDIATE; UPDATE probe SET value=42"); err != nil {
				return classifiedFailure(err)
			}
			close(release)
			return classifiedFailure(conn.raw.Exec("COMMIT"))
		}))
		if err != nil {
			return err
		}
		if _, err := withDatabaseBudget(ctx, conn, at, noValue(func(context.Context, *dbConn) error { return nil })); err == nil {
			t.Error("the expired input budget refuses further work")
		}
		return nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	if err := <-committed; err != nil {
		t.Fatal(err)
	}
	if got := queryInt64(t, conn, "SELECT value FROM probe"); got != 42 {
		t.Fatalf("value = %d", got)
	}
}

func TestDb_DatabaseScopeTests_ExpiredCompletionReportsContentionAfterBoundedGrace(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	url := databaseURLFor(filepath.Join(t.TempDir(), "completion.db"))
	writer := fileConn(t, url)
	mustExecBatch(t, writer, "CREATE TABLE probe(value INTEGER); BEGIN IMMEDIATE;")
	conn := fileConn(t, url)
	started := time.Now()
	expired := deadlineUntil(started.Add(-time.Millisecond))
	_, err := completeDatabaseBudget(ctx, conn, expired, noValue(func(ctx context.Context, conn *dbConn) error {
		_, err := sqlExecute(conn, "INSERT INTO probe VALUES(42)")
		return classifiedFailure(err)
	}))
	elapsed := time.Since(started)
	if err == nil || !strings.Contains(err.Error(), "database is locked") {
		t.Fatalf("completion = %v, want contention", err)
	}
	if completionGrace != time.Second {
		t.Fatalf("completionGrace = %v", completionGrace)
	}
	if elapsed < completionGrace*9/10 || elapsed >= completionGrace*2 {
		t.Fatalf("contention reported after %v", elapsed)
	}
}

func assertBusyTimeout(t *testing.T, conn *dbConn, want int64) {
	t.Helper()
	if got := queryInt64(t, conn, "PRAGMA busy_timeout"); got != want {
		t.Fatalf("busy_timeout = %d, want %d", got, want)
	}
}

func TestDb_DatabaseScopeTests_ScopeExitRestoresTimeoutsAndAllowsSequentialConnectionsAndThreads(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	first := memoryConn(t)
	second := memoryConn(t)
	succeed := noValue(func(context.Context, *dbConn) error { return nil })
	for index, conn := range []*dbConn{first, second} {
		mustExecBatch(t, conn, "PRAGMA busy_timeout=137")
		if _, err := withDatabaseBudget(ctx, conn, newDeadline(), succeed); err != nil {
			t.Fatal(err)
		}
		assertBusyTimeout(t, conn, 137)
		failing := noValue(func(context.Context, *dbConn) error { return appError("expected") })
		if _, err := withDatabaseBudget(ctx, conn, newDeadline(), failing); err == nil || err.Error() != "expected" {
			t.Fatalf("connection %d: %v", index, err)
		}
		assertBusyTimeout(t, conn, 137)
		func() {
			defer func() {
				if recover() == nil {
					t.Fatalf("connection %d: the scope panic propagates", index)
				}
			}()
			_, _ = withDatabaseBudget(ctx, conn, newDeadline(), noValue(func(context.Context, *dbConn) error {
				panic("scope unwind")
			}))
		}()
		assertBusyTimeout(t, conn, 137)
	}
	_, err := withDatabaseBudget(ctx, first, newDeadline(), noValue(func(context.Context, *dbConn) error {
		done := make(chan error, 1)
		go func() {
			conn, err := connect("sqlite:///:memory:")
			if err != nil {
				done <- err
				return
			}
			_, err = withDatabaseBudget(context.Background(), conn, newDeadline(), succeed)
			done <- errors.Join(err, conn.close())
		}()
		return <-done
	}))
	if err != nil {
		t.Fatal(err)
	}
}

func TestDb_DatabaseScopeTests_CrossConnectionNestingFailsBeforeChangingTheInnerTimeout(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	outer := memoryConn(t)
	inner := memoryConn(t)
	mustExecBatch(t, inner, "PRAGMA busy_timeout=137")
	_, err := withDatabaseBudget(ctx, outer, newDeadline(), noValue(func(ctx context.Context, _ *dbConn) error {
		func() {
			defer func() {
				if recover() == nil {
					t.Error("nesting a scope on another connection panics")
				}
			}()
			_, _ = withDatabaseBudget(ctx, inner, newDeadline(), noValue(func(context.Context, *dbConn) error { return nil }))
		}()
		assertBusyTimeout(t, inner, 137)
		return nil
	}))
	if err != nil {
		t.Fatal(err)
	}
}

func TestDb_DatabaseScopeTests_NestedExitRestoresTheEarliestOuterDeadlineOnEveryExit(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	cases := []struct {
		name  string
		panic bool
	}{
		{name: "nested scope returns"},
		{name: "nested scope panics", panic: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			url := databaseURLFor(filepath.Join(t.TempDir(), "nested.db"))
			writer := fileConn(t, url)
			mustExecBatch(t, writer, "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE")
			conn := fileConn(t, url)
			mustExecBatch(t, conn, "PRAGMA busy_timeout=137")
			started := time.Now()
			outer := deadlineUntil(started.Add(160 * time.Millisecond))
			_, err := withDatabaseBudget(ctx, conn, outer, func(ctx context.Context, conn *dbConn) (int64, error) {
				if tc.panic {
					func() {
						defer func() {
							if recover() == nil {
								t.Errorf("%s: the nested panic propagates", tc.name)
							}
						}()
						_, _ = withDatabaseBudget(ctx, conn, newDeadline(), noValue(func(context.Context, *dbConn) error {
							panic("nested unwind")
						}))
					}()
				} else if _, err := withDatabaseBudget(ctx, conn, newDeadline(), noValue(func(context.Context, *dbConn) error { return nil })); err != nil {
					return 0, err
				}
				return withDatabaseBudget(ctx, conn, newDeadline(), func(ctx context.Context, conn *dbConn) (int64, error) {
					rows, err := sqlRows(conn, "SELECT value FROM probe")
					if err != nil {
						return 0, classifiedFailure(err)
					}
					return rows[0][0].(int64), nil
				})
			})
			elapsed := time.Since(started)
			if kind, _ := cafleetKind(err); kind != errorDatabaseBusy {
				t.Fatalf("%s: %v, want database busy", tc.name, err)
			}
			if elapsed < 140*time.Millisecond || elapsed >= 700*time.Millisecond {
				t.Fatalf("%s: busy reported after %v", tc.name, elapsed)
			}
			assertBusyTimeout(t, conn, 137)
		})
	}
}

func TestDb_DatabaseScopeTests_ConsecutiveSqlWaitsShareOneAbsoluteDeadline(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	url := databaseURLFor(filepath.Join(t.TempDir(), "aggregate.db"))
	writer := fileConn(t, url)
	mustExecBatch(t, writer, "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE")
	released := make(chan error, 1)
	go func() {
		time.Sleep(80 * time.Millisecond)
		released <- writer.raw.Exec("ROLLBACK")
	}()
	conn := fileConn(t, url)
	started := time.Now()
	at := deadlineUntil(started.Add(200 * time.Millisecond))
	read := func(conn *dbConn) (int64, error) {
		rows, err := sqlRows(conn, "SELECT value FROM probe")
		if err != nil {
			return 0, classifiedFailure(err)
		}
		return rows[0][0].(int64), nil
	}
	_, err := withDatabaseBudget(ctx, conn, at, func(ctx context.Context, conn *dbConn) (int64, error) {
		if value, err := read(conn); err != nil || value != 42 {
			return 0, fmt.Errorf("the first read after the release: %d %w", value, err)
		}
		if err := <-released; err != nil {
			return 0, err
		}
		blocker := fileConn(t, url)
		if err := blocker.raw.Exec("BEGIN EXCLUSIVE"); err != nil {
			return 0, err
		}
		return read(conn)
	})
	elapsed := time.Since(started)
	if kind, _ := cafleetKind(err); kind != errorDatabaseBusy {
		t.Fatalf("%v, want database busy", err)
	}
	if elapsed < 180*time.Millisecond || elapsed >= 700*time.Millisecond {
		t.Fatalf("busy reported after %v", elapsed)
	}
}

func TestDb_DatabaseScopeTests_SharedDeadlineLoweredMidScopeAbortsTheRunningWait(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	const slowSum = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter"
	const lowerAfter = 100 * time.Millisecond
	cases := []struct {
		name string
		busy bool
	}{
		{name: "long statement"},
		{name: "busy wait", busy: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			url := databaseURLFor(filepath.Join(t.TempDir(), "shared.db"))
			conn := fileConn(t, url)
			mustExecBatch(t, conn, "CREATE TABLE probe(value INTEGER)")
			holder := fileConn(t, url)
			if tc.busy {
				mustExecBatch(t, holder, "BEGIN IMMEDIATE")
			}
			shared := &sharedDeadline{}
			lowered := make(chan struct{})
			go func() {
				time.Sleep(lowerAfter)
				shared.lower(time.Now())
				close(lowered)
			}()
			started := time.Now()
			_, err := withSharedDatabaseDeadline(ctx, conn, started.Add(30*time.Second), shared, noValue(func(ctx context.Context, conn *dbConn) error {
				if tc.busy {
					_, err := sqlExecute(conn, "INSERT INTO probe VALUES(1)")
					return classifiedFailure(err)
				}
				_, err := sqlRows(conn, slowSum)
				return classifiedFailure(err)
			}))
			elapsed := time.Since(started)
			<-lowered
			if err == nil {
				t.Fatalf("%s: the lowered shared deadline aborts the wait", tc.name)
			}
			var failure *cafleetError
			if !errors.As(err, &failure) || !failure.transientStorage() {
				t.Fatalf("%s: a wait cut off by the shared deadline is transient: %v", tc.name, err)
			}
			if elapsed < lowerAfter-10*time.Millisecond {
				t.Fatalf("%s: the wait ends before the shared deadline is lowered: %v", tc.name, elapsed)
			}
			if elapsed >= 2*time.Second {
				t.Fatalf("%s: the abort follows the 30 s frame, not the lowered shared deadline: %v", tc.name, elapsed)
			}
			if tc.busy {
				mustExecBatch(t, holder, "ROLLBACK")
			}
			if _, err := sqlExecute(conn, "INSERT INTO probe VALUES(2)"); err != nil {
				t.Fatalf("%s: the connection is reusable: %v", tc.name, err)
			}
		})
	}
}

func TestDb_DatabaseScopeTests_ALateErrorOtherThanAnInterruptKeepsItsCause(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	cases := []struct {
		name   string
		shared bool
		cause  func() *cafleetError
	}{
		{name: "permanent", cause: func() *cafleetError { return appError("permanent cause") }},
		{name: "busy", cause: func() *cafleetError { return databaseBusyError("busy cause") }},
		{name: "permanent, shared", shared: true, cause: func() *cafleetError { return appError("permanent cause") }},
		{name: "busy, shared", shared: true, cause: func() *cafleetError { return databaseBusyError("busy cause") }},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			conn := memoryConn(t)
			until := time.Now().Add(50 * time.Millisecond)
			late := noValue(func(context.Context, *dbConn) error {
				time.Sleep(100 * time.Millisecond)
				return tc.cause()
			})
			var err error
			if tc.shared {
				_, err = withSharedDatabaseDeadline(ctx, conn, until, &sharedDeadline{}, late)
			} else {
				_, err = withDatabaseDeadline(ctx, conn, until, late)
			}
			if err == nil || err.Error() != tc.cause().Error() {
				t.Fatalf("%s: %v, want the operation's error %q", tc.name, err, tc.cause())
			}
		})
	}
}

func TestDb_DatabaseScopeTests_ALateValueOrInterruptFailsWithDeadlineExceeded(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	cases := []struct {
		name   string
		result error
	}{
		{name: "late value"},
		{name: "late interrupt", result: databaseInterruptedError("database error: interrupted")},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			conn := memoryConn(t)
			_, err := withDatabaseDeadline(ctx, conn, time.Now().Add(50*time.Millisecond), noValue(func(context.Context, *dbConn) error {
				time.Sleep(100 * time.Millisecond)
				return tc.result
			}))
			if kind, _ := cafleetKind(err); kind != errorDeadlineExceeded {
				t.Fatalf("%s: %v, want deadline exceeded", tc.name, err)
			}
		})
	}
}

func TestDb_DatabaseScopeTests_SharedDeadlineLeftUnsetBoundsTheScopeByItsOwnDeadlineOnly(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn := memoryConn(t)
	value, err := withSharedDatabaseDeadline(ctx, conn, time.Now().Add(5*time.Second), &sharedDeadline{}, func(ctx context.Context, conn *dbConn) (int64, error) {
		rows, err := sqlRows(conn, "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter")
		if err != nil {
			return 0, classifiedFailure(err)
		}
		return rows[0][0].(int64), nil
	})
	if err != nil || value != 50005000 {
		t.Fatalf("sum = %d %v", value, err)
	}
}

func TestDb_DatabaseScopeTests_TheInheritedDeadlineIsTheTightestFromTheInnermostCompletionScope(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn := memoryConn(t)
	if _, ok := inheritedDeadline(ctx).get(); ok {
		t.Fatal("no scope, no inherited deadline")
	}
	inherited := func(ctx context.Context) time.Time {
		at, ok := inheritedDeadline(ctx).get()
		if !ok {
			t.Fatal("a scope has an inherited deadline")
		}
		return at.instant()
	}
	origin := time.Now()
	outer := deadlineUntil(origin.Add(2 * time.Second))
	tighter := deadlineUntil(origin.Add(time.Second))
	looser := deadlineUntil(origin.Add(3 * time.Second))
	_, err := withDatabaseBudget(ctx, conn, outer, noValue(func(ctx context.Context, conn *dbConn) error {
		if got := inherited(ctx); !got.Equal(outer.instant()) {
			t.Errorf("outer scope: inherited %v", got)
		}
		if got := deadlineInheritedOrNew(ctx).instant(); !got.Equal(outer.instant()) {
			t.Errorf("outer scope: deadlineInheritedOrNew = %v", got)
		}
		if got := boundedByInherited(ctx, origin.Add(5*time.Second)); !got.Equal(outer.instant()) {
			t.Errorf("boundedByInherited lowers a later instant to %v", got)
		}
		if got := boundedByInherited(ctx, origin.Add(time.Millisecond)); !got.Equal(origin.Add(time.Millisecond)) {
			t.Errorf("boundedByInherited keeps an earlier instant, got %v", got)
		}
		_, err := withDatabaseBudget(ctx, conn, looser, noValue(func(ctx context.Context, conn *dbConn) error {
			if got := inherited(ctx); !got.Equal(outer.instant()) {
				t.Errorf("a looser nested budget inherits the outer deadline, got %v", got)
			}
			_, err := withDatabaseBudget(ctx, conn, tighter, noValue(func(ctx context.Context, conn *dbConn) error {
				if got := inherited(ctx); !got.Equal(tighter.instant()) {
					t.Errorf("a tighter nested budget is inherited, got %v", got)
				}
				return nil
			}))
			return err
		}))
		if err != nil {
			return err
		}
		_, err = completeDatabaseBudget(ctx, conn, looser, noValue(func(ctx context.Context, conn *dbConn) error {
			if got := inherited(ctx); !got.Equal(looser.instant()) {
				t.Errorf("a completion scope is bounded by its own deadline only, got %v", got)
			}
			return nil
		}))
		return err
	}))
	if err != nil {
		t.Fatal(err)
	}
	before := time.Now()
	fallback := deadlineInheritedOrNew(ctx).instant()
	if fallback.Before(before.Add(5*time.Second)) || fallback.After(time.Now().Add(5*time.Second)) {
		t.Fatalf("deadlineInheritedOrNew outside a scope = %v, want five seconds away", fallback)
	}
}

func TestDb_TestSupport_ReadBudgetsRecordTheInheritedBudgetOfEachRead(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	conn, _ := initializedFileConn(t)
	recorded := recordReadBudgets(t, conn, "fleets")
	at := deadlineUntil(time.Now().Add(3 * time.Second))
	_, err := withDatabaseBudget(ctx, conn, at, noValue(func(ctx context.Context, conn *dbConn) error {
		_, err := sqlRows(conn, "SELECT * FROM fleets")
		return classifiedFailure(err)
	}))
	if err != nil {
		t.Fatal(err)
	}
	budgets := recorded.take()
	if len(budgets) == 0 || !slices.ContainsFunc(budgets, func(budget time.Time) bool { return budget.Equal(at.instant()) }) {
		t.Fatalf("recorded budgets %v, want the scope deadline %v", budgets, at.instant())
	}
	if again := recorded.take(); len(again) != 0 {
		t.Fatalf("take drains the record, left %v", again)
	}
}

func TestDb_TestSupport_FaultPlansFailMatchingStatementsUntilCleared(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		plan  faultPlan
		cause string
	}{
		{name: "abort trigger", plan: faultPlanAbort("refuse_fleet", "BEFORE INSERT ON fleets", "fleet storage refused"), cause: "fleet storage refused"},
		{name: "refused commit", plan: faultPlanRefuseCommit()},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			conn, url := initializedFileConn(t)
			other := fileConn(t, url)
			insert := func(conn *dbConn) error {
				return conn.raw.Exec("BEGIN; INSERT INTO fleets(name, created_at) VALUES('f', 'c'); COMMIT;")
			}
			tc.plan.arm(t, conn)
			err := insert(conn)
			if err == nil || !strings.Contains(classifiedFailure(err).Error(), tc.cause) {
				t.Fatalf("%s: armed insert = %v", tc.name, err)
			}
			if !conn.raw.GetAutocommit() {
				mustExecBatch(t, conn, "ROLLBACK")
			}
			if tc.cause != "" {
				if err := insert(other); err == nil {
					t.Fatalf("%s: the trigger applies to every connection", tc.name)
				}
				if !other.raw.GetAutocommit() {
					mustExecBatch(t, other, "ROLLBACK")
				}
			}
			if got := tableCount(t, conn, "fleets"); got != 0 {
				t.Fatalf("%s: fleets = %d", tc.name, got)
			}
			tc.plan.clear(t, conn)
			if err := insert(conn); err != nil {
				t.Fatalf("%s: cleared insert = %v", tc.name, err)
			}
		})
	}
}

func TestDb_TestSupport_WalSnapshotReadsTheDatabaseAndItsSidecars(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "wal.db")
	conn := fileConn(t, databaseURLFor(path))
	mustExecBatch(t, conn, "PRAGMA journal_mode=WAL; CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1);")
	snapshot := walSnapshot(t, path)
	for index, suffix := range []string{"", "-wal", "-shm"} {
		if snapshot[index].path != path+suffix {
			t.Fatalf("snapshot %d names %s", index, snapshot[index].path)
		}
	}
	if len(snapshot[1].bytes) == 0 {
		t.Fatal("the WAL holds the uncheckpointed write")
	}
}

func TestCreateDirAll(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	file := filepath.Join(root, "file")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name  string
		path  string
		errno syscall.Errno
	}{
		{name: "missing ancestors", path: filepath.Join(root, "a", "b", "c")},
		{name: "existing directory", path: root},
		{name: "trailing separator", path: filepath.Join(root, "d", "e") + "/"},
		{name: "regular file in the way", path: file, errno: syscall.EEXIST},
		{name: "regular file as an ancestor", path: filepath.Join(file, "x"), errno: syscall.ENOTDIR},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Parallel()
			err := createDirAll(c.path)
			if c.errno != 0 {
				if !errors.Is(err, c.errno) {
					t.Fatalf("%s: want %v, got %v", c.name, c.errno, err)
				}
				return
			}
			if err != nil {
				t.Fatalf("%s: %v", c.name, err)
			}
			if !isDirectory(c.path) {
				t.Fatalf("%s: %s is not a directory", c.name, c.path)
			}
		})
	}
}
