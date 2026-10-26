# Munder Difflin: agent runtime (spawn, drive, observe, recover)

**Source.** Shallow clone of `munder-difflin` at `main`, commit `d3460def46bf145ea2667f0286ee4b82473f1b4c`. Latest dated release in the CHANGELOG is 0.4.6 (2026-08-27). Paths are relative to the repo root (`src/...`, `test/...`). Line numbers match this snapshot.

**Method and limits.** Read-only. Nothing was executed: no app scripts, binaries, npm commands, or tests. Behaviour is taken from code. Where a claim rests only on a code comment or a CHANGELOG entry, it is marked *self-reported*, because I could not reproduce it.

---

## 0. Summary

- **Every agent is an interactive TUI inside a node-pty PTY.** Nothing consumes `stream-json` or `codex exec --json`. The one print-mode CLI is Copilot (`-p`), and it exits after each turn. The app's own hidden helper sessions (memory condensation) also run as interactive PTYs, deliberately not `claude -p`.
- **Prompts go in as argv when the CLI allows it.** For Claude, the hive protocol is passed with `--append-system-prompt`. Mid-run text is typed into the PTY: text first, then a separate Enter, after a readiness check. Mail itself is plain files under `<hive>/agents/<id>/inbox|outbox`, so the PTY is only a doorbell.
- **State comes from a local hook bridge.** Claude gets per-session `--settings` hooks plus a statusLine. Other CLIs get config-file hook shims or a loopback proxy. Claude usage comes from an embedded OTLP collector. The TUI text is regex-scraped as a fallback.
- **Cost** comes from Claude OTel (live, cumulative) with a transcript-JSONL fallback priced by a model-family table. Grok reads its own `usage.json`. Qwen and Crush go through a proxy that synthesises cost samples. I found no usage parser for Codex, Gemini, agy, OpenCode, Pi, Copilot, Cursor or Kimi.
- **Reliability** rests on: a pure circuit-breaker policy (advisory by default), a pure worker-wake watchdog, process-group kills with a PID-recycling guard, session resume from recorded session ids, and a fenced login-shell PATH capture.

---

## 1. Supported CLIs and how each is launched

### 1.1 Provider table

Source of truth: `src/shared/agentProvider.ts`. The provider union is at lines 24-37 and the presets run from line 170 to 580. "Mail" means the CLI can receive hive inbox mail (`canReceiveInbox`).

| id | binary | run mode | auto (skip-permission) flag | model flag | how the hive protocol enters | hook or proxy bridge | resume | mail |
|---|---|---|---|---|---|---|---|---|
| claude | `claude` | TUI | `--permission-mode bypassPermissions` (176) | `--model` | `--append-system-prompt` (hive.ts:994) | native `--settings` hooks, statusLine, OTel env | `--resume <sid>` after the transcript is copied into the target cwd (index.ts:2871-2885) | yes |
| codex | `codex` | TUI | `-a never -s workspace-write` (210), keeps the OS sandbox | `--model` | positional argument, plus `--dangerously-bypass-hook-trust` and `--add-dir` for hive dirs (hive.ts:881-887) | per-agent `CODEX_HOME/config.toml` `[hooks]` pointing at the cth-hook shim (hive.ts:2193-2262) | `codex resume <sid>` subcommand (238-242; index.ts:2909-2931) | yes |
| grok | `grok` | TUI | `--permission-mode bypassPermissions` (254) | `--model` | positional (264) | Grok hook shim normalises camelCase payloads (`GROK_HOOK_SHIM`, hive.ts:3603) | `--resume <sid>` (266) | yes |
| kimi | `kimi` | TUI | `--auto` (275) | `--model` | none: no flag, no positional, no bridge, so the protocol is never sent (hive.ts:846-975) | none | none | no (283) |
| gemini | `gemini` | TUI | `--approval-mode=yolo` (294) | `--model` | `-i <prompt>` (302) | hooks via `GEMINI_CLI_SYSTEM_SETTINGS_PATH` (hive.ts:910-913) | `--resume` (304) | yes |
| antigravity | `agy` | TUI | `--dangerously-skip-permissions` (313) | `--model` | `-i <prompt>` (320) | agy translating hook shim, scoped by `AGENT_ID` (hive.ts:3173) | `--conversation <id>` (322) | yes |
| qwen | `qwen` | TUI | `--yolo` (333) | `--model` | `-i <prompt>` (343) | proxy sidecar; `OPENAI_BASE_URL` is rewritten to the loopback port (340; hive.ts:926-953) | none | yes |
| opencode | `opencode` | TUI | none. Permission allow is written into `OPENCODE_CONFIG_CONTENT`, and only when autoMode is on (index.ts:2997-3012) | `--model provider/model`, no default (393) | `--prompt <prompt>` (380) | plugin in `OPENCODE_CONFIG_DIR` (374; hive.ts:2429) | none (fresh spawn) | yes |
| crush | `crush` | bare TUI | `--yolo` (431) | `--model provider/model` | typed into the TUI after boot (`seedDelivery`, 457) | proxy sidecar plus `CRUSH_GLOBAL_CONFIG` (441; hive.ts:940-950) | `--session <id>` (458) | yes |
| pi | `pi` | TUI | `--approve` (476). Tool auto-allow is in the extension and gated by `HIVE_AUTO_APPROVE` (index.ts:2992-2994) | `--model provider/model` | positional (495) | extension installed into `PI_CODING_AGENT_DIR` (hive.ts:2392; 486) | `--session <id>` (496) | yes |
| copilot | `copilot` | print mode `-p`, exits per turn | `-s --allow-all-tools --no-ask-user` (515) | `--model` | `-p <prompt>` (520) | none | `--resume <id>`, best effort (524) | no (527) |
| cursor | `cursor-agent` (alias `agent`) | bare TUI | `--force --trust` (547) | `--model` | typed after boot (559) | none | `--resume` exists, but no session id is ever captured, so it is effectively unused (561) | yes (terminal delivery) |
| custom | user string | TUI | none | none | none | none | none | no (579) |

Provider inference from a command line: `inferAgentProvider` at agentProvider.ts:634-653.

### 1.2 Exact command lines

**Claude worker from the GUI, defaults (autoMode is true, config.ts:427).** Built in three steps:

1. The renderer builds `claude --model <m> --permission-mode bypassPermissions` (`buildSpawnCommand`, `src/renderer/src/store/config.ts:408-435`).
2. `spawnAgentCore` appends `--append-system-prompt "<protocol>" --settings <HIVE>/agents/<id>/settings.json` (hive.ts:994-1004). It then appends `--remote-control-session-name-prefix <AgentName>` (index.ts:2855-2858), `--max-turns N` when set (2861-2863), and `--resume <sid>` when a session is recorded (2871-2885). `--model` is added only if the argv does not already have one (2843-2848).
3. Environment (hive.ts:803-808, 981-989): `AGENT_ID`, `AGENT_NAME`, `HIVE_ROOT`, `AGENT_DIR`, `HIVE_NODE`, `HIVE_SOCK`, `COLORFGBG`, `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_METRICS_EXPORTER=otlp`, `OTEL_LOGS_EXPORTER=otlp`, `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`, `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:<port>`, `OTEL_METRIC_EXPORT_INTERVAL=5000`, `OTEL_LOGS_EXPORT_INTERVAL=2000`, `OTEL_RESOURCE_ATTRIBUTES=agent.id=...,agent.name=...`.

The Claude branch of `ensureAgent`, condensed (hive.ts:994-1004):

