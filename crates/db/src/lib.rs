//! SQLite storage. Only `openclaimd` opens the database for writing.

use openclaim_domain::{OcError, OcResult};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};

pub use rusqlite;

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../../migrations/0001_init.sql"))];

pub fn db_err(e: rusqlite::Error) -> OcError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            OcError::conflict(format!("constraint violated: {e}"))
        }
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(f.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) =>
        {
            OcError::conflict("database is busy; retry").with_next_action("retry the command")
        }
        _ => OcError::internal(format!("database error: {e}")),
    }
}

pub trait DbResultExt<T> {
    fn db(self) -> OcResult<T>;
}

impl<T> DbResultExt<T> for Result<T, rusqlite::Error> {
    fn db(self) -> OcResult<T> {
        self.map_err(db_err)
    }
}

pub struct Database {
    pub conn: Connection,
    pub path: PathBuf,
}

impl Database {
    pub fn open(path: &Path) -> OcResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OcError::internal(format!("create data dir: {e}")))?;
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .db()?;
        Self::configure(&conn)?;
        let mut db = Database { conn, path: path.to_path_buf() };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> OcResult<Self> {
        let conn = Connection::open_in_memory().db()?;
        Self::configure(&conn)?;
        let mut db = Database { conn, path: PathBuf::from(":memory:") };
        db.migrate()?;
        Ok(db)
    }

    fn configure(conn: &Connection) -> OcResult<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA trusted_schema = OFF;",
        )
        .db()
    }

    pub fn schema_version(&self) -> OcResult<i64> {
        let exists: bool = self
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='meta'", [], |r| r.get::<_, i64>(0))
            .db()?
            > 0;
        if !exists {
            return Ok(0);
        }
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0))
            .optional()
            .db()?;
        Ok(v.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    fn migrate(&mut self) -> OcResult<()> {
        let current = self.schema_version()?;
        for (version, sql) in MIGRATIONS {
            if *version <= current {
                continue;
            }
            let tx = self.conn.transaction().db()?;
            tx.execute_batch(sql).db()?;
            tx.execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [version.to_string()],
            )
            .db()?;
            tx.commit().db()?;
        }
        Ok(())
    }

    /// `PRAGMA integrity_check` + foreign key check.
    pub fn integrity(&self) -> OcResult<Vec<String>> {
        let mut problems = Vec::new();
        let mut stmt = self.conn.prepare("PRAGMA integrity_check").db()?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).db()?;
        for r in rows {
            let r = r.db()?;
            if r != "ok" {
                problems.push(r);
            }
        }
        let mut stmt = self.conn.prepare("PRAGMA foreign_key_check").db()?;
        let rows = stmt.query_map([], |r| Ok(format!("foreign key violation in {}", r.get::<_, String>(0)?))).db()?;
        for r in rows {
            problems.push(r.db()?);
        }
        Ok(problems)
    }

    /// Consistent online copy of the database.
    pub fn backup_to(&self, dest: &Path) -> OcResult<()> {
        if dest.exists() {
            return Err(OcError::conflict(format!("{} already exists", dest.display())));
        }
        self.conn
            .execute("VACUUM INTO ?1", [dest.to_string_lossy().to_string()])
            .db()?;
        Ok(())
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_has_fts5() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        db.conn
            .execute("INSERT INTO opportunities_fts(opportunity_id, title, administrator, summary, case_number) VALUES ('x','Walmart money transfers','Rust Consulting','refund checks','')", [])
            .unwrap();
        let n: i64 = db.conn.query_row("SELECT count(*) FROM opportunities_fts WHERE opportunities_fts MATCH 'transfer'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        assert!(db.integrity().unwrap().is_empty());
    }

    #[test]
    fn reopen_is_idempotent_and_backup_works() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("oc.sqlite");
        drop(Database::open(&p).unwrap());
        let db = Database::open(&p).unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        let b = dir.path().join("backup.sqlite");
        db.backup_to(&b).unwrap();
        assert!(b.exists());
        assert!(db.backup_to(&b).is_err());
    }
}
