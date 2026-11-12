use crate::error::CafleetError;
use crate::time::Deadline;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use std::time::Duration;
use std::time::Instant;

const BASELINE_SQL: &str = include_str!("../assets/migrations/V1__baseline.sql");

fn baseline() -> refinery::Migration {
    let sql = format!(
        "{BASELINE_SQL}\nINSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, '{}');\n",
        crate::cli::VERSION
    );
    refinery::Migration::unapplied("V1__baseline", &sql)
        .expect("the embedded baseline has a valid migration name")
}

/// The file path a `sqlite:///<path>` database URL names.
pub(crate) fn database_path(database_url: &str) -> Result<&str, CafleetError> {
    database_url.strip_prefix("sqlite:///").ok_or_else(|| {
        CafleetError::App(format!(
            "database URL must use the sqlite scheme (sqlite:///<path>); got '{database_url}'"
        ))
    })
}

pub(crate) fn connect(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::default())
}

pub(crate) fn connect_read_only(database_url: &str) -> Result<Connection, CafleetError> {
    connect_with_flags(database_url, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
}

pub(crate) fn open_existing(
    database_url: &str,
    read_only: bool,
) -> Result<Option<Connection>, CafleetError> {
    let path = database_path(database_url)?;
    if !std::path::Path::new(path).try_exists().map_err(|cause| {
        CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
    })? {
        return Ok(None);
    }
    let conn = if read_only {
        connect_read_only(database_url)?
    } else {
        connect_with_flags(
            database_url,
            rusqlite::OpenFlags::default() & !rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )?
    };
    Ok(Some(conn))
}

fn connect_with_flags(
    database_url: &str,
    flags: rusqlite::OpenFlags,
) -> Result<Connection, CafleetError> {
    let path = database_path(database_url)?;
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| CafleetError::App(format!("failed to open database at '{path}': {e}")))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .map_err(|e| CafleetError::App(format!("failed to apply connection PRAGMAs: {e}")))?;
    Ok(conn)
}

pub(crate) fn initialize(conn: &mut Connection) -> Result<(), CafleetError> {
    refinery::Runner::new(&[baseline()])
        .set_grouped(true)
        .run(conn)
        .map_err(|e| CafleetError::App(format!("database initialization failed: {e}")))?;
    Ok(())
}

pub(crate) const APPLICATION_ID: i64 = 1128351289;

fn metadata_rows(
    conn: &Connection,
    sql: &str,
    name: &str,
) -> Result<Vec<Vec<rusqlite::types::Value>>, CafleetError> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map([name], |row| {
        (0..row.as_ref().column_count())
            .map(|column| row.get(column))
            .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(crate::error::CafleetError::from)
}

const COLUMNS: &str =
    "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid";
const FOREIGN_KEYS: &str = "SELECT \"table\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?1) ORDER BY \"from\", \"to\"";
const INDEX_COLUMNS: &str = "SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno";
type Metadata = Vec<Vec<rusqlite::types::Value>>;

struct SchemaIndex {
    name: String,
    unique: i64,
    partial: i64,
    columns: Metadata,
}

struct SchemaTable {
    name: String,
    columns: Metadata,
    foreign_keys: Metadata,
    indexes: Vec<SchemaIndex>,
}

impl SchemaTable {
    fn matches(&self, conn: &Connection) -> Result<bool, CafleetError> {
        if metadata_rows(conn, COLUMNS, &self.name)? != self.columns
            || metadata_rows(conn, FOREIGN_KEYS, &self.name)? != self.foreign_keys
        {
            return Ok(false);
        }
        for index in &self.indexes {
            let definition: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT \"unique\", partial FROM pragma_index_list(?1) WHERE name=?2",
                    rusqlite::params![self.name, index.name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if definition != Some((index.unique, index.partial))
                || metadata_rows(conn, INDEX_COLUMNS, &index.name)? != index.columns
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn all_match(tables: &[SchemaTable], conn: &Connection) -> Result<bool, CafleetError> {
    for table in tables {
        if !table.matches(conn)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_schema(conn: &Connection) -> Result<Vec<SchemaTable>, CafleetError> {
    let mut tables = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )?;
    let names = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut schema = Vec::new();
    for name in names {
        let mut indexes =
            conn.prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1)")?;
        let indexes = indexes
            .query_map([&name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let indexes = indexes
            .into_iter()
            .map(|(name, unique, partial)| {
                Ok(SchemaIndex {
                    columns: metadata_rows(conn, INDEX_COLUMNS, &name)?,
                    name,
                    unique,
                    partial,
                })
            })
            .collect::<Result<Vec<_>, CafleetError>>()?;
        schema.push(SchemaTable {
            columns: metadata_rows(conn, COLUMNS, &name)?,
            foreign_keys: metadata_rows(conn, FOREIGN_KEYS, &name)?,
            name,
            indexes,
        });
    }
    Ok(schema)
}

fn reference_schema() -> Result<Vec<SchemaTable>, CafleetError> {
    let mut expected = Connection::open_in_memory()?;
    initialize(&mut expected)?;
    read_schema(&expected)
}

pub(crate) fn schema_matches(conn: &Connection) -> Result<bool, CafleetError> {
    let reference = reference_schema()?;
    let snapshot = if conn.is_autocommit() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let matches = all_match(&reference, conn)?;
    if let Some(snapshot) = snapshot {
        snapshot.commit()?;
    }
    Ok(matches)
}

#[derive(Debug)]
pub(crate) enum SchemaState {
    Missing,
    Current,
    Incompatible { path: String },
    Unreachable { cause: CafleetError },
}

impl SchemaState {
    pub(crate) fn require_current(&self) -> Result<(), CafleetError> {
        match self {
            SchemaState::Current => Ok(()),
            SchemaState::Incompatible { path } => Err(crate::db::incompatible_format(path)),
            SchemaState::Missing => Err(CafleetError::App(
                "no cafleet database; run 'cafleet setup'".into(),
            )),
            SchemaState::Unreachable { cause } => Err(cause.clone()),
        }
    }
}

fn ledger_is_baseline(conn: &Connection) -> Result<bool, CafleetError> {
    let ledger_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='refinery_schema_history')",
        [], |row| row.get(0),
    )?;
    if !ledger_exists {
        return Ok(false);
    }
    let mut statement =
        conn.prepare("SELECT version, name FROM refinery_schema_history ORDER BY version")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows == [(1, "baseline".to_owned())])
}

fn records_running_version(conn: &Connection) -> Result<bool, CafleetError> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT cafleet_version FROM cafleet_database WHERE singleton_id=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(recorded.as_deref() == Some(crate::cli::VERSION))
}

pub(crate) fn incompatible_format(path: &str) -> CafleetError {
    CafleetError::App(format!(
        "database format is incompatible at '{path}'; choose an empty database path and run 'cafleet setup'."
    ))
}

#[derive(Debug, PartialEq, Eq)]
enum Classification {
    Missing,
    Current,
    Incompatible,
}

fn classify_schema(conn: &Connection) -> Result<Classification, CafleetError> {
    let identity: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let populated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
        [],
        |row| row.get(0),
    )?;
    if identity == 0 && !populated {
        return Ok(Classification::Missing);
    }
    if identity != crate::db::APPLICATION_ID
        || !ledger_is_baseline(conn)?
        || !schema_matches(conn)?
        || !records_running_version(conn)?
    {
        return Ok(Classification::Incompatible);
    }
    Ok(Classification::Current)
}

pub(crate) fn classify_selected_schema(conn: &Connection, database_url: &str) -> SchemaState {
    match classify_schema(conn) {
        Ok(Classification::Missing) => SchemaState::Missing,
        Ok(Classification::Current) => SchemaState::Current,
        Ok(Classification::Incompatible) => match crate::db::database_path(database_url) {
            Ok(path) => SchemaState::Incompatible { path: path.into() },
            Err(cause) => SchemaState::Unreachable { cause },
        },
        Err(cause) => SchemaState::Unreachable { cause },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Wait,
    Completion,
    Cancellable,
}

struct DatabaseWait {
    deadline: Deadline,
    completion: bool,
    /// Statements abort through the progress handler once this passes.
    cancellation: Option<Instant>,
}

#[derive(Default)]
struct DatabaseWaits {
    identity: Option<usize>,
    frames: Vec<DatabaseWait>,
}

impl DatabaseWaits {
    /// The tightest deadline from the innermost completion scope inward, so mandatory finalization
    /// after an irreversible effect is not bounded by an enclosing, already expired budget.
    fn innermost_deadline(&self) -> Option<Deadline> {
        let start = self
            .frames
            .iter()
            .rposition(|frame| frame.completion)
            .unwrap_or(0);
        self.frames[start..]
            .iter()
            .map(|frame| frame.deadline)
            .min_by_key(|deadline| deadline.instant())
    }
}

thread_local! {
    static DATABASE_WAITS: std::cell::RefCell<DatabaseWaits> = const {
        std::cell::RefCell::new(DatabaseWaits { identity: None, frames: Vec::new() })
    };
}

pub(crate) fn inherited_deadline() -> Option<Deadline> {
    DATABASE_WAITS.with(|waits| waits.borrow().innermost_deadline())
}

fn inherited_database_deadline(conn: &Connection) -> Option<Deadline> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        if waits.frames.is_empty() {
            assert!(
                waits.identity.is_none(),
                "an empty database scope has no owner"
            );
            return None;
        }
        assert_eq!(
            waits.identity,
            Some(std::ptr::from_ref(conn).addr()),
            "inherited database scopes use the same connection"
        );
        waits.innermost_deadline()
    })
}

