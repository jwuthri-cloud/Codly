//! Code index: exact symbol and path hits, concept queries, confidence levels, pointer packs,
//! incremental updates, ignore rules and concurrency, on a small multi-language repo.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use codly_core::index::Index;
use codly_core::model::{Confidence, SearchResult};

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

    // new file, listed explicitly (what the watcher / a commit gives us)
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
