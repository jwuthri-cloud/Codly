//! Code index: exact symbol and path hits, concept queries, confidence levels, pointer packs,
//! incremental updates, ignore rules and concurrency, on a small multi-language repo.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use codly_core::git::Git;
use codly_core::index::{Index, WorktreeView};
use codly_core::model::{Confidence, SearchResult};

/// Your own git settings (`~/.gitconfig`, the system file) reach no git of this test: neither
/// the core's (`core_env`) nor the test's own, which commits as a fixed author.
const NO_GIT_CONFIG: [(&str, &str); 2] = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")];

/// Who the test's own commits are by, whatever the repository says.
const TEST_AUTHOR: [(&str, &str); 4] =
    [("GIT_AUTHOR_NAME", "Test"), ("GIT_AUTHOR_EMAIL", "test@example.com"), ("GIT_COMMITTER_NAME", "Test"), ("GIT_COMMITTER_EMAIL", "test@example.com")];

/// The environment the core runs with: this process's, without your git settings.
fn core_env() -> std::collections::HashMap<String, String> {
    std::env::vars().chain(NO_GIT_CONFIG.map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// Same-size rewrites inside one second would not move mtime on every filesystem.
fn bump_mtime(p: &Path) {
    let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
}

struct Fixture {
    repo: tempfile::TempDir,
    _data: tempfile::TempDir,
    index: Index,
}

fn fixture() -> Fixture {
    let repo = tempfile::tempdir().unwrap();
    let r = repo.path();
    write(
        r,
        "src/net/upload.rs",
        "//! Upload client: retries failed uploads with exponential backoff.\n\
         \n\
         pub struct UploadClient {\n    base_url: String,\n}\n\
         \n\
         impl UploadClient {\n\
         \x20   /// Send one file; retries with exponential backoff when the server answers 503.\n\
         \x20   pub async fn send(&self, path: &str) -> Result<(), Error> {\n\
         \x20       let mut delay = 100;\n\
         \x20       for attempt in 0..5 {\n\
         \x20           if self.try_send(path).await.is_ok() {\n\
         \x20               return Ok(());\n\
         \x20           }\n\
         \x20           sleep(delay).await;\n\
         \x20           delay *= 2;\n\
         \x20       }\n\
         \x20       Err(Error::Exhausted)\n\
         \x20   }\n\
         }\n\
         \n\
         pub fn retained_sampling(rate: f64) -> f64 {\n    rate.clamp(0.0, 1.0)\n}\n",
    );
    write(
        r,
        "src/web/app.ts",
        "// Web app shell: routing and session handling.\n\
         export class SessionStore {\n  private ttl = 3600;\n  load(id: string): Session | null {\n    return this.cache.get(id) ?? null;\n  }\n}\n\
         \n\
         export function parseConfig(raw: string): Config {\n  return JSON.parse(raw);\n}\n\
         \n\
         export const renderPage = (route: string) => {\n  return `<main>${route}</main>`;\n};\n",
    );
    write(
        r,
        "lib/billing/invoice.py",
        "\"\"\"Invoices: totals, tax and rounding rules.\"\"\"\n\n\
         TAX_RATE = 0.2\n\n\
         class Invoice:\n    def __init__(self, lines):\n        self.lines = lines\n\n\
         \x20   def compute_total(self):\n        subtotal = sum(l.price for l in self.lines)\n        return round(subtotal * (1 + TAX_RATE), 2)\n",
    );
    write(
        r,
        "pkg/server/server.go",
        "package server\n\n// Server accepts connections and dispatches requests.\ntype Server struct {\n\tAddr string\n}\n\n\
         func (s *Server) Start() error {\n\treturn nil\n}\n\nfunc NewServer(addr string) *Server {\n\treturn &Server{Addr: addr}\n}\n",
    );
    write(
        r,
        "App/Store.swift",
        "import Foundation\n\npublic final class ReceiptStore {\n    public func persist(receipt: Data) {\n    }\n}\n",
    );
    write(
        r,
        "src/main/java/Cache.java",
        "public class Cache {\n    public static int evictOldest(int count) {\n        return count;\n    }\n}\n",
    );
    write(r, "lib/ruby/ledger.rb", "module Ledger\n  class Entry\n    def reconcile!\n      true\n    end\n  end\nend\n");
    write(
        r,
        "include/net.h",
        "#define NET_MAX_PACKET 1500\nstruct net_packet {\n    int len;\n};\nint net_send_packet(struct net_packet *p);\n",
    );
    write(
        r,
        "docs/guide.md",
        "# Operations guide\n\n## Rotating credentials\n\nRotate the signing key every 90 days.\n\n## Restoring backups\n\nRun the restore job.\n",
    );
    write(r, ".gitignore", "ignored.txt\n/build/\n");
    write(r, "ignored.txt", "retained sampling secret\n");
    write(r, "build/out.js", "function bundled() {}\n");
    write(r, "node_modules/dep/index.js", "function dependency() {}\n");
    write(r, "Cargo.lock", "[[package]]\nname = \"retained\"\n");
    write(r, "assets/logo.bin", "binary\0blob");
    let data = tempfile::tempdir().unwrap();
    let index = Index::open(r, &data.path().join("index").join("ws.sqlite")).unwrap();
    Fixture { repo, _data: data, index }
}

fn paths(r: &SearchResult) -> Vec<&str> {
    r.citations.iter().map(|c| c.path.as_str()).collect()
}

#[test]
fn index_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Index>();
}

#[test]
fn refresh_indexes_source_and_skips_ignored_binary_and_lock_files() {
    let f = fixture();
    let n = f.index.refresh().unwrap();
    assert_eq!(n, 10, "9 sources + .gitignore; ignored, vendored, lock and binary files excluded");
    assert_eq!(f.index.file_count(), 10);
    assert_eq!(f.index.refresh().unwrap(), 0, "nothing changed");
}

#[test]
fn exact_symbol_hit_returns_the_definition_span() {
    let f = fixture();
    f.index.refresh().unwrap();
    let r = f.index.search("retained_sampling", 5).unwrap();
    assert_eq!(r.confidence, Confidence::Exact);
    let c = &r.citations[0];
    assert_eq!(c.path, "src/net/upload.rs");
    assert_eq!(c.symbol.as_deref(), Some("retained_sampling"));
    assert_eq!((c.start_line, c.end_line), (22, 24));
    assert_eq!(c.id, "C1");
    assert!(c.why[0].contains("exact symbol"), "{:?}", c.why);
    assert!(r.fallback_rg.is_empty());
}

#[test]
fn qualified_names_pick_the_right_container() {
    let f = fixture();
    f.index.refresh().unwrap();
    let r = f.index.search("UploadClient::send", 5).unwrap();
    assert_eq!(r.confidence, Confidence::Exact);
    assert_eq!(r.citations[0].symbol.as_deref(), Some("UploadClient::send"));
    assert_eq!(r.citations[0].start_line, 9);
    let r = f.index.search("where is Server.Start called?", 5).unwrap();
    assert_eq!(r.confidence, Confidence::Exact);
    assert_eq!(r.citations[0].path, "pkg/server/server.go");
}

#[test]
fn symbols_of_every_supported_language_are_found() {
    let f = fixture();
    f.index.refresh().unwrap();
    for (query, path) in [
        ("compute_total", "lib/billing/invoice.py"),
        ("parseConfig", "src/web/app.ts"),
        ("evictOldest()", "src/main/java/Cache.java"),
        ("reconcile!", "lib/ruby/ledger.rb"),
        ("`net_send_packet`", "include/net.h"),
        ("`ReceiptStore`", "App/Store.swift"),
        ("NET_MAX_PACKET", "include/net.h"),
    ] {
        let r = f.index.search(query, 5).unwrap();
        assert_eq!(r.confidence, Confidence::Exact, "{query}");
        assert_eq!(r.citations[0].path, path, "{query}");
    }
}

#[test]
fn exact_path_hits_by_full_path_suffix_and_line_suffix() {
    let f = fixture();
    f.index.refresh().unwrap();
    for q in ["src/net/upload.rs", "look at upload.rs please", "net/upload.rs:12-40"] {
        let r = f.index.search(q, 3).unwrap();
        assert_eq!(r.confidence, Confidence::Exact, "{q}");
        assert_eq!(r.citations[0].path, "src/net/upload.rs", "{q}");
        assert!(r.citations[0].why[0].contains("exact path"));
    }
}

#[test]
fn concept_query_ranks_the_relevant_file_first() {
    let f = fixture();
    f.index.refresh().unwrap();
    let r = f.index.search("retry failed uploads with exponential backoff", 5).unwrap();
    assert_ne!(r.confidence, Confidence::Exact);
    assert_eq!(r.citations[0].path, "src/net/upload.rs", "{:?}", paths(&r));
    assert!(r.citations[0].start_line >= 1 && r.citations[0].end_line >= r.citations[0].start_line);
    let why = r.citations[0].why.join(" | ");
    assert!(why.contains("backoff") || why.contains("path matches"), "mechanical evidence expected: {why}");

    let r = f.index.search("rotate the signing key credentials", 5).unwrap();
    assert_eq!(r.citations[0].path, "docs/guide.md");
    assert_eq!(r.citations[0].symbol.as_deref(), Some("Rotating credentials"), "markdown sections name the citation");
}

#[test]
fn camel_and_snake_case_find_each_other() {
    let f = fixture();
    f.index.refresh().unwrap();
    // The file says `retained_sampling`; the query says camelCase prose.
    let r = f.index.search("clamp the retainedSampling rate", 5).unwrap();
    assert_eq!(r.citations[0].path, "src/net/upload.rs", "{:?}", paths(&r));
    // And the other way round: query in snake_case, code in camelCase.
    let r = f.index.search("render page route main", 5).unwrap();
    assert_eq!(r.citations[0].path, "src/web/app.ts", "{:?}", paths(&r));
}

#[test]
fn unknown_vocabulary_is_low_confidence_with_rg_hints_and_an_empty_pack() {
    let f = fixture();
    f.index.refresh().unwrap();
    let (pack, r) = f.index.pointer_pack("frobnicate the zxqv flurble").unwrap();
    assert_eq!(r.confidence, Confidence::Low);
    assert!(!r.fallback_rg.is_empty(), "low confidence suggests rg patterns");
    assert!(pack.contains("low confidence") && pack.contains("rg -n -i"), "{pack}");
    assert!(!pack.contains("[C1]"), "no pointers are injected at low confidence: {pack}");
}

#[test]
fn pointer_pack_is_one_line_per_citation_and_small() {
    let f = fixture();
    f.index.refresh().unwrap();
    let (pack, r) = f.index.pointer_pack("fix UploadClient::send retry and backoff when the server answers 503").unwrap();
    assert_eq!(r.confidence, Confidence::Exact);
    let lines: Vec<&str> = pack.lines().filter(|l| l.starts_with("[C")).collect();
    assert!(!lines.is_empty());
    assert!(lines[0].starts_with("[C1] src/net/upload.rs:9-"), "{}", lines[0]);
    assert!(lines[0].contains("UploadClient::send") && lines[0].trim_end().ends_with("(exact)"), "{}", lines[0]);
    assert!(pack.len() / 4 <= 1200, "pack must stay within ~1.2k tokens, got {} chars", pack.len());
}

#[test]
fn modified_deleted_and_new_files_are_picked_up() {
    let f = fixture();
    let r = f.repo.path();
    f.index.refresh().unwrap();

    // modify (same size: only mtime tells us)
    let p = r.join("lib/billing/invoice.py");
    let body = std::fs::read_to_string(&p).unwrap().replace("compute_total", "compute_totals");
    std::fs::write(&p, &body).unwrap();
    bump_mtime(&p);
    assert_eq!(f.index.refresh().unwrap(), 1);
    assert_eq!(f.index.search("compute_totals", 3).unwrap().confidence, Confidence::Exact);
    assert_ne!(f.index.search("compute_total", 3).unwrap().confidence, Confidence::Exact, "the old name is gone");

    // delete
    std::fs::remove_file(r.join("pkg/server/server.go")).unwrap();
    f.index.refresh().unwrap();
    assert_eq!(f.index.file_count(), 9);
    assert!(f.index.search("NewServer", 3).unwrap().citations.iter().all(|c| c.path != "pkg/server/server.go"));

    // new file, listed explicitly (what a merge gives us)
    write(r, "src/cache/lru.rs", "/// Least-recently-used cache.\npub struct LruCache {}\n");
    f.index.update_paths(&[r.join("src/cache/lru.rs")]).unwrap();
    assert_eq!(f.index.search("LruCache", 3).unwrap().citations[0].path, "src/cache/lru.rs");

    // deletion reported through update_paths, relative path
    std::fs::remove_file(r.join("src/cache/lru.rs")).unwrap();
    f.index.update_paths(&[PathBuf::from("src/cache/lru.rs")]).unwrap();
    assert!(f.index.search("LruCache", 3).unwrap().citations.iter().all(|c| c.path != "src/cache/lru.rs"));
}

#[test]
fn update_paths_handles_directories_and_respects_ignore_rules() {
    let f = fixture();
    let r = f.repo.path();
    f.index.refresh().unwrap();
    write(r, "src/gen/a.rs", "pub fn alpha_widget() {}\n");
    write(r, "src/gen/b.rs", "pub fn beta_widget() {}\n");
    write(r, "build/new.js", "function ignoredByGitignore() {}\n");
    write(r, "node_modules/dep/more.js", "function vendored() {}\n");
    f.index
        .update_paths(&[
            r.join("src/gen"),
            r.join("build/new.js"),
            r.join("node_modules/dep/more.js"),
            PathBuf::from("/definitely/outside.rs"),
        ])
        .unwrap();
    assert_eq!(f.index.search("alpha_widget", 3).unwrap().confidence, Confidence::Exact);
    assert_eq!(f.index.search("beta_widget", 3).unwrap().confidence, Confidence::Exact);
    assert_ne!(f.index.search("ignoredByGitignore", 3).unwrap().confidence, Confidence::Exact);
    assert_ne!(f.index.search("vendored", 3).unwrap().confidence, Confidence::Exact);

    // a removed directory takes its files with it
    std::fs::remove_dir_all(r.join("src/gen")).unwrap();
    f.index.update_paths(&[r.join("src/gen")]).unwrap();
    assert_ne!(f.index.search("alpha_widget", 3).unwrap().confidence, Confidence::Exact);
}

#[test]
fn hand_written_source_is_indexed_whatever_its_header_or_directory_is_called() {
    let f = fixture();
    let r = f.repo.path();
    // A header that mentions generated code, in a file a person wrote.
    write(r, "src/model.rs", "//! Shared types (TypeScript bindings are generated by ts-rs into `gen/`).\npub struct EventContract;\n");
    write(
        r,
        "src/gen/EventContract.ts",
        "// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.\nexport type EventContract = {};\n",
    );
    // First-party code in a directory that is third-party code in most repos.
    write(r, ".codlyignore", "!/src/vendor/\n");
    write(r, "src/vendor/invoice.rs", "pub fn vendor_invoice_total() {}\n");
    write(r, "vendor/dep/lib.rs", "pub fn third_party_dependency() {}\n");
    f.index.refresh().unwrap();
    assert!(f.index.is_indexed("src/model.rs"));
    assert!(!f.index.is_indexed("src/gen/EventContract.ts"));
    assert!(f.index.is_indexed("src/vendor/invoice.rs"));
    assert!(!f.index.is_indexed("vendor/dep/lib.rs"));
    assert_eq!(f.index.search("EventContract", 3).unwrap().citations[0].path, "src/model.rs");

    // A change reported by path follows the same rules as the walk.
    write(r, "src/vendor/refund.rs", "pub fn vendor_refund_total() {}\n");
    write(r, "vendor/dep/more.rs", "pub fn another_dependency() {}\n");
    f.index.update_paths(&[r.join("src/vendor/refund.rs"), r.join("vendor/dep/more.rs"), r.join("vendor")]).unwrap();
    assert!(f.index.is_indexed("src/vendor/refund.rs"));
    assert!(!f.index.is_indexed("vendor/dep/more.rs"));
    assert_eq!(f.index.refresh().unwrap(), 0, "the walk agrees: nothing left to do");
}

#[test]
fn identical_content_with_a_new_mtime_is_not_reindexed() {
    let f = fixture();
    f.index.refresh().unwrap();
    let p = f.repo.path().join("docs/guide.md");
    bump_mtime(&p);
    assert_eq!(f.index.refresh().unwrap(), 0, "content hash unchanged: only the stored mtime moves");
    assert_eq!(f.index.refresh().unwrap(), 0);
}

#[test]
fn reopening_the_database_keeps_the_index() {
    let repo = tempfile::tempdir().unwrap();
    write(repo.path(), "a.rs", "pub fn persisted_symbol() {}\n");
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("i.sqlite");
    {
        let idx = Index::open(repo.path(), &db).unwrap();
        assert_eq!(idx.refresh().unwrap(), 1);
    }
    let idx = Index::open(repo.path(), &db).unwrap();
    assert_eq!(idx.file_count(), 1);
    assert_eq!(idx.refresh().unwrap(), 0);
    assert_eq!(idx.search("persisted_symbol", 3).unwrap().confidence, Confidence::Exact);
}

#[test]
fn hostile_query_text_never_errors() {
    let f = fixture();
    f.index.refresh().unwrap();
    for q in [
        "",
        "   ",
        "\"",
        "AND OR NOT NEAR(",
        "foo* bar:baz -qux",
        "'; DROP TABLE files; --",
        "((((",
        "a b",
        "日本語 テスト",
        "x".repeat(5000).as_str(),
    ] {
        let r = f.index.search(q, 5).unwrap();
        assert!(r.citations.len() <= 5, "{q:?}");
        let (pack, _) = f.index.pointer_pack(q).unwrap();
        assert!(pack.len() < 6000);
    }
    assert!(f.index.search("anything", 0).is_ok());
}

#[test]
fn searching_while_a_refresh_runs_works() {
    let repo = tempfile::tempdir().unwrap();
    for i in 0..600 {
        write(
            repo.path(),
            &format!("src/m{}/file{i}.rs", i % 20),
            &format!("pub fn handler_number_{i}() {{\n    // process request {i}\n}}\n"),
        );
    }
    let data = tempfile::tempdir().unwrap();
    let idx = std::sync::Arc::new(Index::open(repo.path(), &data.path().join("i.sqlite")).unwrap());
    let worker = {
        let idx = idx.clone();
        std::thread::spawn(move || idx.refresh().unwrap())
    };
    let mut searches = 0;
    while !worker.is_finished() {
        idx.search("handler number request", 5).unwrap();
        idx.file_count();
        searches += 1;
    }
    assert_eq!(worker.join().unwrap(), 600);
    assert!(searches > 0);
    assert_eq!(idx.search("handler_number_42", 3).unwrap().confidence, Confidence::Exact);
}

#[test]
fn repeated_queries_are_deterministic() {
    let f = fixture();
    f.index.refresh().unwrap();
    let a = f.index.search("session store cache load", 8).unwrap();
    let b = f.index.search("session store cache load", 8).unwrap();
    assert_eq!(paths(&a), paths(&b));
    assert_eq!(a.confidence, b.confidence);
}

// ───────────── the index against the files on disk ─────────────

#[test]
fn a_new_file_is_found_by_the_next_search_that_is_due() {
    let f = fixture();
    let r = f.repo.path();
    f.index.refresh().unwrap();
    assert!(f.index.search("UploadClient", 3).unwrap().warnings.is_empty(), "an index that matches the files warns about nothing");

    // Written in an editor while Codly runs: nobody tells the index.
    write(r, "src/cache/lru.rs", "/// Least-recently-used cache.\npub struct LruCache {}\n");
    f.index.refresh_if_due(Duration::from_secs(3600), Instant::now()).unwrap();
    assert!(!f.index.is_indexed("src/cache/lru.rs"), "the files were compared a moment ago: not again yet");
    f.index.refresh_if_due(Duration::ZERO, Instant::now()).unwrap();
    let found = f.index.search("LruCache", 3).unwrap();
    assert_eq!(found.citations[0].path, "src/cache/lru.rs");
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
}

#[test]
fn a_result_says_how_old_the_index_is_when_it_could_not_be_refreshed() {
    let f = fixture();
    let r = f.repo.path();
    let stale = f.index.search("UploadClient", 3).unwrap();
    assert!(stale.warnings[0].contains("has not been updated yet"), "never refreshed: {:?}", stale.warnings);
    f.index.refresh().unwrap();

    // The checkout goes away (a disk that is unmounted, a folder that is moved).
    let away = r.with_extension("away");
    std::fs::rename(r, &away).unwrap();
    assert!(f.index.refresh_if_due(Duration::ZERO, Instant::now()).is_err());
    let kept = f.index.search("UploadClient", 3).unwrap();
    assert_eq!(kept.citations[0].path, "src/net/upload.rs", "what was indexed is kept, not thrown away");
    let w = &kept.warnings[0];
    assert!(w.contains("could not be updated") && w.contains("is not there") && w.contains("it last read all the files"), "{w}");
    let (pack, _) = f.index.pointer_pack("fix UploadClient::send retry and backoff").unwrap();
    assert!(pack.contains("Note: The code index could not be updated"), "a task brief says it too: {pack}");

    // Back again: the next refresh works and the warning goes.
    std::fs::rename(&away, r).unwrap();
    f.index.refresh_if_due(Duration::ZERO, Instant::now()).unwrap();
    assert!(f.index.search("UploadClient", 3).unwrap().warnings.is_empty());
}

#[test]
fn an_index_opened_again_says_that_it_is_from_an_earlier_run() {
    let repo = tempfile::tempdir().unwrap();
    write(repo.path(), "a.rs", "pub fn persisted_symbol() {}\n");
    let data = tempfile::tempdir().unwrap();
    let db = data.path().join("i.sqlite");
    Index::open(repo.path(), &db).unwrap().refresh().unwrap();

    let idx = Index::open(repo.path(), &db).unwrap();
    let before = idx.search("persisted_symbol", 3).unwrap();
    assert_eq!(before.confidence, Confidence::Exact);
    let w = &before.warnings[0];
    assert!(w.contains("has not been updated yet") && w.contains("it last read all the files") && w.contains("ago"), "{w}");
    idx.refresh().unwrap();
    assert!(idx.search("persisted_symbol", 3).unwrap().warnings.is_empty());
}

// ───────────── a search asked from an agent's worktree ─────────────

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").envs(NO_GIT_CONFIG).envs(TEST_AUTHOR).arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A git repository (the user's checkout, on `main`) with its index, and a task's worktree
/// of it on another branch.
struct Checkouts {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    worktree: PathBuf,
    index: Index,
}

fn checkouts() -> Checkouts {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    write(&repo, "src/net/upload.rs", "pub struct UploadClient {\n    base_url: String,\n}\n");
    write(&repo, "src/web/session.ts", "export class SessionStore {\n  private ttl = 3600;\n}\n");
    write(&repo, "lib/billing/invoice.py", "class Invoice:\n    def compute_total(self):\n        return 0\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let index = Index::open(&repo, &dir.path().join("index.sqlite")).unwrap();
    index.refresh().unwrap();
    let worktree = dir.path().join("task-worktree");
    git(&repo, &["worktree", "add", "-q", "-b", "codly/t1", worktree.to_str().unwrap()]);
    Checkouts { _dir: dir, repo, worktree, index }
}

fn marked(r: &SearchResult) -> Vec<&str> {
    r.citations.iter().filter(|c| c.why.iter().any(|w| w.contains("differs in your worktree"))).map(|c| c.path.as_str()).collect()
}

#[tokio::test]
async fn a_search_from_a_worktree_says_what_differs_there() {
    let c = checkouts();
    let (repo, wt) = (&c.repo, &c.worktree);
    assert!(c.index.covers(repo) && c.index.covers(&repo.join("src")) && !c.index.covers(wt));
    // The task's branch changed one file, removed one and added one; one more file is not
    // committed yet, and a lockfile is never indexed.
    write(wt, "src/net/upload.rs", "// Moved down by the task.\n\n\npub struct UploadClient {\n    base_url: String,\n}\n");
    std::fs::remove_file(wt.join("lib/billing/invoice.py")).unwrap();
    write(wt, "src/cache/lru.rs", "pub struct LruCache {}\n");
    git(wt, &["add", "-A"]);
    git(wt, &["commit", "-qm", "task"]);
    write(wt, "src/cache/ttl.rs", "pub struct TtlCache {}\n");
    write(wt, "Cargo.lock", "[[package]]\n");
    let cli = Git::new("git".into(), core_env());
    let view = WorktreeView::read(&cli, repo, wt, Duration::from_secs(60)).await;

    // A file the task changed: cited, and marked, because its lines are the checkout's.
    let r = c.index.search_in(&view, "UploadClient", 5).unwrap();
    assert_eq!(r.citations[0].path, "src/net/upload.rs");
    assert!(r.citations[0].why[0].contains("differs in your worktree: read the file"), "{:?}", r.citations[0].why);
    assert!(r.warnings.iter().any(|w| w.contains("differ in your worktree") && w.contains("main@")), "{:?}", r.warnings);
    // The files that exist only there are named: the index cannot search them.
    let new = r.warnings.iter().find(|w| w.contains("new in your worktree")).expect("the new files are named");
    assert!(new.contains("src/cache/lru.rs") && new.contains("src/cache/ttl.rs") && !new.contains("Cargo.lock"), "{new}");
    assert!(!new.contains("upload.rs") && !new.contains("invoice.py"), "changed and removed files are not new: {new}");

    // A file the task removed: its matches are left out, and the result does not stay "exact".
    assert_eq!(c.index.search("compute_total", 5).unwrap().confidence, Confidence::Exact, "it is there in the checkout");
    let r = c.index.search_in(&view, "compute_total", 5).unwrap();
    assert!(!paths(&r).contains(&"lib/billing/invoice.py"), "{:?}", paths(&r));
    assert_ne!(r.confidence, Confidence::Exact);
    assert!(r.warnings.iter().any(|w| w.contains("left out")), "{:?}", r.warnings);
    // With the lower confidence come the patterns to grep the worktree with.
    assert!(r.fallback_rg.iter().any(|p| p.contains("total")), "{:?}", r.fallback_rg);
    let (pack, _) = c.index.pointer_pack_in(&view, "compute_total").unwrap();
    assert!(pack.contains("rg -n -i '") && !pack.contains("<the rarest words"), "{pack}");

    // A file that is the same on both sides: cited as it is.
    let r = c.index.search_in(&view, "SessionStore", 5).unwrap();
    assert_eq!(r.citations[0].path, "src/web/session.ts");
    assert!(marked(&r).is_empty(), "{:?}", r.citations);

    // You edit that file in your checkout and do not commit: now the two differ.
    write(repo, "src/web/session.ts", "// Sessions.\nexport class SessionStore {\n  private ttl = 60;\n}\n");
    c.index.refresh().unwrap();
    let view = WorktreeView::read(&cli, repo, wt, Duration::from_secs(60)).await;
    let r = c.index.search_in(&view, "SessionStore", 5).unwrap();
    assert_eq!(marked(&r).first(), Some(&"src/web/session.ts"), "{:?}", r.citations);

    // The pointer pack of a task brief carries the mark and the notes.
    let (pack, _) = c.index.pointer_pack_in(&view, "UploadClient").unwrap();
    assert!(pack.contains("[C1] src/net/upload.rs") && pack.contains("differs in your worktree"), "{pack}");
    assert!(pack.contains("Note: Not searched, because they are new in your worktree: src/cache/lru.rs"), "{pack}");
    // A pack without pointers says to search with rg, which reads the worktree: no notes.
    let (pack, r) = c.index.pointer_pack_in(&view, "rotate the signing key credentials").unwrap();
    assert_eq!(r.confidence, Confidence::Low);
    assert!(pack.contains("low confidence") && !pack.contains("Note:"), "{pack}");

    // Asked from the checkout itself, nothing is marked.
    let r = c.index.search("UploadClient", 5).unwrap();
    assert!(r.warnings.is_empty() && marked(&r).is_empty(), "{r:?}");
}

#[tokio::test]
async fn a_worktree_that_git_cannot_compare_is_named_in_the_result() {
    let c = checkouts();
    // Not a checkout of this repository.
    let elsewhere = tempfile::tempdir().unwrap();
    write(elsewhere.path(), "src/net/upload.rs", "pub struct UploadClient;\n");
    let view = WorktreeView::read(&Git::new("git".into(), core_env()), &c.repo, elsewhere.path(), Duration::from_secs(60)).await;
    let r = c.index.search_in(&view, "UploadClient SessionStore", 5).unwrap();
    assert_eq!(r.citations[0].path, "src/net/upload.rs");
    assert!(!paths(&r).contains(&"src/web/session.ts"), "a match in a file that is not there is left out: {:?}", paths(&r));
    assert!(r.warnings.iter().any(|w| w.contains("could not be compared")), "{:?}", r.warnings);
    assert!(marked(&r).is_empty(), "nothing is known to differ: {:?}", r.citations);
}
