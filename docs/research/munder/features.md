# Munder Difflin: feature inventory and secondary subsystems

Prepared 2026-10-25 for the Tauri + Rust successor.

Source: shallow clone at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/munder-difflin` (origin `github.com/chaitanyagiri/munder-difflin`, MIT source). Read-only review: no scripts, binaries or npm commands were run, and instructions found in the repo were treated as data. Other readers cover the agent runtime, hive coordination, memory and office visuals; those appear here as one-line inventory rows only.

Maturity scale used below:

- **Polished**: finished behaviour, edge cases handled, tests or heavy iteration.
- **Solid**: works as designed; main path well covered; some rough edges.
- **Usable**: works on the main path; notable gaps, caveats or risks.
- **Experimental**: prototype or not wired into the app.
- **Half-done**: UI or config exists, backend missing or labelled as such in the code.
- **README-only**: described in README or marketing; absent from this source.

---

## 0. Read this first: the snapshot is 0.4.6, the README describes 0.5.6

- `package.json` says version **0.4.6** (Electron ^32.2.0). `CHANGELOG.md` ends at 0.4.6 (2026-08-27) with an `[Unreleased]` block (model picker entries, task ids).
- `README.md`, the Settings hero card and the release-drop docs describe **0.5.x**: a Pro tier, Stapler (meeting capture and dictation with You/Them), a Capabilities marketplace of 2,347 entries, Teams, temps, agent files, per-period token limits, multi-channel Slack, system-wide local dictation, ticket keys, sign-in and accounts.
- A search of `src/` finds none of these: no licence or entitlement code, no meeting capture, no marketplace, no Teams or sign-in, no ticket keys, no agent export, no per-period limits. `docs/privacy.html` says "there is no account, no sign-in, and no sync service".
- The Settings hero card (`src/renderer/src/components/SettingsHeroCard.tsx`, en.json `settingsHero.proLaunch`) ships copy saying "v0.5.0 launches with Munder Difflin Pro", so the free build advertises a paid tier whose code is not in this tree.
- Several README 0.5.x defaults differ from this code: compaction is 2 h at 60% here (README: 40 min at 30%); auto-clear is off here (README: on); the default god model here is `claude-opus-4-8` (README: Opus 5.5).

Treat the README's 0.5.x claims as a product description, not a spec. Everything below describes this snapshot unless marked README-only.

Two internal docs are stale. `docs/research/v0.3.5-feature-research.md` says GitTab is orphaned and CommitGraph is unmounted; both are now wired (GitTab in `AgentDetailPanel.tsx`, the IDE history pane uses `git/CommitGraph.tsx`). Its `config:update` security finding still stands (section 5.5).

---

## 1. What the product is

A desktop harness (Electron 32, React, Pixi.js, xterm.js, node-pty) that runs real terminal coding-agent CLIs as PTY processes, one per "agent", and coordinates them through a local hive: markdown memory, file mailboxes, a task board and an event log. A "GOD" orchestrator (Michael, the user's clone) routes work. The renderer shows an office floor with avatars, a command center, an IDE with git rails and a settings modal. Main-process IPC is roughly 160 handlers; the preload exposes a typed `window.cth` bridge. The UI is localised (en, zh-CN, ar with RTL); en.json, zh-CN.json and ar.json each have 1,160 keys.

Size of the areas most relevant to a port: `src/main/index.ts` 5,511 lines, `src/main/hive.ts` 3,669, `src/main/config.ts` 885, `src/main/realtimeActions.ts` 885, `SettingsModal.tsx` 2,121, `OnboardingWizard.tsx` 917, `IdePanel.tsx` 891. About 120 test files in `test/`, mostly unit tests of pure modules plus a few Electron e2e tests.

---

## 2. Complete feature inventory

Key-file paths are relative to `src/` unless they start with another top-level folder.

### 2.1 Agent runtime, floor and coordination (other readers go deep)

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Agent engines (12 presets + custom) | claude, codex, grok, kimi, gemini, antigravity, qwen, opencode, crush, pi, copilot, cursor, plus any custom command. Each has a launch command, model flags, auto-approve flag and install info. | `shared/agentProvider.ts`, `main/index.ts` (spawn), `main/cliInstall.ts` | Solid for claude; codex, gemini and qwen carry `TODO-verify` models or flags |
| Real PTY terminals | node-pty process per agent, xterm.js with WebGL renderer, pooled terminals, fullscreen view, Arabic/RTL joining mode, link handling | `main/pty.ts`, `main/ptyEnv.ts`, `main/shellEnv.ts`, `renderer/components/terminalPool.ts`, `PtyTerminalView.tsx`, `FullscreenTerminal.tsx`, `renderer/terminal/*` | Polished |
| Office floor | Pixi.js floor; avatars walk to stations; envelopes fly between desks; tool and thought bubbles | `renderer/scene/office/*`, `OfficeFloor.tsx`, `themeRegistry.ts` | Polished (visuals reader) |
| Agent cards, strip, control strip, detail panel | Status, engine, model, context gauge, current task and action, and per-agent detail tabs (including Git) | `AgentCard.tsx`, `AgentStrip.tsx`, `AgentControlStrip.tsx`, `AgentDetailPanel.tsx` | Polished |
| Add/edit agent | Pick engine, model, role, autonomy, working folder; OSS model quick picks; import a hire | `AddAgentModal.tsx`, `EditAgentModal.tsx`, `HivePicker.tsx`, `shared/ossModels.ts` | Solid |
| GOD orchestrator (Michael) | Single agent the user briefs; routes, assigns, escalates; roster-aware before spawning | `main/index.ts`, `main/hive.ts`, `shared/godIdentity.ts`, `shared/agentRole.ts` | Solid (coordination reader) |
| Hive | Per-agent inboxes and outboxes, router, blackboard, append-only log, single-committer git repo | `main/hive.ts`, `HIVE.md`, `SPEC.md` | Solid |
| Message queue composer | Queue messages to an agent, send now, steer, idle-gated delivery so typing never lands mid-tool | `MessageQueueComposer.tsx`, `hooks/queueDelivery.ts`, `hooks/useHive.ts` | Solid |
| Ask Me | Agent questions appear as answerable cards with options; markdown rendering | `AskMeTab.tsx`, `askMeOrder.ts`, `markdown/*` | Solid |
| Command center | Fleet view, kanban, schedules, memory search, activity, restart and continue | `CommandCenterPanel.tsx`, `TasksKanban.tsx`, `TaskDetailOverlay.tsx` | Solid |
| Task board | Todo, doing, blocked, done columns; tasks carry `dependsOn` in the data model | `TasksKanban.tsx`, `shared/taskLedger.ts`, `main/hive.ts` | Solid |
| Threads panel, Workers tab | Conversation threads with replies; workers spawned by Slack or god | `ThreadsPanel.tsx`, `WorkersTab.tsx` | Usable |
| Circuit breaker and budgets | Steer, then constrain, then stop (optional hard kill). Token budgets per floor, token velocity, repeated-tool and error-storm limits | `main/breaker.ts`, Settings > Autonomy & Budgets | Solid. Legacy `costCapUsd` still enforced by the breaker but has no UI |
| Usage and cost ledger | Claude sends OTel to a loopback collector; other engines read transcripts; lifetime cost ledger; per-agent token caps | `main/telemetry.ts`, `main/usage.ts`, `main/costLifetime.ts`, `main/pricing.ts`, `main/transcript.ts` | Solid |
| Memory | Markdown memory per agent, mined into a shared semantic palace (MemPalace CLI), search, graph view, condensation | `main/memory.ts`, `main/reflect.ts`, `MemoryPanel.tsx`, `MemoryGraphPanel.tsx` | Solid (memory reader) |
| Git worktrees per agent | Optional isolated worktree so parallel agents do not share a branch; worktree GC | `main/git.ts`, `main/worktreeDeps.ts` | Usable |
| Heartbeat and hourly ops standup | Standup mission on by default (asks god for status and compacts); heartbeat opt-in (types into god's PTY) | `main/config.ts` (`OPS_STANDUP_MISSION`, `HEARTBEAT_MISSION`) | Solid |

### 2.2 Triggers, schedules and inbound work

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Scheduled missions | Interval or weekday/time-of-day schedules in local time, DST-safe, 6 h catch-up window | `shared/weeklySchedule.ts`, `main/config.ts`, `renderer/components/triggers/SchedulesSection.tsx` | Solid (tested) |
| Context rules | Auto-compact (on by default: every 2 h, at 60% of context, 40% for large windows) and auto-clear (off by default: 90%/80%); custom focus text | `shared/triggers.ts`, `triggers/ContextSection.tsx` | Solid |
| Triggers hub | One tab for schedules, context rules, webhooks, org and history | `renderer/components/triggers/TriggersTab.tsx` and siblings | Solid |
| Webhooks | Many endpoints behind one HTTP server; per-endpoint secret; JSON-schema check; trigger mode (strict by default); status by capability token | `main/webhook.ts`, `shared/triggers.ts`, `triggers/WebhooksSection.tsx` | Usable (see 6.2) |
| Trigger history | Ledger of inbound and outbound messages with decisions (auto-allowed, pending, approved, rejected), 500-entry cap | `main/triggerHistory.ts`, `triggers/TriggerHistoryTab.tsx` | Solid |
| Slack bridge | Slack Events API in (HMAC-verified, mention-gated, thread-activated, deduplicated); replies through a loopback token-gated helper; one summary reply when a Slack task reaches done | `main/slack.ts`, `main/slack-trigger.cjs`, `resources/md-slack-reply.cjs`, done-notifier in `main/index.ts` | Usable (see 6.2) |
| Organisation (peer) trigger | Org key and "message your teammate's clone". UI and persistence only | `triggers/OrgSection.tsx`, `shared/triggers.ts` | Half-done (the UI says so) |
| Ephemeral Slack workers | God spawns an isolated worker for a Slack request, the worker posts its answer into the thread, then is torn down | `main/index.ts`, `main/workerLaunch.ts`, `main/workerWake.ts` | Usable |

### 2.3 Voice ("Talk" to Michael)

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Realtime session | WebRTC speech-to-speech with OpenAI `gpt-realtime-2.1` via `@openai/agents-realtime`; semantic VAD with barge-in; echo cancellation; device picker; voice "cedar"; random greeting | `renderer/src/realtime/session.ts`, `DevicePicker.tsx`, `main/realtime.ts` | Solid (the CHANGELOG says live-verified with a real key) |
| Ephemeral token mint | Main decrypts the stored OpenAI key, mints a short-lived client secret; the real key never reaches the renderer | `main/realtime.ts` | Solid |
| Read tools (13) | Floor state, fleet, list agents, agent detail, tasks, board, memory, triggers, config (allowlisted), cost, activity, messages (redacted), app info | `renderer/src/realtime/tools.ts` | Solid |
| Action tools (20 verbs) | Ping, dispatch, steer, create, assign and update tasks, delete task, spawn, kill, pause, halt, resume, archive, unarchive, clear context, edit or create schedule, auto-delivery, gate tool, update setting; plus `wait_for`, `confirm_action`, `cancel_action` | `main/realtimeActions.ts`, `renderer/src/realtime/actions.ts` | Solid design (see 6.2) |
| Completion and floor pushes | Watcher emits when a dispatched task finishes; floor-delta watcher injects status changes as silent conversation items | `main/realtimeCompletionWatcher.ts`, `main/realtimeFloorWatcher.ts` | Solid |
| Voice message access | Message bodies read through a redaction pass in main (provider keys, Slack and GitHub tokens, JWTs, PEM blocks, Bearer tokens, key=value secrets) | `main/hive.ts` (`redactSecrets`), `renderer/src/realtime/VOICE-MESSAGE-ACCESS.md` | Solid |
| Cost HUD and spend cap | Live token and USD meter using audio-rate pricing (an upper bound); optional session cap; idle auto-disconnect (default 3 min) | `renderer/src/realtime/CostHud.tsx`, `costStore.ts`, `shared/realtimePricing.ts` | Usable |

### 2.4 Dictation and cloud AI

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Free Flow dictation | Hold the Alt/Option key alone (about 320 ms) or click the composer mic; audio goes to Groq Whisper; transcript is appended to the draft, never auto-sent | `main/freeflow.ts`, `renderer/src/freeflow/recorder.ts`, `freeflow/holdOption.ts`, `MessageQueueComposer.tsx` | Usable: cloud-only, window-scoped (see 6.2) |
| Groq chat | Chat completion for "VDE AI assist" with endpoint pinned and secret-looking payloads blocked | `main/groq.ts` | Half-done: no callers |
| VDE prototype | Standalone static HTML prototype | `prototypes/vde/` | Experimental, not wired |
| GitHub issues and CI runs | Lists up to 30 issues and CI runs by shelling out to the `gh` CLI in the agent's folder | `main/github.ts`, `github:*` IPC | Usable (needs `gh`) |

### 2.5 Integrations, MCP and skills

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Integrations registry | Labelled REST endpoints with metadata only; 9 templates: GitHub, Custom REST, Linear, Jira, Confluence, Notion, Stripe, Sentry, HubSpot | `shared/integrations.ts`, `main/integrations.ts`, `renderer/components/IntegrationsRegistry.tsx` | Solid (no dedicated test) |
| Loopback secret broker | 127.0.0.1 proxy. Workers call `/i/<id>/<path>` with a per-worker capability token; broker injects the credential upstream; path confined to the registered origin; 10 MB body cap; 30 s timeout | `main/integrationBroker.ts` | Solid design |
| Secret store | Secrets encrypted with Electron `safeStorage`, fail-closed (no plaintext fallback), kept out of config.json | `main/integrations.ts` | Solid |
| MCP defaults catalog | 10 stdio servers in the safe-readonly, write and secret tiers (no entry uses write). Six read-only ones on by default (sequential-thinking, time, fetch, context7, filesystem and git scoped to the agent folder); four secret or write ones off (GitHub, Postgres, Google mail/calendar, Brave search) | `shared/mcpCatalog.ts`, `renderer/components/McpDefaultsSettings.tsx` | Usable (several `TODO-verify` packages) |
| Skills, local | Discovers installed skills: Claude Code `SKILL.md` folders; OpenCode and Codex reported as plugins | `main/skills.ts`, `renderer/components/SkillsTab.tsx` | Solid (tested) |
| Skills, catalog and install | Browses 227 skills parsed from a third-party README (24 h cache); installs a skill folder from GitHub with size and file caps; uninstall with guards | `main/skills.ts`, `skills:*` IPC | Usable. The file header still says "nothing here installs anything" |
| Bundled skills | Date-range skills (today, yesterday, this and last week, month, quarter, year, last 7/30 days), a `temporal` resolver, `capabilities` catalog, `md-audit`, `md-fetch-summarize`, `md-hive-sync` | `resources/skills/` | Solid |
| Knowledge graph | Ingest documents into a local store; keyword search over chunks; agents query via a CLI; PDFs need `pdftotext` (poppler); images are metadata-only | `main/kg-core.cjs`, `main/knowledge.ts`, `resources/kg.cjs` | Usable (no semantic search) |

### 2.6 Setup, install and models

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Onboarding wizard | Seven steps: persona, welcome, home folder, orchestrator engine, repos, permissions and telemetry, done | `renderer/components/OnboardingWizard.tsx` | Solid |
| Prerequisites page | Shows uv, git, Node, MemPalace and each engine CLI with what each one buys; "Ask Michael to install" seeds a dispatch that the user confirms | `shared/toolCatalog.ts`, `renderer/components/SetupPanel.tsx` | Solid |
| Missing CLI install ladder | Picks npm, then Node-plus-npm, then the vendor's native script, else manual. Runs visibly in a terminal | `shared/agentProvider.ts`, `main/cliInstall.ts` | Solid, but system-changing (see 5.5) |
| Node.js installer | If no usable Node, installs latest LTS Node and npm system-wide; asks for a password in the visible terminal; checksum-verified against nodejs.org | `main/nodeInstall.ts` | Solid |
| BYOK keys and local models | Keys for Anthropic, OpenAI, Google, OpenRouter and Groq in the encrypted store; base URLs for Ollama, LM Studio and vLLM | `renderer/components/AiEnginesSettings.tsx`, `providerKey:*` IPC | Solid |
| Model catalog | Compiled-in catalog plus a remote copy on GitHub (fetched every 6 h); schema v1 only; ids length-capped and newline-free | `main/modelCatalog.ts`, `shared/modelCatalog.json`, `shared/modelCatalogPayload.ts` | Solid |
| OSS model quick picks | Mac-runnable local tags and BYOK third-party OSS providers | `shared/ossModels.ts` | Solid |
| Hire manifests and Agent Gallery | Shareable role templates as JSON, validated by a shared validator; import from file or `munderdifflin://hire` deep link; import pre-fills the form and a human still spawns. SSRF guard on remote fetch. Static gallery of 20 manifests | `shared/hire.ts`, `main/hire.ts`, `docs/hires/` | Solid |

### 2.7 IDE, files and git

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Built-in IDE | Monaco editor with tabs, save (Cmd/Ctrl+S), find and replace, markdown split/preview, image preview, file tree | `renderer/ide/IdePanel.tsx`, `MonacoEditor.tsx`, `MonacoDiff.tsx`, `ImagePreview.tsx`, `monaco.ts` | Polished |
| Git rails | CHANGES, HISTORY and COMPARE tabs: commit graph (hand-rolled lanes), per-commit files and diffs, branch or ref compare with ahead/behind counts, guarded checkout | `renderer/ide/GitPanes.tsx`, `renderer/components/git/CommitGraph.tsx`, `git/graph.ts`, `main/git.ts` | Solid |
| Guarded checkout | Refuses when a live PTY in the tree produced output in the last 10 s, and requires a clean tree | `main/index.ts` (`git:checkout`), `main/git.ts` (`checkoutRef`) | Solid |
| Agent Git tab | Status, branch, log and commit graph for one agent's folder | `renderer/components/GitTab.tsx` (mounted in `AgentDetailPanel.tsx`) | Usable |
| Files tab | File tree and CodeMirror editor for an agent's folder | `FilesTab.tsx`, `FileTree.tsx`, `CodeEditor.tsx` | Usable |
| Markdown preview | Renders agent and file markdown with React (no raw HTML sink); external links open in the OS browser; relative `.md` links open in context | `renderer/markdown/MarkdownPreview.tsx`, `mdLinks.ts`, `rehypeAutoDir.ts` | Solid |

### 2.8 App shell, settings, updates, persistence

| Feature | What it does | Key files | Maturity |
|---|---|---|---|
| Settings modal | Seven sections: General, Prerequisites, Agents & Models, Autonomy & Budgets, Connections, Voice, Memory & Knowledge; one Save button; no search box | `renderer/components/SettingsModal.tsx` (2,121 lines) | Solid, very large |
| Config store | About 111 typed fields in `userData/config.json`; atomic temp-plus-rename writes; migrations; path normalisation; mirrored by hand into renderer and preload types | `main/config.ts`, `renderer/src/store/config.ts`, `preload/index.ts` | Solid, but see 6.2 |
| Persistence | SQLite (better-sqlite3, WAL): key-value (window bounds) and command history (every submitted prompt) | `main/db.ts` | Usable |
| Multi-floor windows | "New Floor" opens another office in its own window on its own folder | `main/index.ts` (`isFloor`), `multiWindow` config | Usable |
| Auto-update | electron-updater against GitHub releases; download in background; restart only on user action; notify-only fallback; update log | `main/updater.ts`, `renderer/components/UpdateBadge.tsx`, `UpdateToast.tsx`, `UpdatesSection.tsx` | Solid (it once silently never installed, per its own header) |
| Release digest and release drops | Digest of release notes for the toast; a release body with `<!-- drop -->` markers renders as an authored page in a sandboxed iframe | `shared/releaseNotes.ts`, `shared/releaseDrop.ts`, `renderer/components/ReleaseDrop.tsx` | Polished |
| Settings hero card | Plan blurb, sponsor slot and notice fetched from `docs/hero.json` on GitHub; carries the Pro announcement text | `main/hero.ts`, `shared/heroPayload.ts`, `renderer/components/SettingsHeroCard.tsx` | Usable |
| Telemetry (analytics) | PostHog, 12 allowlisted events, opt-out (see section 5) | `main/analytics.ts`, `TELEMETRY.md` | Polished contract; one consent gap |
| Loopback OTel collector | Receives Claude Code OpenTelemetry on 127.0.0.1 only; PII-free by allowlist | `main/telemetry.ts` | Solid |
| Localisation | en, zh-CN, ar; RTL layout; fonts bundled | `renderer/src/i18n/*`, `renderer/src/assets/fonts/` | Solid |
| Design system | Tokens in CSS and TS, light and dark, pixel-art chrome | `renderer/src/design/*`, `DESIGN.md` | Solid |
| Office themes | Six theme ids; only `office` is fully built, the five TV-show themes fall back and are labelled experimental | `renderer/scene/office/themeRegistry.ts`, `OfficeThemePicker.tsx` | Half-done |
| Quit and crash protection | Quit warning when PTYs are live; blocked banner; worktree cleanup keeps worktrees that hold unintegrated work | `QuitWarningModal.tsx`, `BlockedBanner.tsx` | Solid |
| Keep awake, login item, notifications | `powerSaveBlocker`, open at login, native notifications on finish or input-needed (notifications default off) | `main/index.ts` | Solid |
| Tool waterfall | Per-agent timeline of tool spans from the OTel collector | `ToolWaterfall.tsx`, `main/telemetry.ts` | Usable |

### 2.9 README-only (not in this source)

| Feature (as the README describes it) | Note |
|---|---|
| Pro tier, licence and checkout | Launch offer "free until 24 Oct 2026", then $150/yr. No code. |
| Stapler (meetings, You/Them capture, dictation into any app, on-device Apple recogniser, bundled Whisper) | Source dictation is Groq-only and window-scoped |
| Capabilities marketplace (2,347 entries, 622 reviewed) | Source has a 10-entry MCP catalog |
| Teams and sign-in, Teams-to-Teams messaging | Source has only the unbuilt org key |
| Temps (short-lived agents) and per-period token limits (5 h, daily, weekly, monthly) | Source has per-agent caps and a floor budget |
| Agent file export with 36 templates | Source has 20 gallery manifests |
| Multi-channel Slack and DMs, Slack manifest copy | Source: one optional channel ID |
| Inbound presets for GitHub, Linear and Telegram | Source: generic endpoints only; no Telegram code |
| Ticket keys such as V53-299, per-agent Tasks tab, Keep my agent order, handoff.md on clear | Not found |

---

## 3. How the secondary subsystems work

### 3.1 Realtime voice (`src/main/realtime*.ts`, `src/renderer/src/realtime/`)

- **Transport.** The renderer opens a WebRTC session to OpenAI with the `@openai/agents-realtime` SDK. Main mints an ephemeral client secret (`POST /v1/realtime/client_secrets`, with a fallback to the legacy sessions endpoint on 404) from the stored OpenAI key and passes only that token to the renderer. The renderer CSP allows `connect-src` to `api.openai.com` and nothing else external.
- **Session.** The persona and tool list are a stable prefix, so prompt caching works. A floor snapshot is injected once as a silent conversation item at connect. Later status changes arrive as silent "(Floor update: ...)" items from `realtimeFloorWatcher.ts`. Dispatched work is tracked by `realtimeCompletionWatcher.ts`, which makes Michael speak the result unprompted.
- **Mic lifecycle.** The mic mutes while a tool call runs. The session disconnects on an idle timer (default 180 s, 0 means never) or when a session spend cap is hit.
- **Safety spine (main).** Every write goes through `realtimeActions.ts`. Verbs are soft (ping, dispatch, steer, task edits, resume, auto-delivery, gate tool, delete task, unarchive) or destructive (spawn, kill, pause, halt, archive, clear context, schedule edits, setting changes). Destructive verbs need a two-step echo-back and a distinct confirm word; bare "yes" is rejected. A single pending action expires after 120 s. Voice may never kill, pause, halt or archive god, and never act on mass targets ("all", "everyone", lists). Settings are split into soft, confirm and forbidden tiers; anything not in the table (including `harnessHome` and secret keys) is forbidden. Every action is attributed to actor `michael-voice`.
- **Gap.** There is no voice verb for restart or model change, although the research note listed it.

### 3.2 Slack bridge (`src/main/slack.ts`, `slack-trigger.cjs`, `resources/md-slack-reply.cjs`)

- Inbound: a bare `node:http` server on port 3847 (default) verifies Slack's HMAC over the raw body with a 5-minute timestamp window, answers `url_verification`, and filters events. It accepts `app_mention`, `message` containing a mention, and `file_share` with a mention. Messages in a thread where the bot was mentioned are accepted too. A bounded FIFO deduplicates retries. Up to 10 files per message are downloaded in main.
- Routing: main sends the text to the renderer over `slack:incomingMessage`. The renderer prepends an "AUTONOMOUS REQUEST PROTOCOL" to god's prompt: triage fast, delegate to an existing agent, act without asking except for high-severity actions, reply in the Slack thread.
- Outbound: agents call `md-slack-reply.cjs`, which reads a discovery file (`port`, `token`, mode 0600) and POSTs to a loopback reply server bound to 127.0.0.1. Main holds the bot token and calls `chat.postMessage`. A poller posts one summary when a Slack-origin task reaches done.
- Exposure: a public tunnel (tunnelmole) forwards Slack's requests to the local port. The tunnel URL changes on restart; the code has a `TODO: optional persistent domain`.

### 3.3 Webhooks (`src/main/webhook.ts`, `shared/triggers.ts`)

- Routes: `POST /<endpointId>` with `x-md-webhook-secret`; `GET /<endpointId>` with `x-md-webhook-token` returns only that task's status. A bare `POST /` is the `legacy` alias.
- Gates: constant-time secret compare; an unknown endpoint gets the same 401 as a wrong secret; 1 MB body cap; rate limits of 120 per minute globally and 60 per minute per endpoint; 192-bit capability tokens shown once (only a hash is stored).
- Trust: `TriggerMode` is `strict` (default: every message waits for approval), `communication-only` (directives wait) or `allow-all`. Kind is declared by the sender or guessed; the guess errs toward "directive".
- Ledger: every inbound and outbound message goes to a plaintext JSON ledger with full bodies (500 entries). Secrets are excluded field by field.

### 3.4 Triggers, schedules and history

- Four trigger types share one contract (`shared/triggers.ts`): schedules (missions), context rules (compact and clear), webhooks, and org (peer, config-only).
- Schedules are either an interval or a weekday/time pattern, in local time. Weekly times are rebuilt from calendar fields so DST shifts do not drift them.
- Context rules fire when both the time gap has elapsed and the agent's context is at least the threshold. Clear is destructive and ships disabled.
- The trigger ledger lives in `userData/trigger-history.json`. Entries are built field by field so a caller's whole object, secret included, cannot be copied in; `updateTriggerHistory` patches an existing entry; the list is capped at 500.

### 3.5 Skills (`src/main/skills.ts`)

- Local discovery reads Claude Code skill folders (`SKILL.md` frontmatter: `name`, `description`) at user, project and bundled scope. OpenCode and Codex are listed as plugin directories.
- Catalog: fetched from a third-party README on GitHub, parsed from markdown, cached for 24 h, with a stale-cache fallback.
- Install: fetches the skill folder through the GitHub contents API with size and file-count caps and a path check, and writes it to `~/.claude/skills` (Claude Code only). Uninstall refuses any path outside the known skills roots: `~/.claude/skills`, the OpenCode and Codex plugin folders, and each registered repo's `.claude` and `.opencode` folders.

### 3.6 MCP catalog and tool catalog (`shared/mcpCatalog.ts`, `shared/toolCatalog.ts`)

- MCP entries are tiered `safe-readonly` (on by default), `write` and `secret` (off until consent). No entry currently uses `write`. Filesystem and git are scoped to the agent folder at merge time. Several package names are marked `TODO-verify`.
- The tool catalog is the Prerequisites list: uv, MemPalace, git, Node and each engine CLI, with the engine rows derived from the provider presets so the two cannot drift.

### 3.7 Integrations registry and broker (`shared/integrations.ts`, `main/integrations.ts`, `main/integrationBroker.ts`)

- An integration is a record (id, label, kind, baseUrl, auth type, `secretRef`, enabled). Auth types are `none`, `bearer`, `header` or `github`. The record never holds the secret.
- A worker never sees the credential. It calls the loopback broker with a capability token that lists the integration ids it may use. The broker strips the worker's own auth headers, injects the stored secret, confines the path under the registered origin and streams the response back.
- The broker has no dedicated unit test in `test/`.

### 3.8 GitHub (`src/main/github.ts`)

- Shells out to `gh issue list` and `gh run list` in an agent's folder. Errors come back as `{ ok: false }`. Nothing is cached.

### 3.9 Free Flow dictation (`src/main/freeflow.ts`, `src/renderer/src/freeflow/`)

- The renderer records webm/opus with `getUserMedia` and `MediaRecorder` and sends bytes to main over IPC. Main posts multipart to Groq's transcription endpoint with the user's key (60 s timeout, 25 MB cap). The result is appended to the target agent's composer draft and is never sent automatically.
- Activation: a solo Alt/Option hold of about 320 ms arms recording; any other key aborts it; `preventDefault` is never called. The gesture works while the window has focus.
- Default model `whisper-large-v3-turbo`. The key is stored in plaintext `config.json` (see 5.3).
- `main/groq.ts` is a separate Groq chat-completion helper written for the VDE prototype. Its endpoint is pinned, prompts are capped at 80,000 characters, and payloads that look like secrets are blocked before egress. It has no callers in this snapshot.

### 3.10 Model catalogs (`main/modelCatalog.ts`, `shared/modelCatalog*.ts`, `shared/ossModels.ts`)

- The compiled catalog is the floor. A remote copy from `docs/model-catalog.json` on GitHub replaces a provider's list when it validates; a provider not mentioned keeps the built-in list.
- The remote copy is data only (escaped text, length-capped ids, control characters stripped), and only schema version 1 is accepted.
- Remote data can change what `--model` value an agent is spawned with, so this is a trust decision as well as a data feed (see 5.5).

### 3.11 Analytics and the OTel collector (`main/analytics.ts`, `main/telemetry.ts`)

- `analytics.ts` is the outbound PostHog client, described in section 5.1.
- `telemetry.ts` is an embedded OTLP/HTTP JSON receiver bound to 127.0.0.1. Spawned Claude agents are started with `CLAUDE_CODE_ENABLE_TELEMETRY=1` and an OTLP endpoint pointing here. It keeps a PII-free allowlist of fields, feeds the circuit breaker and the cost ledger, and never persists raw records.

### 3.12 Config store (`main/config.ts`)

- One JSON file in `userData/config.json`. About 111 typed fields. Writes go through a temp file and a rename, so a crash cannot leave a half-written file. A parse failure falls back silently to factory defaults.
- Defaults that matter: `autoMode: true` (autonomous), `telemetryEnabled: true`, `autoUpdate: true`, `multiWindow: true`, `semanticMemory: true`, `freeflowEnabled: true` (no effect until a Groq key exists), `realtimeVoiceEnabled: false`, `slackEnabled: false`, `webhookEnabled: false`, `knowledgeGraph.enabled: false`, `notifications: false`, `contextTrigger.clear.enabled: false`.
- The config is mirrored by hand in the renderer store and the preload typings, which is how defaults drift (see 6.2).

### 3.13 Updates, release notes, release drops and the hero card

- `updater.ts` uses electron-updater against GitHub releases, downloads in the background and installs only when the user chooses "restart to update". Fallback is a `releases/latest` poll with a notify-only state. The header records a past bug where a missing named export silently forced notify-only mode.
- `releaseNotes.ts` turns a release body into at most 5 short lines (280 characters) for the toast.
- `releaseDrop.ts` renders an authored HTML block from the release body. It is shown in `<iframe sandbox="allow-popups">` with a CSP of `default-src 'none'`. No scripts. Links open through the OS browser, and only http(s) URLs are accepted.
- `hero.ts` fetches `docs/hero.json` (plan blurb, optional sponsor, optional notice) from GitHub. The Pro announcement text is hard-coded in the renderer.

### 3.14 Hire manifests and Agent Gallery (`shared/hire.ts`, `main/hire.ts`, `docs/hires/`)

- A hire is a JSON role template with validation shared between main and the static gallery (`docs/hires/validator.js`). Import comes from a file or a `munderdifflin://hire` link. Main validates the manifest and hands it to the renderer, which pre-fills the Add-Agent modal; a manifest never auto-spawns an agent. Remote fetch resolves DNS and refuses private, loopback, link-local and cloud-metadata addresses.
- The gallery is a static site with 20 manifests and a spec (`docs/hires/spec/HIRE_SPEC.md`).

### 3.15 IDE, git rails and markdown (`renderer/ide/`, `renderer/markdown/`)

- Monaco is self-hosted. The git rails call `git:*` IPC handlers (`logGraph`, `commitFiles`, `showFile`, `compareRefs`, `worktrees`, guarded `checkout`). History and compare produce diff tabs.
- Markdown uses `react-markdown` with `remark-gfm`. No `rehype-raw`, so raw HTML is shown as text.

---

## 4. Onboarding flow and settings surface

### 4.1 Onboarding (7 steps)

1. **persona**: technical or non-technical. Changes copy only.
2. **welcome**: feature showcase, then "set it up".
3. **home**: choose the hive folder (default suggestion `~/HarnessAgents`; `~` is expanded before the folder is created).
4. **orchestrator**: choose Michael's engine and model. Each engine CLI is probed. An engine with no install path on this platform blocks Next; a missing engine that can be installed is installed on first run.
5. **repos**: register git folders (optional).
6. **permissions**: autonomy (default **Autonomous**), keep Mac awake, desktop notifications, open at login, and "Share anonymous usage stats" (default **on**).
7. **done**: `updateConfig({ onboardingComplete, audience, harnessHome, registeredRepos, autoMode, godProvider, godModel, telemetryEnabled })`.

Finishing onboarding also fires `onboarding_completed` (main), and the hive services are bootstrapped in the same handler.

### 4.2 Settings sections (seven)

- **General**: home folder with move or fresh options; keep awake; simple explanations; desktop notifications; scheduled auto-compact (off by default); auto-update; anonymous usage stats; language; Arabic terminal rendering; reset and danger zone.
- **Prerequisites**: tool status and "Ask Michael to install".
- **Agents & Models**: default model for newly spawned Claude agents; max turns per run; AI engines (BYOK keys for five providers, local base URLs).
- **Autonomy & Budgets**: Ask-first versus Autonomous; circuit breaker thresholds (repeated-tool limit, error-storm limit), hard stop (default off); floor token budget; token velocity.
- **Connections**: Slack (signing secret, bot token, channel ID, port, start or stop, proactive posting off), webhooks (per-endpoint secret, mode, schema), organisation key (config only), MCP defaults, and the integrations registry.
- **Voice**: Free Flow (Groq key, model), Realtime (OpenAI key status, mic and speaker pickers, idle disconnect, cost cap).
- **Memory & Knowledge**: semantic memory toggle, cross-session recall, Knowledge Graph documents and file ingest.

Feature and copy strings live in `renderer/src/i18n/locales/en.json`, under `settings.*`.

---

## 5. Telemetry and privacy behaviour

### 5.1 What the app's analytics send (`main/analytics.ts`, `TELEMETRY.md`)

- **Destination**: PostHog Cloud US (`us.i.posthog.com`), overridable at build time. Official builds only: the key is injected in release CI. Dev and fork builds compile with an empty key and the module is a no-op.
- **Gates, in order**: build key present; `DO_NOT_TRACK` not set to anything but `0` or empty (checked at every send); `telemetryEnabled` (default true, live switch in Settings > General).
- **Identity**: a random UUID (`telemetry-install-id`) in userData. Events carry `$process_person_profile: false`, `$ip: null` and GeoIP disabled. Common properties: `app_version`, `os`, `arch`.
- **The 12 events and their only properties**:
  - `first_run`, `app_launched` (each start), `session_ended` (`duration_bucket`).
  - `update_applied` (`from_version`, `to_version`, `via` = auto, manual or unknown).
  - `agent_spawn_attempted`, `agent_spawned`, `agent_spawn_failed` (`reason`), `agent_install_started` and `agent_install_finished` (`rung`, `outcome`) — each with `provider`.
  - `onboarding_completed` (`provider`).
  - `message_sent` (`surface`: terminal, composer, steer or hive). A count, never content, and only for messages a person sent.
  - `feature_used` (`feature`: slack_trigger, webhook_trigger, hire_install, voice_dictation). Once per session per feature.
- The allowlist is enforced in `track()`; unknown events and keys are dropped. The code matches TELEMETRY.md (I checked both lists).

### 5.2 Consent timing gap (a real issue)

On first launch of an official build, `analytics.init()` runs during app start with `enabled = (readConfig().telemetryEnabled !== false)`. With no config file yet, `enabled` is **true**, so `first_run` and `app_launched` are sent before the onboarding wizard appears. The wizard's "Share anonymous usage stats" box (default on) is only written when the user presses Finish, which calls `analytics.setEnabled`.

The wizard comment (`OnboardingWizard.tsx`, near `shareStats`) says "unchecking before finishing means nothing is ever sent". That is wrong for those two events. TELEMETRY.md's "uncheck during onboarding" has the same gap. Users can still stop it with `DO_NOT_TRACK` or Settings, and the events contain no personal data.

### 5.3 Local data at rest

| Data | Where | Protection |
|---|---|---|
| Settings, including Slack bot token, Slack signing secret, webhook secret and Groq key | `userData/config.json` | **Plaintext** JSON |
| Integration secrets and provider keys (OpenAI, Anthropic, Google, OpenRouter, Groq, custom REST) | `userData/integration-secrets.json` | Electron `safeStorage`, fail-closed |
| Provider keys at spawn | Injected into the agent's environment in main | Visible to the agent CLI, by design |
| Trigger ledger with full inbound webhook bodies | `userData` (500 entries) | Plaintext JSON; secrets excluded |
| Command history (every prompt) and window bounds | SQLite in `userData` | Plaintext, WAL |
| Hive: memory markdown, mailboxes, tasks, log, git history | User-chosen home folder | Plaintext files |
| Knowledge store (copies of ingested docs, PDFs, images) | `userData/knowledge` (default) | Plaintext copies |
| Reply helper discovery file (port and token) | `userData` | mode 0600 |
| Telemetry install id, last version, `updater.log` | `userData` | Plaintext |
| Skills, model and hero caches | `userData` | Plaintext |

### 5.4 Network egress

| Destination | When | What leaves the machine |
|---|---|---|
| `us.i.posthog.com` | Official builds, analytics on | The 12 allowlisted events above |
| `api.openai.com` | Talk is on | Microphone audio; tool results (tasks, memory excerpts, redacted message bodies, config subset); the persona |
| `api.groq.com` | Free Flow dictation (and the unused chat helper) | Audio clips up to 25 MB |
| `slack.com` (`chat.postMessage`) | Slack replies | Reply text |
| tunnelmole (third party) | Webhooks or Slack enabled | Inbound HTTP, including Slack signatures, the webhook secret header and bodies. The relay sits between Slack or callers and the app |
| `api.github.com`, `github.com` releases | Update check (on by default); skill install | Plain GET requests |
| `raw.githubusercontent.com` | Model catalog (6 h), hero card, skills catalog (24 h) | Plain GET requests; no opt-out toggle |
| `nodejs.org`, `claude.ai`, `opencode.ai`, `cursor.com` install scripts | User accepts a missing-CLI install | Downloads that run in a visible terminal |
| Integration upstreams (GitHub, Linear, Jira, Notion, Stripe, Sentry and others) | A worker calls an enabled integration | Whatever that worker sends |
| Local Ollama, LM Studio, vLLM | User-configured | Prompts stay local |

Renderer CSP limits the renderer's own connections to `self` and OpenAI.

### 5.5 Security notes that matter for a port

- **Slack is open ingress into an autonomous orchestrator.** Channel filtering is optional (blank means any channel the bot has joined). There is no sender or user allowlist. The Slack path bypasses the `TriggerMode` gate that webhooks use, and the protocol tells god to act without asking. Anyone who can post in a channel the bot has joined and mention it can direct the orchestrator, and text from the thread (including links and pasted files) reaches the agent as instructions.
- **Inbound servers bind every interface.** `WebhookServer` and `SlackWebhookServer` call `server.listen(port)` with no host, so they accept connections on the LAN as well as through the tunnel. The reply server and the OTel collector bind 127.0.0.1 correctly.
- **Plaintext secrets** in `config.json`: Slack bot token, Slack signing secret, webhook secret and the Free Flow Groq key. There are two Groq key stores: encrypted (AI Engines) and plaintext (Free Flow).
- **Permission policy is permissive.** The main window grants every non-media permission request (`setPermissionRequestHandler` returns `true`). Media is gated on the mic feature flags.
- **Raw config write.** `config:update` merges any `Partial<HarnessConfig>` into config.json with no key allowlist, so any key, including `harnessHome` and secrets, is writable by the renderer. The voice path has an allowlist; the UI path does not.
- **Remote data changes behaviour.** The model catalog can change which `--model` value is spawned, and the hero card can display sponsor and notice text. Both are length-capped or escaped, but neither is signed.
- **Installers.** The missing-CLI ladder runs `curl ... | bash` and PowerShell `irm | iex` from vendor domains. The auto-installer starts when a spawn finds the CLI missing (unless that spawn opts out) and runs visibly in the agent's terminal. The Node installer runs with administrator rights after the user types a password, and only after the download matches the checksum published on nodejs.org.
- **Skill installs** add third-party instructions that agents follow with the user's tools. The catalog is unreviewed third-party markdown.
- **Permissive IPC.** About 160 handlers. Several accept untyped arguments and validate them inline.

### 5.6 Website tracking (not the app)

`docs/privacy.html` says the site runs PostHog with autocapture and session recording off and does not load when Do Not Track is on. It also says Google Fonts receives IP addresses. `docs/pro/index.html` loads a Google Ads gtag when Do Not Track is off.

### 5.7 Not collected

No crash reporter: `crashReporter` is not configured and there is no Sentry SDK. Crashes stay in logs on the machine.

---

## 6. What users seem to value, and what looks half-done

### 6.1 Features users seem to value most (from README emphasis, CHANGELOG headlines, community PRs and issue templates)

1. **Many real CLIs in one floor.** The README leads with it, the bug template asks for the agent CLI and its version, and community PRs keep adding engines (Gemini CLI #176, Copilot #101).
2. **The office floor and one card per agent.** The README's hero visual, the first option in the feature-request area list ("Office floor / avatars"), and the README's lead screenshots.
3. **One orchestrator you can talk to, and voice.** Realtime Michael was the 0.3.2 headline; it is "Talk" in the UI.
4. **Dictation.** The README's first feature block is "Cancel your Granola and Wispr Flow subscriptions"; source ships Groq-only Free Flow.
5. **Setup that works on a bare machine.** 0.3.6 ("a machine with nothing installed can now run agents"), 0.4.4 ("the first run stops silently failing"), the Prerequisites page and the missing-CLI card.
6. **Memory across sessions.** README emphasis. Fixes for semantic memory on Apple Silicon (0.4.5) and condensation (0.3.8) show how much this matters.
7. **Autonomy and the breaker.** README "autonomy with a leash". Several fixes concern breaker false positives on idle or compacting agents.
8. **Cost numbers that are true.** 0.4.5 says cost reporting was "off by more than half after a restart"; 0.4.6 reports lifetime spend.
9. **Updates that actually install.** 0.3.7 and 0.4.6 are "auto-update, fixed" and "updates itself".
10. **Languages.** The 0.4.6 headline; community PRs for Chinese (#205) and Arabic RTL (#213) were among the notable contributions.
11. **Files and git inside the app.** 0.3.3 headline; "Files / git" is an issue area.
12. **Slack and webhooks.** README "getting work in and out"; hardened in 0.3.2 (proactive posting off by default, explicit-thread guard).

### 6.2 Features that look half-done, drifted or risky

| Item | Evidence | Status |
|---|---|---|
| Organisation (peer) messaging | "UI + persistence only", "the transport service does not exist yet" in `triggers.ts`; "Configuration only for now" in Settings | Half-done |
| Slack channel scope and sender control | Single optional channel ID; no sender allowlist; no `TriggerMode` gate | Risky (5.5) |
| Public URL persistence for webhooks and Slack | Tunnel URL changes on restart; `TODO: optional persistent domain` | Half-done |
| Webhook presets | Generic only; no GitHub, Linear or Telegram signature handling | Missing |
| Kimi lifecycle hooks | `agentProvider.ts`: "Munder Difflin does not yet install a Kimi hook bridge" | Half-done |
| Engine flags and models | Several `TODO-verify` comments (Codex long-context model, Qwen base URL spike, Gemini interactive flag) | Unverified |
| MCP package names | `TODO-verify` on time, fetch and git (uvx), and on Postgres, Gmail and Brave packages | Unverified |
| Free Flow | Cloud only; hold-Alt only; no Windows or Linux shortcut as README describes; plaintext key | Usable |
| Knowledge graph | Keyword search over chunks; PDFs need `pdftotext`; images are metadata-only | Usable |
| Groq chat helper and VDE prototype | `groqChat` has no callers; `prototypes/vde` is a static page | Dead or experimental |
| TV-show office themes | Five of six themes not built (the picker shows a "soon" tag); the show names and any show art need a clearance decision before shipping | Half-done |
| Voice | No verb for restart or model change; cost estimate uses the audio rate for all tokens, so it is an upper bound; pricing comment still names gpt-realtime-2 | Usable |
| Voice tests | `test/realtime-findcard.test.cjs` tests a copy of the matcher, not the module | Weak test |
| Skills header | `main/skills.ts` says nothing installs, but `installSkill` is wired to IPC | Stale comment |
| Dead dependencies | `commit-graph` and `localtunnel` are in `package.json` but unused in `src/` | Cleanup |
| Docs drift | CHANGELOG stops at 0.4.6; release-drops doc says `sandbox=""`, the code uses `allow-popups`; analytics comment says "Settings > Privacy", the UI is under General; five source files cite `hive/docs/integrations-spec.md`, which is not in the repo (only `integration-templates.md` is) | Cleanup |
| Legacy cost cap | `costCapUsd` is still enforced by the breaker but absent from the UI | Cleanup |
| Config drift | Config mirrored by hand across three files; the research note found default drift (since fixed in several places) | Structural |
| Settings size | `SettingsModal.tsx` is 2,121 lines | Structural |
| Broker coverage | `integrationBroker.ts` has no dedicated test | Gap |

---

## 7. Ideas for the successor, ranked by user value

Ranked by how much each one matters to users. The verdict is Copy, Improve (copy with changes) or Skip.

| Rank | Idea | Verdict | Why it matters and what to change |
|---|---|---|---|
| 1 | Agent floor: one card per agent with status, context gauge, breaker state, current action and a live terminal | Copy | The core of the product, and the README's lead. In Tauri, keep state in Rust and stream it over channels. |
| 2 | Multi-engine PTY runtime with provider presets and a "missing CLI is a card" flow | Copy, Improve | Drives the "works with the subscriptions you already pay for" promise. Use a Rust PTY crate. Prefer npm or native installs that the user confirms; drop blind `curl \| bash` by default. |
| 3 | Onboarding that detects prerequisites and offers installs; "ask the orchestrator to install" with a confirm step | Copy | Users value "set up once". Keep the visible-terminal and checksum-verified installer behaviour. |
| 4 | Voice orchestrator: ephemeral token minted in the core, WebRTC from the webview, echo-back confirm, distinct confirm words, 120 s pending TTL, hard denylist for god and mass targets | Copy, Improve | Keep the safety model. Move the policy table into Rust, add restart and model change, log every voice action. |
| 5 | Dictation: hold a key, transcribe, append to the draft, never auto-send | Copy, Improve | Dictation was a headline feature. Make local transcription (whisper.cpp) the default and cloud opt-in. The README's "any app" claim needs OS accessibility permissions. |
| 6 | Triggers: schedules, context rules, webhooks with a strict default mode, and a ledger | Copy, Improve | Apply the same gate to Slack. Keep strict as the default for every external source. |
| 7 | Slack and webhook ingress | Improve | Bind 127.0.0.1. Prefer Slack Socket Mode (outbound connection, no public URL). Add user and channel allowlists. Avoid default public tunnels. |
| 8 | Integrations broker with capability tokens, path confinement and enable consent | Copy, Improve | Agents never see credentials. Store all secrets in the OS keychain (not `config.json`), and add a test suite for the broker. |
| 9 | Telemetry contract: allowlist, anonymous install UUID, DNT, build-key gating, public TELEMETRY.md | Copy, Improve | Fix the first-run ordering (defer `first_run` and `app_launched` until the consent choice). Show a local "what was sent" log. |
| 10 | IDE and git rails: Monaco, CHANGES, HISTORY and COMPARE, commit graph, guarded checkout | Copy | Users with many agents need per-agent diffs. Extend the checkout guard to every worktree. |
| 11 | Usage and cost ledger from native telemetry and transcripts | Copy | Cost accuracy was a repeated fix (0.4.5). Add per-engine fixture tests. |
| 12 | Auto-update that installs on user action, with release-note digest | Copy | Use the Tauri updater with signatures. Keep the digest. |
| 13 | Localisation (en, zh-CN, ar), RTL and bundled fonts, with key-parity tests | Copy | Community PRs (#205, #213) drove the first pass. |
| 14 | Hire manifests and the Agent Gallery, with an SSRF-guarded import | Copy | Keep "import pre-fills, a human spawns". |
| 15 | Skills discovery and bundled date-range skills | Copy | Pin and hash third-party skill installs. |
| 16 | Remote model catalog with a compiled floor and schema versioning | Copy, Improve | Sign the remote payload before it can change spawn flags. |
| 17 | Remote sponsor and plan copy, star prompts | Skip | Unrequested content from the network. Make any of it opt-in. |
| 18 | Org or peer messaging, Telegram and GitHub or Linear presets, Pro-tier announcements | Skip for now | Not built; design first. |
| 19 | Dead code: `groqChat`, the VDE prototype, `commit-graph` and `localtunnel` | Skip | Do not port. |
| 20 | TV-show office themes | Skip unless cleared | Trademark and art clearance first. |
| 21 | Config as one typed schema (Rust structs generating TypeScript) with a validated write path | Improve | Replace the raw `config:update` merge and the three-file mirror. |
| 22 | Deny-by-default permissions in the webview | Improve | Use Tauri's capability allowlist. Do not grant every permission request. |

Non-negotiables for the port, even though they are not ranked: keep the renderer off the network except the voice provider's API; keep secrets out of plaintext files; bind local servers to loopback only; require explicit consent before any installer runs; and fix the first-run telemetry ordering before shipping.

---

## 8. Open questions and limits of this review

- The 0.5.x source (Pro, Stapler, Capabilities, Teams, temps, sign-in) is not in this snapshot. Confirm whether it exists in a private repo before deciding which of the README features to build.
- Nothing was run. Behaviour claims come from reading code, comments and the CHANGELOG. In particular, the Talk loop, Slack threading and tunnel behaviour were not exercised.
- The clone is shallow, so git history was not available for dating individual features.
