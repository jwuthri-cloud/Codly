//! Opening a repository checks Git first: a Git that cannot be used is named with its fix,
//! where every folder used to read "is not a git repository".

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codly_core::orchestrator::{Core, CoreConfig, StartOptions};

/// `Core::shutdown`, and every run exited when it was told to stop. A fake agent that ignores
/// the stop fails here, in its own test: else the shutdown's last resort kills every process
/// group this test process started, the check commands of the tests beside it too.
async fn shut_down(core: &Core) {
    core.shutdown().await;
    assert_eq!(core.runs_given_up(), 0, "a run outlived the shutdown: its fake did not exit when told to stop");
}

/// Your own git settings (`~/.gitconfig`, the system file) do not reach the core's git (`core_env`).
const NO_GIT_CONFIG: [(&str, &str); 2] = [("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")];

/// The environment the core runs with: this process's, without your git settings.
fn core_env() -> HashMap<String, String> {
    std::env::vars().chain(NO_GIT_CONFIG.map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

/// A core whose `git` is a script with `body`. None where Codly would not run it: it takes
/// Homebrew's git before the one on PATH.
async fn core_with_git(root: &Path, body: &str) -> Option<Arc<Core>> {
    if Path::new("/opt/homebrew/bin/git").exists() {
        return None;
    }
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("git"), format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env: HashMap<String, String> = core_env();
    env.insert("PATH".into(), format!("{}:{}", bin.display(), env.get("PATH").cloned().unwrap_or_default()));
    let cfg = CoreConfig { data_dir: root.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions { spawner: None, env: Some(env), clis: Some(vec![]), index: false };
    Some(Core::start_with(cfg, opts).await.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_git_that_is_too_old_is_named_with_its_fix() {
    let dir = tempfile::tempdir().unwrap();
    let Some(core) = core_with_git(dir.path(), "echo 'git version 2.37.1 (Apple Git-137.1)'").await else { return };
    let err = core.open_workspace(dir.path().to_str().unwrap()).await.unwrap_err().to_string();
    assert!(err.contains("Git 2.37.1 is too old") && err.contains("`brew install git`"), "{err}");
    assert!(!err.contains("not a git repository"), "{err}");
    shut_down(&core).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_without_the_command_line_tools_is_told_to_install_them() {
    let dir = tempfile::tempdir().unwrap();
    let stub = "echo 'xcode-select: note: No developer tools were found, requesting install.' >&2; exit 1";
    let Some(core) = core_with_git(dir.path(), stub).await else { return };
    let err = core.open_workspace(dir.path().to_str().unwrap()).await.unwrap_err().to_string();
    assert!(err.contains("Git cannot be used") && err.contains("`xcode-select --install`"), "{err}");
    shut_down(&core).await;
}
