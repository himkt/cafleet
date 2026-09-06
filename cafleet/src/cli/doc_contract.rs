//! Maintenance commands for the bounded generated documentation blocks.
//! Only `generate` writes; ordinary tests and builds never update documents.
use std::any::TypeId;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use clap::CommandFactory;
use rusqlite::Connection;

fn cli_table(command: &clap::Command) -> String {
    let mut out = String::from(
        "| Argument | Type | Values per occurrence | Action | Parser default | Required |\n\
         |---|---|---|---|---|---|\n",
    );
    for arg in command.get_arguments().filter(|arg| {
        !matches!(
            arg.get_action(),
            clap::ArgAction::Help | clap::ArgAction::Version
        )
    }) {
        let name = arg
            .get_long()
            .map(|long| format!("--{long}"))
            .unwrap_or_else(|| {
                arg.get_value_names().expect("positional value name")[0].to_string()
            });
        let id = arg.get_value_parser().type_id();
        let kind = if id == TypeId::of::<i64>() {
            "i64"
        } else if id == TypeId::of::<bool>() {
            "bool"
        } else {
            panic!("unsupported documentation argument type: {name}")
        };
        let defaults = arg
            .get_default_values()
            .iter()
            .map(|v| v.to_str().expect("UTF-8 parser default"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "| `{name}` | `{kind}` | {} | `{:?}` | {} | {} |",
            arg.get_num_args().expect("built command value count"),
            arg.get_action(),
            if defaults.is_empty() {
                "—".to_string()
            } else {
                format!("`{defaults}`")
            },
            if arg.is_required_set() { "yes" } else { "no" }
        )
        .unwrap();
    }
    out.push_str("\nParser defaults only; runtime environment fallbacks and value constraints remain in the prose.\n");
    if command.is_subcommand_negates_reqs_set() {
        out.push_str(
            "Required arguments apply to the loop form; selecting a subcommand negates them.\n",
        );
    }
    out
}

fn schema_table(conn: &Connection, name: &str) -> String {
    let mut out = String::from(
        "| Column | SQLite type | DDL NOT NULL | DDL default | PK position |\n\
         |---|---|---|---|---|\n",
    );
    let mut statement = conn.prepare(
        "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid"
    ).unwrap();
    let rows = statement
        .query_map([name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (name, kind, not_null, default, pk) = row.unwrap();
        writeln!(
            out,
            "| `{name}` | `{kind}` | {not_null} | {} | {pk} |",
            default
                .map(|v| format!("`{v}`"))
                .unwrap_or_else(|| "—".into())
        )
        .unwrap();
    }
    out.push_str("\nDDL metadata only: PK position is separate from the NOT NULL flag; foreign keys and runtime semantics remain in the prose.\n");
    out
}

fn indexes(conn: &Connection) -> String {
    let mut out = String::from(
        "| Table | Index | Unique | Columns | Partial | Definition |\n|---|---|---|---|---|---|\n",
    );
    let mut statement = conn
        .prepare(
            "SELECT t.name, i.name, i.\"unique\", i.partial, s.sql
         FROM sqlite_schema t JOIN pragma_index_list(t.name) i
         JOIN sqlite_schema s ON s.name=i.name AND s.type='index'
         WHERE t.type='table' AND t.name NOT LIKE 'sqlite_%'
           AND t.name != 'refinery_schema_history' AND i.origin='c'
         ORDER BY t.name, i.name",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (table, name, unique, partial, sql) = row.unwrap();
        let mut columns = conn
            .prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
            .unwrap();
        let columns = columns
            .query_map([&name], |row| row.get::<_, Option<String>>(0))
            .unwrap()
            .map(|v| {
                v.unwrap()
                    .unwrap_or_else(|| "(expression; see definition)".into())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sql = sql
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('|', "&#124;");
        writeln!(
            out,
            "| `{table}` | `{name}` | {unique} | `{columns}` | {partial} | `{sql}` |"
        )
        .unwrap();
    }
    out.push_str("\nApplication-created indexes only; SQLite autoindexes for PRIMARY KEY/UNIQUE constraints and migration bookkeeping are excluded.\n");
    out
}

fn update(generate: bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixture = tempfile::Builder::new()
        .prefix(".doc-contract-")
        .tempdir_in(root)
        .unwrap();
    let mut conn = crate::db::connect(&format!(
        "sqlite:///{}",
        fixture.path().join("schema.db").display()
    ))
    .unwrap();
    crate::db::migrate_to_head(&mut conn).unwrap();
    let mut cli = super::CliArgs::command();
    cli.build();
    let monitor = cli.find_subcommand("monitor").unwrap();
    let capture = cli
        .find_subcommand("member")
        .unwrap()
        .find_subcommand("capture")
        .unwrap();
    let blocks = [
        ("cli-monitor", cli_table(monitor)),
        (
            "cli-monitor-scan",
            cli_table(monitor.find_subcommand("scan").unwrap()),
        ),
        ("cli-member-capture", cli_table(capture)),
        (
            "schema-monitor-runtime",
            schema_table(&conn, "monitor_runtime"),
        ),
        (
            "schema-asset-installs",
            schema_table(&conn, "asset_installs"),
        ),
        ("schema-indexes", indexes(&conn)),
    ];
    // Validate every destination before writing any file.
    let mut updates = BTreeMap::new();
    for (file, count) in [("SPEC.md", 6), ("docs/docs/spec/cli-options.md", 3)] {
        let path = root.join(file);
        let original = std::fs::read_to_string(&path).unwrap();
        let mut rendered = original.clone();
        assert_eq!(
            original.matches("<!-- BEGIN GENERATED ").count(),
            count,
            "{file}: unexpected start markers"
        );
        assert_eq!(
            original.matches("<!-- END GENERATED ").count(),
            count,
            "{file}: unexpected end markers"
        );
        for (key, content) in blocks.iter().take(count) {
            let begin = format!("<!-- BEGIN GENERATED {key} -->");
            let end = format!("<!-- END GENERATED {key} -->");
            assert_eq!(
                rendered.matches(&begin).count(),
                1,
                "{file}: missing/duplicate {begin}"
            );
            assert_eq!(
                rendered.matches(&end).count(),
                1,
                "{file}: missing/duplicate {end}"
            );
            let start = rendered.find(&begin).unwrap() + begin.len();
            let finish = rendered.find(&end).unwrap();
            assert!(start < finish, "{file}: reversed markers for {key}");
            assert!(
                !rendered[start..finish].contains("<!-- BEGIN GENERATED "),
                "{file}: nested markers"
            );
            rendered.replace_range(start..finish, &format!("\n{content}"));
        }
        updates.insert(path, (original, rendered));
    }
    for (path, (original, rendered)) in updates {
        if generate {
            if original != rendered {
                std::fs::write(&path, rendered).unwrap();
            }
        } else {
            assert_eq!(
                original,
                rendered,
                "{}: generated documentation drift; run mise //cafleet:docs-generate",
                path.display()
            );
        }
    }
}

#[test]
fn check() {
    update(false);
}

#[test]
#[ignore = "explicit maintenance write: mise //cafleet:docs-generate"]
fn generate() {
    update(true);
}