```ts
args.push('--append-system-prompt', this.injectedPrompt(meta, dir, root, /* … */));
const settingsPath = join(dir, 'settings.json');
this.writeJson(settingsPath, this.hookSettings(shim, meta.cwd, /* … */));
args.push('--settings', settingsPath);
```

**Codex worker (fresh):** `codex --model <m> -a never -s workspace-write --dangerously-bypass-hook-trust --add-dir <agentDir> --add-dir <hiveRoot> "<protocol>"`. Env: `CODEX_HOME=<agentDir>/.codex` and `CODEX_NON_INTERACTIVE=1` (agentProvider.ts:216). Resume is `codex resume <sid> ...` with the same tail.

**Gemini:** `gemini --model <m> --approval-mode=yolo -i "<protocol>"`. **Pi:** `pi --model <p/m> --approve "<protocol>"`. **OpenCode:** `opencode --model <p/m> --prompt "<protocol>"`, with `OPENCODE_CONFIG_DIR` and, when autoMode is on, `OPENCODE_CONFIG_CONTENT={"autoupdate":false,"permission":{"edit":"allow","bash":"allow","webfetch":"allow"}}`. **Copilot:** `copilot --model <m> -s --allow-all-tools --no-ask-user -p "<brief>"`. **Grok:** `grok --model <m> --permission-mode bypassPermissions "<protocol>"`.

**Model selection.** Per-agent `--model` from the renderer wins. For Claude, the fallback order is `cfg.defaultModel`, then `modelForRole` (index.ts:2843-2847). `modelForRole` (config.ts:763-776) returns the god's `godModel`, then the provider's `recommendedOrchestratorModel`, then `claude-opus-4-8` for the god. Triage-like roles get a helper model, and everything else gets `claude-sonnet-4-6`. For a god-spawned ephemeral worker, `buildWorkerLaunch` takes the command from the request, appends the provider's auto flag when there is no stance, splits the line into argv, and adds `--model` from `requestModel` (workerLaunch.ts:24-77).

---

## 2. Driving the agents

### 2.1 PTY layer (`src/main/pty.ts`)

- One node-pty per agent, created in `PtyManager.spawn` (pty.ts:552-736). The core call is `pty.spawn(file, spawnArgs, { name: 'xterm-256color', cols, rows, cwd, env: buildPtyEnv(...) })` (664-673). Default size is 100x30.
- **Identity guard.** Each spawn captures its `PtySession` object, and the callbacks check it with `if (this.sessions.get(opts.id) !== session) return;` (pty.ts:698, 710). Late `onData` or `onExit` events from a killed process whose id was reused by a respawn are dropped (698, 710). The comment explains the failure this prevents: the dying process would spray bytes into the new TUI and emit a false exit.
- **Tail ring.** The last 8 KB of output is kept (`TAIL_MAX`, pty.ts:40, 703). On exit the handler receives `{signal, tail, command, cwd}` (707-730). The comment notes that a provider killed by a signal can exit with code 0, so the exit code alone is not enough.
- **Writes are raw.** `write(id, data)` forwards the string to the PTY (738-747). `PtyManager` does no pacing and no bracketed paste. Pacing lives in the callers: the renderer (2.3), and the main-process watchdog, which waits 140 ms between its text and its Enter (index.ts:5185-5192).
- **Redraw.** `redraw(id)` resizes to the same size to force a repaint (763-772). It is used for the first attach, because startup output may predate the renderer's subscription.
- **Output routing.** Output goes only to the owning WebContents, so multiple floor windows do not see each other's terminals (`owner`, 56-61; `safeSend`, 385-391).
- **Renderer side.** `terminalPool.ts` keeps one xterm per PTY id (`acquireTerminal`, 124-141, `scrollback: 100000`). The pool exists because node-pty keeps no scrollback. Detached terminals keep their buffer and subscription.

### 2.2 Channels that put text into an agent

| channel | mechanism | where |
|---|---|---|
| spawn argv or system prompt | `--append-system-prompt`, positional, `-i`, `--prompt`, `-p` | hive.ts:846-994 |
| hook `additionalContext` | roster (god only), standing goal in `<goal>` (once per session, then on change), operator steer. Only on SessionStart, UserPromptSubmit, PostToolUse. No typing. | hooks.ts:521-580 |
| queued messages, renderer drain | `submitToPty`, one writer per PTY via a promise chain | useHive.ts:84-160 |
| inbox-wake nudge, main process | `nudgeWorker`: text, then `\r` 140 ms later. Does not consult the renderer's draft gate. | index.ts:5180-5193 |
| seed typed into a bare TUI | `seedPrompt` from the spawn result, for Crush and Cursor | index.ts:2777-2800; useHive.ts:465-468 |
| god boot | `/remote-control <name>`, then `INITIAL_GOD_PROMPT`. Skipped for a resumed god. | useHive.ts:455-470 |
| hidden helper sessions | bracketed paste, then `\r` 140 ms later, after boot goes quiet | hiddenClaude.ts:185-192 |

### 2.3 Typed delivery (the renderer's drain)

The write sequence at the core of `submitToPty` (useHive.ts:146-152), with the rules around it listed below:

```ts
const wrote = await window.cth.writePty(ptyId, payload);   // the text (bracketed only if multi-line)
if (!wrote?.ok) throw new Error(wrote?.error ?? `pty write failed: ${ptyId}`);
await new Promise((r) => setTimeout(r, 140));
const submitted = await window.cth.writePty(ptyId, '\r');  // Enter, a separate keystroke
```


- **One writer per PTY.** Every write for a PTY is appended to `writeChains`, so the boot sequence and the nudge cannot interleave (84-86). The comment records the failure this fixed: they were jamming onto one line ("Unknown command: /remote-control...").
- **Text and Enter are separate writes**, 140 ms apart. The comment (about lines 118-127) explains that one chunk containing both becomes a paste, and the Enter lands as a newline in the input box.
- **Bracketed paste only for multi-line text.** Single-line text is sent raw because agy treats paste markers as literal input and never submits (useHive.ts:136-141).
- **Readiness.** `waitForTerminalReady` polls `listPtys` every 100 ms for up to 30 s (useHive.ts:91-121). Ready means the PTY has produced output and the provider's settle time has passed: 650 ms for kimi, 500 ms for grok, gemini and codex, 400 ms otherwise (agentProvider.ts:229-237). Quiet output is not required, because Codex repaints its status line continuously (agentProvider.ts:239-257).
- **Drain gate** (`docs/message-queue.md` section 2, checked against code). The front message of a queue is sent only when all of these hold:
  - the target is `idle`, or `looping` (breaker-pinned) with its PTY quiet for `QUIESCE_IDLE_MS = 12000` (`canDeliverToAgent`, queueDelivery.ts:48-56, called at useHive.ts:810);
  - auto-delivery is not paused, unless the user pressed "send now" (`manual`);
  - the boot grace has passed, `BOOT_GRACE_MS = 35000` (useHive.ts:47);
  - `isTerminalAutomationSafe`: no user draft and no open picker;
  - at least `FLUSH_COOLDOWN_MS = 4500` since the last delivery to this agent (useHive.ts:784);
  - `checkPrecondition` still holds at delivery time. A stale nudge is dropped, not deferred.
  
  The message is acknowledged only after both writes succeed. After `MAX_SEND_ATTEMPTS = 3` failed writes it is dropped with a warning (useHive.ts:787-789). The drain runs on store changes (debounced 200 ms) and on a 3 s backstop tick (message-queue.md section 1; useHive.ts:952).
