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
| Merge | serial merge queue into `codly/<plan>/integration`, `git merge-tree` pre-check; Codly merges into the user's branch and pushes it only when the user presses the button; a pull request (`gh`) is opened the same way | agents-backend §6.8 |
| Storage | one SQLite (rusqlite bundled, WAL); `event` table is the source of truth; raw provider lines in `runs/<id>/raw.ndjson`, deleted 30 days after the run's last line (Settings → Storage) | replayable timeline and world |
| Retrieval | per-repo FTS5 over paths, regex symbols and contextual 60-line chunks (path + enclosing symbols prefixed) → RRF → agreement confidence → **pointer pack** (≤ 1.2k tokens) + `codly_search` | commit-replay Hit@5 0.66–0.84 on 4 real repos vs 0.25–0.32 naive grep on 3 of them (0.69 on pi-mono); `examples/bench_localize.rs` |
| Memory | scoped memories (global / project = shared, agent = private, task), FTS5; temporal knowledge triples, many derived from real events | the user's "shared + single memory, triplets, RAG" |
| World | three.js (`src/office/`: engine, reducer, sims, traits, breakroom); characters and the building are drawn in code, furniture models and sounds are third-party CC0 / CC BY assets (`public/furniture`, `public/audio`, credits in each folder); A*, DOM text overlay; frames on demand: about 30 fps while the World is on screen (agents breathe and take breaks), none when nothing moves (Reduce Motion) | design-world §9; Munder's PixiJS floor never idles and ships non-redistributable LimeZu tiles |
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

`codly serve` (src-tauri/src/web.rs) is the same process with its window hidden and an HTTP
server on 127.0.0.1: `POST /api/cmd/<name>` runs a command through Tauri's own IPC dispatch on
that window (same handlers, same ACL), and the two Channels become server-sent events
(`/api/events` for the UI bus, `/api/pty/<id>` for a terminal). A token, printed in the link
and kept in the data dir, is traded once for an HttpOnly cookie. Pages carry the window's
content security policy and `nosniff`, no-referrer and no-framing headers. Another `--host` is
refused without `--allow-remote` (plain HTTP, and a page can open a terminal); a served-to-the-
network run gets a token of its own that is never kept. An app that is already open serves for
a second `codly serve`, but not to the network: nobody would see the link it prints.

`/api/voice/ws` is passed
through to an optional voice relay on the same machine (`voice/relay.py`, OpenAI Realtime);
without the relay the call fails with "the voice relay is not running". The relay takes a call
only from `codly serve`: it must carry the token the relay wrote for this user at its start
(`~/.config/codly/voice-<port>.token`) and no browser `Origin`.

Logs go to stdout and to `<data dir>/logs/codly.log` (src-tauri/src/logging.rs: 5 MB, one older
file kept; a panic hook writes the place and backtrace). Settings → Troubleshooting builds a
report from the shell side (src-tauri/src/diagnostics.rs), so it works when the core did not
start, and scrubs tokens, keys, emails and paths under the home folder from it (repository and
agent names stay). The file takes debug lines from Codly's own crates only (`CODLY_LOG=debug`):
a library's debug output stays on stdout.

PATH: GUI apps get launchd's minimal PATH, so Codly captures the login shell env once
(`$SHELL -ilc '/usr/bin/env -0'`, 5 s timeout) and resolves each CLI on it at every spawn.
Child envs are scrubbed of variables that silently change billing or session behaviour
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `CLAUDE_CODE_*`…), unless the user turned on their own
API key for that vendor (`byok.rs`).

Credentials. Sign-in state comes from the CLI's own status command (`claude auth status`,
`codex login status`) or settings file (Gemini: sign-in method and email). Codly does not read the
CLIs' sign-in tokens, with one exception that is off by default: `Settings.read_claude_quota`
(Settings → Accounts, "Show Claude usage before an agent runs"). With it on, a refresh of the
accounts makes `quota.rs` read the Claude Code OAuth token (login Keychain, else the sign-in's
`.credentials.json`) and send it to `https://api.anthropic.com/api/oauth/usage`, as `/usage`
does. The token goes to curl on stdin and is not stored or logged. `detect_clis`, which the
onboarding polls, never does this. Keys the user gives Codly live in the Keychain: API keys
(`byok.rs`), the Groq key (`dictation.rs`), webhook secrets and Slack tokens
(`inbound/secrets.rs`) and keys of agents' tool servers (`housekeeping/connections.rs`).

