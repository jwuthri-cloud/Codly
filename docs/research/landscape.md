# Multi-agent coding orchestrators: landscape and lessons for Codly (Munder Difflin, Pixel Agents, Vibe Kanban, Claude Squad and others)

> Research date: **2026-10-25**. Author: research subagent for Codly.
> Scope: Munder Difflin (deep), Pixel Agents (deep), Vibe Kanban / Claude Squad / Crystal / Agent of Empires (source-level), and about 20 other orchestrators (architecture-level).
> Source clones (read-only, nothing built or executed) live in
> `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/`.

## Evidence legend

| Tag | Meaning |
|---|---|
| **[V-src]** | I read it in the cloned source code. The file path and commit are given. |
| **[V-doc]** | I read it in official docs or a vendor page today. |
| **[V-api]** | Live data from the GitHub, crates.io, npm, or HN Algolia APIs, fetched 2026-10-25. |
| **[C]** | A claim from marketing, a README, a third party, or a forum. It was not independently verified. |
| **[I]** | My inference from the evidence. |

Clone commits:

| Repo | Commit | Commit date |
|---|---|---|
| HarnessMD/munder-difflin | `d3460def46bf` | 2026-10-25 |
| pixel-agents-hq/pixel-agents | `b1bafd02896d` | 2026-10-16 |
| BloopAI/vibe-kanban | `d5cbb5380fa0` | 2026-09-19 |
| smtg-ai/claude-squad | `ce1ffb4392b0` | 2026-08-20 |
| stravu/crystal | `1e18e0bc9812` | 2026-02-26 |
| agent-of-empires/agent-of-empires | `815ebf8d9e1c` | 2026-10-25 |

---

## 0. TL;DR: the decision-relevant findings

1. **Munder Difflin is open source and MIT-licensed.** It is an Electron 32, React, and Pixi.js 8 app with `node-pty` and `better-sqlite3`. The repo moved from `chaitanyagiri/munder-difflin` to **`HarnessMD/munder-difflin`** (9,059★ [V-api]; the site still says 6.7K). The open-source build is the free "Classic" pixel office. "Pro" adds a sidebar, the Stapler, a capabilities catalogue, and Teams with sealed cross-machine messages, and it ships as a separate, apparently closed build from harnessmd.com [V-src + I].
   - **How it drives agents [V-src]:** every CLI runs as a real interactive TUI in a PTY. Structured state comes from each CLI's native lifecycle hooks, which a node shim forwards over a Unix domain socket. The original SPEC used tmux; the code has since moved to node-pty.
   - **How agents message each other:** through files in a hive directory that is a git repo, using `outbox/` → router → `inbox/`. Messages use FIPA-lite speech acts with a hop cap. Only the main process commits to that repo. Idle agents are woken by typing a nudge into their TUI after 12 s of PTY silence.
2. **Pixel Agents ("pixel office for Claude Code") is MIT and has 9,887★ [V-api].** It is a VS Code extension plus `npx pixel-agents`. It uses two signals together:
   - **(a)** 12 Claude Code hooks installed globally into `~/.claude/settings.json`, which POST to a local server.
   - **(b)** Polling of the transcript JSONL every 500 ms. It moved off `fs.watch`, which it found unreliable on macOS.

   When hooks are missing, it falls back to timers: 7 s with no tool result means a permission bubble, and 5 s of silence after a text-only turn means waiting [V-src].
3. **Every serious tool converges on the same primitives:** git worktree per task, a PTY or structured stream per agent, hooks for status, and diff review leading to a PR or squash-merge. Nobody ships **explainable, causally linked orchestration**. That means a view of why each action happened, which task, message, or dependency triggered it, what it cost, and what context the agent had. Nobody ships a **replayable visualization driven only by events** either. These are Codly's open lanes.
4. **The market is consolidating fast.** Vibe Kanban, the Rust-backend reference design, is **sunsetting**: bloop shut down on 2026-04-10 and the project is now community-maintained [V-doc]. Crystal was deprecated and became Nimbalyst in 2026-02. Terragon shut down, with an open-source snapshot on 2026-01-16. **Roo Code was archived on 2026-05-15** [V-api]. `coder/agentapi`, a screen-scraping HTTP wrapper, is archived [V-api]. Meanwhile **Claude Code itself now ships much of an orchestrator** [V-doc]:
   - subagents and agent teams
   - agent view (`claude agents`, a background supervisor, and worktrees under `.claude/worktrees/`)
   - cross-session messaging over a per-session Unix domain socket
   - HTTP hooks (`"type": "http"`) and 33 hook events

   **Codly cannot differentiate on "parallel Claude sessions".** It must win on multi-vendor support, explainability, and visualization.
5. **There is a policy risk to the "use existing subscriptions" premise.** In May 2026 Anthropic announced that Agent SDK, `claude -p`, and third-party-app usage would move off subscription limits on June 15. HN read this as "rip conductor". The change was **paused** on June 15. As of the 2026-10-07 help-center update, the SDK, `claude -p`, and third-party apps "still draw from your subscription limits", and Max and Team plans *also* get monthly API credits [V-doc, support.claude.com/en/articles/15036540]. Secondary sources conflict [C]. **Interactive TUI use is the most policy-robust path** [I]. Codly should be able to run Claude Code either interactively in a PTY or headless with stream-json, and should surface the billing mode per agent.
6. **Best protocol per CLI, verified in Vibe Kanban and Agent of Empires source:**

   | CLI | Integration |
   |---|---|
   | Claude Code | `-p --output-format=stream-json --input-format=stream-json` with the stdio control protocol, or an interactive PTY with hooks |
   | Codex | `codex app-server` (JSON-RPC: `thread/start`, `turn/start`) |
   | Gemini | `--acp` (Vibe Kanban pinned an old `--experimental-acp`) |
   | Copilot | `--acp` |
   | Qwen | `--acp` |
   | OpenCode | `opencode serve` (HTTP), or `opencode acp` |
   | Cursor | `cursor-agent -p --output-format=stream-json` |
   | Amp | `--execute --stream-json` |
   | Pi | `pi-acp` adapter |

   Claude and Codex also have ACP adapters: `@agentclientprotocol/claude-agent-acp` 0.95.0 and `@agentclientprotocol/codex-acp` 2.2.2 [V-api]. The Rust crate is `agent-client-protocol` **3.5.0** [V-api].
7. **Scope hooks per session; never install them globally.** Superset's own `HOOKS_INVESTIGATION.md` shows that global hook registration leaked into every Droid, Codex, and Cursor session on the machine [V-src via gh]. Munder Difflin does it the right way [V-src]:

   | CLI | Scoping mechanism |
   |---|---|
   | Claude | `--settings <file>` |
   | Codex | `CODEX_HOME` per agent |
   | Gemini | `GEMINI_CLI_SYSTEM_SETTINGS_PATH` |
   | OpenCode | `OPENCODE_CONFIG_DIR` |

---

## 1. Munder Difflin: deep dive

### 1.1 Product facts

| Item | Finding | Tag |
|---|---|---|
| URL | https://munderdiffl.in/ (download from https://harnessmd.com/download) | V-doc |
| Repo | https://github.com/HarnessMD/munder-difflin (formerly `chaitanyagiri/munder-difflin`, GitHub redirects) | V-api |
| Stars / activity | 9,059★, pushed 2026-10-25. CHANGELOG is 121 KB. | V-api |
| License | MIT for code. The pixel art is "Modern Interiors by LimeZu" and is carved out in `LICENSE-ASSETS`. | V-src |
| Author | Chaitanya Giri (`package.json` "author") | V-src |
| Version | `package.json` on main says `0.4.6`. The README advertises a 0.5.6 download. The Pro and OSS builds appear to be versioned separately. | V-src / I |
| Pricing | Free forever (Classic office, unlimited local agents, 12 providers, MIT). Pro lists at $200, launch offer $150/365 days or $20/30 days, PPP down to about $100, free for sign-ins until 2026-10-24. Teams: "priced by call". | V-doc |
| HN | "Munder Difflin – Agent harness to run an office of your clones", 2026-08-22, 312 points, 145 comments | V-api |
| Supported CLIs | Claude Code, Codex, Grok, Kimi Code, Gemini CLI, Antigravity (`agy`), Qwen, OpenCode, Crush, Pi, Copilot, Cursor, plus a custom command. BYOK and local models through Ollama, LM Studio, or vLLM. | V-src (`src/shared/agentProvider.ts`) |
| Telemetry | PostHog, with a hard allowlist documented in `TELEMETRY.md` (anonymous, counts only) | V-src |

Stack, verified in `package.json`: Electron `^32.2.0`, electron-vite, React 18, zustand, **pixi.js `^8.5.1`** for the office, `@xterm/xterm` 5.5 with the webgl addon, `node-pty ^1.0.0`, `better-sqlite3 ^11.10`, Monaco and CodeMirror, `@openai/agents-realtime` for voice, and `localtunnel`/`tunnelmole` for webhooks. There is **no Rust**.

Open-core split [I]: the OSS `src/` contains no Pro sidebar code. It also contains no X25519, XChaCha20, or Ed25519 crypto for "sealed cross-machine messages"; `grep` finds no matches. Those features ship only in the harnessmd.com build.

### 1.2 Architecture: two data planes

`SPEC.md` describes the load-bearing design [V-src]:

```
Terminal plane: raw PTY bytes → xterm.js (byte-for-byte authentic)
Event plane:    CLI lifecycle hooks → shim → Unix socket → main process → renderer (avatars)
```

> "Hooks alone don't give you the raw stream the user expects to see. The tmux pipe alone doesn't tell you which tool is running without fragile output parsing. Together: the canvas is event-driven; the terminal view is byte-for-byte authentic." (`SPEC.md` §2)

The SPEC's tmux plan, which used `pipe-pane` for reads and `send-keys` for input, **was replaced by node-pty** (`src/main/pty.ts`). The app now owns the process lifecycle. `PtySession` keeps an 8 KB ring buffer, `tail`, so it can explain crashes, and a `lastOutputAt` timestamp so it can detect idle:

```ts
// src/main/pty.ts
const TAIL_MAX = 8192;            // trailing output kept for crash diagnostics
interface PtySession { id; proc: pty.IPty; cwd; command; owner: WebContents|null;
  lastOutputAt: number;  // heartbeat: "never type into a PTY that produced output in the last few seconds"
  tail: string; hasOutput: boolean; }
```

