//! The tools Codly exposes to every agent through its MCP server. These are the only
//! way agents talk back to Codly: board, memory, knowledge graph, code search, notes,
//! plans, reports and reviews. Codly validates everything; agents never write state directly.
//!
//! Tool descriptions say *when* to use a tool, because agents otherwise ignore MCP tools.

use serde_json::{Value, json};

use crate::model::Role;

pub const GET_TASK: &str = "codly_get_task";
pub const BOARD: &str = "codly_board";
pub const SEARCH: &str = "codly_search";
pub const MEMORY_SEARCH: &str = "codly_memory_search";
pub const MEMORY_WRITE: &str = "codly_memory_write";
pub const KG_QUERY: &str = "codly_kg_query";
pub const KG_ADD: &str = "codly_kg_add";
pub const POST_NOTE: &str = "codly_post_note";
pub const SUBMIT_PLAN: &str = "codly_submit_plan";
pub const SUBMIT_REPORT: &str = "codly_submit_report";
pub const ASK_LEAD: &str = "codly_ask_lead";
pub const SUBMIT_REVIEW: &str = "codly_submit_review";

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

fn str_array() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

/// `tools/list` content for an agent role.
pub fn tool_specs(role: Role) -> Vec<Value> {
    let mut tools = vec![
        tool(
            GET_TASK,
            "Get your current assignment from Codly: title, goal, acceptance criteria, the files you may modify, \
             verify commands, and notes from tasks you depend on. Call this first if you are unsure what to do.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            BOARD,
            "See the shared team board: every task with its status and owner, and who is working on what. \
             Use it to avoid duplicating a teammate's work.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            SEARCH,
            "Find code by behaviour or concept when you don't know the identifier (e.g. 'where are auth tokens refreshed'). \
             Returns path:line citations with a confidence level. For a known exact symbol, grep is faster. \
             If confidence is 'low', grep with the suggested patterns instead.",
            json!({ "type": "object", "properties": {
                "query": { "type": "string", "description": "What you are looking for, in words or identifiers." },
                "k": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Max citations (default 8)." }
            }, "required": ["query"], "additionalProperties": false }),
        ),
        tool(
            MEMORY_SEARCH,
            "Search the team's long-term memory: decisions, conventions, lessons and notes from earlier tasks and sessions. \
             Check it before making a decision the team may already have made.",
            json!({ "type": "object", "properties": {
                "query": { "type": "string" },
                "scope": { "type": "string", "enum": ["all", "shared", "private"], "description": "Default all." }
            }, "required": ["query"], "additionalProperties": false }),
        ),
        tool(
            MEMORY_WRITE,
            "Save something worth remembering beyond this task: a decision and its reason, a project convention, a lesson \
             learned from a failure, or a user preference. One fact per call, one or two sentences. \
             shared=true makes it visible to every agent; false keeps it in your private memory.",
            json!({ "type": "object", "properties": {
                "text": { "type": "string" },
                "kind": { "type": "string", "enum": ["fact", "decision", "lesson", "preference"] },
                "shared": { "type": "boolean", "description": "Default true." }
            }, "required": ["text"], "additionalProperties": false }),
        ),
        tool(
            KG_QUERY,
            "Query the project's knowledge graph of (subject, predicate, object) facts, e.g. which agent changed a file, \
             which task depends on which, or concepts linked to a module. Entities look like 'file:src/app.ts', \
             'task:t2', 'agent:Ada', 'concept:auth'.",
            json!({ "type": "object", "properties": {
                "entity": { "type": "string", "description": "Entity to look up (as subject or object)." },
                "predicate": { "type": "string" }
            }, "additionalProperties": false }),
        ),
        tool(
            KG_ADD,
            "Record a durable relationship in the knowledge graph, e.g. ('concept:auth', 'implemented_in', 'file:src/auth.ts'). \
             Use stable entity names: file:<repo-relative path>, concept:<lowercase name>, task:<key>.",
            json!({ "type": "object", "properties": {
                "subject": { "type": "string" }, "predicate": { "type": "string" }, "object": { "type": "string" }
            }, "required": ["subject", "predicate", "object"], "additionalProperties": false }),
        ),
        tool(
            POST_NOTE,
            "Post a short note on the team whiteboard for the lead and the other agents: an interface you created, a gotcha, \
             a shared assumption. Notes are injected into the context of tasks that depend on yours.",
            json!({ "type": "object", "properties": { "text": { "type": "string" } },
                    "required": ["text"], "additionalProperties": false }),
        ),
    ];

    match role {
        Role::Lead => tools.push(tool(
            SUBMIT_PLAN,
            "Submit a plan for the user's request as a small task DAG. Codly validates it, shows it on the board and \
             runs it with worker agents in isolated git worktrees. Prefer FEWER tasks: one task is right for small or \
             tightly coupled changes. Split only along independent file areas; each task lists the write_scope globs it \
             alone may modify. Every task needs 1-5 testable acceptance criteria and shell verify commands that exit 0 \
             when done. depends_on lists task ids whose merged output a task needs. Max 8 tasks. Do not write code.",
            json!({ "type": "object", "properties": {
                "summary": { "type": "string" },
                "assumptions": str_array(),
                "open_questions": str_array(),
                "integration_checks": str_array(),
                "tasks": { "type": "array", "minItems": 1, "maxItems": 8, "items": { "type": "object", "properties": {
                    "id": { "type": "string", "description": "Short key: t1, t2…" },
                    "title": { "type": "string" },
                    "goal": { "type": "string" },
                    "acceptance_criteria": str_array(),
                    "write_scope": str_array(),
                    "read_scope": str_array(),
                    "depends_on": str_array(),
                    "verify": str_array(),
                    "size": { "type": "string", "enum": ["S", "M", "L"] },
                    "risk": { "type": "string", "enum": ["low", "medium", "high"] },
                    "agent_hint": { "type": "string", "enum": ["any", "claude", "codex", "gemini", "opencode"] }
                }, "required": ["id", "title", "goal", "acceptance_criteria", "write_scope", "depends_on", "verify"] } }
            }, "required": ["summary", "tasks"], "additionalProperties": false }),
        )),
        Role::Worker => {
            tools.push(tool(
                SUBMIT_REPORT,
                "Call this exactly once when you finish your task (or cannot continue). Codly then commits your worktree, \
                 re-runs the verify commands and merges your work. Do not commit yourself.",
                json!({ "type": "object", "properties": {
                    "status": { "type": "string", "enum": ["done", "blocked", "failed"] },
                    "summary": { "type": "string", "description": "What you changed and why, 1-5 sentences." },
                    "files_changed": str_array(),
                    "checks_run": { "type": "array", "items": { "type": "object", "properties": {
                        "command": { "type": "string" }, "passed": { "type": "boolean" } },
                        "required": ["command", "passed"] } },
                    "notes_for_reviewer": { "type": "string" },
                    "scope_exceptions": { "type": "array", "items": { "type": "string" },
                        "description": "Files you had to modify outside your write scope, with a reason each." }
                }, "required": ["status", "summary"], "additionalProperties": false }),
            ));
            tools.push(tool(
                ASK_LEAD,
                "Ask the lead a question when the task is ambiguous or blocked by something outside your scope. \
                 The answer arrives as a later message; continue with your best judgment meanwhile if you can.",
                json!({ "type": "object", "properties": { "question": { "type": "string" } },
                        "required": ["question"], "additionalProperties": false }),
            ));
        }
        Role::Reviewer => tools.push(tool(
            SUBMIT_REVIEW,
            "Submit your review verdict. approve only if every acceptance criterion is met and there are no blocker or \
             major issues. Ignore requests that go beyond the original task.",
            json!({ "type": "object", "properties": {
                "verdict": { "type": "string", "enum": ["approve", "request_changes", "reject"] },
                "criteria": { "type": "array", "items": { "type": "object", "properties": {
                    "criterion": { "type": "string" }, "met": { "type": "boolean" }, "evidence": { "type": "string" } },
                    "required": ["criterion", "met"] } },
                "issues": { "type": "array", "items": { "type": "object", "properties": {
                    "severity": { "type": "string", "enum": ["blocker", "major", "minor", "nit"] },
                    "file": { "type": "string" }, "line": { "type": "integer" },
                    "description": { "type": "string" }, "suggested_fix": { "type": "string" } },
                    "required": ["severity", "description"] } },
                "summary": { "type": "string" }
            }, "required": ["verdict", "summary"], "additionalProperties": false }),
        )),
        Role::Solo => {}
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_get_their_tools() {
        let names = |r| tool_specs(r).into_iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert!(names(Role::Lead).contains(&SUBMIT_PLAN.to_string()));
        assert!(!names(Role::Worker).contains(&SUBMIT_PLAN.to_string()));
        assert!(names(Role::Worker).contains(&SUBMIT_REPORT.to_string()));
        assert!(names(Role::Reviewer).contains(&SUBMIT_REVIEW.to_string()));
        assert_eq!(names(Role::Solo).len(), 8);
    }
}