## 3. One event stream, two views

Every adapter normalizes its provider protocol into `EventKind` (`crates/codly-core/src/model.rs`,
mirrored to TypeScript in `src/lib/gen/`). The orchestrator adds domain events (task, plan,
decision, commit, verify, merge, memory, triple). Each event is persisted with a global id
(the UI cursor) and broadcast; streaming deltas are ephemeral (id 0). So is an agent's change
of activity or usage at each tool call: its row and the UI follow it, the log does not keep it.

- **Workspace**: chat threads (Lead + 1:1 per agent), per-agent timeline lanes, shared board +
  each agent's own todo list, task DAG, diffs, terminal, approvals inbox, inspector.
- **World** (`src/office/`): a pure reducer (`reducer.ts`) over the same events → per-agent activity →
  locomotion and poses in the three.js engine (`engine.ts`), plus `sims.ts` (needs, matchmaking),
  `traits.ts` and the break room. The building is the repo's **lot** (core `lot:{ws}`, `crate::lot`):
  walls, doors, windows, floor tiles and furniture as data; `layout.ts` finds the rooms and turns
  furniture with a job (desks, racks, shelves, the mailbox, archive cabinets) into stations. A
  repo's first lot is the Studio (`src/office/lots/studio.json`) if it has history, else the Garage.
  Honesty contract: an agent's work motion comes from a real event (≤ 250 ms from event to
  visible state) and every bubble has provenance. Ambient life is not an event: characters
  breathe and glance, and an idle agent takes breaks in the break room (`breakroom.ts`,
  `sims.ts`).
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
          │ codly_submit_report (or a completed turn's end)
          ▼
       gate: Codly commits → runs verify → scope check → [reviewer, other vendor] → fix loop ≤ 2
          ▼
       merge queue (serial): merge-tree pre-check → merge → dependents become ready
          ▼
       plan done → Lead gets a status turn → summary to you → "Merge into <branch>" / keep branch
```

**Your branch stays yours.** A merge into your checkout is refused while you are in the middle
of your own merge, rebase or cherry-pick, and asks first when the checked-out branch is not the
one the plan started from. The last merge Codly made there can be taken back with "Undo merge"
(`git reset --keep` to the commit before it) while that loses nothing: the branch is still at
that merge commit, no remote branch has it, and no tracked file has uncommitted changes. The
merged work always stays on a branch: if its branch was deleted since, the undo puts it back.

Every routing, scheduling, gate and merge choice is written to the `decision` table and
emitted as an event, so "Why did Codex get this task?" always has an answer.

**Only a turn that completed is gated.** A worker turn that ends on an error or at its turn limit
is not finished work: Codly asks the same session to continue (after 15 s, then 60 s), and a third
failure in a row blocks the task with the error. If you resume the worker, or a turn that answers
a message completes, before that wait is over, the worker is asked to continue then: neither is
gated. A worker that crashes mid-turn, or that does not start, is started again after a wait (5 s,
then 30 s); its third failure in a row blocks the task, and so does, at once, a cause only you
can fix (a CLI that is not installed or not signed in). A turn the worker completes ends the row,
time does not. A session that ran out of context starts
again as a new session. A worker the circuit breaker constrains is paused
until you resume it (one you hold already reads why on your Resume); a breaker stop says so on the
task.

**No agent sits "running" without end.** A watchdog looks at every live run each 15 s. A turn that
gives no event for 10 minutes (60 with a tool call open; `Settings.watchdog`) is flagged on its
task ("stalled: no output for 10 min") and you are notified. At twice that its turn is interrupted
and the agent is asked to continue, once. Silent for as long again, the run is stopped: a worker's
task goes back in line (blocked from its third start on), a Lead or a 1:1 agent resumes with your
next message. A turn that waits for your answer is not silent. A worker that sits between turns
for 2 minutes on a task that still runs (you interrupted it) is flagged as idle; nothing is done
to it. Below that, each adapter gives Stop a limit: a turn that has not ended 8 s (Claude) or 15 s
(Codex, ACP agents) after the interrupt request loses its process (or, on the shared Codex server,
its thread) and the session resumes with the next run. That exit is no crash, and who asked for
the interrupt says what is next: after the watchdog's, the work goes back in line as above; after
yours, the circuit breaker's or a tool block's, a worker's task waits for you, Blocked, and
nothing restarts it. A write to a CLI that stopped reading its stdin gives up after 10 s and ends
that process.

**A plan that cannot go on says so.** When nothing of a running plan runs or can start (a task is
blocked or failed and the rest waits for it), Codly says so once in the plan's thread. You can
retry the task, cancel it together with the tasks that wait for it, or keep what is merged: the
plan then finishes with its merged tasks. After a restart of Codly, a task that was in its checks,
review or merge is checked again with no worker turn.

**A budget stops spend.** Settings → Autonomy & Budgets has a token budget and a cost budget for
all agents together (both off by default). What the team used is counted from every usage report
and saved with it, so quitting does not give a budget back; only Reset counters does. Past a
budget the scheduler starts no task (each waiting task says which budget and the way out), agents
in a turn are steered and then constrained, agents with no turn running are left alone, and you
get one message. A budget alone never kills an agent, also with Hard stop: a kill is for a limit
the agent crossed itself. The Lead is exempt: it only talks to you. Cost is what the CLIs report
(Claude Code: an estimate at list price; Codex and Gemini: none). Every run reports its own cost,
from zero: an ACP agent (OpenCode) reports a session's running total, so a run that resumes a
session is told what the earlier runs cost (`SpawnSpec::session_cost_usd`) and the adapter takes
it off. Codly does not pass a spend cap to the CLIs themselves.

**Parallel requests.** Each request is its own thread (`lead`, `lead:<id>`) with its own Lead
session ("Lead", "Lead 2"…, at most 3 live; idle ones pause and resume by session id). Plans,
follow-ups and workers' questions stay in their request's thread; every Lead's board header
shows the other requests' live plans and files so they don't plan conflicting changes. Write
scopes are leased globally, so parallel plans never edit the same files at once.

**Several accounts per CLI.** Each sign-in is its own config dir (`CLAUDE_CONFIG_DIR`,
`CODEX_HOME`, `GEMINI_CLI_HOME`; Codex gets one app-server per `CODEX_HOME`), listed in
Settings → Accounts. Agents' `RateLimit` events feed each account's usage windows (persisted);
a refresh also asks each signed-in Codex account's app-server for them (`account/rateLimits/read`:
Codex reads them with its own sign-in, Codly sees no credential), and with `read_claude_quota`
on it asks Anthropic for the windows of each Claude sign-in Codly may use (§2, Credentials).
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
strict (ask for everything), autonomous (allow what shows no risk; the rules read the command line,
not what a script or program then does, so this is not a sandbox) and **Full access**
(auto-approve everything, risky commands included, except the hard floors: questions, permission
changes, changes to the agent's own CLI settings, writes by read-only agents and Codly's own data
dir still ask).

**Saved rules** (`orchestrator/approval_rules.rs`): "Always allow in this repo" on an approval card
saves a rule in Codly's store (table `approval_rule`; not in the repo, not in a worktree) that the
broker follows for every agent of that repo. A rule is the exact command, or the first words of one
plain command (`cargo test`): a longer line that chains, pipes or substitutes anything never matches
a prefix, and neither does a second target (`make docs deploy`) or a form that reaches past the
agent's own worktree and branch (`git checkout -B main`). A rule covers a package download into
the project and a git command that changes the repository (the exact command also covers other
network use); deletes, `git push`, `sudo`, a pipe into a shell, secrets, paths outside the
worktree (an install for the whole machine too: `npm install -g`), a publish, a change to your own
account or settings (`cargo login`) and what cannot be checked always ask, and so do strict mode
and read-only agents. Settings → Safety lists the rules; removing one makes Codly ask again.

**Triggers and inbound** (Settings → Triggers / Inbound). `triggers.rs` holds the pure policy and
`orchestrator/triggers.rs` the loop: schedules that send a prompt to the Lead on a repeating clock
and a network mode that gates every
inbound path. `inbound/` has the sources: a loopback webhook listener (one secret per endpoint,
created switched off) and Slack over Socket Mode (config and gating exist; the socket is not
connected yet). An inbound message is never the user's: the thread stores and shows it under its
source's name, the Lead reads it as quoted untrusted data, and a plan made after it waits for Run plan
unless you let that endpoint run plans without asking (off for every new endpoint). Your next message
in the thread, or a schedule of yours that fires there, ends the wait for the plans that follow. One
agent picked to answer instead of the Lead makes no plan, so this gate does not cover it. Secrets live in the
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

What the code index covers: every text file that `.gitignore`, `.ignore` and `.codlyignore`
(same syntax, for the index only) do not ignore, except lockfiles, binaries, minified bundles
and files whose header says a tool generated them. Directories that are never source
(`.git`, `node_modules`, tool caches) are always left out. Directories named `vendor`, `dist`,
`target`, `coverage`, `venv`, `Pods`, `.build` or `.cache` are left out by default, because
they are build output or third-party code in most repos. If one holds your own code, add a
`!` rule for it to `.codlyignore`, for example `!/src/vendor/`.

## 6. Safety

- Agents run unmodified CLIs signed in by you. Codly does not read their sign-in tokens unless
  you turn on `read_claude_quota`; keys you give Codly are in the Keychain (§2, Credentials).
- Worktree isolation; Codex `workspace-write` sandbox (its network stays closed unless you
  turn on `codex_network_access`); Claude workers in `acceptEdits` with
  shell commands routed to the broker; Lead and reviewers read-only. The exception is an agent
  hired with Git isolation off: it edits the user's checkout directly (the wizard warns).
- Approval broker: rules (path outside worktree, symlinks followed; `rm -rf`, `git push`,
  `curl | sh`, `sudo`, `npx`/`uvx` downloads, `printenv` and `.env` reads, changes to an agent's
  own CLI settings, git inside a nested repository → ask you, except in Full access, which
  approves them), then you. No silent timeouts; choices mirror the provider's. The rules read
  the command line only: what a test, a script or an interpreter does once it runs is not
  checked (only Codex has an OS sandbox).
- The gate runs the plan's verify and integration commands (the Lead wrote them) through the
  same risk rules: one with a risk is not run until you allow it on its task (an integration
  check is refused: the plan is still done, and its message names the check to run yourself).
  Full access runs them. The rules read the command's text only: a test runner still runs the
  worker's code, unsandboxed.
- A new worktree (`worktree_setup.rs`) gets, once: copies of the checkout's git-ignored files
  that the built-in list (`node_modules`, `.venv`, `venv`, root `.env` files) or the repo's
  `.worktreeinclude` (gitignore syntax) names, never a tracked file, and only a path that the
  worktree's own branch ignores too (`git check-ignore` there before each copy: the snapshot
  is `git add -A`, and an ignore rule that is not committed is not in the worktree); a block
  of 10 ports in `CODLY_PORT_BASE` (setup script, verify commands, the agent); and the repo's
  `.codly/setup.sh`. That script is code from the repo: it is off until the user turns it on
  in Settings, which shows its text and what the risk rules see in it, and Codly runs only
  the exact text that was turned on (a changed file waits for the user again). It runs like
  a verify command (scrubbed env, 10 min), unsandboxed, from a private copy of that text
  started by the program its `#!` line names (`/bin/sh` without one). A chat line (a plan, a
  1:1 agent) or a Decision (a task) says that it runs. If it fails, the plan does not start
  or the task is blocked with its last line; a 1:1 agent is told in its chat.
