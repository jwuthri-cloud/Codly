//! The query pipeline (docs/research/context-engineering.md §4.5–4.7):
//!
//! 1. exact: identifiers and paths in the query are looked up directly → `Exact`;
//! 2. candidates: a files ranker (bm25 over path / symbol / leading-comment terms) and a
//!    chunk ranker (FTS5 AND, relaxed by dropping the commonest term, then OR of the rarest);
//! 3. fuse with RRF at file level, keep each file's best 1–3 chunks;
//! 4. confidence from ranker agreement, citations with mechanical `why` strings.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use rusqlite::{Connection, params};

use crate::model::{Citation, Confidence, SearchResult};

use super::terms::{self, QueryTerms, fts_quote};

/// Every constant that shapes ranking, so the benchmark can sweep them without recompiling.
/// The defaults are what ships.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// Use `NEAR(…, near_distance)` as the strictest chunk level.
    pub near: bool,
    pub near_distance: u32,
    /// Relax the AND chunk query until at least this many chunks matched.
    pub min_hits: usize,
    /// Most terms in one AND (the rarest ones).
    pub max_and_terms: usize,
    /// Terms in the OR chunk query (the rarest ones that occur in the index)…
    pub or_terms: usize,
    /// …as long as their document frequencies (each capped at 3000) add up to at most this:
    /// the cost of an OR query is the length of its postings lists.
    pub or_df_budget: i64,

    pub rrf_k: f64,
    /// A file's chunk score is its best chunk plus this share of its next two (0 = best only).
    pub agg_gamma: f64,
    /// RRF weights of the three candidate lists: files, strict (AND) chunks, OR chunks.
    pub w_files: f64,
    pub w_and: f64,
    pub w_or: f64,
    pub bm25_path: f64,
    pub bm25_symbols: f64,
    pub bm25_doc: f64,
    /// Run an AND query on the files table before the OR one.
    pub files_and_first: bool,
    // ponytail: these are one global fit; the in-product benchmark (examples/bench_localize.rs,
    // context-engineering §6) should re-fit them per repo after the first index.
    /// Confidence: p = sigmoid(conf_b + conf_agree·agree5 + conf_score·ln(1 + best OR bm25)),
    /// the chance that a relevant file is among the top 5. Logistic fit on the commit replay of
    /// four repos (leave-one-repo-out AUC 0.74; `High` held 0.89 of hits, `Medium` 0.80, `Low` 0.46).
    pub conf_b: f64,
    pub conf_agree: f64,
    pub conf_score: f64,
    pub high_p: f64,
    pub medium_p: f64,
    /// Fewer present query terms than this is always `Low` (hit rate 0.06 in the replay).
    pub min_terms: usize,
    /// After this long the chunk stage is skipped (files-only answer).
    pub timeout_ms: u64,
    /// Run the strict (AND/NEAR) chunk levels: evidence for `why`, and a fusion list when
    /// `w_and` > 0. The replay found them neutral for ranking, so they cost little and weigh 0.
    pub ladder: bool,
    /// Look up identifiers and paths named in the query before searching.
    pub exact: bool,
    /// At most this many exact citations lead the list; the hybrid ranking fills the rest.
    pub exact_max: usize,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            near: false,
            near_distance: 20,
            min_hits: 20,
            max_and_terms: 8,
            or_terms: 32,
            or_df_budget: 45_000,
            rrf_k: 8.0,
            agg_gamma: 0.25,
            w_files: 0.3,
            w_and: 0.0,
            w_or: 1.0,
            bm25_path: 2.0,
            bm25_symbols: 3.0,
            bm25_doc: 1.0,
            files_and_first: false,
            conf_b: -5.35,
            conf_agree: 0.31,
            conf_score: 1.8,
            high_p: 0.85,
            medium_p: 0.65,
            min_terms: 3,
            timeout_ms: 300,
            ladder: true,
            exact: true,
            exact_max: 2,
        }
    }
}

impl Tuning {
    /// `key=value` override (benchmark sweeps). Returns false for an unknown key or bad value.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        let f = || value.parse::<f64>().ok();
        let b = || matches!(value, "1" | "true" | "on" | "yes");
        match key {
            "near" => self.near = b(),
            "near_distance" => return f().map(|v| self.near_distance = v as u32).is_some(),
            "min_hits" => return f().map(|v| self.min_hits = v as usize).is_some(),
            "max_and_terms" => return f().map(|v| self.max_and_terms = v as usize).is_some(),
            "or_terms" => return f().map(|v| self.or_terms = v as usize).is_some(),
            "or_df_budget" => return f().map(|v| self.or_df_budget = v as i64).is_some(),
            "rrf_k" => return f().map(|v| self.rrf_k = v).is_some(),
            "agg_gamma" => return f().map(|v| self.agg_gamma = v).is_some(),
            "w_files" => return f().map(|v| self.w_files = v).is_some(),
            "w_and" => return f().map(|v| self.w_and = v).is_some(),
            "w_or" => return f().map(|v| self.w_or = v).is_some(),
            "bm25_path" => return f().map(|v| self.bm25_path = v).is_some(),
            "bm25_symbols" => return f().map(|v| self.bm25_symbols = v).is_some(),
            "bm25_doc" => return f().map(|v| self.bm25_doc = v).is_some(),
            "files_and_first" => self.files_and_first = b(),
            "conf_b" => return f().map(|v| self.conf_b = v).is_some(),
            "conf_agree" => return f().map(|v| self.conf_agree = v).is_some(),
            "conf_score" => return f().map(|v| self.conf_score = v).is_some(),
            "high_p" => return f().map(|v| self.high_p = v).is_some(),
            "medium_p" => return f().map(|v| self.medium_p = v).is_some(),
            "min_terms" => return f().map(|v| self.min_terms = v as usize).is_some(),
            "timeout_ms" => return f().map(|v| self.timeout_ms = v as u64).is_some(),
            "ladder" => self.ladder = b(),
            "exact" => self.exact = b(),
            "exact_max" => return f().map(|v| self.exact_max = v as usize).is_some(),
            _ => return false,
        }
        true
    }
}

