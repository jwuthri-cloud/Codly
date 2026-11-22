//! Copies of the main database, and the way back when it cannot be used.
//!
//! Copies live in `<data>/backups/`. Each is a whole SQLite file written with `VACUUM INTO`:
//! a consistent copy, made while the app reads and writes.
//! - `update`: before an update of Codly changes the data (a migration), made by `Store::open`;
//! - `daily`: once a day while the app runs (`Store::backup_if_due`, the background sweep).
//!
//! The last `KEEP_UPDATE` and `KEEP_DAILY` of each kind are kept.
//!
//! A database that cannot be used makes `Store::open` fail with a `StoreError`. `problem` turns
//! a failed start into what the start-up screen shows, and `recover` moves the file aside
//! (it is never deleted) and puts a copy, or nothing, in its place.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, ErrorCode};
use rusqlite_migration::Migrations;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::Store;

/// The folder of the copies, beside the database.
const DIR: &str = "backups";
const KEEP_DAILY: usize = 3;
const KEEP_UPDATE: usize = 2;
const DAY_MS: i64 = 86_400_000;

/// Why the database cannot be used. The text is what the start-up screen says.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Codly's database is damaged and cannot be read ({detail}).")]
    Damaged { detail: String },
    #[error(
        "Codly's data was saved by a newer version of Codly (data version {found}; this version reads up to {supported}). \
         Install the newer Codly again, or start from a backup made before the update."
    )]
    Newer { found: i64, supported: i64 },
    #[error("Codly could not update its data for this version ({detail}). The data is unchanged.")]
    UpdateFailed { detail: String },
    #[error(
        "Codly could not copy its data before updating it ({detail}). The data is unchanged. \
         Check that the disk has free space, then try again."
    )]
    BackupFailed { detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BackupKind {
    /// Made once a day while Codly runs.
    Daily,
    /// Made before an update of Codly changed the data.
    Update,
}

impl BackupKind {
    fn word(self) -> &'static str {
        match self {
            BackupKind::Daily => "daily",
            BackupKind::Update => "update",
        }
    }
}

/// One copy of the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DbBackup {
    pub path: String,
    pub kind: BackupKind,
    /// When it was made (unix ms).
    pub created_at: i64,
    /// The data version it holds. A Codly that writes an older version cannot read it.
    pub schema: i64,
    pub bytes: i64,
}

/// What the start-up screen shows when the database cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartProblem {
    /// What is wrong, in plain words.
    pub message: String,
    /// The database file ("Open the data folder" shows it in Finder).
    pub database: String,
    /// The copies this version of Codly can start from, newest first.
    pub backups: Vec<DbBackup>,
    /// Starting with no data is a way out. False when the database itself is fine and only
    /// its copy before an update could not be made.
    pub start_empty: bool,
}

fn dir(db: &Path) -> PathBuf {
    db.with_file_name(DIR)
}

/// `codly-<unix ms>-v<data version>-<kind>.sqlite`
fn file_name(created_at: i64, schema: i64, kind: BackupKind) -> String {
    format!("codly-{created_at}-v{schema}-{}.sqlite", kind.word())
}

fn parse_name(name: &str) -> Option<(i64, i64, BackupKind)> {
    let mut parts = name.strip_prefix("codly-")?.strip_suffix(".sqlite")?.split('-');
    let created_at = parts.next()?.parse().ok()?;
    let schema = parts.next()?.strip_prefix('v')?.parse().ok()?;
    let kind = match parts.next()? {
        "daily" => BackupKind::Daily,
        "update" => BackupKind::Update,
        _ => return None,
    };
    parts.next().is_none().then_some((created_at, schema, kind))
}

/// The copies in `dir`, newest first. A file that is not a finished copy is not listed.
fn list(dir: &Path) -> Vec<DbBackup> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<DbBackup> = entries
        .flatten()
        .filter_map(|entry| {
            let (created_at, schema, kind) = parse_name(&entry.file_name().to_string_lossy())?;
            let bytes = entry.metadata().ok().filter(|m| m.is_file())?.len() as i64;
            Some(DbBackup { path: entry.path().to_string_lossy().into_owned(), kind, created_at, schema, bytes })
        })
        .collect();
    found.sort_unstable_by_key(|b| std::cmp::Reverse(b.created_at));
    found
}

