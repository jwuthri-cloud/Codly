//! Plan validation and normalization (agents-backend §6.4). The Lead proposes a task DAG
//! through `codly_submit_plan`; Codly checks it deterministically, rejects what only the Lead
//! can fix (every problem at once, phrased so it can act on it) and repairs what is
//! mechanical, returning each repair as a `Fix` the caller records as a `plan_validate`
//! decision. Pure: no I/O.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::scope;
use crate::model::{ConsultantSpec, Provider, ROLE_TITLES, role_title};

pub const MAX_TASKS: usize = 8;

/// Dependency manifests and lockfiles: whoever owns one goes first (agents-backend §4.4).
const HOT_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "deno.json",
    "Cargo.toml",
    "Cargo.lock",
    "go.mod",
    "go.sum",
    "pyproject.toml",
    "poetry.lock",
    "uv.lock",
    "requirements.txt",
    "Pipfile",
    "Pipfile.lock",
    "Gemfile",
    "Gemfile.lock",
    "composer.json",
    "composer.lock",
    "pubspec.yaml",
    "pubspec.lock",
    "Package.swift",
    "Package.resolved",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "pom.xml",
    "mix.exs",
    "mix.lock",
];

/// `codly_submit_plan` arguments (see mcp/tools.rs for the schema the Lead sees).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanInput {
    pub summary: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub integration_checks: Vec<String>,
    pub tasks: Vec<TaskInput>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskInput {
    pub id: String,
    pub title: String,
    pub goal: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub verify: Vec<String>,
    /// "S" | "M" | "L"
    #[serde(default)]
    pub size: Option<String>,
    /// "low" | "medium" | "high"
    #[serde(default)]
    pub risk: Option<String>,
    /// "any" or a provider name.
    #[serde(default)]
    pub agent_hint: Option<String>,
    /// An account id from the board's Accounts line: pins the task to that sign-in.
    /// Checked against the registry when the plan is stored (plans.rs).
    #[serde(default)]
    pub account: Option<String>,
    /// A worker team member's name, or "consultant". Checked against the team when the plan
    /// is stored (plans.rs).
    #[serde(default)]
    pub assignee: Option<String>,
    /// The consultant to hire: its job title, and optionally its harness and model.
    #[serde(default)]
    pub consultant: Option<ConsultantInput>,
    /// Team members only: "continue" (default) or "fresh".
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsultantInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl TaskInput {
    pub fn hint(&self) -> Option<Provider> {
        self.agent_hint.as_deref().and_then(parse_provider)
    }

    /// The named team member, if the task is assigned to one (not "consultant").
    pub fn member(&self) -> Option<&str> {
        self.assignee.as_deref().filter(|a| !a.eq_ignore_ascii_case(CONSULTANT))
    }

    pub fn consultant_spec(&self) -> Option<ConsultantSpec> {
        let c = self.consultant.as_ref()?;
        Some(ConsultantSpec {
            provider: c.provider.as_deref().and_then(parse_provider),
            model: c.model.clone(),
            title: c.title.as_deref().and_then(role_title).map(String::from),
        })
    }

    pub fn fresh_session(&self) -> bool {
        self.session.as_deref() == Some("fresh")
    }
}

/// The `assignee` value that asks for a consultant.
pub const CONSULTANT: &str = "consultant";

/// Model names travel to CLI flags: one token, no spaces.
pub fn valid_model(m: &str) -> bool {
    !m.is_empty() && m.len() <= 64 && !m.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Normalize a task's staffing fields; errors and fixes are pushed for the caller.
pub fn check_staffing(t: &mut TaskInput, errors: &mut Vec<String>, fixes: &mut Vec<Fix>) {
    let id = t.id.clone();
    t.assignee = t.assignee.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(|a| {
        if a.eq_ignore_ascii_case(CONSULTANT) { CONSULTANT.to_string() } else { a.to_string() }
    });
    if let Some(c) = &mut t.consultant {
        c.model = c.model.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(String::from);
        c.provider = c.provider.as_deref().map(str::trim).filter(|p| !p.is_empty() && *p != "any").map(String::from);
        match c.provider.as_deref().map(|p| (p, parse_provider(p))) {
            Some((p, None)) => errors.push(format!("task {id}: consultant.provider '{p}' is not one of claude, codex, gemini, opencode")),
            Some((_, Some(p))) => c.provider = Some(p.as_str().into()),
            None => {}
        }
        if let Some(m) = c.model.as_deref().filter(|m| !valid_model(m)) {
            errors.push(format!("task {id}: consultant.model '{m}' must be one word (e.g. sonnet, opus:high)"));
        }
        if c.model.is_some() && c.provider.is_none() {
            errors.push(format!("task {id}: consultant.model needs consultant.provider (a model belongs to one harness)"));
        }
        match c.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            None => c.title = None,
            Some(t) => match role_title(t) {
                Some(canon) => c.title = Some(canon.into()),
                None => errors.push(format!("task {id}: consultant.title '{t}' is not one of: {}", ROLE_TITLES.join(", "))),
            },
        }
        if t.assignee.is_none() {
            t.assignee = Some(CONSULTANT.into());
        }
    }
    if let Some(m) = t.member()
        && t.consultant.is_some()
    {
        errors.push(format!("task {id}: assigned to {m} and to a consultant; pick one"));
    }
    match t.session.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        None => {}
        Some(s) if s.is_empty() || s == "continue" => t.session = None,
        Some(s) if s == "fresh" => t.session = Some(s),
        Some(s) => {
            fixes.push(Fix {
                summary: format!("{id}: ignored unknown session '{s}' (continue or fresh)"),
                detail: json!({ "rule": "session", "task": id, "session": s }),
            });
            t.session = None;
        }
    }
}