- **User draft and picker protection** (`terminalAutomation.ts`, `terminalPool.ts`). A draft is inferred from keystrokes, then checked against xterm's rendered prompt row (`promptLineHasText`, terminalPool.ts:454-466). The screen read can only clear a phantom draft, never create one, and reads within `ECHO_GRACE_MS = 1000` are ignored (terminalPool.ts:435). Both draft and picker blocks expire after 30 minutes (`STALE_INPUT_MS`, terminalAutomation.ts:50). Automatic delivery never sends Ctrl-U or Esc (message-queue.md section 3). Those keys go out only from explicit user buttons (terminalPool.ts:541, 567).

### 2.4 Idle: what the harness means by it

- **Hooks** (renderer, useHive.ts:495-520): `Stop` or `SubagentStop` sets idle. `PreToolUse`, `PostToolUse` and `UserPromptSubmit` set working. `PreCompact` sets compacting and `PostCompact` sets resumed.
- **Quiet output.** The PTY has been silent for 12 s (renderer `QUIESCE_IDLE_MS`, useHive.ts:43; main `WORKER_WAKE_IDLE_MS`, workerWake.ts:40).
- **Footer.** The text "esc to interrupt" is shown only during a turn, so its presence means running (usePtyParser.ts:105-106).
- **Boot grace** of 35 s after spawn, in both the renderer and the main-process watchdog (workerWake.ts:42).
- **Stop no longer forces continuation.** hooks.ts:490-500 returns `{}` at Stop, with the comment that forced continuation "bypassed terminal-draft/HITL safety and could spend credits while a user was answering a question". Inbox files stay durable, and the renderer wakes the agent later. `drainForStop` (hive.ts:1467) has no callers now. Comments that still describe a Stop-to-drain block are stale (agentProvider.ts:226-231; hive.ts:2179-2183).

### 2.5 Mail transport

- Each agent directory holds `inbox/`, `inbox/.done/`, `outbox/`, `outbox/.sent/`, `memory.md`, `cursor.json` and `identity.md` (hive.ts:742-770).
- A poll router moves outbox files to recipients' inboxes every 1.5 s (`startRouter`, hive.ts:1832). The comment says the poll was chosen as "cheap and robust vs fs.watch quirks on macOS" (hive.ts:1831).
- The harness, not the agent, fills in `id`, `from`, `hops` and timestamps. `hops` is capped to stop god and hookless-worker relay loops (hive.ts:1630-1667; test `hive-hop-cap`).
- The nudge text names the message ids, but only as diagnostics. It says the pending inbox directory stays authoritative (`src/shared/hiveNudge.ts:6-20`).
- Agents never run git on the hive. The main process is the only committer, with retry and stale-lock recovery (hive.ts:1-30).

---

## 3. How the app knows what an agent is doing

Sources, in order of precedence: hooks, then statusLine and OTel, then transcripts, then TUI scraping.

### 3.1 Hook bridge (primary)

**Claude.** `hookSettings()` (hive.ts:1245-1307) writes a per-agent `settings.json` that is passed with `--settings`:

- hooks: `Stop`, `SubagentStop`, `PreToolUse` (matcher `*`), `PostToolUse` (`*`), `UserPromptSubmit`, `Notification`, `SessionStart`, `PreCompact`, `PostCompact`. Each runs the bundled-node launcher on the shim (`nodeRun`, hive.ts:599-602).
- `statusLine: { type: 'command', command: '<shim> --status' }` (hive.ts:1277). Its payload carries `context_window.total_input_tokens` and `context_window_size`, which gives the real window size (200k or 1M). The comment says nothing else exposes that.
- `theme: 'auto'`, so the TUI follows theme changes (hive.ts:1258-1266).

**Shim** (`HOOK_SHIM`, hive.ts:3111-3150). It reads the payload from stdin, adds `agent_id` from `AGENT_ID` if missing, writes one JSON line to `HIVE_SOCK`, relays the reply to stdout and exits. It has a 5 s cap. In `--status` mode it prints `ctx 123k/200k (61%)` into the TUI and forwards the payload fire-and-forget.

**Server** (`HookServer`, `src/main/hooks.ts`). Newline-delimited JSON, one request per connection, 256 KB frame cap (hooks.ts:25, 301-309). `handle()` (383-597) routes each event, and `emit()` sends `hive:hookEvent` to the renderer after `validateHookEvent` (hookEvents.ts:15-30). The socket path is `<hive>/hooks.sock` on POSIX and `\\.\pipe\munder-difflin-<sha1-12>` on Windows (hive.ts:462-470).

**Other CLIs.** Each gets a config or plugin shim that writes the same payload shape to `HIVE_SOCK`:

- Codex: `[[hooks.<Event>]]` tables in a per-agent `config.toml` for PreToolUse, PostToolUse, Stop, SubagentStop, SessionStart, UserPromptSubmit, PreCompact and PostCompact (hive.ts:2213-2262). The comment says a bare `hooks.json` never fires (*self-reported*). `auth.json` is symlinked into the per-agent home (hive.ts:2199-2206).
- agy, Gemini and Grok: translating shims (`AGY_HOOK_SHIM` at hive.ts:3173, `GEMINI_HOOK_SHIM` at 3546, `GROK_HOOK_SHIM` at 3603).
- Pi: a bundled extension that posts on `tool_call` and `agent_end` (hive.ts:2392).
- OpenCode: a plugin that uses `session.idle` (hive.ts:2429). The comment marks its live behaviour as unverified.
- Qwen and Crush (proxy tier): a loopback sidecar tees each LLM response and synthesises `Status`, `PostToolUse`, `Stop` and `CostSample`. `Stop` fires after an 800 ms idle debounce, and a new request cancels it (hive.ts:3328-3480). The comment says idle is heuristic.

**Mapping to avatar status** (`useHive.ts:480-566`). `PreToolUse` with a tool sets `working` and `using <Tool>`, and picks the avatar station (`stationForTool`). A Notification is classified as needs-human (permission, approve, confirm, needs your) or idle-waiting (useHive.ts:535-546). A needs-human notification sets god to `blocked` and a worker to `waiting`. A breaker level of constrained or stopped pins the status to `looping` (useHive.ts:486-488, 556-566).

### 3.2 Transcript JSONL (secondary, read by polling)

- **Location.** `~/.claude/projects/<key>/<sessionId>.jsonl`. The key is the cwd with every non-alphanumeric character turned into `-` (transcript.ts:13-15). The legacy key is read only when the current one is absent (transcript.ts:43-56).
- **Path learned from hooks.** The `transcript_path` field of any hook payload is stored per agent (hooks.ts:387-389). `hive:agentContext` tail-reads the last 256 KB for the newest assistant usage (index.ts:3886-3890; transcript.ts:351-386).
- **Not watched.** A search for `fs.watch`, `watchFile` and `chokidar` in `src/main` finds only comments. Readers are triggered by polls: the breaker beat every 30 s, and telemetry queries. The per-file cache keys on size and mtime and parses only the appended tail (transcript.ts:255-300).
- **Other uses.** Session-id resume copies a transcript across cwds (`seedSessionTranscript`, transcript.ts:75-94). `resolveSessionCwd` reads the cwd from a transcript record (transcript.ts:103-130). The hidden condense call reads the last assistant text (hiddenClaude.ts:61-97).

### 3.3 OpenTelemetry (live usage and tool spans, Claude only)

- An embedded OTLP/HTTP-JSON collector bound to 127.0.0.1 on an ephemeral port (telemetry.ts:118-125, 254-259). It is injected only for Claude (hive.ts:981). Telemetry comments say other engines get nothing from it.
- `/v1/metrics` feeds usage. `/v1/logs` feeds `tool_result` spans (a ring of 200 per agent) and `api_error`, which is wired to `breaker.recordError` (telemetry.ts:287-291, 343-370; index.ts:282-283).
- Body cap of 8 MB (telemetry.ts:108). Only an allowlist of attribute keys is read, to keep PII out (telemetry.ts:542-560).