/// What the pointer pack needs per citation and the public `Citation` has no field for.
#[derive(Debug, Clone)]
pub(crate) struct CitationMeta {
    /// Symbol kind (`fn`, `struct`…) of `Citation.symbol`.
    pub kind: Option<String>,
    pub level: Confidence,
}

/// The cheap, local signals confidence is computed from (§4.6), exposed so the benchmark can
/// fit thresholds per repo.
#[derive(Debug, Clone, Default)]
pub struct Features {
    /// Query terms after filtering, and how many of them occur in the index at all.
    pub n_terms: usize,
    pub n_rare: usize,
    /// Files shared by the top 3/5/10 of the files ranker and of the chunk rankers.
    pub agree3: usize,
    pub agree5: usize,
    pub agree10: usize,
    /// |bm25| of the best, second and fifth distinct file of the OR chunk ranker.
    pub or_top1: f64,
    pub or_top2: f64,
    pub or_top5: f64,
    /// Most query terms found together in one chunk (AND ladder), and how many chunks the
    /// ladder collected.
    pub and_best_n: usize,
    pub and_hits: usize,
    /// Does the top fused file hold a chunk from the AND ladder, with how many terms?
    pub top_and_n: usize,
    /// Is the top fused file in the top 3 of both the files ranker and the chunk rankers?
    pub top_in_both: bool,
    pub dirs5: usize,
    pub min_df: i64,
    /// Milliseconds spent in: exact lookups, files ranker, term statistics, AND ladder, OR chunks,
    /// citations.
    pub phase_ms: [f64; 6],
}

pub(crate) struct Ranked {
    pub result: SearchResult,
    pub meta: Vec<CitationMeta>,
    pub features: Features,
}

// ───────────────────────────── query parsing ─────────────────────────────

#[derive(Debug, Clone)]
struct Hint {
    name: String,
    qualifier: Option<String>,
    /// Written like code (backticks, `a::b`, `snake_case`, `f()`, or a lone word): matched in
    /// any case and against any kind of symbol. A bare camelCase word may be a brand
    /// (`GitHub`, `macOS`), so it only counts on an exact-case match against real definitions.
    strict: bool,
}

#[derive(Debug, Default)]
struct Parsed {
    terms: QueryTerms,
    idents: Vec<Hint>,
    paths: Vec<String>,
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_alphabetic() || f == '_' || f == '$') && c.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
}

/// camelCase that is unlikely to be a brand: `retainedSampling`, `LunaSamplerConfig`
/// (not `macOS`, `GitHub`, `WebSocket`, `OpenAI`).
fn strong_camel(s: &str) -> bool {
    let humps = s.bytes().skip(1).filter(|b| b.is_ascii_uppercase()).count();
    let first_part = s.bytes().take_while(|b| !b.is_ascii_uppercase()).count();
    if s.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase()) {
        humps >= 1 && first_part >= 4
    } else {
        humps >= 2 && s.bytes().any(|b| b.is_ascii_lowercase())
    }
}

fn known_ext(ext: &str) -> bool {
    super::lang::detect_lang(&format!("x.{ext}")).is_some()
}

fn parse_query(q: &str) -> Parsed {
    let mut p = Parsed { terms: terms::query_terms(q), ..Parsed::default() };
    let toks: Vec<&str> = q.split_whitespace().collect();
    for raw in &toks {
        let mut t = raw.trim_matches(|c: char| {
            matches!(c, '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '!' | '?' | ':' | '*')
        });
        t = t.trim_end_matches('.');
        if t.len() < 2 || t.contains("://") {
            continue;
        }
        // `src/a.rs:120` and `src/a.rs:12-40`
        if let Some((head, tail)) = t.rsplit_once(':')
            && !tail.is_empty()
            && tail.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            && !head.ends_with(':')
        {
            t = head;
        }
        let call = raw.find('(').is_some_and(|i| i > 0);
        let ext = t.rsplit_once('.').map(|(_, e)| e);
        let path_like =
            t.contains('/') && !t.contains("::") || ext.is_some_and(|e| known_ext(e) && t.len() > e.len() + 1 && !t.contains("::"));
        if path_like {
            let t = t.trim_start_matches("./");
            if !t.is_empty() && p.paths.len() < 3 && !p.paths.iter().any(|x| x == t) {
                p.paths.push(t.to_string());
            }
            continue;
        }
        // Qualified names: Foo::bar, Foo.bar, Foo#bar, foo->bar.
        let segs: Vec<&str> =
            t.split("::").flat_map(|s| s.split("->")).flat_map(|s| s.split(['.', '#'])).filter(|s| !s.is_empty()).collect();
        if segs.is_empty() || !segs.iter().all(|s| is_ident(s)) {
            continue;
        }
        let name = *segs.last().expect("non-empty");
        let qualified = segs.len() > 1;
        let snake = segs.iter().any(|s| s.contains('_') && s.chars().filter(|c| c.is_alphabetic()).count() >= 3);
        let ticked = raw.contains('`');
        let strict = ticked || qualified || call || snake;
        if (strict || strong_camel(name)) && name.len() >= 2 && p.idents.len() < 4 {
            let qualifier = qualified.then(|| segs[segs.len() - 2].to_string());
            if !p.idents.iter().any(|h| h.name == name && h.qualifier == qualifier) {
                p.idents.push(Hint { name: name.to_string(), qualifier, strict });
            }
        }
    }
    // A bare word that is itself a symbol name ("refresh") is looked up too, but only when
    // it is the entire query: otherwise plain words are ordinary search terms.
    if p.idents.is_empty() && p.paths.is_empty() && toks.len() == 1 {
        let w = toks[0].trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if is_ident(w) && w.len() >= 3 {
            p.idents.push(Hint { name: w.to_string(), qualifier: None, strict: true });
        }
    }
    p
}

// ───────────────────────────── exact stage ─────────────────────────────

#[derive(Debug, Clone)]
struct Site {
    file_id: i64,
    path: String,
    name: String,
    kind: String,
    container: Option<String>,
    line: i64,
    end_line: i64,
    rank: f64,
}

impl Site {
    fn qualified(&self) -> String {
        match &self.container {
            Some(c) => format!("{c}::{}", self.name),
            None => self.name.clone(),
        }
    }
}

fn is_test_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("/test/")
        || l.contains("/tests/")
        || l.contains("__tests__")
        || l.contains("/spec/")
        || l.contains("_test.")
        || l.contains(".test.")
        || l.contains(".spec.")
        || l.starts_with("test/")
        || l.starts_with("tests/")
        || l.contains("/fixtures/")
        || l.contains("/mocks/")
        || l.contains("/testdata/")
}

