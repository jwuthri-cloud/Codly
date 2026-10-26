//! Commit-replay localization benchmark: Stage A of docs/research/context-engineering.md §6.
//!
//! For each of the last N non-merge (first-parent) commits, oldest first:
//!   * the index holds the commit's PARENT snapshot exactly (parent of the oldest commit
//!     extracted once with `git archive`, then every later parent reached by applying the
//!     previous diffs through `Index::update_paths`, which exercises the incremental path);
//!   * query = the commit subject (default; closest to a user prompt) or the cleaned message;
//!   * gold = files the commit modified or deleted that the index holds (added files cannot
//!     be retrieved);
//!   * we score Hit@k, Recall@k and MRR of `Index::search`, plus the confidence calibration
//!     (Hit@5 per level) and latencies.
//!
//! The repo is only read through `git log/diff-tree/archive`; nothing in it is executed.
//!
//!   cargo run --release --example bench_localize -- <repo> [--n 300] [--query subject|full]
//!       [--k 50] [--all-parents] [--max-gold 25] [--no-porter] [--no-path-in-chunks] [--no-symbols-in-chunks] [--set key=value]...
//!       [--variant name:key=value,key=value]... [--grep] [--json out.json] [--tsv out.tsv] [--work DIR] [--keep]
//!   cargo run --release --example bench_localize -- search <repo> [--set k=v]... <query words>
//!
//! ponytail: the pilot (and the spec) replays the exact parent per commit; this does too, but
//! only for linear first-parent history. With --all-parents the snapshot jumps between
//! arbitrary parents (correct, just slower). Line-level gold and Span-Recall@budget are not
//! measured; add them when the pointer pack starts carrying snippets.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use codly_core::index::{Index, IndexOptions, Tuning};
use codly_core::model::Confidence;

struct Args {
    repo: PathBuf,
    n: usize,
    full: bool,
    k: usize,
    all_parents: bool,
    max_gold: usize,
    opts: IndexOptions,
    tune: Vec<(String, String)>,
    /// `name:key=value,key=value` — extra tunings evaluated on the same index state.
    variants: Vec<(String, Vec<(String, String)>)>,
    grep: bool,
    json: Option<PathBuf>,
    tsv: Option<PathBuf>,
    work: PathBuf,
    keep: bool,
    rest: Vec<String>,
}

fn parse_args() -> Args {
    let mut it = std::env::args().skip(1).peekable();
    let mut a = Args {
        repo: PathBuf::new(),
        n: 300,
        full: false,
        k: 50,
        all_parents: false,
        max_gold: 25,
        opts: IndexOptions::default(),
        tune: Vec::new(),
        variants: Vec::new(),
        grep: false,
        json: None,
        tsv: None,
        work: std::env::temp_dir().join(format!("codly-bench-{}", std::process::id())),
        keep: false,
        rest: Vec::new(),
    };
    let mut positional = Vec::new();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--n" => a.n = it.next().and_then(|v| v.parse().ok()).expect("--n <int>"),
            "--query" => a.full = it.next().as_deref() == Some("full"),
            "--k" => a.k = it.next().and_then(|v| v.parse().ok()).expect("--k <int>"),
            "--all-parents" => a.all_parents = true,
            "--max-gold" => a.max_gold = it.next().and_then(|v| v.parse().ok()).expect("--max-gold <int>"),
            "--no-porter" => a.opts.porter = false,
            "--no-path-in-chunks" => a.opts.path_in_chunks = false,
            "--no-symbols-in-chunks" => a.opts.symbols_in_chunks = false,
            "--set" => {
                let kv = it.next().expect("--set key=value");
                let (k, v) = kv.split_once('=').expect("--set key=value");
                a.tune.push((k.to_string(), v.to_string()));
            }
            "--variant" => {
                let v = it.next().expect("--variant name:key=value,key=value");
                let (name, rest) = v.split_once(':').unwrap_or((v.as_str(), ""));
                let kvs = rest
                    .split(',')
                    .filter(|x| !x.is_empty())
                    .map(|kv| {
                        let (k, v) = kv.split_once('=').expect("key=value");
                        (k.to_string(), v.to_string())
                    })
                    .collect();
                a.variants.push((name.to_string(), kvs));
            }
            "--grep" => a.grep = true,
            "--json" => a.json = it.next().map(PathBuf::from),
            "--tsv" => a.tsv = it.next().map(PathBuf::from),
            "--work" => a.work = it.next().map(PathBuf::from).expect("--work DIR"),
            "--keep" => a.keep = true,
            _ => positional.push(arg),
        }
    }
    if positional.first().map(String::as_str) == Some("search") {
        a.rest = positional;
        return a;
    }
    a.repo = PathBuf::from(positional.first().expect("usage: bench_localize <repo> [options]"));
    a
}