- The gate is fair to the worker: a merge conflict has its own rounds (3) apart from the fix
  rounds, and a check that runs out of time (Settings, 10 min by default) blocks the task for
  you. From its card you re-run the checks or, when its checks stopped it, merge anyway; the
  failure stays in its record. A team member's task shares the member's worktree: while the
  member is on other work, the task goes back in line for it and keeps your choice.
- Clean-room profile for untrusted repos (`--setting-sources`, `--strict-mcp-config`).
- By itself, Codly removes a worktree only when it is clean and merged, and deletes a
  `codly/…` branch only when its work is merged and its plan is over (merged, cancelled or
  failed) for a day: the housekeeping sweep, which can be switched off. A dirty or unmerged
  worktree, a branch with commits of its own, a plan that waits for Merge and one you chose
  to keep are never touched by it. You delete those in Settings → Storage, after a confirm
  that names what is lost; the branch stays unless you tick it too. Every removal is a
  `storage` decision with the commit it was at.
- Checkpoints (`orchestrator/checkpoints.rs`): before each turn of a worker or a 1:1 agent, Codly
  keeps the files of the agent's worktree (tracked and untracked, not ignored) as a commit under
  a private ref, `refs/codly/checkpoints/<agent>/<n>`: on no branch, the last 50 per agent, and
  neither HEAD nor the worktree's index moves. "Restore files to this point" in the agent's Git
  tab first keeps the present files (so a restore can be undone), then puts the files back. It
  changes files only: the conversation inside the CLI stays, and the agent's next prompt says
  what was put back. It is refused while a turn runs in that worktree (whoever the agent is) or
  Codly checks a task there, and it never runs in your own checkout: an agent hired with Git
  isolation off has no checkpoints.
