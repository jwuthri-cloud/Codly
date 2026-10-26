# Codly research critique: contradictions, re-verification, gaps, and the consolidated architecture

*Completeness and consistency review of the seven research reports in `docs/research/`, plus a skim of `fixtures/`. Date: 2026-10-25. Machine: macOS 26.5 arm64, `claude` 2.1.309, `codex-cli` 0.161.0, `gemini` 0.62.0, `opencode` 1.18.31, git 2.54.0, node 22.23.3, bun 1.3.14.*

Reports reviewed:

| Report | Lines | Scope |
|---|---|---|
| `cli-claude-codex.md` | 1154 | Claude Code stream-json and control protocol; Codex app-server and exec |
| `cli-others-acp.md` | 631 | ACP; Gemini, OpenCode, Cursor, Copilot and Pi |
| `landscape.md` | 830 | Munder Difflin, Pixel Agents, Vibe Kanban and about 20 other orchestrators |
| `context-engineering.md` | 724 | Jev leads, retrieval, FTS5 measurements, MCP delivery |
| `tauri-stack.md` | 885 | Tauri 2, IPC, crates, frontend, testing |
| `agents-backend-orchestration.md` | 1424 | Deep Agents sidecar, orchestration, decisions, calibration |
| `design-world.md` | 1230 | Design system, world engine, Inspector |

**Evidence labels**

- **[V-critic]**: I re-verified it today. This covers local `--help` output, free control-protocol-only sessions, source in the cloned repos, crates.io or npm metadata, and official pages fetched today. The check is named.
- **[Report]**: carried over from a sibling report. I did not re-check it.
- **[I]**: my inference or recommendation.

**Method and safety notes**

- **No paid prompt was run.**
  - The only `claude` runs were `--help` and control-only sessions: `initialize` and `get_context_usage`, with no user message, `--no-session-persistence`, and a scratch cwd.
  - The captured stdout held only `system/hook_*` lines and control responses, with no `assistant` or `result` line, so no inference ran.
  - **The user's own `SessionStart` hooks still fired** in those sessions, even under `env -i` (see G6).
- crates.io requests used the User-Agent `codly-research-critic/0.1 (generic)`. No personal data was sent.
- Downloaded crate tarballs (tokio-util 0.7.20, libsqlite3-sys 0.38.2) were only read.
- Scratch outputs are in `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/critic/`. They are temporary. The one reusable script is copied into Appendix A.

---

## 0. Bottom line

1. **The reports agree on the shape:**
   - a Rust core that owns the truth;
   - one append-only event log;
   - native protocols for Claude Code (stream-json plus the stdin control protocol) and Codex (`codex app-server`);
   - ACP for the long tail;
   - one git worktree per task, with verification that Codly runs itself;
   - both views as pure projections of the log.
2. **They disagree on about 30 concrete choices** (§2). The ones that cause rework if left open:
   - the Claude transport (stream-json or an interactive PTY);
   - rusqlite 0.41 or 0.40.2;
   - an in-process core or a launchd daemon;
   - who commits inside worktrees;
   - the ACP SDK or the ACP schema crate;
   - which git library;
   - **five incompatible event schemas with three meanings of `seq`**;
   - macOS 13 or 14;
   - a mailbox or a blackboard.
3. **Two recommended code paths are wrong as written** [V-critic]:
   - **The stdout pump in `cli-claude-codex` §5.2 stops after one bad line.** It uses `FramedRead` plus `LinesCodec`. On the first oversized or non-UTF-8 line, tokio-util 0.7.20 returns `None` once, so the `while let Some` loop ends. Nobody reads stdout after that, the pipe fills, and the CLI blocks.
   - **The worker prompt in `agents-backend` §6.6 cannot work under Codex.** It tells the agent to "commit your work". Codex `workspace-write` makes the worktree's resolved gitdir read-only, so the agent cannot commit.
4. **Three verified facts that change the design:**
   - **(a) The Claude 2.1.309 tool set is not what three reports assume.**
     - The default tool set has no `Grep`, no `Glob`, and **`TodoWrite` no longer exists**.
     - Plans now come from `TaskCreate`, `TaskUpdate`, `TaskList` and `TaskGet`. Search happens through `Bash` unless Codly names `Grep,Glob` explicitly.
     - **Unknown `--tools` names are silently ignored.**
   - **(b) `--system-prompt-snapshot` is on by default.** An `--append-system-prompt` brief is recorded once per conversation and reused as-is on later `--resume`, even if the new launch passes different text.
   - **(c) Claude's headless stream has no live Bash output.**
     - `tool_progress` carries only `elapsed_time_seconds`. The ACP adapter builds its "terminal output" from the finished tool result.
     - So "click an agent → live terminal" is truly live only for Codex (`item/commandExecution/outputDelta`) or for a PTY.
5. **The Anthropic legal page adds an obligation that no report listed.**
   - Its rule: running Claude Code in your products *"requires agreeing to our Commercial Terms of Service"*. It also requires:
     - an unmodified binary, with no authentication method disabled;
     - no paying for, reselling or intermediating usage;
     - trademark limits.
   - The carve-out for an end user signing in with their own subscription is real. The 2026-10-07 support update still says `claude -p` and third-party apps draw from subscription limits [V-critic].
6. **The biggest unverified dependency is a live Claude `can_use_tool` approval round trip on 2.1.309.** The whole Claude approval UX rests on it. Spend the first paid run there (§6).

---

## 1. Spot-check ledger (what I re-verified, and the verdict)

