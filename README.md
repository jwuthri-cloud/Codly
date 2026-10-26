# Codly

A team of coding agents, one living office.

Codly runs the agent CLIs you already use — **Claude Code, Codex, Gemini CLI, OpenCode** — as a
team under a supervisor, each in its own git worktree, on your existing subscriptions. You get two
synchronized views of the same real events:

- **Workspace** — chat with the Lead (and 1:1 with any agent), per-agent timeline, shared board and
  each agent's own todo list, task dependency graph, diffs, terminal, approvals inbox, and an
  inspector that explains *what* each agent is doing and *why* (with provenance on every fact).
- **World** — an animated pixel office where agents walk to the library to search, the server room
  to run tests, raise a hand when they need you, and celebrate when tests pass. Nothing in it is
  fake: every motion is an event.

## How it works

1. You describe what you want to the **Lead**. It reads the repo (read-only) and either answers or
   submits a small task plan.
2. Codly validates the plan (no cycles, no two tasks writing the same files at once), you click
   **Run plan**, and workers start in isolated worktrees, each with a pointer pack of relevant code
   and the team's memory.
3. When a worker finishes, Codly commits, re-runs the verify commands, checks the write scope,
   optionally asks a reviewer from another vendor, and merges into the plan's integration branch.
4. When the plan is done you review the diff and merge it into your branch. Codly never pushes.

Agents coordinate through Codly's MCP tools (`codly_board`, `codly_search`, `codly_memory_*`,
`codly_kg_*`, `codly_post_note`, `codly_submit_plan`, `codly_submit_report`, …), so every exchange is
typed, validated and visible.

## Requirements

- macOS 14+
- At least one agent CLI installed and signed in (`claude`, `codex`, `gemini`, `opencode`)
- For development: Rust 1.90+, Node 22+, pnpm

## Develop

```bash
pnpm install
pnpm tauri dev          # the app
pnpm dev                # the UI alone in a browser, on a scripted mock session
cargo test --workspace  # core tests (adapters replay real CLI fixtures)
```

## Docs

- [Architecture](docs/ARCHITECTURE.md) — decisions, processes, event model, orchestration, memory.
- [Research](docs/research/) — the reports behind every decision, the consolidated
  [critique](docs/research/CRITIQUE.md), and a source-level teardown of Munder Difflin.

## Notes on subscriptions

Codly spawns the unmodified CLIs that you installed and signed in to; it never reads, stores or
proxies credentials. Each agent shows a billing badge (subscription, API key, third party), and API-key
variables that would silently switch billing are removed from agent environments. Provider terms
change; check them for heavy parallel use.