pub(crate) fn bounded_by_inherited(conn: &Connection, until: Instant) -> Instant {
    inherited_database_deadline(conn).map_or(until, |inherited| until.min(inherited.instant()))
}

fn remaining_database_wait() -> Option<Duration> {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        assert!(
            waits.identity.is_some() && !waits.frames.is_empty(),
            "a busy handler has an owning connection scope"
        );
        let start = waits
            .frames
            .iter()
            .rposition(|wait| wait.completion)
            .unwrap_or(0);
        waits.frames[start..]
            .iter()
            .try_fold(Duration::MAX, |remaining, wait| {
                wait.deadline.remaining().map(|left| remaining.min(left))
            })
            .ok()
    })
}

fn database_cancelled() -> bool {
    DATABASE_WAITS.with(|waits| {
        let waits = waits.borrow();
        let until = waits
            .frames
            .iter()
            .filter_map(|frame| frame.cancellation)
            .min();
        Instant::now() >= until.expect("a progress handler has an owning cancellable scope")
    })
}

fn wait_for_database_deadline(_: i32) -> bool {
    let Some(remaining) = remaining_database_wait() else {
        return false;
    };
    std::thread::sleep(remaining.min(Duration::from_millis(10)));
    remaining_database_wait().is_some()
}

struct DatabaseBudget<'a> {
    conn: &'a mut Connection,
    identity: usize,
    previous_timeout: Duration,
    active: bool,
}

impl<'a> DatabaseBudget<'a> {
    fn new(
        conn: &'a mut Connection,
        deadline: Deadline,
        kind: ScopeKind,
    ) -> Result<Self, CafleetError> {
        let identity = std::ptr::from_ref(conn).addr();
        DATABASE_WAITS.with(|waits| {
            if let Some(owner) = waits.borrow().identity {
                assert_eq!(
                    owner, identity,
                    "nested database scopes use the same connection"
                );
            }
        });
        let previous: u32 = conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        conn.busy_handler(Some(wait_for_database_deadline))?;
        DATABASE_WAITS.with(|waits| {
            let mut waits = waits.borrow_mut();
            waits.identity = Some(identity);
            let inherited = waits
                .innermost_deadline()
                .map(|inherited| inherited.instant());
            waits.frames.push(DatabaseWait {
                deadline,
                completion: kind == ScopeKind::Completion,
                cancellation: (kind == ScopeKind::Cancellable).then(|| {
                    inherited.map_or(deadline.instant(), |until| until.min(deadline.instant()))
                }),
            });
        });
        if kind == ScopeKind::Cancellable {
            conn.progress_handler(1000, Some(database_cancelled))?;
        }
        Ok(Self {
            conn,
            identity,
            previous_timeout: Duration::from_millis(u64::from(previous)),
            active: true,
        })
    }

