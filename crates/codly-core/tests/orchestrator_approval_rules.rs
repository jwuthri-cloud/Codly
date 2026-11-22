//! "Always allow in this repo" end to end, with fake agents in place of the CLI: a rule saved
//! from one agent's request answers every agent of the repo, never what Codly always asks
//! about, and stops answering once it is removed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codly_core::adapters::{AdapterEvent, AgentCmd, AgentHandle, SpawnSpec};
use codly_core::model::*;
use codly_core::orchestrator::{Core, CoreConfig, SpawnFuture, Spawner, StartOptions};
use codly_core::store::Store;
use tokio::sync::mpsc;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("LC_ALL", "C").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn temp_repo(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    repo
}

fn claude() -> CliInfo {
    CliInfo {
        provider: Provider::Claude,
        path: Some("/usr/bin/true".into()),
        version: Some("fake".into()),
        logged_in: Some(true),
        billing: Billing::Subscription,
        plan: None,
        problem: None,
        transport: "fake".into(),
        install: codly_core::env::install_hint(Provider::Claude),
        default_model: None,
    }
}

/// What Codly answered to each command a fake agent asked to run, in order.
type Answers = Arc<Mutex<Vec<(String, ApprovalChoice)>>>;

/// A fake agent: a message "run: <command>" makes it ask to run that command in its worktree,
/// and its turn ends when the answer arrives.
struct Fake(Answers);

impl Spawner for Fake {
    fn spawn(&self, spec: SpawnSpec, events: mpsc::UnboundedSender<AdapterEvent>) -> SpawnFuture {
        let answers = self.0.clone();
        Box::pin(async move {
            let (tx, mut rx) = mpsc::unbounded_channel::<AgentCmd>();
            let (agent_id, run_id) = (spec.agent_id.clone(), spec.run_id.clone());
            let send = {
                let (agent_id, run_id) = (agent_id.clone(), run_id.clone());
                move |kind| {
                    let _ = events.send(AdapterEvent { agent_id: agent_id.clone(), run_id: run_id.clone(), kind });
                }
            };
            send(EventKind::SessionStarted {
                provider: spec.provider,
                model: None,
                cwd: spec.cwd.to_string_lossy().into_owned(),
                cli_version: None,
                session_id: Some("sess".into()),
                billing: Billing::Subscription,
                tools: vec![],
            });
            let handle = AgentHandle { agent_id: agent_id.clone(), run_id: run_id.clone(), tx };
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match cmd {
                        AgentCmd::Prompt(text) => {
                            send(EventKind::TurnStarted { input: Some(text.clone()) });
                            if let Some((_, command)) = text.rsplit_once("run: ") {
                                let command = command.trim().to_string();
                                let approval = Approval {
                                    id: new_id(),
                                    agent_id: agent_id.clone(),
                                    run_id: run_id.clone(),
                                    task_id: None,
                                    call_id: Some("c1".into()),
                                    subject: ApprovalSubject::Command { command: command.clone(), cwd: None },
                                    reason: None,
                                    choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
                                    risks: vec![],
                                    status: ApprovalStatus::Pending,
                                    decided_by: None,
                                    created_at: now_ms(),
                                };
                                send(EventKind::ApprovalRequested { approval });
                                // The answer arrives as the next command.
                                if let Some(AgentCmd::Approval { decision, .. }) = rx.recv().await {
                                    answers.lock().unwrap().push((command, decision.choice));
                                }
                            }
                            send(EventKind::TurnCompleted { status: TurnStatus::Completed, result_text: None, duration_ms: Some(1) });
                        }
                        AgentCmd::Shutdown => {
                            send(EventKind::Exited { code: Some(0) });
                            break;
                        }
                        AgentCmd::Interrupt | AgentCmd::Approval { .. } | AgentCmd::Session { .. } => {}
                    }
                }
            });
            Ok(handle)
        })
    }
}

async fn wait_for<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A started core with two hired 1:1 agents in one repo (balanced mode, the default).
async fn two_agents(dir: &Path, answers: &Answers) -> (Arc<Core>, String, String) {
    let repo = temp_repo(dir);
    let cfg = CoreConfig { data_dir: dir.join("data"), app_exe: PathBuf::from("/usr/bin/true") };
    let opts = StartOptions {
        spawner: Some(Arc::new(Fake(answers.clone()))),
        env: Some(std::env::vars().collect()),
        clis: Some(vec![claude()]),
        index: false,
    };
    let core = Core::start_with(cfg, opts).await.unwrap();
    core.open_workspace(repo.to_str().unwrap()).await.unwrap();
    let ana = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    let bo = core.hire_agent(Provider::Claude, None, None).await.unwrap();
    (core, ana.id, bo.id)
}

/// Tell an agent to ask for a command (once its last turn is over).
async fn ask(core: &Core, agent: &str, command: &str) {
    wait_for("the agent to be free", async || {
        let a = core.snapshot().await.unwrap().agents.into_iter().find(|a| a.id == agent)?;
        matches!(a.status, AgentStatus::Idle | AgentStatus::Offline).then_some(())
    })
    .await;
    core.send_chat(&format!("agent:{agent}"), &format!("run: {command}")).await.unwrap();
}

/// The requests that wait for you, once there are `n`.
async fn waiting(core: &Core, n: usize) -> Vec<Approval> {
    wait_for("the requests to wait for you", async || {
        let pending = core.snapshot().await.unwrap().approvals;
        (pending.len() == n).then_some(pending)
    })
    .await
}

