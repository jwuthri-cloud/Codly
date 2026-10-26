//! SQLite plumbing of the index: connections, schema, and the write operations
//! (replace a file's rows, remove a file). Reads live in query.rs.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};

use super::parse::ParsedFile;

/// Bump when the table layout or what goes into the terms changes: the index is a cache,
/// so an old one is dropped and rebuilt.
const SCHEMA_VERSION: &str = "3";

#[derive(Debug, Clone, Copy)]
pub struct IndexOptions {
    /// Wrap the tokenizer in `porter` (English stemming: `parsing` finds `parse`).
    pub porter: bool,
    /// Prefix every chunk with the terms of its file path, so a chunk of `guardian/transport.rs`
    /// is also found by "guardian transport" when its own lines never say so.
    pub path_in_chunks: bool,
    /// Prefix every chunk with the names of the symbols that enclose its first line.
    pub symbols_in_chunks: bool,
}

impl Default for IndexOptions {
    fn default() -> Self {
        IndexOptions { porter: true, path_in_chunks: true, symbols_in_chunks: true }
    }
}

impl IndexOptions {
    /// Stored in the DB: a change rebuilds the index.
    fn fingerprint(&self) -> String {
        format!("porter={} path={} sym={}", self.porter as u8, self.path_in_chunks as u8, self.symbols_in_chunks as u8)
    }
}

fn tokenizer(o: &IndexOptions) -> &'static str {
    if o.porter { "porter unicode61 remove_diacritics 2" } else { "unicode61 remove_diacritics 2" }
}

pub(crate) fn open_writer(db_path: &Path, opts: &IndexOptions) -> anyhow::Result<Connection> {
    if let Some(dir) = db_path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let conn = Connection::open(db_path).with_context(|| format!("open {}", db_path.display()))?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -65536)?;
    init(&conn, opts)?;
    Ok(conn)
}

pub(crate) fn open_reader(db_path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open(db_path).with_context(|| format!("open {}", db_path.display()))?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "query_only", true)?;
    conn.pragma_update(None, "cache_size", -32768)?;
    Ok(conn)
}

fn init(conn: &Connection, opts: &IndexOptions) -> anyhow::Result<()> {
    let want_tok = tokenizer(opts);
    let want_opts = opts.fingerprint();
    let have: Option<(String, String)> = conn
        .query_row("SELECT (SELECT value FROM meta WHERE key='schema'), (SELECT value FROM meta WHERE key='options')", [], |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })
        .optional()
        .unwrap_or(None);
    let current = matches!(&have, Some((s, o)) if s == SCHEMA_VERSION && *o == want_opts);
    if !current {
        // Fresh file, older layout, or a different tokenizer: start over.
        conn.execute_batch(
            "DROP TABLE IF EXISTS fts_files; DROP TABLE IF EXISTS fts_chunks; DROP TABLE IF EXISTS chunks;
             DROP TABLE IF EXISTS symbols; DROP TABLE IF EXISTS skipped; DROP TABLE IF EXISTS files;
             DROP TABLE IF EXISTS meta;",
        )?;
    }
    conn.execute_batch(&include_str!("schema.sql").replace("{TOKENIZE}", want_tok))?;
    if !current {
        conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('schema', ?1), ('options', ?2)", params![SCHEMA_VERSION, want_opts])?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct FileState {
    pub mtime_ns: i64,
    pub bytes: i64,
    pub hash: i64,
}

/// path → stored state, for the files table and the skipped table (mtime, bytes).
pub(crate) type StoredState = (HashMap<String, FileState>, HashMap<String, (i64, i64)>);

pub(crate) fn load_state(conn: &Connection) -> anyhow::Result<StoredState> {
    let mut files = HashMap::new();
    let mut stmt = conn.prepare("SELECT path, id, mtime_ns, bytes, hash FROM files")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, FileState { mtime_ns: r.get(2)?, bytes: r.get(3)?, hash: r.get(4)? })))? {
        let (p, s) = row?;
        files.insert(p, s);
    }
    let mut skipped = HashMap::new();
    let mut stmt = conn.prepare("SELECT path, mtime_ns, bytes FROM skipped")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))))? {
        let (p, s) = row?;
        skipped.insert(p, s);
    }
    Ok((files, skipped))
}

