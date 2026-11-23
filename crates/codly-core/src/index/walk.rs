//! Finding the files to index: `ignore`'s walker (the one ripgrep uses) for a full refresh,
//! and a single-path ignore check for `update_paths` (the files a merge touched).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::lang;

/// 1 MiB: bigger files are data dumps or bundles, not source worth chunking.
pub(crate) const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone)]
pub(crate) struct Found {
    /// Repo-relative, `/` separated.
    pub rel: String,
    pub abs: PathBuf,
    pub mtime_ns: i64,
    pub bytes: u64,
}

pub(crate) fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as i64).unwrap_or(0)
}

pub(crate) fn rel_string(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let mut s = String::with_capacity(rel.as_os_str().len());
    for (i, c) in rel.components().enumerate() {
        if i > 0 {
            s.push('/');
        }
        s.push_str(c.as_os_str().to_str()?);
    }
    (!s.is_empty()).then_some(s)
}

/// A walker over `start`, a directory at or below the index `root`.
fn builder(root: &Path, start: &Path) -> WalkBuilder {
    // The walker applies the ignore files itself; this answers the one question it cannot:
    // did a `!` rule ask for a directory that is left out by default?
    let check = Mutex::new(IgnoreCheck::new(root));
    let mut b = WalkBuilder::new(start);
    b.hidden(false) // .github/, .eslintrc… are code; `.git` is cut by filter_entry below
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false) // global excludes would make results depend on the user's machine
        .ignore(true)
        .parents(true)
        .require_git(false) // honour .gitignore in a plain directory too
        .follow_links(false)
        .max_filesize(Some(MAX_FILE_BYTES))
        .add_custom_ignore_filename(".codlyignore")
        .filter_entry(move |e| {
            if e.depth() == 0 || !e.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            let Some(name) = e.file_name().to_str() else { return true };
            if lang::is_denied_dir(name) {
                return false;
            }
            !lang::is_default_excluded_dir(name) || check.lock().unwrap_or_else(|p| p.into_inner()).rules(e.path(), true) == Rules::Keep
        });
    b
}

/// Every candidate file under `root` (or under `sub`, a directory inside it), sorted by path
/// so runs are deterministic.
pub(crate) fn walk(root: &Path, sub: Option<&Path>) -> Vec<Found> {
    let start = sub.unwrap_or(root);
    let mut out = Vec::new();
    for entry in builder(root, start).build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel) = rel_string(root, entry.path()) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        out.push(Found { rel, abs: entry.path().to_path_buf(), mtime_ns: mtime_ns(&meta), bytes: meta.len() });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Would a full walk have returned this path? Applies the `.gitignore`, `.ignore` and
/// `.codlyignore` files of every directory from the root down to the file, deepest last,
/// plus the two directory lists of `lang`, without walking anything.
pub(crate) struct IgnoreCheck {
    root: PathBuf,
    cache: HashMap<PathBuf, Option<Gitignore>>,
}

/// What the ignore files say about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rules {
    /// A pattern ignores it.
    Ignore,
    /// A `!` pattern asks for it.
    Keep,
    /// No pattern names it.
    Silent,
}

impl IgnoreCheck {
    pub fn new(root: &Path) -> Self {
        IgnoreCheck { root: root.to_path_buf(), cache: HashMap::new() }
    }

