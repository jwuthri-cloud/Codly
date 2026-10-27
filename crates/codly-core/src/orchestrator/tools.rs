//! The MCP `ToolHandler`: what every `codly_*` tool does (specs in mcp/tools.rs). The
//! server has already resolved the caller's token and checked the tool against its role.
//! Results are compact JSON; text another agent wrote carries `"untrusted": true`.

use std::sync::{Arc, Weak};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{Inner, guard, prompts};
use crate::mcp::{ToolCtx, ToolHandler, tools as t};
use crate::memory::{self, MemoryView};
use crate::model::*;

const MAX_MEMORY: usize = 1000;

pub(super) fn handler(inner: Weak<Inner>) -> ToolHandler {
    Arc::new(move |ctx, tool, args| {
        let inner = inner.clone();
        Box::pin(async move {
            let inner = inner.upgrade().ok_or_else(|| "Codly is shutting down".to_string())?;
            inner.call_tool(ctx, &tool, args).await.map_err(|e| e.to_string())
        })
    })
}

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

impl Inner {
    async fn call_tool(self: &Arc<Self>, ctx: ToolCtx, tool: &str, args: Value) -> Result<Value> {
        let ws = self.store.agent_workspace(&ctx.agent_id)?.or_else(|| self.workspace().map(|w| w.id));
        let ws = ws.as_deref();
        match tool {
            t::GET_TASK => self.tool_get_task(&ctx),
            t::BOARD => self.tool_board(&ctx, ws),
            t::SEARCH => {
                let q = s(&args, "query").ok_or_else(|| anyhow!("query is required"))?;
                let k = args.get("k").and_then(Value::as_u64).unwrap_or(8).clamp(1, 20) as usize;
                Ok(serde_json::to_value(self.search(q, k).await?)?)
            }
            t::MEMORY_SEARCH => self.tool_memory_search(&ctx, ws, &args),
            t::MEMORY_WRITE => self.tool_memory_write(&ctx, ws, &args),
            t::KG_QUERY => {
                let triples = self
                    .store
                    .with(|db| guard("knowledge graph", || memory::query_triples(db, ws, s(&args, "entity"), s(&args, "predicate"), 50)))?;
                Ok(json!({ "triples": triples.iter().map(|t| json!({
                    "subject": t.subject, "predicate": t.predicate, "object": t.object, "source": t.source, "since": t.valid_from,
                })).collect::<Vec<_>>() }))
            }
            t::KG_ADD => {
                let field = |k: &str| -> Result<String> {
                    let v = s(&args, k).ok_or_else(|| anyhow!("{k} is required"))?;
                    if v.len() > 200 {
                        bail!("{k} is too long (max 200 chars)");
                    }
                    Ok(v.to_string())
                };
                let triple = Triple {
                    id: String::new(),
                    workspace_id: ws.map(String::from),
                    subject: field("subject")?,
                    predicate: field("predicate")?,
                    object: field("object")?,
                    source: "agent".into(),
                    source_event: None,
                    confidence: 0.8,
                    valid_from: 0,
                    valid_to: None,
                };
                let added = self.add_triple(ws, triple, Some(&ctx.agent_id))?;
                Ok(json!({ "added": added.is_some(), "note": if added.is_some() { "recorded" } else { "already known" } }))
            }
            t::POST_NOTE => {
                let text = s(&args, "text").ok_or_else(|| anyhow!("text is required"))?;
                if text.chars().count() > MAX_MEMORY {
                    bail!("a note is at most {MAX_MEMORY} characters; keep it to the essentials");
                }
                let (scope, scope_id) = match ctx.task_id.as_deref() {
                    Some(task) => (MemoryScope::Task, Some(task)),
                    None => (MemoryScope::Project, None),
                };
                let m = self.add_memory(ws, scope, scope_id, MemoryKind::Note, text, &ctx.agent_id, None, 0.5)?;
                Ok(json!({ "id": m.id, "visible_to": "the lead, the board, and every task that depends on yours" }))
            }
            t::SUBMIT_PLAN => self.submit_plan(&ctx.agent_id, args).await,
            t::SUBMIT_REPORT => {
                let status = s(&args, "status").ok_or_else(|| anyhow!("status is required (done, blocked or failed)"))?;
                if !matches!(status, "done" | "blocked" | "failed") {
                    bail!("status must be done, blocked or failed");
                }
                if s(&args, "summary").is_none() {
                    bail!("summary is required: what you changed and why, 1-5 sentences");
                }
                self.store.set_run_report(&ctx.run_id, &args)?;
                Ok(
                    json!({ "ok": true, "next": "Codly commits your worktree and runs the verify commands when your turn ends. End your turn now with a short summary." }),
                )
            }
            t::ASK_LEAD => self.tool_ask_lead(&ctx, ws, &args),
            t::ASK_USER => self.tool_ask_user(&ctx, ws, &args),
            t::MESSAGE_MEMBER => {
                let to = s(&args, "to").ok_or_else(|| anyhow!("to is required: a team member's name"))?;
                let text = s(&args, "text").ok_or_else(|| anyhow!("text is required"))?;
                self.message_member(&ctx.agent_id, to, text).await
            }
            t::QUICK_TASK => self.quick_task(&ctx.agent_id, args).await,
            t::REASSIGN => self.reassign(&ctx.agent_id, &args),
            t::SUBMIT_REVIEW => {
                let verdict = s(&args, "verdict").ok_or_else(|| anyhow!("verdict is required"))?;
                if !matches!(verdict, "approve" | "request_changes" | "reject") {
                    bail!("verdict must be approve, request_changes or reject");
                }
                self.store.set_run_report(&ctx.run_id, &args)?;
                Ok(json!({ "ok": true, "next": "Thanks. End your turn now." }))
            }
            other => bail!("unknown tool {other}"),
        }
    }

