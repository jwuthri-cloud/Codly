//! Code retrieval for the workspace: a per-repo SQLite FTS5 index over paths, symbols and
//! 60-line chunks, fused with RRF, with agreement-based confidence and pointer citations.
//! See docs/research/context-engineering.md §4.
//!
//! CONTRACT: the public API below is fixed (the orchestrator and the MCP tools call it).
//!
//! Layout: `walk` finds files (`ignore`), `parse` turns one into symbols + chunks +
//! terms (`terms` splits identifiers, `symbols` extracts definitions, `lang` filters),
//! `store` owns the schema and writes, `query` ranks, `pack` renders the prompt block.

mod lang;
mod pack;
mod parse;
mod query;
mod store;
mod symbols;
pub(crate) mod terms;
mod walk;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension};

use crate::model::SearchResult;

pub use query::{Features, Tuning};
pub use store::IndexOptions;

use store::{FileState, Outcome};

/// Files written per transaction during a refresh.
const BATCH: usize = 200;

/// Per-repo code index. One `Index` covers one checkout.
// ponytail: agents work in git worktrees; the spec's "delta overlay per worktree" (base ∪
// overlay − tombstones) is not built. An `Index` per worktree root works today and costs a
// second cold index; the overlay is the upgrade once that hurts.
///
/// `Send + Sync`: reads and writes use separate connections (WAL), so a
/// search never waits for a refresh in progress; it sees the batches committed so far.
pub struct Index {
    /// Canonical repo root (what the walker yields paths under).
    root: PathBuf,
    /// The root as the caller spelled it (`/var/…` for the canonical `/private/var/…`):
    /// watchers and git report paths either way.
    raw_root: PathBuf,
    opts: IndexOptions,
    writer: Mutex<Connection>,
    // ponytail: one reader connection serializes concurrent searches; a small pool is the
    // upgrade if the MCP server ever fans out many agents at once.
    reader: Mutex<Connection>,
    tuning: Mutex<Tuning>,
}

impl Index {
    /// Open (or create) the index DB for `repo_root` at `db_path`
    /// (`<data>/index/<workspace_id>.sqlite`).
    pub fn open(repo_root: &Path, db_path: &Path) -> anyhow::Result<Index> {
        Self::open_with(repo_root, db_path, &IndexOptions::default())
    }

    /// `open` with explicit options (tokenizer). Changing them rebuilds the index.
    #[doc(hidden)]
    pub fn open_with(repo_root: &Path, db_path: &Path, opts: &IndexOptions) -> anyhow::Result<Index> {
        let root = repo_root.canonicalize().with_context(|| format!("repo root {}", repo_root.display()))?;
        let writer = store::open_writer(db_path, opts)?;
        let reader = store::open_reader(db_path)?;
        Ok(Index {
            root,
            raw_root: repo_root.to_path_buf(),
            opts: *opts,
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            tuning: Mutex::new(Tuning::default()),
        })
    }

    /// Incremental (re)index: walk with `ignore`, skip unchanged files (mtime+size), parse,
    /// chunk, extract symbols, write in batched transactions. Blocking; call from
    /// `spawn_blocking`. Returns the number of files (re)indexed.
    pub fn refresh(&self) -> anyhow::Result<usize> {
        let mut conn = self.writer.lock().map_err(|_| anyhow::anyhow!("index writer poisoned"))?;
        let (known, skipped) = store::load_state(&conn)?;
        let found = walk::walk(&self.root, None);
        let present: HashSet<&str> = found.iter().map(|f| f.rel.as_str()).collect();
        let todo: Vec<&walk::Found> = found
            .iter()
            .filter(|f| {
                let same = |m: i64, b: i64| m == f.mtime_ns && b == f.bytes as i64;
                match known.get(&f.rel) {
                    Some(s) => !same(s.mtime_ns, s.bytes),
                    None => !skipped.get(&f.rel).is_some_and(|(m, b)| same(*m, *b)),
                }
            })
            .collect();
        let gone: Vec<Outcome> =
            known.keys().chain(skipped.keys()).filter(|p| !present.contains(p.as_str())).map(|p| Outcome::Gone(p.clone())).collect();
        let cold = known.is_empty();
        let n = self.process_and_write(&mut conn, &todo, &known, gone)?;
        if n > 0 {
            if cold && n >= 1000 {
                // One big merge after a bulk load makes later queries read a single segment.
                conn.execute_batch(
                    "INSERT INTO fts_files(fts_files) VALUES('optimize'); INSERT INTO fts_chunks(fts_chunks) VALUES('optimize');",
                )?;
            }
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA optimize;");
        }
        Ok(n)
    }