/// A mechanical repair Codly made to the plan, recorded as a decision.
#[derive(Debug, Clone)]
pub struct Fix {
    pub summary: String,
    pub detail: Value,
}

#[derive(Debug, Clone)]
pub struct ValidPlan {
    pub plan: PlanInput,
    pub fixes: Vec<Fix>,
}

pub fn parse_provider(s: &str) -> Option<Provider> {
    match s.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claude_code" => Some(Provider::Claude),
        "codex" => Some(Provider::Codex),
        "gemini" | "gemini-cli" => Some(Provider::Gemini),
        "opencode" => Some(Provider::Opencode),
        "cursor" => Some(Provider::Cursor),
        "copilot" => Some(Provider::Copilot),
        _ => None,
    }
}

/// Task keys end up in branch names and worktree paths, so they are strict.
fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 32
        && k.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn clean_list(v: &mut Vec<String>) {
    let mut seen = HashSet::new();
    v.retain_mut(|s| {
        *s = s.trim().to_string();
        !s.is_empty() && seen.insert(s.clone())
    });
}

/// A brief is markdown: a goal line, then headings or list items. A lone paragraph is a flat
/// blob the worker has to read as one wall.
fn is_structured(goal: &str) -> bool {
    goal.lines().skip(1).any(|l| {
        let l = l.trim_start();
        let numbered = l.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        l.starts_with('#') || l.starts_with("- ") || l.starts_with("* ") || numbered
    })
}

