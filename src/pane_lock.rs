use crate::broker::MemberRecord;
use crate::broker::Placement;
use crate::config::Settings;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::time::Deadline;
use rusqlite::Connection;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) fn valid_input(text: &str) -> bool {
    !text.trim().is_empty() && !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

pub(crate) fn member_pane_input_error(member_id: i64) -> impl FnOnce(CafleetError) -> CafleetError {
    move |error| {
        CafleetError::App(format!(
            "member {member_id} pane input: {}",
            error.message()
        ))
    }
}

pub(crate) struct PaneLock {
    file: File,
    path: PathBuf,
}

pub(crate) fn lock_path(
    database_url: &str,
    placement: &Placement,
) -> Result<PathBuf, CafleetError> {
    let pane = placement
        .mux_pane_id
        .as_deref()
        .ok_or_else(|| CafleetError::App("pane input requires a concrete placement".into()))?;
    let database =
        std::fs::canonicalize(crate::db::database_path(database_url)?).map_err(|error| {
            CafleetError::App(format!(
                "cannot resolve database input-lock identity: {error}"
            ))
        })?;
    let mut directory = database.into_os_string();
    directory.push(".pane-locks");
    let key = format!("{}\0{}\0{pane}", placement.backend, placement.mux_session);
    let key = crate::digest::sha256_hex(key.as_bytes());
    Ok(PathBuf::from(directory).join(format!("{key}.lock")))
}

impl PaneLock {
    pub(crate) fn validate_binding(
        &self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<(), CafleetError> {
        use std::os::unix::fs::MetadataExt;
        let path = lock_path(database_url, placement)?;
        if self.path != path {
            return Err(CafleetError::App("held pane lock binding mismatch".into()));
        }
        let open = self
            .file
            .metadata()
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        let named = std::fs::symlink_metadata(&path)
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        if !named.is_file()
            || named.file_type().is_symlink()
            || (open.dev(), open.ino()) != (named.dev(), named.ino())
        {
            return Err(CafleetError::App(
                "held pane lock pathname identity mismatch".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn try_acquire(
        database_url: &str,
        placement: &Placement,
    ) -> Result<Option<Self>, CafleetError> {
        let path = lock_path(database_url, placement)?;
        let directory = path
            .parent()
            .expect("a pane lock has its database-adjacent directory");
        std::fs::create_dir_all(directory).map_err(|error| {
            CafleetError::App(format!(
                "cannot create pane input-lock directory {}: {error}",
                directory.display()
            ))
        })?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                CafleetError::App(format!(
                    "cannot open pane input lock {}: {error}",
                    path.display()
                ))
            })?;
        Ok(try_lock(&file, &path)?.then_some(Self { file, path }))
    }

    /// Releases this held lock so the member's runner can write its staged input.
    pub(crate) fn release(&self) -> Result<(), CafleetError> {
        self.file.unlock().map_err(|error| {
            CafleetError::App(format!(
                "cannot release pane input {}: {error}",
                self.path.display()
            ))
        })
    }

    /// Retakes a released lock, retrying every 50 ms until `lock_deadline`; `false` means it
    /// stayed busy.
    pub(crate) fn reacquire_within(&self, lock_deadline: Deadline) -> Result<bool, CafleetError> {
        let reacquired = retry_until(lock_deadline, || {
            Ok(try_lock(&self.file, &self.path)?.then_some(()))
        })?;
        Ok(reacquired.is_some())
    }

    /// Retry a held lock every 50 ms until `lock_deadline`; `None` means it stayed busy.
    pub(crate) fn acquire_within(
        database_url: &str,
        placement: &Placement,
        lock_deadline: Deadline,
    ) -> Result<Option<Self>, CafleetError> {
        retry_until(lock_deadline, || Self::try_acquire(database_url, placement))
    }
}

/// `false` means another holder has the lock.
fn try_lock(file: &File, path: &Path) -> Result<bool, CafleetError> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(CafleetError::App(format!(
            "cannot lock pane input {}: {error}",
            path.display()
        ))),
    }
}

/// Repeats `attempt` every 50 ms until it yields a value, making one last attempt after the
/// sleep that reaches `deadline`; `None` means every attempt came up empty.
fn retry_until<T>(
    deadline: Deadline,
    mut attempt: impl FnMut() -> Result<Option<T>, CafleetError>,
) -> Result<Option<T>, CafleetError> {
    loop {
        if let Some(value) = attempt()? {
            return Ok(Some(value));
        }
        let Ok(left) = deadline.remaining() else {
            return Ok(None);
        };
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

#[cfg(test)]
impl PaneLock {
    pub(crate) const RELEASED_LOCK_GRACE: Duration = Duration::from_secs(2);
    /// Acquire a lock the caller has just seen released. A child forked by a concurrent
    /// test shares the released descriptor's flock until its exec closes it, so retry briefly.
    pub(crate) fn acquire_released(
        database_url: &str,
        placement: &Placement,
    ) -> Result<Option<Self>, CafleetError> {
        Self::acquire_within(
            database_url,
            placement,
            Deadline::until(std::time::Instant::now() + Self::RELEASED_LOCK_GRACE),
        )
    }
}

pub(crate) struct LockedMember {
    pub(crate) member: MemberRecord,
    guard: PaneLock,
}

impl LockedMember {
    pub(crate) fn pane_lock(&self, database_url: &str) -> Result<&PaneLock, CafleetError> {
        self.guard
            .validate_binding(database_url, &self.member.placement)?;
        Ok(&self.guard)
    }
}

pub(crate) struct LockedDeletion<S, T> {
    pub(crate) snapshot: S,
    pub(crate) value: T,
    _span: crate::time::DeferTermination,
    guards: Vec<PaneLock>,
}

pub(crate) fn pane_lock_for<'a>(
    guards: &'a [PaneLock],
    database_url: &str,
    placement: &Placement,
) -> Result<&'a PaneLock, CafleetError> {
    let path = lock_path(database_url, placement)?;
    let guard = guards
        .iter()
        .find(|guard| guard.path == path)
        .ok_or_else(|| CafleetError::App("matching held pane lock required".into()))?;
    guard.validate_binding(database_url, placement)?;
    Ok(guard)
}

impl<S, T> LockedDeletion<S, T> {
    pub(crate) fn pane_lock(
        &self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<&PaneLock, CafleetError> {
        pane_lock_for(&self.guards, database_url, placement)
    }

    pub(crate) fn snapshot_and_pane_lock(
        &mut self,
        database_url: &str,
        placement: &Placement,
    ) -> Result<(&mut S, &PaneLock), CafleetError> {
        let guard = pane_lock_for(&self.guards, database_url, placement)?;
        Ok((&mut self.snapshot, guard))
    }
}

pub(crate) fn lock_deletion<S, T>(
    conn: &mut Connection,
    settings: &Settings,
    deadline: Deadline,
    mut load: impl FnMut(&mut Connection) -> Result<(S, Vec<(i64, Placement)>), CafleetError>,
    mut revalidate: impl FnMut(&mut Connection, &mut S, &[PaneLock]) -> Result<Option<T>, CafleetError>,
    busy: impl Fn(i64) -> CafleetError,
) -> Result<LockedDeletion<S, T>, CafleetError> {
    loop {
        let (mut snapshot, targets) = with_database_budget(conn, deadline, &mut load)?;
        let mut targets = targets
            .into_iter()
            .map(|(member_id, placement)| {
                let path = lock_path(&settings.database_url, &placement)
                    .map_err(member_pane_input_error(member_id))?;
                Ok((path, member_id, placement))
            })
            .collect::<Result<Vec<_>, CafleetError>>()?;
        targets.sort_by(|left, right| (&left.0, left.1).cmp(&(&right.0, right.1)));
        targets.dedup_by(|left, right| left.0 == right.0);
        let mut guards = Vec::new();
        for (_, member_id, placement) in targets {
            deadline.remaining()?;
            let guard = retry_until(deadline, || {
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .map_err(member_pane_input_error(member_id))
            })?
            .ok_or_else(|| busy(member_id))?;
            guards.push(guard);
        }
        let span = crate::time::DeferTermination::begin()?;
        let value = with_database_budget(conn, deadline, |conn| {
            revalidate(conn, &mut snapshot, &guards)
        })?;
        if let Some(value) = value {
            return Ok(LockedDeletion {
                snapshot,
                value,
                _span: span,
                guards,
            });
        }
        drop(span);
        drop(guards);
        deadline.remaining()?;
    }
}

const PRESS_LOCK_RESERVE: Duration = Duration::from_secs(1);

pub(crate) fn lock_member_with_reservation(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    deadline: Deadline,
) -> Result<Option<LockedMember>, CafleetError> {
    let placement = &member.placement;
    let Some(guard) = PaneLock::acquire_within(
        &settings.database_url,
        placement,
        deadline.reserving(PRESS_LOCK_RESERVE),
    )
    .map_err(member_pane_input_error(member.member_id))?
    else {
        return Ok(None);
    };
    let current = with_database_budget(conn, deadline, |conn| {
        crate::broker::get_member(conn, member.member_id, member.fleet_id)
    })?
    .ok_or_else(|| CafleetError::App(format!("Member {} not found", member.member_id)))?;
    if current.placement != member.placement {
        return Err(CafleetError::App(format!(
            "member {} placement changed while acquiring pane input",
            member.member_id
        )));
    }
    Ok(Some(LockedMember {
        member: current,
        guard,
    }))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::time::Duration;

    use rusqlite::Connection;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::pane_lock::*;

    pub(super) const TEST_WORK_RESERVE: Duration = Duration::from_millis(3500);

    pub(crate) struct LockFixture {
        pub(crate) _dir: tempfile::TempDir,
        pub(crate) conn: Connection,
        pub(crate) settings: Settings,
        pub(crate) member: MemberRecord,
    }

    impl LockFixture {
        pub(crate) fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "locks");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let member = crate::broker::get_member(&conn, worker, fleet)
                .unwrap()
                .unwrap();
            let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()));
            Self {
                _dir: dir,
                conn,
                settings,
                member,
            }
        }

        pub(crate) fn placement(&self) -> &Placement {
            &self.member.placement
        }

        pub(crate) fn hold(&self) -> PaneLock {
            PaneLock::try_acquire(&self.settings.database_url, self.placement())
                .unwrap()
                .expect("the fixture lock is free")
        }

        pub(crate) fn hold_for(&self, hold: Duration) -> std::thread::JoinHandle<()> {
            let guard = self.hold();
            std::thread::spawn(move || {
                std::thread::sleep(hold);
                drop(guard);
            })
        }

        pub(crate) fn writer_deadline() -> Deadline {
            Deadline::new().reserving(TEST_WORK_RESERVE)
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::runtime::test_support::*;

    mod pane_guard {
        use crate::pane_lock::PaneLock;
        use crate::pane_lock::pane_lock_for;
        use crate::pane_lock::tests::*;
        #[test]
        fn runner_recovery_pane_guard_validates_the_actual_database_placement_and_named_inode() {
            use std::os::unix::fs::PermissionsExt;
            let fixture = crate::pane_lock::test_support::LockFixture::new();
            let configuration = &fixture.settings;
            let placement = fixture.placement().clone();
            let held = PaneLock::try_acquire(&configuration.database_url, &placement)
                .unwrap()
                .unwrap();
            held.validate_binding(&configuration.database_url, &placement)
                .unwrap();
            let mut changed = placement.clone();
            changed.mux_pane_id = Some("%foreign".into());
            assert!(
                held.validate_binding(&configuration.database_url, &changed)
                    .is_err()
            );
            let sibling_path = fixture._dir.path().join("sibling.db");
            let mut sibling = Connection::open(&sibling_path).unwrap();
            crate::db::initialize(&mut sibling).unwrap();
            let sibling_url = format!("sqlite:///{}", sibling_path.display());
            assert!(held.validate_binding(&sibling_url, &placement).is_err());
            let path =
                crate::pane_lock::lock_path(&configuration.database_url, &placement).unwrap();
            let original = path.with_extension("retained");
            std::fs::rename(&path, &original).unwrap();
            std::fs::write(&path, b"replacement lock identity").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                held.validate_binding(&configuration.database_url, &placement)
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"replacement lock identity");
        }

        #[test]
        fn runner_recovery_pane_guard_selection_reuses_a_deduplicated_physical_guard() {
            let fixture = Fixture::new();
            let database_url = format!("sqlite:///{}", fixture.conn.path().unwrap());
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement;
            let guards = vec![
                PaneLock::try_acquire(&database_url, &placement)
                    .unwrap()
                    .unwrap(),
            ];
            let selected = pane_lock_for(&guards, &database_url, &placement).unwrap();
            assert!(std::ptr::eq(selected, &guards[0]));
            let mut shared = placement.clone();
            shared.mux_window_id = "@shared".into();
            assert!(std::ptr::eq(
                pane_lock_for(&guards, &database_url, &shared).unwrap(),
                &guards[0]
            ));
            assert!(
                PaneLock::try_acquire(&database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            assert!(pane_lock_for(&[], &database_url, &placement).is_err());
        }
    }

    mod pane_lock_wait_tests {
        use crate::pane_lock::test_support::LockFixture;
        use crate::pane_lock::*;
        use std::time::Instant;

        #[test]
        fn an_uncontended_lock_is_acquired_at_once() {
            let fixture = LockFixture::new();
            let started = Instant::now();
            let guard = PaneLock::acquire_within(
                &fixture.settings.database_url,
                fixture.placement(),
                LockFixture::writer_deadline(),
            )
            .unwrap();
            assert!(guard.is_some());
            assert!(started.elapsed() < Duration::from_millis(200));
        }

        #[test]
        fn a_lock_released_within_the_wait_is_acquired() {
            let fixture = LockFixture::new();
            let holder = fixture.hold_for(Duration::from_millis(300));
            let started = Instant::now();
            let guard = PaneLock::acquire_within(
                &fixture.settings.database_url,
                fixture.placement(),
                LockFixture::writer_deadline(),
            )
            .unwrap();
            assert!(guard.is_some());
            assert!(started.elapsed() >= Duration::from_millis(250));
            assert!(started.elapsed() < Duration::from_millis(1000));
            holder.join().unwrap();
        }

        #[test]
        fn a_lock_still_busy_at_the_reserved_deadline_returns_none() {
            let fixture = LockFixture::new();
            let _held = fixture.hold();
            let started = Instant::now();
            let guard = PaneLock::acquire_within(
                &fixture.settings.database_url,
                fixture.placement(),
                LockFixture::writer_deadline(),
            )
            .unwrap();
            assert!(guard.is_none());
            let waited = started.elapsed();
            assert!(waited >= Duration::from_millis(1400), "{waited:?}");
            assert!(waited < Duration::from_millis(2000), "{waited:?}");
        }

        #[test]
        fn physical_lock_identity_uses_canonical_database_backend_session_and_pane() {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;
            use std::os::unix::fs::symlink;
            let fixture = LockFixture::new();
            let url = &fixture.settings.database_url;
            let placement = fixture.placement();
            let alias = fixture._dir.path().join("alias.db");
            symlink(crate::db::database_path(url).unwrap(), &alias).unwrap();
            let alias_url = format!("sqlite:///{}", alias.display());
            let path = lock_path(url, placement).unwrap();
            assert_eq!(lock_path(&alias_url, placement).unwrap(), path);
            let mut duplicate = placement.clone();
            duplicate.mux_window_id = "@99".into();
            duplicate.coding_agent = "codex".into();
            assert_eq!(lock_path(url, &duplicate).unwrap(), path);
            for alternative in [
                Placement {
                    backend: "herdr".into(),
                    ..placement.clone()
                },
                Placement {
                    mux_session: "other".into(),
                    ..placement.clone()
                },
                Placement {
                    mux_pane_id: Some("%99".into()),
                    ..placement.clone()
                },
            ] {
                assert_ne!(lock_path(url, &alternative).unwrap(), path);
                assert!(PaneLock::try_acquire(url, &alternative).unwrap().is_some());
            }
            let guard = PaneLock::try_acquire(url, placement).unwrap().unwrap();
            let metadata = std::fs::metadata(&path).unwrap();
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            std::fs::write(&path, "operator marker").unwrap();
            assert!(
                PaneLock::try_acquire(&alias_url, &duplicate)
                    .unwrap()
                    .is_none()
            );
            drop(guard);
            let _guard = PaneLock::acquire_released(&alias_url, &duplicate)
                .unwrap()
                .unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), metadata.ino());
            assert_eq!(std::fs::read(&path).unwrap(), b"operator marker");
        }
    }

    mod deletion_locks {
        use crate::broker::Placement;
        use crate::error::CafleetError;
        use crate::pane_lock::PaneLock;
        use crate::pane_lock::lock_deletion;
        use crate::pane_lock::lock_path;
        use crate::pane_lock::test_support::LockFixture;
        use crate::pane_lock::tests::*;
        use crate::time::Deadline;
        use std::time::Duration;
        use std::time::Instant;

        /// The fixture's Director and member placements, in physical lock-key order.
        fn director_and_member_in_key_order(fixture: &LockFixture) -> Vec<(i64, Placement)> {
            let fleet = fixture.member.fleet_id;
            let director = crate::broker::get_fleet(&fixture.conn, fleet)
                .unwrap()
                .unwrap()
                .director_member_id;
            let director_placement = crate::broker::get_member(&fixture.conn, director, fleet)
                .unwrap()
                .unwrap()
                .placement;
            let mut targets = vec![
                (director, director_placement),
                (fixture.member.member_id, fixture.placement().clone()),
            ];
            targets.sort_by_key(|(_, placement)| {
                lock_path(&fixture.settings.database_url, placement).unwrap()
            });
            targets
        }

        #[test]
        fn deletion_deduplicates_physical_panes_and_revalidates_after_all_locks() {
            let mut fixture = LockFixture::new();
            let placement = fixture.placement().clone();
            let member = fixture.member.member_id;
            let locked = lock_deletion(
                &mut fixture.conn,
                &fixture.settings,
                Deadline::new(),
                |_| {
                    Ok((
                        (),
                        vec![(member, placement.clone()), (member + 1, placement.clone())],
                    ))
                },
                |_, _, _| {
                    assert!(
                        PaneLock::try_acquire(&fixture.settings.database_url, &placement)
                            .unwrap()
                            .is_none()
                    );
                    Ok(Some(()))
                },
                |_| CafleetError::App("duplicate pane remained locked".into()),
            )
            .unwrap();
            assert!(
                PaneLock::try_acquire(&fixture.settings.database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            drop(locked);
            assert!(
                PaneLock::acquire_released(&fixture.settings.database_url, &placement)
                    .unwrap()
                    .is_some()
            );
        }

        #[test]
        fn deletion_acquires_physical_keys_in_sorted_order() {
            let fixture = LockFixture::new();
            let targets = director_and_member_in_key_order(&fixture);
            let high_guard = PaneLock::try_acquire(&fixture.settings.database_url, &targets[1].1)
                .unwrap()
                .unwrap();
            let low = targets[0].1.clone();
            let settings = fixture.settings.clone();
            let path = fixture.conn.path().unwrap().to_owned();
            let (loaded, wait_loaded) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                let mut conn = Connection::open(path).unwrap();
                let locked = lock_deletion(
                    &mut conn,
                    &settings,
                    Deadline::until(Instant::now() + Duration::from_secs(2)),
                    |_| {
                        loaded.send(()).unwrap();
                        Ok(((), targets.iter().rev().cloned().collect()))
                    },
                    |_, _, _| Ok(Some(())),
                    |_| CafleetError::App("sorted lock wait expired".into()),
                )
                .unwrap();
                drop(locked);
            });
            wait_loaded.recv_timeout(Duration::from_secs(1)).unwrap();
            let limit = Instant::now() + Duration::from_millis(700);
            loop {
                if PaneLock::try_acquire(&fixture.settings.database_url, &low)
                    .unwrap()
                    .is_none()
                {
                    break;
                }
                assert!(
                    Instant::now() < limit,
                    "the lower key is owned while the higher key waits"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(high_guard);
            thread.join().unwrap();
        }
    }
}