**Hook shim.** `HOOK_SHIM` in `src/main/hive.ts:3111` is written to disk and run with a bundled node. It reads the hook JSON from stdin, stamps `agent_id` from `AGENT_ID`, writes one NDJSON line to `HIVE_SOCK`, waits for the reply, and prints it to stdout so the CLI receives the hook decision. It has a 5 s hard timeout, and **failing open (exit 0) means "allow"**:

```js
// src/main/hive.ts (HOOK_SHIM, abridged)
if (!payload.agent_id) payload.agent_id = process.env.AGENT_ID || null;
const c = net.createConnection(sock, () => c.write(JSON.stringify(payload) + '\n'));
c.on('data', (d) => { resp += d; }); c.on('end', () => done(0));
c.on('error', () => process.exit(0));          // fail-open
setTimeout(() => process.exit(0), 5000).unref();
```

**Per-session Claude settings.** These are injected with `--settings <file>` and never touch `~/.claude` (`hive.ts` `hookSettings()` around line 1252) [V-src]. The file contains:
- hooks for `Stop`, `SubagentStop`, `PreToolUse` (`*`), `PostToolUse` (`*`), `UserPromptSubmit`, `Notification`, and others;
- a `statusLine` command. It feeds `context_window.{total_input_tokens,context_window_size}`, which the code comment calls "the only clean programmatic source for the session's REAL context window";
- a per-session MCP bundle;
- **an OS sandbox block**:

```ts
sandbox: { enabled: true, filesystem: { allowWrite: writableDirs } },
permissions: { additionalDirectories: writableDirs }
// comment: "Verified live (claude 2.1.239): with this block, bypass mode still writes cwd and
// the listed dirs but `touch $HOME/x` fails with 'Operation not permitted'."
```

Identity and protocol are injected with `--append-system-prompt` for Claude. Other CLIs receive them as the initial prompt (`-i`, `--prompt`, `-p`, or a positional argument), or **by typing into the TUI after boot** (`seedDelivery: 'type-into-tui'`), which is used for Crush and Cursor [V-src].

**Bridges per provider** (`src/shared/agentProvider.ts`) [V-src]. This is the most complete public map of how to get lifecycle events out of each CLI:

| Provider | Default cmd / auto flag | Event bridge | Hook scope | Resume |
|---|---|---|---|---|
| claude | `claude` / `--permission-mode bypassPermissions` | native hooks via `--settings` | per session | `--resume <sid>` |
| codex | `codex` / `-a never -s workspace-write` (plus `--add-dir` for the hive dir) | Codex `hooks.json`, Claude-shaped. `Stop` honors `{decision:'block'}`. | **per-agent `CODEX_HOME`** | `codex resume [SESSION_ID]` (a subcommand, not a flag) |
| gemini | `gemini` / `--approval-mode=yolo` | Gemini hooks (`BeforeTool`/`AfterTool`/`BeforeAgent`/`AfterAgent`/`SessionStart`) | **`GEMINI_CLI_SYSTEM_SETTINGS_PATH`** per agent | `--resume` |
| antigravity | `agy` / `--dangerously-skip-permissions` | agy hooks with a translating shim | **global** `~/.gemini/config/hooks.json`, guarded by `AGENT_ID` | `--conversation <id>` |
| opencode | `opencode` / (`permission:allow` via `OPENCODE_CONFIG_CONTENT`) | plugin on `tool.execute.before/after` + `session.idle`, marked "LIVE-UNVERIFIED" | `OPENCODE_CONFIG_DIR` per agent | n/a |
| grok | `grok` / `--permission-mode bypassPermissions` | camelCase hooks normalized by a shim | per agent | `--resume` |
| crush, qwen | `--yolo` | **proxy bridge**: a loopback reverse proxy over the LLM API synthesizes hook events and cost samples | env base URL | `--session` |
| pi | `pi` / `--approve` | hooks shim | n/a | `--session` |
| copilot | `copilot -p … -s --allow-all-tools --no-ask-user` | none (`canReceiveInbox:false`) | n/a | `--resume` |
| cursor | `cursor-agent` / `--force --trust` | none; seed typed into TUI | n/a | `--resume` |
| kimi | `kimi` / `--auto` | none yet | n/a | n/a |