/// What a waiting request can be saved as, once the broker has decided to ask you about it
/// (the request shows a moment before that).
async fn offered(core: &Core, approval: &Approval) -> ApprovalRuleOffer {
    wait_for("the broker to ask", async || {
        let offer = core.approval_rule_offer(&approval.id).unwrap();
        (!offer.is_empty()).then_some(offer)
    })
    .await
}

/// What the agents were answered, once there are `n` answers.
async fn answered(answers: &Answers, n: usize) -> Vec<(String, ApprovalChoice)> {
    wait_for("the answers", async || {
        let got = answers.lock().unwrap().clone();
        (got.len() == n).then_some(got)
    })
    .await
}

fn deny() -> ApprovalDecision {
    ApprovalDecision { choice: ApprovalChoice::Deny, message: None, answers: None }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_rule_answers_every_agent_of_the_repo_until_it_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let answers = Answers::default();
    let (core, ana, bo) = two_agents(dir.path(), &answers).await;

    // Ana wants a package download: Codly asks, and the card can save it as a rule.
    ask(&core, &ana, "pnpm install").await;
    let first = waiting(&core, 1).await.remove(0);
    let offer = offered(&core, &first).await;
    assert_eq!((offer.exact.as_deref(), offer.prefix.as_deref()), (Some("pnpm install"), Some("pnpm install")));
    let rule = core.add_approval_rule(&first.id, ApprovalRuleKind::Prefix).unwrap();
    assert_eq!((rule.kind, rule.command.as_str()), (ApprovalRuleKind::Prefix, "pnpm install"));
    assert_eq!(answered(&answers, 1).await, vec![("pnpm install".to_string(), ApprovalChoice::AllowOnce)], "saving the rule answers the card");
    assert_eq!(core.approval_rules().unwrap(), vec![rule.clone()]);
    assert!(core.approval_rule_offer(&first.id).unwrap().is_empty(), "an answered request offers nothing");

    // Bo, another agent of the repo, is not asked about a command the rule covers.
    ask(&core, &bo, "pnpm install lodash").await;
    assert_eq!(answered(&answers, 2).await[1], ("pnpm install lodash".to_string(), ApprovalChoice::AllowOnce));
    waiting(&core, 0).await;
    // The automatic answer is on record, and names the rule.
    wait_for("the decision", async || {
        let events = core.events(None, None, 500).await.unwrap();
        events
            .iter()
            .any(|e| {
                matches!(&e.kind, EventKind::Decision { decision_type, summary, .. }
                if decision_type == "approval_policy" && summary.contains("your rule for this repo: commands that start with `pnpm install`"))
            })
            .then_some(())
    })
    .await;

    // A longer line that chains a push still asks, and nothing can be saved from it.
    ask(&core, &bo, "pnpm install && git push").await;
    let chained = waiting(&core, 1).await.remove(0);
    wait_for("the broker to ask", async || {
        let events = core.events(None, None, 500).await.unwrap();
        events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Decision { summary, detail, .. } if summary.starts_with("asking you") && detail["approval_id"] == chained.id.as_str()))
            .then_some(())
    })
    .await;
    assert!(core.approval_rule_offer(&chained.id).unwrap().is_empty());
    let e = core.add_approval_rule(&chained.id, ApprovalRuleKind::Exact).unwrap_err();
    assert!(e.to_string().contains("always asks"), "{e}");
    core.resolve_approval(&chained.id, deny()).await.unwrap();
    assert_eq!(answered(&answers, 3).await[2].1, ApprovalChoice::Deny);

    // Removed: Codly asks again.
    core.remove_approval_rule(&rule.id).unwrap();
    assert!(core.approval_rules().unwrap().is_empty());
    assert!(core.remove_approval_rule(&rule.id).is_err(), "removed once");
    ask(&core, &ana, "pnpm install").await;
    let again = waiting(&core, 1).await.remove(0);
    assert_eq!(offered(&core, &again).await.prefix.as_deref(), Some("pnpm install"));
    core.resolve_approval(&again.id, deny()).await.unwrap();
    assert_eq!(answered(&answers, 4).await[3], ("pnpm install".to_string(), ApprovalChoice::Deny));
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_request_from_two_agents_is_answered_together_and_the_rule_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let answers = Answers::default();
    let (core, ana, bo) = two_agents(dir.path(), &answers).await;

    ask(&core, &ana, "pnpm add lodash").await;
    ask(&core, &bo, "pnpm add lodash").await;
    let both = waiting(&core, 2).await;
    assert_ne!(both[0].agent_id, both[1].agent_id);
    for a in &both {
        assert_eq!(offered(&core, a).await.exact.as_deref(), Some("pnpm add lodash"));
    }
    // One answer, on one card: both agents get it.
    let rule = core.add_approval_rule(&both[0].id, ApprovalRuleKind::Exact).unwrap();
    let got = answered(&answers, 2).await;
    assert!(got.iter().all(|a| *a == ("pnpm add lodash".to_string(), ApprovalChoice::AllowOnce)), "{got:?}");
    assert!(core.snapshot().await.unwrap().approvals.is_empty());
    assert_eq!((rule.kind, rule.command.as_str()), (ApprovalRuleKind::Exact, "pnpm add lodash"));
    assert_eq!(core.approval_rules().unwrap(), vec![rule.clone()]);

    // The rule is in Codly's store, not in a worktree: it is there after the app stops.
    core.shutdown().await;
    let store = Store::open(&dir.path().join("data/codly.sqlite")).unwrap();
    assert_eq!(store.approval_rules(&rule.workspace_id).unwrap(), vec![rule]);
}