- The user's branch: Codly commits and merges by itself only on `codly/…` branches. Four
  commands change the user's branch or its remote (commits, merges, push), each behind a
  button: `merge_plan` and `merge_agent` (merge into the checked-out branch),
  `commit_checkout` (the user's own changes) and `push_checkout` (`git push` of the
  checked-out branch, with the user's SSH agent). Two more push one of Codly's own `codly/…`
  branches (a plan's integration branch or a solo agent's branch, never the user's own, never
  with force) to the user's remote: `create_pull_request`, which then opens the pull request
  with the user's `gh` and its sign-in (Codly stores no token), and `push_pr_branch`, the way
  without `gh`. Both run from a dialog that names the remote, the branch and the base, and
  send nothing when the repository no longer matches it. The dialog also says how many of the
  user's own unpushed commits the branch starts from, since they go with it. Codly never pushes
  on its own.
- The workspace IDE saves files where its agent works (`ide_write`, `ide_create`): in the
  agent's worktree, or in the user's checkout when the agent has none (the Lead, or an agent
  hired with Git isolation off). It never commits.
- Codly's own network use: CLI installers and skill installs (`git clone` of a catalog entry)
  when the user clicks, agents' default tool servers fetched by `npx`/`uvx`, Groq dictation and
  the Claude usage read when switched on, `git push` on Push and on Create pull request, and
  the user's `gh` for the pull-request step (`gh auth status` for the dialog, plus `gh pr view`
  when a pull request is on record; `gh pr create`; `gh pr view` and `gh pr checks` once per
  Refresh: no polling). Inbound webhooks and
  `codly serve` listen on 127.0.0.1 (`codly serve --host` changes that).

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