pub fn validate(input: PlanInput) -> Result<ValidPlan, String> {
    let mut plan = input;
    let mut errors: Vec<String> = vec![];
    let mut fixes: Vec<Fix> = vec![];

    plan.summary = plan.summary.trim().to_string();
    if plan.summary.is_empty() {
        errors.push("summary is empty: describe the plan in one sentence".into());
    }
    if plan.tasks.is_empty() {
        errors.push("a plan needs at least one task".into());
    }
    if plan.tasks.len() > MAX_TASKS {
        errors.push(format!("{} tasks is too many (max {MAX_TASKS}): merge closely related tasks", plan.tasks.len()));
    }
    for list in [&mut plan.assumptions, &mut plan.open_questions, &mut plan.integration_checks] {
        clean_list(list);
    }

    let mut seen: HashMap<String, String> = HashMap::new();
    for t in &mut plan.tasks {
        t.id = t.id.trim().to_string();
        let id = t.id.clone();
        if !valid_key(&id) {
            errors.push(format!("task id '{id}' must be 1-32 letters, digits, '-' or '_', starting with a letter or digit (e.g. t1)"));
        } else if let Some(prev) = seen.insert(id.to_ascii_lowercase(), id.clone()) {
            errors.push(format!("duplicate task id '{id}' (same as '{prev}'; ids are case-insensitive)"));
        }
        t.title = t.title.trim().to_string();
        t.goal = t.goal.trim().to_string();
        if t.title.is_empty() {
            errors.push(format!("task {id}: title is empty"));
        }
        if t.goal.is_empty() {
            errors.push(format!("task {id}: goal is empty: say what to change, where and why"));
        } else if !is_structured(&t.goal) {
            fixes.push(Fix {
                summary: format!("{id}: brief is one flat paragraph: write a goal line, then ### Scope, ### Steps, ### Acceptance and ### Out of scope bullets"),
                detail: json!({ "rule": "brief_shape", "task": id }),
            });
        }
        clean_list(&mut t.acceptance_criteria);
        if t.acceptance_criteria.is_empty() {
            errors.push(format!("task {id}: add 1-5 testable acceptance_criteria"));
        }
        clean_list(&mut t.verify);
        if t.verify.is_empty() {
            errors.push(format!(
                "task {id}: add at least one verify command (a shell command run from the repo root that exits 0 when the task is done, e.g. the project's test or build command)"
            ));
        }
        let mut scope_ok = vec![];
        for g in &t.write_scope {
            match scope::normalize(g) {
                Ok(g) => scope_ok.push(g),
                Err(e) => errors.push(format!("task {id}: {e}")),
            }
        }
        clean_list(&mut scope_ok);
        if let Err(e) = scope::ScopeMatcher::new(&scope_ok) {
            errors.push(format!("task {id}: {e}"));
        }
        t.write_scope = scope_ok;
        clean_list(&mut t.read_scope);
        clean_list(&mut t.depends_on);
        t.size = t.size.as_deref().map(|s| s.trim().to_ascii_uppercase()).filter(|s| matches!(s.as_str(), "S" | "M" | "L"));
        t.risk = t.risk.as_deref().map(|s| s.trim().to_ascii_lowercase()).filter(|s| matches!(s.as_str(), "low" | "medium" | "high"));
        match t.agent_hint.as_deref().map(str::trim) {
            None | Some("") | Some("any") => t.agent_hint = None,
            Some(h) => match parse_provider(h) {
                Some(p) => t.agent_hint = Some(p.as_str().into()),
                None => {
                    fixes.push(Fix {
                        summary: format!("{id}: ignored unknown agent_hint '{h}'"),
                        detail: json!({ "rule": "agent_hint", "task": id, "hint": h }),
                    });
                    t.agent_hint = None;
                }
            },
        }
        t.account = t.account.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(String::from);
        check_staffing(t, &mut errors, &mut fixes);
    }
    if !errors.is_empty() {
        return Err(reject(&errors));
    }

    let keys: Vec<String> = plan.tasks.iter().map(|t| t.id.clone()).collect();
    for t in &plan.tasks {
        for d in &t.depends_on {
            if d == &t.id {
                errors.push(format!("task {} depends on itself", t.id));
            } else if !keys.contains(d) {
                errors.push(format!("task {} depends on unknown task '{d}' (known: {})", t.id, keys.join(", ")));
            }
        }
    }
    if !errors.is_empty() {
        return Err(reject(&errors));
    }
    if let Some(cycle) = find_cycle(&plan.tasks) {
        return Err(reject(&[format!(
            "dependency cycle {}: remove one depends_on edge so the tasks run in a clear order",
            cycle.join(" → ")
        )]));
    }

    order_overlaps(&mut plan.tasks, &mut fixes);
    if let Some(fix) = collapse_if_small(&mut plan) {
        fixes.push(fix);
    }
    Ok(ValidPlan { plan, fixes })
}

fn reject(errors: &[String]) -> String {
    let list: Vec<String> = errors.iter().map(|e| format!("- {e}")).collect();
    format!("Plan rejected:\n{}\nFix these and call codly_submit_plan again.", list.join("\n"))
}

/// before[i][j]: task j (transitively) depends on task i.
fn closure(tasks: &[TaskInput]) -> Vec<Vec<bool>> {
    let n = tasks.len();
    let idx: HashMap<&str, usize> = tasks.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
    let mut before = vec![vec![false; n]; n];
    for (j, t) in tasks.iter().enumerate() {
        for d in &t.depends_on {
            if let Some(&i) = idx.get(d.as_str()) {
                before[i][j] = true;
            }
        }
    }
    // Transitive closure (Floyd–Warshall): whoever precedes k also precedes what k precedes.
    for k in 0..n {
        let after_k = before[k].clone();
        for row in before.iter_mut().filter(|r| r[k]) {
            for (cell, &kj) in row.iter_mut().zip(&after_k) {
                *cell |= kj;
            }
        }
    }
    before
}