// ───────────────────────────── git ─────────────────────────────

fn git_bytes(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().expect("run git");
    if !out.status.success() {
        panic!("git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    out.stdout
}

struct Commit {
    sha: String,
    parent: String,
    author: String,
    subject: String,
    body: String,
}

fn commits(repo: &Path, n: usize, first_parent: bool) -> Vec<Commit> {
    let n_s = n.to_string();
    let mut args = vec!["log", "--no-merges", "-n", &n_s, "--reverse", "--format=%H%x1f%P%x1f%an%x1f%s%x1f%b%x1e"];
    if first_parent {
        args.insert(2, "--first-parent");
    }
    let raw = String::from_utf8_lossy(&git_bytes(repo, &args)).into_owned();
    raw.split('\x1e')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            let f: Vec<&str> = rec.split('\x1f').collect();
            if f.len() < 5 || f[0].is_empty() {
                return None;
            }
            Some(Commit {
                sha: f[0].to_string(),
                parent: f[1].split(' ').next().unwrap_or("").to_string(),
                author: f[2].to_string(),
                subject: f[3].to_string(),
                body: f[4].to_string(),
            })
        })
        .collect()
}

/// (status letter, path) pairs between two revisions, renames split into D + A.
fn name_status(repo: &Path, from: &str, to: &str) -> Vec<(char, String)> {
    let raw = git_bytes(repo, &["diff-tree", "--no-commit-id", "--name-status", "-r", "-z", "--no-renames", from, to]);
    let mut parts = raw.split(|b| *b == 0).map(|s| String::from_utf8_lossy(s).into_owned());
    let mut out = Vec::new();
    while let (Some(s), Some(p)) = (parts.next(), parts.next()) {
        if let Some(c) = s.chars().next() {
            out.push((c, p));
        }
    }
    out
}

/// `git archive <rev> [-- paths]` piped into `tar -x` under `dir`.
fn extract(repo: &Path, rev: &str, dir: &Path, paths: &[String]) {
    std::fs::create_dir_all(dir).expect("create snapshot dir");
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(["archive", "--format=tar", rev]);
    if !paths.is_empty() {
        cmd.arg("--").args(paths);
    }
    let mut g = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn git archive");
    let st = Command::new("tar").arg("-x").arg("-C").arg(dir).stdin(g.stdout.take().expect("pipe")).status().expect("run tar");
    let gst = g.wait().expect("wait git");
    assert!(st.success() && gst.success(), "archive of {rev} failed");
}

// ───────────────────────────── queries ─────────────────────────────

fn clean_message(raw: &str) -> String {
    let mut out = Vec::new();
    for l in raw.lines() {
        let low = l.trim().to_ascii_lowercase();
        if ["co-authored-by:", "signed-off-by:", "change-id:", "reviewed-by:", "cc:"].iter().any(|p| low.starts_with(p)) {
            continue;
        }
        out.push(l);
    }
    let mut s = out.join("\n");
    // "(#1234)" PR numbers and URLs
    while let Some(i) = s.find("(#") {
        match s[i..].find(')') {
            Some(j) => s.replace_range(i..i + j + 1, ""),
            None => break,
        }
    }
    s.split_whitespace().filter(|w| !w.contains("://")).collect::<Vec<_>>().join(" ")
}

