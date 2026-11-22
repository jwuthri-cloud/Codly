//! Copies of the main database, and the way back when it cannot be used.
//!
//! Copies live in `<data>/backups/`. Each is a whole SQLite file written with `VACUUM INTO`:
//! a consistent copy, made while the app reads and writes.
//! - `update`: before an update of Codly changes the data (a migration), made by `Store::open`;
//! - `daily`: once a day while the app runs (`Store::backup_if_due`, the background sweep).
//!
//! The last `KEEP_UPDATE` and `KEEP_DAILY` of each kind are kept. An update that fails is tried
//! again on the same data: its copy is made once, not once per attempt.
//!
//! A database that cannot be used makes `Store::open` fail with a `StoreError`. `problem` turns
//! a failed start into what the start-up screen shows, and `recover` moves the file aside
//! (it is never deleted) and puts a copy, or nothing, in its place. A start with no data moves
//! the copies aside too (`backups.before-<time>/`): the copies of the new, empty database
//! never push out the last copies of the old data.

use std::io::{ErrorKind, Read};
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
    #[error(
        "Codly could not update its data for this version ({detail}). The data is unchanged, and the version of Codly you had before still opens it. \
         Starting from a backup would not help: the same update would fail on it."
    )]
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
    /// The copies this version of Codly can start from, newest first. Empty when the update of
    /// the data failed: the same update would fail on a copy.
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

/// `name` is a copy that was not finished: `codly-<unix ms>-v<data version>-<kind>.part`.
fn unfinished(name: &str) -> bool {
    name.strip_suffix(".part").is_some_and(|stem| parse_name(&format!("{stem}.sqlite")).is_some())
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

/// Remove the copies in `dir` that were cut short (the app quit, the disk was full): nothing
/// lists them, and nothing else removes them. One copy is written at a time, so none of them
/// is in progress.
fn remove_unfinished(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        if unfinished(&entry.file_name().to_string_lossy())
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            tracing::warn!("the unfinished backup {} was not removed: {e}", entry.path().display());
        }
    }
    Ok(())
}

/// The two files hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    const CHUNK: u64 = 1 << 16;
    let (mut a, mut b) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut left, mut right) = (vec![], vec![]);
    loop {
        left.clear();
        right.clear();
        a.by_ref().take(CHUNK).read_to_end(&mut left)?;
        b.by_ref().take(CHUNK).read_to_end(&mut right)?;
        if left != right {
            return Ok(false);
        }
        if left.is_empty() {
            return Ok(true);
        }
    }
}

/// The copy from before an update that already holds the bytes of `part`, a new one of the
/// same size: the update failed and is tried again on the same data. `VACUUM INTO` writes the
/// same data to the same bytes.
fn same_copy(dir: &Path, part: &Path, new: &DbBackup) -> Option<DbBackup> {
    list(dir).into_iter().filter(|b| b.kind == BackupKind::Update && b.schema == new.schema && b.bytes == new.bytes).find(|b| {
        same_bytes(part, Path::new(&b.path)).unwrap_or_else(|e| {
            tracing::warn!("the backup {} could not be compared: {e}", b.path);
            false
        })
    })
}

/// Write the database `conn` has open to `part`, then give it its name (`copy.path`). A copy
/// from before an update that is already in `dir` is kept as it is, and returned.
fn finish(conn: &Connection, dir: &Path, part: &Path, mut copy: DbBackup) -> Result<DbBackup> {
    conn.execute("VACUUM INTO ?1", [part.to_string_lossy().as_ref()])?;
    let file = std::fs::File::open(part)?;
    file.sync_all()?;
    copy.bytes = file.metadata()?.len() as i64;
    if copy.kind == BackupKind::Update
        && let Some(same) = same_copy(dir, part, &copy)
    {
        std::fs::remove_file(part)?;
        return Ok(same);
    }
    std::fs::rename(part, &copy.path)?;
    Ok(copy)
}