fn kind_weight(kind: &str) -> f64 {
    match kind {
        "struct" | "class" | "enum" | "trait" | "interface" | "type" | "object" | "protocol" | "record" | "actor" | "message"
        | "service" | "table" => 1.0,
        "fn" | "method" | "init" | "macro" => 0.6,
        "const" | "static" | "var" => 0.3,
        "heading" => 0.2,
        _ => 0.0, // impl, mod, extension: where code lives, not what it defines
    }
}

/// Definitions of one identifier. Empty when the name is unknown; callers decide whether
/// the number of sites is small enough to call the hit exact.
fn symbol_sites(conn: &Connection, h: &Hint) -> anyhow::Result<Vec<Site>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.name, s.kind, s.container, s.line, s.end_line, f.id, f.path
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 COLLATE NOCASE OR s.name IN (?2, ?3, ?4) LIMIT 120",
    )?;
    // Ruby spells predicates, bang methods and setters `name?`, `name!`, `name=`.
    let rows = stmt.query_map(params![h.name, format!("{}?", h.name), format!("{}!", h.name), format!("{}=", h.name)], |r| {
        Ok(Site {
            name: r.get(0)?,
            kind: r.get(1)?,
            container: r.get(2)?,
            line: r.get(3)?,
            end_line: r.get(4)?,
            file_id: r.get(5)?,
            path: r.get(6)?,
            rank: 0.0,
        })
    })?;
    let mut sites: Vec<Site> = rows.collect::<Result<_, _>>()?;
    if h.strict {
        // Prefer the spelling the user typed; fall back to any case.
        if sites.iter().any(|s| s.name == h.name) {
            sites.retain(|s| s.name == h.name);
        }
    } else {
        sites.retain(|s| s.name == h.name && !matches!(s.kind.as_str(), "heading" | "mod"));
    }
    if let Some(q) = &h.qualifier {
        let stem = |p: &str| p.rsplit('/').next().unwrap_or(p).split('.').next().unwrap_or("").to_ascii_lowercase();
        let q_l = q.to_ascii_lowercase();
        let matching: Vec<Site> = sites
            .iter()
            .filter(|s| s.container.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(q)) || stem(&s.path) == q_l)
            .cloned()
            .collect();
        if !matching.is_empty() {
            sites = matching;
        }
    }
    for s in &mut sites {
        s.rank = kind_weight(&s.kind) - if is_test_path(&s.path) { 1.0 } else { 0.0 } - s.path.len() as f64 * 0.001
            + if s.name == h.name { 0.5 } else { 0.0 };
    }
    sites.sort_by(|a, b| b.rank.total_cmp(&a.rank).then_with(|| a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    Ok(sites)
}

/// Files matching a path-like token by suffix on a `/` boundary.
fn path_sites(conn: &Connection, hint: &str) -> anyhow::Result<Vec<(i64, String, i64)>> {
    let esc: String = hint.chars().flat_map(|c| if matches!(c, '%' | '_' | '\\') { vec!['\\', c] } else { vec![c] }).collect();
    let mut stmt = conn
        .prepare_cached("SELECT id, path, lines FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\' ORDER BY length(path) LIMIT 12")?;
    let rows = stmt.query_map(params![hint, format!("%/{esc}")], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ───────────────────────────── candidates ─────────────────────────────

#[derive(Debug, Clone)]
struct ChunkHit {
    file_id: i64,
    start: i64,
    end: i64,
    bm25: f64,
    /// 0 = NEAR, 1.. = AND (1 + terms dropped), 100 = OR fallback.
    level: u32,
    /// How many query terms this chunk is known to contain.
    n_terms: usize,
}

const OR_LEVEL: u32 = 100;
/// Document frequencies are counted up to this many chunks; beyond it a term is "common".
// ponytail: a capped count says nothing about how common "common" is, so at 100k files a query of
// ubiquitous terms can take ~200 ms (96k-file Linux tree). A stored table of the few thousand
// commonest terms, rebuilt after bulk loads, would bound the OR cost exactly.
const DF_CAP: i64 = 3000;

struct Hybrid {
    /// File ids, best first (files ranker).
    files: Vec<i64>,
    /// Strict chunk hits: NEAR, then AND with the commonest term dropped one at a time.
    and: Vec<ChunkHit>,
    /// OR over the rarest terms, by bm25.
    or: Vec<ChunkHit>,
    /// `and` followed by the `or` hits it does not already hold: what citations draw from.
    chunks: Vec<ChunkHit>,
    /// Query terms present in the index, rarest first, with their (capped) document frequency.
    rare: Vec<String>,
    rare_df: Vec<i64>,
    /// The first `max_and_terms` of `rare`: the ones the AND levels use.
    ladder: Vec<String>,
}

fn run_fts<T>(what: &str, r: rusqlite::Result<T>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("index {what} query failed: {e}");
            None
        }
    }
}

fn df_capped(conn: &Connection, term: &str) -> i64 {
    let mut stmt = match conn.prepare_cached("SELECT count(*) FROM (SELECT 1 FROM fts_chunks WHERE fts_chunks MATCH ?1 LIMIT ?2)") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    stmt.query_row(params![fts_quote(term), DF_CAP], |r| r.get(0)).unwrap_or(0)
}

fn chunk_query(conn: &Connection, expr: &str, limit: usize, level: u32, n_terms: usize) -> Vec<ChunkHit> {
    let sql = "SELECT k.s, c.file_id, c.start_line, c.end_line
               FROM (SELECT rowid, bm25(fts_chunks) AS s FROM fts_chunks WHERE fts_chunks MATCH ?1 ORDER BY s LIMIT ?2) k
               JOIN chunks c ON c.id = k.rowid ORDER BY k.s";
    run_fts(
        "chunk",
        (|| {
            let mut stmt = conn.prepare_cached(sql)?;
            let rows = stmt.query_map(params![expr, limit as i64], |r| {
                Ok(ChunkHit { bm25: r.get(0)?, file_id: r.get(1)?, start: r.get(2)?, end: r.get(3)?, level, n_terms })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })(),
    )
    .unwrap_or_default()
}

fn files_query(conn: &Connection, expr: &str, t: &Tuning, limit: usize) -> Vec<i64> {
    let sql = "SELECT rowid FROM fts_files WHERE fts_files MATCH ?1 ORDER BY bm25(fts_files, ?2, ?3, ?4) LIMIT ?5";
    run_fts(
        "files",
        (|| {
            let mut stmt = conn.prepare_cached(sql)?;
            let rows = stmt.query_map(params![expr, t.bm25_path, t.bm25_symbols, t.bm25_doc, limit as i64], |r| r.get::<_, i64>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })(),
    )
    .unwrap_or_default()
}

fn distinct_files(hits: &[ChunkHit]) -> Vec<i64> {
    let mut seen = HashSet::new();
    hits.iter().filter(|h| seen.insert(h.file_id)).map(|h| h.file_id).collect()
}

/// Files ordered by their chunks' evidence: best chunk plus `gamma` × the next two.
fn files_by_chunks(hits: &[ChunkHit], gamma: f64) -> Vec<i64> {
    if gamma <= 0.0 {
        return distinct_files(hits);
    }
    let mut per: HashMap<i64, Vec<f64>> = HashMap::new();
    for h in hits {
        per.entry(h.file_id).or_default().push(-h.bm25); // bm25() is negative: better = more negative
    }
    let mut v: Vec<(i64, f64)> = per
        .into_iter()
        .map(|(id, mut s)| {
            s.sort_by(|a, b| b.total_cmp(a));
            (id, s[0] + gamma * s.iter().skip(1).take(2).sum::<f64>())
        })
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(id, _)| id).collect()
}

fn hybrid(conn: &Connection, q: &QueryTerms, t: &Tuning, deadline: Instant, phases: &mut [f64; 6]) -> Hybrid {
    let started = Instant::now();
    let mut hy = Hybrid {
        files: Vec::new(),
        and: Vec::new(),
        or: Vec::new(),
        chunks: Vec::new(),
        rare: Vec::new(),
        rare_df: Vec::new(),
        ladder: Vec::new(),
    };
    if q.is_empty() {
        return hy;
    }
    // ── files ranker: OR over every term (the table has one row per file, so this is cheap)
    let all: Vec<&String> = q.required.iter().chain(q.wholes.iter()).collect();
    let or_expr = all.iter().map(|s| fts_quote(s)).collect::<Vec<_>>().join(" OR ");
    if t.files_and_first && q.required.len() >= 2 {
        let and_expr = q.required.iter().map(|s| fts_quote(s)).collect::<Vec<_>>().join(" AND ");
        hy.files = files_query(conn, &and_expr, t, 50);
    }
    let seen: HashSet<i64> = hy.files.iter().copied().collect();
    hy.files.extend(files_query(conn, &or_expr, t, 50).into_iter().filter(|id| !seen.contains(id)));
    hy.files.truncate(50);
    phases[1] = started.elapsed().as_secs_f64() * 1000.0;
    if Instant::now() > deadline {
        return hy;
    }
    let t_stats = Instant::now();

    // ── chunk rankers. Document frequencies order the terms rarest-first and drop the ones
    // the index has never seen (they would empty every AND).
    let mut by_df: Vec<(String, i64)> = q.required.iter().map(|s| (s.clone(), df_capped(conn, s))).filter(|(_, df)| *df > 0).collect();
    by_df.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    hy.rare_df = by_df.iter().map(|(_, df)| *df).collect();
    hy.rare = by_df.into_iter().map(|(s, _)| s).collect();
    hy.ladder = hy.rare.iter().take(t.max_and_terms.max(1)).cloned().collect();
    let ladder = &hy.ladder;
    phases[2] = t_stats.elapsed().as_secs_f64() * 1000.0;
    let t_and = Instant::now();
    let quote_all = |ts: &[String]| ts.iter().map(|s| fts_quote(s)).collect::<Vec<_>>();

    if t.ladder || t.w_and > 0.0 {
        let mut seen: HashSet<(i64, i64)> = HashSet::new();
        let mut push = |hits: Vec<ChunkHit>, and: &mut Vec<ChunkHit>| {
            and.extend(hits.into_iter().filter(|h| seen.insert((h.file_id, h.start))));
        };
        if t.near && ladder.len() >= 2 {
            let n = ladder.len().min(5);
            let expr = format!("NEAR({}, {})", quote_all(&ladder[..n]).join(" "), t.near_distance);
            push(chunk_query(conn, &expr, 50, 0, n), &mut hy.and);
        }
        let mut n = ladder.len();
        while n >= 2 && hy.and.len() < t.min_hits && Instant::now() < deadline {
            let expr = quote_all(&ladder[..n]).join(" AND ");
            push(chunk_query(conn, &expr, 50, (1 + ladder.len() - n) as u32, n), &mut hy.and);
            n -= 1;
        }
    }
    phases[3] = t_and.elapsed().as_secs_f64() * 1000.0;
    let t_or = Instant::now();
    if (t.w_or > 0.0 || hy.and.is_empty()) && !hy.rare.is_empty() && Instant::now() < deadline {
        let mut m = 0;
        let mut cost = 0;
        while m < hy.rare.len().min(t.or_terms.max(1)) && (m < 3 || cost + hy.rare_df[m] <= t.or_df_budget) {
            cost += hy.rare_df[m];
            m += 1;
        }
        let mut terms: Vec<String> = hy.rare[..m].to_vec();
        terms.extend(q.wholes.iter().take(2).cloned());
        // OR hits are only known to hold one term (a single-term query: all of them).
        hy.or = chunk_query(conn, &quote_all(&terms).join(" OR "), 80, OR_LEVEL, 1);
    }
    phases[4] = t_or.elapsed().as_secs_f64() * 1000.0;
    let have: HashSet<(i64, i64)> = hy.and.iter().map(|h| (h.file_id, h.start)).collect();
    hy.chunks = hy.and.clone();
    hy.chunks.extend(hy.or.iter().filter(|h| !have.contains(&(h.file_id, h.start))).cloned());
    hy
}

#[derive(Debug, Clone)]
struct Cand {
    file_id: i64,
    score: f64,
    r_files: Option<usize>,
    r_chunks: Option<usize>,
}

/// RRF of `lists` (each a best-first list of file ids with a weight).
fn rrf(lists: &[(&[i64], f64)], k: f64) -> Vec<(i64, f64)> {
    let mut m: HashMap<i64, f64> = HashMap::new();
    for (list, w) in lists {
        for (r, id) in list.iter().enumerate() {
            *m.entry(*id).or_default() += w / (k + r as f64 + 1.0);
        }
    }
    let mut v: Vec<(i64, f64)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Fused file ranking plus the chunk rankers' own file order (for the agreement feature).
fn fuse(hy: &Hybrid, t: &Tuning) -> (Vec<Cand>, Vec<i64>) {
    let and_files = distinct_files(&hy.and);
    let or_files = files_by_chunks(&hy.or, t.agg_gamma);
    let chunk_order: Vec<i64> = rrf(&[(&and_files, t.w_and), (&or_files, t.w_or)], t.rrf_k).into_iter().map(|(id, _)| id).collect();
    let fused = rrf(&[(&hy.files, t.w_files), (&and_files, t.w_and), (&or_files, t.w_or)], t.rrf_k);
    let r_files: HashMap<i64, usize> = hy.files.iter().enumerate().map(|(r, id)| (*id, r)).collect();
    let r_chunks: HashMap<i64, usize> = chunk_order.iter().enumerate().map(|(r, id)| (*id, r)).collect();
    let mut v: Vec<Cand> = fused
        .into_iter()
        .map(|(id, score)| Cand { file_id: id, score, r_files: r_files.get(&id).copied(), r_chunks: r_chunks.get(&id).copied() })
        .collect();
    // Ties break on the stronger evidence, then on id so results are stable.
    v.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.r_chunks.unwrap_or(usize::MAX).cmp(&b.r_chunks.unwrap_or(usize::MAX)))
            .then_with(|| a.file_id.cmp(&b.file_id))
    });
    (v, chunk_order)
}

// ───────────────────────────── citations ─────────────────────────────

#[derive(Debug, Clone)]
struct SymRow {
    name: String,
    kind: String,
    container: Option<String>,
    line: i64,
    end_line: i64,
}

struct FileInfo {
    path: String,
    lines: i64,
    symbols: Vec<SymRow>,
}

fn load_file(conn: &Connection, id: i64) -> Option<FileInfo> {
    let (path, lines): (String, i64) =
        conn.prepare_cached("SELECT path, lines FROM files WHERE id = ?1").ok()?.query_row([id], |r| Ok((r.get(0)?, r.get(1)?))).ok()?;
    let mut stmt =
        conn.prepare_cached("SELECT name, kind, container, line, end_line FROM symbols WHERE file_id = ?1 ORDER BY line").ok()?;
    let symbols = stmt
        .query_map([id], |r| Ok(SymRow { name: r.get(0)?, kind: r.get(1)?, container: r.get(2)?, line: r.get(3)?, end_line: r.get(4)? }))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    Some(FileInfo { path, lines, symbols })
}

fn name_terms(s: &str) -> Vec<String> {
    let mut out = String::new();
    terms::push_terms(s, &mut out);
    out.split(' ').filter(|t| !t.is_empty()).map(String::from).collect()
}

/// The symbol a span is best described by: one that overlaps it, names the query's terms,
/// and is as small as possible (a method over the class around it).
fn pick_symbol<'a>(syms: &'a [SymRow], start: i64, end: i64, qset: &HashSet<&str>) -> Option<&'a SymRow> {
    let mut best: Option<(&SymRow, f64)> = None;
    for s in syms.iter().filter(|s| s.line <= end && s.end_line >= start) {
        let hits = name_terms(&s.name).iter().filter(|t| qset.contains(t.as_str())).count()
            + s.container.as_deref().map_or(0, |c| name_terms(c).iter().filter(|t| qset.contains(t.as_str())).count().min(1));
        let score = hits as f64 * 10.0 + kind_weight(&s.kind) - (s.end_line - s.line) as f64 / 500.0;
        if best.is_none_or(|(_, b)| score > b) {
            best = Some((s, score));
        }
    }
    best.map(|(s, _)| s)
}