/// A dependency cycle as a list of ids ending where it started, if any.
pub fn find_cycle(tasks: &[TaskInput]) -> Option<Vec<String>> {
    let idx: HashMap<&str, usize> = tasks.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
    // 0 = unvisited, 1 = on the stack, 2 = done
    fn dfs(i: usize, tasks: &[TaskInput], idx: &HashMap<&str, usize>, color: &mut [u8], stack: &mut Vec<usize>) -> Option<Vec<String>> {
        color[i] = 1;
        stack.push(i);
        for d in &tasks[i].depends_on {
            let Some(&j) = idx.get(d.as_str()) else { continue };
            if color[j] == 1 {
                let start = stack.iter().position(|&s| s == j).unwrap_or(0);
                let mut cyc: Vec<String> = stack[start..].iter().map(|&s| tasks[s].id.clone()).collect();
                cyc.push(tasks[j].id.clone());
                return Some(cyc);
            }
            if color[j] == 0
                && let Some(c) = dfs(j, tasks, idx, color, stack)
            {
                return Some(c);
            }
        }
        stack.pop();
        color[i] = 2;
        None
    }
    let mut color = vec![0u8; tasks.len()];
    for i in 0..tasks.len() {
        if color[i] == 0
            && let Some(c) = dfs(i, tasks, &idx, &mut color, &mut vec![])
        {
            return Some(c);
        }
    }
    None
}

/// Topological order (stable: plan order among tasks whose dependencies are satisfied).
pub fn topo_order(tasks: &[TaskInput]) -> Vec<usize> {
    let mut done: HashSet<&str> = HashSet::new();
    let mut out = vec![];
    while out.len() < tasks.len() {
        let next = tasks.iter().enumerate().find(|(i, t)| !out.contains(i) && t.depends_on.iter().all(|d| done.contains(d.as_str())));
        match next {
            Some((i, t)) => {
                done.insert(t.id.as_str());
                out.push(i);
            }
            None => break, // cycle: validate() rejects those before calling this
        }
    }
    out
}

/// Order unordered tasks whose write scopes may collide, and put owners of hot files
/// (manifests, lockfiles) before every other task writing in the same package.
fn order_overlaps(tasks: &mut [TaskInput], fixes: &mut Vec<Fix>) {
    let n = tasks.len();
    loop {
        let before = closure(tasks);
        let unordered = |i: usize, j: usize| !before[i][j] && !before[j][i];
        let mut edge: Option<(usize, usize, Fix)> = None; // (dependent, dependency, why)
        'scan: for i in 0..n {
            for j in (i + 1)..n {
                if !unordered(i, j) {
                    continue;
                }
                if let Some((a, b)) = scope::overlap(&tasks[i].write_scope, &tasks[j].write_scope) {
                    let (ti, tj) = (&tasks[i].id, &tasks[j].id);
                    let summary = format!("{tj} now waits for {ti}: their write scopes overlap ('{a}' / '{b}')");
                    let detail = json!({ "rule": "scope_overlap", "task": tj, "depends_on": ti, "globs": [a, b] });
                    edge = Some((j, i, Fix { summary, detail }));
                    break 'scan;
                }
            }
        }
        if edge.is_none() {
            'hot: for i in 0..n {
                for hot in tasks[i].write_scope.iter().filter(|g| is_hot_file(g)) {
                    let dir = hot.rfind('/').map_or("", |p| &hot[..=p]);
                    let area = vec![format!("{dir}**")];
                    for j in 0..n {
                        if j != i && unordered(i, j) && scope::overlap(&area, &tasks[j].write_scope).is_some() {
                            let (ti, tj) = (&tasks[i].id, &tasks[j].id);
                            let summary = format!("{tj} now waits for {ti}: {ti} owns {hot}, and manifest edits go first");
                            let detail = json!({ "rule": "hot_file", "task": tj, "depends_on": ti, "file": hot });
                            edge = Some((j, i, Fix { summary, detail }));
                            break 'hot;
                        }
                    }
                }
            }
        }
        let Some((dependent, dependency, fix)) = edge else { return };
        let dep_id = tasks[dependency].id.clone();
        tasks[dependent].depends_on.push(dep_id);
        fixes.push(fix);
    }
}

