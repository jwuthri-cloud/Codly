# Munder Difflin: coordination layer (research notes)

- **Subject:** Munder Difflin v0.4.6 (MIT, Electron), coordination only: hive, supervisor, task board, messaging, git worktrees, recovery, hiring.
- **Source:** shallow clone at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/munder-difflin`
- **Date:** 2026-10-25
- **Method:** read-only. No scripts, binaries or npm commands were run. Line numbers are from that clone.
- **Confidence markers:** "inferred" means my reading of the code, not a reproduced behaviour. "unverified" means the code's own comments say so, or I could not check it.

## Short names used below

| Short name | Path (repo-relative) |
|---|---|
| hive.ts | src/main/hive.ts (3,669 lines, read in full) |
| hooks.ts | src/main/hooks.ts |
| index.ts | src/main/index.ts (coordination regions only) |
| control.ts, closingTime.ts, roster.ts, git.ts, worktreeDeps.ts, breaker.ts, workerWake.ts, workerLaunch.ts, integrationBroker.ts, memory.ts, db.ts, hire.ts | src/main/<name> |
| shared modules (taskLedger, broadcast, hiveNudge, agentRole, godIdentity, hire, hireQueue, agentProvider, mcpCatalog) | src/shared/<name>.ts |
| useHive.ts, queueDelivery.ts, useRestoreTeam.ts, store.ts | src/renderer/src/hooks/ and src/renderer/src/store/ |
| TasksKanban, ThreadsPanel, MessageQueueComposer, CommandCenterPanel, WorkersTab, HivePicker, AddAgentModal, AgentDetailPanel, AskMeTab | src/renderer/src/components/<name>.tsx |

---

## 0. Summary

- **The hive is files plus one git repo.** Agents are ordinary CLI processes that read and write JSON files in their own folders. A router in the main process moves each outbox message into the recipient's inbox every 1.5 s (hive.ts:1832, 1842). Only main commits to git (hive.ts:4-5).
- **There is no planner in the code.** Michael (`god`) is a normal agent with a long orchestration prompt (hive.ts:1586). Decomposition, the 4-part dispatch contract, "sole scribe" and "assignee" rules are prompt conventions. Nothing enforces them.
- **Waking an idle agent means typing into its terminal.** The renderer queues a nudge and types it when the agent reads as idle: a hook turn-end, or 12 s of PTY silence (useHive.ts:690-730, 782-955). The Stop-hook drain described in HIVE.md was retired for safety (hooks.ts:493-499).
- **The strongest idea to copy is the control plane.** Pause, halt, steer and live-roster injection all ride hook return values. Nothing is typed into the TUI (hooks.ts:424-427, 506-528, 536-580).
- **Worktrees are isolated but not integrated.** Each agent gets `agent/<id>`. No merge or conflict code exists. A normal agent's worktree is force-removed on tab close, including uncommitted edits (index.ts:486-491).
- **Several integrity bugs** came out of code reading (section 9). The most serious: an agent-chosen message id becomes a file path (hive.ts:1622, 1650), and corrupt JSON is read as empty state and written back (hive.ts:1897-1908, 2794-2796).

---

## 1. What the hive is

### 1.1 Layout on disk

Root: `<harnessHome>/hive/`. `harnessHome` is a folder the user picks (HivePicker.tsx component doc: "A hive is a harness home folder: its own agents, memory, tasks, and history"). The app config is `userData/config.json` (config.ts:491).

| Path under `hive/` | Written by | Purpose |
|---|---|---|
| `registry.json` | main | Hive identity: `godId`, `agents{id → record}` (role, cwd, sessionId, archived, onHold, cwdValid). hive.ts:160-188 |
| `agents/<id>/identity.md` | main, every spawn | Who I am, role, capabilities. hive.ts:1494-1506 |
| `agents/<id>/memory.md` | agent (append) | Durable memory, mined into MemPalace. hive.ts:764-767 |
| `agents/<id>/inbox/*.json` | main (router) | One file per message. Agent moves handled files to `inbox/.done/`. |
| `agents/<id>/outbox/*.json` | agent | Messages to send. Router moves them to `outbox/.sent/`. hive.ts:1883 |
| `agents/<id>/cursor.json` | main | `{lastProcessed}`. Read only by the retired drain (hive.ts:1470-1477). |
| `agents/<id>/settings.json` | main | Per-session Claude `--settings`: hooks, sandbox, MCP. hive.ts:1002-1003 |
| `agents/<id>/.gitignore` | main | Keeps inbox, outbox, settings, cursor and `.codex/` out of git and MemPalace. hive.ts:239-251 |
| `board.md` | god, by prompt | Freeform plan. hive.ts:640-643 |
| `tasks.json` | god (hand edits); main (UI, webhook, Slack writes) | Kanban ledger. hive.ts:644-645 |
| `log.jsonl` | main | Append-only event feed, never rotated. hive.ts:2749-2755 |
| `fleet.json` | main, every 8 s | Live snapshot god reads. hive.ts:2549-2555; index.ts:1287-1329 |
| `cost-ledger.jsonl` | main | One row per usage sample. Untracked from git. hive.ts:2773-2791 |
| `spawn-requests/` | god | Drop box for ephemeral workers, gated by a setting. hive.ts:3055-3088 |
| `PROTOCOL.md`, `COMMANDS.md` | main, regenerated each boot | Agent-facing docs. hive.ts:2955-3102 |
| `bin/` | main | `cth-hook.cjs`, `hive-proxy.cjs`, the `hive-node` launcher, `runtime/node` shim. hive.ts:483-595, 662-672 |
| `hooks.sock` | main | Hook socket on POSIX. Windows uses a named pipe. hive.ts:462-470 |
| `crashes/` | main | Raw PTY tail on abnormal exit. Gitignored, mode 0600, max 50 files. hive.ts:376, 2696-2747 |
| `.git/` | main only | Single-committer repo. hive.ts:674-677 |

Outside the hive root:

- `<harnessHome>/roster.json` and `roster-backups/`: the UI floor (cards, notes, queues). roster.ts:54-60
- `<harnessHome>/worktrees/<agentId>`: isolated worktrees. index.ts:2733-2739
- `userData/harness.db`: SQLite, holds only `kv` and `command_history` (db.ts:49-68, 81)
- Global files written by bridges and pre-accept (see 9.6): `~/.claude/settings.json`, `~/.claude.json`, `~/.codex/{sessions,archived_sessions}`, `~/.grok/hooks/munder-hive.json`, `~/.gemini/config/hooks.json` and `~/.gemini/antigravity-cli/hooks.json`

### 1.2 Storage choice and why it hurt

- **No SQLite for hive state.** The DB file has two tables. Future migrations for agents, message queue and cost ledger are "reserved" but not built (db.ts:36-47).
- **Git is the audit layer by design.** "Everything the hive knows is files in one local git repo" (HIVE.md:36-40). Every mutation calls `commit()`, which runs `git add -A` then `git commit` (hive.ts:2893-2909). Examples of per-mutation commits: hive.ts:801 (register), 1026 (role), 1219 (session), 1674 (send), 1891 (route), 1933 (tasks).
- **Scale problem that shaped the code.** From hive.ts:229-233: "Twenty Codex agents took the hive's .git to 7.5GB, at which point git's own auto-gc tried to repack it and took 22GB of RAM doing so." Mitigations are visible in code: `gc.autoDetach=false` (hive.ts:2808-2821), untracking the cost ledger (hive.ts:2832-2858), untracking Codex homes (hive.ts:2860-2890), and the `.codex/` ignore line (hive.ts:239).
- Git runs with `commit.gpgsign=false`, `user.name=Hive`, an 8 s timeout (hive.ts:2825-2830). Stale `index.lock` or `HEAD.lock` files older than 10 s are deleted (hive.ts:2911-2919). Commit retries: 5 attempts (hive.ts:2898).

### 1.3 Data model

- `HiveMessage` (hive.ts:69-82): `id`, `conversation`, `in_reply_to`, `from`, `to` (agent id, `god`, or `broadcast`), `act`, `subject`, `body`, `hops`, `requires_reply`, `needs_human`, `created_at`.
- `MessageAct` (hive.ts:54): `request | inform | propose | query | agree | refuse | done`.
- `HiveTask` (hive.ts:119-144): `id`, `title`, `description?`, `assignee?`, `status` (`todo | doing | blocked | done`), `dependsOn[]`, `priority` (number), `createdAt`, `humanQA?[]`, `result?`, `slack?`, `webhook?{tokenHash}`.
- `RegistryAgent` (hive.ts:160-183): `id`, `name`, `provider`, `role`, `capabilities`, `cwd`, `isGod`, `isAssistant`, `status`, `lastSeen`, `archived`, `onHold`, `sessionId`, `cwdValid`.

Id formats:

- Message id: `stamp()-shortRand()`, for example `2026-05-30T14-03-11-123Z-a1b2c3` (hive.ts:212-219, 1622).
- Agent id: `slug(name)-<base36 timestamp>` (AddAgentModal.tsx:133-135).
- Worker id: `worker-<reqId>` (index.ts:4709-4711).

### 1.4 Concurrency model

- Only main writes to the hive repo. Agents write their own folders, and the router moves files between them.
- Some writes are atomic: `atomicWriteJson` writes a temp file then renames it (hive.ts:2800-2804). Others are not (see 9.2).
- The single-writer rule is a prompt convention: "NEVER write into another agent's folder — the orchestrator delivers your outbox" (hive.ts:1602). The sandbox allows writes to the whole hive root (hive.ts:1240-1243, 1288-1293), so the OS does not enforce it.

### 1.5 Flow

```
agent A (CLI) -- writes --> agents/A/outbox/<x>.json
                                |  router, every 1.5 s (hive.ts:1832-1893)
                                v
                       routeMessage (hive.ts:1678)
                                |-- agents/B/inbox/<id>.json          (file-based agents)
                                |-- terminal work order typed into PTY (hookless providers)
                                |-- bounce to god                      (no inbox, hop cap, archived)
renderer, every 4 s ---> reads inbox -> queues nudge -> types into PTY when idle (useHive.ts:690-955)
main watchdog, every 15 s ---> types nudge into stalled worker PTY (index.ts:5204-5271)
hook socket <-- hooks from B (Stop, PreToolUse, UserPromptSubmit ...) --> deny / continue:false / additionalContext
```

---

## 2. Supervisor / manager ("Michael")

### 2.1 Identity and boot

- Default name: `DEFAULT_GOD_NAME = 'Michael'` (godIdentity.ts:3). Agent id `god` (useHive.ts:27). Avatar `michael`, accent `lemon` (useHive.ts:421-426).
- Boot (useHive.ts:376-475): spawns `pty-god` with `resume: true` and `isGod: true`, after a 1.2 s delay. Once the terminal reports ready, it types `/remote-control`. On a fresh spawn only, it then types `INITIAL_GOD_PROMPT` (useHive.ts:453-469). A resumed god skips orientation: "a RESUMED Michael already has his full context and must not be re-oriented mid-thread" (useHive.ts:447-449).
- God is never archived (index.ts:912) and is the target of `to: 'human'` (hive.ts:1684).

`INITIAL_GOD_PROMPT` (useHive.ts:75-82) opens with "You're online as Michael, the orchestrator of the hive. Get oriented, then start running the floor:" and asks god to read memory, drain the inbox, review `board.md` and `tasks.json`, check `fleet.json`, and skim `COMMANDS.md`.

### 2.2 The system prompt

For Claude it goes in `--append-system-prompt` (hive.ts:994). Other CLIs get it as an initial prompt through a flag or as a positional argument (hive.ts:846-974). God's text is built in `injectedPrompt` (hive.ts:1530-1613). Key parts, quoted:

- Role: "You are the GOD / ORCHESTRATOR of this hive — your job is to ORCHESTRATE, not to implement" (hive.ts:1586).
- Delegation: "BEFORE you spawn anything, CHECK THE LIVE ROSTER … prefer routing to an EXISTING agent that fits … One capable owner beats a duplicate." (hive.ts:1586)
- Scope: "OWN ONLY THE IMPORTANT … task decomposition, dispatch decisions, sign-offs, conflict resolution, branch integration, and final QA — and remain the sole scribe of board.md." (hive.ts:1586)
- Dispatch contract: "(1) OBJECTIVE … (2) OUTPUT … (3) TOOLS … (4) BOUNDARIES — scope limits + the definition of done." (hive.ts:1586)
- Human asks: set `"status": "blocked"` and push `{q, askedAt}` onto `humanQA` (hive.ts:1587).
- Assignee rule: "ALWAYS set each task's "assignee" to the worker's agent id the moment you dispatch it, and NEVER clear it" (hive.ts:1587).
- Budget: "Steward the token budget." (hive.ts:1587)

Other prompt pieces:

- The PROTOCOL file (hive.ts:2955-3100), covering workspace, sending, rules, board vs tasks, asks, guardrails, fleet monitoring, spawning, semantic memory.
- God's spawn line, present only when the setting is on: "SPAWNING A WORKER: you can start an ephemeral worker yourself by writing ONE JSON file into …" (hive.ts:1582-1583).
- Prep-assistant persona: "Do NOT perform the task yourself — your only output is the improved prompt sent to …" (hive.ts:1588-1589).
- Slack rules and the Slack "AUTONOMOUS REQUEST PROTOCOL" (hive.ts:1592-1594; index.ts:1403-1411).

Prompt-cache rule (hive.ts:1511-1517): "PROMPT-CACHE INVARIANT — keep this prefix VOLATILE-FREE. … Volatile context belongs on the live channels — the inbox (hive messages) and the PTY — never baked into this prefix." This is a good rule to keep.

The harness does not enforce any of the contract, scribe, assignee or delegation rules. There is no decomposition code.

### 2.3 How god sees the whole floor

Context reaches god through five channels:

1. **`fleet.json`**, every 8 s (index.ts:5280-5281, 1287-1325). Per agent: `id`, `name`, `role`, `cwd`, `isGod`, `breaker`, `tokens`, `usd` (lifetime), `sessionUsd`, `lastTool`, `lastActiveSecAgo`, `inboxBacklog`, `onHold`. The file also carries hook-socket health.
2. **Live roster as hook context**, injected on `SessionStart` and every `UserPromptSubmit` (hooks.ts:536-543 calling hive.ts:2588-2662). It lists up to 24 agents (hive.ts:2615) with `ctx NN%` from the statusLine (hive.ts:2635-2640), and an `ON HOLD — 1:1 with the human` marker (hive.ts:2630, 2653-2658). It opens with: "This is the CURRENT floor and it SUPERSEDES any roster earlier in this conversation" (hive.ts:2648-2649). This is the most reusable context mechanism in the codebase.
3. **Mail**: worker terminal events via `informGod` (index.ts:509-522), heartbeat digests (index.ts:1182-1184), breaker escalations (index.ts:1265-1270), settle reports (index.ts:4936-4946).
4. **Shared files**: `board.md`, `tasks.json`, `registry.json`, `log.jsonl`, and each agent's `memory.md` and inbox (paths in 1.1).
5. **Semantic memory**: MemPalace search across the team (hive.ts:3090-3099; memory.ts header).

Heartbeat (index.ts:1158-1184): mail from `heartbeat`, `scheduler`, `breaker` and `system` does not count as god's actionable mail (`SYSTEM_SENDERS`, index.ts:1162). Heartbeat digests go to god's inbox and never type into god's PTY, so the heartbeat "defers around a working god instead of interrupting him" (index.ts:1177-1181).

### 2.4 Guardrails enforced by the harness

- **Circuit breaker** (breaker.ts:1-22). Levels: `healthy → steering → constrained → stopped`. Signals: cost and token velocity from usage samples, repeated identical tool calls and API error storms from hooks, and no progress from coordination-file mtimes and working-directory mtimes. The header says: "steer-first, one level per beat (never jump to a kill), de-escalates a level per healthy beat (recovery), and `hardStop` is OFF by default." God is exempt (index.ts:1197-1199 and index.ts:1240).
- **Enforcement** (index.ts:1259-1275): steer and constrain are mail. Stop kills the PTY and tears the agent down.
- **Control registry** (control.ts:1-18). Pause and tool gates become `PreToolUse` `permissionDecision: 'deny'` (hooks.ts:506-519). Halt returns `{continue:false}` at the next hook (hooks.ts:424-427). Steer notes are queued (max 20, control.ts:25) and delivered once through `additionalContext` on `UserPromptSubmit` or `PostToolUse` (hooks.ts:526-528).
- **Approvals are not in the app.** "Slow human APPROVAL deliberately rides Claude's native prompt" (control.ts:9-10; hooks.ts:502-505). The hook can only deny.
- **Hop cap 12** (hive.ts:205). The counter is harness-owned (hive.ts:1630-1637). Over the cap, a message is dropped with a log line (hive.ts:1659-1666).
- **Worker token caps and idle reaping** (index.ts:4951-4981).

### 2.5 Autonomy switches

- `orchestratorMaySpawn`, default `false` (main/config.ts:428). God cannot spawn workers until the operator turns it on. The doc note says declining "also means declining to CONSUME" (index.ts:4994-4996).
- `autoMode`, default `true` (main/config.ts:427). For Claude this adds `--permission-mode bypassPermissions` (agentProvider.ts, claude preset). The containment is then the OS sandbox the hook settings enable (hive.ts:1281-1293).

### 2.6 Prep assistant

- The `isAssistant` agent is send-only. It is excluded from broadcast (broadcast.ts:44) and direct mail to it bounces to god (hive.ts:1703-1706).
- The renderer frames each "enrich" request as `ENRICH TASK: …` and asks for a self-contained prompt back (useHive.ts:161-168).

---

## 3. Task board

### 3.1 Shared versus per-agent

- **One shared ledger**: `tasks.json`, shaped `{ "tasks": [...] }` (hive.ts:644-645). Plus `board.md` as narrative.
- **No per-agent kanban or todo list exists.** Per-agent state is `identity.md`, `memory.md`, inbox and outbox. The only link to an agent is `assignee`.

### 3.2 States and fields

- Four states: `todo`, `doing`, `blocked`, `done` (hive.ts:124; TasksKanban columns).
- `blocked` plus an unanswered `humanQA` entry means "waiting on the human". `openQuestion()` and `waitsOnHuman()` in TasksKanban.tsx pick the newest entry with no answer and no dismissal.
- `result` is posted back to Slack when the card reaches `done` (hive.ts:131-137).
- `priority` is a number. The UI clamps it to 1 to 5 and defaults missing values to 3 (TasksKanban.tsx, `parseTasks`). Webhook cards use `priority: 1` (index.ts:1877-1883).
- Unknown fields survive. `mergeTaskLedger` keeps every field the writer did not mention (taskLedger.ts:53-67). The god writes fields such as `repo`, `scope`, `commit` and `result` that the schema does not model (taskLedger.ts:4-8).

### 3.3 How agents read and update it

- **No MCP server, no CLI, no skill for tasks.** The only harness MCP-related code is the default MCP catalog for external servers (mcpCatalog.ts). Nothing exposes the board to agents.
- **Agents edit `tasks.json` directly.** God is told to keep it accurate (hive.ts:1587). Other agents are told: "Keep the task you're working reflected in its status" (hive.ts:3007).
- **Main-process writers** go through IPC: `hive:addTask`, `hive:patchTask`, `hive:deleteTask` (index.ts:3514-3533). Each merges against the latest on-disk file (hive.ts:1936-1968).
- **Card creators**: god by hand; Slack promotion (useHive.ts `ensureSlackCard`, around lines 897-921); webhook (index.ts:1877-1891); voice actions through the same hive functions (index.ts:4476-4482).

### 3.4 Dependencies

`dependsOn: string[]` is stored and displayed. AskMeTab uses it to list dependents of a blocked card (AskMeTab.tsx:43). **Nothing enforces it.** No code blocks dispatch while a dependency is open. (inferred from a grep for `dependsOn`: only display and parse sites.)

### 3.5 Dispatch

- Human work does not go straight to a worker. The Command Center sends everything to god as `act: 'request'` (CommandCenterPanel.tsx, `dispatch`, lines 562-578). Its comment reads: "ALL human dispatch flows through the god — never directly into a worker's inbox."
- A worker picked in the dropdown becomes a suggestion appended to the message.
- Ephemeral workers get their objective as a `request` with conversation `worker-<reqId>` (index.ts:4814-4819).

---

## 4. Messaging

### 4.1 Envelope rules

- Only `request`, `query` and `propose` expect a reply by default (hive.ts:1638). The PROTOCOL says: "Only `request`, `query`, and `propose` expect a reply. `inform` and `done` are terminal" (hive.ts:2990-2991).
- Threads are a `conversation` string. Replies carry `in_reply_to` (hive.ts:69-82). ThreadsPanel groups by `conversation` (ThreadsPanel.tsx, `groupThreads`, lines 31-48). There is no thread object in storage.
- `normalize()` takes `id`, `from`, `hops`, `created_at` from the sender when present, with a clamp on `hops` (hive.ts:1619-1641). See 9.1 and 9.5.

### 4.2 Lifecycle of one message

1. The agent writes `agents/<from>/outbox/<any>.json` (hive.ts:2973-2988).
2. `routeOnce`, every 1.5 s (hive.ts:1832-1893), parses the file. It repairs raw newlines inside strings (hive.ts:315-362, 1860-1868), forces `msg.from` to the owning folder (hive.ts:1881), routes it, and moves the file to `outbox/.sent/` (hive.ts:1883). A malformed file goes to `.sent/bad-<name>` (hive.ts:1862-1864).
3. `routeMessage` (hive.ts:1678-1779) resolves targets, applies the per-target rules in 4.3, logs `kind: 'message'` with the list of targets that accepted it (hive.ts:1774), emits `hive:message` to the floor (hive.ts:1775), and calls the closing-time observer (hive.ts:1776-1778).
4. The recipient reads its inbox and moves handled files to `inbox/.done/`. This is a protocol step, not a harness check (hive.ts:1600-1601).
5. On worker release, `settleInbox` files unread mail that arrived before the done signal into `.done/`, and returns unanswered `request`/`query` items so god can be told (hive.ts:1078-1113; index.ts:4926-4946).

### 4.3 Per-target rules (`routeMessage`)

| Recipient | What happens | Where |
|---|---|---|
| Prep assistant | Bounced to god | hive.ts:1703-1706 |
| Archived agent (terminal closed) | Filed in its inbox. If the message expects a reply, the sender gets "[no one is there to answer — … is archived]" | hive.ts:1720-1739 |
| Hookless provider (kimi, copilot, custom) | Terminal work order typed into its PTY. If there is no renderer or PTY, bounced to god | hive.ts:1746-1751; useHive.ts:609-635 |
| Proxy-tier (qwen, crush) | Terminal work order (`inboxDelivery: 'terminal'`) | hive.ts:1757-1763; agentProvider.ts:340, 441 |
| All others (claude, codex, grok, gemini, antigravity, opencode, pi, cursor) | Filed in inbox; woken by the renderer nudge | hive.ts:1764 |
| No inbox folder | Drop logged with reason `no-inbox`, bounced to god | hive.ts:1769-1772 |
| Hop cap exceeded | Drop logged with reason `hop-cap` | hive.ts:1659-1666 |

Broadcast: `selectBroadcastTargets` returns every agent except the sender, the prep assistant and archived agents (broadcast.ts:36-48). Each target gets its own file with the same message id.

### 4.4 How a message reaches a running CLI

Nothing pushes mail into a running CLI. There are four mechanisms:

1. **Renderer nudge (primary).** Every 4 s the renderer reads each live agent's inbox. For any message id not yet nudged, it queues the nudge with precondition `inbox-nonempty` (useHive.ts:690-730; hiveNudge.ts:25-28). The queue keeps at most one pending nudge per agent (store.ts:911-917). The drain types it only when all of these hold (useHive.ts:798-888; queueDelivery.ts:48-56, 82-92):
   - status is `idle`, or it is breaker-pinned and quiet for at least 12 s;
   - the 35 s boot grace has passed;
   - auto-delivery is not paused;
   - there is no user draft or open menu (`isTerminalAutomationSafe`);
   - the 4.5 s cooldown has passed;
   - the inbox is still non-empty (re-checked right before typing).

   `submitToPty` (useHive.ts:129-157) uses bracketed paste for multi-line text, sends Enter 140 ms later, and serializes writes per PTY.
2. **Main watchdog (stalled workers).** `runWorkerWakeBeat` runs every 15 s (index.ts:5171, 5204-5271) with the same kind of guards: 12 s idle, 35 s boot grace, 60 s cooldown, 5-minute re-arm after a permission prompt, and a 90 s stall rule (workerWake.ts:1-70). It types the nudge via `nudgeWorker` (index.ts:5180-5193). God is never a candidate (index.ts:5211).
3. **Terminal work order** for providers without a mailbox. The text is "WORK ORDER FROM HIVE …" and asks the agent to report "in this terminal" (useHive.ts:170-194).
4. **Hook returns** (no typing). Steer notes, god's live roster and standing goals arrive as `additionalContext` on `SessionStart`, `UserPromptSubmit` and `PostToolUse` (hooks.ts:521-580).

**Retired: the Stop-hook drain.** HIVE.md says a Stop hook returns `{"decision":"block","reason":…}` to keep the agent working on its mail (HIVE.md:59-61, 137-141). The code says otherwise: "Never turn unread hive mail into a forced continuation at Stop. That old path bypassed terminal-draft/HITL safety and could spend credits while a user was answering a question" (hooks.ts:493-496). Stop now returns `{}` (hooks.ts:497-499). `drainForStop` is called only from a test (test/hive-windows-prompt.test.cjs:102). It still compares ids as strings (hive.ts:1473), which is the bug described in useHive.ts:284-288, so reviving it would reintroduce that bug.

**Human to agent (personal chat).** MessageQueueComposer adds to a per-agent queue. That queue is drained into the terminal by the same logic as above (store.ts:892-935; useHive.ts:782-955). This is not hive mail: nothing is written to an inbox.

**Human to god (dispatch box).** `hiveSend({to:'god', act:'request'}, 'human')` (CommandCenterPanel.tsx:562-578). **Human reply in a thread:** `hiveSend({to: last.from, act:'inform', conversation, in_reply_to}, 'human')` (ThreadsPanel.tsx:73-81). `to: 'human'` is rewritten to god (hive.ts:1684).

### 4.5 How replies come back

- **Agent outbox files.** Hook-aware and bridged agents are told to write replies to their outbox (hive.ts:2973-2988), and the router delivers them.
- **Terminal providers.** They are told to "report changes, validation, blockers, and next step in this terminal" (useHive.ts:186-192). I found no code that parses terminal output back into hive mail. Their replies are visible only in the terminal (inferred).
- **Harness-generated replies**: "no one is there" notices (hive.ts:1720-1739), settle reports to god (index.ts:4936-4946), breaker and heartbeat mail.
- **No delivery receipts.** `kind: 'message'` in the log records which targets accepted the file (hive.ts:1774). Nothing records that the agent read or answered it.

### 4.6 Voice read layer and secret redaction

`voiceMessages` (hive.ts:2015-2076) returns subject and body after `redactSecrets` (hive.ts:275-302). The pattern battery is duplicated in a test (test/voice-messages.test.cjs). The comment says "The renderer holds ZERO redaction policy."

---

## 5. Git: worktrees, cleanup, integration

### 5.1 Two repos

- **Project repo.** The user's repo. Agents work here or in a worktree.
- **Hive repo.** `<harnessHome>/hive/.git`. Single committer, file-level audit (section 1.2).

### 5.2 Creation

- **Trigger.** `pty:spawn` with `isolate: true`. The Add Agent default is off (AddAgentModal.tsx:238). Ephemeral workers default to on (index.ts:4745).
- **Path.** `<harnessHome>/worktrees/<hive id>`. The id is slugged and a containment check is applied (index.ts:2733-2741).
- **Branch.** `agent/<slug(basename(path))>`, which is `agent/<agentId>` (git.ts:254-258).
- **Base.** The current branch of the origin repo, otherwise `main` (index.ts:2743-2744).
- **Command.** `git worktree add <path> -b agent/<id> <base>`. On failure it retries `git worktree add <path> <base>` (git.ts:263-273). That fallback does not reuse an existing `agent/<id>`. It tries to check out the base branch, which git refuses if the base is already checked out elsewhere (standard git behaviour, not tested here).
- **Non-repo cwd.** Isolation is skipped silently (index.ts:2730). No warning is emitted from that block.
- **Known gap**, from the code's own note: "NOTE (tracked, not yet hardened)… reuse-existing-worktree handling here is the follow-up" (index.ts:2725-2729).
- **Dependencies.** `linkWorktreeDeps` symlinks the base checkout's `node_modules` into the worktree (worktreeDeps.ts:14-32; `junction` on Windows, line 27). No install step runs. Consequence (inferred): a branch that changes dependencies changes the base checkout's `node_modules`, and two branches cannot pin different versions.

### 5.3 Cleanup: three paths

1. **Normal agent (tab closed or killed).** `teardownPty` runs `git worktree remove --force` at once (index.ts:486-491; git.ts:277-283). Uncommitted and untracked edits in the worktree are deleted. Commits survive on `agent/<id>`. The kill confirm says only "Close {{name}}? The PTY process will terminate and the agent is archived (kept in history, off the floor)." (en.json:152; AgentDetailPanel.tsx `onKill`). The code's comment at index.ts:482 reads: "Normal agents keep the immediate force-remove." The renderer confirm does not mention the worktree.
2. **Ephemeral worker (done, reaped or stopped).** `finalizeWorkerWorktree` (index.ts:528-566) unlinks the dependency link, then keeps the worktree if `worktreeHasUnintegratedWork` is true: dirty, or commits ahead of base, or the check failed (git.ts:291-312). Kept worktrees are tracked and god is told. Otherwise the worktree is removed and tracked for scratch-directory cleanup.
3. **GC sweep** (index.ts:4847-4884). Runs at most once a minute (index.ts:4836-4837). Reclaims a preserved worktree only if `worktreeIsGcSafe` holds: clean, and either zero commits ahead of base or an empty `git diff --quiet base HEAD` (git.ts:332-351). The second test catches squash merges. Reclaiming sends god a note.

### 5.4 Merging and conflicts

- **No automatic merge exists.** The only `merge-base` use is in compare (git.ts:463).
- **God is the "sole integrator"** (index.ts:383, 543). Its prompt lists "branch integration" as its job (hive.ts:1586).
- **Conflicts** get no special handling. "Unresolvable conflicts" is one of god's critical escalation categories (hive.ts:1586-1587).
- **Human tools.** A Git tab with HISTORY and COMPARE. The checkout IPC refuses while any PTY in that repo produced output in the last 10 s (index.ts:3446-3471), and `checkoutRef` requires a clean tree (git.ts:492-511).

### 5.5 Hive repo commit policy

`commit()` (hive.ts:2893-2909) runs `git add -A` then `git commit -q -m <message>`, up to 5 attempts, with stale-lock cleanup. Every state change commits, including routing and session capture. Section 1.2 covers the cost.

---

## 6. Recovery and resumption

### 6.1 App restart

`bootstrapHiveServices` (index.ts:5108-5167) runs on boot. In order:

- `ensureHive`, refresh the generated docs, record runtime info, and write an `app-start` event (index.ts:5122-5135).
- `archiveOrphanedAgents` (index.ts:5137; 906-920). Any agent with no live PTY is archived, except god.
- Message router (index.ts:5138), spawn-request poller (index.ts:5139), hook server (index.ts:5154), missions (index.ts:5146-5148), telemetry, memory and reflector (index.ts:5159-5164), and always-on beats (index.ts:5166).

Roster source: `roster.json` is the restore source, and localStorage is the fallback (roster.ts:6-21). `store/rosterSource.ts` stops one hive's team from appearing in another.

### 6.2 Team restore

`useRestoreTeam.ts`:

- **Auto restore** fires 2.5 s after the restorable list appears (useRestoreTeam.ts:33, 221-247). Only agents that had a terminal open at last quit are included.
- **Concurrency.** Spawns run in parallel (line 97) but are added in roster order (lines 193-195).
- **Worktree re-entry.** If the saved worktree still exists it becomes the cwd, with `isolate: false`. If not, the base checkout is used and the card says so (lines 122-131).
- **Resume.** `resume: true` makes main look up `hive.lastSession(id)` (index.ts:2872). For Claude it copies the transcript into the cwd's project folder first, and attaches `--resume` only if the transcript exists (index.ts:2868-2875). For other CLIs the resume flag or subcommand is used (index.ts:2896-2934). Codex finds the CODEX_HOME that owns the session (index.ts:2918).
- **Already running.** An `already exists` error counts as alive and the agent leaves the restorable list (useRestoreTeam.ts:174-179).
- **Restored workers become normal agents.** "ephemerality is a property of the hiring, not of the card" (index.ts:4786-4789).

### 6.3 Session identity

- The session id is captured from hook payloads (hooks.ts:431 calling hive.ts:1208-1221, written only when it changes). It is the `--resume` key and the cost dedupe key (hive.ts:172-176).
- Statusline ticks do not record sessions (hooks.ts:395-400).
- Bug fixed in code: `ensureAgent` used to wipe the session id. The fix spreads the prior entry (hive.ts:772-776).

### 6.4 Crash diagnostics

Abnormal exits (non-zero code or any signal) append a structured event to `log.jsonl`, and the raw PTY tail goes to `crashes/` (hive.ts:2696-2747). Files are mode 0600 and capped at 50 (hive.ts:376, 2733). They are gitignored because "a secret written there would be permanent" (hive.ts:651-655).

### 6.5 Power resume and PTY liveness

`healthCheckPtys` probes each PTY pid with signal 0 and logs or notifies dead ones (index.ts:5310-5330). The comment says auto-revive is renderer-owned (index.ts:5302-5309). The renderer's auto-revive effect (useHive.ts:328-332, "effect #7") was not read.

### 6.6 Closing time (graceful shutdown)

Protocol (closingTime.ts:1-25):

1. The human presses "closing time".
2. The harness mails god a brief. The brief tells god to broadcast closing time, have each worker park WIP, update `memory.md`, and send a message with subject `CLOSING-TIME-ACK` (closingTime.ts:112-130).
3. God waits for every ACK, saves its own state, and sends `CLOSING-TIME-COMPLETE` (closingTime.ts:123).
4. The harness watches the router and quits.

Harness checks:

- **Verified ACKs.** "the harness independently verifies the ACKs and will reject a premature conclusion" (closingTime.ts:123). A COMPLETE from god with missing ACKs gets a `refuse` mail naming the workers (closingTime.ts:184-209). Workers whose terminals died are excused (closingTime.ts:189-193).
- **Interrupt.** Steer notes reach busy agents at their next hook boundary (closingTime.ts:138-143; hooks.ts:526-528).
- **Timeouts.** Show "taking long" after 6 min (closingTime.ts:48). A 2.5 s grace after COMPLETE before teardown (closingTime.ts:51, 213-216).
- **No god, no protocol.** The UI falls back to a hard quit (closingTime.ts:93-95).
- **Cancel.** Clears queued steers and tells god to disregard the brief (closingTime.ts:151-168).

### 6.7 Idle detection

- Hook status is authoritative. A provider-agnostic fallback flips a `working` agent to idle after 12 s of PTY silence (useHive.ts:637-676). The comment explains why: "A non-Claude bridge that fires a 'working' event but never its turn-end signal (Stop / session.idle / agent_end) would pin the agent 'working' forever" (useHive.ts:35-37).
- Worker reaping: idle 20 min by PTY silence (main/config.ts:440; index.ts:4969-4981), token cap (index.ts:4951-4968), and done (index.ts:4911-4950).

### 6.8 Hook socket health

- The socket is verified every 15 s (index.ts:5173, 5291-5292). Re-binding uses backoff `[500, 1000, 2000, 4000, 8000]` ms and never takes over a live stranger's socket (hooks.ts:83, 187-211, 213-256, 315-337).
- Health is written into `fleet.json` (hooks.ts:55-78; index.ts:1325). The comment explains why: a dead socket "looks exactly like 'no workers have spawned yet'" (index.ts:5287-5289).

---

## 7. Hiring

### 7.1 By a human: Add Agent

AddAgentModal.tsx (lines 191-241):

- **Name** defaults to `Jim` (line 191).
- **Character** comes from the cast (cast.ts). Fifteen names: michael, jim, pam, dwight, kevin, angela, oscar, stanley, phyllis, andy, kelly, ryan, toby, creed, meredith (cast.ts:9-12, 27-46). **Cosmetic only**: avatar, portrait and shirt colour. Behaviour comes from `identity.md`, the role, the goal, the capabilities and the protocol (hive.ts:1494-1506, 1596-1613).
- **Accent**, **provider** (default taken from `defaultCommand`, lines 186-189), **model** (per-provider default in `providerDefaultModels`, line 223), **command** (editable, tokenized), **description** (the role, line 205), **goal** (standing briefing, line 237), **isolate** (default off, line 238), **resume session id** (line 241).
- **Templates**: five briefing templates, including repo janitor, docs writer and bug triager (AddAgentModal.tsx:54-80), plus a "Generate a hire with AI" copy-paste prompt (lines 82-116).
- **Token cap** from a manifest is applied after spawn (lines 493-495).

### 7.2 Shareable hire manifests

- **Spec** `munder-difflin/hire@1` (shared/hire.ts:27). Fields include `name`, `description`, `goal`, `character`, `accent`, `provider`, `model`, `commandFlags`, `capabilities`, `isolate`, `tokenCap`, `skills`, `mcpServers` (shared/hire.ts:43-82).
- **Security model** (shared/hire.ts:9-19): "It can NEVER auto-spawn an agent. Importing only pre-fills the Add-Agent modal; the human reviews the final command and clicks spawn."
  - No raw command. The binary always comes from the local provider preset.
  - Flags are default-deny. The allowlist is `--model`, `--max-turns`, `--output-format`, `--verbose` (shared/hire.ts:139-144).
  - Skills and MCP servers are references into bundled allowlists. Write or secret MCP servers need explicit consent and are never auto-enabled (shared/hire.ts:286-307).
- **Transport** (main/hire.ts): deep link `munderdifflin://hire?src=<https URL>` (shared/hire.ts:322-343). HTTPS only, except loopback. Fetch has a 10 s timeout and a 64 KB cap (main/hire.ts:155; shared/hire.ts:95). Redirects are followed by hand and must stay HTTPS (main/hire.ts:175-193). Every resolved address is checked against private, loopback and link-local ranges (main/hire.ts:27-53, 123-144).
- **Review queue** (shared/hireQueue.ts:16-33). Validated manifests wait in a queue and the human reviews them one at a time.

### 7.3 Hired by god: ephemeral workers

- **Gate**: `orchestratorMaySpawn`, default off (main/config.ts:428; index.ts:4997-5005).
- **Request**: one JSON file in `spawn-requests/` with `objective`, `cwd`, and optional `name`, `command`, `provider`, `model`, `isolate`, `tokenCap`, `slack`, `character`, `accent` (index.ts:4591-4615; PROTOCOL: hive.ts:3054-3082).
- **Launch**: `buildWorkerLaunch` turns the request into an executable and argv, including auto-mode inheritance (workerLaunch.ts).
- **Limits**: `maxConcurrentWorkers` 4 and `workerIdleTimeoutMinutes` 20 (main/config.ts:439-440). The poll runs every 1.5 s (index.ts:4618).
- **Id and dispatch**: `worker-<reqId>` (index.ts:4709-4711). The objective is sent as a request. Its tail says: "[WORKER COMPLETION] When finished, signal done by sending ONE outbox message to god with "act":"done"" (index.ts:4818).
- **Done detection**: an outbox `act: 'done'` whose `created_at` is later than the spawn time (index.ts:4658-4682). See 9.5.
- **Failures** archive the request to `.failed/` with the reason and inform god (index.ts:4701-4704).

PROTOCOL.md says this is "the ONLY spawn route you can complete on your own: a hire manifest under `research/hires/` needs the human to confirm it in the UI" (hive.ts:3077-3078).

### 7.4 Provider choice

Thirteen presets (agentProvider.ts): `claude`, `codex`, `grok`, `kimi`, `gemini`, `antigravity`, `qwen`, `opencode`, `crush`, `pi`, `copilot`, `cursor`, `custom`. The god's engine is set by `godProvider` and `godModel` (renderer store/config.ts:77-78; useHive.ts:399-400).

### 7.5 Role durability

- `role` is the durable job string. Live captions such as "standby" are rejected (agentRole.ts:10, 23-35). The file says: "Live run-state belongs on `status` / `action` — never on role" (agentRole.ts:4-6).

### 7.6 Provider capability matrix

From agentProvider.ts and hive.ts. "Mail path" means how a hive message reaches the agent (section 4.3).

| Provider | Status and turn signal | Protocol at launch | Mail path | Resume |
|---|---|---|---|---|
| claude | `--settings` hooks: Stop, PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, PreCompact, PostCompact, plus statusLine (hive.ts:1245-1308) | `--append-system-prompt` (hive.ts:994) | Inbox file, renderer nudge, watchdog | `--resume <sid>` with transcript copy (index.ts:2872-2875) |
| codex | Per-agent `CODEX_HOME/config.toml` `[[hooks.*]]` with `timeout = 30` (hive.ts:2193-2271) | Positional prompt | Inbox file, renderer nudge (agentProvider.ts:232) | `codex resume <sid>` (index.ts:2909-2931) |
| grok | Global `~/.grok/hooks/munder-hive.json` (hive.ts:2515-2547) | Positional (agentProvider.ts:264) | Inbox file, renderer nudge (262) | `--resume` (266) |
| kimi | None | None (hive.ts:974 returns bare args) | Terminal work order, because `canReceiveInbox: false` (283) | None |
| gemini | Per-agent system settings file, `GEMINI_CLI_SYSTEM_SETTINGS_PATH` (hive.ts:2140-2172, 913) | `-i` (302) | Inbox file, renderer nudge (300) | `--resume` (304) |
| antigravity (agy) | Global `~/.gemini/config/hooks.json` and `~/.gemini/antigravity-cli/hooks.json` (hive.ts:2095-2131) | `-i` (320) | Inbox file, renderer nudge (319) | `--conversation` (322) |
| qwen | Loopback proxy tees the LLM traffic and synthesizes events; a Stop fires after 800 ms of quiet (hive.ts:3328-3541, 3362-3370). Routed by its bridge | `-i` (343) | Terminal work order (`inboxDelivery: 'terminal'`, agentProvider.ts:340; hive.ts:1757-1763) | None |
| opencode | Per-agent plugin posts events on `session.idle` and tool calls (hive.ts:2429-2459, 3291-3316). Marked LIVE-UNVERIFIED in the code | `--prompt` (380) | Inbox file, renderer nudge (379) | None |
| crush | Proxy plus `CRUSH_GLOBAL_CONFIG` base URL override (hive.ts:940-951, 2476-2503). Marked LIVE-UNVERIFIED | None. Seed typed into the TUI (agentProvider.ts:457; useHive.ts:740-760) | Terminal work order (agentProvider.ts:441) | `--session` (458) |
| pi | Per-agent extension in `PI_CODING_AGENT_DIR` (hive.ts:2392-2416, 3232-3282). Marked LIVE-UNVERIFIED | Positional (495) | Inbox file, renderer nudge (491) | `--session` (496) |
| copilot | None | `-p` (520) | Terminal work order (`canReceiveInbox: false`, 527) | `--resume` (524) |
| cursor | None | Type-into-TUI seed (559) | Inbox file, renderer nudge (554). No bridge, so the wake relies on the quiet fallback (inferred) | `--resume` (561) |
| custom | None | None | Terminal work order, or bounce to god when no PTY (579) | None |

---

## 8. Architecture notes worth keeping

- **The hook shim is a dumb pipe.** Each hook forwards its JSON to the socket and relays the response (hive.ts:3111-3158). All logic lives in the main process. Good design.
- **Per-agent config isolation by environment**: `CODEX_HOME`, `GEMINI_CLI_SYSTEM_SETTINGS_PATH`, `OPENCODE_CONFIG_DIR`, `PI_CODING_AGENT_DIR`, `CRUSH_GLOBAL_CONFIG` (hive.ts:874, 913, 908, 898, 948-949). The Claude hook settings are per session, but the Claude pre-accept step still writes global files (9.6).
- **Scoping by agent id in global hooks.** Global hook files (agy, grok) are inert unless `AGENT_ID` is set (hive.ts:2512-2514, 3169-3170).
- **Proxy-tier sidecar** (hive-proxy.cjs). Tee LLM traffic on loopback, and never log bodies (hive.ts:3318-3327). Idle is a heuristic: a turn with no tool call and no new request for 800 ms counts as a Stop (hive.ts:3362-3370).
- **Timeout units differ per CLI.** Codex reads `timeout` as seconds and floors `0` to one second, which broke every codex worker's SessionStart (hive.ts:2232-2245).
- **Redaction at the boundary**, with a lockstep test (hive.ts:270-274).
- **Main owns most state, but not all of it.** The renderer owns the floor roster and the per-agent message queues (persisted to localStorage and mirrored to roster.json), and makes the delivery decisions (useHive.ts; store.ts). Tasks are polled from `tasks.json` every 5 s (TasksKanban.tsx).

---

## 9. Risks found while reading (verify before relying on them)

None of these was reproduced. Each follows from the code as written.

### 9.1 Agent-chosen message id becomes a file path (high)

- `normalize()` keeps the sender's id: `id: partial.id ?? \`${stamp()}-${shortRand()}\`` (hive.ts:1622).
- `deliver()` writes `join(inbox, \`${msg.id}.json\`)` with no validation (hive.ts:1647-1652, 1650).
- `join` resolves `..` segments. An id with `..` writes a `.json` file, containing the message, outside the inbox, into any existing directory the app can write to. The router runs in the main process, which is not sandboxed.
- The same unchecked id flows through `bounceToGod` (hive.ts:1659-1668).
- PROTOCOL says "The harness fills in `id`, `from`, `hops`, and timestamps" (hive.ts:2987). The code does not do that for `id`.
- **Fix:** generate ids in main, and validate any agent-supplied value against a strict pattern.

### 9.2 Corrupt state reads as empty and is written back (high)

- `readJson` returns the fallback on any parse error (hive.ts:2794-2796).
- `registry()` falls back to `{godId: null, agents: {}}` (hive.ts:1897-1901).
- `ensureAgent` reads the registry, adds one agent, and writes the whole thing back (hive.ts:749-794). If the file is unreadable at that moment, the write leaves only that agent. Session ids for every other agent are lost, which breaks `--resume`.
- `tasks()` falls back to `{tasks: []}` (hive.ts:1906-1908). `addTask` reads through that fallback (hive.ts:1939-1945) and `writeTasks` merges against another read that falls back to empty (hive.ts:1924-1934). A single `addTask` on an invalid `tasks.json` therefore writes only the new card. Webhook, Slack promotion and voice add cards this way. God edits the file by hand, so an invalid hand edit followed by any of these adds wipes the board.
- `writeJson` is a plain `writeFileSync` (hive.ts:2797-2799). It is used for `tasks.json` (hive.ts:1931) and for registry updates (hive.ts:1023, 1141, 1175), so a crash mid-write can leave a torn file.
- Contrast: roster.ts refuses to overwrite a non-empty roster with an empty one and keeps backups.
- **Fix:** fail closed (quarantine the bad file and keep the last good state in memory), write temp then fsync then rename everywhere, and never write back a value that came from a failed read.

### 9.3 Normal-agent worktrees are force-removed on close (high)

- Closing a tab or killing a normal agent calls `git worktree remove --force` at once (index.ts:486-491; git.ts:277-283).
- Uncommitted and untracked work in that worktree is deleted. Commits remain on `agent/<id>`.
- The renderer confirm does not mention this (en.json:152).

### 9.4 The sandbox does not enforce the single-writer rule (medium)

The sandbox allow-list includes the whole hive root (hive.ts:1240-1243, 1288-1293). An agent can therefore write directly into another agent's inbox. Inferred from the allow-list; not tested.

### 9.5 Timestamps and ids come from the agent (medium)

- `created_at` is kept from the sender (hive.ts:1640).
- The done-signal logic compares that timestamp to the spawn time (index.ts:4673-4677). A sender can forge the time.
- `settleInbox` cut-off uses the same timestamps (hive.ts:1093-1095).

### 9.6 Global user config is modified (medium)

- Claude: `~/.claude/settings.json` (global permissions) and `~/.claude.json` (project trust), written before each spawn (config.ts:799-800, 826, 876-885; index.ts:2955).
- Codex: each agent's `sessions/` and `archived_sessions/` are moved under `~/.codex/<kind>/munder-difflin/<hash>/<agent>` and symlinked back (hive.ts:2273-2347, 2302-2304). The code carries cross-device (EXDEV) and rollback handling for these moves (hive.ts:2280-2288, 2339-2346).
- Grok and agy: global hook files (hive.ts:2119-2120, 2539-2543). These stay after uninstall.

### 9.7 Stale documentation

- HIVE.md describes the Stop-block drain (HIVE.md:59-61, 137-141), which the code retired.
- HIVE.md says to write hooks to `<cwd>/.claude/settings.local.json` (HIVE.md:207). The code uses a per-session `--settings` file (hive.ts:1000-1004).
- HIVE.md's layout omits `spawn-requests/`, `roster.json`, `worktrees/`, `fleet.json` and `crashes/` (HIVE.md:69-83).

### 9.8 Mail has no receipts, and terminal replies are not captured

Covered in 4.4 and 4.5.

### 9.9 Growth and polling

- `log.jsonl` is never rotated, and `logTail` reads the whole file on every call (hive.ts:2663-2668). The Activity tab calls it every 3 s (CommandCenterPanel.tsx:1264-1266).
- Inbox and outbox folders are polled from both renderer and main (useHive.ts:692-696; ThreadsPanel.tsx:62; index.ts:5204-5271).
- Archived `.done/` and `.sent/` folders have no pruning policy.

### 9.10 Silent isolation fallback

Non-repo cwd: isolation is skipped with no warning (index.ts:2730).

### 9.11 Duplicated constant with a sync test

`MINE_IGNORE_LINES` is defined in both hive.ts:239 and memory.ts:33, with the comment "MUST STAY IN SYNC". test/mine-ignore-sync.test.cjs is there to catch drift between the two copies.

### 9.12 Nested .gitignore also hides messages from git (inferred)

The per-agent `.gitignore` lines `inbox/` and `outbox/` (hive.ts:239) also apply to git, because git honours nested ignore files. The test header describes the list as "kept out of an agent's mempalace/git index" (test/mine-ignore-sync.test.cjs:1-10). So message bodies are probably not in hive git history. Confirm with `git check-ignore -v` in a real hive before relying on git as the audit trail.

---

## 10. What to steal and what to do better (for the Rust/Tauri port)

### 10.1 Steal (keep as is or nearly so)

1. **Files as the agent-facing protocol.** One JSON file per message in `inbox/` and `outbox/`, archives in `.done/` and `.sent/`, one writer per folder. Any CLI can use it and a human can inspect it (hive.ts:1842-1893; PROTOCOL hive.ts:2955-3100).
2. **Hook socket design.** Newline-framed JSON with a frame cap (hooks.ts:24-25), an ownership probe that never steals a live socket (hooks.ts:315-337), health reported into the snapshot (hooks.ts:55-78), and re-bind with backoff (hooks.ts:83, 187-256).
3. **Hook return values as control.** Deny for pause and gates (hooks.ts:506-519), `continue: false` for halt (hooks.ts:424-427), `additionalContext` for steer, roster and goals (hooks.ts:521-580). Control without typing.
4. **Live roster injected into the orchestrator on each prompt**, with a context-percentage column and an on-hold marker (hive.ts:2588-2662; hooks.ts:536-543).
5. **Per-session injection.** `--settings`, `--append-system-prompt` and environment-scoped homes. The pre-accept step is the exception (9.6) (hive.ts:994-1004, 874, 898, 908, 913).
6. **Exact context usage from the Claude statusLine** (hive.ts:1277; hooks.ts:401-419).
7. **Circuit breaker ladder.** steer, then constrain, then stop; one level per beat; de-escalate on health; `hardStop` off by default (breaker.ts:1-22).
8. **Roster durability.** Backup before every write, append-only backups, temp-then-rename, and refusal to overwrite a non-empty roster with an empty one until a non-empty write has landed (roster.ts:117-169).
9. **Worktree safety predicates.** Dirty, ahead or unknown means keep. Squash-merge detection by tree equality (git.ts:291-351).
10. **Hire manifests.** No raw commands, allowlisted flags and references, human confirmation before spawn, SSRF-guarded fetch (shared/hire.ts; main/hire.ts).
11. **Closing time as a verified ACK ledger** with a timeout and a grace period (closingTime.ts:175-218). In Rust, the harness should send the brief itself and track ACKs itself, so that correctness does not depend on the orchestrator's LLM.
12. **Settle on release.** File unread mail up to the done signal, keep later mail pending, and report unanswered requests (hive.ts:1078-1113).
13. **Crash tail capture.** Structured log line, 0600 file, capped, gitignored (hive.ts:2696-2747).
14. **Prompt-cache-stable system prompt** (hive.ts:1511-1517).
15. **Capability broker for integrations.** Workers hold a per-worker token and never see the secret. The grant is revoked on teardown (integrationBroker.ts:1-22; index.ts:452).
16. **No silent drops.** Every failed route is logged and bounced to god (hive.ts:1659-1672, 1720-1739, 1769-1772).
17. **Schema migrations by `user_version`** (db.ts:91-102).

### 10.2 Do better

**A. Storage and integrity**

1. **Use SQLite (WAL) as the source of truth, with an append-only event table.** Keep files as the agent interface: materialize inbox files from rows, and ingest outbox files through validation. Export board, memory and tasks to git on a debounce or on checkpoint, not per mutation. This avoids the 7.5 GB problem (hive.ts:229-234).
2. **Generate every id and timestamp in the harness, and validate all agent text used in paths** (9.1, 9.5; hive.ts:1622, 1640, 1650).
3. **Fail closed on corrupt state.** Quarantine unreadable files, keep the last good state, and refuse writes that would shrink the record (9.2; hive.ts:1897-1908, 2794-2796).
4. **Atomic writes everywhere.** Temp, fsync, rename. No `writeFileSync` on live state (hive.ts:2797-2799 versus 2800-2804).
5. **Make removal reversible.** Keep dirty worktrees on close and ask first. Archive with a snapshot (index.ts:486-491).
6. **Enforce the single-writer rule in the sandbox.** Allow writes only to the agent's own folder and its outbox (hive.ts:1240-1243; 9.4).
7. **Real dependency gating.** A card is runnable only when every `dependsOn` is done. Validate state transitions (hive.ts:124; AskMeTab.tsx:43).
8. **Enforce the board's single scribe by capability,** not by prompt. Only the orchestrator's session gets write access to `board.md` (hive.ts:3000).
9. **One source of truth for the roster.** The codebase already documents the drift between `roster.json`, `registry.json` and localStorage (roster.ts:6-21).
10. **Rotate and index the log.** Stop re-reading `log.jsonl` on every poll (hive.ts:2663-2668; CommandCenterPanel.tsx:1264-1266).
11. **Delivery receipts.** Track `queued → delivered → read → answered` per message. Today only "delivered" is logged, as a per-message summary (hive.ts:1774).
12. **Harness-owned cursors and hops.** Do not compare ids as strings (hive.ts:1473; useHive.ts:284-288).

**B. Process and delivery**

13. **Make the app binary the hook shim.** The bundled-Node launcher, the `runtime/node` shim and the `ELECTRON_RUN_AS_NODE` tricks exist only because the shim is a Node script (hive.ts:483-595). A Rust `hook` subcommand removes the PATH problem.
14. **Typed per-adapter timeouts** that are tested against each CLI's own resolver (hive.ts:2232-2245).
15. **Adapter trait with explicit capabilities.** Use the matrix in 7.6: status source, prompt delivery, mail path, resume strategy and timeouts. Mark unverified adapters and run a runtime self-test, as the hook health beat does (hooks.ts:187-211).
16. **Reduce the wake machinery.** Today wake is five timers and guards (useHive.ts:637-955; index.ts:5204-5271; workerWake.ts). Two options to evaluate, both my suggestions and untested here:
    - inject mail at `UserPromptSubmit` or `SessionStart` through `additionalContext` for hook-aware CLIs, and type only when an agent is idle;
    - wake idle agents with a headless resumed turn instead of typing into the TUI. This needs a test of two processes on one session.
17. **Per-run nonce for worker done detection.** Include a nonce in the dispatch and echo it in the done message, instead of comparing timestamps (index.ts:4658-4682).
18. **Per-worktree dependencies.** Use copy-on-write clones (APFS `clonefile`, reflink on Linux) or a per-worktree install, not a shared `node_modules` symlink (worktreeDeps.ts:27).
19. **Integration as a first-class step.** An integration worktree merges agent branches in order, reports conflicts to the owning agent, and records the result. Today this is "god does it in its own session" (git.ts supplies the predicates).
20. **Do not mutate user-global CLI config** unless the CLI only reads global config, and then make the change explicit, reversible and visible (9.6).
21. **Watch the filesystem instead of polling.** Use `notify` for inbox and outbox, and one scheduler, not 1.5 to 4 s polls from both renderer and main (hive.ts:1832; useHive.ts:692; CommandCenterPanel.tsx:1264).
22. **Approvals in the app.** The code only denies. Consider answering `PreToolUse` with an app-side decision, so approvals appear in the app, and keep Claude's native prompt as the fallback (hooks.ts:502-505).

**C. Product**

23. **Keep personas cosmetic.** The office cast is avatar only (cast.ts). Behaviour comes from identity, role, goal and protocol. Say so in the UI.
24. **Version the orchestrator prompt and test it.** The contract, the sole-scribe rule and the assignee rule are in prompt text only (hive.ts:1586-1587). Some of them can be enforced by the harness instead.
25. **Ship the god-only dispatch path as the default,** as the Command Center already does (CommandCenterPanel.tsx:562-578).

---

## 11. Suggested shape for the Rust port (sketch, not evaluated)

- **Core inside the Tauri backend**, as tokio tasks: hive store (SQLite WAL with `user_version` migrations), mailbox materializer and outbox ingester, router, hook server (Unix socket on macOS and Linux, named pipe on Windows), PTY manager, worktree manager (shell out to `git`), and the scheduler for breaker, snapshot and reaper.
- **Frontend subscribes to events** instead of polling.
- **Candidate crates, not evaluated:** `tokio`, `notify`, `rusqlite` (bundled, WAL), `serde_json`, `portable-pty`, `interprocess` or tokio's Unix listener plus named pipes, `tracing`.
- **Keep the proven ideas from section 10.1**, and fix the items in section 9 before porting them.

---

## 12. Open questions to verify

1. Does the Claude sandbox `allowWrite` behave as the comment at hive.ts:1281-1285 says ("Verified live (claude 2.1.239)")? Not re-verified.
2. Are inbox and outbox really excluded from the hive git repo? Run `git check-ignore -v agents/<id>/inbox/x.json` in a real hive (9.12).
3. Reproduce 9.1 (path traversal) and 9.2 (empty write-back) with tests.
4. Codex hook discovery and timeout behaviour, using the resolver the code mentions (hive.ts:2243-2245).
5. The Pi, OpenCode and Crush bridges are marked LIVE-UNVERIFIED in the code's own comments. Treat them as unproven.
6. Does the renderer's auto-revive effect actually respawn dead PTYs with `--resume`? Only its comment was read (useHive.ts:328-332).

## 13. Coverage

- **Read in full:** src/main/hive.ts, src/shared/taskLedger.ts, broadcast.ts, hiveNudge.ts, agentRole.ts, godIdentity.ts, hire.ts, hireQueue.ts, src/main/worktreeDeps.ts, control.ts, roster.ts, closingTime.ts, db.ts, git.ts, src/main/hire.ts, src/main/hooks.ts, src/renderer/src/hooks/queueDelivery.ts, useRestoreTeam.ts, HIVE.md, resources/skills/md-hive-sync/SKILL.md, docs/research/v0.3.4-implementation-plan.md, docs/research/v0.3.5-feature-research.md.
- **Targeted ranges:** src/main/index.ts (about 2,000 of 5,511 lines: the coordination regions), useHive.ts (about 550 of 1,249), store.ts (queue parts), TasksKanban.tsx, ThreadsPanel.tsx, MessageQueueComposer.tsx, CommandCenterPanel.tsx, HivePicker.tsx, WorkersTab.tsx (header), AddAgentModal.tsx, AgentDetailPanel.tsx, AskMeTab.tsx, agentProvider.ts, breaker.ts (header), workerWake.ts (header), integrationBroker.ts (header), memory.ts (header), config.ts (parts), workerLaunch.ts, cast.ts.
- **Not read:** telemetry.ts, realtime*.ts, slack.ts, webhook.ts, reflect.ts, palaceReap.ts, cliInstall.ts, the test bodies (names only), hive/docs/integration-templates.md (header only), docs/research/v0.3.5-explainer.html, CHANGELOG.md.
- **Other observations, not coordination:** `config:update` accepts an unvalidated `Partial<HarnessConfig>` (index.ts:3201). The v0.3.5 research note flags it too.
