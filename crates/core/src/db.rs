//! SQLite storage: connection setup and schema migrations.
//!
//! Migrations are embedded with `include_str!` so a built binary always carries the
//! schema it expects — no sidecar `.sql` files to lose during packaging, and no chance
//! of the app and its schema drifting apart.
//!
//! Applied versions are recorded in `schema_migrations`, so a database from an older
//! build is upgraded in place rather than recreated. This matters more than usual here:
//! the whole point of the application is that the user's data outlives the tool.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Ordered schema versions. Append only; never edit an already-released entry, because
/// existing databases have already run it and will not run it again.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "init", include_str!("../migrations/0001_init.sql")),
    (
        2,
        "editable_dates",
        include_str!("../migrations/0002_editable_dates.sql"),
    ),
    (
        3,
        "file_versions_ignored_pending_series",
        include_str!("../migrations/0003_file_versions_ignored_pending_series.sql"),
    ),
];

/// Latest schema version this build expects.
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|(v, _, _)| *v).unwrap_or(0)
}

/// Opens (creating if needed) the library database, tuning it for a desktop app and
/// bringing the schema up to date.
pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let conn = Connection::open(path)?;
    prepare(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// An in-memory database, for tests.
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    prepare(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Applies the pragmas the app relies on.
///
///  * `foreign_keys` is OFF by default in SQLite. The schema leans on `ON DELETE
///    CASCADE` for every child table, so forgetting this would leave orphaned files,
///    tags and relations behind when a work is deleted.
///  * WAL keeps reads from blocking writes, so the UI can refresh while an import runs.
fn prepare(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Runs any migrations newer than the database's current version.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )?;

    let applied: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    for (version, name, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        // Each migration is atomic: a failure halfway through must not leave a
        // partially-applied schema recorded as done.
        conn.execute_batch("BEGIN")?;
        match conn.execute_batch(sql) {
            Ok(()) => {
                conn.execute(
                    "INSERT INTO schema_migrations (version, name, applied_at)
                     VALUES (?1, ?2, datetime('now'))",
                    rusqlite::params![version, name],
                )?;
                conn.execute_batch("COMMIT")?;
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(Error::Database(e));
            }
        }
    }
    Ok(())
}

/// Current schema version of a database, or 0 when it has never been migrated.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?)
}

/// Default database location inside an application data directory.
pub fn default_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("bookmarks.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(std::result::Result::unwrap).collect()
    }

    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
        rows.map(std::result::Result::unwrap).collect()
    }

    #[test]
    fn migration_creates_every_table() {
        let conn = open_in_memory().unwrap();
        let tables = table_names(&conn);
        for expected in [
            "works",
            "files",
            "tags",
            "work_tags",
            "work_relations",
            "pending_relations",
            "schema_migrations",
        ] {
            assert!(
                tables.contains(&expected.to_string()),
                "missing table {expected}: {tables:?}"
            );
        }
    }

    #[test]
    fn foreign_keys_pragma_is_on() {
        let conn = open_in_memory().unwrap();
        let on: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            on, 1,
            "cascades are load-bearing; foreign_keys must be enabled"
        );
    }

    #[test]
    fn works_table_has_the_planned_columns() {
        let conn = open_in_memory().unwrap();
        let cols = columns(&conn, "works");
        for expected in [
            "title",
            "author",
            "summary",
            "my_comment",
            "published_at",
            "published_prec",
            "date_is_approx",
            "date_source",
            "completed_at",
            "word_count",
            "word_count_source",
            "language",
            "language_label",
            "translation_role",
            "source_url",
            "external_id",
            "ao3_tags",
            "status",
            "created_at",
            "updated_at",
        ] {
            assert!(
                cols.contains(&expected.to_string()),
                "missing column {expected}: {cols:?}"
            );
        }
    }

    #[test]
    fn schema_version_is_recorded_and_idempotent() {
        let conn = open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), latest_version());
        // Re-running must be a no-op, not an error.
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), latest_version());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count as usize,
            MIGRATIONS.len(),
            "each migration recorded once"
        );
    }

    #[test]
    fn deleting_a_work_cascades_to_children() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO works (id, title, status, created_at, updated_at)
             VALUES (1, 'T', 'has_file', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO files (work_id, format_class, format_detail, original_name, rel_path, imported_at)
             VALUES (1, 'html', 'html', 'a.html', '000001/work.html', 'now')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (10, 'fluff')", [])
            .unwrap();
        conn.execute("INSERT INTO work_tags (work_id, tag_id) VALUES (1, 10)", [])
            .unwrap();

        conn.execute("DELETE FROM works WHERE id = 1", []).unwrap();

        for table in ["files", "work_tags"] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table} rows must cascade away with the work");
        }
        // The tag itself is shared vocabulary and must survive.
        let tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            tags, 1,
            "tags are shared across works and must not be deleted with one"
        );
    }

    #[test]
    fn tag_names_are_unique_case_insensitively() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO tags (name) VALUES ('Fluff')", [])
            .unwrap();
        let dup = conn.execute("INSERT INTO tags (name) VALUES ('fluff')", []);
        assert!(
            dup.is_err(),
            "'Fluff' and 'fluff' must not coexist as separate tags"
        );
        // A genuinely different tag is still fine.
        conn.execute("INSERT INTO tags (name) VALUES ('Angst')", [])
            .unwrap();
    }

    #[test]
    fn external_id_is_unique_but_allows_multiple_nulls() {
        let conn = open_in_memory().unwrap();
        let ins = |id: i64, ext: Option<&str>| {
            conn.execute(
                "INSERT INTO works (id, title, external_id, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'has_file', 'now', 'now')",
                rusqlite::params![id, format!("T{id}"), ext],
            )
        };
        ins(1, Some("7929115")).unwrap();
        assert!(
            ins(2, Some("7929115")).is_err(),
            "the same AO3 work must not import twice"
        );
        ins(3, None).unwrap();
        ins(4, None).unwrap();
        assert!(
            ins(5, None).is_ok(),
            "record-only works have no external id"
        );
    }

    #[test]
    fn relations_are_symmetric_and_reject_self_or_reverse_duplicates() {
        let conn = open_in_memory().unwrap();
        for id in 1..=3 {
            conn.execute(
                "INSERT INTO works (id, title, status, created_at, updated_at)
                 VALUES (?1, ?2, 'has_file', 'now', 'now')",
                rusqlite::params![id, format!("T{id}")],
            )
            .unwrap();
        }
        // Stored smaller-id-first.
        conn.execute(
            "INSERT INTO work_relations (work_a, work_b, kind, source) VALUES (1, 2, 'translation', 'ao3_html')",
            [],
        )
        .unwrap();
        // The reverse order is rejected, which is exactly what prevents a duplicate
        // relation that only one lookup direction would find.
        assert!(conn
            .execute(
                "INSERT INTO work_relations (work_a, work_b, kind, source) VALUES (2, 1, 'translation', 'ao3_html')",
                [],
            )
            .is_err());
        // Self-relation is nonsense and rejected.
        assert!(conn
            .execute(
                "INSERT INTO work_relations (work_a, work_b, kind, source) VALUES (3, 3, 'translation', 'ao3_html')",
                [],
            )
            .is_err());
    }

    #[test]
    fn format_class_is_constrained_to_the_three_classes() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO works (id, title, status, created_at, updated_at)
             VALUES (1, 'T', 'has_file', 'now', 'now')",
            [],
        )
        .unwrap();
        // 'epub' is not a valid class — it belongs to 'other' with a format_detail.
        let bad = conn.execute(
            "INSERT INTO files (work_id, format_class, format_detail, original_name, rel_path, imported_at)
             VALUES (1, 'epub', 'epub', 'a.epub', 'p.epub', 'now')",
            [],
        );
        assert!(bad.is_err(), "only pdf|html|other are valid format classes");
    }

    #[test]
    fn migration_is_recorded_atomically() {
        let conn = open_in_memory().unwrap();
        let (version, name, _) = MIGRATIONS[0];
        let row: (String,) = conn
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = ?1",
                [version],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert_eq!(row.0, name);
    }

    #[test]
    fn every_migration_is_applied_in_order_on_a_fresh_database() {
        let conn = open_in_memory().unwrap();
        let recorded: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT version FROM schema_migrations ORDER BY version")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).unwrap();
            rows.map(std::result::Result::unwrap).collect()
        };
        let expected: Vec<i64> = MIGRATIONS.iter().map(|(v, _, _)| *v).collect();
        assert_eq!(recorded, expected, "all migrations recorded, in order");
        assert_eq!(
            schema_version(&conn).unwrap(),
            expected.last().copied().unwrap()
        );
    }

    /// Upgrading an existing library is the path that matters most: re-running from scratch
    /// is easy, but a user's database already holds their data and must survive.
    #[test]
    fn migrating_an_existing_v1_library_preserves_its_data() {
        let dir = std::env::temp_dir().join("bookmarks-migration-tests");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");

        // Build a database that looks like one created by the previous release: schema v1
        // only, with a work already in it.
        {
            let conn = Connection::open(&path).unwrap();
            prepare(&conn).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                     version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);",
            )
            .unwrap();
            conn.execute_batch(MIGRATIONS[0].2).unwrap();
            conn.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'init', 'then')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO works (id, title, author, word_count, status, created_at, updated_at)
                 VALUES (7, 'Existing work', 'Someone', 1234, 'has_file', 'then', 'then')",
                [],
            )
            .unwrap();
            assert_eq!(schema_version(&conn).unwrap(), 1);
        }

        // Opening the library must upgrade it in place.
        let conn = db_open(&path);
        assert_eq!(
            schema_version(&conn).unwrap(),
            latest_version(),
            "the database must be brought up to date"
        );

        let (title, words): (String, i64) = conn
            .query_row(
                "SELECT title, word_count FROM works WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the existing row must survive the upgrade");
        assert_eq!(title, "Existing work");
        assert_eq!(words, 1234);

        // The new columns exist and default sensibly for rows that predate them.
        let (prec, approx): (Option<String>, i64) = conn
            .query_row(
                "SELECT completed_prec, completed_is_approx FROM works WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(
            prec.is_none(),
            "a work that predates the column has no completion date"
        );
        assert_eq!(approx, 0);

        // Upgrading again must be a no-op rather than an error.
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), latest_version());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opens a database the way the application does.
    fn db_open(path: &std::path::Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        prepare(&conn).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn completed_date_fields_are_present() {
        let conn = open_in_memory().unwrap();
        let cols = columns(&conn, "works");
        for expected in ["completed_prec", "completed_is_approx", "completed_source"] {
            assert!(
                cols.contains(&expected.to_string()),
                "missing column {expected}: {cols:?}"
            );
        }
    }
}