/// What the writer is asked to do with one path.
pub(crate) enum Outcome {
    Indexed(Box<ParsedFile>),
    /// Content identical to what is stored; only mtime moved.
    Touched {
        path: String,
        mtime_ns: i64,
    },
    Skipped {
        path: String,
        mtime_ns: i64,
        bytes: i64,
        reason: &'static str,
    },
    Gone(String),
}

pub(crate) fn apply(tx: &Connection, outcome: Outcome) -> anyhow::Result<()> {
    match outcome {
        Outcome::Indexed(f) => {
            tx.prepare_cached("DELETE FROM skipped WHERE path = ?1")?.execute([&f.path])?;
            replace_file(tx, &f)
        }
        Outcome::Touched { path, mtime_ns } => {
            tx.prepare_cached("UPDATE files SET mtime_ns = ?2 WHERE path = ?1")?.execute(params![path, mtime_ns])?;
            Ok(())
        }
        Outcome::Skipped { path, mtime_ns, bytes, reason } => {
            // A file that used to be indexed and is now rejected (became minified, binary…).
            remove_file(tx, &path)?;
            tx.prepare_cached("INSERT OR REPLACE INTO skipped(path, mtime_ns, bytes, reason) VALUES (?1, ?2, ?3, ?4)")?
                .execute(params![path, mtime_ns, bytes, reason])?;
            Ok(())
        }
        Outcome::Gone(path) => {
            remove_file(tx, &path)?;
            tx.prepare_cached("DELETE FROM skipped WHERE path = ?1")?.execute([&path])?;
            Ok(())
        }
    }
}

fn delete_rows(tx: &Connection, id: i64) -> anyhow::Result<()> {
    tx.prepare_cached("DELETE FROM fts_chunks WHERE rowid IN (SELECT id FROM chunks WHERE file_id = ?1)")?.execute([id])?;
    tx.prepare_cached("DELETE FROM chunks WHERE file_id = ?1")?.execute([id])?;
    tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")?.execute([id])?;
    tx.prepare_cached("DELETE FROM fts_files WHERE rowid = ?1")?.execute([id])?;
    Ok(())
}

pub(crate) fn remove_file(tx: &Connection, path: &str) -> anyhow::Result<()> {
    let id: Option<i64> = tx.prepare_cached("SELECT id FROM files WHERE path = ?1")?.query_row([path], |r| r.get(0)).optional()?;
    if let Some(id) = id {
        delete_rows(tx, id)?;
        tx.prepare_cached("DELETE FROM files WHERE id = ?1")?.execute([id])?;
    }
    Ok(())
}

fn replace_file(tx: &Connection, f: &ParsedFile) -> anyhow::Result<()> {
    let existing: Option<i64> = tx.prepare_cached("SELECT id FROM files WHERE path = ?1")?.query_row([&f.path], |r| r.get(0)).optional()?;
    let id = match existing {
        Some(id) => {
            delete_rows(tx, id)?;
            tx.prepare_cached("UPDATE files SET lang = ?2, bytes = ?3, lines = ?4, mtime_ns = ?5, hash = ?6 WHERE id = ?1")?
                .execute(params![id, f.lang, f.bytes as i64, f.lines, f.mtime_ns, f.hash])?;
            id
        }
        None => {
            tx.prepare_cached("INSERT INTO files(path, lang, bytes, lines, mtime_ns, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?
                .execute(params![f.path, f.lang, f.bytes as i64, f.lines, f.mtime_ns, f.hash])?;
            tx.last_insert_rowid()
        }
    };
    tx.prepare_cached("INSERT INTO fts_files(rowid, path_terms, symbol_terms, doc_terms) VALUES (?1, ?2, ?3, ?4)")?.execute(params![
        id,
        f.path_terms,
        f.symbol_terms,
        f.doc_terms
    ])?;
    {
        let mut ins =
            tx.prepare_cached("INSERT INTO symbols(file_id, name, kind, container, line, end_line) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for s in &f.symbols {
            ins.execute(params![id, s.name, s.kind, s.container, s.line, s.end_line])?;
        }
    }
    {
        let mut ins_chunk = tx.prepare_cached("INSERT INTO chunks(file_id, start_line, end_line) VALUES (?1, ?2, ?3)")?;
        let mut ins_fts = tx.prepare_cached("INSERT INTO fts_chunks(rowid, body_terms) VALUES (?1, ?2)")?;
        for c in &f.chunks {
            ins_chunk.execute(params![id, c.start_line, c.end_line])?;
            ins_fts.execute(params![tx.last_insert_rowid(), c.terms])?;
        }
    }
    Ok(())
}
