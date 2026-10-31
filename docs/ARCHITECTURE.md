# Codly — architecture (v0.1 vertical slice)

Codly is a local-first macOS app that runs a team of coding agents — the Claude Code, Codex,
Gemini CLI and OpenCode you already have installed and signed in to — under one supervisor,
in isolated git worktrees, with two synchronized views of the same truth: a **Workspace**
(chat, timeline, board, dependency graph, diffs, terminal, inspector) and a **World** (an
animated three.js office where every motion is a real event).

Research behind every decision: `docs/research/` (7 reports + `CRITIQUE.md`, the
consolidated decision list) and `docs/research/munder/` (a source-level teardown of
Munder Difflin). Fixtures captured from the real CLIs: `docs/research/fixtures/`.

## 1. Decisions in one table

| Area | Decision | Why (evidence) |
|---|---|---|
| Shell | Tauri 2.12 + Svelte 5 + Vite 8, no SvelteKit | fine-grained updates for streaming; ~10× smaller than Electron; tauri-stack §6 |
| Core | `codly-core` crate, **no Tauri dependency** | testable with `cargo test`, daemon-ready |
| Claude Code | `claude -p --input-format stream-json --output-format stream-json` + the stdin **control protocol** (`can_use_tool` approvals, `interrupt`) | the protocol both official SDKs use; Anthropic's 2026-10-07 note: `claude -p` still draws on your plan; cli-claude-codex §2 |
| Codex | one private `codex app-server` (JSON-RPC over stdio), one thread per agent | live command output, real approvals, `turn/diff`, rate limits; §3.3 |
| Gemini, OpenCode, Cursor, Copilot | **ACP** client (own tokio transport, schema types) | one adapter for the long tail; cli-others-acp §11 |
| Inter-agent context | **Codly MCP server** given to every agent (`codly_*` tools) + a supervisor **blackboard** | typed, validated, auditable; Munder's file mailboxes and typed PTY nudges were its most fragile part (coordination.md §9) |
| Supervisor | the **Lead**: a real CLI agent (read-only) that chats with you and submits plans through `codly_submit_plan`; Codly enforces the DAG, scopes, leases, gates | Munder's supervisor rules were prompt-only and unenforced |
| Isolation | one git worktree + branch per task, branched from the plan's integration branch; **Codly commits**, agents never do | Codex sandbox makes `.git` read-only (CRITIQUE W2) |
| Merge | serial merge queue into `codly/<plan>/integration`, `git merge-tree` pre-check, never push | agents-backend §6.8 |
| Storage | one SQLite (rusqlite bundled, WAL); `event` table is the source of truth; raw provider lines in `runs/<id>/raw.ndjson` | replayable timeline and world |
| Retrieval | per-repo FTS5 over paths, regex symbols and contextual 60-line chunks (path + enclosing symbols prefixed) → RRF → agreement confidence → **pointer pack** (≤ 1.2k tokens) + `codly_search` | commit-replay Hit@5 0.66–0.84 on 4 real repos vs 0.25–0.32 naive grep on 3 of them (0.69 on pi-mono); `examples/bench_localize.rs` |
| Memory | scoped memories (global / project = shared, agent = private, task), FTS5; temporal knowledge triples, many derived from real events | the user's "shared + single memory, triplets, RAG" |
| World | three.js (`src/office/`: engine, reducer, sims, traits, breakroom), procedural art (zero third-party assets), A*, DOM text overlay, render-on-change (0 % CPU idle) | design-world §9; Munder's PixiJS floor never idles and ships non-redistributable LimeZu tiles |
| Approvals | one broker: Codly rules first (balanced / strict / autonomous / full access), then you; choices are exactly what the provider offered | CRITIQUE D6 |

## 2. Processes

```
Codly.app (Tauri, one process)
├─ webview: Svelte UI ── commands ──► src-tauri ──► codly-core (tokio)
│                ◄── one ordered Channel of event batches (≈30 Hz, coalesced)
└─ codly-core
   ├─ store (SQLite WAL)          ├─ index (FTS5, per repo)     ├─ pty (portable-pty terminals)
   ├─ orchestrator: Lead, planner, scheduler, gates, merge queue, approval broker
   ├─ mcp server (Unix socket 0600, per-run tokens)
   └─ adapters (one actor per agent run)
        ├─ claude -p … stream-json   (one process per agent)
        ├─ codex app-server          (one process, N threads)
        └─ gemini --acp / opencode acp / cursor-agent acp / copilot --acp
             └─ each agent launches `<Codly exe> mcp` (stdio MCP bridge → Unix socket)
```

PATH: GUI apps get launchd's minimal PATH, so Codly captures the login shell env once
(`$SHELL -ilc '/usr/bin/env -0'`, 5 s timeout) and resolves each CLI on it at every spawn.
Child envs are scrubbed of variables that silently change billing or session behaviour
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `CLAUDE_CODE_*`…). Codly never reads tokens or keychains.

## 3. One event stream, two views