    fn tool_get_task(&self, ctx: &ToolCtx) -> Result<Value> {
        let Some(task_id) = ctx.task_id.as_deref() else {
            return Ok(json!({ "task": null, "note": "You have no assigned task. Ask the user what to do, or read codly_board." }));
        };
        let task = self.task(task_id)?;
        let ws = self.store.task_workspace(task_id)?;
        let agents = ws.as_deref().map(|w| self.store.agents(w)).transpose()?.unwrap_or_default();
        let name = |id: &Option<Id>| id.as_deref().and_then(|i| agents.iter().find(|a| a.id == i)).map(|a| a.name.clone());
        let deps: Vec<Value> = task
            .depends_on
            .iter()
            .filter_map(|d| self.store.task(d).ok().flatten())
            .map(|d| {
                json!({ "key": d.key, "title": d.title, "status": d.status, "by": name(&d.assignee),
                        "summary": d.summary, "untrusted": d.summary.is_some() })
            })
            .collect();
        let notes: Vec<Value> = self
            .store
            .memory_events(ws.as_deref(), 200)?
            .into_iter()
            .filter(|m| m.kind == MemoryKind::Note && m.scope_id.as_ref().is_some_and(|s| task.depends_on.contains(s) || s == &task.id))
            .take(10)
            .map(|m| json!({ "from": name(&Some(m.author.clone())).unwrap_or(m.author), "text": m.text, "untrusted": true }))
            .collect();
        Ok(json!({
            "key": task.key, "title": task.title, "goal": task.goal, "status": task.status,
            "acceptance_criteria": task.acceptance, "write_scope": task.write_scope, "verify": task.verify,
            "branch": task.branch, "worktree": task.worktree, "attempt": task.attempt, "status_reason": task.status_reason,
            "depends_on": deps, "notes": notes,
            "rules": "Do not commit; Codly commits and merges. Call codly_submit_report once when done.",
        }))
    }