A test (`crates/codly-core/tests/docs.rs`) fails when a module of the engine, a file of the
shell or a folder of the UI is missing from this map.

```
crates/codly-core/src/    the engine: no Tauri dependency
  lib.rs            the module list
  model.rs          contract: domain + event types (ts-rs → src/lib/gen)
  env.rs            login-shell env, CLI and sign-in detection, scrubbed child envs
  proc.rs           process groups, bounded NDJSON reader, shutdown ladder
  adapters/         claude.rs (stream-json + control), codex.rs (app-server), acp.rs, classify.rs;
                    codex/account.rs = questions to a Codex sign-in outside a run (models, usage
                    windows, the user's own MCP servers, compaction);
                    testdata/ = the recorded transcripts that the tests replay
  orchestrator/     the Core facade (mod.rs) and what is behind it:
                      lead.rs prompts.rs names.rs         Leads and 1:1 chats, every word said to an agent, names
                      planner.rs plans.rs scope.rs        plan validation, plans and tasks, write scopes
                      scheduler.rs accounts.rs            routing; sign-ins, usage windows, account choice
                      codex_account.rs                    model catalogs, Codex usage read, "Compact now"
                      retry.rs stuck.rs                   worker retries with a wait; plans that cannot go on by themselves
                      runs.rs ingest.rs                   agent runs; the one event pipeline
                      gate.rs rebrief.rs                  commit → verify → scope → review → merge queue; a worker's task
                                                          again after a context compaction
                      approvals.rs controls.rs ask_me.rs  approval policy, live agent controls, "ask me" cards
                      approval_rules.rs                   "always allow in this repo": saved approval rules
                      breaker.rs breaker_run.rs           circuit breaker: policy and enforcement
                      watchdog.rs                         silent turns and idle workers: flag, interrupt, stop
                      interrupts.rs                       who asked for an interrupt; what follows when the CLI does not stop
                      team.rs hires.rs careers.rs         standing team and consultants, hired agents, XP and offers
                      workspace.rs agent_git.rs ide.rs    open a repo, snapshot, merge, commit, push; Git tab; IDE facade
                      pull_request.rs                     the pull-request step: push a Codly branch, `gh pr create`, checks
                      checkpoints.rs                      an agent's files before each turn (private refs); restore
                      tools.rs triggers.rs memgraph.rs    MCP tool handler, trigger loop, Knowledge room lenses
                      housekeeping.rs                     glue for housekeeping/ (storage: housekeeping/storage.rs)
                      worktree_setup.rs                   a new worktree made ready: copied files, ports, setup script
                      wallet.rs lots.rs snapshot.rs       the game: coins, plots, office export files
  store/            SQLite: schema.sql, later migrations (m2…m4), typed queries
  git.rs            worktrees, commits, merges, push (shells out to the user's git)
  mcp/              server.rs (Unix socket), bridge.rs (`codly mcp`), tools.rs (tool specs)
  index/            code retrieval: FTS5 over paths, symbols and chunks; pointer packs
  memory/           memories, triples, packs; panel.rs feeds the Knowledge room
  pty.rs            terminals
  quota.rs          opt-in: Claude usage through the sign-in's OAuth token (`read_claude_quota`)
  byok.rs           opt-in: your own API key per vendor (Keychain), each for one CLI only
  dictation.rs      opt-in: Free Flow dictation through Groq Whisper (key in the Keychain)
  inbound/          webhook listener, Slack source (not connected yet), secrets.rs (Keychain)
  triggers.rs       schedules and network mode: pure policy
  housekeeping/     archive.rs (board archiving), connections.rs (agents' default tool servers),
                    storage.rs (worktrees, `codly/…` branches, run logs: what is left, what may go)
  skills.rs         skills manager: scan, bundled catalog, install (git clone), uninstall
  install.rs        onboarding: install commands per CLI, run in a visible terminal
  ide.rs            confined file operations for the workspace IDE
  repos.rs          the git repositories under a folder, for `codly serve`'s repository picker
  blockers.rs       "ask me" cards (codly_ask_user)
  hire.rs           hire manifests
  career.rs         XP, ranks, traits: pure functions
  lot.rs price.rs   build mode: the ready-made lots, the price table
crates/codly-core/        build.rs (builds in the ready-made lots), skills/catalog.json, tests/ (integration
                          tests), examples/ (adapter_smoke.rs and e2e.rs need live CLIs; bench_localize.rs)
src-tauri/src/            the macOS shell
  main.rs lib.rs    entry point; window, plugins, the command list
  cli.rs            what `codly <arguments>` asks for, decided before a window or a server starts
  boot.rs           starts the core, and starts it again after a failed start (`retry_core`)
  logging.rs        the log file (`<data dir>/logs/codly.log`) and the panic hook
  diagnostics.rs    Settings → Troubleshooting: the scrubbed report for a bug report
  commands.rs       most Tauri commands: thin wrappers over Core
  agent_controls.rs ask_me.rs dictation.rs hire_commands.rs ide_commands.rs inbound.rs
  onboarding_commands.rs pr_commands.rs    the other commands, one file per topic
  ui_bus.rs         one ordered Channel of coalesced event batches per webview
  web.rs            `codly serve`: the same UI over HTTP; passes /api/voice/ws to the voice relay
  ipc_tests.rs      the real commands through Tauri's IPC layer (mock runtime)
src-tauri/                build.rs (command list), capabilities/ and permissions/ (ACL), tests/acl.rs
src/                      the Svelte 5 app
  lib/              api.ts (transport), mock.ts and demo/ (the browser demo's backend), state/, util/,
                    gen/ (types generated by ts-rs)
  shell/            the frame: top bar, rail, crew strip, office layer, office tabs
  home/             the conversation with the Lead: composer, request tabs, cards, dictation, voice
  rooms/            plan/, changes/, knowledge/, terminal/
  sheet/            the side sheet of an agent, a task or a file
  overlays/         Settings (one file per section), Welcome, palette, add-agent wizard, skills manager
  ide/              the workspace IDE overlay
  office/           the World (three.js): engine, reducer, sims, traits, breakroom; build/ (build mode)
  world/            the neighbourhood map: plots, the town, agents' homes
  onboarding/       the first-run quest line
  components/       shared widgets
  styles/           design tokens and base CSS
public/                   third-party furniture models and sounds (CC0, CC BY; credits in each folder)
site/                     the landing page and the demo build
scripts/                  install-app.sh (build and install Codly.app), kaykit-clips.py (animation clips)
voice/                    experimental voice relay (Python, OpenAI Realtime) behind `codly serve`; its
                          start.sh needs the author's own OpenAI workload identity, so it does not
                          run for anyone else as it is
research-skills/          optional research skills, installed by hand; not part of the app
docs/                     this file, design/ (SIMS, PROGRESSION, REDESIGN), research/
```