### 3.4 PTY output scraping (renderer fallback)

`usePtyParser.ts` reads the rendered TUI text and infers:

- tool lines of the form `● Read SPEC.md` (regex at line 6), turned into the action text `read spec.md`, with the station taken from the tool name (lines 8-14);
- approval or question prompts, matched by `BLOCK_HINTS` (lines 29-35, for example "Do you want to proceed" and `(y/n)`). For god this sets `blocked` with Approve and Deny buttons that would send `y\r` or `n\r` (about line 164). For a worker it sets `waiting`. The `BlockedBanner` component that would render those buttons is not imported anywhere in `src`, so they are dead in this snapshot;
- `awaiting`, set 4 s after the last tool line (usePtyParser.ts:57-66);
- the context limit, from `/context` output such as `235.3k/1m tokens` (line 41).

The parser's own doc comment calls it "a stopgap until we wire real Claude Code hooks". The hook path overrides it, and the parser only refines the on-floor action and station (useHive.ts:484-485).

### 3.5 Status vocabulary

- From hooks: `using Read`, `thinking` (agy PreInvocation), `compacting context`, `resumed`, `idle`, `looping`, `waiting on god`, `waiting on you`.
- From the TUI parser: `read spec.md`, `bash npm test`, `awaiting`.
- `running tests` appears only in demo data (`store/mockEvents.ts:33`). No test-run classifier exists.

---

## 4. Permissions and approvals

**4.1 Auto mode is the default.** `autoMode: true` (config.ts:427). When it is on, each provider's skip flag is applied:

- Claude: `--permission-mode bypassPermissions`. `argsWithAutoModeFlag` (agentProvider.ts:686-692) appends it idempotently, but only when argv has no explicit stance (`hasAutoModeStance`, 697-703). The main-process path exists because a worker spawned without the flag "deadlocked", since a cross-session message came back "held for the recipient user's approval" with nobody able to grant it (index.ts:2835-2837).
- Codex: `-a never -s workspace-write`. The OS sandbox stays on, and hive directories are added with `--add-dir`. The comment is explicit that the old `--dangerously-bypass-approvals-and-sandbox` was dropped (agentProvider.ts:201-211).
- OpenCode: a permission allow map inside `OPENCODE_CONFIG_CONTENT`, built only when autoMode is on (index.ts:2997-3012). The preset comment ties this to the floor `config.autoMode` toggle (agentProvider.ts:357-361).
- Pi: the extension auto-allows tools only when `HIVE_AUTO_APPROVE=1` (index.ts:2992-2994).
- Copilot: `--allow-all-tools --no-ask-user`. Kimi: `--auto`.

**4.2 Claude's first-run dialogs are pre-accepted globally.** `ensureClaudePermissionsAccepted(cwd)` runs before every Claude spawn (index.ts:2954-2956; config.ts:876-885). It writes `~/.claude/settings.json` (`skipDangerousModePermissionPrompt`, `skipAutoPermissionPrompt`) and `~/.claude.json` (`projects[cwd].hasTrustDialogAccepted`) when those are not already set. Its doc comment says that without this the bypass-mode warning makes the agent exit with code 1. Preserving malformed files byte-for-byte is tested (`claude-permissions-config` test). This mutates the user's global Claude config, so treat it as a design decision, not a free default.

**4.3 Claude's own sandbox is switched on per session.** For a worker with writable dirs, `hookSettings` sets `sandbox: { enabled: true, filesystem: { allowWrite } }` and `permissions.additionalDirectories` (hive.ts:1287-1296). The comment says bypass mode silences prompts only, while the sandbox is a separate layer. It notes that both layers are needed or the agent deadlocks on its own inbox.

**4.4 Operator gate (human-in-the-loop, pre-approved).** At `PreToolUse`, HookServer denies the call if the agent is paused or the tool is gated, and returns `hookSpecificOutput.permissionDecision: 'deny'` with a reason (hooks.ts:506-519; `control.toolDecision`, control.ts:107-113). It also sends `control:approvalRequest` to the renderer (hooks.ts:612-614). This is the only tool-level gate the app enforces itself.

**4.5 Human approvals in general** go through Claude's own permission prompt in the agent's terminal. They can be answered remotely through `/remote-control`. The app types `/remote-control <name>` into god at boot (useHive.ts:455-462), and gives each Claude worker a session-name prefix (index.ts:2855-2858). The hive has no approval queue. A message aimed at `human` is routed to god (hive.ts:15-17, 2994-2998). Notifications are off by default (config.ts:446).

**4.6 Fail-open.** When the hook socket is not listening, the shim's `connect()` fails and it exits 0 with empty stdout, which Claude reads as allow. The comment says "every agent hook is ALLOWED and no cost is recorded" (hooks.ts:55-60, 262). The operator gate and the breaker both go inert in that state. The app surfaces this (`hive:degraded`, fleet health, a toast after the back-off is spent) but does not fail closed.

**4.7 Codex hook trust.** `--dangerously-bypass-hook-trust` is passed so Codex runs the hive-authored hooks without an interactive trust prompt (hive.ts:873-881). The comment justifies this as "automation that already vets hook sources".

---

## 5. Cost and usage tracking

### 5.1 Sources

| source | providers | where | nature | persisted? |
|---|---|---|---|---|
| OTel metrics (live) | Claude | `telemetry.ts:305-340`. `claude_code.token.usage` by type (input, output, cacheRead, cacheCreation) and `claude_code.cost.usage` to `usd`. The comment says these are "DELTA + monotonic", so each export is summed. | cumulative since process start, in memory | via the ledger (below) |
| transcript fallback | Claude, before telemetry or when telemetry is off | `telemetry.ts:439-460` calls `readAgentUsage(cwd, {sessionId})` (transcript.ts:313-337). Per-record pricing (transcript.ts:222-227). | cumulative, recomputed from disk | no. `sessionId` is set to `''`, which keeps it out of the ledger (index.ts:1224). |
| Grok `usage.json` | Grok | `telemetry.ts:485-500`, reads `~/.grok/sessions/<cwd>/<sid>/usage.json` | cumulative file snapshot | yes, deduplicated by `CumulativeSampleGate` (usage.ts:122-140) |
| proxy `CostSample` | Qwen, Crush | `hive.ts:3436-3438` reads OpenAI-shape `prompt_tokens`, `completion_tokens`, and `hooks.ts:439-463` prices them with `estimateCostUsd` | per response | yes |
| none found | Codex, Gemini, agy, OpenCode, Pi, Copilot, Cursor, Kimi | no code reads Codex rollout token counts, Gemini `usageMetadata`, or similar | n/a | n/a |

The usage provider is the telemetry collector (`const usageProvider: UsageProvider = telemetry`, index.ts:262). The `StubUsageProvider` in `usage.ts` is a legacy interim. `getAgentUsage` tries live OTel first, then Grok, then the transcript (telemetry.ts:191-199).

### 5.2 Pricing

- **Live path.** The app trusts Claude's own cost. The file header says the live telemetry path "does NOT use this" table (pricing.ts:4-9).
- **Fallback table** (pricing.ts:27-29), USD per million tokens, input / output / cache read / cache write:
  - Opus: 15 / 75 / 1.5 / 18.75
  - Sonnet: 3 / 15 / 0.3 / 3.75
  - Haiku: 0.8 / 4 / 0.08 / 1.0