fn is_bot(author: &str) -> bool {
    let a = author.to_ascii_lowercase();
    a.contains("[bot]") || a.contains("dependabot") || a.contains("renovate")
}

// ───────────────────────────── metrics ─────────────────────────────

#[derive(Default, Clone)]
struct Row {
    sha: String,
    subject: String,
    n_gold: usize,
    hit1: f64,
    hit5: f64,
    hit10: f64,
    r5: f64,
    r10: f64,
    mrr: f64,
    conf: String,
    took_ms: f64,
    leak: bool,
    first_gold_rank: Option<usize>,
    feat: Option<codly_core::index::Features>,
    phase_ms: [f64; 6],
}

fn score(ranked: &[String], gold: &HashSet<String>) -> (f64, f64, f64, f64, f64, f64, Option<usize>) {
    let at = |k: usize| ranked.iter().take(k).filter(|p| gold.contains(*p)).count() as f64;
    let first = ranked.iter().take(50).position(|p| gold.contains(p));
    (
        (at(1) > 0.0) as u8 as f64,
        (at(5) > 0.0) as u8 as f64,
        (at(10) > 0.0) as u8 as f64,
        at(5) / gold.len() as f64,
        at(10) / gold.len() as f64,
        first.map_or(0.0, |i| 1.0 / (i as f64 + 1.0)),
        first,
    )
}

