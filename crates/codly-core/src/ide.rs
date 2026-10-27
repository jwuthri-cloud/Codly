//! The workspace IDE's file operations: list, read, write, create and search, all confined to
//! one root (an agent's worktree). The UI sends repo-relative paths; nothing else ever names
//! a file, so `confine` is the single gate every operation passes through.
//!
//! Policy (the webview is not trusted with the filesystem):
//! - a path must be relative, with no `..`, no `.git` component, and must resolve (symlinks
//!   included) to somewhere inside the canonical root, even when the file does not exist yet;
//! - `.git`, `node_modules` and `target` are never listed or searched;
//! - reads are capped at 2 MiB and refuse binary / non-UTF-8 files; writes obey the same cap.
//!
//! Everything here is synchronous and pure (no `Core`); the facade runs it on a blocking thread.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Largest file the IDE reads or writes.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Directories that are never listed or searched.
pub const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target"];
const MAX_LIST: usize = 5000;
const MAX_FILE_HITS: usize = 50;
const MAX_TEXT_HITS: usize = 200;
const MAX_WALK_FILES: usize = 20_000;
const SNIPPET_CHARS: usize = 200;

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeEntry {
    pub name: String,
    /// Root-relative, '/'-separated.
    pub path: String,
    pub is_dir: bool,
    pub size: i64,
}

/// A text file opened in the editor.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeFile {
    pub path: String,
    pub content: String,
    pub size: i64,
}

/// One matching line of a text search.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeHit {
    pub path: String,
    /// 1-based.
    pub line: i64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeSearch {
    /// Paths whose name contains the query.
    pub files: Vec<String>,
    /// Lines whose text contains the query.
    pub hits: Vec<IdeHit>,
    /// A cap was reached; narrow the query.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IdeChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

/// A file with uncommitted changes in the worktree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeChange {
    pub path: String,
    pub kind: IdeChangeKind,
}

/// What the IDE's Home tab shows.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdeInfo {
    pub agent_id: String,
    pub agent_name: String,
    /// Absolute root every path is relative to.
    pub root: String,
    pub project: String,
    pub branch: Option<String>,
    pub changed: i64,
}

// ───────────────────────────── confinement ─────────────────────────────

/// Resolve `rel` (a root-relative path) to an absolute path guaranteed to lie inside `root`.
///
/// Rejects absolute paths, `..`, NUL bytes and any `.git` component outright. The deepest
/// existing ancestor is canonicalised (so a symlink that leaves the root is caught, for the
/// file itself and for any directory above it) and the not-yet-existing tail, which can only
/// be plain names, is appended. A dangling symlink is refused: it could be created through.
pub fn confine(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.contains('\0') {
        bail!("invalid path");
    }
    let root = fs::canonicalize(root).with_context(|| format!("cannot open {}", root.display()))?;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(n) if n == ".git" => bail!("{rel}: .git is off limits"),
            Component::Normal(n) => tail.push(n),
            Component::CurDir => {}
            Component::ParentDir => bail!("{rel}: paths cannot contain .."),
            Component::RootDir | Component::Prefix(_) => bail!("{rel}: paths must be relative to the workspace"),
        }
    }
    // The longest prefix that exists on disk (a link counts as existing even if dangling).
    let mut base = root.clone();
    let mut found = 0;
    for n in &tail {
        let next = base.join(n);
        if fs::symlink_metadata(&next).is_err() {
            break;
        }
        base = next;
        found += 1;
    }
    let existing = fs::canonicalize(&base).with_context(|| format!("{rel}: broken link"))?;
    if !existing.starts_with(&root) {
        bail!("{rel}: outside the workspace");
    }
    // What does not exist yet can only be plain names, so it cannot lead out of `existing`.
    let mut out = existing;
    out.extend(&tail[found..]);
    Ok(out)
}

