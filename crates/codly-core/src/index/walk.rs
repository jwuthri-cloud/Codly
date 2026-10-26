//! Finding the files to index: `ignore`'s walker (the one ripgrep uses) for a full refresh,
//! and a single-path ignore check for `update_paths` (watcher events, commits).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

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

fn builder(root: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
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
        .filter_entry(|e| {
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && e.depth() > 0 && e.file_name().to_str().is_some_and(lang::is_denied_dir))
        });
    b
}

/// Every candidate file under `root` (or under `sub`, a directory inside it), sorted by path
/// so runs are deterministic.
pub(crate) fn walk(root: &Path, sub: Option<&Path>) -> Vec<Found> {
    let start = sub.unwrap_or(root);
    let mut out = Vec::new();
    for entry in builder(start).build().flatten() {
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
/// plus the denied-directory list, without walking anything.
pub(crate) struct IgnoreCheck {
    root: PathBuf,
    cache: HashMap<PathBuf, Option<Gitignore>>,
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
        if rel.components().any(|c| c.as_os_str().to_str().is_some_and(lang::is_denied_dir)) {
            return true;
        }
        let mut ignored = false;
        let mut dir = self.root.clone();
        let comps: Vec<_> = rel.components().collect();
        for i in 0..comps.len() {
            if let Some(m) = self.matcher(&dir) {
                match m.matched_path_or_any_parents(abs, is_dir) {
                    ignore::Match::Ignore(_) => ignored = true,
                    ignore::Match::Whitelist(_) => ignored = false,
                    ignore::Match::None => {}
                }
            }
            if i + 1 < comps.len() {
                dir.push(comps[i].as_os_str());
            }
        }
        ignored
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
}
