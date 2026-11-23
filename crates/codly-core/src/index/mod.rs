//! Code retrieval for the workspace: a per-repo SQLite FTS5 index over paths, symbols and
//! 60-line chunks, fused with RRF, with agreement-based confidence and pointer citations.
//! See docs/research/context-engineering.md §4.
//!
//! CONTRACT: the public API below is fixed (the orchestrator and the MCP tools call it).
//!
//! Layout: `walk` finds files (`ignore`), `parse` turns one into symbols + chunks +
//! terms (`terms` splits identifiers, `symbols` extracts definitions, `lang` filters),
//! `store` owns the schema and writes, `query` ranks, `pack` renders the prompt block.
//! `fresh` knows how far the index is behind the files, and `worktree` what differs in the
//! worktree an agent asks from: both end up as `SearchResult::warnings`.

mod fresh;
mod lang;
mod pack;
mod parse;
mod query;
mod store;
mod symbols;
pub(crate) mod terms;
mod walk;
mod worktree;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension};

use crate::model::SearchResult;

pub use query::{Features, Tuning};
pub use store::IndexOptions;
pub use worktree::WorktreeView;

use fresh::Fresh;
use store::{FileState, Outcome};

/// Files written per transaction during a refresh.
const BATCH: usize = 200;

/// Per-repo code index. One `Index` covers one checkout.
// ponytail: agents work in git worktrees; the spec's "delta overlay per worktree" (base ∪
// overlay − tombstones) is not built. A search asked from a worktree says which files differ
// there (`WorktreeView`) and does not search them; the overlay is the upgrade.
///
/// `Send + Sync`: reads and writes use separate connections (WAL), so a
/// search never waits for a refresh in progress; it sees the batches committed so far, and
/// its result says that the index is behind (`SearchResult::warnings`).
pub struct Index {
    /// Canonical repo root (what the walker yields paths under).
    root: PathBuf,
    /// The root as the caller spelled it (`/var/…` for the canonical `/private/var/…`):
    /// callers and git report paths either way.
    raw_root: PathBuf,
    opts: IndexOptions,
    writer: Mutex<Connection>,
    // ponytail: one reader connection serializes concurrent searches; a small pool is the
    // upgrade if the MCP server ever fans out many agents at once.
    reader: Mutex<Connection>,
    tuning: Mutex<Tuning>,
    fresh: Mutex<Fresh>,
    /// Signalled when a refresh ends.
    refreshed: Condvar,
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
        let fresh = Fresh { refreshed_at: store::refreshed_at(&writer)?, ..Fresh::default() };
        Ok(Index {
            root,
            raw_root: repo_root.to_path_buf(),
            opts: *opts,
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            tuning: Mutex::new(Tuning::default()),
            fresh: Mutex::new(fresh),
            refreshed: Condvar::new(),
        })
    }

    /// The canonical root of the checkout this index covers.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Is `dir` inside the checkout this index covers (so its files are the indexed ones)?
    pub fn covers(&self, dir: &Path) -> bool {
        dir.canonicalize().is_ok_and(|d| d.starts_with(&self.root))
    }

    /// Incremental (re)index: walk with `ignore`, skip unchanged files (mtime+size), parse,
    /// chunk, extract symbols, write in batched transactions. Blocking; call from
    /// `spawn_blocking`. Returns the number of files (re)indexed.
    pub fn refresh(&self) -> anyhow::Result<usize> {
        claim_refresh(&mut self.fresh_state());
        self.refresh_claimed()
    }

    /// `refresh`, unless one ended less than `every` ago: the walk reads no file content, but
    /// it visits every file, so a search asks for it at most that often. When a refresh is in
    /// progress, wait for it instead of starting another.
    ///
    /// `until` is when the asking search stops waiting; take it before a thread is started for
    /// this call. A refresh that is still in progress then is a long one: from then on search
    /// results say that the index is being updated, and later searches do not wait for it.
    /// Blocking: a refresh that this call starts runs to its end, also after `until`. If the
    /// index is still behind afterwards (a long first build, an error), the search results
    /// say so.
    pub fn refresh_if_due(&self, every: Duration, until: Instant) -> anyhow::Result<()> {
        let mut state = self.fresh_state();
        if state.running > 0 {
            // Nobody waits longer than the first search that asked.
            let late_from = state.late_from.map_or(until, |at| at.min(until));
            state.late_from = Some(late_from);
            let left = late_from.saturating_duration_since(Instant::now());
            drop(self.refreshed.wait_timeout_while(state, left, |s| s.running > 0));
            return Ok(());
        }
        if state.checked.is_some_and(|at| at.elapsed() < every) {
            return Ok(());
        }
        claim_refresh(&mut state);
        state.late_from = Some(until);
        drop(state);
        self.refresh_claimed().map(|_| ())
    }

    fn fresh_state(&self) -> MutexGuard<'_, Fresh> {
        // Plain data: a panic elsewhere leaves it usable.
        self.fresh.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Run a refresh that `claim_refresh` counted, and record how it ended (also on a panic).
    fn refresh_claimed(&self) -> anyhow::Result<usize> {
        struct Claim<'a> {
            index: &'a Index,
            /// When the files were read, or why they were not; None = the refresh panicked.
            outcome: Option<Result<i64, String>>,
        }
        impl Drop for Claim<'_> {
            fn drop(&mut self) {
                let mut state = self.index.fresh_state();
                state.running -= 1;
                if state.running == 0 {
                    state.late_from = None;
                }
                state.checked = Some(Instant::now());
                match self.outcome.take() {
                    Some(Ok(at)) => {
                        state.refreshed_at = Some(at);
                        state.error = None;
                    }
                    Some(Err(e)) => state.error = Some(e),
                    None => state.error = Some("the refresh stopped on an internal error".to_string()),
                }
                drop(state);
                self.index.refreshed.notify_all();
            }
        }
        let mut claim = Claim { index: self, outcome: None };
        let result = self.read_files();
        claim.outcome = Some(match &result {
            Ok((_, at)) => Ok(*at),
            Err(e) => Err(format!("{e:#}")),
        });
        result.map(|(n, _)| n)
    }

    /// The refresh itself. Returns the number of files (re)indexed and when it started: the
    /// index then holds the files as they were at that time.
    fn read_files(&self) -> anyhow::Result<(usize, i64)> {
        let at = crate::model::now_ms();
        // A checkout that is not there (an unmounted disk, a moved folder) is an error, not
        // a repository whose files were all deleted.
        if !self.root.is_dir() {
            anyhow::bail!("{} is not there", self.root.display());
        }
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
        store::set_refreshed_at(&conn, at)?;
        Ok((n, at))
    }

    /// Re-index specific changed paths (the files a merge touched). Blocking.
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
        Ok(self.answer(query, k, None)?.result)
    }

    /// `search` for an agent that works in a worktree: matches in files the worktree does not
    /// have are left out, matches in files that differ there are marked, and the result names
    /// the files that are new there (they are not searched).
    pub fn search_in(&self, view: &WorktreeView, query: &str, k: usize) -> anyhow::Result<SearchResult> {
        Ok(self.answer(query, k, Some(view))?.result)
    }

    /// ≤ ~1.2k-token pointer pack for a task prompt: one line per citation
    /// (`[C1] path:12-70 symbol — why (confidence)`), or a short "search yourself with:
    /// rg …" hint when confidence is low. Returns (pack text, the result it came from).
    pub fn pointer_pack(&self, query: &str) -> anyhow::Result<(String, SearchResult)> {
        let ranked = self.answer(query, 10, None)?;
        let text = pack::render(&ranked);
        Ok((text, ranked.result))
    }

    /// `pointer_pack` for a task that runs in a worktree (see `search_in`).
    pub fn pointer_pack_in(&self, view: &WorktreeView, query: &str) -> anyhow::Result<(String, SearchResult)> {
        let ranked = self.answer(query, 10, Some(view))?;
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

    /// A ranked answer that tells the truth about the files the asker sees: seen from `view`
    /// if the asker works in a worktree, and with a warning when the index is behind the files.
    fn answer(&self, query: &str, k: usize, view: Option<&WorktreeView>) -> anyhow::Result<query::Ranked> {
        let mut ranked = match view {
            Some(view) => {
                let k = k.clamp(1, 60);
                // Twice as many candidates: the ones the worktree does not have are left out.
                let mut ranked = self.ranked(query, 2 * k)?;
                view.apply(self, &mut ranked, k);
                ranked
            }
            None => self.ranked(query, k)?,
        };
        if let Some(behind) = self.fresh_state().warning(crate::model::now_ms()) {
            ranked.result.warnings.insert(0, behind);
        }
        Ok(ranked)
    }

    /// Has the index seen this repo-relative path in the checkout (indexed it, or left it out
    /// for its content)?
    fn knows(&self, rel_path: &str) -> bool {
        let Ok(conn) = self.reader.lock() else { return false };
        conn.prepare_cached("SELECT 1 FROM files WHERE path = ?1 UNION ALL SELECT 1 FROM skipped WHERE path = ?1 LIMIT 1")
            .and_then(|mut st| st.query_row([rel_path], |_| Ok(())).optional())
            .ok()
            .flatten()
            .is_some()
    }
}

