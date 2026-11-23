//! Try the work before you merge it: run a command of your own in a plan's integration
//! worktree, or in an agent's worktree, and read its output while it runs.
//!
//! - The command is yours. Codly guesses none: the one you ran last is kept for the repo and
//!   shown again, and it runs when you press Run on the text you see.
//! - It runs as your terminal would run it (`/bin/sh -c`, your login environment), with the
//!   worktree's port block in `CODLY_PORT_BASE` (worktree_setup.rs): a dev server started
//!   with `--port $CODLY_PORT_BASE` does not collide with the one of your checkout.
//! - It leads a process group of its own, and the whole group ends with it: on Stop, when
//!   that work is merged, when its worktree is removed and when Codly quits (SIGTERM, then
//!   SIGKILL for what is left of the group after `STOP_GRACE`). A command that ends by itself
//!   takes along what it left in the background.
//! - A finished plan and a solo agent's own worktree only. A task's worktree is the gate's:
//!   it commits everything in it and checks the files, so a file your command wrote there
//!   would go into the task's commit. In a solo agent's worktree it does go into the agent's
//!   next commit, and the page says so before you run.
//! - One preview per worktree. Its output lives in memory only (the last `MAX_LINES` lines).
//!
//! ponytail: a preview does not outlive a crash of Codly cleanly: its processes go on until
//! you end them. Killing by a recorded id at the next start could hit an unrelated process.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::sync::{Notify, watch};
use ts_rs::TS;

use super::worktree_setup::PORT_VAR;
use super::{Core, Inner, lock, prompts};
use crate::adapters::classify::strip_ansi;
use crate::model::*;

/// After SIGTERM, how long the command and everything it started have to end before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// How often a running preview looks whether its worktree is still there.
const WORKTREE_LOOK: Duration = Duration::from_secs(2);
/// After the end, how long the last output lines may take to arrive.
const DRAIN: Duration = Duration::from_secs(1);
const MAX_LINES: usize = 2000;
const MAX_LINE_BYTES: usize = 2000;
const MAX_COMMAND_CHARS: usize = 2000;
const COMMAND_PREFIX: &str = "preview_command:";

/// The previews of this process, by worktree: the one that runs there, or the last one that
/// ended (kept until the next one, so its output can still be read).
static PREVIEWS: LazyLock<Mutex<HashMap<PathBuf, Arc<Run>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

static LOCAL_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https?)://(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\]):(\d{2,5})(/[\w\-./~%]*)?").unwrap());

/// Whose worktree a preview runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[ts(export)]
pub enum PreviewSource {
    /// A plan: its integration worktree, where the work of its tasks is merged.
    Plan(Id),
    /// An agent: its own worktree.
    Agent(Id),
}

/// Start or stop the preview of a worktree.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "do", rename_all = "snake_case")]
#[ts(export)]
pub enum PreviewAction {
    /// Run `command`, the text you saw and confirmed, and keep it as this repo's command.
    Start { command: String },
    /// End the command that runs, with everything it started.
    Stop,
}

/// What the page already shows of a run, so that a refresh brings the new lines only.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct PreviewSeen {
    /// `PreviewRun::id`.
    pub run: Id,
    /// `PreviewRun::printed` of the last answer.
    pub lines: i64,
}

/// The preview of one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Preview {
    /// The folder the command runs in.
    pub worktree: String,
    /// The command you ran last in this repo. None = you ran none yet.
    pub command: Option<String>,
    /// The first port of the worktree's own block, as `CODLY_PORT_BASE` gives it to the
    /// command. None = the worktree has no block (yet: it gets one at its first run).
    pub port_base: Option<u16>,
    /// The command that runs here, or the last one that ended. None = none since Codly started.
    pub run: Option<PreviewRun>,
}

/// One run of a command in a worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewRun {
    pub id: Id,
    pub command: String,
    pub started_at: i64,
    /// How it ended, in words ("ended", "exited with code 1", "stopped"). None = it runs.
    pub ended: Option<String>,
    /// The first address on this machine that its output named: where to look at the result.
    pub url: Option<String>,
    /// Output lines (stdout and stderr together), after the ones `PreviewSeen` said you have.
    pub lines: Vec<String>,
    /// How many lines it printed so far: send it back as `PreviewSeen::lines`.
    pub printed: i64,
}

/// A command started in a worktree, running or ended.
struct Run {
    id: Id,
    /// The data folder of the Core that started it: a Core that shuts down ends its own.
    owner: PathBuf,
    command: String,
    started_at: i64,
    log: Mutex<Log>,
    /// Stop was asked.
    stopping: Notify,
    /// How it ended. Set once: after the process group is gone and the output is complete.
    end: watch::Sender<Option<String>>,
}

