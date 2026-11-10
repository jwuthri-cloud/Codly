//! Git repositories on this machine, for the "Open repository" picker of `codly serve`: a browser
//! page cannot show a folder dialog, so the picker lists the repositories under the home folder.
//! Read-only: folder names and paths, nothing from inside the repositories.

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use ts_rs::TS;

/// One repository the picker offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RepoEntry {
    /// The folder's name.
    pub name: String,
    /// Its absolute path.
    pub path: String,
    /// When its `.git` last changed (ms since the epoch); the list is most recent first.
    pub modified_ms: i64,
}

/// Folders never searched: dependencies, build output and app data.
const SKIP: &[&str] = &["node_modules", "target", "Library", "Applications", "snap"];
/// The most folder entries one search reads, so a very large home folder stays quick.
const MAX_ENTRIES: usize = 20_000;

/// Repositories under `root`, at most `depth` folders down (1 = only `root`'s children), most
/// recently changed first, at most `limit`. A repository is a folder that holds a `.git` (a
/// folder, or a file for worktrees and submodules). Hidden folders (Codly's own worktrees among
/// them), symlinks and `SKIP` are not searched, and nothing inside a repository is listed.
pub fn find_repos(root: &Path, depth: usize, limit: usize) -> Vec<RepoEntry> {
    let mut found = Vec::new();
    let mut todo = vec![(root.to_path_buf(), 1)];
    let mut read = 0;
    while let Some((dir, level)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            read += 1;
            if read > MAX_ENTRIES {
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // `file_type` does not follow symlinks, so a link is never searched.
            if name.starts_with('.') || SKIP.contains(&name.as_str()) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path();
            match std::fs::symlink_metadata(path.join(".git")) {
                Ok(git) => found.push(RepoEntry { name, path: path.to_string_lossy().into_owned(), modified_ms: modified_ms(&git) }),
                Err(_) if level < depth => todo.push((path, level + 1)),
                Err(_) => {}
            }
        }
    }
    found.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms).then_with(|| a.name.cmp(&b.name)));
    found.truncate(limit);
    found
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    /// A repository whose `.git` is a file (as in a worktree), last changed at `secs`.
    fn repo(root: &Path, rel: &str, secs: u64) {
        let dir = root.join(rel);
        fs::create_dir_all(&dir).unwrap();
        let git = fs::File::create(dir.join(".git")).unwrap();
        git.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    fn names(root: &Path, depth: usize, limit: usize) -> Vec<String> {
        find_repos(root, depth, limit).into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn lists_repositories_newest_first_and_skips_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        repo(root, "git/alpha", 300);
        repo(root, "git/beta", 100);
        repo(root, "Documents/Projects/delta", 200); // three folders down
        fs::create_dir_all(root.join("code/gamma/.git")).unwrap(); // a .git folder, changed now
        repo(root, "git/alpha/vendor/nested", 400); // inside a repository
        repo(root, "a/b/c/too-deep", 500); // four folders down
        repo(root, ".local/share/worktree", 600); // a hidden folder
        repo(root, "web/node_modules/package", 700); // dependencies
        std::os::unix::fs::symlink(root.join("git/alpha"), root.join("link")).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();

        assert_eq!(names(root, 3, 50), ["gamma", "alpha", "delta", "beta"]);
        let alpha = find_repos(root, 3, 50).into_iter().find(|r| r.name == "alpha").unwrap();
        assert_eq!(alpha.path, root.join("git/alpha").to_string_lossy());
        assert_eq!(alpha.modified_ms, 300_000);
        assert_eq!(names(root, 3, 2), ["gamma", "alpha"]);
        assert_eq!(names(root, 2, 50), ["gamma", "alpha", "beta"]);
        assert!(names(root, 1, 50).is_empty());
    }

    #[test]
    fn a_missing_folder_has_no_repositories() {
        assert!(find_repos(Path::new("/no/such/folder"), 3, 50).is_empty());
    }
}