/// Root-relative '/'-separated form of `abs` (which must be inside the canonical `root`).
fn rel_of(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let parts: Vec<&str> = rel.components().map(|c| c.as_os_str().to_str()).collect::<Option<_>>()?;
    Some(parts.join("/"))
}

fn join_rel(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() || dir == "." { name.to_string() } else { format!("{dir}/{name}") }
}

// ───────────────────────────── operations ─────────────────────────────

/// Children of a directory ("" = the root): folders first, then files, each by name. Skipped
/// directories and links that leave the root are left out.
pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<IdeEntry>> {
    let dir = confine(root, rel)?;
    if !dir.is_dir() {
        bail!("{rel}: not a folder");
    }
    let mut out = Vec::new();
    for e in fs::read_dir(&dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".DS_Store" || name == ".git" {
            continue;
        }
        let path = join_rel(rel, &name);
        // `confine` follows links, so a symlink is judged by where it really points.
        let Ok(abs) = confine(root, &path) else { continue };
        let Ok(meta) = fs::metadata(&abs) else { continue };
        if meta.is_dir() && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        if !meta.is_dir() && !meta.is_file() {
            continue;
        }
        out.push(IdeEntry { name, path, is_dir: meta.is_dir(), size: meta.len() as i64 });
        if out.len() >= MAX_LIST {
            break;
        }
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

fn looks_binary(head: &[u8]) -> bool {
    head.contains(&0)
}

/// Read a text file (≤ 2 MiB, UTF-8, no NUL bytes).
pub fn read_file(root: &Path, rel: &str) -> Result<IdeFile> {
    let abs = confine(root, rel)?;
    let meta = fs::metadata(&abs).with_context(|| format!("{rel}: cannot read"))?;
    if !meta.is_file() {
        bail!("{rel}: not a file");
    }
    if meta.len() > MAX_FILE_BYTES {
        bail!("{rel}: too large to open ({:.1} MB, the limit is 2 MB)", meta.len() as f64 / 1_048_576.0);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(&abs)?.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if looks_binary(&bytes[..bytes.len().min(8000)]) {
        bail!("{rel}: binary file, not shown");
    }
    let content = String::from_utf8(bytes).map_err(|_| anyhow!("{rel}: not UTF-8 text, not shown"))?;
    Ok(IdeFile { path: rel.trim_start_matches("./").to_string(), size: meta.len() as i64, content })
}

/// Overwrite an existing text file.
pub fn write_file(root: &Path, rel: &str, content: &str) -> Result<()> {
    if content.len() as u64 > MAX_FILE_BYTES {
        bail!("{rel}: too large to save (the limit is 2 MB)");
    }
    let abs = confine(root, rel)?;
    if !abs.is_file() {
        bail!("{rel}: no such file (use New file to create it)");
    }
    fs::write(&abs, content).with_context(|| format!("{rel}: cannot save"))
}

/// Create a new empty file (and any missing folders above it). Fails if it exists.
pub fn create_file(root: &Path, rel: &str) -> Result<()> {
    if rel.trim_matches('/').is_empty() || rel.ends_with('/') {
        bail!("give the file a name");
    }
    let abs = confine(root, rel)?;
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).with_context(|| format!("{rel}: cannot create the folder"))?;
    }
    // Re-check after creating folders: nothing created may have resolved elsewhere.
    confine(root, rel)?;
    fs::OpenOptions::new().write(true).create_new(true).open(&abs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists { anyhow!("{rel} already exists") } else { anyhow!("{rel}: {e}") }
    })?;
    Ok(())
}

