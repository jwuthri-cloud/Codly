//! What the Knowledge room's Find, Files and Messages lenses need, as pure functions:
//!
//! - `exact_search`: literal (case-insensitive) text search across the shared board (blackboard
//!   notes), tasks and memories. The meaning-ranked search is the existing `codly_search` +
//!   `search_memories` + triples; this is the "I remember the exact words" one.
//! - `build_message_graph`: who talked to whom, built only from records Codly already keeps:
//!   chat messages (the Lead's requests, 1:1 chats, `codly_ask_lead` questions and the Lead's
//!   replies) and blackboard notes. Nothing new is recorded for it.
//! - the per-agent memory file (`<data>/memory/<agent id>.md`): a markdown file the user keeps
//!   for one agent, appended to that agent's brief at task start. Access is confined to that
//!   directory: the caller names an agent id, never a path.
//!
//! The act of a message (request / query / propose / agree_done / refuse / inform) is a cheap
//! keyword heuristic, like `derive_triples`: deterministic, no LLM, and shown as "inferred".

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::*;

// ───────────────────────────── exact search ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HitKind {
    /// A blackboard note (the team whiteboard).
    Board,
    Task,
    /// Any other memory: fact, decision, lesson, preference, summary.
    Memory,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExactHit {
    pub kind: HitKind,
    /// Task or memory id.
    pub id: Id,
    pub title: String,
    /// The matching field's text, trimmed around the match.
    pub snippet: String,
    /// Which field matched: "title", "goal", "text"…
    pub field: String,
    /// The match inside `snippet`, in characters.
    pub at: u32,
    pub len: u32,
    /// Author (memories) or assignee (tasks): an agent id, "user" or "codly".
    pub who: Option<String>,
    pub ts: i64,
}

/// Characters of context kept on each side of a match.
const SNIPPET_CONTEXT: usize = 48;

fn fold(s: &str) -> Vec<char> {
    // One char per char, so offsets in the folded text are offsets in the original.
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// Char offset of the first occurrence of `needle` (already folded) in `hay`.
fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// `text` cut to a window around the match, with "…" where it was trimmed.
/// Returns (snippet, match offset in snippet, match length) in chars.
fn snippet(text: &str, at: usize, len: usize) -> (String, u32, u32) {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_CONTEXT);
    let end = (at + len + SNIPPET_CONTEXT).min(chars.len());
    let mut out = String::new();
    let mut off = at - start;
    if start > 0 {
        out.push('…');
        off += 1;
    }
    out.extend(chars[start..end].iter().map(|c| if *c == '\n' { ' ' } else { *c }));
    if end < chars.len() {
        out.push('…');
    }
    (out, off as u32, len as u32)
}

/// Case-insensitive literal search over tasks (title, key, goal, acceptance, summary, status
/// reason) and memories (text; superseded ones are skipped). Title matches rank first, then
/// newest. A blank query matches nothing. At most `limit` hits.
pub fn exact_search(query: &str, tasks: &[Task], memories: &[Memory], limit: usize) -> Vec<ExactHit> {
    let needle = fold(query.trim());
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(bool, ExactHit)> = Vec::new();
    for t in tasks {
        let acceptance = t.acceptance.join("\n");
        let fields: [(&str, &str); 6] = [
            ("title", &t.title),
            ("key", &t.key),
            ("goal", &t.goal),
            ("acceptance", &acceptance),
            ("summary", t.summary.as_deref().unwrap_or("")),
            ("status", t.status_reason.as_deref().unwrap_or("")),
        ];
        if let Some((name, text, at)) = fields.iter().find_map(|(n, x)| find_chars(&fold(x), &needle).map(|a| (*n, *x, a))) {
            let (snippet, at, len) = snippet(text, at, needle.len());
            hits.push((
                name == "title" || name == "key",
                ExactHit { kind: HitKind::Task, id: t.id.clone(), title: format!("{} · {}", t.key, t.title), snippet, field: name.into(), at, len, who: t.assignee.clone(), ts: t.updated_at },
            ));
        }
    }
    for m in memories.iter().filter(|m| m.superseded_by.is_none()) {
        if let Some(at) = find_chars(&fold(&m.text), &needle) {
            let (snippet, at, len) = snippet(&m.text, at, needle.len());
            let kind = if m.kind == MemoryKind::Note { HitKind::Board } else { HitKind::Memory };
            let title: String = m.text.lines().next().unwrap_or("").chars().take(60).collect();
            hits.push((false, ExactHit { kind, id: m.id.clone(), title, snippet, field: "text".into(), at, len, who: Some(m.author.clone()), ts: m.created_at }));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.ts.cmp(&a.1.ts)).then_with(|| a.1.id.cmp(&b.1.id)));
    hits.into_iter().take(limit).map(|(_, h)| h).collect()
}