fn is_hot_file(glob: &str) -> bool {
    !glob.contains(['*', '?', '[', '{']) && HOT_FILES.contains(&glob.rsplit('/').next().unwrap_or(glob))
}

/// "Not worth parallelizing": every task is S-sized and the DAG is a chain, or the whole
/// plan writes at most 3 named files. One worker then does it all (agents-backend §4.8).
fn collapse_if_small(plan: &mut PlanInput) -> Option<Fix> {
    let tasks = &plan.tasks;
    if tasks.len() < 2 || !tasks.iter().all(|t| t.size.as_deref() == Some("S")) {
        return None;
    }
    let before = closure(tasks);
    let n = tasks.len();
    let chain = (0..n).all(|i| (0..n).all(|j| i == j || before[i][j] || before[j][i]));
    let concrete = tasks.iter().all(|t| !t.write_scope.is_empty() && t.write_scope.iter().all(|g| !g.contains(['*', '?', '[', '{'])));
    let files: HashSet<&String> = tasks.iter().flat_map(|t| &t.write_scope).collect();
    if !(chain || (concrete && files.len() <= 3)) {
        return None;
    }
    // Tasks the Lead staffed differently are a deliberate split.
    let staffing: HashSet<String> = tasks.iter().map(|t| format!("{:?}|{:?}|{:?}", t.assignee, t.consultant, t.session)).collect();
    if staffing.len() > 1 {
        return None;
    }

    let order = topo_order(tasks);
    let ids: Vec<String> = order.iter().map(|&i| tasks[i].id.clone()).collect();
    let mut merged = TaskInput {
        id: "t1".into(),
        title: truncate(&plan.summary, 80),
        goal: String::from("Do these steps in order:\n"),
        size: Some("S".into()),
        ..Default::default()
    };
    let unrestricted = tasks.iter().any(|t| t.write_scope.is_empty());
    for (n, &i) in order.iter().enumerate() {
        let t = &tasks[i];
        // Indent the brief under its number so its headings and bullets stay inside the item.
        let brief = t.goal.lines().enumerate().map(|(i, l)| if i == 0 { l.to_string() } else { format!("   {l}") }).collect::<Vec<_>>().join("\n");
        merged.goal.push_str(&format!("{}. {}: {brief}\n", n + 1, t.title));
        merged.acceptance_criteria.extend(t.acceptance_criteria.iter().cloned());
        merged.verify.extend(t.verify.iter().cloned());
        merged.read_scope.extend(t.read_scope.iter().cloned());
        if !unrestricted {
            merged.write_scope.extend(t.write_scope.iter().cloned());
        }
    }
    merged.goal = merged.goal.trim_end().to_string();
    clean_list(&mut merged.acceptance_criteria);
    clean_list(&mut merged.verify);
    clean_list(&mut merged.read_scope);
    clean_list(&mut merged.write_scope);
    let rank = |r: &Option<String>| match r.as_deref() {
        Some("high") => 3,
        Some("medium") => 2,
        Some("low") => 1,
        _ => 0,
    };
    merged.risk = tasks.iter().map(|t| &t.risk).max_by_key(|r| rank(r)).cloned().flatten();
    let hints: HashSet<&Option<String>> = tasks.iter().map(|t| &t.agent_hint).collect();
    merged.agent_hint = if hints.len() == 1 { tasks[0].agent_hint.clone() } else { None };
    let pins: HashSet<&Option<String>> = tasks.iter().map(|t| &t.account).collect();
    merged.account = if pins.len() == 1 { tasks[0].account.clone() } else { None };
    merged.assignee = tasks[0].assignee.clone();
    merged.consultant = tasks[0].consultant.clone();
    merged.session = tasks[0].session.clone();

    let why = if chain { "a chain of small tasks" } else { "small tasks writing at most 3 files" };
    let fix = Fix {
        summary: format!("collapsed {} small tasks ({}) into one: {why} runs faster in one worker", ids.len(), ids.join(", ")),
        detail: json!({ "rule": "collapse_small", "merged": ids, "chain": chain }),
    };
    plan.tasks = vec![merged];
    Some(fix)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}