- **Matching** is by family substring: `opus`, `haiku`, `sonnet`. Anything else falls back to Sonnet (pricing.ts:45-51). A `[1m]` suffix is stripped first (pricing.ts:40-42).
- **Estimate** `estimateCostUsd` (pricing.ts:65-73) is applied per assistant record, using that record's own model, so a mixed-model agent is priced correctly (transcript.ts:222-227). The original bug was Sonnet pricing for everyone, which the comment says left Opus about 5x undercosted (pricing.ts:11-12).

### 5.3 Ledger and lifetime totals

- **Ledger.** Append-only `<hive>/cost-ledger.jsonl`, one row per sample: `agent_id, session_id, ts, input, output, cache_read, cache_creation, model, usd`. The snake_case columns are chosen so a later SQLite migration is a straight insert (hive.ts:2773-2787).
- **Dedup gate.** A row is written only when `sample.sessionId` is truthy (index.ts:1224-1231). The comment records 2,417 identical rows left behind by re-appending a cumulative sample (usage.ts:107-110). The index.ts comment ties the rewrite to the 30 s breaker beat (index.ts:1217-1220).
- **Restart problem.** Live counters restart at zero after an app restart, under the same session id. Reading the last value therefore "hid 59% of real spend" on a live ledger (costLifetime.ts:6-15).
- **Fix.** `CostLedgerTotals` folds the ledger incrementally, reading at most 8 MB per pass (costLifetime.ts:58, 123-149). Any decrease within one (agent, session) means a reset, and the previous peak is committed (172-178). Lifetime is the sum of committed and open peaks (`recompute`, 181-188). It reports `null` until it has been warm, so a cold zero is never shown as fact (78-81). The comment rejects a dollar threshold because restarts often drop straight to zero.

The reset rule, verbatim in shape (costLifetime.ts:172-178):

```ts
if (usd < s.peak - EPS) {          // counter went backwards: the previous segment ended at its peak
  s.committed += s.peak;
  s.peak = usd;
} else if (usd > s.peak) {
  s.peak = usd;
}
```

### 5.4 Consumers

