//! Write scopes: repo-relative globs a task alone may modify (agents-backend §4.4, §6.4).
//!
//! Two questions, two tools:
//! - "may this task touch this file?" (the gate) → exact matching with `globset`;
//! - "may these two tasks run in parallel?" (planner edges, scheduler leases) → a
//!   conservative literal-prefix test that also covers files that do not exist yet.
//!
//! An empty scope means "unrestricted": it matches and overlaps everything.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

const META: &[char] = &['*', '?', '[', '{'];

/// Clean one scope entry: trim, drop a leading "./", and reject anything that could
/// escape the repository or touch git's own state.
pub fn normalize(glob: &str) -> Result<String, String> {
    let g = glob.trim();
    let g = g.strip_prefix("./").unwrap_or(g);
    if g.is_empty() {
        return Err("empty write_scope entry".into());
    }
    if g.starts_with('/') || g.starts_with('~') {
        return Err(format!("write_scope '{g}' must be repo-relative (no leading / or ~)"));
    }
    if g.split('/').any(|seg| seg == "..") {
        return Err(format!("write_scope '{g}' must not contain '..'"));
    }
    if g == ".git" || g.starts_with(".git/") {
        return Err(format!("write_scope '{g}' points into .git; Codly owns git"));
    }
    Ok(g.to_string())
}

/// The part of a glob before its first wildcard ("src/ui/**" → "src/ui/").
pub fn literal_prefix(glob: &str) -> &str {
    glob.find(META).map_or(glob, |i| &glob[..i])
}

/// The first pair of globs (one from each scope) that may match a common path, or None.
/// Sound over-approximation: if neither literal prefix extends the other, the globs
/// diverge on a literal character before any wildcard, so no path can match both.
pub fn overlap(a: &[String], b: &[String]) -> Option<(String, String)> {
    const ALL: &str = "<everything>";
    if a.is_empty() || b.is_empty() {
        let x = a.first().cloned().unwrap_or_else(|| ALL.into());
        let y = b.first().cloned().unwrap_or_else(|| ALL.into());
        return Some((x, y));
    }
    for x in a {
        for y in b {
            let (px, py) = (literal_prefix(x), literal_prefix(y));
            if px.starts_with(py) || py.starts_with(px) {
                return Some((x.clone(), y.clone()));
            }
        }
    }
    None
}

/// Exact matcher for the gate's scope check.
pub struct ScopeMatcher {
    set: GlobSet,
    unrestricted: bool,
}

impl ScopeMatcher {
    /// `*` stays within one path segment, `**` crosses them. An entry without wildcards
    /// matches that path and everything under it (a file or a directory).
    pub fn new(globs: &[String]) -> Result<ScopeMatcher, String> {
        let mut b = GlobSetBuilder::new();
        for g in globs {
            let g = normalize(g)?;
            let base = g.trim_end_matches('/');
            let mut pats = vec![];
            if g.ends_with('/') {
                pats.push(format!("{base}/**"));
            } else if !g.contains(META) {
                pats.push(base.to_string());
                pats.push(format!("{base}/**"));
            } else {
                pats.push(g.clone());
            }
            for p in pats {
                let glob = GlobBuilder::new(&p)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| format!("write_scope '{g}' is not a valid glob: {e}"))?;
                b.add(glob);
            }
        }
        let set = b.build().map_err(|e| e.to_string())?;
        Ok(ScopeMatcher { set, unrestricted: globs.is_empty() })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.unrestricted || self.set.is_match(path)
    }

    /// The paths outside the scope.
    pub fn outside<'a>(&self, paths: &'a [String]) -> Vec<&'a String> {
        paths.iter().filter(|p| !self.matches(p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn normalizes_and_rejects_escapes() {
        assert_eq!(normalize(" ./src/** ").unwrap(), "src/**");
        assert!(normalize("/etc/passwd").is_err());
        assert!(normalize("src/../../x").is_err());
        assert!(normalize(".git/config").is_err());
        assert!(normalize("~/x").is_err());
    }

    #[test]
    fn prefix_overlap_is_conservative() {
        assert!(overlap(&s(&["src/server/**"]), &s(&["src/client/**"])).is_none());
        assert!(overlap(&s(&["src/**"]), &s(&["src/ui/x.ts"])).is_some());
        assert!(overlap(&s(&["src/a.ts"]), &s(&["src/b.ts"])).is_none());
        assert!(overlap(&s(&["**/*.md"]), &s(&["docs/x.md"])).is_some());
        assert!(overlap(&s(&[]), &s(&["anything"])).is_some(), "unrestricted overlaps all");
        assert!(overlap(&s(&["package.json"]), &s(&["package.json"])).is_some());
    }

    #[test]
    fn matcher_semantics() {
        let m = ScopeMatcher::new(&s(&["src/ui/**", "src/*.ts", "docs", "README.md"])).unwrap();
        assert!(m.matches("src/ui/a/b.svelte"));
        assert!(m.matches("src/app.ts"));
        assert!(!m.matches("src/lib/app.ts"), "* does not cross /");
        assert!(m.matches("docs/guide/intro.md"), "plain directory entry");
        assert!(m.matches("README.md"));
        assert!(!m.matches("package.json"));
        assert!(ScopeMatcher::new(&[]).unwrap().matches("anything/at/all"));
        assert!(ScopeMatcher::new(&s(&["src/["])).is_err());
    }
}