/// The data version this Codly writes: the number of its migrations.
fn latest_schema() -> Result<i64> {
    let fresh = Connection::open_in_memory()?;
    Ok(i64::from(super::migrations().pending_migrations(&fresh)?))
}

/// The copies this version of Codly can read. None when the version cannot be told: a copy
/// that cannot be checked is not offered.
fn usable(dir: &Path) -> Vec<DbBackup> {
    match latest_schema() {
        Ok(latest) => list(dir).into_iter().filter(|b| b.schema <= latest).collect(),
        Err(e) => {
            tracing::warn!("the data version of this Codly could not be read: {e}");
            vec![]
        }
    }
}

fn damaged(e: &rusqlite::Error) -> bool {
    matches!(e.sqlite_error_code(), Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase))
}

/// A failed read of the database file at start: damage is named as such.
pub(super) fn read_error(e: rusqlite::Error) -> anyhow::Error {
    if damaged(&e) { StoreError::Damaged { detail: e.to_string() }.into() } else { anyhow::Error::new(e).context("read codly.sqlite") }
}

/// Write a copy of the database `conn` has open into `dir`, then drop the copies of its kind
/// that are past the number kept.
fn write(conn: &Connection, dir: &Path, kind: BackupKind, now: i64) -> Result<DbBackup> {
    std::fs::create_dir_all(dir)?;
    let schema: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let path = dir.join(file_name(now, schema, kind));
    // Written under another name first: a copy that was cut short is never listed.
    let part = path.with_extension("part");
    match std::fs::remove_file(&part) {
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(anyhow::Error::new(e).context(format!("remove {}", part.display()))),
        _ => {}
    }
    conn.execute("VACUUM INTO ?1", [part.to_string_lossy().as_ref()])?;
    let file = std::fs::File::open(&part)?;
    file.sync_all()?;
    let bytes = file.metadata()?.len() as i64;
    std::fs::rename(&part, &path)?;
    let keep = match kind {
        BackupKind::Daily => KEEP_DAILY,
        BackupKind::Update => KEEP_UPDATE,
    };
    for old in list(dir).into_iter().filter(|b| b.kind == kind).skip(keep) {
        if let Err(e) = std::fs::remove_file(&old.path) {
            tracing::warn!("the old backup {} was not removed: {e}", old.path);
        }
    }
    Ok(DbBackup { path: path.to_string_lossy().into_owned(), kind, created_at: now, schema, bytes })
}

/// Bring the database file `db`, open on `conn`, to the data version of `migrations`. One
/// that a newer Codly wrote is refused and left alone. One with data is copied before it is
/// changed, and stays as it was when the change fails.
pub(super) fn update(conn: &mut Connection, db: &Path, migrations: &Migrations<'_>) -> Result<()> {
    // The whole schema is read here, so a file that is not a database, or one with a broken
    // schema, fails now and not at some later query.
    let found: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .and_then(|_| conn.pragma_query_value(None, "user_version", |r| r.get(0)))
        .map_err(read_error)?;
    let pending = i64::from(migrations.pending_migrations(conn).context("read the data version of codly.sqlite")?);
    if pending < 0 {
        return Err(StoreError::Newer { found, supported: found + pending }.into());
    }
    if pending == 0 {
        return Ok(());
    }
    // A new database has nothing to copy.
    if found > 0 {
        write(conn, &dir(db), BackupKind::Update, crate::model::now_ms()).map_err(|e| match e.downcast_ref::<rusqlite::Error>() {
            Some(sql) if damaged(sql) => StoreError::Damaged { detail: sql.to_string() },
            _ => StoreError::BackupFailed { detail: format!("{e:#}") },
        })?;
    }
    migrations.to_latest(conn).map_err(|e| match &e {
        rusqlite_migration::Error::RusqliteError { err, .. } if damaged(err) => StoreError::Damaged { detail: err.to_string() },
        _ => StoreError::UpdateFailed { detail: e.to_string() },
    })?;
    Ok(())
}