fn qualified(s: &SymRow) -> String {
    match &s.container {
        Some(c) => format!("{c}::{}", s.name),
        None => s.name.clone(),
    }
}

fn level_label(c: Confidence) -> &'static str {
    match c {
        Confidence::Exact => "exact",
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn why_chunk(
    hit: Option<&ChunkHit>,
    ladder: &[String],
    required: usize,
    cand: &Cand,
    path: &str,
    sym: Option<&SymRow>,
    qset: &HashSet<&str>,
) -> Vec<String> {
    let mut why = Vec::new();
    if let Some(h) = hit {
        match h.level {
            0 => why.push(format!("terms near each other in chunk: {}", ladder[..h.n_terms.min(ladder.len())].join(", "))),
            OR_LEVEL => why.push("chunk matches some query terms (partial)".to_string()),
            _ => {
                let shown = ladder[..h.n_terms.min(ladder.len())].join(", ");
                if h.n_terms >= required {
                    why.push(format!("chunk has all {required} query terms: {shown}"));
                } else {
                    why.push(format!("chunk has {}/{required} query terms: {shown}", h.n_terms));
                }
            }
        }
    }
    let path_hits: Vec<String> = name_terms(path).into_iter().filter(|t| qset.contains(t.as_str())).fold(Vec::new(), |mut a, t| {
        if !a.contains(&t) {
            a.push(t)
        }
        a
    });
    if !path_hits.is_empty() {
        why.push(format!("path matches: {}", path_hits.join(", ")));
    }
    if let Some(s) = sym {
        let hits: Vec<String> = name_terms(&s.name).into_iter().filter(|t| qset.contains(t.as_str())).collect();
        if !hits.is_empty() {
            why.push(format!("symbol {} matches: {}", s.name, hits.join(", ")));
        }
    }
    let mut ranks = Vec::new();
    if let Some(r) = cand.r_files {
        ranks.push(format!("files#{}", r + 1));
    }
    if let Some(r) = cand.r_chunks {
        ranks.push(format!("chunks#{}", r + 1));
    }
    if !ranks.is_empty() {
        why.push(format!("ranks {}", ranks.join(" ")));
    }
    why
}

// ───────────────────────────── fallback hints ─────────────────────────────

/// `rg -i` patterns for when the index is not confident: the rarest terms joined the ways
/// identifiers are written.
fn rg_patterns(q: &QueryTerms, ladder: &[String]) -> Vec<String> {
    // Rarest terms first, but present in the query's own order inside a pattern.
    let rare: Vec<&String> = if ladder.is_empty() { q.required.iter().take(3).collect() } else { ladder.iter().take(3).collect() };
    if rare.is_empty() {
        return Vec::new();
    }
    let mut ordered: Vec<&String> = rare.clone();
    ordered.sort_by_key(|t| q.required.iter().position(|r| r == *t).unwrap_or(usize::MAX));
    let mut out = Vec::new();
    if ordered.len() >= 2 {
        let (a, b) = (ordered[0], ordered[1]);
        out.push(format!("{a}.*{b}"));
        let cap = |s: &str| {
            let mut c = s.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        };
        out.push(format!("{a}_{b}|{a}{}|{}{}", cap(b), cap(a), cap(b)));
    }
    if ordered.len() >= 3 {
        out.push(ordered.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("|"));
    } else {
        out.push(format!("{}\\w*", ordered[0]));
    }
    out.dedup();
    out
}

// ───────────────────────────── the search ─────────────────────────────

pub(crate) fn search(conn: &Connection, query: &str, k: usize, t: &Tuning) -> anyhow::Result<Ranked> {
    let started = Instant::now();
    let deadline = started + std::time::Duration::from_millis(t.timeout_ms);
    let k = k.clamp(1, 60);
    let mut parsed = parse_query(query);
    if !t.exact {
        parsed.idents.clear();
        parsed.paths.clear();
    }
    let mut cits: Vec<Citation> = Vec::new();
    let mut meta: Vec<CitationMeta> = Vec::new();
    let qset: HashSet<&str> = parsed.terms.required.iter().chain(parsed.terms.wholes.iter()).map(String::as_str).collect();

    // ── 1. exact: identifiers and paths named in the query
    let mut exact = false;
    let mut exact_spans: Vec<(i64, i64, i64)> = Vec::new(); // (file, start, end)
    for h in &parsed.idents {
        let sites = symbol_sites(conn, h)?;
        let files: HashSet<i64> = sites.iter().map(|s| s.file_id).collect();
        // Many homonyms (`new`, `run`) say nothing: only a handful of sites is "exact".
        if sites.is_empty() || files.len() > 4 {
            continue;
        }
        exact = true;
        for s in sites.iter().take(6) {
            if cits.len() >= k.min(t.exact_max.max(1)) || exact_spans.iter().any(|e| e.0 == s.file_id && e.1 == s.line) {
                continue;
            }
            exact_spans.push((s.file_id, s.line, s.end_line));
            cits.push(Citation {
                id: String::new(),
                path: s.path.clone(),
                start_line: s.line,
                end_line: s.end_line,
                symbol: Some(s.qualified()),
                why: vec![format!("exact symbol: {} {} at {}:{}", s.kind, s.qualified(), s.path, s.line)],
                score: 1.0 - cits.len() as f64 * 0.01,
            });
            meta.push(CitationMeta { kind: Some(s.kind.clone()), level: Confidence::Exact });
        }
    }
    for p in &parsed.paths {
        let sites = path_sites(conn, p)?;
        if sites.is_empty() || sites.len() > 4 {
            continue;
        }
        exact = true;
        for (id, path, lines) in sites {
            if cits.len() >= k.min(t.exact_max.max(1)) {
                break;
            }
            // Other words in the query narrow the file to its relevant chunk; otherwise its head.
            let (start, end) = best_chunk_in_file(conn, id, &parsed.terms).unwrap_or((1, lines.clamp(1, 60)));
            exact_spans.push((id, start, end));
            cits.push(Citation {
                id: String::new(),
                path: path.clone(),
                start_line: start,
                end_line: end,
                symbol: None,
                why: vec![format!("exact path: {path}")],
                score: 1.0 - cits.len() as f64 * 0.01,
            });
            meta.push(CitationMeta { kind: None, level: Confidence::Exact });
        }
    }

    // ── 2–3. candidates and fusion
    let mut phases = [0.0f64; 6];
    phases[0] = started.elapsed().as_secs_f64() * 1000.0;
    let hy = hybrid(conn, &parsed.terms, t, deadline, &mut phases);
    let t_cit = Instant::now();
    let (cands, chunk_order) = fuse(&hy, t);

    // ── confidence features
    let top_files: Vec<i64> = hy.files.iter().take(10).copied().collect();
    let agree = |n: usize| chunk_order.iter().take(n).filter(|f| top_files[..top_files.len().min(n)].contains(f)).count();
    let (agree3, agree5, agree10) = (agree(3), agree(5), agree(10));
    let mut or_scores: Vec<f64> = Vec::new();
    {
        let mut seen = HashSet::new();
        for h in &hy.or {
            if seen.insert(h.file_id) {
                or_scores.push(h.bm25.abs());
            }
        }
    }
    let top_and_n = cands.first().map_or(0, |c| hy.and.iter().filter(|h| h.file_id == c.file_id).map(|h| h.n_terms).max().unwrap_or(0));
    let features = Features {
        n_terms: parsed.terms.required.len(),
        n_rare: hy.rare.len(),
        agree3,
        agree5,
        agree10,
        or_top1: or_scores.first().copied().unwrap_or(0.0),
        or_top2: or_scores.get(1).copied().unwrap_or(0.0),
        or_top5: or_scores.get(4).copied().unwrap_or(0.0),
        and_best_n: hy.and.iter().map(|h| h.n_terms).max().unwrap_or(0),
        and_hits: hy.and.len(),
        top_and_n,
        top_in_both: cands.first().is_some_and(|c| c.r_files.is_some_and(|r| r < 3) && c.r_chunks.is_some_and(|r| r < 3)),
        dirs5: cands.iter().take(5).filter_map(|c| infos_dir(conn, c.file_id)).collect::<HashSet<_>>().len(),
        min_df: hy.rare_df.first().copied().unwrap_or(0),
        phase_ms: phases,
    };
    let p_hit = 1.0 / (1.0 + (-(t.conf_b + t.conf_agree * agree5 as f64 + t.conf_score * (1.0 + features.or_top1).ln())).exp());
    let confidence = if exact {
        Confidence::Exact
    } else if cands.is_empty() || hy.rare.len() < t.min_terms {
        Confidence::Low
    } else if p_hit >= t.high_p {
        Confidence::High
    } else if p_hit >= t.medium_p {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    // ── citations from the fused ranking
    let mut infos: HashMap<i64, Option<FileInfo>> = HashMap::new();
    for (rank, cand) in cands.iter().enumerate() {
        if cits.len() >= k {
            break;
        }
        let info = infos.entry(cand.file_id).or_insert_with(|| load_file(conn, cand.file_id));
        let Some(info) = info.as_ref() else { continue };
        let quota = match rank {
            0 => 3,
            1 | 2 => 2,
            _ => 1,
        };
        let mut hits: Vec<&ChunkHit> = hy.chunks.iter().filter(|h| h.file_id == cand.file_id).collect();
        hits.sort_by(|a, b| a.level.cmp(&b.level).then(a.bm25.total_cmp(&b.bm25)));
        let first = hits.first().copied();
        let mut spans: Vec<(i64, i64, Option<&ChunkHit>)> = Vec::new();
        for (i, &h) in hits.iter().enumerate() {
            let keep = i == 0 || (spans.len() < quota && first.is_some_and(|f| h.level <= f.level && h.bm25.abs() >= 0.5 * f.bm25.abs()));
            if !keep {
                continue;
            }
            // merge with an overlapping or adjacent span of the same file
            if let Some(m) = spans.iter_mut().find(|s| h.start <= s.1 + 1 && h.end >= s.0 - 1 && (h.end.max(s.1) - h.start.min(s.0)) <= 130)
            {
                m.0 = m.0.min(h.start);
                m.1 = m.1.max(h.end);
            } else {
                spans.push((h.start, h.end, Some(h)));
            }
        }
        if spans.is_empty() {
            // Matched by path / symbol / comment terms only: point at the best-named symbol, else the head.
            let best = info
                .symbols
                .iter()
                .map(|s| (s, name_terms(&s.name).iter().filter(|t| qset.contains(t.as_str())).count()))
                .filter(|(_, n)| *n > 0)
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.line.cmp(&a.0.line)));
            match best {
                Some((s, _)) => spans.push((s.line, s.end_line.max(s.line), None)),
                None => spans.push((1, info.lines.clamp(1, 60), None)),
            }
        }
        for (start, end, hit) in spans {
            if cits.len() >= k {
                break;
            }
            if exact_spans.iter().any(|e| e.0 == cand.file_id && start <= e.2 && end >= e.1) {
                continue; // already cited as an exact definition
            }
            let sym = pick_symbol(&info.symbols, start, end, &qset);
            let in_both = cand.r_files.is_some() && cand.r_chunks.is_some();
            let level = match confidence {
                Confidence::Exact => Confidence::Medium,
                Confidence::High if in_both && rank < 3 => Confidence::High,
                Confidence::High | Confidence::Medium if in_both || rank < 3 => Confidence::Medium,
                _ => Confidence::Low,
            };
            cits.push(Citation {
                id: String::new(),
                path: info.path.clone(),
                start_line: start,
                end_line: end,
                symbol: sym.map(qualified),
                why: why_chunk(hit, &hy.ladder, parsed.terms.required.len(), cand, &info.path, sym, &qset),
                score: cand.score,
            });
            meta.push(CitationMeta { kind: sym.map(|s| s.kind.clone()), level });
        }
    }
    for (i, c) in cits.iter_mut().enumerate() {
        c.id = format!("C{}", i + 1);
    }
    let mut features = features;
    features.phase_ms[5] = t_cit.elapsed().as_secs_f64() * 1000.0;
    let fallback_rg =
        if matches!(confidence, Confidence::Low | Confidence::Medium) { rg_patterns(&parsed.terms, &hy.rare) } else { Vec::new() };
    tracing::debug!(query, ?confidence, agree5, p_hit, citations = cits.len(), "index search");
    Ok(Ranked {
        result: SearchResult {
            query: query.to_string(),
            confidence,
            citations: cits,
            fallback_rg,
            took_ms: started.elapsed().as_millis() as i64,
        },
        meta,
        features,
    })
}