fn mean(xs: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.collect();
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

/// 95 % bootstrap interval of the mean over commits.
fn ci(xs: &[f64]) -> (f64, f64) {
    if xs.len() < 2 {
        return (0.0, 0.0);
    }
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut means: Vec<f64> =
        (0..1000).map(|_| (0..xs.len()).map(|_| xs[(next() % xs.len() as u64) as usize]).sum::<f64>() / xs.len() as f64).collect();
    means.sort_by(f64::total_cmp);
    (pct(&means, 0.025), pct(&means, 0.975))
}

fn conf_name(c: Confidence) -> &'static str {
    match c {
        Confidence::Exact => "exact",
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

// ───────────────────────────── naive grep baseline ─────────────────────────────

/// What an agent's first `grep` looks like: files containing the most distinct query words.
struct Grep {
    files: HashMap<String, Vec<u8>>,
}

const GREP_STOP: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "add", "adds", "fix", "fixes", "update", "use", "uses", "make", "when", "not",
    "new", "remove", "support", "allow", "into", "are", "was",
];

impl Grep {
    fn rank(&self, query: &str) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for w in query.split(|c: char| !c.is_ascii_alphanumeric()) {
            let w = w.to_ascii_lowercase();
            if w.len() >= 3 && !GREP_STOP.contains(&w.as_str()) && !words.contains(&w) {
                words.push(w);
            }
        }
        words.truncate(8);
        let mut scored: Vec<(usize, &String)> = self
            .files
            .iter()
            .map(|(p, text)| (words.iter().filter(|w| memchr_find(text, w.as_bytes())).count(), p))
            .filter(|(n, _)| *n > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        scored.into_iter().take(50).map(|(_, p)| p.clone()).collect()
    }
}

fn memchr_find(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

fn load_grep_file(idx: &Index, root: &Path, rel: &str, into: &mut HashMap<String, Vec<u8>>) {
    if !idx.is_indexed(rel) {
        into.remove(rel);
        return;
    }
    if let Ok(b) = std::fs::read(root.join(rel)) {
        into.insert(rel.to_string(), b.to_ascii_lowercase());
    }
}

// ───────────────────────────── main ─────────────────────────────

fn main() {
    let args = parse_args();
    if args.rest.first().map(String::as_str) == Some("search") {
        return search_cmd(&args);
    }
    let _ = std::fs::remove_dir_all(&args.work);
    std::fs::create_dir_all(&args.work).expect("work dir");
    let snap = args.work.join("snapshot");
    let db = args.work.join("index.sqlite");

    let mut base = Tuning::default();
    for (k, v) in &args.tune {
        assert!(base.set(k, v), "unknown tuning key or value: {k}={v}");
    }
    let mut variants: Vec<(String, Tuning)> = Vec::new();
    for (name, kvs) in &args.variants {
        let mut t = base.clone();
        for (k, v) in kvs {
            assert!(t.set(k, v), "unknown tuning key or value: {k}={v}");
        }
        variants.push((name.clone(), t));
    }
    if variants.is_empty() {
        variants.push(("codly index".to_string(), base.clone()));
    }

    let all = commits(&args.repo, args.n, !args.all_parents);
    println!(
        "repo {}  commits selected: {}  query: {}",
        args.repo.display(),
        all.len(),
        if args.full { "full message" } else { "subject" }
    );
    let first = all.first().expect("no commits");
    assert!(!first.parent.is_empty(), "the oldest selected commit has no parent in this clone (shallow?); lower --n or deepen the clone");

    // Parent snapshot of the oldest commit.
    let t0 = Instant::now();
    extract(&args.repo, &first.parent, &snap, &[]);
    let t_extract = t0.elapsed();
    let idx = Index::open_with(&snap, &db, &args.opts).expect("open index");
    let t1 = Instant::now();
    let n_files = idx.refresh().expect("refresh");
    let t_build = t1.elapsed();
    let db_mb = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
    println!(
        "base snapshot {}: extract {:.1}s, index {} files in {:.1}s, db {:.0} MB",
        &first.parent[..10],
        t_extract.as_secs_f64(),
        n_files,
        t_build.as_secs_f64(),
        db_mb
    );

    let mut grep = args.grep.then(|| {
        let mut files = HashMap::new();
        let rels: Vec<String> = walk_rel(&snap);
        for r in rels {
            load_grep_file(&idx, &snap, &r, &mut files);
        }
        Grep { files }
    });

    let mut cur = first.parent.clone();
    let mut rows: Vec<Vec<Row>> = variants.iter().map(|_| Vec::new()).collect();
    let mut grep_rows: Vec<Row> = Vec::new();
    let mut skipped: HashMap<&'static str, usize> = HashMap::new();
    let mut update_ms: Vec<f64> = Vec::new();

    for c in &all {
        if c.parent.is_empty() {
            *skipped.entry("no_parent").or_default() += 1;
            continue;
        }
        // Move the snapshot to this commit's parent.
        if cur != c.parent {
            let t = Instant::now();
            let diff = name_status(&args.repo, &cur, &c.parent);
            let changed = apply_diff(&args.repo, &c.parent, &snap, &diff);
            idx.update_paths(&changed).expect("update_paths");
            if let Some(g) = grep.as_mut() {
                for (_, p) in &diff {
                    load_grep_file(&idx, &snap, p, &mut g.files);
                }
            }
            update_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            cur = c.parent.clone();
        }
        if is_bot(&c.author) {
            *skipped.entry("bot").or_default() += 1;
            continue;
        }
        let query = if args.full { clean_message(&format!("{}\n{}", c.subject, c.body)) } else { clean_message(&c.subject) };
        if query.split_whitespace().count() < 4 {
            *skipped.entry("short_message").or_default() += 1;
            continue;
        }
        let diff = name_status(&args.repo, &c.parent, &c.sha);
        let gold: HashSet<String> =
            diff.iter().filter(|(s, _)| matches!(s, 'M' | 'D' | 'T')).map(|(_, p)| p.clone()).filter(|p| idx.is_indexed(p)).collect();
        if gold.is_empty() {
            *skipped.entry("no_retrievable_gold").or_default() += 1;
            continue;
        }
        if gold.len() > args.max_gold {
            *skipped.entry("gold_too_large").or_default() += 1;
            continue;
        }

        let q_low = query.to_ascii_lowercase();
        let leak = gold.iter().any(|p| {
            let stem = p.rsplit('/').next().unwrap_or(p).split('.').next().unwrap_or("").to_ascii_lowercase();
            stem.len() >= 4 && q_low.contains(&stem)
        });
        for (vi, (_, tuning)) in variants.iter().enumerate() {
            idx.set_tuning(tuning.clone());
            let t = Instant::now();
            let (res, feat) = idx.search_explain(&query, args.k).expect("search");
            let took = t.elapsed().as_secs_f64() * 1000.0;
            let mut ranked: Vec<String> = Vec::new();
            for ci in &res.citations {
                if !ranked.contains(&ci.path) {
                    ranked.push(ci.path.clone());
                }
            }
            let (hit1, hit5, hit10, r5, r10, mrr, first_gold) = score(&ranked, &gold);
            rows[vi].push(Row {
                sha: c.sha[..10].to_string(),
                subject: c.subject.chars().take(100).collect(),
                n_gold: gold.len(),
                hit1,
                hit5,
                hit10,
                r5,
                r10,
                mrr,
                conf: conf_name(res.confidence).to_string(),
                took_ms: took,
                leak,
                first_gold_rank: first_gold.map(|r| r + 1),
                phase_ms: feat.phase_ms,
                feat: (vi == 0).then_some(feat),
            });
        }
        if let Some(g) = &grep {
            let gr = g.rank(&query);
            let (h1, h5, h10, r5g, r10g, mrrg, _) = score(&gr, &gold);
            grep_rows.push(Row { hit1: h1, hit5: h5, hit10: h10, r5: r5g, r10: r10g, mrr: mrrg, ..Row::default() });
        }
    }

    report(&variants, &rows, &grep_rows, &skipped, &update_ms, all.len());
    if let Some(p) = &args.json {
        write_json(p, &args, &variants, &rows);
    }
    if let Some(p) = &args.tsv {
        let mut f = std::fs::File::create(p).expect("create tsv");
        for ((name, _), rs) in variants.iter().zip(&rows) {
            writeln!(
                f,
                "{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.1}",
                args.repo.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                name,
                rs.len(),
                mean(rs.iter().map(|r| r.hit1)),
                mean(rs.iter().map(|r| r.hit5)),
                mean(rs.iter().map(|r| r.hit10)),
                mean(rs.iter().map(|r| r.mrr)),
                mean(rs.iter().map(|r| r.r10)),
                {
                    let mut l: Vec<f64> = rs.iter().map(|r| r.took_ms).collect();
                    l.sort_by(f64::total_cmp);
                    pct(&l, 0.5)
                },
            )
            .unwrap();
        }
    }
    if !args.keep {
        let _ = std::fs::remove_dir_all(&args.work);
    }
}

fn walk_rel(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(p);
            } else if t.is_file()
                && let Ok(r) = p.strip_prefix(root)
            {
                out.push(r.to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// Apply `diff` (status, path) to the snapshot directory so it equals `rev`; returns the
/// absolute paths touched, for `Index::update_paths`.
fn apply_diff(repo: &Path, rev: &str, snap: &Path, diff: &[(char, String)]) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    let mut to_extract: Vec<String> = Vec::new();
    for (s, p) in diff {
        let abs = snap.join(p);
        if *s == 'D' {
            let _ = std::fs::remove_file(&abs);
        } else {
            // a path that was a directory (or symlink) and is now a file, and vice versa
            let _ = std::fs::remove_file(&abs);
            to_extract.push(p.clone());
        }
        changed.push(abs);
    }
    for chunk in to_extract.chunks(200) {
        extract(repo, rev, snap, chunk);
    }
    changed
}

fn report(
    variants: &[(String, Tuning)],
    all_rows: &[Vec<Row>],
    grep_rows: &[Row],
    skipped: &HashMap<&'static str, usize>,
    update_ms: &[f64],
    selected: usize,
) {
    let rows = &all_rows[0];
    if rows.is_empty() {
        println!("no evaluable commits (skipped: {skipped:?})");
        return;
    }
    println!("\nevaluated {} of {} commits; skipped {:?}", rows.len(), selected, skipped);
    let median_gold = {
        let mut g: Vec<f64> = rows.iter().map(|r| r.n_gold as f64).collect();
        g.sort_by(f64::total_cmp);
        pct(&g, 0.5)
    };
    println!(
        "median gold files per commit: {median_gold:.0}; query leakage (a gold file's basename appears in the query): {:.0}%",
        100.0 * mean(rows.iter().map(|r| r.leak as u8 as f64))
    );

    println!("\n| method | Hit@1 | Hit@5 (95% CI) | Hit@10 | R@5 | R@10 | MRR (95% CI) | ms p50/p95 |");
    println!("|---|---|---|---|---|---|---|---|");
    let line = |name: &str, rs: &[Row]| {
        let h5: Vec<f64> = rs.iter().map(|r| r.hit5).collect();
        let mrr: Vec<f64> = rs.iter().map(|r| r.mrr).collect();
        let (h5l, h5h) = ci(&h5);
        let (ml, mh) = ci(&mrr);
        let mut lat: Vec<f64> = rs.iter().map(|r| r.took_ms).collect();
        lat.sort_by(f64::total_cmp);
        println!(
            "| {name} | {:.3} | {:.3} ({:.2}–{:.2}) | {:.3} | {:.3} | {:.3} | {:.3} ({:.2}–{:.2}) | {:.0}/{:.0} |",
            mean(rs.iter().map(|r| r.hit1)),
            mean(h5.iter().copied()),
            h5l,
            h5h,
            mean(rs.iter().map(|r| r.hit10)),
            mean(rs.iter().map(|r| r.r5)),
            mean(rs.iter().map(|r| r.r10)),
            mean(mrr.iter().copied()),
            ml,
            mh,
            pct(&lat, 0.5),
            pct(&lat, 0.95)
        )
    };
    for ((name, _), rs) in variants.iter().zip(all_rows) {
        line(name, rs);
    }
    if !grep_rows.is_empty() {
        line("naive grep", grep_rows);
    }

    for ((name, _), rs) in variants.iter().zip(all_rows).take(1) {
        println!("\nconfidence calibration ({name})");
        println!("| confidence | n | share | Hit@1 | Hit@5 | MRR |");
        println!("|---|---|---|---|---|---|");
        for level in ["exact", "high", "medium", "low"] {
            let sel: Vec<&Row> = rs.iter().filter(|r| r.conf == level).collect();
            if sel.is_empty() {
                continue;
            }
            println!(
                "| {level} | {} | {:.0}% | {:.3} | {:.3} | {:.3} |",
                sel.len(),
                100.0 * sel.len() as f64 / rs.len() as f64,
                mean(sel.iter().map(|r| r.hit1)),
                mean(sel.iter().map(|r| r.hit5)),
                mean(sel.iter().map(|r| r.mrr))
            );
        }
    }

    let phase_mean: Vec<f64> = (0..6).map(|i| mean(rows.iter().map(|r| r.phase_ms[i]))).collect();
    println!(
        "\nmean ms per phase: exact {:.1} | files ranker {:.1} | term stats {:.1} | AND ladder {:.1} | OR chunks {:.1} | fuse+citations {:.1}",
        phase_mean[0], phase_mean[1], phase_mean[2], phase_mean[3], phase_mean[4], phase_mean[5]
    );
    let mut lat: Vec<f64> = rows.iter().map(|r| r.took_ms).collect();
    lat.sort_by(f64::total_cmp);
    let mut upd = update_ms.to_vec();
    upd.sort_by(f64::total_cmp);
    println!(
        "\nsearch latency ms: p50 {:.1}  p95 {:.1}  max {:.1}   |   snapshot update ms/commit (extract + update_paths): p50 {:.0}  p95 {:.0}",
        pct(&lat, 0.5),
        pct(&lat, 0.95),
        lat.last().copied().unwrap_or(0.0),
        pct(&upd, 0.5),
        pct(&upd, 0.95)
    );
}

fn write_json(path: &Path, args: &Args, variants: &[(String, Tuning)], all: &[Vec<Row>]) {
    let mut f = std::fs::File::create(path).expect("create json");
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    writeln!(f, "{{\"repo\":\"{}\",\"rows\":[", esc(&args.repo.display().to_string())).unwrap();
    for i in 0..all[0].len() {
        let r = &all[0][i];
        let ranks: Vec<String> = variants
            .iter()
            .zip(all)
            .map(|((name, _), rs)| format!("\"{}\":{}", esc(name), rs[i].first_gold_rank.map_or("null".to_string(), |x| x.to_string())))
            .collect();
        let confs: Vec<String> = variants.iter().zip(all).map(|((name, _), rs)| format!("\"{}\":\"{}\"", esc(name), rs[i].conf)).collect();
        writeln!(
            f,
            "{}{{\"sha\":\"{}\",\"subject\":\"{}\",\"n_gold\":{},\"ranks\":{{{}}},\"confidence\":{{{}}},\"leak\":{},\"feat\":{}}}",
            if i > 0 { "," } else { "" },
            r.sha,
            esc(&r.subject),
            r.n_gold,
            ranks.join(","),
            confs.join(","),
            r.leak,
            r.feat.as_ref().map_or("null".to_string(), |f| format!(
                "{{\"n_terms\":{},\"n_rare\":{},\"agree3\":{},\"agree5\":{},\"agree10\":{},\"or_top1\":{:.3},\"or_top2\":{:.3},\"or_top5\":{:.3},\"and_best_n\":{},\"and_hits\":{},\"top_and_n\":{},\"top_in_both\":{},\"dirs5\":{},\"min_df\":{}}}",
                f.n_terms, f.n_rare, f.agree3, f.agree5, f.agree10, f.or_top1, f.or_top2, f.or_top5, f.and_best_n, f.and_hits, f.top_and_n, f.top_in_both, f.dirs5, f.min_df
            ))
        )
        .unwrap();
    }
    writeln!(f, "]}}").unwrap();
}

/// `search <repo> <query…>`: index the working tree (cached next to the work dir) and print
/// what the orchestrator would inject.
fn search_cmd(args: &Args) {
    let repo = PathBuf::from(args.rest.get(1).expect("search <repo> <query>"));
    let query = args.rest[2..].join(" ");
    std::fs::create_dir_all(&args.work).expect("work dir");
    let db = args.work.join("search.sqlite");
    let idx = Index::open_with(&repo, &db, &args.opts).expect("open");
    let mut tuning = Tuning::default();
    for (k, v) in &args.tune {
        assert!(tuning.set(k, v), "unknown tuning key or value: {k}={v}");
    }
    idx.set_tuning(tuning);
    let t = Instant::now();
    let n = idx.refresh().expect("refresh");
    println!("refresh: {n} files (re)indexed in {:.2}s; {} files total", t.elapsed().as_secs_f64(), idx.file_count());
    let t = Instant::now();
    let (pack, res) = idx.pointer_pack(&query).expect("pack");
    println!("confidence {:?} in {:.1} ms (reported {} ms)\n", res.confidence, t.elapsed().as_secs_f64() * 1000.0, res.took_ms);
    println!("{pack}");
    println!("--- citations ---");
    for c in &res.citations {
        println!("{} {}:{}-{} {:?} score {:.4}\n      {}", c.id, c.path, c.start_line, c.end_line, c.symbol, c.score, c.why.join(" | "));
    }
    println!("fallback_rg: {:?}", res.fallback_rg);
}