    fn restore(&mut self) -> Result<(), CafleetError> {
        if !self.active {
            return Ok(());
        }
        let (nested, cancellable) = DATABASE_WAITS.with(|waits| {
            let mut waits = waits.borrow_mut();
            assert_eq!(waits.identity, Some(self.identity));
            waits
                .frames
                .pop()
                .expect("an active database budget is registered");
            if waits.frames.is_empty() {
                waits.identity = None;
            }
            let cancellable = waits
                .frames
                .iter()
                .any(|frame| frame.cancellation.is_some());
            (!waits.frames.is_empty(), cancellable)
        });
        self.active = false;
        let restored = if nested {
            self.conn.busy_handler(Some(wait_for_database_deadline))
        } else {
            self.conn.busy_timeout(self.previous_timeout)
        };
        let handler = if cancellable {
            self.conn.progress_handler(1000, Some(database_cancelled))
        } else {
            self.conn.progress_handler(0, None::<fn() -> bool>)
        };
        CafleetError::merge_cleanup(
            restored.map_err(CafleetError::from),
            handler.map_err(CafleetError::from),
        )
    }
}

impl Drop for DatabaseBudget<'_> {
    fn drop(&mut self) {
        self.restore()
            .expect("an open borrowed connection accepts busy-handler restoration");
    }
}

pub(crate) fn with_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    deadline.remaining()?;
    scoped_database_budget(conn, deadline, ScopeKind::Wait, operation)
}

/// Like [`with_database_budget`] until `until`, and statements also abort once it passes.
pub(crate) fn with_database_deadline<T>(
    conn: &mut Connection,
    until: Instant,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    let deadline = Deadline::until(until);
    deadline.remaining()?;
    let value = scoped_database_budget(conn, deadline, ScopeKind::Cancellable, operation)?;
    deadline.remaining()?;
    Ok(value)
}

// Pane side effects are irreversible; mandatory storage finalization gets a
// bounded grace period independent of the input deadline.
const COMPLETION_GRACE: Duration = Duration::from_secs(1);

pub(crate) fn complete_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    scoped_database_budget(
        conn,
        Deadline::until(deadline.instant().max(Instant::now() + COMPLETION_GRACE)),
        ScopeKind::Completion,
        operation,
    )
}

