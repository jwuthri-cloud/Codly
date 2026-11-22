//! Keeps README.md and docs/ARCHITECTURE.md honest. People read them before they point agents
//! at a real repository, so a sentence that the code no longer keeps, or a code map that
//! leaves out part of the tree, must fail here instead of going stale silently.

use std::path::{Path, PathBuf};

const README: &str = include_str!("../../../README.md");
const ARCHITECTURE: &str = include_str!("../../../docs/ARCHITECTURE.md");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Lower case, with every run of whitespace as one space, so a line wrap cannot hide a phrase.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Names in `dir` that the code map must list: `x.rs` for a Rust file, `x/` for a folder.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().unwrap().is_dir() {
                Some(format!("{name}/"))
            } else {
                name.ends_with(".rs").then_some(name)
            }
        })
        .collect();
    names.sort();
    names
}

#[test]
fn code_map_lists_the_engine_the_shell_and_the_ui() {
    let start = ARCHITECTURE.find("## 8. Code map").expect("ARCHITECTURE.md has a section 8, Code map");
    let map = &ARCHITECTURE[start..];
    let map = &map[..map.find("## 9.").expect("section 9 follows the code map")];
    for dir in ["crates/codly-core/src", "src-tauri/src", "src"] {
        let missing: Vec<String> = entries(&repo().join(dir)).into_iter().filter(|name| !map.contains(name.as_str())).collect();
        assert!(missing.is_empty(), "docs/ARCHITECTURE.md section 8 does not list these entries of {dir}: {missing:?}. Add a line for each.");
    }
}

/// Sentences that were true once. Codly now has a Push button, merges into the branch you have
/// checked out when you press Merge, can read the Claude sign-in token when you switch that
/// on, stores keys you give it in the Keychain, and ships third-party models and sounds.
#[test]
fn claims_the_code_no_longer_keeps_stay_out() {
    const STALE: &[&str] = &[
        "never pushes.",
        "never pushes,",
        "never push |",
        "merges only into integration branches",
        "never reads, stores or proxies",
        "never reads tokens or keychains",
        "never touches credentials",
        "zero third-party assets",
        "0 % cpu idle",
    ];
    for (file, text) in [("README.md", README), ("docs/ARCHITECTURE.md", ARCHITECTURE)] {
        let text = flat(text);
        let found: Vec<&&str> = STALE.iter().filter(|s| text.contains(**s)).collect();
        assert!(found.is_empty(), "{file} says {found:?}, which the code does not keep. Say what it does now.");
    }
}

/// The one read of a CLI's own credential must stay documented where users look: what turns
/// it on and where the token goes.
#[test]
fn the_claude_usage_read_is_documented() {
    for (file, text) in [("README.md", README), ("docs/ARCHITECTURE.md", ARCHITECTURE)] {
        let text = flat(text);
        for needle in ["show claude usage before an agent runs", "api.anthropic.com"] {
            assert!(text.contains(needle), "{file} must say \"{needle}\": crates/codly-core/src/quota.rs reads the Claude sign-in token when that switch is on");
        }
    }
}