    /// Re-index specific changed paths (from the fs watcher or a commit). Blocking.
    pub fn update_paths(&self, paths: &[PathBuf]) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut conn = self.writer.lock().map_err(|_| anyhow::anyhow!("index writer poisoned"))?;
        let mut ign = walk::IgnoreCheck::new(&self.root);
        let mut outcomes: Vec<Outcome> = Vec::new();
        let mut files: Vec<walk::Found> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for p in paths {
            let Some(abs) = self.absolutize(p) else { continue };
            let Some(rel) = walk::rel_string(&self.root, &abs) else { continue };
            match std::fs::metadata(&abs) {
                Ok(m) if m.is_dir() => {
                    if ign.is_ignored(&abs, true) {
                        continue;
                    }
                    let found = walk::walk(&self.root, Some(&abs));
                    let now: HashSet<&str> = found.iter().map(|f| f.rel.as_str()).collect();
                    for old in paths_under(&conn, &rel)? {
                        if !now.contains(old.as_str()) {
                            outcomes.push(Outcome::Gone(old));
                        }
                    }
                    for f in found {
                        if seen.insert(f.rel.clone()) {
                            files.push(f);
                        }
                    }
                }
                Ok(m) if m.is_file() => {
                    if ign.is_ignored(&abs, false) || m.len() > walk::MAX_FILE_BYTES {
                        outcomes.push(Outcome::Gone(rel));
                    } else if seen.insert(rel.clone()) {
                        files.push(walk::Found { rel, abs, mtime_ns: walk::mtime_ns(&m), bytes: m.len() });
                    }
                }
                Ok(_) => {}
                // Deleted (or renamed away): the path may have been a file or a directory.
                Err(_) => {
                    for old in paths_under(&conn, &rel)? {
                        outcomes.push(Outcome::Gone(old));
                    }
                }
            }
        }
        let known = known_for(&conn, &files)?;
        let todo: Vec<&walk::Found> = files.iter().collect();
        self.process_and_write(&mut conn, &todo, &known, outcomes)?;
        Ok(())
    }

    /// Parse `todo` on worker threads and write the results (plus `extra`, e.g. deletions)
    /// from the calling thread in batched transactions. Returns how many files were indexed.
    fn process_and_write(
        &self,
        conn: &mut Connection,
        todo: &[&walk::Found],
        known: &HashMap<String, FileState>,
        extra: Vec<Outcome>,
    ) -> anyhow::Result<usize> {
        let mut indexed = 0usize;
        if !extra.is_empty() {
            write_batch(conn, extra)?;
        }
        if todo.is_empty() {
            return Ok(0);
        }
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(todo.len());
        let next = AtomicUsize::new(0);
        let (tx, rx) = mpsc::sync_channel::<Outcome>(workers * 16);
        let mut result: anyhow::Result<()> = Ok(());
        std::thread::scope(|s| {
            for _ in 0..workers {
                let tx = tx.clone();
                let next = &next;
                s.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(f) = todo.get(i) else { break };
                        if tx.send(process(f, known.get(&f.rel), &self.opts)).is_err() {
                            break; // the writer stopped (error): nothing left to do
                        }
                    }
                });
            }
            drop(tx);
            let mut batch: Vec<Outcome> = Vec::with_capacity(BATCH);
            for outcome in rx.iter() {
                if matches!(outcome, Outcome::Indexed(_)) {
                    indexed += 1;
                }
                batch.push(outcome);
                if batch.len() >= BATCH
                    && let Err(e) = write_batch(conn, std::mem::take(&mut batch))
                {
                    result = Err(e);
                    return; // dropping `rx` stops the workers
                }
            }
            if !batch.is_empty() {
                result = write_batch(conn, batch);
            }
        });
        result?;
        Ok(indexed)
    }

    /// Hybrid search. `k` = max citations.
    pub fn search(&self, query: &str, k: usize) -> anyhow::Result<SearchResult> {
        Ok(self.ranked(query, k)?.result)
    }

    /// ≤ ~1.2k-token pointer pack for a task prompt: one line per citation
    /// (`[C1] path:12-70 symbol — why (confidence)`), or a short "search yourself with:
    /// rg …" hint when confidence is low. Returns (pack text, the result it came from).
    pub fn pointer_pack(&self, query: &str) -> anyhow::Result<(String, SearchResult)> {
        let ranked = self.ranked(query, 10)?;
        let text = pack::render(&ranked);
        Ok((text, ranked.result))
    }

    /// Every indexed file with its size, for the 3D codebase city.
    pub fn files(&self) -> anyhow::Result<Vec<crate::model::RepoFile>> {
        let conn = self.reader.lock().map_err(|_| anyhow::anyhow!("index reader poisoned"))?;
        let mut st = conn.prepare("SELECT path, lines, lang FROM files ORDER BY path")?;
        let rows = st.query_map([], |r| Ok(crate::model::RepoFile { path: r.get(0)?, lines: r.get(1)?, lang: r.get(2)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Number of indexed files (for the UI).
    pub fn file_count(&self) -> usize {
        let Ok(conn) = self.reader.lock() else { return 0 };
        conn.query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0)).map_or(0, |n| n as usize)
    }

    /// `search` plus the signals confidence was computed from (benchmark).
    #[doc(hidden)]
    pub fn search_explain(&self, query: &str, k: usize) -> anyhow::Result<(SearchResult, Features)> {
        let r = self.ranked(query, k)?;
        Ok((r.result, r.features))
    }

    /// Is this repo-relative path in the index? (benchmark and tests)
    #[doc(hidden)]
    pub fn is_indexed(&self, rel_path: &str) -> bool {
        let Ok(conn) = self.reader.lock() else { return false };
        conn.query_row("SELECT 1 FROM files WHERE path = ?1", [rel_path], |_| Ok(())).optional().ok().flatten().is_some()
    }

    /// Replace the ranking constants (benchmark sweeps).
    #[doc(hidden)]
    pub fn set_tuning(&self, t: Tuning) {
        if let Ok(mut g) = self.tuning.lock() {
            *g = t;
        }
    }

    /// `p` (absolute or repo-relative, under either spelling of the root, possibly already
    /// deleted) as an absolute path under the canonical root; None when it is outside the repo.
    fn absolutize(&self, p: &Path) -> Option<PathBuf> {
        let abs = normalize(&if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) });
        if abs.starts_with(&self.root) {
            return Some(abs);
        }
        if let Ok(rest) = abs.strip_prefix(&self.raw_root) {
            return Some(self.root.join(rest));
        }
        // Canonicalize the longest ancestor that still exists, then put the tail back.
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let mut cur = abs.as_path();
        loop {
            if let Ok(c) = cur.canonicalize() {
                let full = tail.iter().rev().fold(c, |a, t| a.join(t));
                return full.starts_with(&self.root).then_some(full);
            }
            match (cur.parent(), cur.file_name()) {
                (Some(parent), Some(name)) => {
                    tail.push(name.to_os_string());
                    cur = parent;
                }
                _ => return None,
            }
        }
    }

    fn ranked(&self, query: &str, k: usize) -> anyhow::Result<query::Ranked> {
        let tuning = self.tuning.lock().map_err(|_| anyhow::anyhow!("index tuning poisoned"))?.clone();
        let conn = self.reader.lock().map_err(|_| anyhow::anyhow!("index reader poisoned"))?;
        query::search(&conn, query, k, &tuning)
    }
}