impl Run {
    /// Ask the command to end. Returns at once.
    fn stop(&self) {
        self.stopping.notify_one();
    }

    /// Wait until it has ended and its process group is gone.
    async fn wait(&self) {
        let mut end = self.end.subscribe();
        // The sender lives in `self`, so the channel cannot close while we wait.
        let _ = end.wait_for(Option::is_some).await;
    }

    fn view(&self, seen: Option<&PreviewSeen>) -> PreviewRun {
        // The end before the output: an ended run's output is complete, so a page that reads
        // "ended" has every line and can stop asking.
        let ended = self.end.borrow().clone();
        let log = lock(&self.log);
        let kept_from = log.printed - log.lines.len() as i64;
        let have = seen.filter(|s| s.run == self.id).map_or(0, |s| s.lines);
        let skip = (have - kept_from).clamp(0, log.lines.len() as i64) as usize;
        PreviewRun {
            id: self.id.clone(),
            command: self.command.clone(),
            started_at: self.started_at,
            ended,
            url: log.url.clone(),
            lines: log.lines.iter().skip(skip).cloned().collect(),
            printed: log.printed,
        }
    }
}

/// The last `MAX_LINES` output lines of a run.
#[derive(Default)]
struct Log {
    lines: VecDeque<String>,
    /// Lines printed in all, the dropped ones included.
    printed: i64,
    url: Option<String>,
}

impl Log {
    fn push(&mut self, line: &[u8]) {
        let line: String = strip_ansi(&String::from_utf8_lossy(line)).chars().filter(|c| !c.is_control() || *c == '\t').collect();
        if self.url.is_none() {
            self.url = local_url(&line);
        }
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.printed += 1;
    }
}

/// Cuts a stream into lines. `\n` and `\r\n` end a line; so does a lone `\r` (a progress line
/// that a tool draws again and again), which leaves no empty line behind. A line is cut at
/// `MAX_LINE_BYTES`.
#[derive(Default)]
struct Lines {
    line: Vec<u8>,
    /// The last byte was `\r`.
    cr: bool,
}

impl Lines {
    fn feed(&mut self, chunk: &[u8], mut out: impl FnMut(&[u8])) {
        for &b in chunk {
            if std::mem::take(&mut self.cr) && b != b'\n' {
                self.end(&mut out);
            }
            match b {
                b'\n' => {
                    out(&self.line);
                    self.line.clear();
                }
                b'\r' => self.cr = true,
                _ if self.line.len() < MAX_LINE_BYTES => self.line.push(b),
                _ => {}
            }
        }
    }

    /// The stream ended: what is left is a line.
    fn end(&mut self, mut out: impl FnMut(&[u8])) {
        if !self.line.is_empty() {
            out(&self.line);
            self.line.clear();
        }
    }
}

/// The first address on this machine that `line` names, as a link a browser opens. A server
/// that listens on every interface (`0.0.0.0`, `[::]`) is reached at `localhost`.
fn local_url(line: &str) -> Option<String> {
    let found = LOCAL_URL.captures(line)?;
    let host = match &found[2] {
        "0.0.0.0" | "[::]" => "localhost",
        host => host,
    };
    let port: u16 = found[3].parse().ok()?;
    let path = found.get(4).map_or("", |p| p.as_str().trim_end_matches('.'));
    Some(format!("{}://{host}:{port}{path}", &found[1]))
}

fn command_key(ws: &Workspace) -> String {
    format!("{COMMAND_PREFIX}{}", ws.id)
}

async fn pump(mut from: impl tokio::io::AsyncRead + Unpin, run: Arc<Run>) {
    let mut buf = [0u8; 8192];
    let mut lines = Lines::default();
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => lines.feed(&buf[..n], |line| lock(&run.log).push(line)),
        }
    }
    lines.end(|line| lock(&run.log).push(line));
}

/// Start `command` in `wt`, at the head of a process group of its own.
fn spawn(owner: &Path, wt: &Path, command: &str, env: HashMap<String, String>) -> Result<Arc<Run>> {
    let mut sh = tokio::process::Command::new("/bin/sh");
    sh.arg("-c")
        .arg(command)
        .current_dir(wt)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = sh.spawn().context("could not start /bin/sh")?;
    let pgid = child.id().and_then(|pid| i32::try_from(pid).ok()).filter(|pid| *pid > 1).ok_or_else(|| anyhow!("the command has no process id"))?;
    let run = Arc::new(Run {
        id: new_id(),
        owner: owner.to_path_buf(),
        command: command.to_string(),
        started_at: now_ms(),
        log: Mutex::default(),
        stopping: Notify::new(),
        end: watch::Sender::new(None),
    });
    let mut pumps = vec![];
    if let Some(out) = child.stdout.take() {
        pumps.push(tokio::spawn(pump(out, run.clone())));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(tokio::spawn(pump(err, run.clone())));
    }
    tokio::spawn(supervise(run.clone(), child, pgid, wt.to_path_buf(), pumps));
    Ok(run)
}

