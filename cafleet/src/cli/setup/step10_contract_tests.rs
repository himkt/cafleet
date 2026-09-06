//! Regression tests for the real setup loop and installer adapters.
use super::*;
use crate::assets::{
    self, Edge, InstallEvent, InstallFault, InstallHooks, InstallOperation, LockMode,
};
use rusqlite::{Connection, params};
use std::cell::Cell;
use std::fs;

struct Fixture {
    dir: tempfile::TempDir,
    conn: Connection,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix(".step10-setup-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = Connection::open(dir.path().join("assets.sqlite3")).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        Self { dir, conn }
    }
    fn home(&self) -> std::path::PathBuf {
        self.dir.path().canonicalize().unwrap()
    }
    fn seed(&self, agent: &str) -> assets::AgentPaths {
        let paths = assets::agent_paths(&|_| None, &self.home(), agent).unwrap();
        for skill in ["cafleet", "cafleet-design-doc", "cafleet-research"] {
            let path = paths.skills_dir.join(skill);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("old"), format!("old {agent}")).unwrap();
        }
        if let Some((_, target)) = &paths.preset {
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"old preset").unwrap();
        }
        self.conn
            .execute(
                "INSERT INTO asset_installs VALUES (?1,?2,'old','old exact timestamp')",
                params![agent, paths.identity],
            )
            .unwrap();
        paths
    }
    fn run(
        &mut self,
        selected: &[String],
        checkpoint: &dyn Fn(&InstallEvent) -> Result<(), InstallFault>,
    ) -> (Result<(), CafleetError>, String, String) {
        let home = self.home();
        let lookup = |_: &str| None;
        let hooks = InstallHooks {
            lock_mode: LockMode::Wait,
            checkpoint,
        };
        let options = SetupAssetsOptions {
            home: &home,
            env: &lookup,
            install_hooks: &hooks,
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result =
            assets_half_with_options(&mut self.conn, selected, &options, &mut out, &mut err);
        (
            result,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }
}
fn okay(_: &InstallEvent) -> Result<(), InstallFault> {
    Ok(())
}
fn installed(agent: &str, paths: &assets::AgentPaths, version: &str) -> String {
    let mut text = format!(
        "{agent}: installed cafleet, cafleet-design-doc (v{version}) -> {}\n",
        paths.skills_dir.display()
    );
    if let Some((_, target)) = &paths.preset {
        text.push_str(&format!(
            "{agent}: installed preset (v{version}) -> {}\n",
            target.display()
        ));
    }
    text
}

#[test]
fn real_committed_cleanup_failure_warns_and_continues_installing_later_backends() {
    let mut fixture = Fixture::new();
    let claude = fixture.seed("claude");
    let codex = fixture.seed("codex");
    let opencode = fixture.seed("opencode");
    let hit = Cell::new(false);
    let (result, out, err) = fixture.run(&[], &|e| {
        if e.operation == InstallOperation::CleanupBackup
            && e.edge == Edge::Before
            && !hit.replace(true)
        {
            return Err(InstallFault::Fail("injected cleanup failure".into()));
        }
        Ok(())
    });
    result.unwrap();
    assert!(hit.get());
    let version = super::super::VERSION;
    assert_eq!(
        out,
        format!(
            "{}{}{}",
            installed("claude", &claude, version),
            installed("codex", &codex, version),
            installed("opencode", &opencode, version)
        )
    );
    let recovery = assets::inspect_install(&claude).unwrap().unwrap();
    let prefix = format!(
        "warning: assets installed at {}; cleanup pending: ",
        claude.identity
    );
    let cause = err
        .strip_prefix(&prefix)
        .unwrap()
        .strip_suffix("; run 'cafleet setup' to recover\n")
        .unwrap();
    assert!(cause.contains("injected cleanup failure"));
    assert_eq!(err.lines().count(), 1);
    let journal = assets::read_journal(&recovery.journal).unwrap();
    assert_eq!(journal.phase, assets::InstallPhase::Committed);
    assert!(journal.entries.iter().any(|entry| entry.backup.exists()));
    let rows = crate::broker::list_asset_installs(&fixture.conn).unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(row["cafleet_version"], version);
        assert_ne!(row["installed_at"], "old exact timestamp");
    }
    for paths in [&codex, &opencode] {
        assert!(assets::inspect_install(paths).unwrap().is_none());
        assert!(paths.skills_dir.join("cafleet/SKILL.md").is_file());
    }
}

#[test]
fn ordinary_failure_stops_real_loop_preserves_prior_backend_and_restores_current_backend() {
    let mut fixture = Fixture::new();
    let claude = fixture.seed("claude");
    let codex = fixture.seed("codex");
    let opencode = fixture.seed("opencode");
    let hit = Cell::new(false);
    let (result, out, err) = fixture.run(&[], &|e| {
        if e.operation == InstallOperation::InstallRename
            && e.edge == Edge::After
            && e.path
                .as_ref()
                .is_some_and(|path| path.starts_with(&codex.skills_dir))
            && !hit.replace(true)
        {
            return Err(InstallFault::Fail("second backend failed".into()));
        }
        Ok(())
    });
    assert!(hit.get());
    assert!(
        result
            .unwrap_err()
            .message()
            .contains("second backend failed")
    );
    assert!(err.is_empty());
    assert!(out.starts_with(&installed("claude", &claude, super::super::VERSION)));
    assert!(!out.contains("opencode: installed"));
    let rows = crate::broker::list_asset_installs(&fixture.conn).unwrap();
    assert_eq!(rows[0]["cafleet_version"], super::super::VERSION);
    for row in &rows[1..] {
        assert_eq!(row["cafleet_version"], "old");
        assert_eq!(row["installed_at"], "old exact timestamp");
    }
    assert_eq!(
        fs::read(codex.skills_dir.join("cafleet/old")).unwrap(),
        b"old codex"
    );
    assert_eq!(
        fs::read(opencode.skills_dir.join("cafleet/old")).unwrap(),
        b"old opencode"
    );
    assert!(assets::inspect_install(&codex).unwrap().is_none());
}

#[test]
fn cleanup_only_adapter_prints_journal_version_without_claiming_current_binary_install() {
    let mut fixture = Fixture::new();
    let paths = fixture.seed("codex");
    let hit = Cell::new(false);
    let hooks = InstallHooks {
        lock_mode: LockMode::Wait,
        checkpoint: &|e| {
            if e.operation == InstallOperation::CleanupBackup
                && e.edge == Edge::Before
                && !hit.replace(true)
            {
                return Err(InstallFault::Fail("cleanup pending".into()));
            }
            Ok(())
        },
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    assets::install_agent_with_hooks(
        &mut fixture.conn,
        "codex",
        &paths,
        "journal-version",
        &hooks,
        &mut out,
        &mut err,
    )
    .unwrap();
    assert!(hit.get());
    assert!(!err.is_empty());
    let before = crate::broker::list_asset_installs(&fixture.conn).unwrap();
    out.clear();
    err.clear();
    assets::install_agent_with_hooks(
        &mut fixture.conn,
        "codex",
        &paths,
        "new-binary-version",
        &InstallHooks {
            lock_mode: LockMode::Wait,
            checkpoint: &okay,
        },
        &mut out,
        &mut err,
    )
    .unwrap();
    assert!(err.is_empty());
    assert_eq!(
        String::from_utf8(out).unwrap(),
        installed("codex", &paths, "journal-version")
    );
    assert_eq!(
        crate::broker::list_asset_installs(&fixture.conn).unwrap(),
        before
    );
    assert!(assets::inspect_install(&paths).unwrap().is_none());
}
