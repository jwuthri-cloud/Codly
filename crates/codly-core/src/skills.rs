//! Skills manager: find the skills the agent CLIs can load, install new ones from a small
//! bundled catalog, and uninstall them again. A skill is a folder with a `SKILL.md` whose
//! frontmatter has a `name` and a `description` (Claude Code's and Codex's shared format).
//!
//! Scanned, in this order:
//! - `<project>/.claude/skills`                 Claude Code, project scope
//! - `~/.claude/skills`                         Claude Code, user scope
//! - `~/.claude/plugins/cache/*/*/<ver>/skills` Claude Code plugins (user scope, read-only)
//! - `~/.codex/skills`                          Codex, user scope (`.system/*` is read-only)
//! - Codly's own, compiled in                   bundled scope, read-only
//!
//! Safety (the only code here that deletes or spawns anything):
//! - uninstall removes one direct child of a *removable* skills dir and nothing else; the path
//!   is checked lexically (absolute, no `..`) and by canonicalising its parent, so a symlinked
//!   or traversing path cannot reach outside. A symlinked skill is unlinked, never followed.
//! - install takes a catalog id, never a URL from the UI; the name is validated, an existing
//!   directory is never overwritten, and the clone lands in a scratch dir first.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which agent CLI loads the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SkillEngine {
    ClaudeCode,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SkillScope {
    Project,
    User,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The skill's folder (for a bundled skill, a `codly:` label: there is no folder).
    pub path: String,
    pub engine: SkillEngine,
    pub scope: SkillScope,
    /// Where it comes from when that is not obvious ("plugin posthog", "Codex system").
    pub origin: Option<String>,
    /// Uninstall is offered (a skill of a plugin, of Codex's system set or of Codly is not ours to remove).
    pub removable: bool,
}

/// One entry of the bundled catalog (`skills/catalog.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSkill {
    /// Also the folder name it installs to.
    pub id: String,
    pub description: String,
    pub category: String,
    pub author: String,
    /// `https://` git URL of the repository.
    pub repo: String,
    /// The skill's folder inside the repository (None: the repository is the skill).
    #[serde(default)]
    pub subdir: Option<String>,
    /// Already present in `~/.claude/skills` (filled in by `catalog`).
    #[serde(default)]
    pub installed: bool,
}

const CATALOG_JSON: &str = include_str!("../skills/catalog.json");
const BUNDLED: &[(&str, &str)] = &[("codly-team", include_str!("../skills/codly-team/SKILL.md"))];

// ───────────────────────────── frontmatter ─────────────────────────────

/// `name` and `description` from a SKILL.md's `---` frontmatter. Handles quoted values and
/// YAML folded/literal blocks and plain multi-line values (joined with single spaces).
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let (mut name, mut desc) = (None, None);
    // The key being collected and its value so far.
    let mut cur: Option<(String, String)> = None;
    let mut flush = |cur: &mut Option<(String, String)>| {
        if let Some((k, v)) = cur.take() {
            let v = unquote(v.trim());
            if !v.is_empty() {
                match k.as_str() {
                    "name" => name = Some(v),
                    "description" => desc = Some(v),
                    _ => {}
                }
            }
        }
    };
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if let Some((_, v)) = cur.as_mut() {
                if !v.is_empty() {
                    v.push(' ');
                }
                v.push_str(line.trim());
            }
            continue;
        }
        flush(&mut cur);
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            // `>`, `|`, `>-`… start a block scalar: the text is on the following lines.
            let v = if matches!(v.chars().next(), Some('>' | '|')) { "" } else { v };
            cur = Some((k.trim().to_string(), v.to_string()));
        }
    }
    flush(&mut cur);
    (name, desc)
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    let q = b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0];
    let s = if q { &v[1..v.len() - 1] } else { v };
    s.chars().take(400).collect()
}

// ───────────────────────────── scanning ─────────────────────────────

/// A directory whose children are skill folders, and what they are.
#[derive(Debug, Clone)]
pub struct Root {
    pub dir: PathBuf,
    pub engine: SkillEngine,
    pub scope: SkillScope,
    pub origin: Option<String>,
    pub removable: bool,
}

/// Where skills live for this user and (optionally) project, in scan order.
pub fn roots(home: &Path, project: Option<&Path>) -> Vec<Root> {
    let mk = |dir: PathBuf, engine, scope, origin: Option<String>, removable| Root { dir, engine, scope, origin, removable };
    let mut out = vec![];
    if let Some(p) = project {
        out.push(mk(p.join(".claude/skills"), SkillEngine::ClaudeCode, SkillScope::Project, None, true));
    }
    out.push(mk(home.join(".claude/skills"), SkillEngine::ClaudeCode, SkillScope::User, None, true));
    out.extend(plugin_roots(home));
    out.push(mk(home.join(".codex/skills"), SkillEngine::Codex, SkillScope::User, None, true));
    out
}