// ───────────────────────────── message graph ─────────────────────────────

/// What a message is for. Same vocabulary as Munder Difflin's message `act`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MsgAct {
    Request,
    Query,
    Propose,
    AgreeDone,
    Refuse,
    Inform,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GraphNode {
    /// An agent id, "user" or "board" (the team whiteboard).
    pub id: String,
    pub label: String,
    /// "agent" | "user" | "board"
    pub kind: String,
    /// Identity hue index (agents only).
    pub color: Option<i64>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub act: MsgAct,
    pub count: u32,
    pub last_ts: i64,
    /// The latest message of this kind between the two, clipped.
    pub sample: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Words of `text` (lowercased, apostrophes kept) joined by single spaces and padded, so a
/// phrase is found with `contains(" phrase ")` and never inside another word.
fn padded_words(text: &str) -> String {
    let t = text.to_lowercase().replace('’', "'");
    let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).collect();
    format!(" {} ", words.join(" "))
}

fn has_any(padded: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| padded.contains(&format!(" {p} ")))
}

/// First word of a message, to tell a question from a statement.
fn starts_with_any(padded: &str, words: &[&str]) -> bool {
    words.iter().any(|w| padded.starts_with(&format!(" {w} ")))
}

/// Cheap, deterministic act of a message. Precedence: refuse, propose, query, agree/done,
/// request; otherwise `default` (a user's or Lead's message is a request, anyone else's an inform).
pub fn classify_act(text: &str, default: MsgAct) -> MsgAct {
    let padded = padded_words(text);
    if has_any(&padded, &["can't", "cannot", "won't", "refuse", "refused", "decline", "declined", "unable", "not possible", "rejected", "no way"]) {
        return MsgAct::Refuse;
    }
    if has_any(&padded, &["i propose", "proposal", "i suggest", "suggest", "let's", "how about", "we could", "what if", "i'd recommend"]) {
        return MsgAct::Propose;
    }
    if text.trim_end().ends_with('?') || starts_with_any(&padded, &["who", "what", "where", "when", "why", "how", "which", "can", "could", "should", "is", "are", "does"]) {
        return MsgAct::Query;
    }
    if has_any(&padded, &["done", "completed", "finished", "merged", "agreed", "agree", "approved", "lgtm", "sounds good", "will do", "ok", "okay", "yes"]) {
        return MsgAct::AgreeDone;
    }
    if starts_with_any(&padded, &["please", "pls", "make", "add", "fix", "implement", "update", "run", "write", "create", "remove", "refactor", "check", "review", "build", "need", "must"]) {
        return MsgAct::Request;
    }
    default
}