/// Watch a run to its end, end its process group, and say how it ended.
async fn supervise(run: Arc<Run>, mut child: tokio::process::Child, pgid: i32, wt: PathBuf, pumps: Vec<tokio::task::JoinHandle<()>>) {
    let mut look = tokio::time::interval(WORKTREE_LOOK);
    // Why Codly ends it. None = the command ended by itself.
    let cut = loop {
        tokio::select! {
            _ = child.wait() => break None,
            _ = run.stopping.notified() => break Some("stopped"),
            _ = look.tick() => {
                if !wt.is_dir() {
                    break Some("stopped: its worktree was removed");
                }
            }
        }
    };
    if cut.is_some() {
        // The grace time is the whole group's, not the shell's alone: `cd web && pnpm dev` is a
        // shell that ends at once and a server that needs a moment to clean up. The shell is
        // waited for meanwhile: until then it counts as a member of the group.
        let _ = tokio::join!(crate::proc::kill_group(pgid, STOP_GRACE), child.wait());
    } else {
        // What a command that ended by itself left in the background would hold the ports. At
        // once, as the gate does after a verify command: the group's id is the command's
        // process id, which the system gives to another process later.
        let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
    }
    let status = child.wait().await;
    let ended = match (cut, status) {
        (Some(why), _) => why.to_string(),
        (None, Ok(status)) => match status.code() {
            Some(0) => "ended".to_string(),
            Some(code) => format!("exited with code {code}"),
            None => "ended by a signal".to_string(),
        },
        (None, Err(e)) => format!("ended, and Codly could not read how: {e}"),
    };
    for mut pump in pumps {
        // A process that left the group can hold the output open: its lines are not waited for.
        if tokio::time::timeout(DRAIN, &mut pump).await.is_err() {
            pump.abort();
        }
    }
    run.end.send_replace(Some(ended));
}

/// End the preview that runs in `wt`, if one does, and wait until its process group is gone.
pub(super) async fn stop_in(wt: &Path) {
    let run = lock(&PREVIEWS).get(wt).cloned();
    if let Some(run) = run {
        run.stop();
        run.wait().await;
    }
}

/// End every preview that the Core with the data folder `owner` started: Codly quits.
pub(super) async fn stop_all(owner: &Path) {
    let runs: Vec<Arc<Run>> = lock(&PREVIEWS).values().filter(|run| run.owner == owner).cloned().collect();
    for run in &runs {
        run.stop();
    }
    for run in &runs {
        run.wait().await;
    }
}

/// Where a preview runs.
struct Place {
    ws: Workspace,
    wt: PathBuf,
    /// The worktree in words: "the worktree of the plan \"Dark mode\"", "Ada's worktree".
    label: String,
    /// Why no command can start there now.
    wait: Option<String>,
}