Every adapter normalizes its provider protocol into `EventKind` (`crates/codly-core/src/model.rs`,
mirrored to TypeScript in `src/lib/gen/`). The orchestrator adds domain events (task, plan,
decision, commit, verify, merge, memory, triple). Each event is persisted with a global id
(the UI cursor) and broadcast; streaming deltas are ephemeral (id 0).

- **Workspace**: chat threads (Lead + 1:1 per agent), per-agent timeline lanes, shared board +
  each agent's own todo list, task DAG, diffs, terminal, approvals inbox, inspector.
- **World** (`src/office/`): a pure reducer (`reducer.ts`) over the same events → per-agent activity →
  locomotion and poses in the three.js engine (`engine.ts`), plus `sims.ts` (needs, matchmaking),
  `traits.ts` and the break room. The building is the repo's **lot** (core `lot:{ws}`, `crate::lot`):
  walls, doors, windows, floor tiles and furniture as data; `layout.ts` finds the rooms and turns
  furniture with a job (desks, racks, shelves, the mailbox, archive cabinets) into stations. A
  repo's first lot is the Studio (`src/office/lots/studio.json`) if it has history, else the Garage.
  Honesty contract: ≤ 250 ms from event to visible state, no random wandering by default,
  provenance on every bubble.
- **Inspector** (both views): what it's doing now and **why** — every fact carries a badge:
  Observed (CLI event), Inferred (Codly heuristic), Provided (Codly put it in context, with
  citations), Model-stated (the model's own words).

## 4. Orchestration

```
you ─► Lead (read-only agent: Read/Grep/Glob + codly tools)
          │ codly_submit_plan {tasks: id, goal, acceptance, write_scope, depends_on, verify}
          ▼
       validate: schema, DAG (no cycles), overlapping write scopes → dependency edges,
                 collapse trivially-small plans to one task              → Decision rows
          ▼ (you click Run plan, or auto-run)
       integration branch codly/<plan>/integration + worktree
          ▼
       scheduler: deps merged ∧ scope lease free ∧ slots (3 total, 2 per provider) → route
          ▼
       worker (worktree, branch codly/<plan>/<key>) ◄─ prompt = task brief + pointer pack
          │                                             + memory pack + dependency notes
          │ codly_submit_report (or turn end)
          ▼
       gate: Codly commits → runs verify → scope check → [reviewer, other vendor] → fix loop ≤ 2
          ▼
       merge queue (serial): merge-tree pre-check → merge → dependents become ready
          ▼
       plan done → Lead gets a status turn → summary to you → "Merge into <branch>" / keep branch
```

Every routing, scheduling, gate and merge choice is written to the `decision` table and
emitted as an event, so "Why did Codex get this task?" always has an answer.

**Parallel requests.** Each request is its own thread (`lead`, `lead:<id>`) with its own Lead
session ("Lead", "Lead 2"…, at most 3 live; idle ones pause and resume by session id). Plans,
follow-ups and workers' questions stay in their request's thread; every Lead's board header
shows the other requests' live plans and files so they don't plan conflicting changes. Write
scopes are leased globally, so parallel plans never edit the same files at once.

**Several accounts per CLI.** Each sign-in is its own config dir (`CLAUDE_CONFIG_DIR`,
`CODEX_HOME`, `GEMINI_CLI_HOME`; Codex gets one app-server per `CODEX_HOME`), listed in
Settings → Accounts. Agents' `RateLimit` events feed each account's usage windows (persisted).
Slots are per account; an account at ≥ 85% of a window is parked. The scheduler sends a task
to the account the Lead pinned it to (`account` in the plan, only when the user names one,
from the Accounts line of its board), else by strategy: Spread (most quota left) or In order.
The route decision names the chosen account, why, and every account it passed over.

**Standing team and consultants.** Team members are set globally in Settings (a repo can switch one
off or add its own): each gets one persistent agent per repo with its own model, role, private memory,
CLI session and a stable worktree, and resumes its conversation task after task. A *consultant*
(`ConsultantSpec` in `model.rs`: title, optional harness and model) is hired by the Lead for one task
and archived when it ends. Hire manifests (`hire.rs`) only pre-fill the add-agent wizard; they can
never start an agent. See `orchestrator/team.rs`.

**Request modes.** Picked per request in the composer (`LeadMode`): **Ask** (answers only), **Plan**
(proposes a plan, you press Run plan), **Auto** (plans and runs at once), **Direct** (no plan card:
hands the work to a team member or consultant at once).

**Approval modes** (`orchestrator/approvals.rs`): balanced (rules decide the obvious, else ask you),
strict (ask for everything), autonomous (allow what stays in the agent's worktree) and **Full access**
(auto-approve everything, risky commands included, except the hard floors: questions, permission
changes, writes by read-only agents and Codly's own data dir still ask).

**Triggers and inbound** (Settings → Triggers / Inbound). `triggers.rs` holds the pure policy and
`orchestrator/triggers.rs` the loop: schedules that send a prompt to the Lead on a repeating clock
and a network mode that gates every
inbound path. `inbound/` has the sources: a loopback webhook listener (one secret per endpoint,
created switched off) and Slack over Socket Mode (config and gating exist; the socket is not
connected yet). Inbound text reaches the Lead wrapped as untrusted outside input. Secrets live in the
Keychain. `dictation.rs` is opt-in voice input: a clip goes to Groq Whisper with your own key and the
transcript lands in the composer draft, never auto-sent.

## 5. Memory and knowledge

| Kind | Scope | Written by | Read by |
|---|---|---|---|
| Memory (fact, decision, lesson, preference) | global / project (shared), agent (private) | agents via `codly_memory_write`, you, Codly (task summaries) | `codly_memory_search`, the memory pack at task start |
| Blackboard note | task (+ project) | `codly_post_note` | dependents' prompts, the Lead, the world's whiteboard |
| Triple (s, p, o, valid_from/to) | project | Codly from real events (agent edited file, task depends on task…), agents via `codly_kg_add` | `codly_kg_query`, the knowledge-graph view |
| Code index | repo | Codly | pointer packs, `codly_search`, the inspector's "why these files" |

Text that came from another agent is injected as quoted, attributed data ("note from Bo,
untrusted") and can never change approvals or policy.

## 6. Safety

- Agents run unmodified CLIs signed in by you; Codly never touches credentials.
- Worktree isolation; Codex `workspace-write` sandbox; Claude workers in `acceptEdits` with
  shell commands routed to the broker; Lead and reviewers read-only.
- Approval broker: rules (path outside worktree, `rm -rf`, `git push`, `curl | sh`, `sudo` →
  always ask), then you. No silent timeouts; choices mirror the provider's.
- Clean-room profile for untrusted repos (`--setting-sources`, `--strict-mcp-config`).
- Never deletes a dirty or unmerged worktree; never pushes.

## 7. Deliberately deferred (ponytail list)

| Deferred | Add when |
|---|---|
| Embeddings (model2vec / MiniLM) for memory and code search | the built-in commit-replay benchmark shows lexical recall is the bottleneck |
| Tree-sitter symbol tables | regex symbols miss too much on the benchmark |
| ~~Deep Agents + OpenRouter sidecar (API-key backend)~~ | dropped (2026-10-26): OpenCode with its OpenRouter provider covers API-key models |
| Launchd daemon so agents survive app restarts | resume-by-session-id proves insufficient |
| PTY "take over" of a headless agent | users ask to drive an agent by hand mid-task |
| Slack Socket Mode connection (the source and its gates exist, the socket does not) | a user asks for Slack as an inbound path |
| Windows/Linux | after macOS ships |

## 8. Code map

```
crates/codly-core/src/
  model.rs        contract: domain + event types (ts-rs → src/lib/gen)
  env.rs proc.rs  login env, CLI detection, scrubbed child envs, process groups, NDJSON reader
  adapters/       claude.rs (stream-json + control), codex.rs (app-server), acp.rs, classify.rs
  orchestrator/   Core facade, Lead, planner validation, scheduler, gates, merge queue, approvals
  store/          schema.sql + typed queries          git.rs   worktrees/commits/merges
  mcp/            server.rs (UDS), bridge.rs (`codly mcp`), tools.rs (tool specs)
  index/          retrieval                            memory/  memories, triples, packs
  pty.rs          terminals
  byok.rs         opt-in OpenRouter key for OpenCode (Keychain)   skills.rs   skills scan/install
  ide.rs          confined file ops for the workspace IDE         install.rs  CLI install terminal
  blockers.rs     "ask me" cards (codly_ask_user)                 hire.rs     hire manifests
  triggers.rs     schedules, network gate                         dictation.rs  Groq Whisper dictation
  inbound/        webhook + Slack sources, Keychain secrets       career.rs   XP, ranks, traits
src-tauri/        commands, UI bus (coalescing Channel), plugins, window
src/              Svelte app: lib/api.ts (transport), lib/state, rooms/, ide/
  office/         the World (three.js): engine, reducer, sims, traits, breakroom
  onboarding/     first-run quest line     shell/   top bar, rail, crew strip, office layer
```

## 9. Verified so far (2026-10-25)

| What | How | Result |
|---|---|---|
| Approval round trip on every CLI | `examples/adapter_smoke.rs`, live | Claude 2.1.309, Codex 0.161, Gemini 0.62, OpenCode 1.18: approve → command runs → turn completes |
| Agents call Codly's MCP tools | live (Claude, Codex, OpenCode) | tools visible and callable without prompts |
| Lead → plan → worker → verify → merge | `examples/e2e.rs`, live | 1 task on Codex in 28 s; 2 parallel Claude tasks + Codex reviewers in 45 s; correct diffs |
| Memory + knowledge graph from real events | `examples/e2e.rs`, live | task-summary memory, 12 temporal triples with repo-relative file entities |
| UI ↔ Rust command boundary | `src-tauri/src/ipc_tests.rs` (Tauri mock runtime, real Core, real ACL) | commands answer with the frontend's argument names; capability allowlist enforced |
| World idle cost | world tests + browser measurement | 0 animation frames and 0 timer wakeups over 10 s when nothing happens |
| Tests | `cargo test --workspace`, `pnpm check`, `pnpm test` | all green |

Not yet verified: driving the bundled app window end to end (manual check pending), Cursor and Copilot (not installed), resume after an app crash with live CLIs.