| # | Claim (source report) | Check I ran | Result | Verdict |
|---|---|---|---|---|
| 1 | The Claude headless flags in the TL;DR exist (cli-claude-codex) | `claude --help` 2.1.309 | Present: `--input-format stream-json`, `--output-format stream-json`, `--include-partial-messages`, `--json-schema <schema>` (inline JSON in the help example), `--permission-prompts host\|none`, `--session-id`, `--setting-sources`, `--strict-mcp-config`, `--forward-subagent-text`. `--bare`: "OAuth and keychain are never read". `--permission-prompt-tool` appears only inside the `--permission-prompts` help text | ✔ |
| 2 | `--permission-mode default` (cli-claude-codex §5.1) | Ran it with stdin at EOF (no inference) | Help lists `acceptEdits, auto, bypassPermissions, manual, dontAsk, plan`. `default` and `manual` are both accepted; `bogusmode` fails with that list | ✔ `default` is a hidden alias. Prefer the documented `manual` |
| 3 | Claude's search and plan tools are `Grep`, `Glob` and `TodoWrite` (context-engineering §2.1; landscape §1.6 and R6; design-world §9.3) | `system/init.tools` in both Claude fixtures; free `get_context_usage` probes with different `--tools` (Appendix A) | **Default init: 28 tools, without Grep, Glob or TodoWrite.** `system_tools` tokens by `--tools` value: `Read` 1008; `Read,Grep` 2284; `Read,Grep,Glob` 2561, so both exist when named. `Read,TodoWrite` 1008, the same as `Read,BogusTool`, so TodoWrite is gone and unknown names are ignored silently. `TaskCreate` 2379, `TaskUpdate` 2714, `TaskList` 1959, `TaskGet` 1695. `Agent` and `Task` are both 2932 (aliases) | ✘ stale in 3 reports |
| 4 | `codex exec` has no approval flag (cli-claude-codex) | `codex exec --help` 0.161.0 | No `-a/--ask-for-approval` on exec, only `--approve-for-me` and the bypass flag. The top-level command has `-a` | ✔ |
| 5 | The app-server is the integration surface (cli-claude-codex, landscape) | `codex --help` | **`app-server` and `remote-control` are labelled `[experimental]`** | ⚠ no report flagged this |
| 6 | Codex workers need `exec --output-schema` for a structured report (agents-backend) | Generated `v2/TurnStartParams.json` | `outputSchema` exists on `turn/start` | ✘ the app-server can do it per turn |
| 7 | Worktree commits are blocked by sandboxes (cli-claude-codex: "Unverified inference") | codex-rs `protocol/src/permissions.rs` @970111f | `default_read_only_subpaths_for_writable_root` makes `<root>/.git` read-only. For a `.git` pointer file it also protects the resolved gitdir: "This applies to … worktrees/submodules (file .git with gitdir pointer)" | ✔ now [V-src]. Contradicts agents-backend's worker prompt |
| 8 | rusqlite: 0.41 (context-engineering, agents-backend, landscape) or `=0.40.2` (tauri-stack) | crates.io API | rusqlite 0.41.0 was published 2026-10-24. libsqlite3-sys 0.39.0 shipped 10-24 and 0.39.1 today. `rusqlite_migration` 2.6.0 needs `rusqlite ^0.40.0`, `tokio-rusqlite` 0.8.0 needs `^0.40.1`, `r2d2_sqlite` 0.35.0 needs `^0.40`. libsqlite3-sys is a `links = "sqlite3"` crate, so two versions cannot coexist | ✔ tauri-stack is right |
| 9 | FTS5 needs 0.41's SQLite 3.54.1 (context-engineering) | Read libsqlite3-sys 0.38.2 `build.rs` and `sqlite3.c` | `-DSQLITE_ENABLE_FTS5` (plus FTS3, JSON1, RTREE). SQLite **3.53.2**, which is at least the 3.43 that `contentless_delete=1` needs | ✔ the FTS5 design works on 0.40.2 |
| 10 | The ACP Rust SDK churns and has a deadlock trap (cli-others-acp) | crates.io and rust-sdk @5ef0fa3 | 85 versions. Majors: 1.0.0 (06-24), 2.0.0 (07-23), 3.0.0 (10-06). Six releases between 10-06 and 10-22. 3.5.0 pins `agent-client-protocol-schema =1.12.0`. The `process` feature uses async-io and async-process, not tokio. `Client.builder()` and `connect_with()` exist. Docs on `block_task()`: "Never await it in a handler … This will deadlock!" | ✔ |
| 11 | Stable ACP surface: no `set_model`; resume, list, close and delete exist (cli-others-acp) | `schema/v1/meta.json` and CHANGELOG @66ff807 | Agent methods include `session_resume`, `session_list`, `session_close`, `session_delete`, `session_set_config_option`, `session_set_mode`, and no `set_model`. 1.26.0 is dated 2026-10-16. `-32002` and `usage_update` are present | ✔ |
| 12 | crates.io `codex-app-server-protocol` is a stale fork (cli-claude-codex) | crates.io | 0.63.0 (2025-12-11), repository `github.com/namastexlabs/codex` | ✔ |
| 13 | codex-acp needs `@openai/codex ^0.160.1` (cli-claude-codex) or `^0.164` (cli-others-acp) | `npm view @agentclientprotocol/codex-acp@2.2.2 dependencies`; repo `package.json` | The published 2.2.2 has `^0.160.1`, which for a 0.x version means **0.160.x only**. Repo main has `^0.164.0` | Both are right about different artifacts. The compat path runs an older Codex than the user's 0.161 |
| 14 | `FramedRead` with `LinesCodec::new_with_max_length` "resyncs at the next \n" (cli-claude-codex §5.2) | Read tokio-util 0.7.20 `codec/framed_impl.rs` | Errored state: "Returning None and setting paused". A `while let Some` loop exits there | ✘ use tauri-stack's bounded byte reader |
| 15 | A conflict pre-check without libgit2 is possible | Scratch repo, git 2.54: `git merge-tree --write-tree --name-only --messages main t2` | Exit 1. Printed the conflicted path and the `CONFLICT (content)` message. Worktree status stayed clean | ✔ resolves C8 |
| 16 | `backgroundThrottling` needs macOS 14 (tauri-stack) | wry 0.57.0 `src/wkwebview/mod.rs` | `os_major_version >= 14` gate; skipped silently below that | ✔ design-world's 13.0 floor loses it |
| 17 | `_meta.terminal_output_delta` gives live command output from the Claude adapter (cli-others-acp R3) | claude-agent-acp @739dde3 `src/tool-calls/renderer.ts` and `src/acp-agent.ts` | Terminal output is built in `resultFields(...)` from the finished tool result. `tool_progress` beats carry only `elapsed_time_seconds` | ✘ for Claude (output only at completion). ✔ for Codex (`outputDelta`) |
| 18 | Anthropic subscription policy (cli-claude-codex §2.12, landscape TL;DR 5) | Fetched code.claude.com `legal-and-compliance` and support article 15036540 | Carve-out confirmed verbatim. Products that run Claude Code must accept the **Commercial Terms**. Name and logo limits. June 15 pause and Oct 7 credits confirmed. The page says nothing about interactive versus programmatic use | ✔ plus a new obligation |
| 19 | Claude HTTP hooks (landscape §8) | Fetched `docs/en/hooks` | `type:"http"` with `url`, `headers`, `allowedEnvVars`, `timeout` (600 s default), `onFailure` (≥ 2.1.295). 33 events. 10,000-character cap on `additionalContext`, with no setting to raise it | ✔ |
| 20 | "Node readline splits on U+2028" (cli-others-acp §8, from Pi's docs) | `readline.createInterface` on a JSON line with raw U+2028, on Node 22.23.3 and Bun 1.3.14 | Read as one line and parsed on both | ✘ not reproduced; low impact |
| 21 | Fixtures are redacted | Regex scan of `fixtures/` | No emails or account IDs. `/Users/julien` paths appear in 4 files | ✔ redact paths before publishing |

---

## 2. Contradictions between reports, and the call

Each row gives the call and the evidence behind it.

| # | Topic | Position A | Position B | **Call** |
|---|---|---|---|---|
| C1 | Default Claude transport | landscape R2: T1, an interactive PTY plus HTTP hooks plus a transcript tail, as the "most policy-robust path" [I] | cli-claude-codex §5.1, agents-backend §6.6, cli-others-acp R1: `claude -p` with stream-json and the control protocol | **stream-json for every Claude agent Codly orchestrates.** The PTY (`claude --resume <id>` in a Codly terminal) is for user take-over (G8) and is a later fallback tier. See C1 notes below |
| C2 | Codex worker path | cli-claude-codex, landscape: `codex app-server` | agents-backend §6.6 and §6.9: `codex exec --json … --output-schema` for workers and `codex exec resume` for recovery | **app-server for every worker.** It gives approvals, deltas, `turn/diff/updated`, and `outputSchema` on `turn/start` [V-critic]. Use `exec --json --ephemeral --output-schema … < /dev/null` only for one-shot L1 decisions. **Recovery is a restart:** new app-server, then `initialize`, then `thread/resume {threadId}` |
| C3 | Codex protocol types | cli-claude-codex: hand-written serde for the subset Codly uses | landscape T2: `codex-app-server-protocol` "from the matching git tag", as Vibe Kanban does | **Hand-written serde, plus a CI diff** of `codex app-server generate-json-schema --out` on the minimum and the latest Codex. A git dependency pins Codly to one Codex version's types and drags in the codex workspace. The crates.io crate is a stale fork [V-critic] |
| C4 | ACP in Rust | landscape T3: `agent-client-protocol` 3.5.0 SDK | cli-others-acp R2: schema crate plus our own transport | **`agent-client-protocol-schema = "1.12"` types plus Codly's own tokio NDJSON JSON-RPC transport.** It is shared with the Codex client and the sidecar. The SDK's churn, exact schema pin, async-io reactor and deadlock trap are verified [V-critic], and Codly needs the raw-line tap anyway |
| C5 | SQLite crate | context-engineering §4.2, agents-backend §6.2, landscape R5: rusqlite 0.41 (or sqlx 0.9) | tauri-stack §5.4: `=0.40.2` with no sqlx | **`rusqlite = { version = "=0.40.2", features = ["bundled"] }` plus `rusqlite_migration = "2.6.0"`.** No sqlx, no `tauri-plugin-sql` [V-critic #8, #9] |
| C6 | Process host | landscape R1: `codlyd` under launchd, so agents survive app restarts | tauri-stack §8.3, context-engineering §4.1, agents-backend §6.9: an in-process `codly-core`, with kill-and-resume | **In-process for v1**, behind a boundary that is ready for a daemon. See C6 notes below |
| C7 | Who commits in a worktree | agents-backend §6.6: the worker runs `git add -A && git commit` | cli-claude-codex §5.8: Codly commits | **Codly commits**, outside any agent sandbox, at each turn end and at the gate. It records `before_head`/`after_head` per turn (the Vibe Kanban pattern from landscape §3.4). Verified that the Codex sandbox protects the gitdir [V-critic #7] |
| C8 | Git library | tauri-stack §5.6, agents-backend §6.6: the `git` CLI for mutations | landscape R4: git2 reads plus a libgit2 in-memory merge pre-check. context-engineering §4.2: gix reads | **The git CLI for all mutations.** `git merge-tree --write-tree --name-only --messages` for the pre-check [V-critic #15]. `gix` only behind a feature, for read-hot paths. **Drop git2**: it needs a C build, runs no hooks, and behaves differently from the user's `git` |
| C9 | Reading stdout | cli-claude-codex §5.2: `FramedRead` plus `LinesCodec` | tauri-stack §5.2: a hand-written bounded byte reader | **tauri-stack's `read_record`**: `\n`-only, 64 MiB cap, skip and continue, bytes in, then `serde_json::from_slice` [V-critic #14] |
| C10 | Where raw lines live | cli-claude-codex §5.3 (`raw: Box<RawValue>` in each event), cli-others-acp R2: in SQLite | agents-backend §3.1 (`runs/<id>/raw.ndjson`), landscape R5 (zstd `raw_streams`): in files | **One append-only file per run**, written before parsing with batched `fsync`. Each event row stores `(file, offset, len)`; the file is zstd-compressed on close. See C10 notes below |
| C11 | Event schema and `seq` | cli-claude-codex: `AgentEvent{run_id, seq (per run), …}`. agents-backend: `event PK (run_id, seq)`; the sidecar's `seq` is per process | design-world: `WorldActivityEvent.seq` "monotonic per project (SQLite rowid)". tauri-stack: snapshot `asOf` is the core's global `seq` | **The layered schema in §5.4.** A global `event_id` (SQLite `INTEGER PRIMARY KEY`) is the only UI cursor. `run_seq` is used only for idempotent ingestion. Add `causation_id` from landscape. `WorldActivity` and `UiBatch` are projections |
| C12 | Inter-agent messaging | landscape R3: a FIPA-lite mailbox; delivery via hook `additionalContext`, stdin, Claude's cross-session socket, then PTY nudges | agents-backend §4.5: "Blackboard wins for v1" | **Blackboard for v1.** Notes are curated by the orchestrator and injected at task start or the next turn. Agents talk to the orchestrator only through `codly-mcp`. Cognition's evidence, Munder's removal of forced continuations, the injection surface (G4), and the undocumented cross-session wire format all point this way |
| C13 | Agent-facing MCP tools | landscape R3: `send_message, inbox, claim_task, update_task, report_progress, request_review`. agents-backend: `report_blocker, post_note, get_task`. context-engineering: `search, symbols, explain_choice, feedback` | — | **One `codly-mcp` server with 6 tools in v1:** `codly_get_task`, `codly_report_blocker`, `codly_post_note`, `codly_submit_report`, `codly_search`, `codly_feedback`. `codly_symbols` is added in v1.5. See C13 notes below |
| C14 | How agents call back into Codly | context-engineering §5.2: a stdio sidecar that proxies to the core over a Unix socket | cli-claude-codex §5.5: rmcp streamable HTTP on 127.0.0.1. landscape R2: HTTP hooks into `codlyd` | **A stdio `codly-mcp` plus a mode-0600 Unix socket in Application Support, with a per-run token.** For Claude PreToolUse/Stop interception in stream-json mode, register **SDK hook callbacks in `initialize`** (`hook_callback`, the Vibe Kanban pattern), so no HTTP listener is needed. A loopback HTTP listener exists only if the PTY tier ships, and then it uses a bearer token and a Host-header check |
| C15 | `--strict-mcp-config` | cli-claude-codex §5.1: always | context-engineering §5.3: "never outside benchmarks" | **On by default**, with a per-project "Inherit my MCP servers" option for trusted repos only. It gives determinism, avoids the up-to-30 s `MCP_TIMEOUT` start wait and tool-schema tokens, and keeps untrusted `.mcp.json` out |
| C16 | Claude worker permission mode | cli-claude-codex: `default` | agents-backend: `acceptEdits` | **`acceptEdits` for workers inside a Codly worktree.** Edits are isolated and gated, and Bash still goes `can_use_tool` → L0 policy → user. **Planner and reviewer are read-only**, via `--permission-mode plan` or a read-only `--tools` list. Write `manual`, not the hidden `default`, wherever prompt-everything is wanted |
| C17 | Concurrency caps | cli-claude-codex: up to 3 per provider | agents-backend: 3 in total, 2 per vendor | **3 in total, 2 per vendor by default**, user-adjustable. A vendor is parked above 85% of its 5-hour window |
| C18 | Pixel renderer | landscape R6: Pixi.js 8 "proven by Munder Difflin" | tauri-stack, design-world: Canvas 2D | **Canvas 2D.** It costs 0 KB against PixiJS's 237 KB gzipped, needs no WebGL context-loss handling (Munder needed `glRecovery.ts`), and supports the idle scheduler that schedules nothing |
| C19 | World art | landscape R6: LimeZu or Metro City packs, "check licences" | design-world §2: no bundled third-party art | **Ship zero bundled art.** Characters are procedural; bring-your-own packs come in v1.1. (The two reports also disagree on whether Metro City is CC0; design-world cites the itch page.) |
| C20 | Minimum macOS | design-world §5.4 and R8: `13.0` | tauri-stack §7.1 and §8.1: `14.0` | **`14.0`, with Vite `build.target: 'safari17'`.** wry ignores the background-throttling policy below 14 [V-critic #16] |
| C21 | Default view | design-world R12: Split by default on windows ≥ 1440 px | landscape pitfall 13: the Professional Workspace by default, with the world opt-in (HN called it "cringe" and wanted "is it a game or a tool?" answered) | **Workspace on first run.** Split and World are one keystroke away, remembered per user, and offered during onboarding |
| C22 | When retrieval lands | context-engineering §7: tree-sitter tags in the first vertical slice | tauri-stack §5.8: defer tree-sitter to v2 | **v1 ships:** FTS5 over paths and 60-line chunks, AND-first queries, RRF, agreement confidence, a pointer pack, and `codly_search`. **Tree-sitter tags come in v1.5**, gated by the commit-replay benchmark. In the pilot, chunks alone reach Hit@5 0.62 against 0.65 for RRF of symbols and chunks (subject-only queries) [Report] |
| C23 | Claude tool vocabulary | landscape and design-world: Read/Grep/Glob → shelf, TodoWrite → board. context-engineering: "agentic Glob/Grep/Read" | 2.1.309 in fact [V-critic #3] | **Classify in this order:** (1) a per-version tool table built from `system/init.tools`; (2) ACP `kind`; (3) input shape; (4) Bash argv rules (design-world §9.2, rule 4). Also **request `Grep,Glob,TaskCreate,TaskUpdate,TaskList,TaskGet` explicitly**, so search and plan show as Observed, not Inferred |
| C24 | The Codex column of the world mapping | design-world §9.3 maps `exec --json` events and puts app-server approvals "out of scope" | The decision is app-server | **Rebuild the Codex column from app-server events.** See C24 notes below |
| C25 | Speech-bubble text | cli-claude-codex §3.3: `agentMessage.phase:"commentary"` is "ideal for the pixel-office speech bubbles" | design-world §8.6: bubbles restate events only; "no invented feelings" | **design-world's rule.** Commentary goes to the Inspector as "Model-stated", or to an optional thought bubble that carries that label |
| C26 | Spawning the sidecar | agents-backend §3.3: the `tauri-plugin-shell` sidecar API | tauri-stack §3: no shell plugin | **No shell plugin.** Resolve the bundled `externalBin` next to the main executable and spawn it through the same tokio supervisor, which is what agents-backend itself recommends. [I: confirm the bundled file name loses its target-triple suffix in a real `tauri build`] |
| C27 | Capturing PATH | cli-claude-codex §5.9: `"$SHELL" -lic 'command -v …'` | tauri-stack §5.10: `-ilc` with `/usr/bin/env -0`, markers and a 5 s timeout | **tauri-stack's module**: safe for multi-line values, has a timeout, and never calls `set_var`. Keep cli-claude-codex's rule to re-resolve the `~/.local/bin/claude` symlink at every spawn, because the updater re-points it |
| C28 | Codex config isolation | landscape §11.2: per-agent `CODEX_HOME` with a linked `auth.json` (the Munder pattern) | cli-claude-codex §5.9: never relocate `CODEX_HOME` in subscription mode | **Don't relocate it.** Use `thread/start.config` and `-c` overrides. Symlinking the user's token file is credential handling Codly should avoid |
| C29 | Where Jev sits | agents-backend §5.2: first choice for L1 decisions in API-key mode | context-engineering §1.7: off by default, per-repo consent, at most 20 candidates | **Off by default everywhere.** In API-key mode, L1 uses a cheap OpenRouter model under the user's own key. Jev is an opt-in `DecisionProvider` behind explicit consent, because it uploads task state and code |
| C30 | Agent display names | design-world mocks use human names ("Ada", "Bo") | landscape R6: neutral role names | **Default label is role plus vendor** ("Worker 2 · Codex"), with generated names as an option |

**C1 notes: why stream-json, not a PTY, for Claude.**

- **The policy text doesn't favour the PTY.** The legal page re-read today treats an unmodified binary run with the user's own subscription the same way in either mode. The support article names `claude -p` as drawing from subscription limits.
- **The PTY tier loses a lot:**
  - streaming deltas;
  - structured approvals, unless Codly adds PermissionRequest hooks and a loopback server;
  - `rate_limit_event`;
  - the free control requests;
  - and it is fragile to TUI changes.
- **landscape's hedge is weaker than it looks.** The paused June change also named "third-party apps", so a PTY inside Codly is not clearly exempt.
- **The real hedges** are API-key BYOK, multi-vendor routing and quota awareness.

**C6 notes: why in-process for v1.**

- **What stays daemon-ready:** `codly-core` has no Tauri dependency, and its commands and event stream are shaped like an IPC protocol. tauri-stack's `codly-devserver` WebSocket bridge is the daemon prototype.
- **What a launchd daemon costs:** a second signed binary, IPC authentication, version skew between app and daemon, and upgrade choreography.
- **Why it isn't needed yet:** resume by session ID already recovers work for every CLI.

**C10 notes: why raw lines go in files.**

- Raw lines are large: Claude's `init` is about 25 KB, `commands_changed` lines were 187 KB, and tool results run to megabytes. In SQLite they would bloat the WAL and queue behind the single writer.
- Separate files keep raw-data retention and redaction (G3) separate from the event table.

**C13 notes: what is not an agent tool.**

- `explain_choice` is a query for the UI's Inspector, not an agent tool.
- `claim_task`, `update_task` and `send_message` stay with the orchestrator, because no LLM writes to the board (landscape pitfall 9).

**C24 notes: the Codex column, rebuilt from app-server events.**

| Activity | App-server event |
|---|---|
| read, search, execute (Observed) | `item/started{commandExecution}` with `commandActions[{type:read\|listFiles\|search}]` |
| await_approval | `thread/status/changed{activeFlags:["waitingOnApproval"]}` plus the `requestApproval` server request |
| plan | `turn/plan/updated` |
| edit, create, delete | `item/*{fileChange}` |
| diffstat | `turn/diff/updated` |
| thinking | `item/reasoning/summaryTextDelta` |
| live terminal | `item/commandExecution/outputDelta` |
| rate_limited | `account/rateLimits/updated` |

---

## 3. Claims that are wrong, stale, or still unverified

### 3.1 Wrong or stale: fix before implementation

- **W1. `cli-claude-codex` §5.2 `pump_stdout` stops after the first bad line** (oversized or non-UTF-8), and the child then blocks on a full pipe [V-critic]. Replace it with tauri-stack §5.2 `read_record`. Add a fixture test with a 70 MiB line and invalid UTF-8.
- **W2. `agents-backend` §6.6 worker prompt:** "Commit your work … git add -A && git commit". This fails under the Codex sandbox [V-critic]. It should probably also fail under a Claude sandbox with `allowWrite` set to the worktree [I]. Change it to "Do not commit; Codly snapshots your worktree", and let Codly commit (D7).
- **W3. `agents-backend` §6.6 Claude worker passes `--json-schema <worker_report.schema.json>`.**
  - The flag takes the JSON text, not a path. The help example is inline JSON, and the same report's planner correctly uses `"$(cat plan.schema.json)"`.
  - Whether `--json-schema` works with multi-turn `--input-format stream-json` is untested (U3).
  - Prefer the cross-vendor **`codly_submit_report`** MCP tool. Codex can also use `turn/start.outputSchema`.
- **W4. Claude tool names are stale in three reports** (C23):
  - context-engineering §2.1: "Glob/Grep(ripgrep)/Read";
  - landscape §1.6 and R6: TodoWrite → board;
  - design-world §9.3: "TodoWrite tool (when present)".

  In 2.1.309, TodoWrite is not a valid tool name at all [V-critic].
- **W5. `cli-others-acp` R3 says `_meta.terminal_output_delta` gives "live command output from the Claude and Codex adapters for free".** For Claude the output arrives only at completion [V-critic #17].
- **W6. `design-world` §5.4 and R8 set `minimumSystemVersion: "13.0"`.** That loses background throttling and conflicts with tauri-stack's `safari17` target (C20).
- **W7. rusqlite 0.41 appears in context-engineering §2.3 and §4.2, agents-backend §6.2 and §6.12, and landscape R5 and §15.** It is incompatible with rusqlite_migration, tokio-rusqlite and r2d2_sqlite today (C5).
- **W8. `landscape` R2 T2: "`codex-app-server-protocol` from the matching git tag".** Don't do this (C3).
- **W9. `design-world` §8.5 and §9.3: the Codex mapping is built from `exec --json`, with app-server "out of scope".** This is superseded (C24).
- **W10. `cli-others-acp` §8 says Node readline splits on U+2028.** This was not reproduced on Node 22.23.3 or Bun 1.3.14 [V-critic #20]. Rust framing on `\n` is fine either way.
- **W11. `cli-claude-codex` §5.5's broker table assumes Codex always offers `decline` and `acceptForSession`.**
  - In fixture (d), the command approval under `approvalPolicy:"untrusted"` offered only `availableDecisions: ["accept", {"acceptWithExecpolicyAmendment":…}, "cancel"]` [V-critic fixture].
  - Render the choices from `availableDecisions`. When `decline` is missing, "deny with feedback" becomes `cancel` plus a `turn/steer` or a new turn.
- **W12. `cli-claude-codex` §5.1 puts the "role/task brief" in `--append-system-prompt`.**
  - With `--system-prompt-snapshot` on (the default), a later `--resume` ignores a changed brief until the conversation is compacted [V-critic help].
  - Put static role rules in `--append-system-prompt-file`. Put the task brief, pointer pack, review feedback and blackboard notes in **user turns**. This matches context-engineering §4.8.
- **W13. The fixed station mappings in `landscape` R6 and `design-world` §9.5 will miss Claude's search activity**, because by default it arrives as `Bash` running `rg` or `grep` (W4). Either the argv rules (Inferred) or the explicit `--tools` list (Observed) is required.

### 3.2 Still unverified and decision-critical (ranked)

1. **U1. The Claude `can_use_tool` round trip on 2.1.309.** This includes `AskUserQuestion`, `updatedInput`, `updatedPermissions`, `interrupt:true` and `control_cancel_request`. It has never been exercised. (cli-claude-codex §6)
2. **U2. Claude `-p` with a per-session `--settings` sandbox block**, `{"sandbox":{"enabled":true,"filesystem":{"allowWrite":[<wt>, <codly dir>]}},"permissions":{"additionalDirectories":[…]}}`. Munder verified it only interactively, on 2.1.239 [Report].
3. **U3. `--json-schema` with multi-turn `--input-format stream-json`.** It is only proven for one-shot `-p` (agents-backend §5.3).
4. **U4. Codex app-server behaviour not yet captured:**
   - `thread/resume` after the app-server restarts mid-task;
   - approval shapes under `approvalPolicy:"on-request"` (the fixture used `untrusted`);
   - per-server `-c mcp_servers.<name>.enabled=false`;
   - nested `thread/start.config` keys.
5. **U5. Gemini fs handling.** Gemini accepting `-32002`, or `fs:false` removing its delegation entirely, plus what 0.65 changed (sessionCapabilities, configOptions).
6. **U6. Cursor, Copilot and Pi are docs-only.** None of them is installed here.
7. **U7. The Deep Agents sidecar was compiled but never executed.**
   - Its v3 event protocol package is `@langchain/protocol@0.0.20`, which is pre-1.0 [Report].
   - Subagent namespace attribution for parallel `task` calls is unproven.
8. **U8. Tauri Channel throughput and WebContent CPU with 3 agents streaming partial messages**, and App Nap behaviour while hidden.
9. **U9. Token savings from the pointer pack and MCP.** Stage B was never run, and the retrieval pilot covers one repo with regex symbols.
10. **U10. Policy questions still open:**
    - OpenAI's position on a third-party app spawning the user's logged-in `codex`;
    - how Anthropic reads "ordinary, individual usage" for 3-way parallel orchestration.

---

## 4. Gaps nobody covered

- **G1. macOS TCC privacy prompts for child processes** [I, spike].
  - Agents run as children of Codly.app, so access to protected folders is attributed to Codly. Those folders are `~/Documents` (this repo lives there), `~/Desktop`, `~/Downloads`, iCloud Drive and removable volumes.
  - The first access triggers "Codly would like to access…". A denial shows up inside agents as `EPERM` / "Operation not permitted", with no obvious cause.
  - **Worktrees in Application Support avoid it, but the source repo does not.** Onboarding has to explain this. Detect EPERM patterns in tool results and show a "Grant access" call to action.
- **G2. Distribution is unowned.**
  - Developer ID signing, notarization, hardened runtime, and updater keys (`tauri-plugin-updater` 2.14.0 [Report]).
  - The Bun sidecar needs `com.apple.security.cs.allow-jit` [Report].
  - **The Mac App Store is effectively out** [I]: App Sandbox and spawning user-installed CLIs that read `~/.claude` and `~/.codex` don't mix.
- **G3. Secrets and retention in persisted data.**
  - Raw NDJSON captures whole files, `.env` reads, and tokens printed by tools.
  - **Policy needed:**
    - a raw-log TTL (for example 30 days; normalized events kept);
    - per-repo opt-out;
    - redaction on "export / share session";
    - no cloud sync;
    - document that at-rest protection relies on FileVault.
  - The fixtures in git hold local paths and the user's skill names.
- **G4. Cross-agent prompt injection.** Blackboard notes, reviewer comments, retrieval snippets and diffs can all carry text that came from the repo, and they flow into other agents' prompts.
  - Type notes and cap their length.
  - Wrap them as quoted data with provenance ("note from Worker 2, untrusted").
  - Never let a note change policy or approvals.
  - Scope each `codly-mcp` token to its run and task.
  - design-world §8.6 covers display sanitization only.
- **G5. The Claude worker tool set needs hardening.**
  - The default `-p` tool set includes [V-critic fixture]:
    - `SendMessage` and `ListAgents`, cross-session messaging to the user's *other* Claude sessions;
    - `CronCreate`, `CronDelete` and `CronList`;
    - `RemoteTrigger`, `PushNotification`, `ScheduleWakeup` and `Workflow`;
    - `EnterWorktree` and `ExitWorktree`, which fight Codly's own worktrees;
    - `DesignSync`, `PublishPlugin` and `ShareOnboardingGuide`.
  - Pass an explicit per-role `--tools` list (D5). Diff `system/init.tools` against the request, because unknown names are dropped silently [V-critic].
- **G6. The user's global Claude setup leaks into Codly agents.**
  - The user's `SessionStart` hooks fired even under `env -i` with no prompt [V-critic]. Plugins, skills, user MCP servers and other tools' global hooks (Pixel Agents, Superset, claude-mem…) will observe or change Codly sessions.
  - Offer a **"clean-room" profile**: `--setting-sources local` or `""`, plus an explicit `--settings`, plus `--strict-mcp-config`. Offer **"inherit my setup"** as the alternative.
  - Show detected hooks and plugins in diagnostics.
- **G7. Claude has no live terminal in headless mode** (W5). Options:
  - **(a)** an elapsed-time heartbeat plus the final output. This is the v1 default; label it "output at completion".
  - **(b)** "Take over" in a PTY (G8).
  - **(c)** an optional Codly-run command tool over MCP that streams through Codly's PTY. This costs Claude's native Bash behaviour; research it, don't ship it in v1.
- **G8. No handoff between headless and interactive is specified.** Proposed flow:
  1. Stop at a turn boundary.
  2. Open `claude --resume <id>` or `codex resume <id>` in a Codly PTY.
  3. When the user exits, re-attach headless with `--resume` or `thread/resume`.
  4. Hold a per-session lock so two processes never drive one session.
- **G9. No workspace bootstrap is specified.**
  - **Setup:** dependency installs (APFS `cp -c` clones were noted), untracked `.env` files that a worktree won't contain, port blocks (Conductor's `PORT..PORT+9`), and setup scripts.
  - **The user's dirty checkout:** worktrees branch from a commit, so uncommitted changes are invisible to agents. Warn, or offer "snapshot WIP".
  - **Git hooks on Codly's commits** (pre-commit, husky, lint-staged): run them, or define a `--no-verify` policy.
- **G10. "Uses your subscription" is not automatically true.**
  - On this machine Gemini is `selectedType: gemini-api-key`, which is API billing. OpenCode sees `OPENROUTER_API_KEY` and `OPENAI_API_KEY` from the environment [Report].
  - Compute a billing badge per agent from auth state plus environment.
  - In "subscription only" mode, also scrub `GEMINI_API_KEY`, `GOOGLE_API_KEY` and `OPENROUTER_API_KEY` for the CLIs that would use them.
- **G11. Prompt-cache reuse across worktrees.**
  - Each worktree's cwd and git status are part of Claude's system prompt, so N parallel workers fragment the cache.
  - `--exclude-dynamic-system-prompt-sections` moves those parts into the first user message [V-critic help]. This could cut quota burn; measure it.
- **G12. Event-schema evolution.** Add `schema_version` on every row, plus a re-normalization job from the raw files when adapters change. cli-claude-codex hinted at this; nobody planned it.
- **G13. Power management.**
  - `UserInitiatedAllowingIdleSystemSleep` (tauri-stack §7.1) still lets the Mac idle-sleep mid-run.
  - Add an opt-in "keep awake while agents run" (`NSActivityIdleSystemSleepDisabled`).
  - Resume wedged runs after wake, as Munder does.
- **G14. No quota telemetry for Gemini, Cursor or Copilot.** Only Claude's and Codex's windows feed the scheduler. Copilot's `quotaSnapshots` is SDK-only [Report].
- **G15. Codly's own observability.** Tracing logs, opt-in crash reports, and an "export diagnostics" bundle with redaction.
- **G16. Workspace scope.** Multi-repo and monorepo sub-path workspaces are undecided. Windows and Linux are out of scope, which the Unix-socket, launchd and objc choices already assume.
- **G17. Claude transcripts and auto-memory scatter per worktree path** (`~/.claude/projects/<sanitized cwd>`). Consider a per-repo `CLAUDE_CODE_PROJECT_DIR_NAME` [Report: documented; untested].

---

## 5. Consolidated architecture decisions (priority order)

### D1. Product and legal guardrails (P0)

- **Codly never touches credentials.** The user installs and logs in to each CLI. Codly spawns the **unmodified** binaries. It never reads keychains or token files, never mints or stores OAuth tokens, and never proxies or resells.
- **Legal review before a public release** must cover:
  - Anthropic's Commercial Terms obligation for products that "run Claude Code";
  - its name and logo rules: say "runs Claude Code" in plain text only, and never use Claude Code or Anthropic names or logos in Codly's own name, logo or sprites;
  - the "ordinary, individual usage" clause;
  - OpenAI's stance, and its "Sign in with ChatGPT" route if Codly is paid or closed source.
- **Billing badge per agent:** `Subscription | API key | Third-party provider | Unknown`. Sources:
  - Claude's `system/init.apiKeySource`;
  - Codex `account/read`;
  - Gemini `selectedType`;
  - the environment.
- **Fallbacks, in order:**
  - BYOK into the same CLIs (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) is the first fallback;
  - the OpenRouter backend comes second;
  - a remote kill switch can turn subscription mode off.

### D2. Process and crate layout (P0)

```
codly/
├─ crates/codly-core      # no tauri dep: supervisor, adapters, approval broker, orchestrator, db, git, fs, index
├─ crates/codly-mcp       # stdio MCP server (rmcp 3.7.0) → UDS (0600) → codly-core; shipped as externalBin
├─ crates/codly-devserver # axum WS bridge for headless UI tests; prototype of a future daemon protocol
└─ src-tauri/             # thin shell: commands + Channels; platform/ (objc: App Nap, occlusion)
```

**Supervisor rules** [V-critic where noted]:

- **Spawning:**
  - `tokio::process::Command` with `.env_clear().envs(captured_login_env ∪ overrides)`, `.process_group(0)` and `.kill_on_drop(true)`;
  - stderr drained into a ring buffer;
  - stdout read through the bounded byte reader with a 64 MiB cap (C9).
- **Shutdown ladder:**
  1. Interrupt: Claude's `control_request{subtype:"interrupt"}`, or Codex's `turn/interrupt`.
  2. Wait 5 s.
  3. Close stdin.
  4. Wait 3 s.
  5. `killpg(SIGTERM)`.
  6. Wait 3 s.
  7. `killpg(SIGKILL)`.
- **On startup:** reconcile. Kill orphans matched by pgid plus process start time, then resume by session ID (agents-backend §6.9).
- **Environment scrub list** (cli-claude-codex §5.9, extended per G10):
  - `CLAUDECODE` and `CLAUDE_CODE_*`, especially `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SIMPLE` and `CLAUDE_CODE_OAUTH_TOKEN`;
  - `CLAUDE_AGENT_SDK_*`;
  - `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL`;
  - `OPENAI_API_KEY` and `CODEX_API_KEY`;
  - in subscription mode only: `GEMINI_API_KEY`, `GOOGLE_API_KEY` and `OPENROUTER_API_KEY`.
- **Child environment additions:** set `NO_COLOR=1` for Codex and `TERM=dumb` for pipes.

**Pin these crates (re-verified today):**

- `tauri 2.12.5` (at least 2.12.3);
- `rusqlite =0.40.2` bundled plus `rusqlite_migration 2.6.0`;
- `agent-client-protocol-schema 1.12.0`;
- `rmcp 3.7.0`;
- `portable-pty 0.9.0`;
- `process-wrap 10.0.1` (optional).

Carried from the reports: `tokio 1.54.1`, `tokio-util 0.7.20`, `serde_json 1.0.154` (`raw_value`), `notify 8.2.1` plus `notify-debouncer-full 0.7.0`, `nix 0.31.3`, `which 8.0.6`, `uuid 1.29` (v7), `ts-rs 12.0.1`, `ignore 0.4.33`, `grep-searcher 0.1.17`, `grep-regex 0.1.14`, `globset 0.4.20`, `jsonschema 0.60.1`, `schemars 1.2.3`, `petgraph 0.8.3`.

### D3. Storage (P0)

- **`codly.sqlite`** (WAL; one writer actor batching every 50–100 ms; 2–4 read connections).
  - Tables from agents-backend §6.2: `workspace`, `plan`, `task`, `task_dep`, `scope_lease`, `agent`, `run`, `event`, `message`, `artifact`, `approval`, `decision`.
  - Additional tables:
    - `turn_commit(run_id, turn_id, before_head, after_head, files_json)`, after Vibe Kanban;
    - `context_snapshot(run_id, turn_id, injected_json, tokens)`;
    - `retrieval_log`.
  - Pragmas, from tauri-stack §5.4: `journal_mode=WAL; synchronous=NORMAL; busy_timeout=5000; foreign_keys=ON`.
- **`index/<repo-id>.sqlite`** is a separate file with its own writer, so index writes never stall the event log.
- **Raw streams:** `runs/<run_id>/raw.ndjson`, appended *before* parsing and zstd-compressed on close. A TTL applies (G3).

### D4. Canonical event model (P0): one envelope, three layers

```rust
/// Row of `event` in codly.sqlite. Timeline, board, inspector and world are projections of this table.
pub struct EventEnvelope {
    pub event_id: i64,               // INTEGER PRIMARY KEY: global, monotonic. The ONLY UI cursor (snapshot.as_of).
    pub schema_version: u16,         // re-normalize from raw on bump (G12)
    pub ts_ms: i64,                  // Codly receive time; provider clocks (Codex emittedAtMs) live in payloads
    pub source: Source,              // Provider(Provider) | Codly
    pub agent_id: Option<AgentId>,
    pub run_id: Option<RunId>,       // one OS process lifetime / sidecar run
    pub run_seq: Option<u64>,        // UNIQUE(run_id, run_seq): idempotent ingestion only
    pub task_id: Option<TaskId>,
    pub session_id: Option<String>,  // claude session_id | codex threadId | acp sessionId | deepagents thread_id
    pub turn_id: Option<String>,
    pub ns: Vec<String>,             // subagent path: claude parent_tool_use_id chain | codex collab | deepagents namespace
    pub causation_id: Option<i64>,   // event_id or decision id that caused this (explainability backbone)
    pub kind: EventKind,
    pub raw: Option<RawRef>,         // { file, offset, len } into runs/<run_id>/raw.ndjson
}
// EventKind = cli-claude-codex §5.3 EventKind (the most complete), with these changes:
//   + SessionStarted { cli, cli_version, tier }         // Codly spawn time (claude system/init only appears after the 1st user msg)
//   + ToolProgress { call_id, elapsed_s }               // claude tool_progress heartbeats
//   + ContextUsage { used, size }                       // ACP usage_update, claude get_context_usage
//   + SubagentSpawned { call_id, child_session, title } and SubagentState { child, state }  (replace SubAgent)
//   + Compaction { phase, pre_tokens, post_tokens }
//   ApprovalRequested.choices = exactly what the provider offered (codex availableDecisions, ACP options[].kind,
//   claude permission_suggestions); never a fixed menu (W11).
// Codly domain kinds (source = Codly): TaskStateChanged, DecisionRecorded{decision_id}, GateResult, TurnCommitted
//   {before, after, files}, MergeResult, NotePosted{message_id}, RetrievalCompleted{confidence, citations}.
// Projections computed in Rust: WorldActivity (design-world §9.2, with seq := event_id, activityId := call_id),
//   UiBatch { events: Vec<(i64 /*event_id*/, UiEvent)> } (tauri-stack §2.4).
```

**Name mapping from the reports to the canonical model:**

| Report | Their name | Canonical |
|---|---|---|
| cli-others-acp | `ToolUpdated` | `ToolCompleted` + `ToolOutputDelta` + `FilesChanged` |
| cli-others-acp | `PermissionAsked` | `ApprovalRequested` |
| landscape | `NeedsInput` | `ApprovalRequested{subject: Question\|Permissions\|…}` |
| landscape | `FileChanged{before_commit, after_commit}` | `FilesChanged` plus the domain event `TurnCommitted` |
| landscape | `MessageSent` / `MessageDelivered` | `NotePosted`, with delivery as `causation_id` on the next `TurnStarted` |
| agents-backend sidecar | `run_state`, `thought_delta`, `tool_updated`, `permission_asked`, `turn_ended` | `StatusChanged`, `ReasoningDelta`, `ToolCompleted`, `ApprovalRequested`, `TurnCompleted` |
| tauri-stack / design-world | `UiEvent`, `WorldActivityEvent` | Projections, not stored as the source of truth |

### D5. Adapter matrix (P0 = vertical slice)

| Agent | Priority | Transport (exact) | Approvals | Live text / live command output | Cost / quota | Resume |
|---|---|---|---|---|---|---|
| **Claude Code** ≥ 2.1.302 | P0 | See the command block below | `can_use_tool` → broker. PreToolUse/Stop through `initialize.hooks` callbacks | Text deltas: yes. **Command output: at completion only** (G7) | `total_cost_usd` is a cumulative list-price estimate, so show per-turn deltas. `rate_limit_event` 5h/7d | `--resume <uuid>`, `--fork-session` |
| **Codex** ≥ 0.161 | P0 | One private `codex app-server` (stdio) per Codly instance and one thread per worktree; see the JSON below | Server requests; choices from `availableDecisions` (W11) | `item/agentMessage/delta`; **`item/commandExecution/outputDelta` is live** | Tokens only, so show an API-equivalent estimate. `account/rateLimits/updated` (5h, weekly) | `thread/resume` after restart; `thread/fork` |
| **OpenCode** | P1 (first ACP agent: free models for CI) | `opencode acp`, plus `OPENCODE_CONFIG_CONTENT` to force *ask* permissions (the defaults allow edit and bash) [verify the schema]. Call `session/close` after probes | `session/request_permission` (choose by `kind`) | Chunks plus tool updates | `usage_update.cost` | `session/resume`, `session/load`, `session/list` |
| **Gemini CLI** | P1 | `gemini --acp --skip-trust`. Capabilities `fs:{readTextFile:false,writeTextFile:false}, terminal:false, _meta:{terminal_output_delta:true}`. Model through the legacy `session/set_model` (0.62) | `request_permission` (only `write_file` and shell asked) | Thoughts; shell output **not** exposed | Tokens in `_meta.quota`; no quota data | `session/load` (no resume capability in 0.62) |
| Cursor | P2, docs-only | `agent acp`; must answer the `cursor/ask_question` and `cursor/create_plan` requests | `allow-once` … (by kind) | ? | none | `session/load` |
| Copilot CLI | P2, docs-only | `copilot --acp` (public preview) | yes | ? | `quotaSnapshots` via the SDK only | per changelog |
| Pi | P3, docs-only | `pi --mode rpc`; wait for `agent_settled` | extension UI | yes | `usage.cost.total` | `switch_session` |
| Deep Agents (API key) | P2 | Bun-compiled `codly-agent`, NDJSON JSON-RPC (agents-backend §3.6) | `interruptOn` → `permission_asked` | v3 stream | OpenRouter `usage.cost` via a fetch tee | Checkpointer thread |
| Claude or Codex via ACP adapters | not shipped in v1 | — | — | — | Loses rate limits and cost; codex-acp 2.2.2 runs `@openai/codex` 0.160.x [V-critic #13]; claude-agent-acp runs SDK 0.3.309 with its own bundled binary | — |

**Claude command line:**

```
claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages \
  --permission-prompt-tool stdio --permission-mode acceptEdits \
  --session-id <uuid> | --resume <uuid> \
  --tools "Bash,Read,Edit,Write,Grep,Glob,TaskCreate,TaskUpdate,TaskList,TaskGet,WebFetch,WebSearch,Agent,ToolSearch" \
  --strict-mcp-config --mcp-config '<codly-mcp json>' \
  --setting-sources user,local \
  --append-system-prompt-file <static role rules> \
  --model <alias> --effort <lvl> -n "codly:<agent>"
```

- Use `--setting-sources user,local` for trusted repos. For untrusted repos, or the clean-room profile, use `local` or `""` (G6).
- **Workers:** the task brief and pointer pack go in the first user turn (W12).
- **Tool list:** `ToolSearch` is assumed to keep deferred MCP tools reachable [verify]. Diff `init.tools` against the request.
- **Planner and reviewer:** read-only, via `--permission-mode plan` or `--tools "Read,Grep,Glob"`, with no Bash.

**Codex `thread/start` params:**

```json
{"model":"<from model/list>","cwd":"<worktree>","approvalPolicy":"on-request","sandbox":"workspace-write",
 "config":{"model_reasoning_effort":"medium","model_reasoning_summary":"auto",
           "mcp_servers":{"codly":{"command":"<Codly.app>/Contents/MacOS/codly-mcp","args":["--run","<id>"]}}},
 "developerInstructions":"<static role rules>","threadSource":"codly"}
```

- The nested `config` shape is unverified [verify].
- Per turn: `turn/start{threadId, input:[{type:"text",text,text_elements:[]}], outputSchema:<worker_report>}`.
- Spawn with `NO_COLOR=1` and drain stderr.
- Use `exec` only for L1 decisions: `codex exec --json --ephemeral --ignore-user-config -s read-only -m <cheap> --output-schema f.json "<prompt>" < /dev/null`.

### D6. Approvals and safety (P0)

- **One Rust `ApprovalBroker`:**
  - L0 rules come first: per-repo allowlist, deny patterns, and path policy (outside the worktree, `.env`, `~/.ssh`, `.git/`).
  - Choices are exactly the ones the provider offered.
  - **No default timeout.** Never auto-pick `allow_always`.
  - A provider cancel closes the card.
  - Requests appear in five places: inbox, world, agent card, dock badge, and a native notification sent from Rust, because the webview can be suspended.
  - **Notes and messages never grant consent.**
- **Safety profile per agent:**
  - **Codex:** `workspace-write` with network off by default.
  - **Claude:** the per-session `--settings` sandbox block, once U2 is verified.
  - **ACP agents:** the strictest mode they offer, such as Gemini `default` and OpenCode with ask permissions.
  - **All agents:** worktree isolation plus Codly's fs watcher as the source of truth.

### D7. Git and worktrees (P0)

- **Layout:**
  - worktrees under `~/Library/Application Support/Codly/worktrees/<repo-id>/<plan>/<task-key>`;
  - task branches named `codly/<plan>/<task-key>`;
  - integration branch `codly/<plan>/integration`.
- **Commands:**
  - always argv `git` behind a per-path mutex;
  - background reads run with `GIT_OPTIONAL_LOCKS=0 GIT_TERMINAL_PROMPT=0 LC_ALL=C`.
- **Codly commits each turn** (`git -C <wt> add -A && git -C <wt> commit -m "codly(<key>): turn <n>"`, with a hooks policy per G9) and stores `turn_commit` rows.
- **Merging:**
  - Pre-check with `git merge-tree --write-tree --name-only --messages codly/<plan>/integration codly/<plan>/<task>`.
  - Merge serially in the integration worktree.
  - Never push.
  - The user picks: merge into a branch, open a PR, or keep.
- **Cleanup:** never auto-delete a dirty or unintegrated worktree. Run GC after integration.

### D8. Orchestration (P1)

Adopt agents-backend §6 as written, except for W2 and W3:

- **Planning:** one worker is the default. Plans grow only when the planner's strict JSON DAG passes deterministic validation:
  - cycles rejected;
  - scope overlaps turned into dependency edges;
  - hot files serialized;
  - collapse back to one task when parallelism wouldn't pay.
- **Scheduling:**
  - 3 in total and 2 per vendor;
  - a task starts only after its dependencies are *merged*;
  - write-scope leases;
  - vendors parked on quota.
- **Gates, in order:**
  1. Codly runs `verify`.
  2. Scope check.
  3. A reviewer from a different vendor, with a clean context.
  4. Integration checks.
  5. The human.

  At most 2 fix loops before escalating.
- **Decisions:**
  - Log every routing, scheduling, gate and merge decision to `decision`, with its features, and record the outcome later.
  - L0 rules, then one L1 call per task, then L2 the human. Calibrate later (isotonic regression plus a Clopper–Pearson selective threshold).

### D9. Context and retrieval (P1, P2)

- **v1:**
  - FTS5 over paths and chunks with AND-first query shaping, RRF and agreement confidence;
  - a pointer pack of about 1.2k tokens or less in the *first user turn*;
  - `codly_search` through `codly-mcp`;
  - `retrieval_log` rows that power the Inspector's "why these files";
  - the `codly bench localize` benchmark in the product.
- **v1.5:** tree-sitter tags (static grammar crates only) and symbol-aligned chunks.
- **v2:** embeddings, only if Stage A and Stage B justify them.
- **Jev:** opt-in only (C29). Make no token-savings claim until Stage B has 3 repeats per arm.

### D10. Frontend and IPC (P0, P1)

- **Framework:** Svelte 5.57, Vite 8.3 and TypeScript 6.0.3, without SvelteKit.
- **Streaming:** one UI `Channel` per webview and one per attached terminal. Coalesce at about 30 Hz with an event-armed timer. Snapshot plus `event_id` cursor (D4). Gate on visibility. Send binary as `InvokeResponseBody::Raw`. Fetch large artifacts on demand.
- **Security:** an app ACL manifest in `build.rs` and a real CSP.
- **Plugins:**
  - single-instance, registered first (fix the order after `tauri add`);
  - window-state with `all() & !VISIBLE`;
  - dialog, opener, notification;
  - **no** shell, store or sql plugins.
- **Types and platform:** `ts-rs` generates the TS types. `minimumSystemVersion: "14.0"` and `build.target: 'safari17'`.
- **Panel libraries:** xterm 6 with WebGL on visible terminals only, `@pierre/diffs` plus CodeMirror 6, Svelte Flow plus dagre, svelte-dnd-action, bits-ui Command, and virtua.

### D11. World and design (P1)

- **Adopt design-world in full:**
  - top-down 3/4 view, 16 px tiles, 16×24 characters;
  - procedural art;
  - Canvas 2D with a frame scheduler that schedules nothing when idle;
  - the honesty contract: ≤ 250 ms to truth, 600 ms dwell, the 400 ms / 1.2 s excursion rule, no wandering by default;
  - OKLCH tokens, Inter and JetBrains Mono;
  - one Inspector with four provenance badges.
- **Apply these changes:** Workspace by default (C21), the rebuilt Codex column (C24), a Claude tool table built from `init.tools` (C23), the bubble rule (C25), macOS 14 (C20), and a perf test asserting an idle world makes 0 rAF calls in 10 s.

### D12. Testing (P0)

- **Fixture replay:** run every file in `fixtures/` through its adapter and snapshot the normalized events with `insta`. Include oversized-line, invalid-UTF-8 and unknown-type cases.
- **Drift CI:**
  - Codex `generate-json-schema` diffs across versions;
  - Claude `init.capabilities`, `claude_code_version`, `--help` and tool-set diffs;
  - the ACP schema version.
- **Live CI:** opt-in live smoke tests for the paid CLIs; free CI via `opencode acp` with `opencode/*` free models; `acp-tck` against Codly's client.
- **UI:** headless UI tests with Playwright WebKit against `MockTransport` (fixture sessions on a virtual clock). Run a few real-app tests through `@wdio/tauri-service` with the embedded `tauri-plugin-wdio-webdriver` behind an `e2e` feature.

---

## 6. Riskiest assumptions and the spikes that settle them (do these first)

| # | Assumption | Why it is risky | Spike (cost) | Fallback |
|---|---|---|---|---|
| 1 | Orchestrating several agents on one subscription stays allowed and unthrottled | Anthropic's 2026 churn (announced → paused → credits); the Commercial Terms requirement; the "ordinary, individual usage" clause; OpenAI is silent | Legal review plus a direct question to Anthropic and OpenAI partner channels. Billing badges and BYOK in the vertical slice (engineering cost only) | Make API-key mode the default for large parallel plans; route work to other vendors |
| 2 | Claude `can_use_tool` over stdio carries every prompt Codly needs (U1) | Never exercised; the whole Claude approval UX depends on it | **1 paid run:** a Bash command plus a Write outside acceptEdits, an `AskUserQuestion`, and a mid-request interrupt to trigger `control_cancel_request` | `--permission-prompt-tool mcp__codly__approve` served by `codly-mcp` (a documented contract) |
| 3 | Codex app-server is stable enough to build on | Labelled `[experimental]` in 0.161 [V-critic]; ships weekly | CI schema diff 0.161 → 0.164 → 0.165-alpha now (free); 1 paid run under `on-request` plus `thread/resume` after a restart (U4) | `exec --json`, with no approvals and the sandbox only |
| 4 | Headless streams give enough signal for an honest, lively world and a "live terminal" | Claude has no live command output; search arrives as Bash; plans come from Task* tools [V-critic] | **1 paid run:** long Bash, forced compaction, explicit `--tools` with Grep, Glob and Task* (design-world spike 2). Settle the take-over UX (G8) | Label "output at completion"; push Codex for long-running commands |
| 5 | Parallel agents pay off for coding | Google: −39–70% on sequential tasks; 19.8% / 41.7% merge-conflict rates [Report] | Dogfood with one worker by default; measure conflict rate and review time per plan | Keep parallelism for disjoint scopes only |
| 6 | macOS platform friction is manageable | TCC prompts (G1), notarizing the Bun JIT sidecar, App Nap and sleep | A notarized test build that spawns claude and codex on a repo in `~/Documents` (no model calls needed for TCC) | Onboarding guidance; worktrees outside protected folders; defer the sidecar |
| 7 | WKWebView plus Channels keep up with 3 agents streaming partial messages | Every `send()` is a JavaScript eval; unmeasured (U8) | MockTransport replaying 3 × 500 events/s; measure WebContent CPU and frame time | Coarser coalescing; drop token deltas while hidden |
| 8 | Retrieval and pointer packs save tokens | Stage B never run; one-repo pilot (U9) | Stage B with 3 repeats per arm on 24 commits (costs quota; after the slice) | Keep retrieval off the critical path; agents grep anyway |
| 9 | Professional users accept a pixel world | HN pushback on Munder ("cringe") [Report] | A 5-user test with Workspace as the default | Keep the World optional; no whimsy in the chrome |

---

## 7. Vertical-slice build order (merged across reports)

1. **`codly-core` skeleton.**
   - The supervisor, the bounded reader, raw-first logs and the SQLite schema (D3).
   - The event envelope (D4) and fixture-replay tests.
   - Env/PATH capture and the env scrub.
2. **First adapters.** The Claude adapter (stream-json, control protocol, `can_use_tool`, hook callbacks) and the Codex app-server adapter. Then the ApprovalBroker and billing badges. **Spikes 2–4 run here.**
3. **Single-task flow.** The worktree manager and Codly's per-turn commits, then: worktree → worker → Codly `verify` → scope check → `merge-tree` pre-check → merge into the integration branch. There is no planner yet; the request is task t1 (agents-backend §6.13).
4. **Workspace UI.** The Tauri shell and UI bus (Channels, snapshot plus `event_id`). Then the Workspace panels:
   - timeline, chat, the approvals inbox;
   - diff;
   - the terminal (live for Codex and PTY take-over);
   - the Inspector with provenance badges.
5. **The World.** Canvas 2D on the `WorldActivity` projection, under the honesty contract, in Split view.
6. **Orchestration.** The planner, DAG validation, scope leases and 2–3 parallel workers. Then the cross-vendor reviewer and the decision log.
7. **ACP adapter.** OpenCode first (free CI), then Gemini.
8. **`codly-mcp` and retrieval.** The tools `get_task`, `report_blocker`, `post_note`, `submit_report` and `search`. Then the FTS5 index, pointer packs and `codly bench localize`.
9. **Later.** Polish crash recovery, the Deep Agents sidecar, the L1 router and calibration, tree-sitter, Cursor and Copilot, and a possible `codlyd`.

---

## 8. Notes on each report

- **`cli-claude-codex.md`: the strongest primary-source work** (live fixtures, a schema generated from the installed binary, SDK source). To fix:
  - W1, the pump loop;
  - W11, the approval choices;
  - W12, the system-prompt snapshot.
  - Its open items are worth doing first: `can_use_tool`, and worktree commits (now settled by C7).
- **`cli-others-acp.md`: an accurate ACP inventory.** Verified: methods, error codes, crate churn, the deadlock trap.
  - W5 overstates what the Claude adapter's terminal extension delivers.
  - W10 repeats Pi's readline claim, which did not reproduce.
  - Advertising `subagents: {}` matters only for the adapters Codly doesn't ship in v1, so it is low priority.
- **`landscape.md`: excellent prior-art teardown.** Its R1 (daemon), R2-T1 (PTY default), R3 (mailbox), R5 (rusqlite 0.41 or sqlx) and R6 (Pixi, licensed art) are each overruled by stronger evidence elsewhere (C1, C6, C12, C5, C18, C19). Its pitfalls list (§12) should be kept as a design checklist.
- **`context-engineering.md`: the Jev verification is rigorous** and the measurements are honest about their limits. Fix the rusqlite version (W7) and the stale Claude tool names (W4). Its §6 benchmark spec is product-ready.
- **`tauri-stack.md`: the most correct on crate-level details.** It was right on rusqlite, the line reader, macOS 14, the shell plugin and Channels. Note that its "defer tree-sitter" sits in tension with context-engineering's slice (C22).
- **`agents-backend-orchestration.md`: the orchestration design is the right v1 design.** Fix W2 and W3, and use the app-server rather than `exec` for workers and recovery (C2). Deep Agents is well researched but unexecuted; keep it out of the slice.
- **`design-world.md`: a complete and implementable world spec.** Update the Codex column (W9), the Claude tool table (W4, W13), the macOS floor (W6) and the default view (C21). Its §8.6 sanitization rules should also cover prompts that agents read from other agents (G4).

---

## 9. Sources re-checked today

**Local runs** (outputs are in the scratchpad `critic/` folder):

- `claude --help` (2.1.309);
- `codex --help`, `codex exec --help` and `codex app-server --help` (0.161.0);
- `gemini --help` (0.62.0) and `opencode --help` (1.18.31);
- `git merge-tree --write-tree` in a scratch repo (git 2.54.0);
- free control-only Claude probes (Appendix A);
- readline tests on Node 22.23.3 and Bun 1.3.14.

**Source read:**

- `openai/codex` @970111f: `codex-rs/protocol/src/permissions.rs`; the generated `v2/TurnStartParams.json`.
- `agentclientprotocol/rust-sdk` @5ef0fa3: `src/agent-client-protocol/{Cargo.toml, src/role/acp.rs, src/jsonrpc.rs, examples/yolo_one_shot_client.rs}`.
- `agentclientprotocol/agent-client-protocol` @66ff807: `schema/v1/{meta.json, schema.json}`, `CHANGELOG.md`.
- `agentclientprotocol/claude-agent-acp` @739dde3: `src/tool-calls/renderer.ts`, `src/acp-agent.ts`.
- `anthropics/claude-agent-sdk-python` @0e89b29: `subprocess_cli.py:36,633`, `types.py:2304`.
- tokio-util 0.7.20: `src/codec/framed_impl.rs` (crates.io tarball).
- libsqlite3-sys 0.38.2: `build.rs`, `sqlite3/sqlite3.c` (crates.io tarball).
- wry 0.57.0: `src/wkwebview/mod.rs` (local cargo registry).
- `docs/research/fixtures/{claude-tool,claude-pong,codex-appserver,acp-gemini-write,acp-opencode-write}.jsonl`.

**Registries:**

- crates.io API for `rusqlite`, `rusqlite_migration`, `tokio-rusqlite`, `r2d2_sqlite`, `libsqlite3-sys`, `agent-client-protocol(-schema)`, `codex-app-server-protocol`, `tauri`, `portable-pty`, `process-wrap` and `rmcp`;
- `npm view @agentclientprotocol/codex-acp@2.2.2` and `@agentclientprotocol/claude-agent-acp@0.95.0`.

**Pages:**

- https://code.claude.com/docs/en/legal-and-compliance
- https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan (updates dated 2026-06-15 and 2026-10-07)
- https://code.claude.com/docs/en/hooks

---

## Appendix A: free probe for Claude tool availability (no inference)

This sends only control requests (`initialize`, `get_context_usage`), never a user message, so the model is never called.

- **What it measures:** `categories[id=system_tools].tokens` changes only when a requested `--tools` name exists.
- **The pattern to watch:** an unknown name, or a removed one such as `TodoWrite`, leaves the count unchanged.
- **Side effect:** the user's `SessionStart` hooks still run.

```python
# usage: python3 -I ctl_probe.py "Read,Grep,Glob"
import json, subprocess, sys, time, os, select
tools = sys.argv[1]
env = {"HOME": os.environ["HOME"], "PATH": "/usr/bin:/bin", "TERM": "dumb", "LANG": "en_US.UTF-8"}
cmd = [os.path.expanduser("~/.local/bin/claude"), "-p", "--input-format", "stream-json",
       "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--no-session-persistence", "--tools", tools]
p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True)
send = lambda o: (p.stdin.write(json.dumps(o) + "\n"), p.stdin.flush())
send({"type": "control_request", "request_id": "r1", "request": {"subtype": "initialize", "hooks": None}})
send({"type": "control_request", "request_id": "r2", "request": {"subtype": "get_context_usage"}})
got, deadline = {}, time.time() + 40
while time.time() < deadline and len(got) < 2:
    if not select.select([p.stdout], [], [], 1.0)[0]: continue
    line = p.stdout.readline()
    if not line: break
    try: o = json.loads(line)
    except ValueError: continue
    if o.get("type") == "control_response": got[o["response"]["request_id"]] = o["response"]
p.stdin.close(); p.wait(timeout=10)
for c in got.get("r2", {}).get("response", {}).get("categories", []):
    if c.get("id", "").startswith("system_tools"): print(tools, c["id"], c["tokens"])
```

Results on 2.1.309, as `system_tools` tokens:

| `--tools` value | Tokens |
|---|---|
| `Read` | 1008 |
| `Read,Bash` | 2293 |
| `Read,Grep` | 2284 |
| `Read,Grep,Glob` | 2561 |
| `Read,TodoWrite` | 1008 |
| `Read,BogusTool` | 1008 |
| `Read,TaskCreate` | 2379 |
| `Read,TaskUpdate` | 2714 |
| `Read,TaskList` | 1959 |
| `Read,TaskGet` | 1695 |
| `Read,Agent` or `Read,Task` | 2932 |
| `default` | 1391, plus 17196 deferred |

---

## 10. Adapter verification (live)

*2026-10-25, `claude` 2.1.309, `codex-cli` 0.161.0, `gemini` 0.62.0, `opencode` 1.18.31. Every run went through Codly's own adapters (`adapters::spawn`) via `cargo run -p codly-core --example adapter_smoke -- <provider>` in a throwaway git repo: the agent is asked to run `echo codly > hello.txt`, Codly answers the approval with `AgentCmd::Approval`, the file must exist afterwards. Spend: 6 paid runs (Claude haiku ×3, Codex gpt-6-luna/low ×2, Gemini 2.5-flash ×1), OpenCode free model ×2, plus free handshake-only runs and control-only probes. Two trimmed live captures are replayed by the tests (`crates/codly-core/src/adapters/testdata/`).*

| CLI | Event sequence (normalized, deltas folded) | Verdict |
|---|---|---|
| Claude | `session_started` (initialize) → `turn_started` → `session_started` (system/init, real tools) → `tool_started Bash` → `rate_limit` ×2 → `approval_requested` → *allow* → `tool_completed` → `message final` → `usage_updated` → `turn_completed completed` → `exited 0` | ✔ U1 settled for Bash |
| Claude, interrupt at approval | … `tool_started TaskCreate` → `plan_updated` ×2 → `tool_started Bash` → `approval_requested` → *Codly interrupts* → `approval_resolved cancelled/provider` → `tool_completed ok:false` → `turn_completed interrupted` | ✔ `control_cancel_request` works |
| Codex | `session_started` (thread/start) → `turn_started` → `tool_started commandExecution` → `approval_requested` → *accept* → `tool_completed` → `usage_updated` → `rate_limit` ×2 → `message final` → `turn_completed completed` → `exited` (thread/unsubscribe) | ✔ |
| Gemini | `session_started` → `turn_started` → `reasoning` → `tool_started run_shell_command` → `approval_requested` → *proceed_once* → `tool_completed` → `message final` → `usage_updated` → `turn_completed completed` → `exited 0` | ✔ |
| OpenCode | same shape as Gemini; `bash` asked through `session/request_permission` | ✔ |
| MCP (Claude, Codex, OpenCode, `--mcp`) | `tool_started` of `codly_post_note` classified `note`, answered by the stand-in codly server, no approval prompt | ✔ |

What differs from the reports, and what Codly does about it:

1. **Claude `can_use_tool` (2.1.309)** carries `tool_name, display_name, input, description, permission_suggestions, blocked_path, tool_use_id` (no `decision_reason`/`title` here). `blocked_path` is set even for a path *inside* the cwd, so it is not a risk by itself. Suggestions came as `addRules → localSettings` plus `addDirectories → session`. `{behavior:"allow", updatedInput}` is accepted (`permission_decision.source: "user_temporary"`).
2. **Interrupt with an approval open:** the CLI sends `control_cancel_request {request_id}`, rejects the tool, adds a user text `[Request interrupted by user for tool use]`, then `result {subtype:"error_during_execution", is_error:true, terminal_reason:"aborted_tools"}`. The process exits 1 when stdin closes afterwards.
3. **MCP deferral is caused by `ToolSearch`** (free `get_context_usage` probes): with `ToolSearch` in `--tools`, MCP tools land in `mcp_tools_deferred` and ~5.9k tokens of system tools (the Task* family) are deferred too; without it every requested tool loads. `"alwaysLoad": true` on the `--mcp-config` server entry (or `_meta["anthropic/alwaysLoad"]` per tool) keeps MCP tools loaded either way. **Codly drops `ToolSearch` from D5's list and sets `alwaysLoad`.** Live: `system/init.tools` lists `mcp__codly__*`, `mcp_servers: [{name:"codly", status:"connected"}]`, and `--allowedTools "mcp__codly__*"` lets the call run without `can_use_tool`.
4. `system/init.tools` reports `Agent` as `Task`. Thinking text is hidden (empty `thinking` plus a signature, and `system/thinking_tokens {estimated_tokens}`), so Claude has no live reasoning text. New subtypes seen: `session_title_changed` (from `-n`), `thinking_tokens`. The `initialize` response's `account` is `{email, organization, subscriptionType:"Claude Team", apiProvider}` with a login env, but only `{tokenSource:"none", apiProvider}` with a bare env.
5. **Task\* plan tracking:** `TaskCreate` results are `Task #<id> created successfully: …` with `tool_use_result.task {id, subject}`; Codly mirrors Claude Code's own bookkeeping (`TaskUpdate` by `taskId`, `TaskList` resyncs) into `PlanUpdated`.
6. **Codex config (free, `--strict-config`):** nested `thread/start.config` objects are read as dotted `-c` keys (a bogus nested key fails with "unknown configuration field `memories.bogus_sub` in -c/--config override"). Valid: `mcp_servers.codly.{command,args,env,default_tools_approval_mode}` with modes `auto|prompt|writes|approve`, `features.memories`, `memories.{generate_memories,use_memories}`. `threadSource` is a free string. `optOutNotificationMethods` works. The app-server exits 0 on stdin EOF, so it never outlives Codly. Codly threads set `default_tools_approval_mode:"approve"` for codly and turn memories off.
7. **Codex `on-request`:** the model asked for escalation as instructed; `availableDecisions` was again `[accept, acceptWithExecpolicyAmendment, cancel]` (W11 holds outside `untrusted` too), with `proposedExecpolicyAmendment` = the full argv `["/bin/zsh","-lc","echo codly > hello.txt"]`. `thread/unsubscribe` → `{status:"unsubscribed"}`.
8. **Memory isolation:** Claude reads `CLAUDE_CODE_DISABLE_AUTO_MEMORY` (name verified in the 2.1.309 binary); Codly sets it to 1 for every Claude agent after scrubbing `CLAUDE_CODE_*`. Gemini 0.62 has **no `save_memory` tool**: the model edits `GEMINI.md`/`MEMORY.md` with `write_file`/`replace`. Codly passes `--policy` with deny rules for `save_memory` and for `write_file`/`replace` whose args match `\.gemini/`. The policy loads (free check); the deny itself was not exercised, to avoid touching `~/.gemini`.
9. **ACP:** Gemini labels its `allow_always` option "Allow for this session", so Codly maps it to `allow_session`. Gemini exposes no shell output. OpenCode's `OPENCODE_CONFIG_CONTENT={"permission":{"edit":"ask","bash":"ask","webfetch":"ask"}}` takes effect (`opencode debug agent build` and live). Its MCP tools arrive as `codly_codly_post_note` and run without asking. OpenCode's first `tool_call` for a shell command carries only `cwd`, so Codly starts the tool when the command arrives.
10. **Not verified live:** `AskUserQuestion` (not in Codly's tool lists), `updatedPermissions` for allow-always and allow-session, Codex `turn/steer` and `thread/resume` after a restart, Gemini `session/load` replay, Cursor and Copilot (not installed).