    fn matcher(&mut self, dir: &Path) -> Option<&Gitignore> {
        self.cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                let mut b = GitignoreBuilder::new(dir);
                let mut any = false;
                // Later additions take precedence, as in the walker.
                let mut files = vec![dir.join(".gitignore"), dir.join(".ignore"), dir.join(".codlyignore")];
                if dir == self.root {
                    files.insert(0, dir.join(".git/info/exclude"));
                }
                for f in files {
                    if f.is_file() {
                        any |= b.add(f).is_none();
                    }
                }
                if any { b.build().ok() } else { None }
            })
            .as_ref()
    }

    pub fn is_ignored(&mut self, abs: &Path, is_dir: bool) -> bool {
        let Ok(rel) = abs.strip_prefix(&self.root) else { return true };
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            return true; // `root/../x` lexically starts with root but is outside it
        }
        // The lists name directories: a file may be called `dist` or `vendor`.
        let comps: Vec<_> = rel.components().collect();
        let dirs = if is_dir { comps.len() } else { comps.len().saturating_sub(1) };
        let mut dir = self.root.clone();
        for c in &comps[..dirs] {
            dir.push(c.as_os_str());
            let Some(name) = c.as_os_str().to_str() else { continue };
            if lang::is_denied_dir(name) || (lang::is_default_excluded_dir(name) && self.rules(&dir, true) != Rules::Keep) {
                return true;
            }
        }
        self.rules(abs, is_dir) == Rules::Ignore
    }

    /// The verdict of the ignore files on `abs` (under the root): the last pattern that
    /// matches it or one of its parent directories, the deepest ignore file winning.
    fn rules(&mut self, abs: &Path, is_dir: bool) -> Rules {
        let Ok(rel) = abs.strip_prefix(&self.root) else { return Rules::Silent };
        let mut verdict = Rules::Silent;
        let mut dir = self.root.clone();
        let comps: Vec<_> = rel.components().collect();
        for i in 0..comps.len() {
            if let Some(m) = self.matcher(&dir) {
                match m.matched_path_or_any_parents(abs, is_dir) {
                    ignore::Match::Ignore(_) => verdict = Rules::Ignore,
                    ignore::Match::Whitelist(_) => verdict = Rules::Keep,
                    ignore::Match::None => {}
                }
            }
            if i + 1 < comps.len() {
                dir.push(comps[i].as_os_str());
            }
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn walk_respects_ignores_without_a_git_repo() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, ".gitignore", "secret.txt\n/build/\n");
        write(r, ".codlyignore", "*.gen.rs\n");
        write(r, "src/a.rs", "fn a() {}");
        write(r, "src/b.gen.rs", "fn b() {}");
        write(r, "secret.txt", "x");
        write(r, "build/out.js", "x");
        write(r, "node_modules/x/index.js", "x");
        write(r, ".github/workflows/ci.yml", "on: push");
        write(r, ".git/config", "x");
        let got: Vec<String> = walk(r, None).into_iter().map(|f| f.rel).collect();
        assert_eq!(got, [".codlyignore", ".github/workflows/ci.yml", ".gitignore", "src/a.rs"]);
    }

    #[test]
    fn ignore_check_matches_the_walker() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, ".gitignore", "secret.txt\n/build/\n");
        write(r, "sub/.gitignore", "local.txt\n!keep.log\n*.log\n");
        let mut c = IgnoreCheck::new(r);
        assert!(c.is_ignored(&r.join("secret.txt"), false));
        assert!(c.is_ignored(&r.join("build/out.js"), false));
        assert!(c.is_ignored(&r.join("sub/local.txt"), false));
        assert!(c.is_ignored(&r.join("node_modules/a.js"), false));
        assert!(!c.is_ignored(&r.join("src/a.rs"), false));
        assert!(!c.is_ignored(&r.join("sub/a.rs"), false));
        assert!(c.is_ignored(&r.join("../elsewhere.rs"), false));
    }

    #[test]
    fn a_rule_brings_back_a_directory_that_is_left_out_by_default() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, ".codlyignore", "!/src/vendor/\n!node_modules/\n");
        write(r, "src/vendor/invoice.rs", "fn pay() {}");
        write(r, "src/vendor/dist/bundle.rs", "fn bundle() {}");
        write(r, "vendor/lib/x.go", "package x");
        write(r, "dist/app.js", "x");
        write(r, "node_modules/x/index.js", "x");
        write(r, "docs/vendor", "a file, not a directory");
        let got: Vec<String> = walk(r, None).into_iter().map(|f| f.rel).collect();
        // First-party code under a name that is usually third-party is back, with what is below
        // it; the same names elsewhere stay out, and nothing brings `node_modules` back.
        assert_eq!(got, [".codlyignore", "docs/vendor", "src/vendor/dist/bundle.rs", "src/vendor/invoice.rs"]);
        let sub: Vec<String> = walk(r, Some(&r.join("src"))).into_iter().map(|f| f.rel).collect();
        assert_eq!(sub, ["src/vendor/dist/bundle.rs", "src/vendor/invoice.rs"]);

        let mut c = IgnoreCheck::new(r);
        for f in &got {
            assert!(!c.is_ignored(&r.join(f), false), "{f}");
        }
        for f in ["vendor/lib/x.go", "dist/app.js", "node_modules/x/index.js"] {
            assert!(c.is_ignored(&r.join(f), false), "{f}");
        }
        assert!(!c.is_ignored(&r.join("src/vendor"), true));
        assert!(c.is_ignored(&r.join("vendor"), true));
    }
}