fn clip(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!("{}…", one_line.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

/// Aggregates edges by (from, to, act) in a stable order.
#[derive(Default)]
struct Edges(BTreeMap<(String, String, MsgAct), (u32, i64, String)>);

impl Edges {
    fn add(&mut self, from: &str, to: &str, act: MsgAct, ts: i64, text: &str) {
        if from == to {
            return;
        }
        let e = self.0.entry((from.into(), to.into(), act)).or_insert((0, i64::MIN, String::new()));
        e.0 += 1;
        if ts >= e.1 {
            e.1 = ts;
            e.2 = clip(text, 140);
        }
    }
}

/// Nodes are the agents (plus "you" and the whiteboard when they took part); an edge is a kind of
/// message between two of them, with how many and the latest. Built from:
/// - chats in a Lead's thread: you → Lead (a request), a worker's `codly_ask_lead` → Lead
///   (a query), the Lead's answer → the asker (its `reply_to`), the Lead → you;
/// - chats in an `agent:<id>` thread: you → agent, agent → you, Lead → member (`codly_message_member`);
/// - blackboard notes: author → the assignees of the tasks that depend on the note's task, else
///   → the whiteboard.
/// Messages from "codly" itself (plan cards, status lines) are not agents talking and are skipped.
pub fn build_message_graph(agents: &[Agent], chats: &[ChatMessage], notes: &[Memory], tasks: &[Task]) -> MessageGraph {
    let known = |id: &str| agents.iter().any(|a| a.id == id);
    let lead_of = |thread: &str| {
        agents
            .iter()
            .find(|a| a.role == Role::Lead && a.thread.as_deref().unwrap_or("lead") == thread)
            .or_else(|| agents.iter().find(|a| a.role == Role::Lead))
    };
    let mut edges = Edges::default();
    let (mut uses_user, mut uses_board) = (false, false);

    for c in chats {
        if c.author == "codly" {
            continue;
        }
        // (from, to, forced act, default act)
        let route: Option<(String, String, Option<MsgAct>, MsgAct)> = if let Some(owner) = c.thread.strip_prefix("agent:") {
            if c.author == "user" {
                Some(("user".into(), owner.into(), None, MsgAct::Request))
            } else if c.author == owner {
                Some((owner.into(), "user".into(), None, MsgAct::Inform))
            } else {
                Some((c.author.clone(), owner.into(), None, MsgAct::Request))
            }
        } else if is_lead_thread(&c.thread) {
            let lead = lead_of(&c.thread).map(|l| l.id.clone());
            match lead {
                None => None,
                Some(lead) if c.author == "user" => Some(("user".into(), lead, None, MsgAct::Request)),
                Some(lead) if c.author == lead => {
                    let asker = c.attachment.as_ref().and_then(|a| a.get("reply_to")).and_then(|v| v.as_str());
                    match asker {
                        Some(asker) => Some((lead, asker.into(), None, MsgAct::Inform)),
                        None => Some((lead, "user".into(), None, MsgAct::Inform)),
                    }
                }
                // A worker speaking in a Lead's thread is codly_ask_lead.
                Some(lead) => Some((c.author.clone(), lead, Some(MsgAct::Query), MsgAct::Query)),
            }
        } else {
            None
        };
        let Some((from, to, forced, default)) = route else { continue };
        let ok = |id: &str| id == "user" || known(id);
        if !ok(&from) || !ok(&to) {
            continue;
        }
        uses_user |= from == "user" || to == "user";
        // The Lead speaking to you in plain prose is an inform unless it reads as something else.
        let act = forced.unwrap_or_else(|| classify_act(&c.text, default));
        edges.add(&from, &to, act, c.ts, &c.text);
    }

    for n in notes.iter().filter(|m| m.kind == MemoryKind::Note && m.superseded_by.is_none()) {
        if !known(&n.author) {
            continue;
        }
        let dependents: Vec<&str> = match (n.scope, n.scope_id.as_deref()) {
            (MemoryScope::Task, Some(task)) => tasks
                .iter()
                .filter(|t| t.depends_on.iter().any(|d| d == task))
                .filter_map(|t| t.assignee.as_deref())
                .filter(|a| *a != n.author && known(a))
                .collect(),
            _ => vec![],
        };
        if dependents.is_empty() {
            uses_board = true;
            edges.add(&n.author, "board", MsgAct::Inform, n.created_at, &n.text);
        }
        for d in dependents {
            edges.add(&n.author, d, MsgAct::Inform, n.created_at, &n.text);
        }
    }

    let mut nodes: Vec<GraphNode> = agents
        .iter()
        .map(|a| GraphNode { id: a.id.clone(), label: a.name.clone(), kind: "agent".into(), color: Some(a.color), title: a.title.clone() })
        .collect();
    if uses_user {
        nodes.push(GraphNode { id: "user".into(), label: "You".into(), kind: "user".into(), color: None, title: None });
    }
    if uses_board {
        nodes.push(GraphNode { id: "board".into(), label: "Whiteboard".into(), kind: "board".into(), color: None, title: None });
    }
    let edges = edges.0.into_iter().map(|((from, to, act), (count, last_ts, sample))| GraphEdge { from, to, act, count, last_ts, sample }).collect();
    MessageGraph { nodes, edges }
}

// ───────────────────────────── per-agent memory file ─────────────────────────────

/// Largest memory file the editor will save (it is read into every brief, clipped).
pub const MAX_MEMORY_FILE: usize = 64 * 1024;
/// Characters of the file that go into a brief.
const BRIEF_BUDGET: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryFile {
    pub agent_id: Id,
    /// Where it lives, relative to Codly's data folder (display only).
    pub path: String,
    pub text: String,
    /// False: nothing saved yet, `text` is the starter template.
    pub exists: bool,
    pub modified_at: Option<i64>,
}

/// `<dir>/<agent id>.md`. The id must be a plain token (letters, digits, `-`, `_`), so no
/// separator or `..` can ever reach the path; a symlink at the target is refused.
pub fn memory_file_path(dir: &Path, agent_id: &str) -> anyhow::Result<PathBuf> {
    if agent_id.is_empty() || agent_id.len() > 64 || !agent_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("not an agent id: {agent_id:?}");
    }
    let path = dir.join(format!("{agent_id}.md"));
    if let Ok(meta) = fs::symlink_metadata(&path)
        && meta.file_type().is_symlink()
    {
        bail!("the memory file is a symlink; refusing to follow it");
    }
    Ok(path)
}

fn template(agent_name: &str) -> String {
    format!("# {agent_name}: memory\n\nWhat {agent_name} should always know. Codly adds this file to {agent_name}'s brief at the start of every task.\n\n- \n")
}

pub fn read_memory_file(dir: &Path, agent_id: &str, agent_name: &str) -> anyhow::Result<MemoryFile> {
    let path = memory_file_path(dir, agent_id)?;
    let shown = format!("memory/{agent_id}.md");
    match fs::read_to_string(&path) {
        Ok(text) => {
            let modified_at = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
            Ok(MemoryFile { agent_id: agent_id.into(), path: shown, text, exists: true, modified_at })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(MemoryFile { agent_id: agent_id.into(), path: shown, text: template(agent_name), exists: false, modified_at: None }),
        Err(e) => Err(e).with_context(|| format!("reading {shown}")),
    }
}

/// Atomic write (temp file in the same folder, then rename).
pub fn write_memory_file(dir: &Path, agent_id: &str, agent_name: &str, text: &str) -> anyhow::Result<MemoryFile> {
    if text.len() > MAX_MEMORY_FILE {
        bail!("a memory file is at most {} KB", MAX_MEMORY_FILE / 1024);
    }
    let path = memory_file_path(dir, agent_id)?;
    fs::create_dir_all(dir).context("creating the memory folder")?;
    let tmp = dir.join(format!(".{agent_id}.md.tmp"));
    fs::write(&tmp, text).context("writing the memory file")?;
    fs::rename(&tmp, &path).context("saving the memory file")?;
    read_memory_file(dir, agent_id, agent_name)
}

/// The agent's memory file appended to its memory pack. A missing, unreadable or still-template
/// file adds nothing.
pub fn with_memory_file(pack: Option<String>, dir: &Path, agent_id: &str) -> Option<String> {
    let Ok(f) = read_memory_file(dir, agent_id, "") else { return pack };
    let body: String = f.text.trim().chars().take(BRIEF_BUDGET).collect();
    if !f.exists || body.is_empty() {
        return pack;
    }
    let section = format!("Your memory file (kept by you and the user):\n{body}");
    Some(match pack {
        Some(p) if !p.trim().is_empty() => format!("{p}\n\n{section}"),
        _ => section,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, key: &str, title: &str, goal: &str) -> Task {
        Task {
            id: id.into(),
            plan_id: None,
            key: key.into(),
            title: title.into(),
            goal: goal.into(),
            acceptance: vec![],
            write_scope: vec![],
            verify: vec![],
            depends_on: vec![],
            status: TaskStatus::Pending,
            assignee: None,
            agent_hint: None,
            account_hint: None,
            consultant: None,
            fresh_session: false,
            branch: None,
            worktree: None,
            attempt: 0,
            status_reason: None,
            summary: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn mem(id: &str, kind: MemoryKind, text: &str, author: &str, ts: i64) -> Memory {
        Memory {
            id: id.into(),
            workspace_id: None,
            scope: MemoryScope::Project,
            scope_id: None,
            kind,
            text: text.into(),
            author: author.into(),
            source_event: None,
            importance: 0.5,
            pinned: false,
            superseded_by: None,
            created_at: ts,
            accessed_at: ts,
        }
    }

    fn agent(id: &str, name: &str, role: Role, thread: Option<&str>) -> Agent {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": name, "provider": "claude", "model": null, "role": role, "status": "idle",
            "activity": null, "activity_label": null, "task_id": null, "run_id": null, "session_id": null,
            "worktree": null, "billing": "subscription", "color": 1, "look_seed": id, "thread": thread,
            "usage": Usage::default(),
            "created_at": 1, "updated_at": 1
        }))
        .unwrap_or_else(|e| panic!("agent fixture: {e}"))
    }

    fn chat(thread: &str, author: &str, text: &str, ts: i64) -> ChatMessage {
        ChatMessage { id: format!("c{ts}"), thread: thread.into(), author: author.into(), text: text.into(), attachment: None, ts }
    }

    #[test]
    fn exact_search_is_literal_case_insensitive_across_kinds() {
        let mut t = task("t1", "t1", "Dark mode toggle", "Add a theme switch to settings");
        t.acceptance = vec!["Persists in localStorage".into()];
        let tasks = [t, task("t2", "t2", "Billing", "Invoices")];
        let mems = [
            mem("m1", MemoryKind::Note, "Settings API: theme is system | light | dark", "bo", 10),
            mem("m2", MemoryKind::Lesson, "Await settings writes before resolving UI state", "cy", 20),
            mem("m3", MemoryKind::Fact, "Unrelated", "codly", 30),
        ];
        let hits = exact_search("SETTINGS", &tasks, &mems, 10);
        // t1's goal ("settings"), the note and the lesson; title matches would come first.
        assert_eq!(hits.len(), 3);
        assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Board).count(), 1);
        assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Memory).count(), 1);
        let h = hits.iter().find(|h| h.kind == HitKind::Task).unwrap();
        assert_eq!(h.field, "goal");
        let chars: Vec<char> = h.snippet.chars().collect();
        let m: String = chars[h.at as usize..(h.at + h.len) as usize].iter().collect();
        assert_eq!(m.to_lowercase(), "settings");
        // Literal phrase, not words: reordered words do not match.
        assert!(exact_search("mode dark", &tasks, &mems, 10).is_empty());
        assert_eq!(exact_search("dark mode", &tasks, &mems, 10)[0].id, "t1");
        assert!(exact_search("   ", &tasks, &mems, 10).is_empty());
        assert_eq!(exact_search("settings", &tasks, &mems, 1).len(), 1);
    }

    #[test]
    fn exact_search_ranks_titles_first_and_skips_superseded() {
        let tasks = [task("t1", "t1", "Cache layer", "Use redis"), task("t2", "t2", "Docs", "Explain the cache")];
        let mut old = mem("m1", MemoryKind::Fact, "cache is redis", "user", 5);
        old.superseded_by = Some("m2".into());
        let hits = exact_search("cache", &tasks, &[old], 10);
        assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
        assert_eq!(hits[0].field, "title");
    }

    #[test]
    fn snippets_trim_around_the_match_with_ellipses() {
        let long = format!("{} needle {}", "a".repeat(100), "b".repeat(100));
        let hits = exact_search("needle", &[], &[mem("m", MemoryKind::Fact, &long, "user", 1)], 5);
        let h = &hits[0];
        assert!(h.snippet.starts_with('…') && h.snippet.ends_with('…'));
        let c: Vec<char> = h.snippet.chars().collect();
        assert_eq!(c[h.at as usize..(h.at + h.len) as usize].iter().collect::<String>(), "needle");
    }

    #[test]
    fn acts_by_keyword() {
        use MsgAct::*;
        assert_eq!(classify_act("I can't touch that file, it is out of scope", Inform), Refuse);
        assert_eq!(classify_act("I propose we split this in two tasks", Inform), Propose);
        assert_eq!(classify_act("Question from Ada (t1): which theme key do we use?", Inform), Query);
        assert_eq!(classify_act("Done. Merged into integration.", Inform), AgreeDone);
        assert_eq!(classify_act("Please add a settings route", Inform), Request);
        assert_eq!(classify_act("The build takes about two minutes", Inform), Inform);
        assert_eq!(classify_act("The build takes about two minutes", Request), Request);
        // Words are whole words: "abandoned" is not "done".
        assert_eq!(classify_act("The branch was abandoned", Inform), Inform);
    }

    #[test]
    fn graph_from_chats_and_notes() {
        let agents = [
            agent("lead", "Lead", Role::Lead, Some("lead")),
            agent("ada", "Ada", Role::Worker, None),
            agent("bo", "Bo", Role::Worker, None),
        ];
        let mut reply = chat("lead", "lead", "Use the `data-theme` attribute.", 4);
        reply.attachment = Some(serde_json::json!({ "reply_to": "ada" }));
        let chats = [
            chat("lead", "user", "Add a dark mode toggle", 1),
            chat("lead", "codly", "Plan ready (2 tasks).", 2),
            chat("lead", "ada", "Question from Ada (t1): which theme key do we use?", 3),
            reply,
            chat("lead", "ada", "Another question: where do tokens live?", 5),
            chat("agent:bo", "lead", "Heads up: ada owns the theme provider", 6),
            chat("agent:bo", "bo", "Thanks, understood. Done with the API.", 7),
            chat("agent:ghost", "user", "hi", 8),
        ];
        let mut t1 = task("t1", "t1", "Theme", "x");
        t1.assignee = Some("ada".into());
        let mut t2 = task("t2", "t2", "Settings API", "y");
        t2.assignee = Some("bo".into());
        t2.depends_on = vec!["t1".into()];
        let mut note = mem("n1", MemoryKind::Note, "Theme tokens key on [data-theme]", "ada", 9);
        note.scope = MemoryScope::Task;
        note.scope_id = Some("t1".into());
        let loose = mem("n2", MemoryKind::Note, "Heads up: flaky e2e", "bo", 10);
        let g = build_message_graph(&agents, &chats, &[note, loose, mem("f", MemoryKind::Fact, "not a note", "bo", 11)], &[t1, t2]);

        let edge = |from: &str, to: &str, act: MsgAct| g.edges.iter().find(|e| e.from == from && e.to == to && e.act == act);
        assert_eq!(edge("user", "lead", MsgAct::Request).unwrap().count, 1);
        assert_eq!(edge("ada", "lead", MsgAct::Query).unwrap().count, 2);
        assert_eq!(edge("ada", "lead", MsgAct::Query).unwrap().sample, "Another question: where do tokens live?");
        assert!(edge("lead", "ada", MsgAct::Inform).is_some(), "the Lead's reply goes to the asker");
        assert!(edge("lead", "bo", MsgAct::Inform).is_some() || edge("lead", "bo", MsgAct::Request).is_some());
        assert_eq!(edge("bo", "user", MsgAct::AgreeDone).unwrap().count, 1);
        // Note on t1 goes to the assignee of t1's dependent; the loose note to the whiteboard.
        assert!(edge("ada", "bo", MsgAct::Inform).is_some());
        assert!(edge("bo", "board", MsgAct::Inform).is_some());
        // "codly" lines and unknown agents are not in the picture; a fact is not a note.
        assert!(g.edges.iter().all(|e| e.from != "codly" && e.to != "ghost" && e.from != "ghost"));
        assert_eq!(g.edges.iter().filter(|e| e.to == "board").count(), 1);
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["lead", "ada", "bo", "user", "board"]);
        // Deterministic.
        let again = build_message_graph(&agents, &chats, &[], &[]);
        assert_eq!(again.edges.len(), build_message_graph(&agents, &chats, &[], &[]).edges.len());
    }

    #[test]
    fn graph_without_traffic_lists_the_agents_only() {
        let agents = [agent("a", "Ada", Role::Worker, None)];
        let g = build_message_graph(&agents, &[], &[], &[]);
        assert_eq!(g.nodes.len(), 1);
        assert!(g.edges.is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("codly-memfile-{name}-{}", new_id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn memory_file_paths_cannot_leave_the_folder() {
        let dir = scratch("paths");
        for bad in ["", "..", "../etc/passwd", "a/b", "a\\b", "x.md", ".hidden", "a b", &"z".repeat(65)] {
            assert!(memory_file_path(&dir, bad).is_err(), "{bad:?} must be refused");
        }
        let p = memory_file_path(&dir, "019a-bc_D").unwrap();
        assert_eq!(p.parent().unwrap(), dir);
        assert!(write_memory_file(&dir, "../x", "X", "hi").is_err());
        assert!(!dir.parent().unwrap().join("x.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn memory_file_refuses_symlinks() {
        let dir = scratch("link");
        let outside = scratch("outside").join("secret.md");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("evil.md")).unwrap();
        assert!(read_memory_file(&dir, "evil", "Evil").is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
    }

    #[test]
    fn memory_file_round_trip_and_brief() {
        let dir = scratch("rt");
        let first = read_memory_file(&dir, "ada", "Ada").unwrap();
        assert!(!first.exists && first.text.contains("Ada"));
        // A template that was never saved adds nothing to a brief.
        assert_eq!(with_memory_file(Some("pack".into()), &dir, "ada").as_deref(), Some("pack"));
        let saved = write_memory_file(&dir, "ada", "Ada", "# Ada\n\n- prefers pnpm\n").unwrap();
        assert!(saved.exists && saved.modified_at.is_some());
        assert_eq!(read_memory_file(&dir, "ada", "Ada").unwrap().text, "# Ada\n\n- prefers pnpm\n");
        let brief = with_memory_file(Some("pack".into()), &dir, "ada").unwrap();
        assert!(brief.starts_with("pack\n\n") && brief.contains("prefers pnpm"));
        assert!(with_memory_file(None, &dir, "ada").unwrap().contains("prefers pnpm"));
        assert!(write_memory_file(&dir, "ada", "Ada", &"x".repeat(MAX_MEMORY_FILE + 1)).is_err());
        // No temp files left behind.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }
}