/// Parent directory of a file (for the dispersion feature).
fn infos_dir(conn: &Connection, file_id: i64) -> Option<String> {
    let path: String = conn.prepare_cached("SELECT path FROM files WHERE id = ?1").ok()?.query_row([file_id], |r| r.get(0)).ok()?;
    Some(path.rsplit_once('/').map_or(String::new(), |(d, _)| d.to_string()))
}

/// The best chunk of one file for the query's terms (used when a path was named exactly).
fn best_chunk_in_file(conn: &Connection, file_id: i64, q: &QueryTerms) -> Option<(i64, i64)> {
    if q.is_empty() {
        return None;
    }
    let (lo, hi): (i64, i64) = conn
        .prepare_cached("SELECT min(id), max(id) FROM chunks WHERE file_id = ?1")
        .ok()?
        .query_row([file_id], |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, Option<i64>>(1)?.unwrap_or(-1))))
        .ok()?;
    if hi < lo {
        return None;
    }
    let expr = q.required.iter().take(8).map(|s| fts_quote(s)).collect::<Vec<_>>().join(" OR ");
    // A file's chunks have consecutive ids, so the file restricts as a rowid range inside FTS5.
    let sql = "SELECT c.start_line, c.end_line FROM fts_chunks JOIN chunks c ON c.id = fts_chunks.rowid
               WHERE fts_chunks MATCH ?1 AND fts_chunks.rowid BETWEEN ?2 AND ?3 ORDER BY bm25(fts_chunks) LIMIT 1";
    run_fts("file chunk", conn.prepare_cached(sql).and_then(|mut s| s.query_row(params![expr, lo, hi], |r| Ok((r.get(0)?, r.get(1)?)))))
}