impl Store {
    /// Make the daily copy when it is due: no copy of any kind is younger than a day. It is
    /// read on a connection of its own, so the app reads and writes while it is made; it reads
    /// the whole database, so call it off the async threads. None = not due, or a database
    /// in memory.
    pub fn backup_if_due(&self, now: i64) -> Result<Option<DbBackup>> {
        let Some(db) = &self.path else { return Ok(None) };
        let dir = dir(db);
        if list(&dir).first().is_some_and(|newest| now - newest.created_at < DAY_MS) {
            return Ok(None);
        }
        let conn = Connection::open(db)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Some(write(&conn, &dir, BackupKind::Daily, now)?))
    }

    /// The copies of this database, newest first.
    pub fn backups(&self) -> Vec<DbBackup> {
        self.path.as_deref().map(|db| list(&dir(db))).unwrap_or_default()
    }
}

/// What to show when a start failed with `err`. None = the failure is not about the database
/// file `db`.
pub fn problem(db: &Path, err: &anyhow::Error) -> Option<StartProblem> {
    let (message, way_out) = match err.chain().find_map(|cause| cause.downcast_ref::<StoreError>()) {
        Some(e) => (e.to_string(), !matches!(e, StoreError::BackupFailed { .. })),
        // Damage that only a later read found (the settings, the runs to recover).
        None => {
            let sql = err.chain().filter_map(|cause| cause.downcast_ref::<rusqlite::Error>()).find(|e| damaged(e))?;
            (StoreError::Damaged { detail: sql.to_string() }.to_string(), true)
        }
    };
    let backups = if way_out { usable(&dir(db)) } else { vec![] };
    Some(StartProblem { message, database: db.to_string_lossy().into_owned(), backups, start_empty: way_out })
}

/// `<path><suffix>`: the `-wal` and `-shm` files SQLite keeps beside a database.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Move the database file `db` aside and start from `backup`, or with no data (None). The
/// file is kept, with its `-wal` and `-shm`, as `<name>.broken-<now>.sqlite` in the same
/// folder: where it was moved to is returned. Nothing may have `db` open.
pub fn recover(db: &Path, backup: Option<&Path>, now: i64) -> Result<PathBuf> {
    let restored = db.with_extension("restore");
    if let Some(from) = backup {
        // Only a copy that Codly made and that this version can read.
        if !usable(&dir(db)).iter().any(|b| Path::new(&b.path) == from) {
            bail!("{} is not a backup this version of Codly can start from", from.display());
        }
        std::fs::copy(from, &restored).with_context(|| format!("copy {}", from.display()))?;
        std::fs::File::open(&restored)?.sync_all()?;
    }
    let stem = db.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let aside = db.with_file_name(format!("{stem}.broken-{now}.sqlite"));
    // The database first: when that fails, nothing has changed.
    for suffix in ["", "-wal", "-shm"] {
        let (from, to) = (beside(db, suffix), beside(&aside, suffix));
        match std::fs::rename(&from, &to) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(anyhow::Error::new(e).context(format!("move {} aside", from.display()))),
            _ => {}
        }
    }
    if backup.is_some() {
        std::fs::rename(&restored, db).with_context(|| format!("put the backup at {}", db.display()))?;
    }
    Ok(aside)
}

#[cfg(test)]
mod tests {
    use rusqlite_migration::M;

    use super::*;