## 9. Verification

The repo has no CI: every check is run by hand. This section says what each check covers and
when a live check was last done. It does not record test results: run the tests.

**Covered by tests** (`cargo test --workspace`, `pnpm check`, `pnpm test`):

| What | Test |
|---|---|
| UI ↔ Rust command boundary | `src-tauri/src/ipc_tests.rs` (Tauri mock runtime, real Core, real ACL): commands answer with the frontend's argument names. `src-tauri/tests/acl.rs`: the handler list, the build.rs list and the capability agree |
| Adapters | recorded CLI transcripts are replayed (`adapters/testdata/`); no live CLI |
| Orchestration | `crates/codly-core/tests/orchestrator_*.rs`, with scripted agents in place of CLIs |
| World frame cost | `src/office/__tests__/idle.test.ts`: a still office asks for 0 animation frames and 0 timers over 10 s; a live one runs at about 30 fps |
| No CLI credential is read by default | `orchestrator/accounts.rs`, test `claude_tokens_are_read_only_after_the_user_switched_it_on` |
| This file's code map, and trust claims that are no longer true | `crates/codly-core/tests/docs.rs` |

**Checked by hand with live CLIs on 2026-10-25. Not re-verified since** (about 250 commits later;
the CLI versions are the ones of that day):

| What | How | Result on 2026-10-25 |
|---|---|---|
| Approval round trip on every CLI | `examples/adapter_smoke.rs` | Claude 2.1.309, Codex 0.161, Gemini 0.62, OpenCode 1.18: approve → command runs → turn completes |
| Agents call Codly's MCP tools | Claude, Codex, OpenCode | tools visible and callable without prompts |
| Lead → plan → worker → verify → merge | `examples/e2e.rs` | 1 task on Codex in 28 s; 2 parallel Claude tasks + Codex reviewers in 45 s; correct diffs |
| Memory + knowledge graph from real events | `examples/e2e.rs` | task-summary memory, 12 temporal triples with repo-relative file entities |

**Never verified**: driving the bundled app window end to end, Cursor and Copilot (not
installed), resume after an app crash with live CLIs. No live check is recorded here for
anything added after 2026-10-25 (Push, several accounts, inbound webhooks, `codly serve`,
dictation, voice, build mode).