pub(crate) fn confidence_label(c: Confidence) -> &'static str {
    level_label(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hints() {
        let p = parse_query("fix retainedSampling in src/guardian/transport.rs and Index::refresh, also parse_url()");
        let names: Vec<String> = p.idents.iter().map(|h| format!("{:?}.{}", h.qualifier, h.name)).collect();
        assert_eq!(names, ["None.retainedSampling", "Some(\"Index\").refresh", "None.parse_url"]);
        assert_eq!(p.paths, ["src/guardian/transport.rs"]);
    }

    #[test]
    fn plain_words_are_not_hints_unless_alone() {
        assert!(parse_query("retry the upload when it fails").idents.is_empty());
        assert_eq!(parse_query("refresh").idents.len(), 1);
        assert!(parse_query("mod.rs:12-40").paths == ["mod.rs"]);
    }

    #[test]
    fn rg_patterns_use_the_rarest_terms() {
        let q = terms::query_terms("retained sampling guardian transport");
        let ladder = vec!["guardian".to_string(), "sampling".to_string(), "retained".to_string(), "transport".to_string()];
        let p = rg_patterns(&q, &ladder);
        assert_eq!(p[0], "retained.*sampling");
        assert!(p[1].contains("retained_sampling") && p[1].contains("retainedSampling"), "{p:?}");
    }
}
