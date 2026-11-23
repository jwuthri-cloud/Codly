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

/// The names that the code map lists under `header`: the first column of the lines indented
/// by two spaces, up to the next line at column 0. Descriptions and deeper lines (a wrapped
/// description, the orchestrator sub-list) do not count, so a name in another block, or one
/// that is the end of a longer name, cannot stand in for a missing line.
fn listed<'a>(map: &'a str, header: &str) -> Vec<&'a str> {
    map.lines()
        .skip_while(|line| line.split(' ').next() != Some(header))
        .skip(1)
        .take_while(|line| line.starts_with(' '))
        .filter_map(|line| line.strip_prefix("  ").filter(|entry| !entry.starts_with(' ')))
        .flat_map(|entry| entry.split("  ").next().unwrap_or_default().split_whitespace())
        .collect()
}

#[test]
fn code_map_lists_the_engine_the_shell_and_the_ui() {
    let start = ARCHITECTURE.find("## 8. Code map").expect("ARCHITECTURE.md has a section 8, Code map");
    let map = &ARCHITECTURE[start..];
    let map = &map[..map.find("## 9.").expect("section 9 follows the code map")];
    for dir in ["crates/codly-core/src", "src-tauri/src", "src"] {
        let names = listed(map, &format!("{dir}/"));
        assert!(!names.is_empty(), "docs/ARCHITECTURE.md section 8 has no `{dir}/` block");
        let missing: Vec<String> = entries(&repo().join(dir)).into_iter().filter(|name| !names.contains(&name.as_str())).collect();
        assert!(missing.is_empty(), "docs/ARCHITECTURE.md section 8 does not list these entries of {dir}: {missing:?}. Add a line for each.");
    }
}

/// The cases a search of the whole section let through: each name below is in the map, but
/// not as an entry of the block that is asked for.
#[test]
fn a_name_counts_only_as_an_entry_of_its_own_block() {
    let map = "\
crates/codly-core/src/    the engine
  model.rs          contract (ts-rs -> src/lib/gen), not git.rs
  orchestrator/     the facade:
                      agent_git.rs ide.rs    Git tab; IDE facade
  lot.rs price.rs   build mode
crates/codly-core/        build.rs
src-tauri/src/            the shell
  main.rs lib.rs    entry point
  hire_commands.rs ide_commands.rs
src/                      the app
  lib/              api.ts
";
    assert_eq!(listed(map, "crates/codly-core/src/"), ["model.rs", "orchestrator/", "lot.rs", "price.rs"]);
    assert_eq!(listed(map, "src-tauri/src/"), ["main.rs", "lib.rs", "hire_commands.rs", "ide_commands.rs"]);
    assert_eq!(listed(map, "src/"), ["lib/"]);
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

/// Which agents have an OS sandbox must stay where users look. Autonomous mode is a list of
/// risky commands, Claude's sandbox is a switch (`Settings.claude_sandbox`), and the shell's
/// Bedrock, Vertex or token sign-in reaches Claude only with its own switch
/// (`Settings.claude_shell_auth`).
#[test]
fn the_sandbox_and_the_shell_sign_in_are_documented() {
    let readme = flat(README);
    for needle in [
        "not a sandbox",
        "sandbox claude's shell",
        "the approval card names the host",
        "gemini, opencode, cursor and copilot agents have no os sandbox",
        "use my shell's claude sign-in",
        "claude_code_use_bedrock",
    ] {
        assert!(readme.contains(needle), "README.md must say \"{needle}\": see adapters/claude.rs `settings_json` and env.rs `apply_shell_auth`");
    }
}