/// Find files by name and lines by content (case-insensitive substring), honouring
/// `.gitignore`, never following links, skipping `.git`/`node_modules`/`target`, big and
/// binary files.
pub fn search(root: &Path, query: &str) -> Result<IdeSearch> {
    let q = query.trim().to_lowercase();
    let mut out = IdeSearch::default();
    if q.chars().count() < 2 {
        return Ok(out);
    }
    let root = fs::canonicalize(root)?;
    let walker = WalkBuilder::new(&root)
        .hidden(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .max_filesize(Some(MAX_FILE_BYTES))
        .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_some_and(|t| t.is_dir()) && e.file_name().to_str().is_some_and(|n| SKIP_DIRS.contains(&n))))
        .build();
    let mut seen = 0usize;
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue; // directories, and links (not followed)
        }
        seen += 1;
        if seen > MAX_WALK_FILES {
            out.truncated = true;
            break;
        }
        let Some(rel) = rel_of(&root, entry.path()) else { continue };
        if rel.to_lowercase().contains(&q) {
            if out.files.len() < MAX_FILE_HITS {
                out.files.push(rel.clone());
            } else {
                out.truncated = true;
            }
        }
        if out.hits.len() >= MAX_TEXT_HITS {
            out.truncated = true;
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else { continue };
        if looks_binary(&bytes[..bytes.len().min(8000)]) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else { continue };
        for (i, line) in text.lines().enumerate() {
            if line.to_lowercase().contains(&q) {
                let snippet: String = line.trim().chars().take(SNIPPET_CHARS).collect();
                out.hits.push(IdeHit { path: rel.clone(), line: i as i64 + 1, text: snippet });
                if out.hits.len() >= MAX_TEXT_HITS {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Parse `git status --porcelain=v1 -z -uall` output.
pub fn parse_status(z: &str) -> Vec<IdeChange> {
    let mut out = Vec::new();
    let mut it = z.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = it.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = (&entry[..2], &entry[3..]);
        let (x, y) = (code.as_bytes()[0], code.as_bytes()[1]);
        if x == b'R' || x == b'C' {
            it.next(); // the original path of a rename/copy follows as its own field
        }
        let kind = if code == "??" {
            IdeChangeKind::Untracked
        } else if x == b'R' || x == b'C' {
            IdeChangeKind::Renamed
        } else if x == b'D' || y == b'D' {
            IdeChangeKind::Deleted
        } else if x == b'A' {
            IdeChangeKind::Added
        } else {
            IdeChangeKind::Modified
        };
        out.push(IdeChange { path: path.to_string(), kind });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// `<tmp>/root/` is the workspace; `<tmp>/outside/secret.txt` is what must stay unreachable.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(tmp.path().join("outside")).unwrap();
        fs::write(tmp.path().join("outside/secret.txt"), "TOP SECRET needle").unwrap();
        fs::write(root.join("README.md"), "# hello\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {\n    println!(\"needle here\");\n}\n").unwrap();
        fs::write(root.join("src/deep/mod.rs"), "// Needle again\n").unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "needle").unwrap();
        fs::write(root.join("target/debug/out.txt"), "needle").unwrap();
        fs::write(root.join(".git/config"), "needle").unwrap();
        (tmp, root)
    }

    #[test]
    fn confine_accepts_plain_relative_paths() {
        let (_t, root) = fixture();
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(confine(&root, "").unwrap(), canon);
        assert_eq!(confine(&root, ".").unwrap(), canon);
        assert_eq!(confine(&root, "src/main.rs").unwrap(), canon.join("src/main.rs"));
        assert_eq!(confine(&root, "./src//main.rs").unwrap(), canon.join("src/main.rs"));
        // Not there yet: allowed, resolved under the existing parent.
        assert_eq!(confine(&root, "src/new/dir/file.rs").unwrap(), canon.join("src/new/dir/file.rs"));
        assert_eq!(confine(&root, "brand-new.txt").unwrap(), canon.join("brand-new.txt"));
    }

    #[test]
    fn confine_rejects_dotdot_absolute_nul_and_git() {
        let (t, root) = fixture();
        for bad in ["..", "../outside/secret.txt", "src/../../outside/secret.txt", "src/..", "a/../b"] {
            assert!(confine(&root, bad).is_err(), "{bad} should be rejected");
        }
        let abs_outside = t.path().join("outside/secret.txt");
        assert!(confine(&root, abs_outside.to_str().unwrap()).is_err());
        assert!(confine(&root, "/etc/passwd").is_err());
        assert!(confine(&root, root.join("README.md").to_str().unwrap()).is_err(), "even an absolute path inside the root");
        assert!(confine(&root, "a\0b").is_err());
        assert!(confine(&root, ".git").is_err());
        assert!(confine(&root, ".git/config").is_err());
        assert!(confine(&root, "src/.git/hooks/pre-commit").is_err());
    }

    #[test]
    fn confine_rejects_symlinks_that_leave_the_root() {
        let (t, root) = fixture();
        let outside = t.path().join("outside");
        symlink(outside.join("secret.txt"), root.join("link-file")).unwrap();
        symlink(&outside, root.join("link-dir")).unwrap();
        symlink(&outside, root.join("src/deep/link-up")).unwrap();
        symlink(t.path().join("outside/nothing-here"), root.join("dangling")).unwrap();
        assert!(confine(&root, "link-file").is_err());
        assert!(confine(&root, "link-dir").is_err());
        assert!(confine(&root, "link-dir/secret.txt").is_err());
        assert!(confine(&root, "src/deep/link-up/secret.txt").is_err());
        // Creating "through" a link directory must fail as well, the file does not exist yet.
        assert!(confine(&root, "link-dir/new.txt").is_err());
        assert!(confine(&root, "link-dir/a/b/new.txt").is_err());
        assert!(confine(&root, "dangling").is_err());
        assert!(!outside.join("new.txt").exists());
    }

    #[test]
    fn confine_follows_symlinks_that_stay_inside() {
        let (_t, root) = fixture();
        symlink(root.join("src"), root.join("alias")).unwrap();
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(confine(&root, "alias/main.rs").unwrap(), canon.join("src/main.rs"));
    }

    #[test]
    fn root_that_is_itself_a_symlink_works() {
        let (t, root) = fixture();
        let via = t.path().join("via-link");
        symlink(&root, &via).unwrap();
        assert!(confine(&via, "src/main.rs").is_ok());
        assert!(confine(&via, "../outside/secret.txt").is_err());
    }

    #[test]
    fn list_dir_sorts_and_skips() {
        let (t, root) = fixture();
        symlink(t.path().join("outside"), root.join("escape")).unwrap();
        symlink(root.join("README.md"), root.join("inside-link.md")).unwrap();
        let top = list_dir(&root, "").unwrap();
        let names: Vec<&str> = top.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "inside-link.md", "README.md"]);
        assert!(top[0].is_dir && !top[1].is_dir);
        let sub = list_dir(&root, "src").unwrap();
        assert_eq!(sub.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["src/deep", "src/main.rs"]);
        assert!(list_dir(&root, "..").is_err());
        assert!(list_dir(&root, "README.md").is_err());
        assert!(list_dir(&root, "escape").is_err());
    }

    #[test]
    fn read_refuses_binary_big_and_foreign_files() {
        let (t, root) = fixture();
        assert_eq!(read_file(&root, "README.md").unwrap().content, "# hello\n");
        fs::write(root.join("bin.dat"), [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        assert!(read_file(&root, "bin.dat").unwrap_err().to_string().contains("binary"));
        fs::write(root.join("latin1.txt"), [0xe9, 0xe8, b'a']).unwrap();
        assert!(read_file(&root, "latin1.txt").unwrap_err().to_string().contains("UTF-8"));
        fs::write(root.join("big.txt"), vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(read_file(&root, "big.txt").unwrap_err().to_string().contains("too large"));
        fs::write(root.join("edge.txt"), vec![b'a'; MAX_FILE_BYTES as usize]).unwrap();
        assert!(read_file(&root, "edge.txt").is_ok());
        assert!(read_file(&root, "src").is_err());
        assert!(read_file(&root, "missing.txt").is_err());
        symlink(t.path().join("outside/secret.txt"), root.join("leak")).unwrap();
        assert!(read_file(&root, "leak").is_err());
        assert!(read_file(&root, "../outside/secret.txt").is_err());
        assert!(read_file(&root, ".git/config").is_err());
    }

    #[test]
    fn write_and_create_stay_inside() {
        let (t, root) = fixture();
        write_file(&root, "README.md", "changed\n").unwrap();
        assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), "changed\n");
        assert!(write_file(&root, "nope.txt", "x").is_err(), "save never creates");
        assert!(write_file(&root, "src", "x").is_err());
        assert!(write_file(&root, "README.md", &"a".repeat(MAX_FILE_BYTES as usize + 1)).is_err());

        create_file(&root, "src/new/thing.rs").unwrap();
        assert!(root.join("src/new/thing.rs").is_file());
        assert!(create_file(&root, "src/new/thing.rs").unwrap_err().to_string().contains("already exists"));
        assert!(create_file(&root, "").is_err());
        assert!(create_file(&root, "dir/").is_err());

        let secret = t.path().join("outside/secret.txt");
        symlink(&secret, root.join("leak")).unwrap();
        symlink(t.path().join("outside"), root.join("out-dir")).unwrap();
        assert!(write_file(&root, "leak", "pwned").is_err());
        assert!(write_file(&root, "../outside/secret.txt", "pwned").is_err());
        assert!(create_file(&root, "out-dir/new.txt").is_err());
        assert!(create_file(&root, "../outside/new.txt").is_err());
        assert!(create_file(&root, ".git/hooks/pre-commit").is_err());
        assert!(write_file(&root, ".git/config", "x").is_err());
        assert_eq!(fs::read_to_string(&secret).unwrap(), "TOP SECRET needle");
        assert!(!t.path().join("outside/new.txt").exists());
        assert!(!root.join(".git/hooks").exists());
    }

    #[test]
    fn search_finds_names_and_lines_and_skips_junk() {
        let (t, root) = fixture();
        symlink(t.path().join("outside"), root.join("escape")).unwrap();
        fs::write(root.join("bin.dat"), [b'n', b'e', b'e', b'd', b'l', b'e', 0]).unwrap();
        let r = search(&root, "needle").unwrap();
        let paths: Vec<&str> = r.hits.iter().map(|h| h.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs") && paths.contains(&"src/deep/mod.rs"));
        assert_eq!(paths.len(), 2, "no node_modules, target, .git, binary, or symlinked hits: {paths:?}");
        let main = r.hits.iter().find(|h| h.path == "src/main.rs").unwrap();
        assert_eq!(main.line, 2);
        assert!(main.text.contains("needle here"));
        let by_name = search(&root, "MAIN.rs").unwrap();
        assert_eq!(by_name.files, ["src/main.rs"]);
        assert!(search(&root, "x").unwrap().hits.is_empty(), "one character is too short");
        assert!(!r.truncated);
    }

    #[test]
    fn status_parsing() {
        let z = " M src/a.rs\0A  new.rs\0?? scratch.txt\0 D gone.rs\0R  moved.rs\0old.rs\0MM both.rs\0";
        let got: Vec<(String, IdeChangeKind)> = parse_status(z).into_iter().map(|c| (c.path, c.kind)).collect();
        assert_eq!(
            got,
            vec![
                ("both.rs".into(), IdeChangeKind::Modified),
                ("gone.rs".into(), IdeChangeKind::Deleted),
                ("moved.rs".into(), IdeChangeKind::Renamed),
                ("new.rs".into(), IdeChangeKind::Added),
                ("scratch.txt".into(), IdeChangeKind::Untracked),
                ("src/a.rs".into(), IdeChangeKind::Modified),
            ]
        );
    }
}