/// Write a copy of the database `conn` has open into `dir`, then drop the copies of its kind
/// that are past the number kept. A copy that could not be finished leaves no file.
fn write(conn: &Connection, dir: &Path, kind: BackupKind, now: i64) -> Result<DbBackup> {
    std::fs::create_dir_all(dir)?;
    remove_unfinished(dir)?;
    let schema: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let path = dir.join(file_name(now, schema, kind));
    // Written under another name first: a copy that was cut short is never listed.
    let part = path.with_extension("part");
    let copy = DbBackup { path: path.to_string_lossy().into_owned(), kind, created_at: now, schema, bytes: 0 };
    let copy = finish(conn, dir, &part, copy).inspect_err(|_| match std::fs::remove_file(&part) {
        Err(e) if e.kind() != ErrorKind::NotFound => tracing::warn!("the unfinished backup {} was not removed: {e}", part.display()),
        _ => {}
    })?;
    let keep = match kind {
        BackupKind::Daily => KEEP_DAILY,
        BackupKind::Update => KEEP_UPDATE,
    };
    for old in list(dir).into_iter().filter(|b| b.kind == kind).skip(keep) {
        if let Err(e) = std::fs::remove_file(&old.path) {
            tracing::warn!("the old backup {} was not removed: {e}", old.path);
        }
    }
    Ok(copy)
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
    // A new database has nothing to copy. An update that is tried again after it failed finds
    // its copy already there (`finish`).
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
    let found = err.chain().find_map(|cause| cause.downcast_ref::<StoreError>());
    let message = match found {
        Some(e) => e.to_string(),
        // Damage that only a later read found (the settings, the runs to recover).
        None => {
            let sql = err.chain().filter_map(|cause| cause.downcast_ref::<rusqlite::Error>()).find(|e| damaged(e))?;
            StoreError::Damaged { detail: sql.to_string() }.to_string()
        }
    };
    // The database is fine when only its copy before an update could not be made.
    let start_empty = !matches!(found, Some(StoreError::BackupFailed { .. }));
    // The data is fine when its update failed, and a copy holds the same data or older data:
    // the update would fail on it again, and each start from one would leave one more
    // database moved aside. No copy is offered then.
    let from_copy = start_empty && !matches!(found, Some(StoreError::UpdateFailed { .. }));
    let backups = if from_copy { usable(&dir(db)) } else { vec![] };
    let message = if from_copy && backups.is_empty() { format!("{message} There is no backup to start from.") } else { message };
    Some(StartProblem { message, database: db.to_string_lossy().into_owned(), backups, start_empty })
}