Two lessons for Codly:
- **The "proxy bridge" trick** (intercepting the CLI's LLM traffic to synthesize events and cost) is the fallback for CLIs with no hooks. It works only for BYOK or base-URL providers, never for subscription OAuth [I].
- **Several presets carry `// TODO-verify` comments.** Flag drift is a constant maintenance cost. One example in the code: Codex has no `--resume` flag, so "restarts used to silently start a brand-new session".

### 1.3 The hook server: what each event drives

`src/main/hooks.ts` `handle()`, around lines 383–560 [V-src], in order:

1. `Status` is pure statusLine telemetry. It updates the context gauge (`hive:contextUpdate {tokens, limit}`) and returns early.
2. **Operator HALT** returns `{ continue: false, stopReason: 'Halted by the operator from the floor.' }`. This is a graceful stop at the next hook boundary, with no PTY kill.
3. `session_id` is captured with `hive.recordSession()` for idempotent `--resume` and cost deduplication.
4. `CostSample`, from the proxy bridge, is appended to the cost ledger.
5. `PostToolUse` feeds the circuit breaker: an identical repeated `(tool_name, tool_input)` signals a runaway loop. `PreCompact`/`PostCompact` open and close a compaction exemption.
6. **Stop / SubagentStop.** Earlier versions returned `{"decision":"block","reason":<inbox>}` to force continuation. **This was removed:**
   > "Never turn unread hive mail into a forced continuation at Stop. That old path bypassed terminal-draft/HITL safety and could spend credits while a user was answering a question."
7. The **HITL gate** on `PreToolUse` returns `permissionDecision: 'deny'` immediately for paused or gated tools. Slow human approval is left to the CLI's native prompt.
8. **Mid-run steering** sends a single `hookSpecificOutput.additionalContext` on `UserPromptSubmit`/`PostToolUse`. It merges three things, because only one `additionalContext` is allowed per hook:
   - the live roster for the orchestrator, including each agent's `ctx NN%`;
   - the standing goal, sent at SessionStart and then **only when it changes**. Repeating a multi-KB goal every prompt "can make it one of the largest elements in a long transcript";
   - queued operator guidance.
9. `Notification` decides "needs human" by **string-matching the message text** ("waiting for your input", "permission", "approve"). Claude now exposes `notification_type`, so this is a pitfall (see §6).

### 1.4 Hive messaging, board, and memory

From `HIVE.md` [V-src]. The on-disk layout under `<harnessHome>/hive/` is a git repo that **only the Electron main process commits to**. That avoids `.git/index.lock` races. It uses retry and backoff and recovers from stale locks (`hive.ts:2806`).

```
hive/ PROTOCOL.md registry.json board.md tasks.json log.jsonl
  agents/<id>/ identity.md memory.md inbox/ inbox/.done/ outbox/ cursor.json
```

Message schema ("FIPA-lite"):

```jsonc
{ "id":"2026-05-30T14-03-11-123Z-a1b2", "conversation":"conv-7f3", "in_reply_to":null,
  "from":"agent.researcher", "to":"agent.coder | god | broadcast",
  "act":"request | inform | propose | query | agree | refuse | done",
  "subject":"…", "body":"…", "hops":3, "requires_reply":true, "needs_human":false,
  "created_at":"ISO-8601" }
```

Rules that prevent livelock:
- Only `request`, `query`, and `propose` obligate a reply.
- `hops` increments on every reply, and past a cap the message escalates to the god agent.
- Processing is idempotent through `cursor.json`.
- Each message is one JSON file, written to a temp file and then atomically renamed.
- Each file has a single writer: agents write only their own `outbox/`, and the router moves messages into the recipient's `inbox/`.

**Delivery to an idle agent** is the weakest part. `src/main/workerWake.ts` types a fixed nudge into the PTY when all of these hold:
- the agent has produced no PTY output for `WORKER_WAKE_IDLE_MS = 12_000`;
- it is not within `BOOT_GRACE_MS = 35_000` of spawn;
- it has not been nudged in the last `COOLDOWN_MS = 60_000`;
- no permission or HITL notification arrived recently.

The text and the Enter key are sent as separate keystrokes:
> `'You have new hive inbox message(s) — read your inbox, act on them now, and move handled ones to inbox/.done/. …'`

**God agent "Michael".** This is an ordinary CLI session pinned to `desk-ceo`, optionally started with `/remote-control` so a phone can approve its prompts. It owns `registry.json`, is the sole writer of `board.md`, and owns the `tasks.json` ledger. "Tune the prompt, not the code."

**Task board.** `hive/tasks.json` is a hand-written ledger that the god agent edits. `src/shared/taskLedger.ts` exists because UI writers holding partial models of a card **deleted unknown fields on every card**. The fix is a field-preserving merge keyed by `id`. Lesson for Codly: the board must be a typed schema owned by Codly, not a free-form file that an LLM edits.

**Memory.** Each agent has a `memory.md`. The optional MemPalace CLI (semantic recall) is detected, and memory degrades gracefully without it. Per `HIVE.md`, vector stores such as Letta, Mem0, or Zep are "architecturally wrong here (they want to own the agent runtime)".

### 1.5 Recovery

- `session_id` is learned from every hook payload. On respawn, the presets choose `--resume <sid>`, `codex resume <sid>`, or `--conversation <id>`. Codex resume has to locate the `CODEX_HOME` that owns the rollout and its `state_5.sqlite` index (`src/main/index.ts` around line 2888) [V-src].
- **Auto-revive after sleep.** The main process detects PTYs that went silent after `power:resume` and respawns exactly those, in the existing worktree, with `isolate:false, resume:true` (`useHive.ts` effect 7) [V-src].
- **Worktrees.** `agent/<slug>` branches come from `git worktree add <path> -b agent/<slug> <base>`. Teardown **preserves** a worktree when it is dirty or ahead of base (`worktreeHasUnintegratedWork`, which "fails SAFE"). A GC pass removes it once the work has been integrated (`src/main/git.ts:263`, `index.ts:382`) [V-src].
- **Circuit breaker** (`src/main/breaker.ts`). Levels escalate `healthy → steering → constrained → stopped`, at most one level per beat. Inputs:
  - token velocity, computed from differences between cumulative samples;
  - identical repeated tool calls;
  - no-progress detection from file mtimes;
  - a compaction exemption.

  Hard stop is **off by default**. The module's comment notes that "Claude Code exposes `--max-turns` but NO dollar ceiling, so we enforce one ourselves."

### 1.6 How the office maps to events

`src/renderer/src/hooks/useHive.ts`, around lines 195–230 and 478–560 [V-src]:

- `PreToolUse(tool)` → `stationForTool(tool)` → avatar walks to a station and carries a glyph:
  - Read/Grep/Glob → shelf
  - Bash → terminal
  - WebFetch/WebSearch → web portal
  - `mcp__*` → MCP corner
  - TodoWrite → board
  - Task (subagent) → mailbox
- Non-Claude tool names (for example agy's `run_command`, `write_file`) go through a **regex heuristic**: `/command|bash|shell|exec|terminal|run_/` → terminal, and so on.
- `PostToolUse`/`UserPromptSubmit` → working. `Stop` → idle at desk. `PreCompact` → "compacting". Breaker state **takes precedence** over hook status and pins the avatar to "looping".
- Routing a message makes an envelope fly from desk to desk (`MessageEnvelope.ts`). `ToolBubble.ts` (ported from `shahar061/the-office`, ISC) shows `"<icon> <target>"` above the character.
- Per the author on HN [C]: "Simulations are deterministic, they do not consume tokens." The ambient motion is scripted. Only state changes come from real events.

### 1.7 Users love / hate (HN 49398152, 2026-08) [C]

- **Love:** it is fun and charming. The spatial map suits many concurrent agents ("it's a mistake to try and communicate it all with text").
- **Hate:**
  - "cringe" and distracting ("tamagotchis to look at while their AI is working");
  - unclear whether it is "a fun game … or a productivity tool";
  - the "GOD orchestrator" label offended some readers;
  - a verbose AI-written landing page;
  - "I don't want to contact you for pricing".

  A related comment on the agent-teams HN thread: "a whimsy-free clone … will win because it's significantly less embarrassing to explain to normal people."

---

## 2. Pixel Agents: deep dive

| Item | Finding | Tag |
|---|---|---|
| Repo | https://github.com/pixel-agents-hq/pixel-agents (publisher `pablodelucca`), MIT, 9,887★, v1.5.0 on npm (2026-10-17) | V-api |
| Forms | VS Code extension (Marketplace and Open VSX) and a standalone `npx pixel-agents` (local server plus browser, token-auth, read-only office without the token) | V-src |
| Providers | Claude Code is the reference. A typed `HookProvider` interface means "a new CLI is a subdirectory". | V-src |
| HN | Several low-traction Show HNs (2026-09) | V-api |

**Activity detection** (`server/src/fileWatcher.ts` header) [V-src]:

> "HOOKS MODE (preferred): Claude Code Hooks API delivers instant, reliable events … HEURISTIC MODE (fallback) … JSONL POLLING (always active): readNewLines + processTranscriptLine run in both modes. They provide tool content (status text, animations) that hooks don't carry."

- **Hooks.** It installs `CLAUDE_HOOK_EVENTS = ['SessionStart','SessionEnd','Stop','PermissionRequest','Notification','PreToolUse','PostToolUse','PostToolUseFailure','SubagentStart','SubagentStop','TeammateIdle','TaskCompleted']` into the **global** `~/.claude/settings.json`. It takes a one-time `.pixel-agents.backup` copy and uses an atomic tmp-write plus rename with re-read and retry. It "deliberately NOT installed: UserPromptSubmit and TaskCreated", which minimizes the data collected (`providers/hook/claude/constants.ts`).

  The hook script reads a registry `~/.pixel-agents/servers/*.json`, skips dead PIDs, and **fans out an HTTP POST to every live server**, VS Code and standalone included.
- **Transcript polling** every 500 ms (`FILE_WATCHER_POLL_INTERVAL_MS`):
  > "Previously used triple-redundant fs.watch + fs.watchFile + setInterval, but fs.watch is unreliable on macOS/WSL2 … The manual poll at 500ms is fast enough."
- **Heuristic timers** (`server/src/constants.ts`) apply only when no hooks are present:

  | Constant | Value |
  |---|---|
  | `PERMISSION_TIMER_DELAY_MS` | 7000 |
  | `TEXT_IDLE_DELAY_MS` | 5000 |
  | `TOOL_DONE_DELAY_MS` | 300 (anti-flicker) |
  | `EXTERNAL_SCAN_INTERVAL_MS` | 3000 (adopts sessions started outside the office if the JSONL mtime is under 2 min old) |
  | `DISMISSED_COOLDOWN_MS` | 180_000 |

- **Context gauge.** It assumes 200k tokens until it sees evidence of a larger window, using `CONTEXT_WINDOW_TIERS = [200_000, 1_000_000]`.

**Event normalization** (`providers/hook/claude/claude.ts`) [V-src]:

```ts
case 'Notification':
  if (notificationType === 'permission_prompt') return { kind: 'permissionRequest' };
  if (notificationType === 'idle_prompt') return { kind: 'turnEnd', awaitingInput: true };
case 'PreToolUse':   → { kind:'toolStart', toolName, input, runInBackground }
case 'Stop':         → { kind:'turnEnd' }
case 'SubagentStart' → { kind:'subagentStart', toolName: agent_type, … }
```

- **Subagents vs teammates.** These are separate concepts in `CONTEXT.md`: a named spawn is a teammate with a seat, and an unnamed spawn is a subagent drawn near its parent. Subagent characters are created **from the JSONL `tool_use` id**, not from hooks, because `SubagentStop` must match the real id.
- **Animation mapping** is deliberately coarse: `CharacterState = IDLE | WALK | TYPE`, plus a provider-supplied `readingTools` set that switches TYPE to a reading pose (`webview-ui/src/office/toolUtils.ts`).

**Lessons for Codly** [I]:
- Use hooks for timing, JSONL for content, timers only as a fallback. This is the right hybrid.
- Use one rich normalized `AgentEvent` type.
- Install hooks **per session**, unlike Pixel Agents' global install.
- A layout editor with "areas mapped to folders" is cheap to build and well loved.

---

## 3. Vibe Kanban (BloopAI): Rust reference architecture

| Item | Finding | Tag |
|---|---|---|
| Repo | https://github.com/BloopAI/vibe-kanban, **Apache-2.0**, 28,453★, last push 2026-09-19 | V-api |
| Status | **"Vibe Kanban is sunsetting."** bloop shut down on 2026-04-10. "We couldn't find a business model that we could get excited about." The project continues community-maintained, and remote services were removed after 30 days. | V-doc (vibekanban.com/blog/shutdown) |
| Stack | Rust workspace: axum 0.8, tokio, sqlx/SQLite, git2 0.20, ts-rs for TS types, json-patch. React frontend. **`crates/tauri-app` (Tauri 2) wraps the axum server.** Also an `rmcp`-based MCP task server and a relay/WebRTC/tunnel stack for remote use. | V-src |
| Distribution | `npx vibe-kanban` (npm 0.1.44, 2026-09-19) | V-api |

### 3.1 How it drives each CLI [V-src, `crates/executors/src/executors/*`]

**Fully structured, no PTY scraping.** Each executor builds a command and parses a protocol:

| Agent | Command (as pinned in source) | Protocol |
|---|---|---|
| Claude Code | `npx -y @anthropic-ai/claude-code@2.1.119 -p --verbose --output-format=stream-json --input-format=stream-json --include-partial-messages --replay-user-messages` plus `--permission-prompt-tool=stdio --permission-mode=bypassPermissions` when approvals are on | NDJSON on stdout plus a **bidirectional control protocol on stdin** |
| Claude follow-up | `--resume <session_id>` and optional `--resume-session-at <message_uuid>` (truncates history: "reset to message") | same |
| Codex | `npx -y @openai/codex@0.124.0 app-server` | JSON-RPC through the `codex-app-server-protocol` crate (git tag `rust-v0.124.0`). Methods `thread/start` and `turn/start`. Server requests such as `FileChangeRequestApproval` route to approvals. |
| Gemini | `npx -y @google/gemini-cli@0.29.3 --experimental-acp` | ACP (`agent-client-protocol = 0.8`) |
| Copilot | `npx -y @github/copilot@1.0.83 --acp` | ACP |
| Qwen | `npx -y @qwen-code/qwen-code@0.9.1 --acp` | ACP |
| OpenCode | `npx -y opencode-ai@1.4.7 serve --hostname 127.0.0.1 --port 0` with `OPENCODE_SERVER_USERNAME/PASSWORD` | HTTP server (SSE) |
| Cursor | `cursor-agent -p --output-format=stream-json [--force/--trust]` | NDJSON |
| Amp | `npx -y @sourcegraph/amp@latest --execute --stream-json [--dangerously-allow-all]` | NDJSON |
| Droid | (`droid.rs`) | stream |

Claude stdio **control protocol shapes**, as implemented in `claude/types.rs` and `claude/protocol.rs` [V-src]. This is the Agent SDK's private wire format, not a publicly versioned contract [I].

```jsonc
// Codly → CLI (stdin)
{"type":"control_request","request_id":"<uuid>","request":{"subtype":"initialize","hooks":{
   "PreToolUse":[{"matcher":"^(?!(Glob|Grep|NotebookRead|Read|Task|TodoWrite)$).*","hookCallbackIds":["tool_approval"]}],
   "Stop":[{"hookCallbackIds":["<stop_git_check>"]}]}}}
{"type":"control_request","request_id":"<uuid>","request":{"subtype":"set_permission_mode","mode":"plan"}}
{"type":"control_request","request_id":"<uuid>","request":{"subtype":"interrupt"}}
{"type":"user","message":{"role":"user","content":"…"}}
// CLI → Codly (stdout)
{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash",
   "input":{…},"permission_suggestions":[…],"tool_use_id":"toolu_…"}}
{"type":"control_request","request_id":"r2","request":{"subtype":"hook_callback","callback_id":"tool_approval","input":{…},"tool_use_id":"…"}}
// Codly → CLI reply
{"type":"control_response","response":{"subtype":"success","request_id":"r1",
   "response":{"behavior":"allow","updatedInput":{…}}}}           // or {"behavior":"deny","message":"…","interrupt":false}
```

Notable details:
- `AskUserQuestion` is routed to the approval UI. Without approvals it is disallowed: `--disallowedTools=AskUserQuestion`.
- A `Stop` hook callback is used for a **"commit reminder"**: it checks git state at turn end.
- `SUPPRESSED_STDERR_PATTERNS` filters known noisy warnings.

### 3.2 Normalized log model (worth copying)

`crates/executors/src/logs/mod.rs` [V-src]:

```rust
pub enum NormalizedEntryType {
    UserMessage, UserFeedback { denied_tool: String }, AssistantMessage,
    ToolUse { tool_name: String, action_type: ActionType, status: ToolStatus },
    SystemMessage, ErrorMessage { error_type }, Thinking, Loading,
    NextAction { failed: bool, execution_processes: usize, needs_setup: bool },
    TokenUsageInfo(TokenUsageInfo), UserAnsweredQuestions { answers } , …
}
pub enum ActionType { FileRead{path}, FileEdit{path, changes}, CommandRun{command, result, category},
    Search{query}, WebFetch{url}, Tool{tool_name, arguments, …}, … }
pub enum ToolStatus { Created, Success, Failed, Denied{reason}, PendingApproval{approval_id}, TimedOut }
```

Transport (`crates/utils/src/log_msg.rs`):

```rust
enum LogMsg { Stdout(String), Stderr(String), JsonPatch(Patch), SessionId(String), MessageId(String), Ready, Finished }
```

Logs stream to the UI as **RFC 6902 JSON Patches** over SSE or WebSocket, so the client keeps a document and applies diffs. Raw logs persist as JSONL per execution (`execution_process_logs.logs TEXT -- JSONL`).

### 3.3 Worktrees, merge, conflicts [V-src]

- `crates/worktree-manager/src/worktree_manager.rs`:
  - Each path gets its own `tokio::Mutex` (`WORKTREE_CREATION_LOCKS`).
  - It runs a "comprehensive cleanup", force-removing `.git/worktrees/<name>` metadata and running `git worktree prune`, then **retries `git worktree add` via the git CLI** ("to inherit sparse-checkout semantics").
  - It normalizes the macOS `/private` path alias.
  - Worktrees go under an app-owned subdirectory, `.vibe-kanban-workspaces`, "so orphan cleanup never touches user's existing folders". The default is `get_vibe_kanban_temp_dir()/worktrees`.
- **Merge** (`crates/git/src/lib.rs:575`): it refuses if the base is ahead (`BranchesDiverged`). If the base branch is checked out somewhere, it uses a CLI squash-merge in that checkout after a staged-changes safety check. Otherwise it does a **pure libgit2 in-memory merge**:

```rust
merge_opts.find_renames(true);
merge_opts.fail_on_conflict(true);           // bail instead of writing a conflicted index
let mut index = repo.merge_commits(base_commit, task_commit, Some(&merge_opts))?;
if index.has_conflicts() { return Err(MergeConflicts{…}) }
let squash = repo.commit(None, sig, sig, msg, &tree, &[base_commit])?;  // single parent
repo.reference(&format!("refs/heads/{base}"), squash, true, "Squash merge")?;
// then reset the task branch to the squash commit "so follow-up work can continue … without conflicts"
```

- **Rebase** shells out to `git`. It detects conflicts from stderr ("could not apply", "resolve all conflicts"), lists the conflicted files, and returns an actionable error. It also has `detect_conflict_op() → Rebase|Merge|CherryPick|Revert` to show an in-progress state.
- **Diffs:** `MAX_INLINE_DIFF_BYTES = 2 MB`. Larger file bodies are omitted from the stream "to avoid UI crashes".

### 3.4 DB schema (SQLite, final state after 80+ migrations)

I reconstructed this by applying all migrations to an in-memory SQLite with a Python script that only parses SQL, then dumped the schema. Key tables:

```sql
projects(id BLOB PK, name, …)            repos(id, path UNIQUE, name, display_name, setup_script,
project_repos(project_id, repo_id)          cleanup_script, archive_script, copy_files, dev_server_script,
tasks(id, project_id, title, description,   default_target_branch, default_working_dir, parallel_setup_script)
  status CHECK IN ('todo','inprogress','done','cancelled','inreview'), parent_workspace_id)
workspaces(id, task_id, container_ref /*worktree path*/, branch, setup_completed_at,
           archived, pinned, name, worktree_deleted)
workspace_repos(workspace_id, repo_id, target_branch)          -- multi-repo workspaces
sessions(id, workspace_id, executor, agent_working_dir, name)
execution_processes(id, session_id, executor_action JSON, status CHECK IN ('running','completed','failed','killed'),
   run_reason CHECK IN ('setupscript','cleanupscript','archivescript','codingagent','devserver'),
   exit_code, dropped, started_at, completed_at)
execution_process_repo_states(execution_process_id, repo_id, before_head_commit, after_head_commit, merge_commit)
execution_process_logs(execution_id, logs TEXT /*JSONL*/, byte_size, inserted_at)
coding_agent_turns(execution_process_id, agent_session_id, prompt, summary, seen, agent_message_id)
merges(workspace_id, repo_id, merge_type CHECK IN ('direct','pr'), merge_commit | pr_number/pr_url/pr_status …)
pull_requests(…), scratch(id, scratch_type, payload) /*drafts*/, tags, attachments, migration_state
```

Two details worth noting:
- **`before_head_commit`/`after_head_commit` for each process** make every agent run a reversible, attributable git range. Codly should copy this.
- `dropped` on `execution_processes` is used for "restore to a checkpoint".

### 3.5 Other mechanics [V-src]

- **Waiting for input** comes from the structured protocol: `can_use_tool` or `AskUserQuestion` produces `ToolStatus::PendingApproval{approval_id}`, and `ApprovalService` runs a timeout watcher that defaults to `TimedOut`. There is no screen scraping.
- **Queued follow-ups.** `QueuedMessageService` holds one queued message per session and sends it when the current execution finishes. The UI shows `QueueStatus::{Empty, Queued}`.
- **Processes** are spawned with `command-group` (`AsyncGroupChild`, `kill_on_drop(true)`), so killing a session kills the whole process group.
- **MCP server for agents.** `vibe_kanban_mcp` exposes tools such as "Create a new workspace and start its first session", "List local workspaces", and "Get status for an execution". Agents can plan and create tickets, and the HN launch thread praised this.
- **Pitfall observed:** CLI versions are **pinned with `npx -y pkg@x.y.z`**: claude-code 2.1.119, codex 0.124.0, gemini-cli 0.29.3. Today's versions are 2.1.310, 0.164.0, and 0.65.0 [V-api]. After the sunset these pins rot, and Gemini has since moved from `--experimental-acp` to `--acp` [V-src in AoE].

HN (Show HN, 2025-07, 195 points) [C]:
- **Love:** "same increase in productivity … as when I first used Cursor", and MCP-driven ticket creation.
- **Hate:** asks for GitHub auth with full private-repo scope ("hard NO"); the MacBook slows after about 4 concurrent runs because of rustc; stacked or dependent PRs are painful.

---

## 4. Claude Squad (smtg-ai): the tmux baseline

- Go TUI (Bubble Tea), **AGPL-3.0**, 8,667★, last push 2026-08-20 [V-api]. It supports Claude Code, Codex, OpenCode, Amp, Aider, and Gemini.
- **Process management.** `tmux new-session -d -s <name> -c <worktree> <program>`, then a client attaches over a PTY (`session/tmux/tmux.go:102`). Sessions survive TUI restarts because tmux owns them [V-src].
- **Waiting detection is screen scraping plus hashing** (`tmux.go:246`) [V-src]:

```go
content, _ := t.CapturePaneContent()          // tmux capture-pane -p -e -J -t <name>
if t.program == ProgramClaude { hasPrompt = strings.Contains(content, "No, and tell Claude what to do differently") }
else if strings.HasPrefix(t.program, ProgramAider) { hasPrompt = strings.Contains(content, "(Y)es/(N)o/(D)on't ask again") }
else if strings.HasPrefix(t.program, ProgramGemini) { hasPrompt = strings.Contains(content, "Yes, allow once") }
if !bytes.Equal(t.monitor.hash(content), t.monitor.prevOutputHash) { … return true, hasPrompt }  // "updated" = pane changed
```

- **Trust and MCP prompts** are auto-dismissed by matching "Do you trust the files in this folder?" or "new MCP server" and sending Enter. **AutoYes** is a daemon that taps Enter on every detected prompt (`daemon/daemon.go`).
- **Worktree lifecycle.**
  - Before `git worktree add -b <branch> <path> <HEAD>` it runs `worktree remove -f` and `RemoveAll` to clear orphans, then records `base_commit_sha` for the diff.
  - **Pause** commits the changes, removes the worktree, keeps the branch, and copies the branch name to the clipboard. **Resume** re-adds the worktree.
  - **Submit** pushes with `git push -u` (or `gh repo sync`).
- **Persistence:** a JSON file (`InstanceData{title, path, branch, status, program, worktree{repo_path, worktree_path, branch_name, base_commit_sha}, diff_stats}`) and no DB.
- **Lesson:** the string-matching approach breaks on every TUI copy change. Claude Code is on 2.1.310 [V-api] and changes UI strings often. Use hooks or protocols instead.

## 5. Crystal → Nimbalyst (stravu / nimbalyst)

- Crystal is MIT with 3,132★ and was **deprecated in Feb 2026** in favor of Nimbalyst. Nimbalyst is MIT, 1,973★, pushed 2026-10-25, runs on macOS, Windows, and Linux, and has an iOS companion app [V-api].
- **Crystal source** [V-src]:
  - Electron, `@homebridge/node-pty-prebuilt-multiarch`, better-sqlite3.
  - Claude runs as `--verbose --output-format stream-json -p <prompt> [--resume <sid>] --mcp-config <file>`. Permissions go through **an MCP permission tool**: `--permission-prompt-tool mcp__crystal-permissions__approve_permission --allowedTools mcp__crystal-permissions__approve_permission` (`claudeCodeManager.ts:72–147`).
  - Codex runs as `codex exec --json -c sandbox_mode=… -c model_reasoning_effort=…` (`codexManager.ts:221`).
  - The `execution_diffs` table stores, per prompt, `git_diff, files_changed, stats_*, before_commit_hash, after_commit_hash` linked to `prompt_markers` (`database/database.ts:209`). This gives a "diff per prompt" timeline.
  - Pitfall: git runs through **shell-interpolated strings**, for example ``execWithShellPath(`git worktree add -b ${branchName} "${worktreePath}" ${baseRef}`)`` in `worktreeManager.ts:158`, which is an injection or quoting risk.
- **Nimbalyst** [V-doc README]:
  - visual WYSIWYG editors (markdown, mockups, Mermaid, Excalidraw, CSV, data models) where agent edits arrive as **red/green diffs you accept or reject in the rendered document**;
  - a session kanban and task trackers stored as plain markdown files in the repo;
  - an `EditorHost` extension SDK.

## 6. Agent of Empires (AoE): Rust, tmux, ACP, containers

- Rust, MIT, 3,383★, v1.21.0, pushed 2026-10-25 [V-api / V-src]. Dependencies: ratatui 0.30, tokio 1.52, axum 0.8 (web dashboard), git2 0.21, rusqlite 0.40, notify 8.2, and **`agent-client-protocol = "2.2"`** with `unstable_end_turn_token_usage`, `unstable_session_compaction`, `unstable_session_fork`, and `unstable_session_notices`.
- tmux persistence, a TUI plus a web and mobile dashboard, worktrees and multi-repo workspaces, sandboxing with **Docker, Podman, and Apple Containers**, and status hooks debounced at 100 ms so that "rapid flickers … don't fire spurious hooks" (`src/status_hooks.rs`).
- **ACP agent registry** (`src/acp/agent_registry.rs`, `install_hints.rs`) [V-src]. This is the current map of ACP entry points:

| Agent | ACP command |
|---|---|
| Claude | `claude-agent-acp` (`npm i -g @agentclientprotocol/claude-agent-acp`) |
| Codex | `codex-acp` (`npm i -g @agentclientprotocol/codex-acp`) |
| OpenCode | `opencode acp` |
| Gemini | `gemini --acp` |
| Mistral Vibe | `vibe-acp` |
| Pi | `pi-acp` |
| Oh-My-Pi | `omp acp` |
| Kimi | `kimi acp` |
| Prime | `prime-agent --mode acp` |

## 7. Superset (superset.sh)

- **Elastic License 2.0** (source-available, **not OSI open source**). 15,546★, YC P26, Electron 44.9, node-pty 1.2-beta, tRPC 11, drizzle-orm with better-sqlite3, xterm 5 betas. macOS first, Linux experimental, no Windows [V-src via `gh api`].
- Terminal-first: any CLI agent runs in a worktree with terminals, a diff viewer, a browser pane with per-workspace port detection, a "Design mode" that sends a clicked element to an agent, PR line comments sent to an agent, and remote or cloud workspaces.
- **Its hook post-mortem is required reading** (`HOOKS_INVESTIGATION.md`) [V-src]. Superset registers hooks in **global** configs (`~/.claude/settings.json`, `~/.codex/hooks.json`, `~/.factory/settings.json`, `~/.cursor/hooks.json` and others) at every app start. Hooks that were not guarded fired in every Droid or Codex session on the machine. The remediation is a runtime guard:

```sh
[ -n "$SUPERSET_HOME_DIR" ] && [ -x "$SUPERSET_HOME_DIR/hooks/notify.sh" ] && SUPERSET_AGENT_ID=<id> "$SUPERSET_HOME_DIR/hooks/notify.sh" || true
```

  It also recommends per-session registration where the tool allows it, and removal on uninstall.
- HN (Launch HN 2026-05-22, 108 points; Show HN 2025-12, 96 points) [C]:
  - **Love:** sessions persist across app kill and update; terminal-first; one user runs "40-50 agents".
  - **Hate:** the name collides with Apache Superset; Linear integration costs $20/month; "they all look increasingly the same".

---

## 8. Claude Code's own orchestration (the platform you integrate with and compete against)

All [V-doc], code.claude.com, fetched 2026-10-25. The current CLI is `@anthropic-ai/claude-code` **2.1.310** [V-api].

| Feature | What it is | Key details for Codly |
|---|---|---|
| **Subagents** | In-session delegated workers | Frontmatter `isolation: worktree`, `background: true`, named subagents can `SendMessage` |
| **Agent teams** (experimental) | A lead plus teammates with a shared task list and mailbox | Enable with `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`. Mailbox at `~/.claude/teams/{team}/inboxes/{agent}.json`. Config at `~/.claude/teams/{team}/config.json` (`members[]` with tmux pane ids). Tasks at `~/.claude/tasks/{team}/`, where claims use file locks and dependencies auto-unblock. `teammateMode`: `in-process` \| `auto` \| `tmux` \| `iterm2`. Hooks: `TeammateIdle`, `TaskCreated`, `TaskCompleted` (exit 2 means block plus feedback). **Teams are not available in `-p`/SDK mode.** No nested teams, no resume of in-process teammates, one team per session. |
| **Agent view / background sessions** (research preview) | A supervisor daemon plus a TUI list | `claude --bg "<prompt>"`, `claude agents --json [--all]`, `claude attach\|logs\|stop\|respawn\|rm <id>`, `claude daemon status`. States: `working\|blocked\|done\|failed\|stopped`; `status` is `busy\|waiting\|idle`; `waitingFor` is `permission prompt\|input needed\|sandbox request\|worker request\|dialog open`. Isolation in **`<repo>/.claude/worktrees/`**. Storage: `~/.claude/daemon/roster.json`, `~/.claude/jobs/<id>/state.json`. Delete is refused when there are unpushed commits unless `--discard-unpushed`. Notification types `agent_needs_input` and `agent_completed`. |
| **Cross-session messaging** | `ListAgents`/`SendMessage` between your sessions | Per-session **Unix domain socket** (`CLAUDE_CODE_MESSAGING_SOCKET`) plus `CLAUDE_CODE_MESSAGING_TOKEN`, sent as the first line `{"type":"auth","token":…}`. Messages are delivered between tool calls, or start a new turn when idle. `crossSessionInbound: accept\|hold\|refuse`. Default holds messages when the receiving session bypasses permissions. Messages up to about 1M chars, at most 50 queued, rate-limited. Messages "can't approve anything". Works for `-p` sessions too, but not `--bare`. `notify_when_idle` subscription. Requires v2.1.224+ (macOS/Linux). |
| **Hooks** | 33 events | Includes `PermissionRequest`, `PermissionDenied`, `PostToolBatch`, `StopFailure`, `WorktreeCreate/Remove`, `FileChanged`, `PreModelSwitch`, `TaskCreated`, `TaskCompleted`, `TeammateIdle` and others. Common input carries `session_id`, `prompt_id`, `transcript_path`, `cwd`, `permission_mode`, `effort`, `hook_event_name`, and `agent_id`/`agent_type` inside subagents. `Notification.notification_type` values: `permission_prompt`, `idle_prompt`, `auth_success`, `elicitation_dialog`, `elicitation_url_dialog`, `elicitation_complete`, `elicitation_response`, `agent_needs_input`, `agent_completed`, `quota_auto_resume_fired`, `quota_auto_resume_stale`, `quota_auto_resume_disabled`, `plugin_notification`. `Stop` includes `last_assistant_message`. **`"type":"http"` hooks POST JSON to a URL** (with `headers`, `allowedEnvVars`, `onFailure:"block"`), so Codly needs no shim. `additionalContext` is capped at 10,000 chars. `permissionDecision` is `allow\|deny\|ask\|defer`. |
| **Dynamic workflows, `/batch`** | Script-driven fan-out of 5–30 worktree subagents | Competes with Codly's "plan → fan-out" for single-vendor users |

**Implication** [I]: for Claude-only users, the built-in features already cover parallelism, worktrees, a status list, and messaging. Codly's value is:
- heterogeneous teams. HN has asked for this repeatedly: "opus in the driver seat, but the subagents should be using different LLMs, such as Gemini or Codex";
- a GUI with explainability and replay;
- context engineering across vendors;
- a living visualization built on all of the above.

---

## 9. Other orchestrators (architecture level)

| Product | Status Oct 2026 | Architecture notes | OSS / license | Love / hate [C] |
|---|---|---|---|---|
| **Conductor** (conductor.build, Melty Labs, YC) | Active. Added Conductor Cloud (microVMs) [C]. | Mac app, **built in Tauri with a Rust backend** per the co-founder's launch thread [C]. Supports Claude Code, Codex, Cursor, and OpenCode. One git worktree per workspace. New branches come from a fetched `origin/<base>`, and the agent renames the branch to match the task. `.conductor/settings.toml` `[scripts]` holds `setup`, `run` (named actions), `archive`, and `run_mode = concurrent\|nonconcurrent`. Env vars: **`CONDUCTOR_WORKSPACE_PATH`, `CONDUCTOR_ROOT_PATH`, `CONDUCTOR_PORT` (10 ports per workspace: `PORT..PORT+9`), `CONDUCTOR_WORKSPACE_NAME`**. Stop sends SIGTERM then SIGKILL to the process group after 5 s. A `.context/` folder holds untracked notes. [V-doc] | Closed | **Love:** chat-centric, polished diffs. **Hate:** macOS / Apple Silicon only; "uses the SDK" → exposed to Anthropic's programmatic metering ("rip conductor"). |
| **Sculptor** (Imbue) | Experimental research preview | Desktop app. Workspaces are now **worktrees**; the container backend (Docker or remote) is experimental. Integrated harnesses: Claude Code and **Pi**, plus any terminal agent. Ships workflow skills (`/sculptor-workflow:fix-bug`). [V-doc README] | MIT (Python), 240★ | Low traction |
| **Terragon** | **Shut down. OSS snapshot 2026-01-16.** | Cloud. Each task runs in a sandbox container. Auto-branch and checkpoint commits pushed to GitHub. `terry` CLI for local takeover plus an MCP server. BYO Claude/ChatGPT subscription. [V-doc README] | Apache-2.0 snapshot | n/a |
| **uzi** (devflowinc) | **Stale** (last push 2025-06) | Go CLI: N agents × worktrees × tmux | MIT, 582★ | n/a |
| **ruflo / claude-flow** (ruvnet) | Very active, 79,256★ | "Agent meta-harness" delivered as Claude Code plugins and mods plus an MCP server (`npx ruflo mcp start`), with claims of "100+ agents", swarms, and self-learning memory. **The README's download badge is an "illustrative counter starting at 98,000,000"** [V-src README alt text]. | MIT | Treat all metrics as marketing [C] |
| **Codex** (CLI, app, cloud) | Active. `@openai/codex` 0.164.0 [V-api]. Docs moved to learn.chatgpt.com. | Cloud tasks run in per-task workspaces from published "environments" (setup script, network presets, secrets) and continue while the laptop sleeps. Review leads to a PR. The desktop app coordinates long-running tasks. Local: `codex app-server` JSON-RPC, `codex exec --json`, Claude-shaped hooks. [V-doc / V-src] | Apache-2.0 (CLI) | Many HN users switch to Codex over Claude limits [C] |
| **OpenHands** | Active, 91,819★ | Python Software Agent SDK (`openhands.sdk`, `.tools`, `.workspace`, `.agent_server`). Workspaces swap Local → Docker → RemoteAPI. The agent-server container exposes REST and WebSocket. [V-doc] | MIT | n/a |
| **Goose** (now `aaif-goose`, Linux Foundation AAIF) | Active, 55,700★ | Rust core with a desktop app (TS) and CLI. **Uses Claude, ChatGPT, or Gemini subscriptions "via ACP"** (driving the vendor CLIs as ACP agents). 15+ providers. Has `goose-acp` and `goose-acp-client`. [V-doc README] | Apache-2.0 | n/a |
| **Cline / Cline Kanban** | Active, 70,835★, `cline` 3.0.73 | **Cline Kanban** (launched 2026-03): a browser board from the CLI; each card gets a worktree and terminal; dependency chains auto-trigger the next card; Commit/Open PR asks the agent to clean up and resolve conflicts; works with Cline, Claude Code, and Codex. "Uses experimental features that bypass permissions and runtime hooks." [C / V-doc partial] | Apache-2.0 | n/a |
| **Roo Code** (orchestrator "Boomerang" mode) | **Archived 2026-05-15** [V-api]. The team pivoted to "Roomote", a Slack cloud agent [C]. | VS Code extension. Orchestrator mode spawned subtasks in other modes. | Apache-2.0 | Users told to migrate to Cline or Kilo Code [C] |
| **Amp** (Sourcegraph spinoff) | Active | Threads, an Agents Panel (Jan 2026), "Thread Map", subagents (Oracle, Librarian) that cannot talk to each other mid-task, "Amp Free" ($10/day, ad-supported), `amp --execute --stream-json`. [C / V-src VK] | Closed | n/a |
| **Jules** (Google) | Active (2025 sources only) | Async cloud VM per task leads to a PR. Jules Tools CLI/TUI. GitHub label `jules` triggers it. [C, likely stale] | Closed | n/a |
| **Devin** (Cognition) | Active. Windsurf rebranded to "Devin Desktop" (2026-06) [C]. | Cloud VMs, "managed Devins" orchestrated in parallel (up to 10 concurrent), playbooks, knowledge base. ACU pricing. [C] | Closed | n/a |
| **Factory** (Droids) | Active | **Missions**: an orchestrator splits work into milestones and features, each feature gets a fresh worker session, and validation runs per milestone. `droid exec --mission`. [V-doc partial / C] | Closed | n/a |
| **Warp** (Oz) | Launched 2026-02 | A cloud control plane for agents: Claude Code, Codex, and Warp's own agent; hundreds of agents; audit trail; shareable run links; local↔cloud handoff; K8s self-host. [C, vendor PR] | Closed | n/a |
| **Zed** agent panel | Active, 92,178★ | A Threads Sidebar with Zed Agent, **External Agents over ACP** (ACP Registry: Claude, Codex, OpenCode, Copilot, Cursor, Pi), and Terminal threads. Threads run in parallel and can live in linked worktrees. It can import external-agent threads. [V-doc] | GPL/AGPL mix | n/a |
| **Gas Town** (Steve Yegge) | Active [C] | Multi-role "town" metaphor (mayor, polecats, witnesses…). Often compared to agent teams on HN. | n/a | Polarizing |

---

## 10. Comparison table (core candidates)

| | Munder Difflin | Pixel Agents | Vibe Kanban | Claude Squad | Crystal / Nimbalyst | Superset | Conductor | AoE | Claude Code native |
|---|---|---|---|---|---|---|---|---|---|
| UI tech | Electron + React + Pixi.js | VS Code webview / browser (canvas) | Web (React) + **Tauri 2** wrapper | Go TUI | Electron | Electron 44 | **Tauri (Rust)** [C] | Rust TUI + axum web | TUI |
| License | MIT (assets separate); Pro closed | MIT | Apache-2.0 (sunsetting) | AGPL-3.0 | MIT | ELv2 | Closed | MIT | Proprietary |
| Process mgmt | node-pty in Electron main | Adopts VS Code terminals or spawns | tokio + `command-group`, stdio pipes | tmux | node-pty | node-pty; sessions survive app restart | ? | tmux + optional containers | own supervisor daemon |
| CLI integration | Interactive TUI + **native hooks via per-agent config** + proxy bridge | Global Claude hooks + JSONL tail | **Structured**: stream-json control, app-server, ACP, serve | Screen scrape | stream-json + MCP permission tool; `codex exec --json` | Interactive TUI + global hooks (guarded) | SDK [C] | tmux + ACP structured view | n/a |
| Isolation | worktree `agent/<slug>`, preserved if unintegrated; Claude sandbox | none | worktree under app dir; multi-repo | worktree + pause/resume | worktree | worktree, remote | worktree, cloud microVM | worktree + Docker/Podman/Apple Containers | `.claude/worktrees/` |
| Inter-agent msg | **File mailbox + router + god agent**, FIPA acts, hop cap | none (visualizes Claude teams) | none (MCP to create tasks) | none | none | none | none | ? | teams mailbox; cross-session UDS |
| "Waiting" detection | hooks + message text match + 12 s PTY quiescence | hooks (`notification_type`) → timers 7 s/5 s | protocol (`can_use_tool`) | string match + content hash | protocol | hooks | ? | tmux + hooks | native |
| Persistence | SQLite (kv, history) + hive git repo of files | JSON files | SQLite (sqlx), JSONL logs per process | JSON file | SQLite | SQLite (drizzle) | ? | SQLite (rusqlite) | JSON in `~/.claude` |
| Review / merge | god agent integrates; git panel | n/a | diff + inline comments → agent; squash merge (libgit2) or PR | push branch / `gh` | diff per prompt | diff viewer, PR comments → agent | diff, checks, PR, merge, archive | diff review | PR labels |
| Visualization | **Pixel office driven by hooks** | **Pixel office driven by hooks + JSONL** | Kanban | list | kanban (Nimbalyst) | sidebar status | list | list | list |
| Explainability | goal/roster injection, cost gauge | activity label, context gauge | normalized log per turn | none | diff per prompt | none | none | none | transcript |

---

## 11. Proven ideas to steal (with source)

1. **Two planes per agent: terminal bytes plus structured events** (Munder Difflin `SPEC.md`). Show the authentic TUI and drive every UI or world state from structured events. Add a third, **content plane**: transcript JSONL tail (Pixel Agents) or protocol stream (Vibe Kanban).
2. **Scope all CLI instrumentation to the session** (Munder Difflin): Claude `--settings <file>`, Codex `CODEX_HOME=<agentdir>` (link the user's `auth.json`), Gemini `GEMINI_CLI_SYSTEM_SETTINGS_PATH`, OpenCode `OPENCODE_CONFIG_DIR`. If a global write is unavoidable, use the Superset env guard, and remove it on uninstall.
3. **Native HTTP hooks** (Claude Code docs): `{"type":"http","url":"http://127.0.0.1:<port>/hooks/<agent-id>","headers":{"Authorization":"Bearer $CODLY_HOOK_TOKEN"},"allowedEnvVars":["CODLY_HOOK_TOKEN"]}`. No node shim. Use `onFailure` deliberately: fail-open for telemetry, `block` only for gates.
4. **statusLine as a telemetry tap** (Munder Difflin). It gives the exact `context_window.total_input_tokens` and `context_window_size` after every response. Pixel Agents had to guess the 200k vs 1M window.
5. **`additionalContext` for steering instead of typing** (Munder Difflin `hooks.ts`). Inject goals, roster, and operator guidance on `UserPromptSubmit`, `PostToolUse`, and `SessionStart`. **Deliver only on change.** Merge everything into the single allowed context block. Respect the 10k-char cap.
6. **Graceful HALT at hook boundaries** with `{"continue":false,"stopReason":…}`. Return a hook-level `permissionDecision:"deny"` for paused or gated tools. Both are immediate and race-free (Munder Difflin).
7. **Speech-act messages with hop caps and idempotent cursors** (Munder Difflin `HIVE.md`). Keep the schema, but store messages in SQLite instead of git and files.
8. **Single writer for shared state** (Munder Difflin's single committer). In Codly, the Rust core is the only writer of the DB, the board, and git refs. Agents act only through tools such as MCP.
9. **A normalized event model with `ActionType` and `ToolStatus`** (Vibe Kanban `logs/mod.rs`), streamed as **JSON Patch** (Vibe Kanban `LogMsg::JsonPatch`). Map tool names to actions **in the adapter**, never in the renderer. Munder Difflin's regex fallback shows why.
10. **`before_head_commit`/`after_head_commit` per execution** (Vibe Kanban) and a **diff per prompt** (Crystal `execution_diffs`). Every agent turn becomes an attributable, revertible git range. That is the backbone of explainability.
11. **Conflict pre-check without touching the working tree**: libgit2 `merge_commits(..., fail_on_conflict(true))` before offering "Merge". Fall back to an agent-assisted rebase that is given the list of conflicted files (Vibe Kanban `git/src/lib.rs`).
12. **Preserve, don't delete** (Munder Difflin `worktreeHasUnintegratedWork`, Claude agent view's `--discard-unpushed`). Never auto-remove a dirty worktree or one that is ahead of base. Run a GC once the work is integrated.
13. **Per-path worktree mutex, metadata cleanup and retry, an app-owned base dir, and normalization of macOS `/private`** (Vibe Kanban worktree manager).
14. **Workspace env contract** (Conductor): `CODLY_WORKSPACE_PATH`, `CODLY_ROOT_PATH`, `CODLY_PORT` (a block of 10), `CODLY_WORKSPACE_NAME`, plus `setup`/`run`/`archive` scripts, `run_mode=nonconcurrent` for fixed-port stacks, and process-group kill with SIGTERM → 5 s → SIGKILL.
15. **Circuit breaker ladder** (Munder Difflin `breaker.ts`): steer → constrain → stop, one level per beat, token velocity from deltas of cumulative samples, identical-tool-call loop detection, compaction exemption, hard stop opt-in.
16. **Resume by recorded session id** (Munder Difflin): capture `session_id` from every hook payload, then respawn with the provider-specific resume form (`--resume`, `codex resume <id>`, `--conversation`, `--session`). Auto-revive PTYs that were wedged by sleep.
17. **Queue follow-ups instead of interrupting** (Vibe Kanban `QueuedMessageService`). Pair this with Claude's native cross-session delivery semantics, which deliver between tool calls.
18. **Agents get an MCP server into the orchestrator** (Vibe Kanban `vibe_kanban_mcp`, Terragon `terry` MCP). It works across all CLIs, unlike per-CLI file conventions.
19. **Sessions outlive the UI** (Superset, Claude agent view supervisor, tmux tools). Run the agent host as a separate long-lived process.
20. **Coarse, honest animation states** (Pixel Agents `IDLE/WALK/TYPE` plus `readingTools`; Munder Difflin stations), with an activity label from the tool input (`formatToolStatus`: "Reading foo.ts").

## 12. Pitfalls to avoid

1. **Global hook installs** (Pixel Agents, Superset). They leak into every session on the machine, are visible in other tools' UIs, and survive uninstall.
2. **Screen scraping TUIs for state** (Claude Squad `"No, and tell Claude what to do differently"`, auto-dismissing trust prompts). It breaks on copy changes, and an AutoYes daemon that presses Enter on anything is a safety hole.
3. **Matching notification message text** (Munder Difflin). Use `notification_type` (`permission_prompt`, `idle_prompt`, `elicitation_dialog`, `agent_needs_input`…).
4. **Forcing continuation from Stop hooks** (`decision:block` to drain inboxes). Munder Difflin removed this because it bypassed HITL and burned credits. If you use it, guard with `stop_hook_active` and a cap.
5. **Typing into a TUI as a message bus** (Munder Difflin `workerWake.ts`). It needs 12 s idle heuristics, boot grace, HITL re-arm, cooldowns, and separate Enter keystrokes. Prefer `additionalContext`, a protocol stdin, Claude's cross-session socket, or ACP `session/prompt`. Keep typing as the last resort.
6. **Pinning vendor CLIs via `npx -y pkg@ver`** (Vibe Kanban). The pins rot within weeks: 2.1.119 vs 2.1.310 today. Detect the user's installed binary, record its version, and gate features with a capability probe (AoE has `capability_probe.rs` and `version_probe.rs`).
7. **Shell-string git commands** (Crystal). Use argv-style `Command` or libgit2/gix.
8. **`fs.watch` on macOS for JSONL tails** (Pixel Agents dropped it). In Rust, `notify` 8.2 (FSEvents) is fine but **keep a 500 ms–1 s poll fallback** and track offsets.
9. **Free-form LLM-edited board files** (Munder Difflin `tasks.json` field loss). The board must be a typed DB entity. LLMs act on it through tools.
10. **Unbounded context injection** (Munder Difflin goal repetition). Inject deltas only and keep a per-agent token budget.
11. **Hard dependency on tmux or iTerm** (Claude Squad, AoE, agent-teams split panes). These are fragile across terminals, and Claude's docs say split panes are unsupported in VS Code, Windows Terminal, and Ghostty. Own the PTYs.
12. **Building a business on "SDK/headless with a subscription"**. Anthropic's May–Oct 2026 policy churn shows this is fragile. Keep an interactive-PTY path for every CLI and label each agent's billing mode.
13. **Over-whimsy as the default.** HN pushback: "cringe", "is it a game or a tool?", "whimsy-free clone wins". The Professional Workspace should be the default. The world view is an opt-in second view with neutral role names, configurable characters, and no religious labels.
14. **Sunset risk from monetization.** Vibe Kanban could not find a business model for free, local users. Plan revenue early.
15. **Electron resource cost and rustc contention**: "MacBook slowing down after 4 concurrent runs" (Vibe Kanban HN). Add a scheduler with concurrency caps and CPU/IO awareness. HN consensus puts the sweet spot at about 2–4 agents per human reviewer.

## 13. Gaps Codly can win on

1. **Explainable orchestration (causal provenance).** Nobody links an agent action to *why*. Codly can build a causal graph and render it on click:

   ```
   Task → plan step → assignment → message/dependency trigger → prompt → tool calls (with inputs/outputs)
        → file changes (before/after commit) → diff hunks → tests → review → merge, with cost/tokens on every edge
   ```

   Vibe Kanban has per-turn logs and Munder Difflin has a roster and log, but neither has cross-agent causality.
2. **Real-event-only, replayable world.** Munder Difflin and Pixel Agents are live-only. In Codly, every animation should cite an `event_id`. The world state is a pure reducer over the event log, so it supports **time-travel replay**, "why is this avatar at the terminal?" (click opens the exact event), and exportable session films. No scripted "fake work".
3. **Context engineering as a first-class surface.** For each agent, show what is in its context and why:
   - the injected goal, roster, memory, and files;
   - the live context % from statusLine or ACP usage;
   - compaction events;
   - a handoff summary when work moves between agents or vendors.

   No competitor visualizes context composition. Munder Difflin has a gauge only. AoE has a `context_primer.rs`.
4. **Heterogeneous teams done properly.** Use protocol-native adapters (stream-json, app-server, ACP, serve, hooks) with one normalized event schema, so an Opus lead can delegate to Codex or Gemini workers. HN keeps asking for this. Claude agent teams are Claude-only, and Conductor and Superset do not orchestrate across vendors.
5. **Subscription and quota-aware scheduling.** Track each vendor's limits (Claude `quota_auto_resume_*` notifications, rate-limit errors), schedule around them, and show a "billing mode" badge (interactive subscription, programmatic credit, or API key). Nobody does this.
6. **Native macOS performance and footprint.** Use Tauri 2 (2.12.5) with Rust owning PTYs, git, FS, and SQLite. Munder Difflin, Superset, Crystal, and Nimbalyst are Electron.
7. **Review throughput.** Provide a review queue with per-task summaries, invariant tests, conflict pre-checks, and stacked-PR awareness. Review is the HN-consensus bottleneck.
8. **Safety by design**:
   - per-session hooks;
   - Claude `sandbox` with `allowWrite` scoped to the worktree and Codly dirs;
   - Codex `-s workspace-write --add-dir`;
   - no AutoYes;
   - agent messages never count as consent (mirroring Claude's rule that a message "can't approve anything").

---

## 14. Recommendations for Codly (opinionated)

### R1. Process host: a separate Rust daemon (`codlyd`) owns all agents

- `codlyd` runs under launchd, so agents survive Tauri window restarts and app updates. Superset users cite this as the killer feature, and Claude uses the same supervisor pattern.
- The Tauri app connects over a UDS socket (or tauri IPC when in-process during dev).
- PTYs use **`portable-pty` 0.9.0** (or `pty-process` 0.5.4). Process groups use **`process-wrap` 10.0.1**, the maintained successor of `command-group` (5.0.1 dates from 2023). On teardown send SIGTERM to the group, then SIGKILL after 5 s.
- Server-side VT parsing for snapshots uses `vt100` 0.18.1 or `alacritty_terminal` 0.26. That lets the world view read terminal state without a webview. Keep an 8 KB tail ring per PTY for crash forensics (Munder Difflin).

### R2. Four integration tiers per CLI, chosen by capability probe and not hard-coded

| Tier | Mechanism | Use for |
|---|---|---|
| **T1 Interactive** (default for Claude subscription) | TUI in a PTY, per-session `--settings` with **HTTP hooks** to `codlyd`, `statusLine` command, **transcript JSONL tail** (notify + 1 s poll fallback) | Claude Code, Codex TUI (hooks via `CODEX_HOME`), Gemini (system settings path), Grok |
| **T2 Structured headless** | Claude `-p --input-format/--output-format stream-json --include-partial-messages --replay-user-messages --permission-prompt-tool=stdio` plus the control protocol; **Codex `app-server`** (`codex-app-server-protocol` from the matching git tag); Cursor `-p --output-format stream-json`; Amp `--execute --stream-json` | Automated workers; Codex by default |
| **T3 ACP** | **`agent-client-protocol` 3.5.0** client: `gemini --acp`, `opencode acp`, `copilot --acp`, `qwen --acp`, `kimi acp`, `pi-acp`, `@agentclientprotocol/claude-agent-acp`, `@agentclientprotocol/codex-acp` | Long tail of CLIs, one code path |
| **T4 Raw PTY** | PTY plus quiescence heuristics only | Custom commands. Show "limited telemetry". |

All tiers emit the **same** `AgentEvent` enum. Sketch:

```rust
pub enum AgentEvent {            // persisted append-only: agent_events(id, agent_id, ts, causation_id, kind, payload JSON)
  SessionStarted{ cli, cli_version, session_id, source /*startup|resume|clear|compact|fork*/, tier },
  TurnStarted{ prompt_id, origin /*user|codly_task|agent_msg*/, causation_id },
  Thinking{ summary: Option<String> },
  ToolStarted{ tool_use_id, raw_name, action: ActionType, input: Value },
  ToolFinished{ tool_use_id, status: ToolStatus, output_digest },
  FileChanged{ path, hunks, before_commit, after_commit },
  NeedsInput{ kind /*permission|question|elicitation|sandbox*/, request_id, detail },
  Usage{ input, output, cache_read, cache_write, context_used, context_window, usd_estimate },
  MessageSent{ msg_id, to, act }, MessageDelivered{ msg_id, via /*hook_ctx|stdin|acp|uds|pty*/ },
  Compaction{ phase /*pre|post*/ }, TurnEnded{ last_message, stop_reason }, Exited{ code, signal, tail },
}
```

Waiting-state mapping, with priority: protocol request (T2/T3) → `PermissionRequest` hook / `Notification{notification_type}` → Stop + `idle_prompt` → JSONL timers (7 s permission, 5 s text idle) → PTY quiescence (≥ 12 s). Never match TUI strings.

### R3. Messaging: Codly mailbox in SQLite, delivered through the least invasive channel

- Store messages as FIPA-lite (Munder Difflin schema, `hops` cap 4, idempotent ids) in a `messages` table. Agents send and read through a **Codly MCP server** (`codly.send_message`, `codly.inbox`, `codly.claim_task`, `codly.update_task`, `codly.report_progress`, `codly.request_review`). Every vendor CLI speaks MCP.
- **Delivery order:**
  1. Next hook `additionalContext` (`UserPromptSubmit`/`PostToolUse`), for a busy agent.
  2. A protocol prompt for T2/T3 agents (stdin `{"type":"user",…}`, ACP `session/prompt`, Codex `turn/start`), for an idle agent.
  3. For an idle T1 Claude agent, *spike first*: post to the session's `CLAUDE_CODE_MESSAGING_SOCKET`. A `SessionStart` command hook can report the socket path and token to `codlyd`. The wire format past the auth line is **undocumented [unverified]**, and a session that bypasses permissions may hold the message.
  4. A guarded PTY nudge (Munder Difflin rules) as the last resort.
- Messages never carry consent. Approvals happen only in Codly's Approvals UI, which is wired to `can_use_tool`, `PermissionRequest` hooks, or ACP `request_permission`.

### R4. Worktrees and merge

- Base dir: `~/Library/Application Support/Codly/worktrees/<repo-id>/<task-slug>`, never a temp dir. Branches are `codly/<task-id>-<slug>`.
- Create with argv `git worktree add -b <branch> <path> <base>` (CLI, for sparse-checkout and hook parity), behind a per-path tokio mutex, with the Vibe Kanban cleanup-and-retry routine. Read and diff with **`git2` 0.21** (or gix 0.90 for read-heavy paths).
- Record `before_head`/`after_head` for every agent turn (Vibe Kanban plus Crystal).
- Merge flow: libgit2 in-memory conflict pre-check → squash or merge into base → or open a PR via `gh`. On conflict, spawn a "resolver" agent with the conflicted file list and both sides' task context.
- Never auto-delete dirty or unintegrated worktrees. Run a GC after integration. Provide setup, run, and archive scripts and a `CODLY_PORT` block of 10 (Conductor contract).

### R5. Persistence (SQLite via `rusqlite` 0.41 or `sqlx` 0.9; WAL; single writer = `codlyd`)

Adopt Vibe Kanban's backbone: `projects`, `repos`, `tasks`, `workspaces`, `sessions`, `execution_processes`, `execution_process_repo_states`, `merges`, `pull_requests`. Add:
- `agent_events` (append-only, `causation_id`/`correlation_id`)
- `messages`
- `task_deps`
- `approvals`
- `cost_ledger`
- `context_snapshots` (what was injected, token counts)
- `raw_streams` (zstd JSONL per execution, for replay and debugging)

The `causation_id` chain is what makes "click an agent → why" a single indexed query.

### R6. Living Agent World

- Use a pure reducer `world = fold(agent_events)`. Rendering runs in the TS frontend: Pixi.js 8 is proven by Munder Difflin, and a plain canvas is enough (Pixel Agents).
- Map normalized `ActionType` to a station:

  | ActionType | Station |
  |---|---|
  | `FileRead` | shelf |
  | `FileEdit` | desk/typing |
  | `CommandRun` | terminal |
  | `Search` / `WebFetch` | web |
  | MCP | MCP corner |
  | `NeedsInput` | raises hand / mailbox |
  | `MessageSent` | envelope flight |
  | `Compaction` | boxing up |
  | breaker | "looping" |

- Ambient idle motion is allowed but tagged as cosmetic. Every *state* change shows its source event on hover.
- Replay and scrub from `agent_events`. One shared store drives both views, so they stay in sync.
- Characters and labels are neutral by default (roles: Lead, Worker, Reviewer). Themes can be swapped. Licensed assets: LimeZu "Modern Interiors" (paid licence, used by Munder Difflin) and JIK-A-4 Metro City (used by Pixel Agents). Check the licences before shipping.

### R7. Safety defaults

- Claude T1 settings per session:
  - `sandbox.enabled=true`;
  - `filesystem.allowWrite=[worktree, codly_agent_dir]`;
  - `permissions.additionalDirectories` mirroring it (Munder Difflin verified both are needed);
  - `crossSessionInbound` set explicitly.
- Codex: `-a never -s workspace-write --add-dir <codly_agent_dir>` in auto mode. Never use `--dangerously-bypass-approvals-and-sandbox` by default.
- No global config writes. If one is unavoidable (agy), guard it with `[ -n "$CODLY_AGENT_ID" ] || exit 0` and register removal on uninstall.
- Add a circuit breaker per Munder Difflin, plus a per-task dollar or token budget, because Claude has only `--max-turns`.

### R8. Product positioning

- Don't compete with Claude agent view on single-vendor parallelism. Lead with **multi-vendor teams + explainability + replayable world + context engineering**, and make the Professional Workspace the default view.
- Track the Anthropic usage policy page (`support.claude.com/en/articles/15036540`) as a release-blocking dependency, and keep T1 interactive mode first-class.

---

## 15. Versions verified today (2026-10-25)

| Package | Version | Source |
|---|---|---|
| `tauri` (crate) | 2.12.5 (max stable) | crates.io API |
| `portable-pty` | 0.9.0 (2025-02) | crates.io API |
| `pty-process` | 0.5.4 | crates.io API |
| `process-wrap` | 10.0.1 | crates.io API |
| `command-group` | 5.0.1 (2023, stale) | crates.io API |
| `agent-client-protocol` | 3.5.0 | crates.io API |
| `git2` / `gix` | 0.21.0 / 0.90.0 | crates.io API |
| `notify` | 8.2.1 | crates.io API |
| `rusqlite` / `sqlx` | 0.41.0 / 0.9.0 | crates.io API |
| `json-patch` / `ts-rs` | 4.2.0 / 12.0.1 | crates.io API |
| `vt100` / `alacritty_terminal` | 0.18.1 / 0.26.0 | crates.io API |
| `@anthropic-ai/claude-code` | 2.1.310 | npm |
| `@anthropic-ai/claude-agent-sdk` | 0.3.310 | npm |
| `@openai/codex` | 0.164.0 | npm |
| `@google/gemini-cli` | 0.65.0 | npm |
| `opencode-ai` | 1.19.4 | npm |
| `@github/copilot` | 1.0.103 | npm |
| `@agentclientprotocol/claude-agent-acp` | 0.95.0 | npm |
| `@agentclientprotocol/codex-acp` | 2.2.2 | npm |
| `@agentclientprotocol/sdk` | 2.0.2 | npm |
| `pixel-agents` | 1.5.0 | npm |
| `vibe-kanban` | 0.1.44 | npm |
| `cline` | 3.0.73 | npm |

Note: in this environment `cargo search` showed different "newest" versions (for example tauri 3.0.0-alpha.5 and notify 9.0.0-rc.6) than the crates.io API. I trust the API values above. Re-check with `cargo add` at implementation time.

## 16. Sources

**Primary: code** (local clones; upstream URLs pinned to the commits listed under the evidence legend)
- https://github.com/HarnessMD/munder-difflin: `SPEC.md`, `HIVE.md`, `src/main/{pty,hooks,hive,git,breaker,workerWake,index}.ts`, `src/shared/{agentProvider,hookEvents,taskLedger}.ts`, `src/renderer/src/hooks/useHive.ts`, `src/renderer/src/scene/office/*`
- https://github.com/pixel-agents-hq/pixel-agents: `CONTEXT.md`, `server/src/{fileWatcher,transcriptParser,constants}.ts`, `server/src/providers/hook/claude/*`, `webview-ui/src/office/toolUtils.ts`
- https://github.com/BloopAI/vibe-kanban: `crates/executors/src/executors/{claude.rs,claude/types.rs,claude/protocol.rs,codex.rs,codex/client.rs,gemini.rs,copilot.rs,qwen.rs,opencode.rs,cursor.rs,amp.rs}`, `crates/executors/src/logs/mod.rs`, `crates/utils/src/log_msg.rs`, `crates/worktree-manager/src/worktree_manager.rs`, `crates/git/src/lib.rs`, `crates/db/migrations/*`, `crates/tauri-app/Cargo.toml`, `crates/mcp/src/task_server/tools/*`, `crates/services/src/services/{queued_message,approvals}.rs`
- https://github.com/smtg-ai/claude-squad: `session/tmux/tmux.go`, `session/git/*`, `session/storage.go`, `daemon/daemon.go`
- https://github.com/stravu/crystal: `main/src/services/panels/{claude/claudeCodeManager.ts,codex/codexManager.ts}`, `main/src/services/worktreeManager.ts`, `main/src/database/database.ts`
- https://github.com/agent-of-empires/agent-of-empires: `src/acp/{agent_registry,install_hints}.rs`, `src/status_hooks.rs`, `Cargo.toml`
- https://github.com/superset-sh/superset: `HOOKS_INVESTIGATION.md`, `LICENSE.md` (ELv2), `apps/desktop/package.json` (via `gh api`)
- READMEs via `gh api`: nimbalyst/nimbalyst, imbue-ai/sculptor, terragon-labs/terragon-oss, ruvnet/ruflo, aaif-goose/goose, agent-of-empires

**Primary: docs**
- https://munderdiffl.in/
- https://code.claude.com/docs/en/agent-teams, …/cross-session-messaging.md, …/agents.md, …/agent-view.md, …/hooks.md
- https://www.conductor.build/docs/, …/concepts/workspaces-and-branches, …/reference/scripts
- https://learn.chatgpt.com/docs/cloud (Codex cloud)
- https://www.vibekanban.com/blog/shutdown
- https://support.claude.com/en/articles/15036540 ("Use the Claude Agent SDK with your Claude plan", updates of 2026-06-15 and 2026-10-07)
- https://zed.dev/docs/ai/parallel-agents, https://zed.dev/docs/ai/external-agents.md
- https://docs.openhands.dev/sdk/arch/overview
- https://docs.cline.bot/kanban/overview
- https://docs.factory.ai/missions/overview
- https://ampcode.com/news

**Forum and secondary** [C]
- HN (via hn.algolia.com API): 49398152 (Munder Difflin), 44533004 (Vibe Kanban), 46368739 and 48236770 (Superset), 48126438 (Claude programmatic usage), 46902368 (agent teams), 46993479 (Ask HN orchestrators), 45489884, 47218318
- https://pasqualepillitteri.it/en/news/21541/anthropic-claude-max-team-api-credits (credits amounts, conflicting)
- https://warp.dev/newsroom/2026/2/10/warp-launches-oz-the-orchestration-platform-for-cloud-coding-agents
- Threadreader thread by Charlie Holtz (Conductor is Tauri)
- kilo.ai / rywalker.com on the Roo Code shutdown
- https://www.testingcatalog.com/cline-debuts-kanban-for-local-parallel-cli-coding-agents/