/// `skills/` of each installed Claude Code plugin (its newest version only):
/// `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/skills`.
fn plugin_roots(home: &Path) -> Vec<Root> {
    let mut out = vec![];
    for market in sorted_dirs(&home.join(".claude/plugins/cache")) {
        for plugin in sorted_dirs(&market) {
            let newest = sorted_dirs(&plugin)
                .into_iter()
                .filter(|v| v.join("skills").is_dir())
                .max_by_key(|v| v.metadata().and_then(|m| m.modified()).ok());
            let dir = match newest {
                Some(v) => v.join("skills"),
                None if plugin.join("skills").is_dir() => plugin.join("skills"),
                None => continue,
            };
            let label = plugin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(Root { dir, engine: SkillEngine::ClaudeCode, scope: SkillScope::User, origin: Some(format!("plugin {label}")), removable: false });
        }
    }
    out
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

fn read_skill(dir: &Path, root: &Root, removable: bool, origin: Option<String>) -> Option<Skill> {
    let text = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let (name, desc) = parse_frontmatter(&text);
    let fallback = dir.file_name()?.to_string_lossy().into_owned();
    Some(Skill {
        name: name.unwrap_or(fallback),
        description: desc.unwrap_or_default(),
        path: dir.to_string_lossy().into_owned(),
        engine: root.engine,
        scope: root.scope,
        origin: origin.or_else(|| root.origin.clone()),
        removable,
    })
}

/// Every skill directly under `root.dir`. A dot-folder without a SKILL.md (Codex's `.system`)
/// is a read-only group: its children are listed one level down.
pub fn scan_root(root: &Root) -> Vec<Skill> {
    let mut out = vec![];
    for dir in sorted_dirs(&root.dir) {
        if let Some(s) = read_skill(&dir, root, root.removable, None) {
            out.push(s);
            continue;
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            let group = format!("{} set", name.trim_start_matches('.'));
            for inner in sorted_dirs(&dir) {
                out.extend(read_skill(&inner, root, false, Some(format!("{} {group}", engine_label(root.engine)))));
            }
        }
    }
    out
}

fn engine_label(e: SkillEngine) -> &'static str {
    match e {
        SkillEngine::ClaudeCode => "Claude Code",
        SkillEngine::Codex => "Codex",
    }
}

/// Codly's own skills, compiled in.
pub fn bundled() -> Vec<Skill> {
    BUNDLED
        .iter()
        .map(|(id, text)| {
            let (name, desc) = parse_frontmatter(text);
            Skill {
                name: name.unwrap_or_else(|| (*id).into()),
                description: desc.unwrap_or_default(),
                path: format!("codly:bundled/{id}"),
                engine: SkillEngine::ClaudeCode,
                scope: SkillScope::Bundled,
                origin: Some("built into Codly".into()),
                removable: false,
            }
        })
        .collect()
}

/// Everything installed, project first, bundled last; the same folder is listed once.
pub fn list(home: &Path, project: Option<&Path>) -> Vec<Skill> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for r in roots(home, project) {
        for s in scan_root(&r) {
            let key = std::fs::canonicalize(&s.path).unwrap_or_else(|_| PathBuf::from(&s.path));
            if seen.insert(key) {
                out.push(s);
            }
        }
    }
    out.extend(bundled());
    out
}

// ───────────────────────────── path safety ─────────────────────────────

/// `path` must be absolute, free of `.`/`..`, and name something directly inside one of `dirs`
/// (compared canonically, so symlinked parents and `..` tricks cannot sneak out). The entry
/// itself is not followed: a symlinked skill is judged by where the link lives.
fn child_of(path: &Path, dirs: &[PathBuf], direct: bool) -> Result<()> {
    if !path.is_absolute() {
        bail!("not an absolute path");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        bail!("path contains . or ..");
    }
    let parent = path.parent().ok_or_else(|| anyhow!("no parent"))?;
    path.file_name().ok_or_else(|| anyhow!("no file name"))?;
    let parent = std::fs::canonicalize(parent).context("parent does not exist")?;
    for d in dirs {
        let Ok(d) = std::fs::canonicalize(d) else { continue };
        if (direct && parent == d) || (!direct && parent.starts_with(&d)) {
            return Ok(());
        }
    }
    bail!("not inside a skills folder")
}