/// Count one more refresh in progress. `Index::refresh_claimed` must follow.
fn claim_refresh(state: &mut Fresh) {
    state.running += 1;
}

/// Read one file and decide what the index should do with it.
fn process(f: &walk::Found, prev: Option<&FileState>, opts: &IndexOptions) -> Outcome {
    // Rejected by name (images, lockfiles, bundles): no need to read it.
    if let Some(reason) = lang::skip_by_name(&f.rel) {
        return Outcome::Skipped { path: f.rel.clone(), mtime_ns: f.mtime_ns, bytes: f.bytes as i64, reason };
    }
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

/// Paths equal to ?1 or in the range [?2, ?3). A range uses the path index of both tables; a
/// LIKE prefix cannot (LIKE ignores case, the index does not), so it read every row.
const PATHS_UNDER_SQL: &str = "SELECT path FROM files WHERE path = ?1 OR (path >= ?2 AND path < ?3)
     UNION SELECT path FROM skipped WHERE path = ?1 OR (path >= ?2 AND path < ?3)";

/// Stored paths equal to `rel` or below it (`rel/…`): what a deleted directory held.
fn paths_under(conn: &Connection, rel: &str) -> anyhow::Result<Vec<String>> {
    // Everything below `rel` sorts from `rel/` up to `rel0`: `0` is the byte after `/`.
    let mut stmt = conn.prepare_cached(PATHS_UNDER_SQL)?;
    let rows = stmt.query_map(rusqlite::params![rel, format!("{rel}/"), format!("{rel}0")], |r| r.get::<_, String>(0))?;
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
    fn a_search_waits_for_the_refresh_in_progress_and_says_so_when_it_is_not_done() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join("a.rs"), "pub fn early_symbol() {}\n").unwrap();
        let idx = Index::open(&repo, &d.path().join("i.sqlite")).unwrap();
        idx.refresh().unwrap();
        std::fs::write(repo.join("b.rs"), "pub fn late_symbol() {}\n").unwrap();

        // Another search starts the next comparison with the files. A search that answers
        // meanwhile has nothing to say about it: the index matched the files a moment ago.
        claim_refresh(&mut idx.fresh_state());
        let r = idx.search("early_symbol", 3).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);

        // The comparison takes longer than this search waits for it.
        idx.refresh_if_due(Duration::ZERO, Instant::now() + Duration::from_millis(20)).unwrap();
        assert!(!idx.is_indexed("b.rs"), "no second refresh is started next to the one in progress");
        let r = idx.search("early_symbol", 3).unwrap();
        assert_eq!(r.citations[0].path, "a.rs", "the search answers from what is indexed");
        assert!(r.warnings[0].contains("is being updated") && r.warnings[0].contains("ago"), "{:?}", r.warnings);
        // It is known to be long now: the next search does not wait for it again.
        let asked = Instant::now();
        idx.refresh_if_due(Duration::ZERO, asked + Duration::from_secs(60)).unwrap();
        assert!(asked.elapsed() < Duration::from_secs(30), "waited {:?}", asked.elapsed());

        // It ends: the new file is there, and nothing is left to warn about.
        idx.refresh_claimed().unwrap();
        let r = idx.search("late_symbol", 3).unwrap();
        assert_eq!(r.citations[0].path, "b.rs");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn normalizes_dotdot() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
    }

    #[test]
    fn paths_under_is_an_index_range_and_takes_names_literally() {
        let d = tempfile::tempdir().unwrap();
        let conn = store::open_writer(&d.path().join("i.sqlite"), &IndexOptions::default()).unwrap();
        let plan = crate::store::query_plan(&conn, PATHS_UNDER_SQL);
        assert!(!plan.lines().any(|l| l.starts_with("SCAN")), "{plan}");

        for p in ["a_b", "a_b/x.rs", "a_b/deep/y.rs", "a_b0", "a_b.rs", "axb/z.rs", "A_B/case.rs", "100%/w.rs", "100x/v.rs"] {
            conn.execute("INSERT INTO files(path, bytes, lines, mtime_ns, hash) VALUES (?1, 0, 0, 0, 0)", [p]).unwrap();
        }
        conn.execute("INSERT INTO skipped(path, mtime_ns, bytes, reason) VALUES ('a_b/logo.png', 0, 0, 'binary'), ('axb/logo.png', 0, 0, 'binary')", []).unwrap();
        let under = |rel: &str| {
            let mut got = paths_under(&conn, rel).unwrap();
            got.sort();
            got
        };
        // `_` and `%` are plain characters, and a sibling that only shares the prefix is not below.
        assert_eq!(under("a_b"), ["a_b", "a_b/deep/y.rs", "a_b/logo.png", "a_b/x.rs"]);
        assert_eq!(under("100%"), ["100%/w.rs"]);
        assert!(under("a").is_empty());
    }

    #[test]
    fn a_file_rejected_by_its_name_is_not_read() {
        // The path does not exist: reading it would report the file as gone.
        let f = walk::Found { rel: "assets/logo.png".into(), abs: PathBuf::from("/nonexistent/assets/logo.png"), mtime_ns: 7, bytes: 4096 };
        match process(&f, None, &IndexOptions::default()) {
            Outcome::Skipped { path, mtime_ns, bytes, reason } => {
                assert_eq!((path.as_str(), mtime_ns, bytes, reason), ("assets/logo.png", 7, 4096, "binary"));
            }
            _ => panic!("expected a skip by name"),
        }
    }
}