fn scoped_database_budget<T>(
    conn: &mut Connection,
    deadline: Deadline,
    kind: ScopeKind,
    operation: impl FnOnce(&mut Connection) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    let mut budget = DatabaseBudget::new(conn, deadline, kind)?;
    let result = operation(budget.conn);
    let restored = budget.restore().map_err(|error| {
        CafleetError::App(format!(
            "restoring database wait failed: {}",
            error.message()
        ))
    });
    CafleetError::merge_cleanup(result, restored)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;
    use std::path::PathBuf;

    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Instant;

    use rusqlite::Connection;
    use rusqlite::hooks::AuthAction;
    use rusqlite::hooks::AuthContext;
    use rusqlite::hooks::Authorization;
    use rusqlite::types::Value;
    use tempfile::TempDir;

    pub(crate) fn migrated_conn(dir: &TempDir) -> Connection {
        let url = format!("sqlite:///{}", dir.path().join("broker.db").display());
        let mut conn = crate::db::connect(&url).unwrap();
        crate::db::initialize(&mut conn).unwrap();
        conn
    }

    pub(crate) fn table_rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut statement = conn.prepare(sql).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[derive(Debug, PartialEq)]
    pub(crate) struct TableSnapshot {
        pub(crate) name: String,
        pub(crate) rows: Vec<Vec<Value>>,
    }

    pub(crate) fn table_snapshot(
        conn: &Connection,
        include: impl Fn(&str) -> bool,
    ) -> Vec<TableSnapshot> {
        let names = conn
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        names
            .into_iter()
            .filter(|name| include(name))
            .map(|name| {
                let quoted = name.replace('"', "\"\"");
                let rows = table_rows(conn, &format!("SELECT * FROM \"{quoted}\" ORDER BY rowid"));
                TableSnapshot { name, rows }
            })
            .collect()
    }

    /// Records the inherited database budget of every read of `table` on the wrapped
    /// connection, clearing the authorizer on drop.
    pub(crate) struct ReadBudgets<'a> {
        conn: &'a mut Connection,
        budgets: Arc<Mutex<Vec<Instant>>>,
    }

    pub(crate) fn record_read_budgets<'a>(
        conn: &'a mut Connection,
        table: &'static str,
    ) -> ReadBudgets<'a> {
        let budgets = Arc::new(Mutex::new(Vec::new()));
        let recorded = budgets.clone();
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::Read { table_name, .. } if table_name == table)
            {
                recorded
                    .lock()
                    .unwrap()
                    .push(crate::time::Deadline::inherited_or_new().instant());
            }
            Authorization::Allow
        }))
        .unwrap();
        ReadBudgets { conn, budgets }
    }

    impl ReadBudgets<'_> {
        pub(crate) fn take(&self) -> Vec<Instant> {
            std::mem::take(&mut *self.budgets.lock().unwrap())
        }
    }

    impl std::ops::Deref for ReadBudgets<'_> {
        type Target = Connection;
        fn deref(&self) -> &Connection {
            self.conn
        }
    }

    impl std::ops::DerefMut for ReadBudgets<'_> {
        fn deref_mut(&mut self) -> &mut Connection {
            self.conn
        }
    }

    impl Drop for ReadBudgets<'_> {
        fn drop(&mut self) {
            self.conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
        }
    }

    pub(crate) fn wal_snapshot(database: &Path) -> [(PathBuf, Vec<u8>); 3] {
        ["", "-wal", "-shm"].map(|suffix| {
            let path = PathBuf::from(format!("{}{suffix}", database.display()));
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::db::test_support::table_rows as rows;
    use crate::db::*;

    mod existing_file_opener {
        use crate::db::tests::Connection;
        use crate::db::tests::connect;
        use crate::db::tests::connect_read_only;

        fn file_with_probe() -> (tempfile::TempDir, String) {
            let dir = tempfile::tempdir().unwrap();
            let url = format!("sqlite:///{}", dir.path().join("existing.db").display());
            let conn = connect(&url).unwrap();
            conn.execute_batch(
                "CREATE TABLE probe(value INTEGER NOT NULL); INSERT INTO probe VALUES(7);",
            )
            .unwrap();
            drop(conn);
            (dir, url)
        }

        fn assert_connection_pragmas(conn: &Connection) {
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                5000
            );
        }

        #[test]
        fn absent_files_return_none_and_remain_absent_in_both_modes() {
            let dir = tempfile::tempdir().unwrap();
            for read_only in [false, true] {
                let path = dir.path().join(format!("missing-{read_only}.db"));
                let url = format!("sqlite:///{}", path.display());
                assert!(!path.exists());
                let opened = crate::db::open_existing(&url, read_only).unwrap();
                assert!(opened.is_none(), "an absent file has no connection");
                assert!(!path.exists(), "opening an absent file preserves absence");
            }
        }

        #[test]
        fn an_existing_writable_connection_keeps_pragmas_and_persists_writes() {
            let (_dir, url) = file_with_probe();
            let conn = crate::db::open_existing(&url, false)
                .unwrap()
                .expect("the existing file has a connection");
            assert_connection_pragmas(&conn);
            assert_eq!(conn.execute("UPDATE probe SET value=11", []).unwrap(), 1);
            drop(conn);
            let reopened = connect_read_only(&url).unwrap();
            assert_eq!(
                reopened
                    .query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                11
            );
        }

        #[test]
        fn an_existing_read_only_connection_keeps_pragmas_and_refuses_writes() {
            let (_dir, url) = file_with_probe();
            let conn = crate::db::open_existing(&url, true)
                .unwrap()
                .expect("the existing file has a connection");
            assert_connection_pragmas(&conn);
            let failure = conn.execute("UPDATE probe SET value=11", []).unwrap_err();
            assert!(
                matches!(
                    &failure,
                    rusqlite::Error::SqliteFailure(code, _)
                        if code.code == rusqlite::ErrorCode::ReadOnly
                ),
                "a valid write is refused as read-only: {failure:?}"
            );
            assert_eq!(
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                7
            );
        }
    }

    fn require_current(conn: &Connection) -> Result<(), CafleetError> {
        classify_selected_schema(conn, "sqlite:///:memory:").require_current()
    }

    fn initialized_conn() -> Connection {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        initialize(&mut conn).unwrap();
        conn
    }

    fn application_rows(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "SELECT * FROM fleets ORDER BY fleet_id",
            "SELECT * FROM members ORDER BY member_id",
            "SELECT * FROM member_placements ORDER BY member_id",
            "SELECT message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at FROM messages ORDER BY message_id",
            "SELECT * FROM asset_installs ORDER BY coding_agent, path",
            "SELECT * FROM sqlite_sequence ORDER BY name",
        ].map(|sql| rows(conn, sql)).into()
    }

    #[test]
    fn repeated_initialization_preserves_all_application_rows() {
        let mut conn = initialized_conn();
        conn.execute_batch(
                "INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'preserved', 'created');
                 INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES
                    (1, 1, 'Director', 'director assignment', 'active', 'registered'),
                    (2, 1, 'worker', 'worker assignment', 'active', 'registered'),
                    (3, 1, 'departed', 'finished assignment', 'deregistered', 'registered');
                 INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, mux_server, coding_agent, created_at) VALUES
                    (1, 'main', '@1', '%0', 'tmux', '/tmp/tmux-0/default', 'claude', 'placed'),
                    (2, 'main', '@1', NULL, 'tmux', '/tmp/tmux-0/default', 'codex', 'placed');
                 UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;
                 INSERT INTO messages(message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at) VALUES
                    (7, 1, 2, 'created-7', 'input_required', 'status-7', 'pending unicode 世界', NULL),
                    (8, 1, 2, 'created-8', 'input_required', 'status-8', 'notified', 'notified-8'),
                    (9, 2, 1, 'created-9', 'completed', 'status-9', 'completed', NULL);
                 INSERT INTO asset_installs VALUES('claude', '/preserved/config', 'installed');"
            ).unwrap();
        assert!(schema_matches(&conn).unwrap());
        require_current(&conn).unwrap();
        let before = application_rows(&conn);
        let ledger = rows(
            &conn,
            "SELECT * FROM refinery_schema_history ORDER BY version",
        );
        for _ in 0..2 {
            initialize(&mut conn).unwrap();
            assert_eq!(application_rows(&conn), before);
            assert_eq!(
                rows(
                    &conn,
                    "SELECT * FROM refinery_schema_history ORDER BY version"
                ),
                ledger
            );
            require_current(&conn).unwrap();
        }
    }

    #[test]
    fn baseline_initialization_records_the_identity_ledger_version_and_shape() {
        assert_eq!(APPLICATION_ID, 0x4341_4639, "ASCII CAF9");
        let conn = initialized_conn();
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            APPLICATION_ID
        );
        assert_eq!(
            rows(&conn, "SELECT version, name FROM refinery_schema_history"),
            vec![vec![
                rusqlite::types::Value::Integer(1),
                rusqlite::types::Value::Text("baseline".into())
            ]]
        );
        assert_eq!(
            rows(
                &conn,
                "SELECT singleton_id, cafleet_version FROM cafleet_database"
            ),
            vec![vec![
                rusqlite::types::Value::Integer(1),
                rusqlite::types::Value::Text(crate::cli::VERSION.into())
            ]]
        );
        assert!(schema_matches(&conn).unwrap());
    }

    #[test]
    fn a_v8_family_file_is_incompatible_and_left_byte_identical() {
        const V8_APPLICATION_ID: i64 = 0x4341_4638;
        for (name, damage) in [
            (
                "cafleet_v8.db",
                "UPDATE refinery_schema_history SET version=8, name='uncertain_cause'",
            ),
            (
                "cafleet_0.0.0.db",
                "UPDATE cafleet_database SET cafleet_version='0.0.0'",
            ),
        ] {
            assert_incompatible_and_untouched(name, |conn| {
                if name == "cafleet_v8.db" {
                    conn.pragma_update(None, "application_id", V8_APPLICATION_ID)
                        .unwrap();
                }
                conn.execute_batch(damage).unwrap();
            });
        }
    }

    fn assert_incompatible_and_untouched(name: &str, damage: impl FnOnce(&Connection)) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let url = format!("sqlite:///{}", path.display());
        {
            let mut conn = connect(&url).unwrap();
            initialize(&mut conn).unwrap();
            damage(&conn);
        }
        let before = std::fs::read(&path).unwrap();
        let conn = connect_read_only(&url).unwrap();
        let state = crate::db::classify_selected_schema(&conn, &url);
        assert!(
            matches!(&state, crate::db::SchemaState::Incompatible { path: shown } if shown == path.to_str().unwrap()),
            "{state:?}"
        );
        assert_eq!(
            state.require_current().unwrap_err().message(),
            format!(
                "database format is incompatible at '{}'; choose an empty database path and run 'cafleet setup'.",
                path.display()
            )
        );
        drop(conn);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_ledger_row_beyond_the_baseline_is_incompatible() {
        let conn = initialized_conn();
        require_current(&conn).unwrap();
        conn.execute(
            "INSERT INTO refinery_schema_history(version, name, applied_on, checksum) VALUES(2, 'hermes_backend', 'applied', '0')",
            [],
        )
        .unwrap();
        assert_eq!(
            classify_schema(&conn).unwrap(),
            Classification::Incompatible
        );
    }

    #[test]
    fn selected_reference_rechecks_ddl_and_accepts_auxiliary_objects() {
        let conn = initialized_conn();
        conn.execute_batch(
            "CREATE TABLE operator_data(value TEXT);
             CREATE INDEX operator_messages ON messages(text);
             CREATE TRIGGER operator_message_insert AFTER INSERT ON messages BEGIN
                INSERT INTO operator_data VALUES(NEW.text);
             END;",
        )
        .unwrap();
        assert!(schema_matches(&conn).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN text TO broken")
            .unwrap();
        assert!(!schema_matches(&conn).unwrap());
        conn.execute_batch("ALTER TABLE messages RENAME COLUMN broken TO text")
            .unwrap();
        assert!(schema_matches(&conn).unwrap());
        conn.execute_batch("CREATE TEMP TABLE messages(wrong TEXT)")
            .unwrap();
        assert!(!schema_matches(&conn).unwrap());
    }

    #[test]
    fn required_schema_components_and_identity_are_refused() {
        for damage in [
            "DROP INDEX idx_messages_to_member_status_ts",
            "DROP INDEX idx_messages_to_member_status_ts; CREATE INDEX idx_messages_to_member_status_ts ON messages(from_member_id, status_timestamp)",
            "DROP TABLE asset_installs",
            "DELETE FROM cafleet_database",
        ] {
            let conn = initialized_conn();
            conn.execute_batch(damage).unwrap();
            assert!(require_current(&conn).is_err(), "{damage}");
        }
    }

    #[test]
    fn structural_reference_checks_required_keys_types_defaults_and_foreign_keys() {
        for (table, original, replacement) in [
            (
                "asset_installs",
                "PRIMARY KEY (coding_agent, path)",
                "PRIMARY KEY (coding_agent)",
            ),
            ("members", "name TEXT NOT NULL", "name TEXT"),
            ("messages", "text TEXT NOT NULL", "text INTEGER NOT NULL"),
            (
                "member_placements",
                "backend TEXT NOT NULL,",
                "backend TEXT NOT NULL DEFAULT 'tmux',",
            ),
            ("messages", "ON DELETE RESTRICT", "ON DELETE CASCADE"),
        ] {
            let conn = initialized_conn();
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(ddl.contains(original));
            let version: i64 = conn
                .query_row("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=ON").unwrap();
            conn.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE name=?2",
                rusqlite::params![ddl.replace(original, replacement), table],
            )
            .unwrap();
            conn.pragma_update(None, "schema_version", version + 1)
                .unwrap();
            conn.execute_batch("PRAGMA writable_schema=OFF").unwrap();
            assert!(!schema_matches(&conn).unwrap(), "{table}: {original}");
        }
    }

    #[test]
    fn connection_pragmas_and_foreign_keys_protect_real_writes() {
        let mut conn = connect("sqlite:///:memory:").unwrap();
        initialize(&mut conn).unwrap();
        assert!(conn.execute("INSERT INTO members(fleet_id, name, description, status, registered_at) VALUES(404, 'orphan', '', 'active', '')", []).is_err());
        conn.execute_batch("INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'valid', 'created'); INSERT INTO members(member_id, fleet_id, name, description, status, registered_at) VALUES(1, 1, 'valid', '', 'active', 'registered');").unwrap();
        for (from, to) in [(404, 1), (1, 404)] {
            assert!(conn.execute("INSERT INTO messages(from_member_id, to_member_id, created_at, status_state, status_timestamp, text) VALUES(?1, ?2, '', 'input_required', '', '')", rusqlite::params![from, to]).is_err());
        }
        for url in ["postgresql:///registry", "mysql://localhost/registry"] {
            assert!(connect(url).is_err());
        }
    }

    #[test]
    fn failed_baseline_initialization_rolls_back_identity_tables_and_ledger() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA max_page_count=1").unwrap();
        assert!(initialize(&mut conn).is_err());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            rows(
                &conn,
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
            )
            .is_empty()
        );
    }

    mod runtime_tests {
        use rusqlite::Connection;
        use rusqlite::params;
        use serde_json::json;
        use tempfile::TempDir;

        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::members::test_support::register;
        use crate::db::tests::test_support::migrated_conn;

        const NOW: &str = "2026-10-26T10:00:00.000000+00:00";

        struct Fixture {
            _dir: TempDir,
            conn: Connection,
            fleet: i64,
            director: i64,
            member: i64,
        }

        impl Fixture {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                let conn = migrated_conn(&dir);
                Self::seeded(dir, conn)
            }

            fn seeded(dir: TempDir, mut conn: Connection) -> Self {
                let (fleet, director) = create_fleet_fixture(&mut conn, "runtime-schema");
                let member = register(&mut conn, fleet, "worker", Some("%2"));
                Self {
                    _dir: dir,
                    conn,
                    fleet,
                    director,
                    member,
                }
            }

            fn run(&self, status: &str) -> rusqlite::Result<i64> {
                self.backend_run("codex", status, None)
            }

            fn backend_run(
                &self,
                backend: &str,
                status: &str,
                native_session_json: Option<&str>,
            ) -> rusqlite::Result<i64> {
                self.conn.execute(
            "INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,
             database_identity_json,launch_nonce,runtime_path,resolved_mode,mode_source,
             permission_argv_json,event_evidence,coverage,status,
             owner_epoch,owner_kind,owner_pid,owner_start_identity,
             cleanup_state,event_seq_reserved_through,native_session_json)
             VALUES(?1,?2,?3,?7,?4,?5,'/isolated/runtime','never','default',
             '[\"--ask-for-approval\",\"never\",\"--sandbox\",\"workspace-write\"]',
             'metadata','unknown',?6,1,'creator',1234,'fixture-owner',
             'incomplete',0,?8)",
            params![self.member,self.fleet,self.director,
                json!({"canonical_path": self.conn.path().unwrap(), "instance_id": "a".repeat(32), "device":1,"inode":2}).to_string(),
                "b".repeat(64),status,backend,native_session_json],
        )?;
                Ok(self.conn.last_insert_rowid())
            }

            fn interaction(&self, run: i64, seq: i64, class: &str) -> rusqlite::Result<i64> {
                self.conn.execute(
            "INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json)
             VALUES(?1,?2,?3,?4,?5,'{}')",
            params![run,seq,class,"c".repeat(64),NOW],
        )?;
                Ok(self.conn.last_insert_rowid())
            }

            fn archived_event(&self, run: i64, seq: i64) -> rusqlite::Result<i64> {
                self.conn.execute(
                    "INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,
             payload_json,notification_state,archive_reason)
             VALUES(?1,?2,'coverage_unknown',?3,'{}',
             'archived','member_inactive')",
                    params![run, seq, NOW],
                )?;
                Ok(self.conn.last_insert_rowid())
            }

            fn input(&self, run: i64, key: &str) -> rusqlite::Result<i64> {
                self.conn.execute(
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at)
             VALUES(?1,?2,'prompt',?3,'in_progress',?4,?4)",
            params![run,key,"d".repeat(64),NOW],
        )?;
                Ok(self.conn.last_insert_rowid())
            }
        }

        fn count(conn: &Connection, table: &str) -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        }

        #[test]
        fn database_instance_identity_is_singleton_stable_and_distinct() {
            let mut first = Fixture::new();
            let second = Fixture::new();
            let identity = |conn: &Connection| {
                conn.query_row(
                    "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
            };
            let original = identity(&first.conn);
            assert_eq!(original.len(), 32);
            assert!(original.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_ne!(original, identity(&second.conn));
            crate::db::initialize(&mut first.conn).unwrap();
            let url = format!("sqlite:///{}", first.conn.path().unwrap());
            assert_eq!(
                identity(&crate::db::tests::connect(&url).unwrap()),
                original
            );
            assert_eq!(count(&first.conn, "runtime_database_identity"), 1);
            assert!(
            first
                .conn
                .execute(
                    "INSERT INTO runtime_database_identity(singleton_id,instance_id) VALUES(2,?1)",
                    ["e".repeat(32)]
                )
                .is_err()
        );
        }

        #[test]
        fn every_nonterminal_status_enforces_one_live_run_per_member() {
            let fixture = Fixture::new();
            for status in ["starting", "prepared", "running", "degraded", "recovering"] {
                let run = fixture.run(status).unwrap();
                assert!(fixture.run("starting").is_err(), "{status}");
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='exited' WHERE run_id=?1",
                        [run],
                    )
                    .unwrap();
                let next = fixture.run("starting").unwrap();
                assert!(next > run);
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='exited' WHERE run_id=?1",
                        [next],
                    )
                    .unwrap();
            }
        }

        #[test]
        fn terminal_run_history_allows_a_new_run_without_losing_the_old_one() {
            let fixture = Fixture::new();
            for (index, status) in ["exited", "failed", "lost"].into_iter().enumerate() {
                let old = fixture.run(status).unwrap();
                let new = fixture.run("starting").unwrap();
                assert!(new > old);
                assert_eq!(count(&fixture.conn, "member_runs"), 2 * (index as i64 + 1));
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='exited' WHERE run_id=?1",
                        [new],
                    )
                    .unwrap();
            }
        }

        #[test]
        fn run_sql_domains_and_json_shapes_reject_corrupt_values() {
            let fixture = Fixture::new();
            let run = fixture.run("starting").unwrap();
            for (field, value) in [
                ("status", "bogus"),
                ("mode_source", "inherited"),
                ("owner_kind", "member"),
                ("cleanup_state", "unknown"),
                ("coverage", "universal"),
                ("event_evidence", "raw"),
                ("backend", "bogus"),
                ("database_identity_json", "[]"),
                ("effective_policy_json", "[]"),
                ("permission_argv_json", "{}"),
                ("cleanup_diagnostics_json", "{}"),
                ("runtime_dir_identity_json", "[]"),
                ("run_lock_identity_json", "[]"),
            ] {
                assert!(
                    fixture
                        .conn
                        .execute(
                            &format!("UPDATE member_runs SET {field}=?1 WHERE run_id=?2"),
                            params![value, run]
                        )
                        .is_err(),
                    "{field}={value}"
                );
            }
            for field in ["owner_epoch", "owner_pid"] {
                assert!(
                    fixture
                        .conn
                        .execute(
                            &format!("UPDATE member_runs SET {field}=0 WHERE run_id=?1"),
                            [run]
                        )
                        .is_err(),
                    "{field}"
                );
            }
            assert!(
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET event_seq_reserved_through=-1 WHERE run_id=?1",
                        [run]
                    )
                    .is_err()
            );
        }

        #[test]
        fn run_backend_check_accepts_every_supported_backend_only() {
            let fixture = Fixture::new();
            for backend in crate::coding_agent::CodingAgent::all() {
                fixture
                    .backend_run(backend.name(), "exited", None)
                    .unwrap_or_else(|error| panic!("{backend:?}: {error}"));
            }
            for backend in ["gemini", "Hermes"] {
                assert!(
                    fixture.backend_run(backend, "exited", None).is_err(),
                    "{backend:?}"
                );
            }
        }

        #[test]
        fn native_session_column_accepts_only_null_or_a_json_object() {
            let fixture = Fixture::new();
            for session in ["{}", "{\"session_key\":\"tool:abc\",\"pid\":12}"] {
                fixture
                    .backend_run("hermes", "exited", Some(session))
                    .unwrap_or_else(|error| panic!("{session}: {error}"));
            }
            for session in ["[]", "not json"] {
                assert!(
                    fixture
                        .backend_run("hermes", "exited", Some(session))
                        .is_err(),
                    "{session:?}"
                );
            }
            let run = fixture.backend_run("hermes", "exited", None).unwrap();
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET native_session_json='{\"session_key\":\"k\"}' WHERE run_id=?1",
                    [run],
                )
                .unwrap();
            assert!(
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET native_session_json='[1]' WHERE run_id=?1",
                        [run]
                    )
                    .is_err()
            );
        }

        #[test]
        fn interaction_identity_and_the_single_open_episode_are_enforced() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let first = fixture.interaction(run, 1, "permission_request").unwrap();
            assert!(fixture.interaction(run, 2, "question").is_err());
            fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",params![NOW,first]).unwrap();
            assert!(fixture.interaction(run, 1, "question").is_err());
            assert!(fixture.interaction(run, 2, "question").unwrap() > first);
        }

        #[test]
        fn interaction_classes_preserve_distinct_native_forms() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let mut seq = 0;
            for class in crate::broker::runtime::InteractionClass::VALUES {
                seq += 1;
                let interaction = fixture.interaction(run, seq, class).unwrap();
                fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",params![NOW,interaction]).unwrap();
            }
            for class in [
                "permission_denied",
                "working",
                "composer_empty",
                "unknown",
                "bogus",
            ] {
                assert!(fixture.interaction(run, seq + 1, class).is_err(), "{class}");
            }
        }

        #[test]
        fn interaction_closure_fields_are_paired_and_reasons_are_typed() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let interaction = fixture.interaction(run, 1, "permission_request").unwrap();
            for assignment in [
                "closed_at='2026-10-26T10:00:00Z'",
                "close_reason='not_visible'",
                "closed_at='2026-10-26T10:00:00Z',close_reason='approved'",
                "evidence_json='[]'",
                "episode_seq=0",
            ] {
                assert!(
                fixture
                    .conn
                    .execute(
                        &format!(
                            "UPDATE member_interactions SET {assignment} WHERE interaction_id=?1"
                        ),
                        [interaction]
                    )
                    .is_err(),
                "{assignment}"
            );
            }
            for reason in crate::broker::runtime::InteractionCloseReason::VALUES {
                fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason=?2 WHERE interaction_id=?3",params![NOW,reason,interaction]).unwrap();
            }
        }

        #[test]
        fn runtime_event_sequences_are_positive_and_unique_within_a_run() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            fixture.archived_event(run, 1).unwrap();
            assert!(fixture.archived_event(run, 1).is_err());
            for seq in [0, -1] {
                assert!(fixture.archived_event(run, seq).is_err());
            }
            let old = fixture.run("exited").unwrap();
            fixture.archived_event(old, 1).unwrap();
        }

        #[test]
        fn notification_state_requires_exactly_its_message_or_archive_reason() {
            let mut fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let event = fixture.archived_event(run, 1).unwrap();
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.member,
                fixture.director,
                "runtime",
            )
            .unwrap();
            for assignment in [
                "notification_state='inbox'",
                "archive_reason=NULL",
                "archive_reason='notified'",
                "notification_state='unknown'",
                "kind='approved'",
                "payload_json='[]'",
                "payload_json='broken'",
            ] {
                assert!(
                    fixture
                        .conn
                        .execute(
                            &format!(
                                "UPDATE member_runtime_events SET {assignment} WHERE event_id=?1"
                            ),
                            [event]
                        )
                        .is_err(),
                    "{assignment}"
                );
            }
            fixture.conn.execute("UPDATE member_runtime_events SET notification_state='inbox',archive_reason=NULL,message_id=?1 WHERE event_id=?2",params![message.message_id,event]).unwrap();
            for assignment in ["message_id=NULL", "archive_reason='member_inactive'"] {
                assert!(
                    fixture
                        .conn
                        .execute(
                            &format!(
                                "UPDATE member_runtime_events SET {assignment} WHERE event_id=?1"
                            ),
                            [event]
                        )
                        .is_err(),
                    "{assignment}"
                );
            }
        }

        #[test]
        fn archive_reasons_accept_the_specified_domain() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let event = fixture.archived_event(run, 1).unwrap();
            for reason in crate::broker::runtime::ArchiveReason::VALUES {
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runtime_events SET archive_reason=?1 WHERE event_id=?2",
                        params![reason, event],
                    )
                    .unwrap();
            }
        }

        #[test]
        fn durable_input_keys_are_run_scoped_and_settlements_require_a_disposition() {
            let fixture = Fixture::new();
            let run = fixture.run("running").unwrap();
            let input = fixture.input(run, "prompt:one").unwrap();
            assert!(fixture.input(run, "prompt:one").is_err());
            let old = fixture.run("exited").unwrap();
            fixture.input(old, "prompt:one").unwrap();
            for assignment in [
                "state='settled'",
                "kind='preview'",
                "kind='approval'",
                "state='running'",
                "disposition='approved'",
            ] {
                assert!(
                    fixture
                        .conn
                        .execute(
                            &format!(
                                "UPDATE member_input_operations SET {assignment} WHERE input_id=?1"
                            ),
                            [input]
                        )
                        .is_err(),
                    "{assignment}"
                );
            }
            for disposition in crate::broker::runtime::InputDisposition::VALUES {
                fixture.conn.execute("UPDATE member_input_operations SET state='settled',disposition=?1 WHERE input_id=?2",params![disposition,input]).unwrap();
            }
        }

        #[test]
        fn runtime_foreign_keys_retain_history_and_reject_orphans() {
            let fixture = Fixture::new();
            assert!(fixture.interaction(404, 1, "question").is_err());
            assert!(fixture.archived_event(404, 1).is_err());
            assert!(fixture.input(404, "orphan").is_err());
            let run = fixture.run("exited").unwrap();
            fixture.interaction(run, 1, "question").unwrap();
            fixture.archived_event(run, 1).unwrap();
            fixture.input(run, "retained").unwrap();
            assert!(
                fixture
                    .conn
                    .execute("DELETE FROM member_runs WHERE run_id=?1", [run])
                    .is_err()
            );
            assert!(
                fixture
                    .conn
                    .execute("DELETE FROM members WHERE member_id=?1", [fixture.member])
                    .is_err()
            );
            assert_eq!(count(&fixture.conn, "member_runs"), 1);
        }
    }

    #[test]
    fn classification_preserves_missing_current_identity_and_sql_causes() {
        let mut conn = crate::db::connect("sqlite:///:memory:").unwrap();
        assert_eq!(classify_schema(&conn).unwrap(), Classification::Missing);
        crate::db::initialize(&mut conn).unwrap();
        assert_eq!(classify_schema(&conn).unwrap(), Classification::Current);
        conn.execute_batch("PRAGMA application_id=1").unwrap();
        assert_eq!(
            classify_schema(&conn).unwrap(),
            Classification::Incompatible
        );
        conn.pragma_update(None, "application_id", crate::db::APPLICATION_ID)
            .unwrap();
        conn.execute_batch(
            "ALTER TABLE refinery_schema_history RENAME COLUMN version TO missing_version",
        )
        .unwrap();
        assert!(classify_schema(&conn).is_err());
    }
    mod database_scope_tests {
        use crate::db::*;

        #[test]
        fn expired_completion_commits_uncontended_work() {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE probe(value INTEGER);")
                .unwrap();
            let expired = Deadline::until(Instant::now() - Duration::from_millis(1));
            assert!(with_database_budget(&mut conn, expired, |_| Ok(())).is_err());
            complete_database_budget(&mut conn, expired, |conn| {
                conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO probe VALUES(42); COMMIT;")
                    .map_err(crate::error::CafleetError::from)
            })
            .unwrap();
            assert_eq!(
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                42
            );
        }

        #[test]
        fn completion_waits_for_a_reader_to_release_commit_after_input_expiry() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("reader.db");
            let mut conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(1)")
                .unwrap();
            let reader = Connection::open(&path).unwrap();
            reader.execute_batch("BEGIN; SELECT * FROM probe").unwrap();
            let (release, released) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                released.recv().unwrap();
                std::thread::sleep(Duration::from_millis(100));
                reader.execute_batch("COMMIT").unwrap();
            });
            let deadline = Deadline::until(Instant::now() + Duration::from_millis(20));
            with_database_budget(&mut conn, deadline, |conn| {
                std::thread::sleep(Duration::from_millis(30));
                complete_database_budget(conn, deadline, |conn| {
                    let tx =
                        conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    tx.execute("UPDATE probe SET value=42", [])?;
                    release.send(()).unwrap();
                    tx.commit().map_err(crate::error::CafleetError::from)
                })?;
                assert!(with_database_budget(conn, deadline, |_| Ok(())).is_err());
                Ok(())
            })
            .unwrap();
            thread.join().unwrap();
            assert_eq!(
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                42
            );
        }

        #[test]
        fn expired_completion_reports_contention_after_bounded_grace() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("completion.db");
            let writer = Connection::open(&path).unwrap();
            writer
                .execute_batch("CREATE TABLE probe(value INTEGER); BEGIN IMMEDIATE;")
                .unwrap();
            let mut conn = Connection::open(&path).unwrap();
            let started = Instant::now();
            let expired = Deadline::until(started - Duration::from_millis(1));
            let error = complete_database_budget(&mut conn, expired, |conn| {
                conn.execute("INSERT INTO probe VALUES(42)", [])
                    .map_err(crate::error::CafleetError::from)
            })
            .unwrap_err();
            assert!(error.message().contains("database is locked"));
            assert!(started.elapsed() >= COMPLETION_GRACE * 9 / 10);
            assert!(started.elapsed() < COMPLETION_GRACE * 2);
        }

        fn assert_timeout(conn: &Connection, expected: u32) {
            assert_eq!(
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u32>(0))
                    .unwrap(),
                expected
            );
        }

        #[test]
        fn scope_exit_restores_timeouts_and_allows_sequential_connections_and_threads() {
            let mut first = Connection::open_in_memory().unwrap();
            let mut second = Connection::open_in_memory().unwrap();
            for conn in [&mut first, &mut second] {
                conn.busy_timeout(Duration::from_millis(137)).unwrap();
                with_database_budget(conn, Deadline::new(), |_| Ok(())).unwrap();
                assert_timeout(conn, 137);
                assert!(
                    with_database_budget(conn, Deadline::new(), |_| Err::<(), _>(
                        CafleetError::App("expected".into())
                    ))
                    .is_err()
                );
                assert_timeout(conn, 137);
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = with_database_budget(
                        conn,
                        Deadline::new(),
                        |_| -> Result<(), CafleetError> { panic!("scope unwind") },
                    );
                }));
                assert!(panic.is_err());
                assert_timeout(conn, 137);
            }
            with_database_budget(&mut first, Deadline::new(), |_| {
                std::thread::spawn(|| {
                    let mut conn = Connection::open_in_memory().unwrap();
                    with_database_budget(&mut conn, Deadline::new(), |_| Ok(())).unwrap();
                })
                .join()
                .unwrap();
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn cross_connection_nesting_fails_before_changing_the_inner_timeout() {
            let mut outer = Connection::open_in_memory().unwrap();
            let mut inner = Connection::open_in_memory().unwrap();
            inner.busy_timeout(Duration::from_millis(137)).unwrap();
            with_database_budget(&mut outer, Deadline::new(), |_| {
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_database_budget(&mut inner, Deadline::new(), |_| Ok(()))
                }));
                assert!(panic.is_err());
                assert_timeout(&inner, 137);
                Ok(())
            })
            .unwrap();
        }

        #[test]
        fn nested_exit_restores_the_earliest_outer_deadline_on_every_exit() {
            for exit in [0, 2] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("nested.db");
                let writer = Connection::open(&path).unwrap();
                writer.execute_batch("CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE").unwrap();
                let mut conn = Connection::open(&path).unwrap();
                conn.busy_timeout(Duration::from_millis(137)).unwrap();
                let started = Instant::now();
                let outer = Deadline::until(started + Duration::from_millis(160));
                let error = with_database_budget(&mut conn, outer, |conn| {
                    match exit {
                        0 => with_database_budget(conn, Deadline::new(), |_| Ok(())).unwrap(),
                        2 => assert!(
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let _ = with_database_budget(
                                    conn,
                                    Deadline::new(),
                                    |_| -> Result<(), CafleetError> { panic!("nested unwind") },
                                );
                            }))
                            .is_err()
                        ),
                        _ => unreachable!(),
                    }
                    with_database_budget(conn, Deadline::new(), |conn| {
                        conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                            .map_err(crate::error::CafleetError::from)
                    })
                })
                .unwrap_err();
                assert!(matches!(error, CafleetError::DatabaseBusy(_)));
                assert!(started.elapsed() >= Duration::from_millis(140));
                assert!(started.elapsed() < Duration::from_millis(700));
                assert_timeout(&conn, 137);
            }
        }

        #[test]
        fn consecutive_sql_waits_share_one_absolute_deadline() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("aggregate.db");
            let writer = Connection::open(&path).unwrap();
            writer
            .execute_batch(
                "CREATE TABLE probe(value INTEGER); INSERT INTO probe VALUES(42); BEGIN EXCLUSIVE",
            )
            .unwrap();
            let release = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                writer.execute_batch("ROLLBACK").unwrap();
            });
            let mut conn = Connection::open(&path).unwrap();
            let started = Instant::now();
            let deadline = Deadline::until(started + Duration::from_millis(200));
            let error = with_database_budget(&mut conn, deadline, |conn| {
                assert_eq!(
                    conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))?,
                    42
                );
                release.join().unwrap();
                let writer = Connection::open(&path).unwrap();
                writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
                conn.query_row("SELECT value FROM probe", [], |row| row.get::<_, i64>(0))
                    .map_err(crate::error::CafleetError::from)
            })
            .unwrap_err();
            assert!(matches!(error, CafleetError::DatabaseBusy(_)));
            assert!(started.elapsed() >= Duration::from_millis(180));
            assert!(started.elapsed() < Duration::from_millis(700));
        }
    }
}