- **Circuit breaker**, every 30 s (index.ts:1199-1277, 5283). It uses `costCapUsd` floor-wide, blaming the single biggest spender (breaker.ts:271-282). It also has a floor-wide token cap, a per-agent work-token cap (below), and a velocity check: Δoutput over Δt of cumulative samples (breaker.ts:372-379).
- **Per-agent work tokens** are `input + output + cacheCreation`, and exclude cache reads. The comment explains that cache reads re-bill on every request and grow with session length, not with work (breaker.ts:79-87, 346-358, issue #189).
- **Fleet snapshot** `<hive>/fleet.json`, every 8 s (index.ts:5281). God reads it instead of `claude agents`, which cannot see the hive's sessions (useHive.ts:75-85).
- **Floor UI.** Cost HUD and per-agent usage, through the lifetime totals.

---

## 6. Reliability

### 6.1 Hook socket health (`hooks.ts`)

- **Ownership probe.** Before using a path, the server connects and sends `{ping: nonce}`. It accepts only a reply that echoes its own nonce and instance id (hooks.ts:315-337). A stranger's live socket is never stolen (220).
- **Orphans, not close.** When a stranger takes the path, the old listener is detached and kept unref'd rather than closed. The comment says libuv unlinks the path by name on close, which would delete the stranger's socket (hooks.ts:339-357). In Rust you would remove the file yourself, so the same check applies.
- **Retry and verify.** Bind retries back off 500 ms to 8 s (hooks.ts:83). The beat re-verifies every 15 s (index.ts:5173, 5291). Health is written into `fleet.json` (hooks.ts:61-78).
- **Framing.** A 256 KB cap, with reject-and-log on oversize frames (hooks.ts:25, 280-300). Multibyte payloads split across chunks are tested (`hooks-framing`).

### 6.2 PTY exit and teardown

- The exit handler records abnormal deaths before teardown (index.ts:595-608). `recordAgentExit` dumps the output tail to `crashes/`, which is gitignored on purpose because it may contain secrets (hive.ts:649-656).
- `teardownPty` (index.ts:443-470) is idempotent. It revokes integration capabilities, drops watchdog and breaker state, forgets telemetry, stops the proxy sidecar, archives the agent, and removes the worktree only when it is safe. The comment says every kill site must call it explicitly, because `kill()` removes the session before the async exit fires, so the exit path is skipped.
- `killByOwner` runs when a floor window closes. `killAll` runs on quit and reset. On Windows it sweeps the tree synchronously, because a deferred sweep never runs during quit (pty.ts:811-843).

### 6.3 Process-tree kill (`procKill.ts`)

- A graceful kill is followed, after `KILL_GRACE_MS = 4000` (procKill.ts:23), by a SIGKILL to the process group on POSIX or `taskkill /T /F` on Windows (`ensureKilled`, 96-108; `hardKillTree`, 34-43).
- **PID-recycling guard.** Before the sweep, the group is re-snapshotted with `ps -axo pid=,pgid=,lstart=` (`groupMembers`, 49-62). The sweep runs only if at least one original member still has the same start time (`shouldSweep`, 77-87). The comment describes the hazard as seen live on 2026-08-30 (incident T1007): a recycled pgid would let the delayed group SIGKILL hit an unrelated process group.
- **Scope.** Sweeps run only on explicit kills, never on natural exit, so a dev server an agent deliberately left running survives (procKill.ts:15-18).

### 6.4 Worker wake watchdog (`workerWake.ts`)

Runs every 15 s in the main process (index.ts:5171, 5285), as a backstop for the renderer's typed nudge. A worker is nudged only if all of these hold, checked in this order by `explain()` (workerWake.ts:254-276):

1. it has undrained inbox mail;
2. it is not god and has a PTY;
3. neither `autoDeliveryPaused`, `paused` nor `halted` is set (ControlRegistry flags);
4. the PTY has produced output, and has been quiet for at least 12 s, unless the worker is stalled;
5. the boot grace (35 s since spawn) has passed;
6. no needs-human notification in the last 5 minutes (`HITL_REARM_MS`);
7. the same inbox ids were not already announced, unless the worker is stalled;
8. at least 60 s since the last nudge (cooldown).

**Stall rule.** Mail older than 90 s with no turn since it landed means the worker is stalled. A turn is a tool span, a usage sample with tokens, or a turn-proving hook event (`isTurnHook`, workerWake.ts:98-112). A zero-token boot sample does not count (workerWake.ts:176-186). The rule is off for an agent with neither telemetry nor any hook event, so a hook-less CLI is not flagged as stalled forever (workerWake.ts:59-66, 231-236).

The beat logs why a worker with old mail is held, once per cooldown (index.ts:5260-5270). The nudge text comes from `inboxNudgeText(ids)`, the same builder the renderer uses (index.ts:5180-5185; `src/shared/hiveNudge.ts`). The `WORKER_WAKE_NUDGE` constant (workerWake.ts:36-37) is kept in step with it by a test.

### 6.5 Circuit breaker (`breaker.ts`)

- **Pure policy.** Levels are `healthy`, `steering`, `constrained`, `stopped` (breaker.ts:31). The caller enforces. Escalation moves one level per beat, and recovery moves one level down per healthy beat (tick, 306-316).
- **Ceiling.** `hardStop` defaults to false (breaker.ts:91). The ceiling line is `const ceiling: BreakerLevel = cfg.hardStop ? 'stopped' : 'constrained';` (breaker.ts:306), so by default the breaker never kills on its own. The config comment says the same (config.ts:147).
- **Which arms can fire depends on the engine.** Velocity and no-progress need a usage sample (breaker.ts:372), and error storms need OTel `api_error` events (index.ts:283). Engines without a usage source, such as Codex, get only the loop check. See pitfall 24.
- **Enforcement** (index.ts:1264-1275). `steer` sends mail ("re-check your approach"). `constrain` sends mail and a toast ("stop active work now"). Neither blocks a tool. `stop` kills the PTY and tears it down, and runs only when the level reaches `stopped`.
- **Triggers** (breaker.ts:339-411): identical tool call 8 times in a row (`repeatedToolLimit`), 5 consecutive API errors (`errorStormLimit`), per-agent work-token cap, floor cost and token caps, output velocity above 60,000 tokens/min, and a no-progress arm. The no-progress arm needs two consecutive beats and is exempt while a recent distinct tool call, a recently changed working directory, or a human prompt in the last 5 minutes exists.
- **Exemptions.** Compaction is exempt from the output-velocity and no-progress arms, with a 5 minute cap if `PostCompact` never arrives (breaker.ts:97-102, 211-226). The human-prompt clock covers a live conversation (#376, #425).
- **Repeat key** is a SHA-256 of the input, after truncating each string to 4096 characters (breaker.ts:228-250). The comment records a 200-character prefix that made distinct Bash calls look identical (#377).

### 6.6 Ephemeral workers (god-triggered)

- God drops a JSON spawn request into `HIVE_ROOT/spawn-requests/` (index.ts:4581-4625). Main polls it every 1.5 s, using atomic rename for archiving. It runs `processSpawnRequest` (index.ts:4689) through the same `spawnAgentCore`, dispatches the objective to the inbox, and watches each worker for a terminal `act: "done"`.
- Defaults: at most 4 concurrent workers, reaped after 20 minutes without output, and a per-worker token cap that is off by default (config.ts:439-441). An idle worker is reaped (index.ts:4969-4979), and a worker over its token cap is reaped (index.ts:4951-4965).
- A `done` signal does not wait for unread mail. Any unread messages are filed under `inbox/.done` and god is told which ones went unanswered (index.ts:4886-4940).

### 6.7 Restart and resume

- **Recording.** A Claude session id is recorded from any hook payload (hooks.ts:431 → `recordSession`, hive.ts:1208-1221). It is also taken from the usage sample (index.ts:1239), as a second source. The CHANGELOG (0.4.4) describes the earlier state, where "Restart & Continue" had nothing to resume. Respawns keep `sessionId` because `ensureAgent` spreads the prior registry entry (hive.ts:772-792).
- **Restore on boot.** `restoreTeam` (`src/renderer/src/hooks/useRestoreTeam.ts:72-192`) spawns every restorable agent concurrently with `resume: true`. If the agent's worktree still exists it re-enters it, otherwise it falls back to the base repo. An "already exists" error retires the agent rather than failing. Results are added in roster order, not completion order.
- **Claude resume.** `--resume <sid>` is attached only if `seedSessionTranscript` succeeds (index.ts:2871-2885). Otherwise the agent starts fresh. `requireResume` refuses to start a replacement at all (index.ts:2935-2941). `resumeNotFound` tells the Add Agent dialog that it started fresh.
- **Other CLIs.** The generic path uses `resumeFlag` or `resumeSubcommand` (index.ts:2896-2934). For Codex, `findCodexHomeForSession` locates the per-agent home that owns the session, because the rollout and the sqlite index live together there (index.ts:2539-2590).
- **Startup migration.** `archiveOrphanedAgents` archives agents marked live with no PTY (index.ts:906-918).

### 6.8 Sleep and wake

On resume from sleep, `onSystemResume` re-arms the scheduler, the always-on beats, the router and the keep-awake blocker (index.ts:5341-5372). It also flushes queued mail. After 15 s it runs `healthCheckPtys`, which probes each PID with `kill(pid, 0)`. It notifies about dead PTYs and emits `power:resume`, but it does not respawn anything (index.ts:5310-5340). The comment says macOS can wedge a child process and socket across a long sleep while node-pty still reports it alive.

### 6.9 Missing CLI: the install ladder

If the engine binary is absent, `spawnAgentCore` runs an installer in the agent's own terminal (index.ts:2666-2720). It relaunches the spawn on a clean exit (index.ts:595-625). The ladder (`chooseInstallRung`, cliInstall.ts:40-44) is: npm, then Node-then-npm, then the vendor's native installer, then a manual hint. Commands are trusted constants, and nothing runs when no installer exists. Claude's native installer is `curl -fsSL https://claude.ai/install.sh | bash`. Cursor's is `curl https://cursor.com/install -fsS | bash` (agentProvider.ts:190-193, 564-567).

### 6.10 PATH and environment for a GUI app

- **Login shell, fenced.** `captureFromLoginShell` runs `$SHELL -ilc` with a marker before and after the script, so rc-file chatter cannot poison the captured value (shellEnv.ts:32-56). It is a blocking `spawnSync` with a 3 s timeout, and it is memoised, caching only successes (shellEnv.ts:18-38).
- **User PATH.** `userShellPath` is captured once. A multi-line result counts as rc noise and falls back to the process PATH (shellEnv.ts:59-72).
- **Command resolution.** `resolveCommand` tries `which` in the login shell, then fixed candidates (`/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `~/.claude/local`, `~/.volta/bin`), then gives up (pty.ts:437-493). Only successful lookups are cached, so a just-installed binary is seen on the next check (pty.ts:411-435). The name is validated by `isSafeCommandName` (`^[A-Za-z0-9._+-]+$`, shellEnv.ts:83-85) before it can reach a shell.
- **Bundled node.** `withHiveRuntimeFallback` appends `<HIVE_ROOT>/bin/runtime` to PATH, so a `node` shim is available when the user has none. Appending, not prepending, is deliberate (pty.ts:16-35).
- **Child env** (`buildPtyEnv`, ptyEnv.ts:46-90). It inherits everything except variables matching `/^CLAUDE(CODE|_)/`, which removes the parent session's identity. Four configuration keys survive (`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`). It then sets `PATH`, `TERM=xterm-256color`, `COLORTERM`, `FORCE_COLOR`, and, on non-Windows platforms, `LANG` and `LC_CTYPE` to UTF-8. The comment explains that a Finder-launched app otherwise gets MacRoman and mangles output.
- **Windows.** Resolution uses `where`, honours `PATHEXT`, and checks known install locations, including `%LOCALAPPDATA%\agy\bin\agy.exe` (shellEnv.ts:96-140; pty.ts:445-474). Prompt-carrying spawns decode the npm `.cmd` shim to its interpreter and script, then pass an argv array (pty.ts:227-329, 510-550). Only undecodable shims fall back to `cmd.exe`, with a loud warning when a multi-line argument would be truncated (pty.ts:653-662).

---

## 7. Pitfalls and bugs they hit

Each item gives the symptom, the cause, the fix, and the source. A "CHANGELOG 0.x.y" reference points at the release-note entry under that version.

**Spawn and CLI quirks**

1. **Windows cmd.exe truncates the hive protocol.** A `.cmd` target ran through `cmd.exe /c` cut the argument at its first newline, so agents booted but never learned they had an inbox (pty.ts:114-138, 140-152; CHANGELOG 0.4.4). Fix: decode the npm shim and spawn its interpreter with argv. Direct-executable shims (OpenCode) needed a separate case.
2. **A full command line passed as the binary name** gave ENOENT, and the worker died within a second while its request was archived as done (workerLaunch.ts:60-66). Fix: tokenise and pass flags as argv.
3. **Codex has no `--resume` flag.** Restarts silently started new sessions. Fix: use the `codex resume` subcommand, with the id before the positional prompt. The comment records the failure mode when the order was reversed: the prompt was read as the session id (agentProvider.ts:238-242; index.ts:2922-2931; CHANGELOG 0.3.4).
4. **Codex hook timeout is in seconds.** `timeout = 0` is floored to one second, so every session start timed out (hive.ts:2213-2224). Fix: 30 s.
5. **Codex config placement.** Hooks in `hooks.json` never fire, and `codex exec` does not fire hooks (hive.ts:2216-2227; *self-reported*). Hooks go in `config.toml` `[hooks]` tables, used only in interactive sessions.
6. **Bare TUIs with a Cobra-style parser.** A positional prompt is read as a subcommand. Crush fails with `unknown command "You are…"`. Fix: type the protocol after boot (agentProvider.ts:450-457, 555-559).
7. **Typing `/compact` into Crush** opens its command palette and swallows input. The table records `null` for Crush (providerAutomation.ts:119-124).
8. **OpenCode silently ran a different model.** A preselected BYOK slug the user had not authenticated fell back to another model while the UI showed Sonnet. Fix: pass no default `--model` (agentProvider.ts:381-392).
9. **Bypass mode exits with code 1** on a fresh machine because of a first-run warning and a folder-trust dialog. Fix: pre-accept them (config.ts:862-875; section 4.2).
10. **A worker spawned without the auto flag deadlocks**, because its approval prompts go nowhere (index.ts:2835-2837).
11. **Hook commands fail under `sh -c` with a stripped PATH** when `node` is missing. Fix: the bundled launcher (hive.ts:1245-1248, 599-602). Running the Electron binary as `node` opens a second app window, so the launcher must be a wrapper (hive.ts:809-812).
12. **Unix socket path limit.** macOS caps `sun_path` at 104 bytes. Codex's control socket overflowed it when `$TMPDIR` was used, so the alias root became `/tmp/mdc` (codexRemote.ts:7-17). The hook socket at `<harnessHome>/hive/hooks.sock` has no length check that I could find, so a long home path could fail to bind (hive.ts:462-470). This is a risk I did not reproduce.
13. **Prompt size near cmd.exe limits.** The protocol is about 6.1 KB, against cmd.exe's 8,191-character ceiling (pty.ts:128-133).

**Transcripts, usage and cost**

14. **Wrong Claude project key for months.** `projectDir()` used the pre-2026 key, dropping the leading slash. Claude Code had moved to dashing every non-alphanumeric. Every reader saw "no transcripts yet", so memory condensation never succeeded (transcript.ts:6-42; CHANGELOG 0.3.8). The comment adds that a fallback-first resolver would keep reading the app's own stale copies.
15. **Inherited `CLAUDE_CODE_CHILD_SESSION` disables transcript saving**, which breaks `--resume` for every agent. Fix: strip parent markers by prefix (ptyEnv.ts:12-22; 2026-08 incident per the comment).
16. **Hidden helper sessions do not strip the same markers.** `runHiddenClaude` builds its env from raw `process.env` plus `userShellPath()` (hiddenClaude.ts:140-144), not through `buildPtyEnv`. If the app was launched from inside a Claude Code session, a condensation session could inherit the marker, write no transcript, and fail with "no assistant response". I found no test for this path, so treat it as a likely gap.
17. **Shared cwd double-counts usage.** Every agent in one repo shares one transcript directory. Summing it all gave one probe worker with zero LLM calls a reported total of 143,369,766 tokens (telemetry.ts D11 comment; transcript.ts:147-153). Fix: filter by session id, and report "no data" until one is known.
18. **Sonnet prices for every model.** Opus was undercosted by about 5x and Haiku overcosted (pricing.ts:11-12, which calls this "cost bug #1").
19. **Cumulative counters reset on restart.** Reading the last value hid 59% of spend on a live ledger (costLifetime.ts:6-15).
20. **Duplicate ledger rows.** Re-appending an unchanged cumulative sample every 30 s wrote 2,417 identical rows (usage.ts:107-110; index.ts:1217-1224).
21. **Cache reads dominate token caps.** A cap on all tokens tripped four to six minutes into a session, because re-read context dominated (breaker.ts:79-87 comment, which cites two agents at a 4M cap). Fix: count work tokens only (breaker.ts:79-87, 346-358; issue #189).
22. **Output-velocity false trips during compaction** (breaker.ts:97-102, 211-226; issue #109).
23. **A 200-character prefix as the repeat key** made nine distinct Bash commands look identical and constrained the agent. Fix: hash capped inputs (breaker.ts:235-239; issue #377).
24. **Breaker arms go blind for engines without a usage source.** The velocity and no-progress arms, and the token and cost caps, need a usage sample (`input.sample && s.lastSample`, breaker.ts:372; caps use `sample`, breaker.ts:352-366). `getAgentUsage` returns null for Codex and the other engines in section 5.1, so those arms never trip for them. The error-storm arm needs OTel `api_error` events, so it is Claude-only (index.ts:283). Loop and hook-based checks still work. This is inferred from the code, not reproduced.

**Input injection and state**

25. **Text and Enter in one write** become a paste, and the Enter is swallowed (useHive.ts:118-127). Fix: separate writes with a settle.
26. **Multi-line text without bracketed paste** submits line by line, so the agent sees only the last fragment (issue #24; useHive.ts:136-141). But agy treats paste markers as literal, so only multi-line text is wrapped.
27. **Phantom drafts** came from counting keystrokes. A TUI that swallows keys for its own menu left the queue blocked against text that did not exist. Fix: read xterm's rendered prompt row, one-directionally (terminalPool.ts:417-466; message-queue.md section 4; CHANGELOG 0.3.4).
28. **Clearing a draft with Ctrl-U destroyed real text.** Expiry now types after the existing text and never erases it (message-queue.md section 3).
29. **A worker stalled 17 minutes** with mail waiting and no nudge, while the watchdog stayed silent (workerWake.ts:47-57; issue #151). Causes: a zero-token boot sample counted as activity (workerWake.ts:176-179), and a stall rule that was off for hook-only engines (issue #368). Fixes are the stall rule and the fail-closed observability gate.
30. **Chatty TUIs never go quiet.** Requiring quiet output blocked queued messages on Codex until every attempt timed out (agentProvider.ts:239-247). Fix: ready after the first frame plus a settle.
31. **Stop-hook forced continuation** bypassed terminal drafts and human HITL, and spent credits while a user was answering a question (hooks.ts:493-496). Removed. Dead code and stale comments remain (see 2.4).
32. **Two automatic writers.** The main-process nudge (index.ts:5180-5193) types into a PTY without the renderer's draft and picker gate. `docs/message-queue.md` section 1 says the drain is the single writer, with only the god boot sequence as an exception, so that document is out of date.
33. **Hooks down means allow-all**, with no cost recorded and the breaker inert (hooks.ts:55-60, 262-263; section 4.6).
34. **Pgid recycling.** A delayed group SIGKILL after the group had emptied could hit an unrelated process group. Fix: the start-time snapshot guard (procKill.ts:64-76; incident T1007 in the comment).
35. **SIGHUP alone leaves orphans.** A child that traps SIGHUP lived on, and its MCP servers were reparented to PID 1 (procKill.ts:4-9; CHANGELOG 0.3.4, issue #110).
36. **Browser context limit.** WebGL allows about 16 live contexts. Restoring a team exceeded that, and Chromium killed a background terminal's renderer while its PTY stayed healthy. Fix: a lease, taken on attach and released on detach (CHANGELOG 0.3.4).
37. **macOS sleep** freezes libuv timers and can leave a PTY wedged. The app re-arms timers on resume and probes PIDs (index.ts:5296-5372).
38. **Lock-step with the window.** Renderer-only drains stop when the window is throttled or occluded. That is why the main-process watchdog exists (workerWake.ts:4-9).
39. **Hive relay loops** between god and hookless workers, bounded by `hops` (hive.ts:1630-1667; `hive-hop-cap` test).
40. **Antigravity's `Stop` fires only on process exit**, so it never reads as idle. The app treats `PostInvocation` as the turn boundary instead (useHive.ts:506-511).
41. **Terminal path tokens are hostile input.** Printed paths can be turned into links, so the app reveals files in the OS browser instead of opening them, to avoid a printed `.dmg` or `.app` becoming an execution (terminalPaths.ts:14-19).

**Stale or dead code to avoid copying**

- `drainForStop` (hive.ts:1467) has no callers.
- `BlockedBanner.tsx` is not imported, so the Approve and Deny injection is dead.
- `StubUsageProvider` is superseded by the telemetry collector.
- `unwrapCliEnvelope` (reflect.ts:419-431) is still called by `parseSummary` (reflect.ts:509), but it only matters for the older `claude -p --output-format json` envelope. Current hidden sessions return plain text.
- The Codex comments about Stop-to-drain are stale (agentProvider.ts:226-231), and so is the agy `canReceiveInbox` comment at line 319, which also cites Stop-to-drain.

---

## 8. What to steal and what to do better (Rust and Tauri)

### 8.1 Steal

1. **Hook-first state with a scrape fallback.** Authoritative typed events come first, and TUI text is a fallback. Keep the enum of states small and explicit.
2. **Per-session injected settings and env, never edits to the user's config.** Use `--settings <file>` for Claude, `AGENT_ID` and `HIVE_SOCK` in the environment, and a statusLine for exact context-window accounting.
3. **A local hook server with a safe protocol.** Newline-delimited JSON, one request per connection, a frame byte cap, a ping and nonce to prove ownership, bind retry with back-off, and health written to a file the UI reads. In Rust: `tokio::net::UnixListener` on POSIX, a named pipe on Windows.
4. **A loopback OTLP collector** for Claude usage, reading only an allowlist of attributes, summing deltas.
5. **Transcript tailing with offsets.** Cache per file by size and mtime, parse only appended complete lines, keep a torn last line unconsumed, and filter by session id.
6. **Mail as files, PTY as doorbell.** Keep inbox and outbox on disk. Use edge-triggered nudges by message id, a cooldown, a boot grace, and a re-arm after a human-approval prompt.
7. **Pure policy modules with tests.** The breaker, the wake watchdog, the terminal-automation gate and the provider-automation table are all pure. Their unit tests are good examples for property tests.
8. **A provider table as data, with an exhaustive type.** The TypeScript `Record<AgentProvider, …>` forces every new CLI to be classified. In Rust, an exhaustive `match` over an enum does the same job.
9. **Process-group kills with a start-time guard**, and explicit kills only. On Windows, consider a Job Object with kill-on-close instead of `taskkill` (`windows` crate).
10. **A visible, trusted-constant installer ladder** that relaunches after success and never runs unknown commands.
11. **Fenced login-shell PATH capture,** cached, with positive results only. Append the bundled runtime rather than prepend it.
12. **Environment hygiene by prefix** with a keep-list for operator configuration.
13. **No shell in the middle on Windows.** Decode `.cmd` shims to an interpreter and argv. Refuse multi-line arguments through `cmd.exe`.
14. **Reveal, do not open,** path tokens that come from agent output.
15. **Resume bookkeeping.** Record the session id from hooks, copy transcripts across cwds, and refuse a silent fresh start when the caller required a resume.

### 8.2 Do better

1. **One writer per PTY, enforced in the backend.** Route every automatic write, including watchdog nudges and boot sequences, through a single Rust actor that owns the queue. Put the draft and picker check there too. To read the prompt line in the backend, run a terminal emulator on the PTY stream (for example the `vt100` crate or `alacritty_terminal`), instead of depending on the webview's xterm.
2. **Fail closed on the hook channel.** When hooks are down, pause or deny rather than allow. Surface the state in the UI, and do not let the operator gate go inert silently.
3. **Do not mutate the user's global CLI config.** If a pre-accept is needed, make it an explicit, reversible, opt-in step with a backup. Prefer flags and per-session files.
4. **Persist usage as append-only per-request deltas**, keyed by session and request id, so writes are idempotent and no reset logic is needed. Keep the transcript fallback for CLIs without telemetry. Show "unknown" rather than $0 for engines with no parser. Keep the price table in a data file with a "verified on" date.
5. **Watch files, do not poll them.** Use the `notify` crate for transcripts and inboxes, with a slow poll as a backstop. The app polls at 1.5 to 30 s, which adds latency that file events would avoid.
6. **Make the status a typed state machine** (starting, idle, working with tool, waiting on human, compacting, looping, stopped, exited), with transitions driven by events. Do not carry status as free text.
7. **Keep docs and comments in sync, and delete dead code.** Several comments and one doc now describe behaviour the code no longer has (section 7, final list).
8. **Do not block the main thread on a login shell.** The app's `spawnSync` shell capture can freeze the UI for about a second per cold resolve. Capture asynchronously at startup, with a timeout.
9. **Check socket path length at startup** and pick a short directory when needed, as the Codex code does.
10. **Harden the breaker's defaults.** It only nags unless `hardStop` is set. Decide on purpose whether a runaway should stop automatically, and say so in the UI.
11. **Use `claude -p --output-format stream-json` for one-shot helper calls only where the billing rationale holds.** The app avoids `-p` for helper sessions, citing a billing change (hiddenClaude.ts:16-19). I could not verify that claim.

### 8.3 Suggested Rust module split

- `pty`: one `portable-pty` per agent, owned by an async actor with the write queue, identity guard, 8 KB tail, and exit events.
- `screen`: a server-side terminal emulator per PTY, used for the draft gate and prompt detection.
- `hooks`: UDS or named-pipe server with the ownership probe and typed events.
- `usage`: OTLP receiver (hyper or axum on 127.0.0.1) plus transcript tailer (`notify`), feeding an append-only ledger.
- `policy`: pure breaker, wake and provider modules, ported with their test cases.
- `procs`: process groups on Unix, Job Objects on Windows.
- `env`: async login-shell capture and env builder.

MIT licence: ideas can be reused freely. Any code copied needs the copyright notice kept.

---

## 9. Open questions and unverified items

- **OpenCode plugin and Pi extension.** The code marks live auto-load and `session.idle` firing as unverified (agentProvider.ts:376-378, 487-490; hive.ts:896-897, 906-907).
- **Proxy parsing for Crush and Qwen.** The comment calls the live parse and synthesised Stop unverified (agentProvider.ts:446-449).
- **Billing rationale for interactive helper sessions** (hiddenClaude.ts:16-19). Self-reported.
- **Hidden-session env gap** (pitfall 16). I did not run it. The test suite has no case for it.
- **macOS `sun_path` for `hooks.sock`.** No length check found. Not reproduced.
- **Stop-to-drain comments** still claim a feature the code removed. Verify before relying on any Codex or agy inbox claim from the comments.
- **Breaker enforcement in practice.** With the defaults, `constrain` only sends mail and a toast. Its effect depends on whether the agent reads the mail.
- **Breaker coverage for non-Claude engines.** Pitfall 24 is inferred from code. A live Codex or Gemini worker run would confirm it.
- **Hook payload shapes** for non-Claude engines are documented only in the shims. I did not check them against the vendors' current releases.