/// `<path><suffix>`: the `-wal` and `-shm` files SQLite keeps beside a database.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Move the database file `db` aside and start from `backup`, or with no data (None). The
/// file is kept, with its `-wal` and `-shm`, as `<name>.broken-<now>.sqlite` in the same
/// folder: where it was moved to is returned. A start with no data also moves the copies of
/// the database aside, as `backups.before-<now>/`: they are the last readable copies of the old
/// data, and the copies of the new database must not count against them. Nothing may have
/// `db` open.
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
    // The copies leave the rotation before the database leaves its place: no state has a new
    // database beside the copies of the old one.
    let (copies, kept) = (dir(db), db.with_file_name(format!("{DIR}.before-{now}")));
    if backup.is_none() {
        match std::fs::rename(&copies, &kept) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(anyhow::Error::new(e).context(format!("move {} aside", copies.display()))),
            _ => {}
        }
    }
    // The database first: when that fails, nothing has changed.
    for suffix in ["", "-wal", "-shm"] {
        let (from, to) = (beside(db, suffix), beside(&aside, suffix));
        match std::fs::rename(&from, &to) {
            Err(e) if e.kind() != ErrorKind::NotFound => {
                // The database stays, so its copies go back to where a start looks for them.
                if suffix.is_empty()
                    && backup.is_none()
                    && let Err(back) = std::fs::rename(&kept, &copies)
                    && back.kind() != ErrorKind::NotFound
                {
                    tracing::warn!("the backups stay in {}: {back}", kept.display());
                }
                return Err(anyhow::Error::new(e).context(format!("move {} aside", from.display())));
            }
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
        assert!(shown.message.ends_with("There is no backup to start from."), "{}", shown.message);
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
        // The copy was made before the update ran, at the version the data had.
        let [copy]: [DbBackup; 1] = list(&dir(&db)).try_into().unwrap_or_else(|all| panic!("one copy from before the update: {all:?}"));
        assert_eq!((copy.kind, copy.schema), (BackupKind::Update, 1));
        assert!(copy.bytes > 0 && Path::new(&copy.path).starts_with(tmp.path().join("backups")));
        // It holds the data the update fails on, so it is not offered as a way out.
        let shown = problem(&db, &err).unwrap();
        assert!(shown.message.contains("could not update its data") && shown.message.ends_with("the same update would fail on it."), "{}", shown.message);
        assert_eq!((shown.backups.len(), shown.start_empty), (0, true));
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
    fn an_update_that_fails_again_on_the_same_data_makes_no_second_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        {
            let store = Store::open_with(&db, &v1()).unwrap();
            store.conn().execute("INSERT INTO note (text) VALUES ('kept')", []).unwrap();
            // Two copies from before earlier updates.
            write(&store.conn(), &dir(&db), BackupKind::Update, 1).unwrap();
            store.conn().execute("INSERT INTO note (text) VALUES ('more')", []).unwrap();
            write(&store.conn(), &dir(&db), BackupKind::Update, 2).unwrap();
            store.conn().execute("INSERT INTO note (text) VALUES ('newest')", []).unwrap();
        }
        let copies = || list(&dir(&db)).into_iter().map(|b| b.created_at).collect::<Vec<_>>();

        // "Try again", three times: one copy of the data, and the copy from before the update
        // before this one is still there.
        for _ in 0..3 {
            let err = Store::open_with(&db, &failing_v2()).err().expect("the update fails");
            assert!(matches!(store_error(&err), StoreError::UpdateFailed { .. }), "{err:#}");
        }
        let [first, older]: [i64; 2] = copies().try_into().unwrap_or_else(|all| panic!("the copy of this update and the one before it: {all:?}"));
        assert!(first > 2 && older == 2, "{first} {older}");
        assert_eq!(std::fs::read_dir(dir(&db)).unwrap().count(), 2, "no unfinished copy is left");

        // The data changed (the Codly from before the update ran again): that is a new copy.
        Store::open_with(&db, &v1()).unwrap().conn().execute("INSERT INTO note (text) VALUES ('later')", []).unwrap();
        // Copies are named by the millisecond.
        std::thread::sleep(Duration::from_millis(2));
        assert!(Store::open_with(&db, &failing_v2()).is_err());
        let [second, kept]: [i64; 2] = copies().try_into().unwrap_or_else(|all| panic!("two copies: {all:?}"));
        assert!(second > first && kept == first, "{second} {kept} {first}");
    }

    #[test]
    fn a_copy_that_was_cut_short_leaves_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        let store = Store::open(&db).unwrap();
        let backups = dir(&db);
        std::fs::create_dir_all(&backups).unwrap();
        let names = || {
            let mut names: Vec<String> = std::fs::read_dir(&backups).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        // The app quit while it wrote a copy, at another time than the next one.
        std::fs::write(backups.join("codly-5-v1-daily.part"), b"half a copy").unwrap();
        // Not a file of Codly's: left alone.
        std::fs::write(backups.join("notes.part"), b"mine").unwrap();
        // The next copy cannot get its name: a folder is in its place.
        let in_the_way = file_name(DAY_MS, latest_schema().unwrap(), BackupKind::Daily);
        std::fs::create_dir(backups.join(&in_the_way)).unwrap();

        assert!(store.backup_if_due(DAY_MS).is_err());
        assert_eq!(names(), [in_the_way.clone(), "notes.part".to_string()], "neither the old unfinished copy nor the new one is left");
        assert!(store.backups().is_empty());

        let copy = store.backup_if_due(2 * DAY_MS).unwrap().expect("no copy yet: one is due");
        assert_eq!(names(), [file_name(2 * DAY_MS, copy.schema, BackupKind::Daily), in_the_way, "notes.part".to_string()]);
    }

    #[test]
    fn a_start_with_no_data_keeps_the_copies_of_the_old_data_out_of_the_rotation() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("codly.sqlite");
        let old: Vec<String> = {
            let store = Store::open(&db).unwrap();
            store.kv_set("k", "old data").unwrap();
            for day in 0..KEEP_DAILY as i64 {
                assert!(store.backup_if_due(day * DAY_MS).unwrap().is_some());
            }
            store.backups().iter().map(|b| Path::new(&b.path).file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(old.len(), KEEP_DAILY);
        std::fs::write(&db, b"this is not a database ".repeat(64)).unwrap();

        // A database that cannot be moved keeps its copies where a start finds them.
        std::fs::create_dir_all(tmp.path().join("codly.broken-7.sqlite").join("in the way")).unwrap();
        assert!(recover(&db, None, 7).is_err());
        assert_eq!(list(&dir(&db)).len(), KEEP_DAILY);
        assert!(!tmp.path().join("backups.before-7").exists());

        // "Start with no data": the copies are moved aside with the database.
        let now = 10 * DAY_MS;
        recover(&db, None, now).unwrap();
        let kept = tmp.path().join(format!("backups.before-{now}"));
        assert!(!dir(&db).exists() && kept.is_dir());
        let fresh = Store::open(&db).unwrap();
        assert!(fresh.backups().is_empty());
        // More days than daily copies are kept.
        for day in 11..=11 + KEEP_DAILY as i64 {
            assert!(fresh.backup_if_due(day * DAY_MS).unwrap().is_some());
        }
        assert_eq!(fresh.backups().len(), KEEP_DAILY);
        // Every copy of the old data is still there, and still a database with that data.
        for name in &old {
            let copy = Connection::open(kept.join(name)).unwrap();
            assert_eq!(copy.query_row("SELECT value FROM kv WHERE key = 'k'", [], |r| r.get::<_, String>(0)).unwrap(), "old data");
        }

        // A start from a copy leaves the copies where they are.
        let from = fresh.backups().remove(0);
        drop(fresh);
        recover(&db, Some(Path::new(&from.path)), now + 1).unwrap();
        assert_eq!(list(&dir(&db)).len(), KEEP_DAILY);
        assert!(!tmp.path().join(format!("backups.before-{}", now + 1)).exists());
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
            store.kv_set("k", &format!("before update {at}")).unwrap();
            write(&store.conn(), &dir(&db), BackupKind::Update, at).unwrap();
        }
        let kept: Vec<(BackupKind, i64)> = store.backups().iter().map(|b| (b.kind, b.created_at)).collect();
        assert_eq!(kept, [(BackupKind::Daily, 4 * DAY_MS), (BackupKind::Daily, 3 * DAY_MS), (BackupKind::Daily, 2 * DAY_MS), (BackupKind::Update, 3), (BackupKind::Update, 2)]);
        assert!(store.backups().iter().all(|b| b.bytes > 0));
        let files = std::fs::read_dir(dir(&db)).unwrap().count();
        assert_eq!(files, 5, "nothing else is left in the folder");
    }
}