/// The removable roots, as paths.
fn removable_dirs(home: &Path, project: Option<&Path>) -> Vec<PathBuf> {
    roots(home, project).into_iter().filter(|r| r.removable).map(|r| r.dir).collect()
}

/// Remove an installed skill. Refuses anything that is not a skill folder (or a link to one)
/// sitting directly in a removable skills dir.
pub fn uninstall(path: &Path, home: &Path, project: Option<&Path>) -> Result<()> {
    child_of(path, &removable_dirs(home, project), true).with_context(|| format!("refusing to remove {}", path.display()))?;
    let meta = std::fs::symlink_metadata(path).context("no such skill")?;
    if !path.join("SKILL.md").is_file() {
        bail!("{} has no SKILL.md; not removing it", path.display());
    }
    if meta.file_type().is_symlink() {
        std::fs::remove_file(path)?;
    } else if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        bail!("not a skill folder");
    }
    Ok(())
}

/// Open a skill's folder in the file manager (only folders under a known skills dir).
pub fn open_folder(path: &Path, home: &Path, project: Option<&Path>) -> Result<()> {
    let all: Vec<PathBuf> = roots(home, project).into_iter().map(|r| r.dir).collect();
    child_of(path, &all, false)?;
    if !path.is_dir() {
        bail!("no such folder");
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(opener).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

// ───────────────────────────── catalog & install ─────────────────────────────

/// Folder names we accept for an installed skill: lowercase letters, digits, `-`, `_`, `.`;
/// no leading dot or dash, so never `.`/`..`, a hidden folder or a path.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with(['.', '-'])
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// A subdirectory inside a repo: relative, plain components.
fn valid_subdir(s: &str) -> bool {
    !s.is_empty() && Path::new(s).components().all(|c| matches!(c, Component::Normal(_)))
}

fn catalog_entries() -> Vec<CatalogSkill> {
    serde_json::from_str(CATALOG_JSON).expect("skills/catalog.json is valid")
}

/// The bundled catalog, each entry marked when `~/.claude/skills/<id>` already exists.
pub fn catalog(home: &Path) -> Vec<CatalogSkill> {
    let dir = home.join(".claude/skills");
    let mut v = catalog_entries();
    for e in &mut v {
        e.installed = std::fs::symlink_metadata(dir.join(&e.id)).is_ok();
    }
    v
}

/// Install a catalog skill into `~/.claude/skills/<id>`.
pub fn install(id: &str, home: &Path) -> Result<Skill> {
    let entry = catalog_entries().into_iter().find(|e| e.id == id).ok_or_else(|| anyhow!("{id} is not in the catalog"))?;
    if !entry.repo.starts_with("https://") {
        bail!("catalog entry {id} is not an https URL");
    }
    let root = home.join(".claude/skills");
    let dest = install_from(&entry.repo, entry.subdir.as_deref(), &entry.id, &root)?;
    let r = Root { dir: root, engine: SkillEngine::ClaudeCode, scope: SkillScope::User, origin: None, removable: true };
    read_skill(&dest, &r, true, None).ok_or_else(|| anyhow!("installed, but SKILL.md is unreadable"))
}

/// `git clone --depth 1 url` into a scratch dir next to `root`, then move the skill (the repo, or
/// its `subdir`) to `root/name`. Refuses to touch an existing `root/name`; cleans up after itself.
pub fn install_from(url: &str, subdir: Option<&str>, name: &str, root: &Path) -> Result<PathBuf> {
    if !valid_name(name) {
        bail!("“{name}” is not a valid skill name (lowercase letters, digits, - _ .)");
    }
    if let Some(s) = subdir
        && !valid_subdir(s)
    {
        bail!("bad subfolder “{s}”");
    }
    std::fs::create_dir_all(root)?;
    let dest = root.join(name);
    if std::fs::symlink_metadata(&dest).is_ok() {
        bail!("{} already exists; remove it first", dest.display());
    }
    let scratch = root.parent().unwrap_or(root).join(format!(".codly-install-{}", uuid::Uuid::new_v4().simple()));
    let result = clone_and_move(url, subdir, &scratch, &dest);
    let _ = std::fs::remove_dir_all(&scratch);
    result.map(|()| dest)
}

fn clone_and_move(url: &str, subdir: Option<&str>, scratch: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(scratch)?;
    let repo = scratch.join("repo");
    // Never prompt, never run the remote's hooks, only talk https (or a local path in tests).
    let out = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "clone", "--depth", "1", "--quiet", "--"])
        .arg(url)
        .arg(&repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", "30")
        .stdin(Stdio::null())
        .output()
        .context("could not run git (is it installed?)")?;
    if !out.status.success() {
        bail!("git clone failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let src = match subdir {
        Some(s) => repo.join(s),
        None => repo.clone(),
    };
    // A symlink in the repo must not lead the move outside the clone.
    let real = std::fs::canonicalize(&src).map_err(|_| anyhow!("the repository has no folder “{}”", subdir.unwrap_or("")))?;
    if !real.starts_with(std::fs::canonicalize(&repo)?) {
        bail!("subfolder escapes the repository");
    }
    if !src.join("SKILL.md").is_file() {
        bail!("no SKILL.md in {}; not a skill", subdir.unwrap_or("the repository"));
    }
    std::fs::rename(&real, dest).context("could not move the skill into place")?;
    Ok(())
}

pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn skill(dir: &Path, name: &str, fm: &str) -> PathBuf {
        let d = dir.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), format!("---\n{fm}\n---\n\n# Body\n")).unwrap();
        d
    }

    #[test]
    fn frontmatter_forms() {
        let (n, d) = parse_frontmatter("---\nname: pdf\ndescription: Work with PDFs\n---\nbody");
        assert_eq!((n.as_deref(), d.as_deref()), (Some("pdf"), Some("Work with PDFs")));
        let (n, d) = parse_frontmatter("\u{feff}---\nname: \"quoted\"\ndescription: 'single'\nlicense: MIT\n---\n");
        assert_eq!((n.as_deref(), d.as_deref()), (Some("quoted"), Some("single")));
        let (_, d) = parse_frontmatter("---\nname: x\ndescription: >\n  first line\n  second line\nother: 1\n---\n");
        assert_eq!(d.as_deref(), Some("first line second line"));
        let (_, d) = parse_frontmatter("---\nname: x\ndescription: plain that\n  wraps\n---\n");
        assert_eq!(d.as_deref(), Some("plain that wraps"));
        assert_eq!(parse_frontmatter("# no frontmatter"), (None, None));
    }

    #[test]
    fn scans_every_root_with_engine_and_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, proj) = (tmp.path().join("home"), tmp.path().join("proj"));
        skill(&proj.join(".claude/skills"), "p1", "name: proj-skill\ndescription: In the project");
        skill(&home.join(".claude/skills"), "u1", "name: user-skill\ndescription: For me");
        skill(&home.join(".claude/skills"), "nofm", "title: nothing useful");
        fs::create_dir_all(home.join(".claude/skills/not-a-skill")).unwrap();
        skill(&home.join(".codex/skills"), "cx", "name: codex-skill\ndescription: For Codex");
        skill(&home.join(".codex/skills/.system"), "sys", "name: sys-skill\ndescription: Shipped with Codex");
        skill(&home.join(".claude/plugins/cache/mkt/acme/1.0.0/skills"), "pl", "name: plugin-skill\ndescription: From a plugin");

        let all = list(&home, Some(&proj));
        let by = |n: &str| all.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("{n} missing in {all:#?}"));
        assert_eq!((by("proj-skill").scope, by("proj-skill").engine), (SkillScope::Project, SkillEngine::ClaudeCode));
        assert!(by("proj-skill").removable);
        assert_eq!(by("user-skill").description, "For me");
        assert_eq!(by("nofm").name, "nofm", "falls back to the folder name");
        assert!(all.iter().all(|s| s.name != "not-a-skill"));
        assert_eq!((by("codex-skill").engine, by("codex-skill").scope), (SkillEngine::Codex, SkillScope::User));
        assert!(!by("sys-skill").removable);
        assert_eq!(by("plugin-skill").origin.as_deref(), Some("plugin acme"));
        assert!(!by("plugin-skill").removable);
        assert_eq!(by("codly-team").scope, SkillScope::Bundled);
        assert!(!by("codly-team").removable);
        // Project skills are listed first, bundled last.
        assert_eq!(all[0].name, "proj-skill");
        assert_eq!(all.last().unwrap().scope, SkillScope::Bundled);
        // Without a project, its skills are not listed.
        assert!(list(&home, None).iter().all(|s| s.name != "proj-skill"));
    }

    #[test]
    fn uninstall_only_inside_removable_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let root = home.join(".claude/skills");
        let a = skill(&root, "a", "name: a");
        let plugin = skill(&home.join(".claude/plugins/cache/m/p/1/skills"), "pl", "name: pl");
        let sys = skill(&home.join(".codex/skills/.system"), "sys", "name: sys");
        let outside = skill(tmp.path(), "outside", "name: outside");
        let unrelated = root.join("unrelated");
        fs::create_dir_all(&unrelated).unwrap();

        // Traversal, relative paths, the root itself, other dirs and read-only skills are refused.
        let traversal = root.join("../../../outside");
        for bad in [&traversal, &root, &home, &outside, &plugin, &sys, Path::new("a"), Path::new("/"), &unrelated] {
            assert!(uninstall(bad, &home, None).is_err(), "{} should be refused", bad.display());
        }
        assert!(outside.exists() && plugin.exists() && sys.exists() && unrelated.exists());
        // A symlink in the skills dir pointing outside: the link goes, the target stays.
        #[cfg(unix)]
        {
            let link = root.join("linked");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            uninstall(&link, &home, None).unwrap();
            assert!(!link.exists() && outside.join("SKILL.md").exists());
            // Reaching out through a symlinked *parent* is refused too.
            let via = root.join("linked2");
            std::os::unix::fs::symlink(tmp.path(), &via).unwrap();
            assert!(uninstall(&via.join("outside"), &home, None).is_err());
            assert!(outside.exists());
        }
        uninstall(&a, &home, None).unwrap();
        assert!(!a.exists());
        // A project skills dir is removable once the project is given.
        let proj = tmp.path().join("proj");
        let p = skill(&proj.join(".claude/skills"), "p", "name: p");
        assert!(uninstall(&p, &home, None).is_err());
        uninstall(&p, &home, Some(&proj)).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn names_and_catalog() {
        for ok in ["pdf", "web-design-guidelines", "a_b.c", "x1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "../x", "A", "a b", "-rf", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(valid_subdir("skills/pdf") && !valid_subdir("../x") && !valid_subdir("/etc") && !valid_subdir(""));

        let tmp = tempfile::tempdir().unwrap();
        let cat = catalog(tmp.path());
        assert!(cat.len() >= 20);
        let ids: HashSet<_> = cat.iter().map(|e| &e.id).collect();
        assert_eq!(ids.len(), cat.len(), "ids are unique");
        assert!(cat.iter().all(|e| valid_name(&e.id) && e.repo.starts_with("https://") && e.subdir.as_deref().is_none_or(valid_subdir)));
        assert!(cat.iter().all(|e| !e.installed));
        skill(&tmp.path().join(".claude/skills"), "pdf", "name: pdf");
        assert!(catalog(tmp.path()).iter().find(|e| e.id == "pdf").unwrap().installed);
        assert!(install("nope", tmp.path()).is_err());
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).current_dir(dir).status().unwrap().success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn installs_from_a_local_repo_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        skill(&repo.join("skills"), "nice", "name: nice\ndescription: A nice skill");
        fs::create_dir_all(repo.join("skills/empty")).unwrap();
        fs::write(repo.join("skills/empty/README.md"), "x").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);

        let root = tmp.path().join("home/.claude/skills");
        let url = repo.to_str().unwrap();
        let dest = install_from(url, Some("skills/nice"), "nice", &root).unwrap();
        assert_eq!(dest, root.join("nice"));
        assert!(dest.join("SKILL.md").is_file() && !dest.join(".git").exists());
        // No scratch dirs left behind.
        let left: Vec<_> = fs::read_dir(root.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("skills")]);

        // Existing directory: refused, and untouched.
        fs::write(dest.join("mine.txt"), "keep").unwrap();
        assert!(install_from(url, Some("skills/nice"), "nice", &root).unwrap_err().to_string().contains("already exists"));
        assert!(dest.join("mine.txt").exists());
        // Bad names, bad subdirs, folders that are not skills, bad repos.
        assert!(install_from(url, Some("skills/nice"), "../evil", &root).is_err());
        assert!(install_from(url, Some("../repo"), "ok", &root).is_err());
        assert!(install_from(url, Some("skills/empty"), "empty", &root).is_err());
        assert!(!root.join("empty").exists());
        assert!(install_from("/does/not/exist", None, "ghost", &root).is_err());
        assert!(!root.join("ghost").exists());
        // The whole repo as the skill keeps its .git so it can be updated.
        skill(&repo, ".", "name: whole");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "root skill"]);
        let whole = install_from(url, None, "whole", &root).unwrap();
        assert!(whole.join("SKILL.md").is_file() && whole.join(".git").exists());
    }
}