    /// A stand-in for Codly's migrations: version 1 has one table.
    fn v1() -> Migrations<'static> {
        Migrations::new(vec![M::up("CREATE TABLE note (text TEXT NOT NULL);")])
    }

    /// The update to version 2 fails after it changed a row.
    fn failing_v2() -> Migrations<'static> {
        Migrations::new(vec![M::up("CREATE TABLE note (text TEXT NOT NULL);"), M::up("UPDATE note SET text = 'changed'; CREATE TABLE broken (;")])
    }

    fn notes(store: &Store) -> Vec<String> {
        let conn = store.conn();
        let mut st = conn.prepare("SELECT text FROM note ORDER BY rowid").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    fn store_error(e: &anyhow::Error) -> &StoreError {
        e.downcast_ref::<StoreError>().unwrap_or_else(|| panic!("not a StoreError: {e:#}"))
    }

    #[test]
    fn a_file_that_is_not_a_database_is_named_and_moved_aside_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        let garbage = b"this is not a database ".repeat(64);
        std::fs::write(&db, &garbage).unwrap();

        let err = Store::open(&db).err().expect("a damaged file does not open");
        assert!(matches!(store_error(&err), StoreError::Damaged { .. }), "{err:#}");
        assert_eq!(std::fs::read(&db).unwrap(), garbage, "a failed start leaves the file alone");
        let shown = problem(&db, &err).unwrap();
        assert!(shown.message.starts_with("Codly's database is damaged and cannot be read"), "{}", shown.message);
        assert_eq!((shown.database.as_str(), shown.backups.len(), shown.start_empty), (db.to_str().unwrap(), 0, true));
        // A failure that is not about the database has no such screen.
        assert_eq!(problem(&db, &anyhow::anyhow!("no login shell")), None);

        let aside = recover(&db, None, 1_700_000_000_000).unwrap();
        assert_eq!(aside, tmp.path().join("codly.broken-1700000000000.sqlite"));
        assert_eq!(std::fs::read(&aside).unwrap(), garbage, "the file that could not be read is kept");
        let fresh = Store::open(&db).unwrap();
        assert_eq!(fresh.last_event_id().unwrap(), 0);
    }

    #[test]
    fn damage_found_by_a_later_read_is_a_database_problem_too() {
        let sql = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), Some("database disk image is malformed".into()));
        let err = anyhow::Error::new(sql).context("read the settings");
        let shown = problem(Path::new("/data/codly.sqlite"), &err).unwrap();
        assert!(shown.message.contains("damaged") && shown.start_empty, "{}", shown.message);
        let busy = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None);
        assert_eq!(problem(Path::new("/data/codly.sqlite"), &anyhow::Error::new(busy)), None, "a locked database is not a damaged one");
    }

    #[test]
    fn a_failing_update_leaves_the_data_as_it_was_with_a_copy_from_before_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        {
            let store = Store::open_with(&db, &v1()).unwrap();
            store.conn().execute("INSERT INTO note (text) VALUES ('kept')", []).unwrap();
            assert!(store.backups().is_empty(), "a new database has nothing to copy");
        }

        let err = Store::open_with(&db, &failing_v2()).err().expect("the update fails");
        assert!(matches!(store_error(&err), StoreError::UpdateFailed { .. }), "{err:#}");
        let shown = problem(&db, &err).unwrap();
        assert!(shown.message.contains("could not update its data") && shown.start_empty, "{}", shown.message);
        // The copy was made before the update ran, at the version the data had.
        let [copy] = shown.backups.as_slice() else { panic!("one copy from before the update: {:?}", shown.backups) };
        assert_eq!((copy.kind, copy.schema), (BackupKind::Update, 1));
        assert!(copy.bytes > 0 && Path::new(&copy.path).starts_with(tmp.path().join("backups")));
        // The update changed a row before it failed: none of it stayed.
        assert_eq!(notes(&Store::open_with(&db, &v1()).unwrap()), ["kept"]);

        // "Start from the last backup": the database is moved aside with its side files, and
        // the copy takes its place.
        if !beside(&db, "-wal").exists() {
            std::fs::write(beside(&db, "-wal"), b"").unwrap();
        }
        let aside = recover(&db, Some(Path::new(&copy.path)), 5).unwrap();
        assert!(aside.exists() && beside(&aside, "-wal").exists() && !beside(&db, "-wal").exists());
        assert_eq!(notes(&Store::open_with(&db, &v1()).unwrap()), ["kept"]);
        assert!(Path::new(&copy.path).exists(), "the copy stays in the backups");
        assert!(!db.with_extension("restore").exists());
    }

    #[test]
    fn a_copy_that_cannot_be_made_stops_the_update() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        drop(Store::open_with(&db, &v1()).unwrap());
        // No folder can be made where a file is.
        std::fs::write(tmp.path().join("backups"), b"in the way").unwrap();
        let two = Migrations::new(vec![M::up("CREATE TABLE note (text TEXT NOT NULL);"), M::up("CREATE TABLE more (x);")]);

        let err = Store::open_with(&db, &two).err().expect("no update without a copy");
        assert!(matches!(store_error(&err), StoreError::BackupFailed { .. }), "{err:#}");
        let shown = problem(&db, &err).unwrap();
        assert!(!shown.start_empty && shown.backups.is_empty(), "the database is fine: nothing to recover from");
        let version: i64 = Store::open_with(&db, &v1()).unwrap().conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 1, "the data was not updated");
    }

    #[test]
    fn a_database_from_a_newer_codly_is_refused_and_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        let latest = latest_schema().unwrap();
        {
            let store = Store::open(&db).unwrap();
            store.kv_set("k", "from the newer Codly").unwrap();
            store.conn().pragma_update(None, "user_version", 99).unwrap();
            store.checkpoint();
        }
        let before = std::fs::read(&db).unwrap();

        let err = Store::open(&db).err().expect("a newer database does not open");
        assert!(matches!(store_error(&err), StoreError::Newer { found: 99, supported } if *supported == latest), "{err:#}");
        assert_eq!(std::fs::read(&db).unwrap(), before, "the newer Codly still finds its data");

        // Only a copy this version can read is offered, and only such a copy can be put back.
        let backups = dir(&db);
        std::fs::create_dir_all(&backups).unwrap();
        let (old, newer) = (backups.join(file_name(10, 1, BackupKind::Update)), backups.join(file_name(20, 99, BackupKind::Daily)));
        std::fs::write(&old, b"").unwrap();
        std::fs::write(&newer, b"").unwrap();
        let stray = backups.join("notes.txt");
        std::fs::write(&stray, b"not a copy").unwrap();
        let shown = problem(&db, &err).unwrap();
        assert_eq!(shown.backups.iter().map(|b| b.path.as_str()).collect::<Vec<_>>(), [old.to_str().unwrap()]);
        for refused in [newer.as_path(), stray.as_path(), Path::new("/etc/hosts")] {
            let e = recover(&db, Some(refused), 30).unwrap_err().to_string();
            assert!(e.contains("is not a backup this version of Codly can start from"), "{e}");
        }
        assert_eq!(std::fs::read(&db).unwrap(), before, "a refused recovery moves nothing");
    }

    #[test]
    fn a_copy_is_made_once_a_day_and_the_last_of_each_kind_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        let store = Store::open(&db).unwrap();
        store.kv_set("k", "day 0").unwrap();
        assert_eq!(Store::open_in_memory().unwrap().backup_if_due(0).unwrap(), None, "a database in memory has no file to copy");

        let first = store.backup_if_due(0).unwrap().expect("no copy yet: one is due");
        assert_eq!((first.kind, first.created_at, first.schema), (BackupKind::Daily, 0, latest_schema().unwrap()));
        assert_eq!(store.backup_if_due(DAY_MS - 1).unwrap(), None, "one copy a day");
        // A copy is a whole database of its own: it holds what was written before it was made.
        store.kv_set("k", "after the copy").unwrap();
        let copy = Connection::open(&first.path).unwrap();
        assert_eq!(copy.query_row("SELECT value FROM kv WHERE key = 'k'", [], |r| r.get::<_, String>(0)).unwrap(), "day 0");

        for day in 1..=4 {
            assert!(store.backup_if_due(day * DAY_MS).unwrap().is_some());
        }
        // Copies from before an update are counted apart: daily copies never push them out.
        for at in [1, 2, 3] {
            write(&store.conn(), &dir(&db), BackupKind::Update, at).unwrap();
        }
        let kept: Vec<(BackupKind, i64)> = store.backups().iter().map(|b| (b.kind, b.created_at)).collect();
        assert_eq!(kept, [(BackupKind::Daily, 4 * DAY_MS), (BackupKind::Daily, 3 * DAY_MS), (BackupKind::Daily, 2 * DAY_MS), (BackupKind::Update, 3), (BackupKind::Update, 2)]);
        assert!(store.backups().iter().all(|b| b.bytes > 0));
        let files = std::fs::read_dir(dir(&db)).unwrap().count();
        assert_eq!(files, 5, "nothing else is left in the folder");
    }
}