    fn tool_board(&self, ctx: &ToolCtx, ws: Option<&str>) -> Result<Value> {
        let Some(ws) = ws else { return Ok(json!({ "plans": [], "tasks": [], "agents": [] })) };
        // Every live plan (parallel requests), then the three latest finished ones.
        let all: Vec<Plan> = self.store.plans(ws, 50)?;
        let live = |p: &&Plan| matches!(p.status, PlanStatus::AwaitingApproval | PlanStatus::Running | PlanStatus::Integrating);
        let plans: Vec<Plan> = all.iter().filter(live).chain(all.iter().filter(|p| !live(p)).take(3)).cloned().collect();
        // A Lead sees which plans are its own request's.
        let mine = (ctx.role == Role::Lead).then(|| self.agent(&ctx.agent_id).ok()).flatten().map(|a| super::lead::thread_of(&a).to_string());
        let tasks = self.store.tasks(ws)?;
        let agents = self.store.agents(ws)?;
        let name = |id: &Option<Id>| id.as_deref().and_then(|i| agents.iter().find(|a| a.id == i)).map(|a| a.name.clone());
        let shown: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.plan_id.as_ref().is_none_or(|p| plans.iter().any(|x| &x.id == p)))
            .filter(|t| t.plan_id.is_some() || !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled))
            .collect();
        let mut out = json!({
            "plans": plans.iter().map(|p| {
                let mut v = json!({ "id": p.id, "summary": p.summary, "status": p.status, "integration_branch": p.integration_branch, "request_thread": p.thread });
                if let Some(t) = &mine {
                    v["yours"] = json!(&p.thread == t);
                }
                v
            }).collect::<Vec<_>>(),
            "tasks": shown.iter().map(|t| json!({
                "key": t.key, "title": t.title, "status": t.status, "owner": name(&t.assignee), "reason": t.status_reason,
                "plan_id": t.plan_id, "write_scope": t.write_scope,
            })).collect::<Vec<_>>(),
            "agents": agents.iter().map(|a| json!({
                "name": a.name, "provider": a.provider, "role": a.role, "status": a.status, "doing": a.activity_label,
                "task": a.task_id.as_deref().and_then(|t| tasks.iter().find(|x| x.id == t)).map(|t| t.key.clone()),
            })).collect::<Vec<_>>(),
        });
        if ctx.role == Role::Lead {
            out["team"] = json!(self.team_view(ws, &agents, &tasks));
            out["consultant_options"] = json!(self.consultant_options());
        }
        Ok(out)
    }

    fn tool_memory_search(&self, ctx: &ToolCtx, ws: Option<&str>, args: &Value) -> Result<Value> {
        let q = s(args, "query").ok_or_else(|| anyhow!("query is required"))?;
        let mut task_ids: Vec<String> = vec![];
        if let Some(tid) = ctx.task_id.as_deref() {
            task_ids.push(tid.to_string());
            if let Ok(Some(t)) = self.store.task(tid) {
                task_ids.extend(t.depends_on);
            }
        }
        let view = MemoryView { workspace_id: ws, agent_id: Some(&ctx.agent_id), task_ids: &task_ids };
        let scope = s(args, "scope").unwrap_or("all");
        let filter = (scope == "private").then_some(MemoryScope::Agent);
        let mut found = self.store.with(|db| guard("memory", || memory::search_memories(db, &view, q, filter, 12)))?;
        if scope == "shared" {
            found.retain(|m| m.scope != MemoryScope::Agent);
        }
        let agents = ws.map(|w| self.store.agents(w)).transpose()?.unwrap_or_default();
        Ok(json!({ "memories": found.iter().map(|m| {
            let by = match m.author.as_str() {
                "user" | "codly" => m.author.clone(),
                id => agents.iter().find(|a| a.id == id).map_or_else(|| "agent".into(), |a| a.name.clone()),
            };
            json!({ "id": m.id, "kind": m.kind, "scope": m.scope, "text": m.text, "by": by,
                    "untrusted": m.author != "user" && m.author != "codly" && m.author != ctx.agent_id })
        }).collect::<Vec<_>>() }))
    }

    fn tool_memory_write(&self, ctx: &ToolCtx, ws: Option<&str>, args: &Value) -> Result<Value> {
        let text = s(args, "text").ok_or_else(|| anyhow!("text is required"))?;
        if text.chars().count() > MAX_MEMORY {
            bail!("one fact per call, one or two sentences (max {MAX_MEMORY} characters)");
        }
        let kind = match s(args, "kind").unwrap_or("fact") {
            "fact" => MemoryKind::Fact,
            "decision" => MemoryKind::Decision,
            "lesson" => MemoryKind::Lesson,
            "preference" => MemoryKind::Preference,
            k => bail!("unknown kind {k}: use fact, decision, lesson or preference"),
        };
        let shared = args.get("shared").and_then(Value::as_bool).unwrap_or(true);
        let (scope, scope_id) = if shared { (MemoryScope::Project, None) } else { (MemoryScope::Agent, Some(ctx.agent_id.as_str())) };
        let m = self.add_memory(ws, scope, scope_id, kind, text, &ctx.agent_id, None, 0.5)?;
        Ok(json!({ "id": m.id, "scope": if shared { "shared with the team" } else { "private to you" } }))
    }

    /// The question goes to the Lead of the request the worker's task came from (its plan's
    /// thread); the reply is forwarded to the worker.
    fn tool_ask_lead(self: &Arc<Self>, ctx: &ToolCtx, ws: Option<&str>, args: &Value) -> Result<Value> {
        let question = s(args, "question").ok_or_else(|| anyhow!("question is required"))?.to_string();
        let ws_id = ws.ok_or_else(|| anyhow!("no workspace"))?.to_string();
        let worker = self.agent(&ctx.agent_id)?;
        let task = ctx.task_id.as_deref().and_then(|t| self.store.task(t).ok().flatten());
        let thread = task.as_ref().map_or_else(default_lead_thread, |t| self.task_thread(t));
        let key = task.map_or_else(|| "no task".into(), |t| t.key);
        self.chat(
            Some(&ws_id),
            &thread,
            &ctx.agent_id,
            &format!("Question from {} ({key}): {}", worker.name, prompts::clip(&question, 2000)),
            None,
        )?;
        let me = self.clone();
        let agent_id = ctx.agent_id.clone();
        tokio::spawn(async move {
            let Ok(Some(ws)) = me.store.workspace(&ws_id) else { return };
            let body = format!("\n{}", prompts::lead_question(&worker.name, &key, &question));
            // A failure was already reported to the worker.
            let _ = me.send_to_lead(&ws, &thread, body, Some(agent_id)).await;
        });
        Ok(json!({ "ok": true, "note": "Sent to the lead. The answer arrives as a later message; continue meanwhile if you can." }))
    }
}