impl Inner {
    fn preview_place(&self, source: &PreviewSource) -> Result<Place> {
        match source {
            PreviewSource::Plan(id) => {
                let plan = self.plan(id)?;
                let wt = plan.integration_worktree.clone().ok_or_else(|| anyhow!("the plan never ran, so there is nothing to try"))?;
                let ws = self.store.workspace(&plan.workspace_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
                // Codly merges the tasks' work in that folder while the plan runs.
                let running = matches!(plan.status, PlanStatus::Running | PlanStatus::Integrating);
                let wait = running.then(|| "the plan is still running; try it when it is done".to_string());
                Ok(Place { ws, wt: PathBuf::from(wt), label: format!("the worktree of the plan \"{}\"", prompts::clip(&plan.summary, 60)), wait })
            }
            PreviewSource::Agent(id) => {
                let agent = self.agent(id)?;
                let wt = agent.worktree.clone().ok_or_else(|| anyhow!("{} has no worktree yet, so there is nothing to try", agent.name))?;
                let ws_id = self.store.agent_workspace(id)?.ok_or_else(|| anyhow!("{} is in no office", agent.name))?;
                let ws = self.store.workspace(&ws_id)?.ok_or_else(|| anyhow!("unknown workspace"))?;
                let wait = if Path::new(&wt) == Path::new(&ws.repo_path) {
                    Some(format!("{} works in your project folder itself: run the command in your own terminal", agent.name))
                } else if agent.role != Role::Solo || !Path::new(&wt).starts_with(self.worktree_root(&ws).join("solo")) {
                    // The gate commits everything in a task's worktree and checks the files. A
                    // solo agent that took a task is in that task's worktree, not in its own
                    // (`solo/<name>`, see `solo_worktree`).
                    Some("only a solo agent's own work can be tried this way; a task's work is tried with its plan, when the plan is done".to_string())
                } else {
                    None
                };
                Ok(Place { ws, wt: PathBuf::from(wt), label: format!("{}'s worktree", agent.name), wait })
            }
        }
    }

    fn preview_view(&self, place: &Place, seen: Option<&PreviewSeen>) -> Result<Preview> {
        let run = lock(&PREVIEWS).get(&place.wt).cloned();
        Ok(Preview {
            worktree: place.wt.to_string_lossy().into_owned(),
            command: self.store.kv_get(&command_key(&place.ws))?,
            port_base: self.worktree_env(&place.wt).into_iter().find(|(name, _)| name == PORT_VAR).and_then(|(_, port)| port.parse().ok()),
            run: run.map(|run| run.view(seen)),
        })
    }

    /// Run `command` in the place's worktree and keep it as the repo's command.
    async fn preview_start(&self, place: &Place, command: &str) -> Result<()> {
        let command = command.trim();
        if command.is_empty() {
            bail!("write the command to run first");
        }
        if command.chars().count() > MAX_COMMAND_CHARS || command.contains('\0') {
            bail!("the command is too long (the limit is {MAX_COMMAND_CHARS} characters); put it in a script and run that");
        }
        if let Some(wait) = &place.wait {
            bail!("{wait}");
        }
        if !place.wt.is_dir() {
            bail!("{} is gone ({})", place.label, place.wt.display());
        }
        self.git.check_worktree(Path::new(&place.ws.repo_path), &place.wt).await?;
        // A worktree from before the port blocks gets its block now.
        self.assign_ports(&place.wt)?;
        let mut env = self.base_env.clone();
        crate::env::refresh_path(&mut env); // a tool installed since Codly started is found
        env.extend(self.worktree_env(&place.wt));
        env.insert("CODLY_REPO_PATH".to_string(), place.ws.repo_path.clone());
        {
            let mut previews = lock(&PREVIEWS);
            if previews.get(&place.wt).is_some_and(|run| run.end.borrow().is_none()) {
                bail!("a command already runs in this worktree; stop it first");
            }
            self.store.kv_set(&command_key(&place.ws), command)?;
            // The output of a run whose worktree is gone is of no use any more.
            previews.retain(|wt, run| run.end.borrow().is_none() || wt.is_dir());
            let run = spawn(&self.cfg.data_dir, &place.wt, command, env)?;
            previews.insert(place.wt.clone(), run);
        }
        let summary = format!("you ran `{}` in {}", prompts::clip(command, 120), place.label);
        let worktree = place.wt.to_string_lossy();
        if let Err(e) = self.decide_by(Some(&place.ws.id), None, "preview", "human", summary, json!({ "worktree": worktree, "command": command })) {
            tracing::warn!("could not record the preview in {worktree}: {e}");
        }
        Ok(())
    }
}

impl Core {
    /// The preview of a plan's integration worktree or of an agent's worktree: the command
    /// kept for the repo, and the run there with its output after `seen`. `action` first
    /// starts your command there or stops the one that runs. Nothing runs without a Start.
    pub async fn preview(&self, source: &PreviewSource, action: Option<PreviewAction>, seen: Option<&PreviewSeen>) -> Result<Preview> {
        let place = self.inner.preview_place(source)?;
        match action {
            Some(PreviewAction::Start { command }) => self.inner.preview_start(&place, &command).await?,
            Some(PreviewAction::Stop) => stop_in(&place.wt).await,
            None => {}
        }
        self.inner.preview_view(&place, seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{CoreConfig, StartOptions, row};

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repo with one commit, in `root/<name>`.
    fn temp_repo(root: &Path, name: &str) -> PathBuf {
        let repo = root.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.name", "Test"]);
        sh(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("app.txt"), "app\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        repo
    }

    async fn start(data: &Path, repo: &Path) -> (Arc<Core>, Workspace) {
        let opts = StartOptions { env: Some(std::env::vars().collect()), clis: Some(vec![]), ..Default::default() };
        let core = Core::start_with(CoreConfig { data_dir: data.to_path_buf(), app_exe: "/usr/bin/true".into() }, opts).await.unwrap();
        let ws = core.open_workspace(repo.to_str().unwrap()).await.unwrap();
        (core, ws)
    }

    /// A plan on record with the given status: its integration worktree on disk, with work
    /// of its own on its branch.
    async fn plan(core: &Core, ws: &Workspace, short: &str, status: PlanStatus) -> (PreviewSource, PathBuf) {
        let inner = &core.inner;
        let repo = Path::new(&ws.repo_path);
        let wt = inner.worktree_root(ws).join(short).join("integration");
        let branch = format!("codly/{short}/integration");
        inner.git.worktree_add(repo, &wt, &branch, "main").await.unwrap();
        std::fs::write(wt.join(format!("{short}.txt")), "work\n").unwrap();
        inner.git.commit_all(&wt, "codly(t1): work").await.unwrap().unwrap();
        let plan = Plan {
            id: new_id(),
            workspace_id: ws.id.clone(),
            thread: "lead".into(),
            request: "do it".into(),
            summary: format!("Plan {short}"),
            assumptions: vec![],
            open_questions: vec![],
            weak_checks: vec![],
            status,
            base_commit: inner.git.head(repo).await.unwrap(),
            base_branch: Some("main".into()),
            integration_branch: Some(branch),
            integration_worktree: Some(wt.to_string_lossy().into_owned()),
            merged_sha: None,
            kept: false,
            created_at: now_ms(),
            updated_at: now_ms(),
        };
        inner.emit(Some(&ws.id), row(None, None, None, EventKind::PlanChanged { plan: plan.clone() })).unwrap();
        (PreviewSource::Plan(plan.id), wt)
    }

    /// A solo agent on record with a worktree of its own on `codly/solo/<name>`.
    async fn solo(core: &Core, ws: &Workspace, name: &str) -> (Agent, PathBuf) {
        let inner = &core.inner;
        let wt = inner.worktree_root(ws).join("solo").join(name);
        inner.git.worktree_add(Path::new(&ws.repo_path), &wt, &format!("codly/solo/{name}"), "main").await.unwrap();
        let agent = inner.create_agent(&ws.id, name, Provider::Claude, Role::Solo, 0, None).unwrap();
        let agent = inner.update_agent(&agent.id, |a| a.worktree = Some(wt.to_string_lossy().into_owned())).unwrap();
        (agent, wt)
    }

    async fn run_in(core: &Core, source: &PreviewSource, command: &str) -> Result<Preview> {
        core.preview(source, Some(PreviewAction::Start { command: command.to_string() }), None).await
    }

    /// Why `command` did not start.
    async fn refused(core: &Core, source: &PreviewSource, command: &str) -> String {
        run_in(core, source, command).await.unwrap_err().to_string()
    }

    /// Read the run of `source` until `done` says yes (10 s at most).
    async fn until(core: &Core, source: &PreviewSource, done: impl Fn(&PreviewRun) -> bool) -> PreviewRun {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let run = core.preview(source, None, None).await.unwrap().run.expect("a run");
            if done(&run) {
                return run;
            }
            assert!(tokio::time::Instant::now() < deadline, "still waiting: {run:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The summaries of the Decisions on record.
    fn decisions(core: &Core, ws: &Workspace) -> Vec<String> {
        let events = core.inner.store.latest_events(Some(&ws.id), 300).unwrap();
        events.into_iter().filter_map(|e| if let EventKind::Decision { summary, .. } = e.kind { Some(summary) } else { None }).collect()
    }

    #[test]
    fn output_is_cut_into_lines() {
        let mut got: Vec<String> = vec![];
        let mut lines = Lines::default();
        let mut feed = |lines: &mut Lines, chunk: &str| lines.feed(chunk.as_bytes(), |l| got.push(String::from_utf8_lossy(l).into_owned()));
        feed(&mut lines, "one\ntw");
        feed(&mut lines, "o\r");
        feed(&mut lines, "\n\r\n10%\r50%\r100%\nlast");
        lines.end(|l| got.push(String::from_utf8_lossy(l).into_owned()));
        assert_eq!(got, ["one", "two", "", "10%", "50%", "100%", "last"]);

        let mut long = Lines::default();
        let mut cut = vec![];
        long.feed("x".repeat(MAX_LINE_BYTES + 50).as_bytes(), |_| unreachable!("no line end yet"));
        long.feed(b"\n", |l| cut.push(l.len()));
        assert_eq!(cut, [MAX_LINE_BYTES]);
    }

    #[test]
    fn the_log_keeps_the_last_lines_as_plain_text_and_finds_the_local_address() {
        let mut log = Log::default();
        log.push(b"\x1b[32m  VITE\x1b[0m ready\x07");
        log.push("  \u{279c}  Local:   http://127.0.0.1:5173/".as_bytes());
        log.push(b"  Network: http://localhost:9999/");
        assert_eq!(log.lines, ["  VITE ready", "  \u{279c}  Local:   http://127.0.0.1:5173/", "  Network: http://localhost:9999/"]);
        assert_eq!(log.url.as_deref(), Some("http://127.0.0.1:5173/"), "the first one stays");
        for _ in 0..MAX_LINES {
            log.push(b"more");
        }
        assert_eq!((log.lines.len(), log.printed), (MAX_LINES, MAX_LINES as i64 + 3));

        assert_eq!(local_url("listening on http://0.0.0.0:3000").as_deref(), Some("http://localhost:3000"));
        assert_eq!(local_url("- Local: https://localhost:8443/app/.").as_deref(), Some("https://localhost:8443/app/"));
        assert_eq!(local_url("on http://[::]:8080/ and http://[::1]:8081").as_deref(), Some("http://localhost:8080/"));
        assert_eq!(local_url("see https://example.com:8080/docs"), None, "only this machine");
        assert_eq!(local_url("http://localhost:99999"), None, "not a port");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn your_command_runs_in_the_worktree_with_its_ports_and_is_kept_for_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "repo");
        let (core, ws) = start(&dir.path().join("data"), &repo).await;
        let (source, wt) = plan(&core, &ws, "ab12", PlanStatus::Done).await;

        let before = core.preview(&source, None, None).await.unwrap();
        assert_eq!((before.command, before.port_base, before.run), (None, None, None), "nothing is guessed, nothing runs by itself");
        assert_eq!(before.worktree, wt.to_string_lossy());

        let command = r#"echo "listening on http://0.0.0.0:$CODLY_PORT_BASE/app in $(pwd -P)"; echo "repo $CODLY_REPO_PATH" >&2"#;
        let started = run_in(&core, &source, &format!("  {command}\n")).await.unwrap();
        assert_eq!(started.command.as_deref(), Some(command));
        assert_eq!(started.port_base, Some(20_000), "the worktree got a port block for its first run");
        let run = until(&core, &source, |run| run.ended.is_some()).await;
        assert_eq!((run.command.as_str(), run.ended.as_deref()), (command, Some("ended")));
        let here = format!("listening on http://0.0.0.0:20000/app in {}", std::fs::canonicalize(&wt).unwrap().display());
        assert!(run.lines.contains(&here), "{:?}", run.lines);
        assert!(run.lines.contains(&format!("repo {}", ws.repo_path)), "stderr too: {:?}", run.lines);
        assert_eq!(run.url.as_deref(), Some("http://localhost:20000/app"));
        assert_eq!(run.printed, 2);

        // A page that has the first line gets the second only; one that shows another run gets all.
        let seen = |run: &str, lines| PreviewSeen { run: run.to_string(), lines };
        let more = core.preview(&source, None, Some(&seen(&run.id, 1))).await.unwrap().run.unwrap();
        assert_eq!((more.lines.as_slice(), more.printed), (&run.lines[1..], 2));
        let all = core.preview(&source, None, Some(&seen("another run", 2))).await.unwrap().run.unwrap();
        assert_eq!(all.lines, run.lines);

        // The command is the repo's: another worktree of it offers the same one, another repo none.
        let (ada, _) = solo(&core, &ws, "ada").await;
        let offered = core.preview(&PreviewSource::Agent(ada.id.clone()), None, None).await.unwrap();
        assert_eq!((offered.command.as_deref(), offered.run), (Some(command), None));
        assert!(decisions(&core, &ws).iter().any(|d| d.starts_with("you ran `echo \"listening") && d.ends_with("in the worktree of the plan \"Plan ab12\"")), "{:?}", decisions(&core, &ws));
        let other = temp_repo(dir.path(), "other");
        let ws2 = core.open_workspace(other.to_str().unwrap()).await.unwrap();
        let (bo, _) = solo(&core, &ws2, "bo").await;
        assert_eq!(core.preview(&PreviewSource::Agent(bo.id), None, None).await.unwrap().command, None);
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_ends_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "repo");
        let (core, ws) = start(&dir.path().join("data"), &repo).await;
        let (source, wt) = plan(&core, &ws, "cd34", PlanStatus::Done).await;
        let beat = wt.join("beat.log");
        let beats = || std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);

        // The command leaves a loop in the background that ignores SIGTERM, as a dev server's
        // helper process may. The loop writes a line every 50 ms while it lives.
        run_in(&core, &source, "(trap '' TERM; while :; do echo beat >> beat.log; sleep 0.05; done) & echo started; wait").await.unwrap();
        let live = until(&core, &source, |run| run.lines.contains(&"started".to_string()) && beats() > 0).await;
        assert_eq!(live.ended, None);
        let e = run_in(&core, &source, "echo second").await.unwrap_err();
        assert!(e.to_string().contains("already runs in this worktree"), "{e}");
        assert_ne!(core.preview(&source, None, None).await.unwrap().command.as_deref(), Some("echo second"), "a refused command is not kept");

        let stopped = core.preview(&source, Some(PreviewAction::Stop), None).await.unwrap().run.unwrap();
        assert_eq!((stopped.id.as_str(), stopped.ended.as_deref()), (live.id.as_str(), Some("stopped")));
        let at_stop = beats();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(beats(), at_stop, "nothing of the group runs after Stop");

        // A command that ignores SIGTERM itself is killed after the grace time.
        run_in(&core, &source, "trap '' TERM; echo deaf; while :; do sleep 0.05; done").await.unwrap();
        let deaf = until(&core, &source, |run| run.lines.contains(&"deaf".to_string())).await;
        assert_ne!(deaf.id, live.id, "a new run takes the place of the ended one");
        let asked = tokio::time::Instant::now();
        let stopped = core.preview(&source, Some(PreviewAction::Stop), None).await.unwrap().run.unwrap();
        assert_eq!(stopped.ended.as_deref(), Some("stopped"));
        assert!(asked.elapsed() >= STOP_GRACE, "it had its time to end: {:?}", asked.elapsed());
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn what_the_command_started_has_the_grace_time_to_clean_up() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "repo");
        let (core, ws) = start(&dir.path().join("data"), &repo).await;
        let (source, wt) = plan(&core, &ws, "gh78", PlanStatus::Done).await;

        // As `cd web && pnpm dev`: a shell that ends at SIGTERM, and under it a server that
        // cleans up first (here: 0.3 s of work, then a file).
        let command = r#"sh -c 'trap "sleep 0.3; echo bye > done.txt; exit 0" TERM; echo ready; while :; do sleep 0.05; done' & wait"#;
        run_in(&core, &source, command).await.unwrap();
        until(&core, &source, |run| run.lines.contains(&"ready".to_string())).await;
        let asked = tokio::time::Instant::now();
        let stopped = core.preview(&source, Some(PreviewAction::Stop), None).await.unwrap().run.unwrap();
        assert_eq!(stopped.ended.as_deref(), Some("stopped"));
        assert_eq!(std::fs::read_to_string(wt.join("done.txt")).ok().as_deref(), Some("bye\n"), "the server was not killed while it cleaned up");
        assert!(asked.elapsed() < STOP_GRACE, "Stop does not wait longer than the group lives: {:?}", asked.elapsed());
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_preview_ends_when_its_work_is_merged_or_its_worktree_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "repo");
        let (core, ws) = start(&dir.path().join("data"), &repo).await;

        // A plan: merged into your branch, its preview ends before its worktree goes.
        let (source, wt) = plan(&core, &ws, "ef56", PlanStatus::Done).await;
        let PreviewSource::Plan(plan_id) = &source else { unreachable!() };
        run_in(&core, &source, "echo up; sleep 300").await.unwrap();
        until(&core, &source, |run| run.lines.contains(&"up".to_string())).await;
        core.merge_plan(plan_id).await.unwrap();
        let run = core.preview(&source, None, None).await.unwrap().run.unwrap();
        assert_eq!(run.ended.as_deref(), Some("stopped"));
        assert!(!wt.exists(), "the merged plan's worktree is removed");
        let e = run_in(&core, &source, "echo again").await.unwrap_err();
        assert!(e.to_string().contains("is gone"), "{e}");

        // A solo agent: its preview ends before Codly commits the worktree for the merge, so
        // nothing writes there meanwhile. This command writes a new file every 20 ms. The
        // agent keeps its worktree.
        let (ada, ada_wt) = solo(&core, &ws, "ada").await;
        let ada_source = PreviewSource::Agent(ada.id.clone());
        std::fs::write(ada_wt.join("ada.txt"), "work\n").unwrap();
        run_in(&core, &ada_source, "i=0; while :; do i=$((i+1)); echo $i > made-$i.txt; sleep 0.02; done").await.unwrap();
        until(&core, &ada_source, |_| ada_wt.join("made-1.txt").exists()).await;
        core.merge_agent(&ada.id).await.unwrap();
        let run = core.preview(&ada_source, None, None).await.unwrap().run.unwrap();
        assert_eq!(run.ended.as_deref(), Some("stopped"));
        assert!(ada_wt.is_dir());
        assert_eq!(sh(&ada_wt, &["status", "--porcelain"]), "", "no file was written after the commit");
        assert!(repo.join("made-1.txt").exists(), "what the command wrote is part of the merged work");

        // A worktree removed in any other way: the preview notices and ends.
        run_in(&core, &ada_source, "echo up; sleep 300").await.unwrap();
        until(&core, &ada_source, |run| run.lines.contains(&"up".to_string())).await;
        std::fs::remove_dir_all(&ada_wt).unwrap();
        let run = until(&core, &ada_source, |run| run.ended.is_some()).await;
        assert_eq!(run.ended.as_deref(), Some("stopped: its worktree was removed"));
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quitting_ends_the_previews_of_that_core_only() {
        let dir = tempfile::tempdir().unwrap();
        let (repo_a, repo_b) = (temp_repo(dir.path(), "a"), temp_repo(dir.path(), "b"));
        let (core_a, ws_a) = start(&dir.path().join("data-a"), &repo_a).await;
        let (core_b, ws_b) = start(&dir.path().join("data-b"), &repo_b).await;
        let (source_a, wt_a) = plan(&core_a, &ws_a, "aa11", PlanStatus::Done).await;
        let (source_b, wt_b) = plan(&core_b, &ws_b, "bb22", PlanStatus::Done).await;
        run_in(&core_a, &source_a, "echo up; sleep 300").await.unwrap();
        run_in(&core_b, &source_b, "echo up; sleep 300").await.unwrap();
        until(&core_a, &source_a, |run| run.lines.contains(&"up".to_string())).await;
        until(&core_b, &source_b, |run| run.lines.contains(&"up".to_string())).await;

        // How the run in `wt` ended, as the registry has it.
        let ended = |wt: &Path| lock(&PREVIEWS).get(wt).unwrap().end.borrow().clone();
        core_a.shutdown().await;
        assert_eq!(ended(&wt_a).as_deref(), Some("stopped"));
        assert_eq!(ended(&wt_b), None, "another Core's preview goes on");
        core_b.shutdown().await;
        assert_eq!(ended(&wt_b).as_deref(), Some("stopped"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_command_that_cannot_run_here_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let repo = temp_repo(dir.path(), "repo");
        let (core, ws) = start(&dir.path().join("data"), &repo).await;

        let (done, _) = plan(&core, &ws, "ab12", PlanStatus::Done).await;
        assert_eq!(refused(&core, &done, "  \n").await, "write the command to run first");
        assert!(refused(&core, &done, &"x".repeat(MAX_COMMAND_CHARS + 1)).await.contains("too long"));
        let (running, _) = plan(&core, &ws, "cd34", PlanStatus::Running).await;
        assert_eq!(refused(&core, &running, "pnpm dev").await, "the plan is still running; try it when it is done");
        assert!(refused(&core, &PreviewSource::Plan("nope".into()), "pnpm dev").await.contains("unknown plan"));

        let inner = &core.inner;
        let new = inner.create_agent(&ws.id, "Ada", Provider::Claude, Role::Solo, 0, None).unwrap();
        assert_eq!(refused(&core, &PreviewSource::Agent(new.id.clone()), "pnpm dev").await, "Ada has no worktree yet, so there is nothing to try");
        inner.update_agent(&new.id, |a| a.worktree = Some(ws.repo_path.clone())).unwrap();
        assert_eq!(refused(&core, &PreviewSource::Agent(new.id), "pnpm dev").await, "Ada works in your project folder itself: run the command in your own terminal");
        // A task's worktree is the gate's: not for its worker, and not for a solo agent that took the task.
        let task_wt = inner.worktree_root(&ws).join("ab12").join("t1").to_string_lossy().into_owned();
        for (name, role) in [("Bo", Role::Worker), ("Cy", Role::Solo)] {
            let agent = inner.create_agent(&ws.id, name, Provider::Claude, role, 0, None).unwrap();
            inner.update_agent(&agent.id, |a| a.worktree = Some(task_wt.clone())).unwrap();
            let e = refused(&core, &PreviewSource::Agent(agent.id), "pnpm dev").await;
            assert!(e.starts_with("only a solo agent's own work can be tried this way"), "{name}: {e}");
        }

        // Nothing ran, and a refused command is not kept either.
        let view = core.preview(&done, None, None).await.unwrap();
        assert_eq!((view.command, view.run), (None, None));
        core.shutdown().await;
    }
}