/// Read one file and decide what the index should do with it.
fn process(f: &walk::Found, prev: Option<&FileState>, opts: &IndexOptions) -> Outcome {
    let bytes = match std::fs::read(&f.abs) {
        Ok(b) => b,
        Err(_) => return Outcome::Gone(f.rel.clone()), // vanished since the walk
    };
    let len = bytes.len() as i64;
    if let Some(p) = prev
        && p.bytes == len
        && p.hash == parse::fnv1a(&bytes)
    {
        return Outcome::Touched { path: f.rel.clone(), mtime_ns: f.mtime_ns };
    }
    match parse::parse(&f.rel, &bytes, f.mtime_ns, opts) {
        parse::Parsed::Indexed(p) => Outcome::Indexed(p),
        parse::Parsed::Skipped(reason) => Outcome::Skipped { path: f.rel.clone(), mtime_ns: f.mtime_ns, bytes: len, reason },
    }
}

fn write_batch(conn: &mut Connection, batch: Vec<Outcome>) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    for o in batch {
        store::apply(&tx, o)?;
    }
    tx.commit()?;
    Ok(())
}

/// Stored paths equal to `rel` or below it (`rel/…`): what a deleted directory held.
fn paths_under(conn: &Connection, rel: &str) -> anyhow::Result<Vec<String>> {
    let esc: String = rel.chars().flat_map(|c| if matches!(c, '%' | '_' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
    let mut stmt = conn.prepare_cached("SELECT path FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\' UNION SELECT path FROM skipped WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'")?;
    let rows = stmt.query_map(rusqlite::params![rel, format!("{esc}/%")], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn known_for(conn: &Connection, files: &[walk::Found]) -> anyhow::Result<HashMap<String, FileState>> {
    let mut out = HashMap::new();
    let mut stmt = conn.prepare_cached("SELECT mtime_ns, bytes, hash FROM files WHERE path = ?1")?;
    for f in files {
        if let Some(s) =
            stmt.query_row([&f.rel], |r| Ok(FileState { mtime_ns: r.get(0)?, bytes: r.get(1)?, hash: r.get(2)? })).optional()?
        {
            out.insert(f.rel.clone(), s);
        }
    }
    Ok(out)
}

/// Lexical `.`/`..` removal (the path may not exist any more, so no `canonicalize`).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Index>();
    }

    #[test]
    fn normalizes_dotdot() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
    }
}
